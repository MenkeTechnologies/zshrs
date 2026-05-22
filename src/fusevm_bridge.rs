//! fusevm bytecode-VM bridge for ShellExecutor.
//!
//! **Extension** — has no Src/exec.c counterpart. C zsh's `Src/exec.c::execlist`
//! (and related routines) implement the native **wordcode VM** that executes
//! compiler output from `parse.c`. zshrs compiles the parsed AST to fusevm
//! bytecode and runs it on a stack VM; this
//! file holds the bridge between fusevm's `ShellHost` trait and our
//! `ShellExecutor` state, the thread-local executor pointer, all
//! `BUILTIN_*` opcode constants, and the giant `register_builtins`
//! handler table that wires zsh builtins onto fusevm CallBuiltin
//! opcodes.

#![allow(unused_imports)]

use crate::history::HistoryEngine;
// MathState is private to math.rs (no public state struct in math.c).
use crate::options::ZSH_OPTIONS_SET;
// TcpSessions struct deleted — modules/tcp.rs uses ZTCP_SESSIONS thread_local.
use crate::zftp::zftp_globals;
// `Profiler` deleted — zprof state is module-level statics now.
use crate::zutil::style_table;
use compsys::cache::CompsysCache;
use compsys::CompInitResult;
use indexmap::IndexMap;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::sync::LazyLock;

use crate::exec_jobs::JobState;
use crate::intercepts::{intercept_matches, AdviceKind, Intercept};
use crate::ported::vm_helper::*;
use std::io::Write;

// ═══════════════════════════════════════════════════════════════════════════
// Thread-local executor context for VM builtin dispatch
// ═══════════════════════════════════════════════════════════════════════════

use crate::ported::zle::zle_thingy::getwidgettarget;
use crate::ported::options::opt_state_get;
use crate::ported::zsh_h::{isset, options, ERREXIT, MAX_OPS};
use crate::socket::bin_zsocket;
use fusevm::op::redirect_op as r;
use fusevm::shell_builtins::*;
use fusevm::Value;
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::ffi::CString;
use std::fs;
use std::io::BufRead;
use std::io::Read;
use std::io::Write as _;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::IntoRawFd;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::zsh_h::CASMOD_CAPS;

thread_local! {
    /// Mirror of C zsh's `doneps4` local in execcmd_exec
    /// (Src/exec.c:2517+). Tracks whether PS4 has been emitted
    /// for the current xtrace line so a coalesced sequence of
    /// XTRACE_ASSIGN + XTRACE_ARGS produces ONE line:
    ///   `<PS4>a=1 b=2 echo 1 2\n`
    /// instead of three. Reset to false by XTRACE_ARGS /
    /// XTRACE_NEWLINE after emitting the trailing `\n`.
    static XTRACE_DONE_PS4: Cell<bool> = const { Cell::new(false) };
}

// Thread-local pointer to the current ShellExecutor.
// Set before VM execution, cleared after. Used by builtin handlers.
thread_local! {
    static CURRENT_EXECUTOR: RefCell<Option<*mut ShellExecutor>> = const { RefCell::new(None) };
}

/// RAII guard that sets/clears the thread-local executor pointer.
///
/// Idempotent: calling `enter` when a context is already active is a no-op
/// for the entry side, and the guard's drop only clears the thread-local if
/// *this* call was the one that set it. Nested `execute_command` invocations
/// (e.g. from inside a builtin handler) reuse the outer pointer instead of
/// stomping it.
pub(crate) struct ExecutorContext {
    we_set_it: bool,
}

impl ExecutorContext {
    pub(crate) fn enter(executor: &mut ShellExecutor) -> Self {
        let we_set_it = CURRENT_EXECUTOR.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_some() {
                false
            } else {
                *slot = Some(executor as *mut ShellExecutor);
                true
            }
        });
        ExecutorContext { we_set_it }
    }
}

impl Drop for ExecutorContext {
    fn drop(&mut self) {
        if self.we_set_it {
            CURRENT_EXECUTOR.with(|cell| {
                *cell.borrow_mut() = None;
            });
        }
    }
}

/// Access the current executor from a builtin handler.
/// # Safety
/// Only call this from within a VM execution context (after ExecutorContext::enter).
#[inline]
pub(crate) fn with_executor<F, R>(f: F) -> R
where
    F: FnOnce(&mut ShellExecutor) -> R,
{
    CURRENT_EXECUTOR.with(|cell| {
        let ptr = cell
            .borrow()
            .expect("with_executor called outside VM context");
        // SAFETY: The pointer is valid for the duration of VM execution,
        // and we're single-threaded within the executor.
        let executor = unsafe { &mut *ptr };
        f(executor)
    })
}

// `try_with_executor` removed. The fallible variant was the bridge
// canonical-side ports used to mirror writes into the legacy
// exec.{variables,arrays,assoc_arrays,positional_params,
// local_save_stack,var_attrs} caches. All such mirrors are now
// dissolved: canonical setaparam / sethparam / setsparam write
// paramtab as the single source of truth; fusevm reads consult
// paramtab via exec.array() / exec.assoc() / exec.scalar() /
// exec.pparams() / exec.param_flags() helpers.
//
// PM_LOCAL scope save lives in BUILTIN_LOCAL dispatcher (with
// with_executor — the mandatory variant). Eval execute_script lives
// in BUILTIN_EVAL dispatcher. Lastval reads from canonical LASTVAL
// atomic that exec.set_last_status keeps current.

/// Look up a canonical builtin by name in `BUILTINS` and dispatch
/// via `execbuiltin` (Src/builtin.c:250). NO shadow check — calls the
/// builtin even if a user function with the same name exists. Used by
/// the `builtin foo` prefix opcode (which explicitly bypasses function
/// lookup per zsh semantics) and by internal call sites where shadowing
/// is unwanted. For zsh's normal name-resolution order (function shadows
/// builtin), use `dispatch_builtin` instead.
pub(crate) fn dispatch_builtin_raw(name: &str, args: Vec<String>) -> i32 {
    let bn_idx = crate::ported::builtin::BUILTINS
        .iter()
        .position(|b| b.node.nam == name);
    if let Some(idx) = bn_idx {
        let bn_static: &'static crate::ported::zsh_h::builtin =
            &crate::ported::builtin::BUILTINS[idx];
        let bn_ptr = bn_static as *const _ as *mut _;
        crate::ported::builtin::execbuiltin(args, Vec::new(), bn_ptr)
    } else {
        1
    }
}

/// Shadow-aware dispatch matching zsh's name-resolution order:
/// alias → reserved word → **function (shadows builtin)** → builtin →
/// external. All `BUILTIN_X` opcode handlers route through here so a
/// user-defined `cd () { … }` (or `r`, `fc`, `which`, … anything in
/// fusevm's name→opcode map) takes precedence over the C builtin —
/// matching `Src/exec.c:execcmd_exec`'s dispatch at c:3050-3068.
/// Without this, compile-time builtin resolution silently ignored
/// user wrappers (e.g. ZPWR's `cd () { builtin cd "$@"; … }`).
pub(crate) fn dispatch_builtin(name: &str, args: Vec<String>) -> i32 {
    if let Some(status) = try_user_fn_override(name, &args) {
        return status;
    }
    dispatch_builtin_raw(name, args)
}

/// Register all zsh builtins with the VM.
pub(crate) fn register_builtins(vm: &mut fusevm::VM) {
    // Macro for builtins that user functions are allowed to shadow.
    // zsh dispatch order is alias → function → builtin; without the
    // try_user_fn_override probe a `cat() { ... }; cat` would silently
    // run the C builtin and ignore the user function.
    macro_rules! reg_overridable {
        ($vm:expr, $id:expr, $name:literal, $method:ident) => {
            $vm.register_builtin($id, |vm, argc| {
                let args = pop_args(vm, argc);
                if let Some(s) = try_user_fn_override($name, &args) {
                    return Value::Status(s);
                }
                let status = with_executor(|exec| exec.$method(&args));
                Value::Status(status)
            });
        };
    }

    // Core builtins
    vm.register_builtin(BUILTIN_CD, |vm, argc| {
        let args = pop_args(vm, argc);
        if let Some(s) = try_user_fn_override("cd", &args) {
            return Value::Status(s);
        }
        let status = dispatch_builtin("cd", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PWD, |vm, argc| {
        let args = pop_args(vm, argc);
        if let Some(s) = try_user_fn_override("pwd", &args) {
            return Value::Status(s);
        }
        // Route through the canonical execbuiltin path so the `rLP`
        // optstr at BUILTINS["pwd"] is parsed into `ops`.
        let status = dispatch_builtin("pwd", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ECHO, |vm, argc| {
        let args = pop_args(vm, argc);
        if let Some(s) = try_user_fn_override("echo", &args) {
            return Value::Status(s);
        }
        // Update `$_` to the last arg before running. C zsh sets
        // zunderscore in execcmd_exec for every simple command,
        // including builtins.
        crate::ported::params::set_zunderscore(&args);
        let status = dispatch_builtin("echo", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PRINT, |vm, argc| {
        let args = pop_args(vm, argc);
        if let Some(s) = try_user_fn_override("print", &args) {
            return Value::Status(s);
        }
        crate::ported::params::set_zunderscore(&args);
        let status = dispatch_builtin("print", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PRINTF, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("printf", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_EXPORT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("export", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_UNSET, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("unset", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_SOURCE, |vm, argc| {
        let args = pop_args(vm, argc);
        // BUILTINS has `source` (c:116, Src/builtin.c) wired to
        // bin_dot. The legacy `dot` lookup-name predated the
        // canonical table merge and silently returned 1 without
        // emitting the "no such file or directory" error from
        // bin_dot's missing-file path, so failed sources looked
        // like silent successes from the user's side.
        let status = dispatch_builtin("source", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_EXIT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("exit", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_RETURN, |vm, argc| {
        let args = pop_args(vm, argc);
        // zsh: bare `return` (no arg) returns with the status of
        // the most recently executed command — `false; return`
        // returns 1, not 0. Direct port of zsh's bin_break/RETURN.
        // The executor's `last_status` is stale here (synced at
        // statement boundaries, not after each VM op), so read
        // the live `vm.last_status` instead.
        let live_status = vm.last_status;
        let status = {
            // Sync canonical LASTVAL to the VM's view BEFORE
            // bin_break("return") reads it for the no-arg fallback.
            with_executor(|exec| exec.set_last_status(live_status));
            dispatch_builtin("return", args)
        };
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_TRUE, |vm, argc| {
        let args = pop_args(vm, argc);
        if let Some(s) = try_user_fn_override("true", &args) {
            return Value::Status(s);
        }
        // `$_` for no-arg `true` is the command name itself ("true").
        // pop_args only updates pending_underscore from args; for
        // bare command name we backfill here.
        if args.is_empty() {
            with_executor(|exec| {
                exec.pending_underscore = Some("true".to_string());
            });
        }
        Value::Status(0)
    });
    vm.register_builtin(BUILTIN_FALSE, |vm, argc| {
        let args = pop_args(vm, argc);
        if let Some(s) = try_user_fn_override("false", &args) {
            return Value::Status(s);
        }
        if args.is_empty() {
            with_executor(|exec| {
                exec.pending_underscore = Some("false".to_string());
            });
        }
        Value::Status(1)
    });
    vm.register_builtin(BUILTIN_COLON, |vm, argc| {
        let args = pop_args(vm, argc);
        if args.is_empty() {
            with_executor(|exec| {
                exec.pending_underscore = Some(":".to_string());
            });
        }
        Value::Status(0)
    });

    vm.register_builtin(BUILTIN_TEST, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("test", args);
        Value::Status(status)
    });

    // Variable declaration
    vm.register_builtin(BUILTIN_LOCAL, |vm, argc| {
        let args = pop_args(vm, argc);
        // Canonical bin_local handles the entire scope chain
        // (`pm->old = oldpm` at Src/params.c:1137 inside createparam,
        // `pm->level = locallevel` at Src/builtin.c:2576 inside
        // typeset_single). The dispatcher only routes args.
        let status = dispatch_builtin("local", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_TYPESET, |vm, argc| {
        let args = pop_args(vm, argc);
        // fusevm's builtin_id maps both `declare` and `typeset` to
        // BUILTIN_TYPESET, so this handler must default to the
        // typeset error-prefix. compile_zsh special-cases `declare`
        // to register BUILTIN_DECLARE explicitly so that path keeps
        // the `declare:` prefix in error messages.
        let status = dispatch_builtin("typeset", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_DECLARE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("declare", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_READONLY, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("readonly", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_INTEGER, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("integer", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_FLOAT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("float", args);
        Value::Status(status)
    });

    // I/O
    vm.register_builtin(BUILTIN_READ, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("read", args);
        Value::Status(status)
    });

    // Control flow
    vm.register_builtin(BUILTIN_BREAK, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("break", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_CONTINUE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("continue", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_SHIFT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("shift", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_EVAL, |vm, argc| {
        // Direct port of `bin_eval(UNUSED(char *nam), char **argv, UNUSED(Options ops), UNUSED(int func))` body from Src/builtin.c:6151:
        //   `if (!*argv) return 0;`
        //   `prog = parse_string(zjoin(argv, ' ', 1), 1);`
        //   `execode(prog, 1, 0, "eval");`
        // The execode invocation lives here (not in the canonical
        // free-fn) because it must run through the bytecode VM's
        // current executor — the same VM that's mid-dispatch.
        let args = pop_args(vm, argc);
        if args.is_empty() {
            return Value::Status(0); // c:6160
        }
        let src = args.join(" "); // c:6166
        let status = with_executor(|exec| {
            // c:6175 execode
            exec.execute_script(&src).unwrap_or(1)
        });
        Value::Status(status)
    });

    // `builtin foo args…`: precmd-modifier that forces builtin dispatch,
    // bypassing alias AND function lookup. Without this, `builtin cd /`
    // inside a user `cd () { … }` wrapper recurses (real-world ZPWR pattern).
    // Handler pops argc args from the stack, treats args[0] as the builtin
    // name, and dispatches the rest via `dispatch_builtin` → `execbuiltin`
    // → `bin_*` directly. No function/alias lookup happens.
    vm.register_builtin(BUILTIN_BUILTIN, |vm, argc| {
        let args = pop_args(vm, argc);
        let Some((name, rest)) = args.split_first() else {
            // `builtin` with no args → list builtins (zsh emits nothing,
            // exit 0). Match that behavior; the BIN_BUILTIN bin_* in C
            // does the same default-list-nothing.
            return Value::Status(0);
        };
        // `builtin foo` MUST bypass function shadow — that's the whole
        // point of the prefix. Use the _raw helper, not the shadow-aware
        // one. Without this, `cd () { builtin cd "$@"; }` recurses.
        Value::Status(dispatch_builtin_raw(name, rest.to_vec()))
    });

    // `command foo args…` — BINF_COMMAND prefix (Src/builtin.c:44). Zsh
    // semantic: bypass alias+function lookup, search builtin then $PATH.
    // Without this, `cd () { command cd "$@" }` would re-invoke the user
    // wrapper (same root cause as the `builtin` bug). Flags `-p`/`-v`/`-V`
    // route to bin_whence with BIN_COMMAND funcid; bare `command foo`
    // dispatches builtin if present, else external (no fork — direct
    // spawn via execute_external since zshrs is non-forking).
    // BUILTIN_COMMAND — `command [-p] [-v|-V] cmd args…` BIN_PREFIX
    // (Src/builtin.c:45). PURE PASSTHRU: prepend "command" and hand
    // to `exec::execcmd_exec` (C port of `Src/exec.c::execcmd_exec`
    // precommand-modifier walk at c:3104-3187). That port already
    // does the -p / -v / -V option parsing, surfaces
    // `has_command_vv` for the whence redirect, and reports the
    // dispatch shape (is_builtin vs external).
    vm.register_builtin(BUILTIN_COMMAND, |vm, argc| {
        let args = pop_args(vm, argc);
        let mut full = Vec::with_capacity(args.len() + 1);
        full.push("command".to_string());
        full.extend(args.clone());
        let dispatch = crate::ported::exec::execcmd_exec(
            &full,
            crate::ported::zsh_h::WC_SIMPLE,
        );
        let post = &full[dispatch.precmd_skip..];
        if dispatch.has_command_vv {
            // `-v` / `-V` → bin_whence with BIN_COMMAND funcid.
            let mut ops = crate::ported::zsh_h::options {
                ind: [0u8; crate::ported::zsh_h::MAX_OPS],
                args: Vec::new(),
                argscount: 0,
                argsalloc: 0,
            };
            let mut name_pos = 0usize;
            let mut flag_byte = b'v';
            for (i, a) in post.iter().enumerate() {
                if a.starts_with('-') && a.len() >= 2 {
                    let body = &a.as_bytes()[1..];
                    if body.contains(&b'V') {
                        flag_byte = b'V';
                    }
                    name_pos = i + 1;
                } else {
                    name_pos = i;
                    break;
                }
            }
            ops.ind[flag_byte as usize] = 1;
            let whence_args: Vec<String> = post[name_pos..].to_vec();
            return Value::Status(crate::ported::builtin::bin_whence(
                "command",
                &whence_args,
                &ops,
                crate::ported::hashtable_h::BIN_COMMAND,
            ));
        }
        if dispatch.is_empty_command {
            return Value::Status(0);
        }
        let Some((name, rest)) = post.split_first() else {
            return Value::Status(0);
        };
        if crate::ported::builtin::BUILTINS
            .iter()
            .any(|b| b.node.nam == name.as_str())
        {
            return Value::Status(dispatch_builtin_raw(name, rest.to_vec()));
        }
        let n = name.clone();
        let r = rest.to_vec();
        Value::Status(
            with_executor(|exec| exec.execute_external(&n, &r, &[])).unwrap_or(127),
        )
    });

    // `exec cmd args…` — BINF_EXEC prefix (Src/builtin.c:45). Zsh
    // semantic: replace the current shell process with `cmd`. On Unix
    // this is `execvp(2)`; the call only returns on error. zshrs is
    // non-forking, so the shell process IS the calling process —
    // execvp here directly replaces it. Options `-a name` (override
    // argv[0]), `-c` (clean env), `-l` (login shell — prepend `-`)
    // ported minimally; advanced redirect-only `exec >file` is handled
    // upstream by compile_zsh and never reaches this handler.
    vm.register_builtin(BUILTIN_EXEC, |vm, argc| {
        let mut args = pop_args(vm, argc);
        let mut argv0_override: Option<String> = None;
        let mut clean_env = false;
        let mut login = false;
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if a == "--" {
                args.remove(i);
                break;
            }
            if !a.starts_with('-') || a.len() < 2 {
                break;
            }
            match a.as_str() {
                "-a" => {
                    args.remove(i);
                    if i < args.len() {
                        argv0_override = Some(args.remove(i));
                    }
                }
                "-c" => {
                    clean_env = true;
                    args.remove(i);
                }
                "-l" => {
                    login = true;
                    args.remove(i);
                }
                _ => break,
            }
        }
        let Some(cmd) = args.first().cloned() else {
            // `exec` with no command + no redirects = no-op success.
            return Value::Status(0);
        };
        let rest: Vec<String> = args[1..].to_vec();
        let display_argv0 = match argv0_override {
            Some(a) => a,
            None => {
                if login {
                    format!("-{}", cmd)
                } else {
                    cmd.clone()
                }
            }
        };
        let mut command = std::process::Command::new(&cmd);
        command.arg0(&display_argv0);
        command.args(&rest);
        if clean_env {
            command.env_clear();
        }
        use std::os::unix::process::CommandExt;
        // `exec` returns the OS error iff exec(2) failed; on success
        // it never returns. Match zsh: print the error to stderr with
        // the `exec` prefix and exit 127 (cmd not found) or 126 (not
        // executable).
        let err = command.exec();
        let code = match err.kind() {
            std::io::ErrorKind::NotFound => {
                eprintln!("zshrs: exec: {}: not found", cmd);
                127
            }
            std::io::ErrorKind::PermissionDenied => {
                eprintln!("zshrs: exec: {}: permission denied", cmd);
                126
            }
            _ => {
                eprintln!("zshrs: exec: {}: {}", cmd, err);
                127
            }
        };
        std::process::exit(code);
    });

    vm.register_builtin(BUILTIN_LET, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("let", args);
        Value::Status(status)
    });

    // Job control
    vm.register_builtin(BUILTIN_JOBS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("jobs", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_FG, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("fg", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_BG, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("bg", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_KILL, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("kill", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_DISOWN, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("disown", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_WAIT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("wait", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_SUSPEND, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("suspend", args);
        Value::Status(status)
    });

    // History — `fc`, `history`, and `r` all route to `bin_fc` (zsh
    // registers them as aliases of the same builtin per Src/builtin.c).
    // Previous "wires deleted; opcode never emitted" comment was wrong:
    // fusevm's `builtin_id("history")` / `("r")` ARE Some(…), so
    // compile_zsh emits Op::CallBuiltin for them — without a registered
    // handler they were silent no-ops, masking user `r () { … }` wrappers
    // (the ZPWR autoload pattern hit this).
    vm.register_builtin(BUILTIN_FC, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("fc", args);
        Value::Status(status)
    });
    vm.register_builtin(BUILTIN_HISTORY, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("history", args);
        Value::Status(status)
    });
    vm.register_builtin(BUILTIN_R, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("r", args);
        Value::Status(status)
    });

    // Aliases
    vm.register_builtin(BUILTIN_ALIAS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("alias", args);
        Value::Status(status)
    });

    // BUILTIN_UNALIAS wire deleted with its stub.

    // Options
    vm.register_builtin(BUILTIN_SET, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("set", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_SETOPT, |vm, argc| {
        let args = pop_args(vm, argc);
        // Canonical bin_setopt per options.c:580 — `isun` discriminant
        // flips the action polarity; setopt → 0, unsetopt → 1.
        let ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let status = crate::ported::options::bin_setopt("setopt", &args, &ops, 0);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_UNSETOPT, |vm, argc| {
        let args = pop_args(vm, argc);
        let ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let status = crate::ported::options::bin_setopt("unsetopt", &args, &ops, 1);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_SHOPT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = crate::extensions::ext_builtins::shopt(&args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_EMULATE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("emulate", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_GETOPTS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("getopts", args);
        Value::Status(status)
    });
    // BUILTIN_AUTOLOAD / BUILTIN_UNFUNCTION wires deleted with their
    // stubs. `bin_functions` stays — wired to the canonical port.
    vm.register_builtin(BUILTIN_AUTOLOAD, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("autoload", args);
        Value::Status(status)
    });
    // BUILTIN_AUTOLOAD / BUILTIN_UNFUNCTION wires deleted with their
    // stubs. `bin_functions` stays — wired to the canonical port.
    vm.register_builtin(BUILTIN_FUNCTIONS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("functions", args);
        Value::Status(status)
    });

    // Traps
    vm.register_builtin(BUILTIN_TRAP, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("trap", args);
        Value::Status(status)
    });

    // BUILTIN_PUSHD / BUILTIN_POPD wires deleted with their stubs.
    // `bin_dirs` stays — wired to the canonical port.
    vm.register_builtin(BUILTIN_DIRS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("dirs", args);
        Value::Status(status)
    });

    // type / whence / where / which all route through `bin_whence`
    // (canonical port at `src/ported/builtin.rs:3734` of
    // `Src/builtin.c:3975`). Each gets its own opcode so funcid +
    // defopts come from the BUILTINS table entry — execbuiltin
    // applies them correctly via the module-level dispatch_builtin.
    vm.register_builtin(BUILTIN_WHENCE, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(dispatch_builtin("whence", args))
    });
    vm.register_builtin(BUILTIN_TYPE, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(dispatch_builtin("type", args))
    });
    vm.register_builtin(BUILTIN_WHICH, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(dispatch_builtin("which", args))
    });
    vm.register_builtin(BUILTIN_WHERE, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(dispatch_builtin("where", args))
    });

    vm.register_builtin(BUILTIN_HASH, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("hash", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_REHASH, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("rehash", args);
        Value::Status(status)
    });

    // `unhash`/`unalias`/`unfunction` share `bin_unhash` (Src/builtin.c:
    // c:4350) but each carries its own funcid (BIN_UNHASH /
    // BIN_UNALIAS / BIN_UNFUNCTION) in the BUILTINS table. Route each
    // through `execbuiltin` so the correct funcid + optstr propagate
    // — earlier wiring passed funcid=0 unconditionally and `unalias`
    // silently no-op'd on the cmdnamtab path.
    fn unhash_via_execbuiltin(name: &str, args: Vec<String>) -> i32 {
        let bn_idx = crate::ported::builtin::BUILTINS
            .iter()
            .position(|b| b.node.nam == name);
        if let Some(idx) = bn_idx {
            let bn_static: &'static crate::ported::zsh_h::builtin =
                &crate::ported::builtin::BUILTINS[idx];
            let bn_ptr = bn_static as *const _ as *mut _;
            crate::ported::builtin::execbuiltin(args, Vec::new(), bn_ptr)
        } else {
            1
        }
    }
    vm.register_builtin(BUILTIN_UNHASH, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(unhash_via_execbuiltin("unhash", args))
    });
    vm.register_builtin(BUILTIN_UNALIAS, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(unhash_via_execbuiltin("unalias", args))
    });
    vm.register_builtin(BUILTIN_UNFUNCTION, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(unhash_via_execbuiltin("unfunction", args))
    });

    // Completion
    vm.register_builtin(BUILTIN_COMPGEN, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_compgen(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_COMPLETE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_complete(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_COMPOPT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_compopt(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_COMPADD, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("compadd", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_COMPSET, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("compset", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_COMPDEF, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_compdef(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_COMPINIT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_compinit(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_CDREPLAY, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_cdreplay(&args));
        Value::Status(status)
    });

    // Zsh-specific
    vm.register_builtin(BUILTIN_ZSTYLE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("zstyle", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZMODLOAD, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("zmodload", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_BINDKEY, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("bindkey", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZLE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("zle", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_VARED, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("vared", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZCOMPILE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("zcompile", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZFORMAT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("zformat", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZPARSEOPTS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("zparseopts", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZREGEXPARSE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("zregexparse", args);
        Value::Status(status)
    });

    // Resource limits
    vm.register_builtin(BUILTIN_ULIMIT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_ulimit(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_LIMIT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_limit(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_UNLIMIT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_unlimit(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_UMASK, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("umask", args);
        Value::Status(status)
    });

    // Misc
    vm.register_builtin(BUILTIN_TIMES, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("times", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_CALLER, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_caller(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_HELP, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_help(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ENABLE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("enable", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_DISABLE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("disable", args);
        Value::Status(status)
    });

    // BUILTIN_NOGLOB wire deleted with its stub.

    vm.register_builtin(BUILTIN_TTYCTL, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("ttyctl", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_SYNC, |vm, argc| {
        let args = pop_args(vm, argc);
        // Canonical bin_sync per files.c:53 — `sync(); return 0;`.
        let ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let status = crate::ported::modules::files::bin_sync("sync", &args, &ops, 0);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_MKDIR, |vm, argc| {
        let args = pop_args(vm, argc);
        // Canonical bin_mkdir wired in BUILTINS table (files.c:63).
        // execbuiltin handles the "pm:" optstr parsing.
        Value::Status(dispatch_builtin("mkdir", args))
    });

    vm.register_builtin(BUILTIN_STRFTIME, |vm, argc| {
        let args = pop_args(vm, argc);
        // Canonical bin_strftime takes (nam, argv, ops, func) per
        // Src/Modules/datetime.c:187. Adapt &[String] → &[&str] +
        // empty options inline (datetime parses no flags).
        let ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let status = crate::ported::modules::datetime::bin_strftime("strftime", &args, &ops, 0);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZSLEEP, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = crate::extensions::ext_builtins::zsleep(&args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZSYSTEM, |vm, argc| {
        let args = pop_args(vm, argc);
        // bin_zsystem now takes the canonical C signature
        // (name, args, ops, func) per Src/Modules/system.c:806.
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _ = with_executor(|_exec| ());
        let status = crate::modules::system::bin_zsystem("zsystem", &args, &ops, 0);
        Value::Status(status)
    });

    // PCRE
    vm.register_builtin(BUILTIN_PCRE_COMPILE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("pcre_compile", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PCRE_MATCH, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("pcre_match", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PCRE_STUDY, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("pcre_study", args);
        Value::Status(status)
    });

    // Database (GDBM)
    vm.register_builtin(BUILTIN_ZTIE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("ztie", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZUNTIE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("zuntie", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZGDBMPATH, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = dispatch_builtin("zgdbmpath", args);
        Value::Status(status)
    });

    // Prompt
    vm.register_builtin(BUILTIN_PROMPTINIT, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(crate::extensions::ext_builtins::promptinit(&args))
    });

    vm.register_builtin(BUILTIN_PROMPT, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(crate::extensions::ext_builtins::prompt(&args))
    });

    // Async / Parallel (zshrs extensions)
    vm.register_builtin(BUILTIN_ASYNC, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_async(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_AWAIT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_await(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PMAP, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_pmap(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PGREP, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_pgrep(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PEACH, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_peach(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_BARRIER, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_barrier(&args));
        Value::Status(status)
    });

    // Intercept (AOP)
    vm.register_builtin(BUILTIN_INTERCEPT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_intercept(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_INTERCEPT_PROCEED, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_intercept_proceed(&args));
        Value::Status(status)
    });

    // Debug / Profile
    vm.register_builtin(BUILTIN_DOCTOR, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_doctor(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_DBVIEW, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_dbview(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PROFILE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_profile(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZPROF, |vm, argc| {
        let args = pop_args(vm, argc);
        // bin_zprof now takes the canonical C signature
        // (name, args, ops, func) per Src/Modules/zprof.c:139.
        let mut ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        if args.iter().any(|a| a == "-c") {
            ops.ind[b'c' as usize] = 1;
        }
        let _ = with_executor(|_exec| ());
        let status = crate::modules::zprof::bin_zprof("zprof", &args, &ops, 0);
        Value::Status(status)
    });

    // ═══════════════════════════════════════════════════════════════════════
    // Coreutils builtins (anti-fork, gated by !posix_mode)
    //
    // All of these are routinely wrapped by user functions in real
    // dotfiles (zpwr, oh-my-zsh, etc.) — `cat() { ... }`, `ls() { ... }`,
    // `find() { ... }`. Each handler MUST consult try_user_fn_override
    // first (via reg_overridable!) so the user definition wins, matching
    // zsh's alias → function → builtin dispatch order.
    // ═══════════════════════════════════════════════════════════════════════

    reg_overridable!(vm, BUILTIN_CAT, "cat", builtin_cat);
    reg_overridable!(vm, BUILTIN_HEAD, "head", builtin_head);
    reg_overridable!(vm, BUILTIN_TAIL, "tail", builtin_tail);
    reg_overridable!(vm, BUILTIN_WC, "wc", builtin_wc);
    reg_overridable!(vm, BUILTIN_BASENAME, "basename", builtin_basename);
    reg_overridable!(vm, BUILTIN_DIRNAME, "dirname", builtin_dirname);
    reg_overridable!(vm, BUILTIN_TOUCH, "touch", builtin_touch);
    reg_overridable!(vm, BUILTIN_REALPATH, "realpath", builtin_realpath);
    reg_overridable!(vm, BUILTIN_SORT, "sort", builtin_sort);
    reg_overridable!(vm, BUILTIN_FIND, "find", builtin_find);
    reg_overridable!(vm, BUILTIN_UNIQ, "uniq", builtin_uniq);
    reg_overridable!(vm, BUILTIN_CUT, "cut", builtin_cut);
    reg_overridable!(vm, BUILTIN_TR, "tr", builtin_tr);
    reg_overridable!(vm, BUILTIN_SEQ, "seq", builtin_seq);
    reg_overridable!(vm, BUILTIN_REV, "rev", builtin_rev);
    reg_overridable!(vm, BUILTIN_TEE, "tee", builtin_tee);
    reg_overridable!(vm, BUILTIN_SLEEP, "sleep", builtin_sleep);
    reg_overridable!(vm, BUILTIN_WHOAMI, "whoami", builtin_whoami);
    reg_overridable!(vm, BUILTIN_ID, "id", builtin_id);

    reg_overridable!(vm, BUILTIN_HOSTNAME, "hostname", builtin_hostname);
    reg_overridable!(vm, BUILTIN_UNAME, "uname", builtin_uname);
    reg_overridable!(vm, BUILTIN_DATE, "date", builtin_date);
    reg_overridable!(vm, BUILTIN_MKTEMP, "mktemp", builtin_mktemp);

    // BUILTIN_EXPAND_WORD_RUNTIME (id 281) was a legacy JSON round-trip
    // bridge that no chunk emits anymore. The constant + handler are
    // removed; the ID stays reserved in the gap before
    // BUILTIN_REGISTER_FUNCTION so future remaps don't reuse it.

    // Pipeline execution — bytecode-native fork-per-stage. Pops N sub-chunk
    // indices, forks N children with stdin/stdout wired through N-1 pipes,
    // each child runs its stage's compiled bytecode and exits. Parent waits
    // and returns the last stage's status.
    //
    // Caveats: post-fork in a multi-threaded program, only async-signal-safe
    // ops are POSIX-safe. We violate this (running the bytecode VM after fork
    // touches mutexes like REGEX_CACHE). In practice, most pipeline stages
    // don't touch shared mutex state — externals fork/exec away, builtins do
    // pure I/O. Risks are bounded; if a stage does touch a held mutex, the
    // child deadlocks.
    vm.register_builtin(BUILTIN_RUN_PIPELINE, |vm, argc| {
        let n = argc as usize;
        if n == 0 {
            return Value::Status(0);
        }

        // Pop N sub-chunk indices (LIFO → reverse to stage order)
        let mut indices: Vec<u16> = Vec::with_capacity(n);
        for _ in 0..n {
            indices.push(vm.pop().to_int() as u16);
        }
        indices.reverse();

        // Clone each stage's sub-chunk
        let stages: Vec<fusevm::Chunk> = indices
            .iter()
            .filter_map(|&i| vm.chunk.sub_chunks.get(i as usize).cloned())
            .collect();
        if stages.len() != n {
            return Value::Status(1);
        }

        // Single stage — no pipe, just run inline
        if n == 1 {
            let stage = stages.into_iter().next().unwrap();
            crate::fusevm_disasm::maybe_print_stdout("pipeline:single", &stage);
            let mut stage_vm = fusevm::VM::new(stage);
            register_builtins(&mut stage_vm);
            let _ = stage_vm.run();
            return Value::Status(stage_vm.last_status);
        }

        // Build N-1 pipes
        let mut pipes: Vec<(i32, i32)> = Vec::with_capacity(n - 1);
        for _ in 0..n - 1 {
            let mut fds = [0i32; 2];
            if unsafe { libc::pipe(fds.as_mut_ptr()) } < 0 {
                // Cleanup any pipes we already created
                for (r, w) in &pipes {
                    unsafe {
                        libc::close(*r);
                        libc::close(*w);
                    }
                }
                return Value::Status(1);
            }
            pipes.push((fds[0], fds[1]));
        }

        // zsh runs the LAST stage of a pipeline in the CURRENT shell
        // (not a forked child) so a trailing `read x` keeps its
        // assignment in the parent. Other shells (bash) fork every
        // stage. Honor zsh by leaving stage N-1 inline. Forks the
        // first N-1 stages with fork(); runs the last in this process
        // with stdin dup2'd to the last pipe's read end and stdout
        // restored after.
        let last_idx = n - 1;
        let stages_vec: Vec<fusevm::Chunk> = stages.into_iter().collect();

        let mut child_pids: Vec<libc::pid_t> = Vec::with_capacity(n - 1);
        for (i, chunk) in stages_vec.iter().take(last_idx).enumerate() {
            match unsafe { libc::fork() } {
                -1 => {
                    // fork failed — kill any children we already started
                    for pid in &child_pids {
                        unsafe { libc::kill(*pid, libc::SIGTERM) };
                    }
                    for (r, w) in &pipes {
                        unsafe {
                            libc::close(*r);
                            libc::close(*w);
                        }
                    }
                    return Value::Status(1);
                }
                0 => {
                    // Reset SIGPIPE to default so a broken-pipe write
                    // kills the child cleanly instead of triggering a
                    // Rust println! panic. The parent shell ignores
                    // SIGPIPE so it can handle EPIPE itself, but child
                    // pipeline stages should die quietly when their
                    // downstream stage closes early (e.g. `seq | head -3`).
                    unsafe {
                        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
                    }
                    // Child: wire stdin from previous pipe's read end
                    if i > 0 {
                        unsafe {
                            libc::dup2(pipes[i - 1].0, libc::STDIN_FILENO);
                        }
                    }
                    // Wire stdout to next pipe's write end
                    unsafe {
                        libc::dup2(pipes[i].1, libc::STDOUT_FILENO);
                    }
                    // Close all original pipe fds (keeping stdin/stdout dups)
                    for (r, w) in &pipes {
                        unsafe {
                            libc::close(*r);
                            libc::close(*w);
                        }
                    }

                    // Run this stage's bytecode on a fresh VM
                    crate::fusevm_disasm::maybe_print_stdout(
                        &format!("pipeline:child:stage:{i}"),
                        chunk,
                    );
                    let mut stage_vm = fusevm::VM::new(chunk.clone());
                    register_builtins(&mut stage_vm);
                    let _ = stage_vm.run();
                    // Flush any buffered output before exiting
                    let _ = std::io::stdout().flush();
                    let _ = std::io::stderr().flush();
                    std::process::exit(stage_vm.last_status);
                }
                pid => {
                    child_pids.push(pid);
                }
            }
        }

        // Parent runs the LAST stage inline. Save stdin, dup the last
        // pipe's read end onto fd 0, run the chunk, restore stdin.
        // Close every other pipe fd so the producer side gets EOF
        // when the last upstream stage exits.
        let saved_stdin = unsafe { libc::dup(libc::STDIN_FILENO) };
        if last_idx > 0 {
            let read_fd = pipes[last_idx - 1].0;
            unsafe {
                libc::dup2(read_fd, libc::STDIN_FILENO);
            }
        }
        // Close all pipe fds in the parent now that stdin is wired.
        // (Children already have their own copies. The dup2 above
        // already gave us a fresh fd 0 if needed.)
        for (r, w) in &pipes {
            unsafe {
                libc::close(*r);
                libc::close(*w);
            }
        }

        // Run the last stage's bytecode on a sub-VM with the host
        // wired up. The host points back at the executor so reads
        // (`read x`) update the parent's variables directly.
        let last_stage_status = {
            let last_chunk = stages_vec.into_iter().last().unwrap();
            crate::fusevm_disasm::maybe_print_stdout("pipeline:last", &last_chunk);
            let mut stage_vm = fusevm::VM::new(last_chunk);
            register_builtins(&mut stage_vm);
            stage_vm.set_shell_host(Box::new(ZshrsHost));
            let _ = stage_vm.run();
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            stage_vm.last_status
        };

        // Restore stdin
        if saved_stdin >= 0 {
            unsafe {
                libc::dup2(saved_stdin, libc::STDIN_FILENO);
                libc::close(saved_stdin);
            }
        }

        // Wait for all forked stages, capture per-stage statuses for PIPESTATUS.
        let mut pipestatus: Vec<i32> = Vec::with_capacity(n);
        for pid in child_pids {
            let mut status: i32 = 0;
            unsafe {
                libc::waitpid(pid, &mut status, 0);
            }
            let s = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else if libc::WIFSIGNALED(status) {
                128 + libc::WTERMSIG(status)
            } else {
                1
            };
            pipestatus.push(s);
        }
        // Append the in-parent last-stage status so `pipestatus` ends
        // with N entries (one per stage).
        pipestatus.push(last_stage_status);
        // Pipeline exit status: by default, the LAST stage's status.
        // With `setopt pipefail` (or `set -o pipefail`), use the
        // first non-zero stage status (so failures earlier in the
        // pipeline propagate even if the last stage succeeded).
        let pipefail_on = with_executor(|exec| {
            crate::ported::options::opt_state_get("pipefail").unwrap_or(false)
        });
        let last_status = if pipefail_on {
            pipestatus
                .iter()
                .copied()
                .rfind(|&s| s != 0)
                .or_else(|| pipestatus.last().copied())
                .unwrap_or(0)
        } else {
            *pipestatus.last().unwrap_or(&0)
        };

        // Populate `pipestatus` (zsh) and `PIPESTATUS` (bash) arrays so
        // scripts can inspect per-stage exit codes. Both names are common
        // in user code; populating both removes a portability foot-gun.
        with_executor(|exec| {
            let strs: Vec<String> = pipestatus.iter().map(|s| s.to_string()).collect();
            exec.set_array("pipestatus".to_string(), strs.clone());
            exec.set_array("PIPESTATUS".to_string(), strs);
        });

        Value::Status(last_status)
    });

    // Array→String join. Pops one value; if it's an Array (e.g. from Op::Glob),
    // joins string-coerced elements with a single space. Pass-through for
    // non-arrays so the op is safe to chain after any String-or-Array producer.
    vm.register_builtin(BUILTIN_ARRAY_JOIN, |vm, _argc| {
        let val = vm.pop();
        match val {
            fusevm::Value::Array(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_str()).collect();
                fusevm::Value::str(parts.join(" "))
            }
            other => other,
        }
    });

    // `cmd &` background execution. Compile_list emits this for any item
    // followed by ListOp::Amp: the cmd is compiled into a sub-chunk, its index
    // pushed, then this builtin pops the index, looks up the chunk, forks. The
    // child detaches via setsid (so SIGINT to the foreground job doesn't kill
    // it), runs the bytecode on a fresh VM with builtins re-registered, exits
    // with the last status. The parent returns Status(0) immediately. Job
    // tracking via JobTable is deferred to Phase G6 — JobTable::add_job
    // currently requires a std::process::Child, which a libc::fork doesn't
    // produce. Until then, `jobs`/`fg`/`wait` can't see these pids.
    //WARNING FAKE AND MUST BE DELETED
    vm.register_builtin(BUILTIN_RUN_BG, |vm, _argc| {
        let sub_idx = vm.pop().to_int() as usize;
        let chunk = match vm.chunk.sub_chunks.get(sub_idx).cloned() {
            Some(c) => c,
            None => return Value::Status(1),
        };

        match unsafe { libc::fork() } {
            -1 => Value::Status(1),
            0 => {
                // Child: detach and run.
                unsafe { libc::setsid() };
                crate::fusevm_disasm::maybe_print_stdout("background_job", &chunk);
                let mut bg_vm = fusevm::VM::new(chunk);
                register_builtins(&mut bg_vm);
                let _ = bg_vm.run();
                let _ = std::io::stdout().flush();
                let _ = std::io::stderr().flush();
                std::process::exit(bg_vm.last_status);
            }
            pid => {
                // Parent: record the PID into `$!` (most recent
                // backgrounded job's pid). zsh exposes this for any
                // script that needs `wait $!`. Also register the
                // bare-pid job so a no-args `wait` can synchronize.
                with_executor(|exec| {
                    exec.set_scalar("!".to_string(), pid.to_string());
                    exec.jobs
                        .add_pid_job(pid, String::new(), crate::exec_jobs::JobState::Running);
                });
                Value::Status(0)
            }
        }
    });

    // ── Indexed-array storage ─────────────────────────────────────────────
    //
    // Stack: pushed values then name (LAST). `arr=(a b c)` → 4 args
    // (a, b, c, arr). `arr=($(cmd))` → 2 args (FlatArray, arr).
    //
    // PURE PASSTHRU: pop name + values, dispatch to canonical
    // `setaparam` / `sethparam` (C port of `Src/params.c:3595/3602`).
    // assignaparam already handles PM_UNIQUE dedupe, type-flag flip,
    // PM_NAMEREF rejection, ASSPM_AUGMENT prepend, and createparam
    // for fresh names.
    vm.register_builtin(BUILTIN_SET_ARRAY, |vm, argc| {
        let n = argc as usize;
        let mut popped: Vec<fusevm::Value> = Vec::with_capacity(n);
        for _ in 0..n {
            popped.push(vm.pop());
        }
        popped.reverse();
        if popped.is_empty() {
            return Value::Status(1);
        }
        let name = popped.pop().unwrap().to_str();
        let mut values: Vec<String> = Vec::new();
        for v in popped {
            match v {
                fusevm::Value::Array(items) => {
                    for it in items {
                        values.push(it.to_str());
                    }
                }
                other => values.push(other.to_str()),
            }
        }
        let blocked = with_executor(|exec| {
            // Two-statement assoc init: `typeset -A m; m=(k v k v ...)`.
            // Route to sethparam through the executor — canonical path.
            if exec.assoc(&name).is_some() {
                if !values.len().is_multiple_of(2) {
                    eprintln!("zshrs:1: bad set of key/value pairs for associative array");
                    return true;
                }
                let mut map: IndexMap<String, String> = IndexMap::new();
                let mut it = values.clone().into_iter();
                while let Some(k) = it.next() {
                    if let Some(v) = it.next() {
                        map.insert(k, v);
                    }
                }
                exec.set_assoc(name.clone(), map);
                #[cfg(feature = "recorder")]
                if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                    let ctx = exec.recorder_ctx();
                    let attrs = exec.recorder_attrs_for(&name);
                    let mut pairs: Vec<(String, String)> = Vec::with_capacity(values.len() / 2);
                    let mut iter = values.iter().cloned();
                    while let Some(k) = iter.next() {
                        if let Some(v) = iter.next() {
                            pairs.push((k, v));
                        }
                    }
                    crate::recorder::emit_assoc_assign(&name, pairs, attrs, false, ctx);
                }
                return false;
            }
            // Indexed-array: setaparam handles PM_UNIQUE dedupe and the
            // type-flag flip internally. The tied-array mirror to a
            // PM_TIED scalar (`typeset -T PATH path`) lives canonically
            // in setaparam's setarrvalue dispatch in C zsh; until our
            // assignaparam wires that, mirror here so PATH stays in
            // sync after `path=(/x)`.
            if let Some((scalar_name, sep)) = exec.tied_array_to_scalar.get(&name).cloned() {
                let joined = values.join(&sep);
                exec.set_scalar(scalar_name, joined);
            }
            exec.set_array(name.clone(), values.clone());
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                let ctx = exec.recorder_ctx();
                let attrs = exec.recorder_attrs_for(&name);
                emit_path_or_assign(&name, &values, attrs, false, &ctx);
            }
            false
        });
        Value::Status(if blocked { 1 } else { 0 })
    });
    // `arr+=(d e f)` — array append. Same calling conventions as SET_ARRAY.
    //
    // PURE PASSTHRU shape: pop name + values, dispatch through the
    // canonical assoc / array setter. assignaparam's ASSPM_AUGMENT
    // flag handles the C-source-equivalent "preserve prior value"
    // semantics; for now we read the current array, extend with
    // new values, write through set_array (which routes to
    // setaparam → assignaparam where PM_UNIQUE dedupe lands).
    vm.register_builtin(BUILTIN_APPEND_ARRAY, |vm, argc| {
        let n = argc as usize;
        let mut popped: Vec<fusevm::Value> = Vec::with_capacity(n);
        for _ in 0..n {
            popped.push(vm.pop());
        }
        popped.reverse();
        if popped.is_empty() {
            return Value::Status(1);
        }
        let name = popped.pop().unwrap().to_str();
        let mut values: Vec<String> = Vec::new();
        for v in popped {
            match v {
                fusevm::Value::Array(items) => {
                    for it in items {
                        values.push(it.to_str());
                    }
                }
                other => values.push(other.to_str()),
            }
        }
        with_executor(|exec| {
            // Assoc append: `typeset -A m; m+=(k1 v1 ...)` — merge pairs
            // into the existing map via canonical set_assoc → sethparam.
            if exec.assoc(&name).is_some() {
                let mut map = exec.assoc(&name).unwrap_or_default();
                let mut it = values.into_iter();
                while let Some(k) = it.next() {
                    if let Some(v) = it.next() {
                        map.insert(k, v);
                    }
                }
                exec.set_assoc(name, map);
                return;
            }
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                let ctx = exec.recorder_ctx();
                let attrs = exec.recorder_attrs_for(&name);
                emit_path_or_assign(&name, &values, attrs, true, &ctx);
            }
            // Indexed-array append. Read current via canonical
            // exec.array, extend, write back through set_array →
            // setaparam where assignaparam stamps PM_UNIQUE etc.
            let tied_scalar = exec.tied_array_to_scalar.get(&name).cloned();
            let mut target = exec.array(&name).unwrap_or_default();
            target.extend(values);
            exec.set_array(name.clone(), target);
            if let Some((scalar_name, sep)) = tied_scalar {
                let joined = exec.array(&name).map(|a| a.join(&sep)).unwrap_or_default();
                exec.set_scalar(scalar_name.clone(), joined.clone());
                std::env::set_var(&scalar_name, &joined);
            }
        });
        Value::Status(0)
    });
    vm.register_builtin(BUILTIN_RUN_SELECT, |vm, argc| {
        if argc < 2 {
            return Value::Status(1);
        }
        let n = argc as usize;
        let mut popped: Vec<fusevm::Value> = Vec::with_capacity(n);
        for _ in 0..n {
            popped.push(vm.pop());
        }
        // popped: [sub_idx, name, word_N, ..., word_1] (popping from top)
        let sub_idx_val = popped.remove(0);
        let name_val = popped.remove(0);
        let mut words: Vec<String> = popped.into_iter().rev().map(|v| v.to_str()).collect();
        // Flatten any Value::Array elements (e.g. `select x in $arr; ...`).
        let mut flat = Vec::with_capacity(words.len());
        for w in words.drain(..) {
            // The pop above already to_str()'d, so Array splice is lost. Re-
            // pop wouldn't help — the host receives flat strings here. This is
            // OK for now since the compile path uses ARRAY_FLATTEN-equivalent
            // reasoning before the call. If splice support is needed, the
            // compile path should call BUILTIN_ARRAY_FLATTEN first.
            flat.push(w);
        }
        let words = flat;

        let sub_idx = sub_idx_val.to_int() as usize;
        let name = name_val.to_str();
        let chunk = match vm.chunk.sub_chunks.get(sub_idx).cloned() {
            Some(c) => c,
            None => return Value::Status(1),
        };

        let prompt =
            with_executor(|exec| exec.scalar("PROMPT3").unwrap_or_else(|| "?# ".to_string()));

        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        let mut last_status: i32 = 0;

        loop {
            // Direct port of zsh's selectlist from
            // src/zsh/Src/loop.c:347-409. Layout is column-major
            // ("down columns, then across") — NOT row-major. With
            // 6 items in 3 cols zsh produces:
            //   1  3  5
            //   2  4  6
            // The previous Rust impl walked row-major which
            // produced 1 2 3 / 4 5 6 (visually similar but wrong
            // for prompts that mention ordering and breaks scripts
            // that rely on column count == ceil(N/rows)).
            //
            // C variable mapping:
            //   ct      -> word count (n)
            //   longest -> max item width + 1, then plus digits-of-ct
            //   fct     -> column count
            //   fw      -> per-column width
            //   colsz   -> row count = ceil(ct / fct)
            //   t1      -> row index, walks 0..colsz
            //   ap      -> item pointer; advances by colsz to step
            //              DOWN a column.
            let term_width: usize = std::env::var("COLUMNS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(80);
            let ct = words.len();
            // loop.c:354-363 — find longest item width.
            let mut longest = 1usize;
            for w in &words {
                let aplen = w.chars().count();
                if aplen > longest {
                    longest = aplen;
                }
            }
            // loop.c:365-367 — `longest++` then add digits of `ct`.
            longest += 1;
            let mut t0 = ct;
            while t0 > 0 {
                t0 /= 10;
                longest += 1;
            }
            // loop.c:369-373 — fct = (cols - 1) / (longest + 3); if
            // 0, fct = 1; else fw = (cols - 1) / fct.
            let raw_fct = (term_width.saturating_sub(1)) / (longest + 3);
            let (fct, fw) = if raw_fct == 0 {
                (1, longest + 3)
            } else {
                (raw_fct, (term_width.saturating_sub(1)) / raw_fct)
            };
            // loop.c:374 — colsz = (ct + fct - 1) / fct.
            let colsz = ct.div_ceil(fct);
            // loop.c:375-395 — for each row t1, walk down columns.
            for t1 in 0..colsz {
                let mut ap_idx = t1;
                while ap_idx < ct {
                    let w = &words[ap_idx];
                    let n = ap_idx + 1;
                    let _ = write!(std::io::stderr(), "{}) {}", n, w);
                    let mut t2 = w.chars().count() + 2;
                    let mut t3 = n;
                    while t3 > 0 {
                        t2 += 1;
                        t3 /= 10;
                    }
                    // Pad to fw (loop.c:389-390).
                    while t2 < fw {
                        let _ = write!(std::io::stderr(), " ");
                        t2 += 1;
                    }
                    ap_idx += colsz;
                }
                let _ = writeln!(std::io::stderr());
            }
            let _ = write!(std::io::stderr(), "{}", prompt);
            let _ = std::io::stderr().flush();

            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(_) => {}
                Err(_) => break,
            }
            let trimmed = line.trim_end_matches(['\n', '\r'][..].as_ref()).to_string();

            with_executor(|exec| {
                exec.set_scalar("REPLY".to_string(), trimmed.clone());
            });

            if trimmed.is_empty() {
                // Empty input → redraw menu without running body.
                continue;
            }

            let chosen = match trimmed.parse::<usize>() {
                Ok(n) if n >= 1 && n <= words.len() => words[n - 1].clone(),
                _ => String::new(),
            };

            with_executor(|exec| {
                exec.set_scalar(name.clone(), chosen);
            });

            // Reset the loop signal before running the body so a stale
            // value from a sibling construct doesn't leak in.
            with_executor(|exec| exec.loop_signal = None);

            crate::fusevm_disasm::maybe_print_stdout("select:body", &chunk);
            let mut body_vm = fusevm::VM::new(chunk.clone());
            register_builtins(&mut body_vm);
            let _ = body_vm.run();
            last_status = body_vm.last_status;

            // Drain the cross-VM loop-control signal. `break` from inside
            // the body sets LoopSignal::Break; `continue` sets Continue.
            // The legacy `BREAK_SELECT=1` env-var sentinel is still honored
            // for backward compat with scripts written before the keyword
            // path landed.
            let signal = with_executor(|exec| exec.loop_signal.take());
            let break_legacy = with_executor(|exec| {
                let v = exec.scalar("BREAK_SELECT");
                exec.unset_scalar("BREAK_SELECT");
                v.map(|s| s != "0" && !s.is_empty()).unwrap_or(false)
            });
            match signal {
                Some(LoopSignal::Break) => break,
                Some(LoopSignal::Continue) => continue,
                None if break_legacy => break,
                None => {}
            }
        }

        Value::Status(last_status)
    });

    // Magic special-parameter assoc lookup. Synthesizes values from
    // shell state for zsh's shell-introspection assocs:
    //   commands, aliases, galiases, saliases, dis_aliases, dis_galiases,
    //   dis_saliases, functions, dis_functions, builtins, dis_builtins,
    //   reswords, options, parameters, jobtexts, jobdirs, jobstates,
    //   nameddirs, userdirs, modules.
    // Returns None if `name` isn't a recognized magic name.

    // `${arr[idx]}` — pop name, then idx_str. zsh is 1-based for positive
    // indices; we honor that. `@`/`*` return the whole array as Value::Array
    // so Op::Exec splice produces N argv slots. For `${foo[key]}` where foo
    // is an assoc, the idx is a string key — we check assoc_arrays first
    // when the idx isn't `@`/`*` and the name has an assoc binding.
    // BUILTIN_ARRAY_INDEX — `${name[idx]}` paramsubst dispatch.
    // PURE PASSTHRU: pops the idx + name, hands the canonical
    // `${name[idx]}` form to `subst::paramsubst` (C port of
    // `Src/subst.c::paramsubst`). All subscript-flag dispatch
    // ((I)pat / (R)pat / (i)/(r)/(K)/(k), range slices `[N,M]`,
    // negative indices, magic-assoc shape lookup, DQ-join collapse)
    // lives inside paramsubst → fetchvalue → getarg in params.rs.
    //
    // Sentinel bytes the compile path tags onto `idx` (`\u{02}` =
    // compile-time DQ, `\u{05}` = outer `(@)` flag, `\u{06}` /
    // `\u{07}` = outer `(v)`/`(k)` flag) are stripped here because
    // paramsubst's input must be a valid zsh expression. The
    // context bumps that previously happened here move into the
    // callers (BUILTIN_EXPAND_TEXT bumps `in_dq_context`; the (v)/
    // (k) outer flag is encoded as an actual flag in the outer
    // paramsubst call once that path is wired).
    vm.register_builtin(BUILTIN_ARRAY_INDEX, |vm, _argc| {
        let mut idx = vm.pop().to_str();
        let name = vm.pop().to_str();
        for sentinel in ['\u{02}', '\u{05}', '\u{06}', '\u{07}'] {
            if let Some(rest) = idx.strip_prefix(sentinel) {
                idx = rest.to_string();
            }
        }
        let body = format!("${{{}[{}]}}", name, idx);
        let mut ret_flags: i32 = 0;
        let (_full, _pos, nodes) =
            crate::ported::subst::paramsubst(&body, 0, false, 0i32, &mut ret_flags);
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            with_executor(|exec| exec.set_last_status(1));
        }
        if nodes.len() == 1 {
            fusevm::Value::str(nodes.into_iter().next().unwrap())
        } else {
            fusevm::Value::Array(nodes.into_iter().map(fusevm::Value::str).collect())
        }
    });
    vm.register_builtin(BUILTIN_BRIDGE_BRACE_ARRAY, |vm, _argc| {
        // Inner body of `${(...)...}` (already stripped of `${`/`}` by
        // the caller). Re-wrap and route through subst.rs's paramsubst
        // so the flag-loop + per-operator array semantics
        // (e.g. `(M)arr:#pat`) execute properly. Earlier this returned
        // the body verbatim, which is why `${(M)arr:#pat}` printed as
        // literal text.
        let body = vm.pop().to_str();
        let full = format!("${{{}}}", body);
        let result = with_executor(|exec| {
            let mut ret_flags: i32 = 0;
            let (_full_str, _new_pos, nodes) =
                crate::ported::subst::paramsubst(&full, 0, false, 0i32, &mut ret_flags);
            // c:Src/subst.c errflag bail — propagate to caller's
            // exit status the way `subst_state_commit_to_executor`
            // used to.
            if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
                exec.set_last_status(1);
            }
            nodes
        });
        if result.is_empty() {
            return fusevm::Value::Array(Vec::new());
        }
        if result.len() == 1 {
            return fusevm::Value::str(result.into_iter().next().unwrap());
        }
        fusevm::Value::Array(result.into_iter().map(fusevm::Value::str).collect())
    });

    // BUILTIN_PARAM_FLAG — `${(flags)name}` paramsubst dispatch.
    // PURE PASSTHRU: pops sentinel-tagged flags + name, hands the
    // canonical `${(flags)name}` form to `subst::paramsubst` (C port
    // of `Src/subst.c::paramsubst`). The bridge does no flag
    // walking, no DQ-context branching, no array/scalar shape
    // selection — all of that lives inside paramsubst.
    //
    // Sentinel bytes the compile path tags onto `flags` (`\u{02}` =
    // DQ-wrapped, `\u{03}` = had `[@]`/`[*]` subscript, `\u{04}` =
    // scalar-assignment context) are stripped before the
    // `${(flags)name}` reconstruction so paramsubst's input is a
    // valid zsh expression. The context bumps that previously
    // happened here move into the caller (BUILTIN_EXPAND_TEXT etc.
    // already bump `in_dq_context` for DQ, and `in_scalar_assign`
    // for scalar-assign RHS).
    vm.register_builtin(BUILTIN_PARAM_FLAG, |vm, _argc| {
        let mut flags = vm.pop().to_str();
        let name = vm.pop().to_str();
        // Strip the compile-path sentinels — paramsubst doesn't
        // recognize them and they'd corrupt the flag-block parse.
        for sentinel in ['\u{02}', '\u{03}', '\u{04}'] {
            if let Some(rest) = flags.strip_prefix(sentinel) {
                flags = rest.to_string();
            }
        }
        let body = format!("${{({}){}}}", flags, name);
        let mut ret_flags: i32 = 0;
        let (_full, _pos, nodes) =
            crate::ported::subst::paramsubst(&body, 0, false, 0i32, &mut ret_flags);
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            with_executor(|exec| exec.set_last_status(1));
        }
        if nodes.len() == 1 {
            fusevm::Value::str(nodes.into_iter().next().unwrap())
        } else {
            fusevm::Value::Array(nodes.into_iter().map(fusevm::Value::str).collect())
        }
    });

    // `foo[key]=val` — single-key set on an assoc array. Stack: [name, key, value].
    // PURE PASSTHRU: assignsparam with `name[key]` form (C port of
    // `Src/params.c::assignsparam` subscript path at c:3210-3231)
    // already does the indexed-array vs assoc decision, PM_HASHED
    // auto-vivification, numeric-subscript bounds handling, and
    // PM_READONLY rejection.
    vm.register_builtin(BUILTIN_SET_ASSOC, |vm, _argc| {
        let value = vm.pop().to_str();
        let key = vm.pop().to_str();
        let name = vm.pop().to_str();
        with_executor(|exec| {
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                let ctx = exec.recorder_ctx();
                let attrs = exec.recorder_attrs_for(&name);
                crate::recorder::emit_assoc_assign(
                    &name,
                    vec![(key.clone(), value.clone())],
                    attrs,
                    true,
                    ctx,
                );
            }
            let _ = exec;
        });
        // Build `name[key]=value` shape for assignsparam's subscript
        // dispatch. Arith-evaluate numeric subscripts on an existing
        // indexed array (`a[i+1]=v` form) before handing off — the
        // canonical port currently only handles literal int / string
        // keys, so pre-resolve here.
        let resolved_key = with_executor(|exec| {
            let is_indexed = exec.array(&name).is_some();
            if is_indexed && key.trim().parse::<i64>().is_err() {
                crate::ported::math::mathevali(
                    &crate::ported::subst::singsub(&key),
                )
                .map(|n| n.to_string())
                .unwrap_or(key.clone())
            } else {
                key.clone()
            }
        });
        let subscripted = format!("{}[{}]", name, resolved_key);
        crate::ported::params::assignsparam(
            &subscripted,
            &value,
            crate::ported::zsh_h::ASSPM_WARN,
        );
        Value::Status(0)
    });

    // Brace expansion. Routes through executor.xpandbraces (already
    // implemented for the pre-fusevm executor). Returns Value::Array.
    // BUILTIN_WORD_SPLIT — `${=var}` IFS-split runtime.
    // PURE PASSTHRU: route through canonical `subst::multsub` with
    // PREFORK_SPLIT flag (C port of `Src/subst.c::multsub` at c:544
    // — the IFS-split walker with whitespace-vs-non-whitespace
    // gating, quote-aware parsing, and empty-field handling).
    vm.register_builtin(BUILTIN_WORD_SPLIT, |vm, _argc| {
        let s = vm.pop().to_str();
        let (_joined, parts, _isarr, _flags) = crate::ported::subst::multsub(
            &s,
            crate::ported::zsh_h::PREFORK_SPLIT,
        );
        if parts.is_empty() || (parts.len() == 1 && parts[0].is_empty()) {
            fusevm::Value::Array(Vec::new())
        } else if parts.len() == 1 {
            fusevm::Value::str(parts.into_iter().next().unwrap())
        } else {
            fusevm::Value::Array(parts.into_iter().map(fusevm::Value::str).collect())
        }
    });

    vm.register_builtin(BUILTIN_BRACE_EXPAND, |vm, _argc| {
        let s = vm.pop().to_str();
        // Direct call to the canonical brace expander (port of
        // Src/glob.c::xpandbraces at glob.rs:1678). Was stubbed
        // as `vec![s]` — every `print X{1,2,3}Y` returned literal.
        let brace_ccl = with_executor(|exec| {
            crate::ported::options::opt_state_get("braceccl").unwrap_or(false)
        });
        let parts = crate::ported::glob::xpandbraces(&s, brace_ccl);
        if parts.len() == 1 {
            fusevm::Value::str(parts.into_iter().next().unwrap_or_default())
        } else {
            fusevm::Value::Array(parts.into_iter().map(fusevm::Value::str).collect())
        }
    });

    // `*(qual)` glob qualifier filter. Stack: [pattern, qualifier].
    // Pattern is glob-expanded normally, then each result is filtered by the
    // qualifier predicate. Common qualifiers:
    //   .  — regular files only
    //   /  — directories only
    //   @  — symlinks
    //   x  — executable
    //   r/w/x — readable/writable/executable
    //   N  — nullglob (no error if no match)
    //   L+N / L-N — size > N / size < N (in bytes)
    //   mh-N / mh+N — modified within N hours / older than N hours
    //   md-N / md+N — modified within N days / older than N days
    //   on/On — sort by name asc/desc (default)
    //   oL/OL — sort by length
    //   om/Om — sort by mtime
    // Pop a scalar pattern, run expand_glob, push Value::Array. Used
    // by the segment-concat compile path for `$D/*`-style words.
    vm.register_builtin(BUILTIN_GLOB_EXPAND, |vm, _argc| {
        let pattern = vm.pop().to_str();
        let matches = with_executor(|exec| exec.expand_glob(&pattern));
        if matches.is_empty() {
            // expand_glob handles NOMATCH internally; if it returns
            // empty here, nullglob was on. Yield empty array.
            return fusevm::Value::Array(Vec::new());
        }
        if matches.len() == 1 && matches[0] == pattern {
            // No real matches; expand_glob returned the literal. Pass
            // back as scalar so downstream ops don't re-flatten.
            return fusevm::Value::str(pattern);
        }
        fusevm::Value::Array(matches.into_iter().map(fusevm::Value::str).collect())
    });

    // `break`/`continue` from a sub-VM body. The compile path emits these
    // when the keyword appears at chunk top-level (no enclosing for/while in
    // the current chunk's patch lists). Outer-loop builtins (BUILTIN_RUN_
    // SELECT and any future loop-via-builtin construct) drain
    // executor.loop_signal after each iteration.
    vm.register_builtin(BUILTIN_SET_BREAK, |_vm, _argc| {
        with_executor(|exec| {
            exec.loop_signal = Some(LoopSignal::Break);
        });
        Value::Status(0)
    });
    vm.register_builtin(BUILTIN_SET_CONTINUE, |_vm, _argc| {
        with_executor(|exec| {
            exec.loop_signal = Some(LoopSignal::Continue);
        });
        Value::Status(0)
    });



    // `${arr[*]}` — join array elements with the first IFS char into
    // a single string. Matches zsh: in DQ context this preserves the
    // join; in array context too the result is one Value::Str.
    // Set or clear a shell option directly. Used by `noglob CMD ...`
    // precommand wrapping — the compiler emits SET_RAW_OPT to flip the
    // option ON before compiling the inner words and OFF after, so glob
    // expansion of the inner args sees the temporary state.
    vm.register_builtin(BUILTIN_SET_RAW_OPT, |vm, _argc| {
        let on = vm.pop().to_int() != 0;
        let opt = vm.pop().to_str();
        with_executor(|exec| {
            if on {
                crate::ported::options::opt_state_set(&opt, true);
            } else {
                crate::ported::options::opt_state_unset(&opt);
            }
        });
        Value::Status(0)
    });

    vm.register_builtin(BUILTIN_ARRAY_JOIN_STAR, |vm, _argc| {
        let name = vm.pop().to_str();
        let result = with_executor(|exec| {
            let sep = exec
                .scalar("IFS")
                .and_then(|s| s.chars().next())
                .map(|c| c.to_string())
                .unwrap_or_else(|| " ".to_string());
            if name == "@" || name == "*" || name == "argv" {
                return exec.pparams().join(&sep);
            }
            if let Some(arr) = exec.array(&name) {
                arr.join(&sep)
            } else {
                exec.get_variable(&name)
            }
        });
        fusevm::Value::str(result)
    });

    vm.register_builtin(BUILTIN_ARRAY_ALL, |vm, _argc| {
        let name = vm.pop().to_str();
        with_executor(|exec| {
            // Special positional names — splice the positional list.
            if name == "@" || name == "*" || name == "argv" {
                return Value::Array(exec.pparams().iter().map(Value::str).collect());
            }
            match exec.array(&name) {
                Some(v) => Value::Array(v.iter().map(Value::str).collect()),
                None => {
                    // Fall back to scalar lookup. zsh (unlike bash)
                    // does NOT IFS-split a scalar variable in a for
                    // list — `for w in $scalar` iterates ONCE with the
                    // scalar value. Word-splitting requires either
                    // sh_word_split option or explicit `${(s.,.)scalar}`.
                    let val = exec.get_variable(&name);
                    if val.is_empty() && !exec.has_scalar(&name) && std::env::var(&name).is_err() {
                        Value::Array(vec![])
                    } else if crate::ported::options::opt_state_get("shwordsplit").unwrap_or(false)
                    {
                        // bash-compat: under setopt sh_word_split, do
                        // split scalars on IFS chars.
                        let ifs = exec.scalar("IFS").unwrap_or_else(|| " \t\n".to_string());
                        let parts: Vec<Value> = val
                            .split(|c: char| ifs.contains(c))
                            .filter(|s| !s.is_empty())
                            .map(Value::str)
                            .collect();
                        Value::Array(parts)
                    } else {
                        Value::Array(vec![Value::str(val)])
                    }
                }
            }
        })
    });

    // BUILTIN_ARRAY_FLATTEN(N): pops N values, flattens one level of Array
    // nesting, pushes the resulting Array AND its length as a separate Int.
    // The two-value return shape lets the caller (for-loop compile path)
    // SetSlot the length before SetSlot'ing the array, without re-deriving
    // the length from the array via a second builtin call.
    // `coproc [name] { body }` — bidirectional pipe to backgrounded body.
    // Stack discipline (top first): [name (str, "" for default), sub_idx (int)].
    // On success: parent's `executor.arrays[name]` becomes [write_fd, read_fd]
    // and Status(0) is returned. The caller writes to the child's stdin via
    // write_fd, reads its stdout via read_fd, and closes both when done.
    //
    // Bash's coproc convention is `${NAME[0]}` = read_fd, `${NAME[1]}` =
    // write_fd. We follow that: arrays[name] = [read_fd_str, write_fd_str].
    vm.register_builtin(BUILTIN_RUN_COPROC, |vm, _argc| {
        let sub_idx = vm.pop().to_int() as usize;
        let raw_name = vm.pop().to_str();
        let name = if raw_name.is_empty() {
            "COPROC".to_string()
        } else {
            raw_name
        };
        let chunk = match vm.chunk.sub_chunks.get(sub_idx).cloned() {
            Some(c) => c,
            None => return Value::Status(1),
        };

        // (parent_read ← child_stdout)
        let mut p2c = [0i32; 2]; // parent writes, child reads
        let mut c2p = [0i32; 2]; // child writes, parent reads
        if unsafe { libc::pipe(p2c.as_mut_ptr()) } < 0 {
            return Value::Status(1);
        }
        if unsafe { libc::pipe(c2p.as_mut_ptr()) } < 0 {
            unsafe {
                libc::close(p2c[0]);
                libc::close(p2c[1]);
            }
            return Value::Status(1);
        }

        match unsafe { libc::fork() } {
            -1 => {
                unsafe {
                    libc::close(p2c[0]);
                    libc::close(p2c[1]);
                    libc::close(c2p[0]);
                    libc::close(c2p[1]);
                }
                Value::Status(1)
            }
            0 => {
                // Child: stdin from p2c[0], stdout to c2p[1]. Close all
                // unused fds. setsid so SIGINT to fg doesn't hit us.
                unsafe {
                    libc::dup2(p2c[0], libc::STDIN_FILENO);
                    libc::dup2(c2p[1], libc::STDOUT_FILENO);
                    libc::close(p2c[0]);
                    libc::close(p2c[1]);
                    libc::close(c2p[0]);
                    libc::close(c2p[1]);
                    libc::setsid();
                }
                crate::fusevm_disasm::maybe_print_stdout("coproc:child", &chunk);
                let mut co_vm = fusevm::VM::new(chunk);
                register_builtins(&mut co_vm);
                let _ = co_vm.run();
                let _ = std::io::stdout().flush();
                let _ = std::io::stderr().flush();
                std::process::exit(co_vm.last_status);
            }
            _pid => {
                // Parent: close child ends, store [read_fd, write_fd] in NAME.
                unsafe {
                    libc::close(p2c[0]);
                    libc::close(c2p[1]);
                }
                let read_fd = c2p[0];
                let write_fd = p2c[1];
                with_executor(|exec| {
                    exec.unset_scalar(&name);
                    exec.set_array(name, vec![read_fd.to_string(), write_fd.to_string()]);
                });
                Value::Status(0)
            }
        }
    });

    vm.register_builtin(BUILTIN_ARRAY_FLATTEN, |vm, argc| {
        let n = argc as usize;
        let start = vm.stack.len().saturating_sub(n);
        let raw: Vec<fusevm::Value> = vm.stack.drain(start..).collect();
        let mut flat: Vec<fusevm::Value> = Vec::with_capacity(raw.len());
        for v in raw {
            match v {
                fusevm::Value::Array(items) => flat.extend(items),
                other => flat.push(other),
            }
        }
        let len = flat.len() as i64;
        // Push the array first; the Int(len) becomes the builtin's return
        // value (which CallBuiltin already pushes). Caller consumes in
        // reverse: SetSlot(len_slot) pops Int, SetSlot(arr_slot) pops Array.
        vm.push(fusevm::Value::Array(flat));
        fusevm::Value::Int(len)
    });

    // Shell variable get/set — routes through executor.variables so nested
    // VMs (function calls) and tree-walker callers see the same storage.
    vm.register_builtin(BUILTIN_GET_VAR, |vm, argc| {
        let args = pop_args(vm, argc);
        let name = args.into_iter().next().unwrap_or_default();
        let live_status = vm.last_status;
        // `$@` and `$*` need splice semantics — return Value::Array of
        // positional params so for-loop's BUILTIN_ARRAY_FLATTEN spreads them
        // and pop_args splits them into argv slots. zsh's `"$@"` bslashquote-each-
        // word semantics matches: each pos-param becomes its own arg.
        // Same for arrays accessed by name (e.g. `$arr` in some contexts).
        let sync_status = |exec: &mut ShellExecutor| {
            exec.set_last_status(live_status);
        };
        if name == "@" || name == "*" {
            return with_executor(|exec| {
                sync_status(exec);
                fusevm::Value::Array(exec.pparams().iter().map(fusevm::Value::str).collect())
            });
        }
        // RC_EXPAND_PARAM: when the option is set and `name` refers to
        // an array, return Value::Array so the enclosing word's
        // BUILTIN_CONCAT_DISTRIBUTE distributes element-wise. Without
        // the option, arrays still join to a space-separated scalar
        // (zsh's default unquoted-array-as-scalar semantics).
        let rc_expand = with_executor(|exec| {
            crate::ported::options::opt_state_get("rcexpandparam").unwrap_or(false)
        });
        if rc_expand {
            let arr_val = with_executor(|exec| {
                sync_status(exec);
                exec.array(&name)
            });
            if let Some(arr) = arr_val {
                return fusevm::Value::Array(arr.into_iter().map(fusevm::Value::str).collect());
            }
        }
        // Magic-assoc fallback FIRST — `${aliases}` / `${functions}`
        // / `${commands}` / etc. should return the value list per
        // zsh's bare-assoc semantics. Without this, those names fell
        // through to `get_variable` which is empty (they live in
        // separate executor tables, not `assoc_arrays`). Return as
        // a Value::Array so `arr=(${aliases})` distributes into
        // multiple elements, matching zsh's array-context word
        // splitting for assoc-bare references.
        let magic_vals = with_executor(|exec| {
            sync_status(exec);
            crate::vm_helper::scan_magic_assoc_keys(&name).map(|keys| {
                keys.iter()
                    .map(|k| exec.get_special_array_value(&name, k).unwrap_or_default())
                    .collect::<Vec<_>>()
            })
        });
        if let Some(vals) = magic_vals {
            // Distinguish "name IS a magic-assoc with no entries"
            // (return Array(empty)) from "name is unknown — fall
            // through to get_variable".
            return fusevm::Value::Array(vals.into_iter().map(fusevm::Value::str).collect());
        }
        // Indexed-array path: return Value::Array so pop_args splats
        // each element into its own argv slot. Direct port of zsh's
        // unquoted `$arr` semantics — each element becomes a separate
        // word in command-arg position.
        //
        // DQ context exception: inside `"...$arr..."`, zsh joins with
        // the first char of $IFS (default space) so the DQ word stays
        // a single argv slot. Detect via in_dq_context (bumped by
        // BUILTIN_EXPAND_TEXT mode 1) and return the joined scalar.
        // Direct port of Src/subst.c:1759-1813 nojoin/sepjoin: in DQ
        // (qt=1) without explicit `(@)`, sepjoin runs and the result
        // is one word.
        let arr_assoc_data = with_executor(|exec| {
            sync_status(exec);
            let in_dq = exec.in_dq_context > 0;
            // KSH_ARRAYS: bare `$arr` returns ONLY arr[0] (zero-
            // based first-element-only semantics). Direct port of
            // Src/params.c getstrvalue's KSH_ARRAYS gate which
            // returns aval[0] instead of the whole array.
            let ksh_arrays = crate::ported::options::opt_state_get("ksharrays").unwrap_or(false);
            if let Some(arr) = exec.array(&name) {
                if ksh_arrays {
                    return Some((vec![arr.first().cloned().unwrap_or_default()], in_dq));
                }
                return Some((arr.clone(), in_dq));
            }
            if let Some(map) = exec.assoc(&name) {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let values: Vec<String> =
                    keys.iter().filter_map(|k| map.get(*k).cloned()).collect();
                if ksh_arrays {
                    return Some((vec![values.into_iter().next().unwrap_or_default()], in_dq));
                }
                return Some((values, in_dq));
            }
            None
        });
        if let Some((items, in_dq)) = arr_assoc_data {
            if in_dq {
                let sep = with_executor(|exec| {
                    exec.get_variable("IFS")
                        .chars()
                        .next()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| " ".to_string())
                });
                return Value::str(items.join(&sep));
            }
            return fusevm::Value::Array(items.into_iter().map(fusevm::Value::str).collect());
        }
        let (val, in_dq) = with_executor(|exec| {
            sync_status(exec);
            (exec.get_variable(&name), exec.in_dq_context > 0)
        });
        // Empty unquoted scalar → drop the arg (zsh "remove empty
        // unquoted words" rule). Returning empty Value::Array makes
        // pop_args contribute zero items. DQ context keeps the empty
        // string so "$a" stays a single empty arg. Direct port of
        // subst.c's elide-empty pass.
        if val.is_empty() && !in_dq {
            return fusevm::Value::Array(Vec::new());
        }
        Value::str(val)
    });

    // `name+=val` (no parens) — runtime dispatch:
    //   - if `name` is in `arrays` → push `val` as new element
    //   - if `name` is in `assoc_arrays` → refuse (zsh errors here)
    //   - else → scalar concat (existing behavior)
    // Stack: [name, value].
    vm.register_builtin(BUILTIN_APPEND_SCALAR_OR_PUSH, |vm, argc| {
        let args = pop_args(vm, argc);
        let mut iter = args.into_iter();
        let name = iter.next().unwrap_or_default();
        let value = iter.next().unwrap_or_default();
        with_executor(|exec| {
            if let Some(mut arr) = exec.array(&name) {
                arr.push(value.clone());
                exec.set_array(name.clone(), arr);
                // PFA-SMR aspect: `name+=elem` array push (scalar form
                // resolved to existing indexed array). is_append=true.
                #[cfg(feature = "recorder")]
                if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                    let ctx = exec.recorder_ctx();
                    let attrs = exec.recorder_attrs_for(&name);
                    emit_path_or_assign(&name, std::slice::from_ref(&value), attrs, true, &ctx);
                }
                return;
            }
            if exec.assoc(&name).is_some() {
                eprintln!("zshrs: {}: cannot use += on assoc without (key val)", name);
                return;
            }
            // typeset -i: `+=` is arithmetic add, not string concat.
            // `typeset -i x=42; x+=8` must store 50, not "428". Per
            // Src/params.c assignsparam:3270-3293, the PM_TYPE switch
            // routes integer/float through matheval. Read PM_INTEGER
            // from the canonical Param flags.
            let is_integer = exec.is_integer_param(&name);
            if is_integer {
                let prev = exec.get_variable(&name);
                let prev_n: i64 = prev.parse().unwrap_or(0);
                let added = crate::ported::math::mathevali(&crate::ported::subst::singsub(&value))
                    .unwrap_or(0);
                let new_val = (prev_n + added).to_string();
                exec.set_scalar(name.clone(), new_val.clone());
                // PFA-SMR aspect: integer-typed append. The append
                // operator is arithmetic; replay should restore the
                // POST-add value so the bundle reflects end state.
                #[cfg(feature = "recorder")]
                if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                    let ctx = exec.recorder_ctx();
                    let attrs = exec.recorder_attrs_for(&name);
                    crate::recorder::emit_assign_typed(&name, &new_val, attrs, ctx);
                }
                return;
            }
            // Scalar concat.
            let prev = exec.get_variable(&name);
            let combined = format!("{}{}", prev, value);
            exec.set_scalar(name.clone(), combined.clone());
            // PFA-SMR aspect: scalar concat (`PATH+=":/foo"` and any
            // other `NAME+=tail` shape). For PATH-family scalars the
            // path-or-assign helper still emits a path_mod with the
            // FULL post-concat value so replay knows the end state.
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                let ctx = exec.recorder_ctx();
                let attrs = exec.recorder_attrs_for(&name);
                let lower = name.to_ascii_lowercase();
                if matches!(
                    lower.as_str(),
                    "path" | "fpath" | "manpath" | "module_path" | "cdpath"
                ) {
                    emit_path_or_assign(&name, std::slice::from_ref(&combined), attrs, true, &ctx);
                } else {
                    crate::recorder::emit_assign_typed(&name, &combined, attrs, ctx);
                }
            }
        });
        Value::Status(0)
    });

    // BUILTIN_SET_VAR — `name=value` runtime scalar assignment.
    // PURE PASSTHRU: hand to canonical `setsparam` (C port of
    // `Src/params.c::setsparam`). That walks assignsparam →
    // assignstrvalue which already does:
    //   - readonly rejection (zerr + errflag at c:2701)
    //   - PM_INTEGER math evaluation (mathevali at c:3590)
    //   - PM_EFLOAT / PM_FFLOAT float coercion (c:3608)
    //   - PM_LOWER / PM_UPPER case fold (via setstrvalue)
    //   - GSU special-param dispatch (homesetfn / ifssetfn / etc.)
    //   - allexport env mirror via the PM_EXPORTED setfn
    //
    // Bridge-only concerns kept here:
    //   - inline_env_stack (zsh `X=foo cmd` scoped env)
    //   - recorder emission (PFA-SMR)
    //   - vm.last_status propagation for `a=$(cmd)` exit-code chaining
    vm.register_builtin(BUILTIN_SET_VAR, |vm, argc| {
        let args = pop_args(vm, argc);
        let mut iter = args.into_iter();
        let name = iter.next().unwrap_or_default();
        let value = iter.next().unwrap_or_default();
        with_executor(|exec| {
            // Inline-assignment frame tracking (`X=foo cmd` reverts on
            // command return).
            if !exec.inline_env_stack.is_empty() {
                let prev_var = crate::ported::params::getsparam(&name);
                let prev_env = std::env::var(&name).ok();
                exec.inline_env_stack
                    .last_mut()
                    .unwrap()
                    .push((name.clone(), prev_var, prev_env));
                std::env::set_var(&name, &value);
            }
            // Canonical setsparam handles readonly, integer math, case
            // fold, GSU dispatch.
            crate::ported::params::setsparam(&name, &value);
            // PM_EXPORTED / allexport env mirror — read AFTER setsparam
            // so the flag bit reflects any GSU setfn side-effects.
            let allexport =
                crate::ported::options::opt_state_get("allexport").unwrap_or(false);
            let already_exported = (exec.param_flags(&name) as u32
                & crate::ported::zsh_h::PM_EXPORTED)
                != 0;
            if allexport || already_exported {
                std::env::set_var(&name, &value);
            }
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled()
                && exec.local_scope_depth == 0
                && !matches!(
                    name.as_str(),
                    "PPID" | "LINENO" | "ZSH_ARGZERO" | "argv0" | "ARGC"
                    | "?" | "_" | "RANDOM"
                )
            {
                let ctx = exec.recorder_ctx();
                let attrs = exec.recorder_attrs_for(&name);
                crate::recorder::emit_assign_typed(&name, &value, attrs, ctx);
            }
        });
        Value::Status(vm.last_status)
    });

    // BUILTIN_REGISTER_FUNCTION (id 282) was a legacy JSON-AST body
    // bridge. ZshCompiler emits BUILTIN_REGISTER_COMPILED_FN (id 305)
    // instead, which carries a base64 bincode of an already-compiled
    // Chunk. The constant + handler are removed; the ID stays reserved.

    // Pre-compiled function registration — used by compile_zsh.rs's
    // FuncDef path. Stack: [name, base64-bincode-of-Chunk]. We decode
    // the base64, deserialize the Chunk, and store directly in
    // executor.functions_compiled. Bypasses the ShellCommand JSON layer.
    // BUILTIN_VAR_EXISTS — `[[ -v name ]]` set-test.
    // PURE PASSTHRU: build `${+name}` and route through canonical
    // `subst::paramsubst` which returns "1" for set / "0" for unset
    // (C port of `Src/subst.c::paramsubst` plus-prefix arm).
    // paramsubst handles all the shapes the 48-line hand-roll did:
    //   - bare scalar / array / assoc
    //   - subscripted `a[N]` / `h[key]`
    //   - positional params (any digit-only name)
    //   - env-var fallback (`HOME` set via getsparam → lookup_special_var)
    vm.register_builtin(BUILTIN_VAR_EXISTS, |vm, _argc| {
        let name = vm.pop().to_str();
        let body = format!("${{+{}}}", name);
        let mut ret_flags: i32 = 0;
        let (_full, _pos, nodes) =
            crate::ported::subst::paramsubst(&body, 0, false, 0i32, &mut ret_flags);
        let result = nodes.into_iter().next().unwrap_or_default();
        fusevm::Value::Bool(result == "1")
    });

    // `time { compound; ... }` — runs the sub-chunk and prints elapsed
    // wall-clock time. zsh's full `time` also tracks user/system CPU via
    // getrusage on the *child*; we approximate via wall-time only since
    // the sub-chunk runs in-process (no fork). Output format matches
    // `time simple-cmd` (already implemented elsewhere via exectime).
    vm.register_builtin(BUILTIN_TIME_SUBLIST, |vm, _argc| {
        let sub_idx = vm.pop().to_int() as usize;
        let chunk_opt = vm.chunk.sub_chunks.get(sub_idx).cloned();
        let Some(chunk) = chunk_opt else {
            return Value::Status(0);
        };
        let start = Instant::now();
        crate::fusevm_disasm::maybe_print_stdout("time_sublist", &chunk);
        let mut sub_vm = fusevm::VM::new(chunk);
        register_builtins(&mut sub_vm);
        let _ = sub_vm.run();
        let status = sub_vm.last_status;
        let elapsed = start.elapsed();
        eprintln!(
            "{:.2}s user {:.2}s system {:.0}% cpu {:.3} total",
            elapsed.as_secs_f64() * 0.7,
            elapsed.as_secs_f64() * 0.1,
            ((elapsed.as_secs_f64() * 0.8) / elapsed.as_secs_f64() * 100.0).min(100.0),
            elapsed.as_secs_f64()
        );
        Value::Status(status)
    });

    // `{name}>file` / `{name}<file` / `{name}>>file` — named-fd allocator.
    // Stack: [path, varid, op_byte]. Opens path with the appropriate mode
    // and stores the resulting fd number in $varid as a string. We use
    // a high starting fd (10+) by allocating then dup'ing — matches zsh's
    // "fresh fd >= 10" promise so subsequent commands don't collide on
    // stdin/out/err.
    vm.register_builtin(BUILTIN_OPEN_NAMED_FD, |vm, _argc| {
        let op_byte = vm.pop().to_int() as u8;
        let varid = vm.pop().to_str();
        let path = vm.pop().to_str();
        let path_c = match std::ffi::CString::new(path.clone()) {
            Ok(c) => c,
            Err(_) => return Value::Status(1),
        };
        let flags = match op_byte {
            b if b == fusevm::op::redirect_op::READ => libc::O_RDONLY,
            b if b == fusevm::op::redirect_op::WRITE || b == fusevm::op::redirect_op::CLOBBER => {
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC
            }
            b if b == fusevm::op::redirect_op::APPEND => {
                libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND
            }
            b if b == fusevm::op::redirect_op::READ_WRITE => libc::O_RDWR | libc::O_CREAT,
            _ => return Value::Status(1),
        };
        let fd = unsafe { libc::open(path_c.as_ptr(), flags, 0o644) };
        if fd < 0 {
            return Value::Status(1);
        }
        // Re-dup to fd >= 10 so positional fds (0/1/2/etc.) stay free.
        let new_fd = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
        let final_fd = if new_fd >= 10 {
            unsafe { libc::close(fd) };
            new_fd
        } else {
            fd
        };
        with_executor(|exec| {
            exec.set_scalar(varid, final_fd.to_string());
        });
        Value::Status(0)
    });

    // BUILTIN_SET_TRY_BLOCK_ERROR — capture the try-block's exit status
    // into $TRY_BLOCK_ERROR so the always-arm can read it.
    vm.register_builtin(BUILTIN_SET_TRY_BLOCK_ERROR, |vm, _argc| {
        let vm_status = vm.last_status;
        with_executor(|exec| {
            exec.set_scalar("TRY_BLOCK_ERROR".to_string(), vm_status.to_string());
        });
        fusevm::Value::Status(0)
    });

    // BUILTIN_BEGIN_INLINE_ENV / END_INLINE_ENV — wrap an
    // inline-assignment-prefixed command (`X=foo Y=bar cmd`):
    // BEGIN pushes a save frame; SET_VAR fires for each assign and
    // ALSO env::set_var's the value (visible to cmd's child); the
    // command runs; END pops the frame and restores both shell-var
    // and process-env state. Direct port of zsh's addvars() →
    // execute_simple → restore-after-exec contract.
    vm.register_builtin(BUILTIN_BEGIN_INLINE_ENV, |_vm, _argc| {
        with_executor(|exec| {
            exec.inline_env_stack.push(Vec::new());
        });
        fusevm::Value::Status(0)
    });
    vm.register_builtin(BUILTIN_END_INLINE_ENV, |_vm, _argc| {
        with_executor(|exec| {
            if let Some(frame) = exec.inline_env_stack.pop() {
                for (name, prev_var, prev_env) in frame.into_iter().rev() {
                    match prev_var {
                        Some(v) => {
                            exec.set_scalar(name.clone(), v);
                        }
                        None => {
                            exec.unset_scalar(&name);
                        }
                    }
                    match prev_env {
                        Some(v) => std::env::set_var(&name, &v),
                        None => std::env::remove_var(&name),
                    }
                }
            }
        });
        fusevm::Value::Status(0)
    });

    // BUILTIN_RESTORE_TRY_BLOCK_STATUS — emitted at the end of an
    // `always` arm. Per zshmisc, the exit status of the entire
    // `{ try } always { finally }` construct is the try-list's
    // status, regardless of what happens in the always-list (the
    // exception is `return`/`exit` inside always, which short-
    // circuits and the cleanup is the only thing that runs). So
    // restore TRY_BLOCK_ERROR unconditionally — the always-list's
    // exit status is discarded for the construct.
    vm.register_builtin(BUILTIN_RESTORE_TRY_BLOCK_STATUS, |_vm, _argc| {
        let try_status = with_executor(|exec| {
            exec.scalar("TRY_BLOCK_ERROR")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0)
        });
        fusevm::Value::Status(try_status)
    });

    vm.register_builtin(BUILTIN_IS_TTY, |vm, _argc| {
        let fd_str = vm.pop().to_str();
        let fd: i32 = fd_str.trim().parse().unwrap_or(-1);
        let is_tty = if fd < 0 {
            false
        } else {
            unsafe { libc::isatty(fd) != 0 }
        };
        fusevm::Value::Bool(is_tty)
    });

    // Set $LINENO before executing the next statement. Direct
    // port of zsh's `lineno` global tracking from Src/input.c
    // (`if ((inbufflags & INP_LINENO) || !strin) && c == '\n')
    // lineno++;`). The compiler emits one of these before each
    // top-level pipe in `compile_sublist`, carrying the line
    // number captured by the parser at `ZshPipe.lineno`. Pops
    // [n], updates `$LINENO` in the variable table.
    vm.register_builtin(BUILTIN_SET_LINENO, |vm, _argc| {
        let n = vm.pop().to_int();
        with_executor(|exec| {
            exec.set_scalar("LINENO".to_string(), n.to_string());
        });
        // Mirror to the file-static `lineno` (utils.c:121) that
        // zerrmsg reads at utils.c:301 for the `:N: msg` prefix.
        crate::ported::utils::set_lineno(n as i32);
        fusevm::Value::Status(0)
    });

    // Direct port of Src/prompt.c:1623 cmdpush. Token is a `CS_*`
    // value (zsh.h:2775-2806) emitted by compile_zsh around each
    // compound command (if/while/[[…]]/((…))/$(…)) and consumed by
    // `%_` in PS4 / prompt expansion.
    vm.register_builtin(BUILTIN_CMD_PUSH, |vm, _argc| {
        let token = vm.pop().to_int() as u8;
        // Route through canonical cmdpush (Src/prompt.c:1623). The
        // prompt expander reads from the file-static `CMDSTACK` at
        // `prompt.rs:2006`, not `exec.cmd_stack` — without this,
        // `%_` in PS4 saw an empty stack during xtrace.
        if (token as i32) < crate::ported::zsh_h::CS_COUNT {
            crate::ported::prompt::cmdpush(token);
        }
        // Canonical `cmdpush()` above already mirrors into the
        // `prompt::CMDSTACK` thread_local (Src/prompt.c:1620). The
        // legacy `exec.cmd_stack` mirror is gone.
        let _ = token;
        fusevm::Value::Status(0)
    });

    // Direct port of Src/prompt.c:1631 cmdpop.
    vm.register_builtin(BUILTIN_CMD_POP, |_vm, _argc| {
        crate::ported::prompt::cmdpop();
        fusevm::Value::Status(0)
    });

    vm.register_builtin(BUILTIN_OPTION_SET, |vm, _argc| {
        let name = vm.pop().to_str();
        // Direct port of `optison(char *name, char *s)` at Src/cond.c:502 — `[[ -o NAME ]]`
        // reads through the same `opts[]` array that `setopt NAME`
        // writes via `dosetopt`. Earlier code read a duplicate Executor
        // HashMap which never saw `bin_setopt`'s writes (those land in
        // `OPTS_LIVE` via `opt_state_set`). Routing through the canonical
        // C port restores the single-store invariant: one `opts[]`,
        // shared between setopt/unsetopt and `[[ -o ]]`.
        let r = crate::ported::cond::optison("test", &name); // c:cond.c:502
        match r {
            0 => fusevm::Value::Bool(true),  // c:cond.c:520 set
            1 => fusevm::Value::Bool(false), // c:cond.c:518/520 unset
            _ => {
                // c:cond.c:514 — unknown option: zwarnnam emitted by
                // optison itself when POSIXBUILTINS is unset; mirror to
                // stderr here for parity with the earlier diagnostic.
                eprintln!("zshrs:1: no such option: {}", name);
                fusevm::Value::Bool(false)
            }
        }
    });

    // BUILTIN_PARAM_FILTER — `${var:#pat}` / `${var:|name}` etc.
    // PURE PASSTHRU: rebuild `${name:#pat}` and route to paramsubst.
    vm.register_builtin(BUILTIN_PARAM_FILTER, |vm, _argc| {
        let pattern = vm.pop().to_str();
        let name = vm.pop().to_str();
        let body = format!("${{{}:#{}}}", name, pattern);
        let mut ret_flags: i32 = 0;
        let (_full, _pos, nodes) =
            crate::ported::subst::paramsubst(&body, 0, false, 0i32, &mut ret_flags);
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            with_executor(|exec| exec.set_last_status(1));
        }
        if nodes.is_empty() {
            fusevm::Value::Array(Vec::new())
        } else if nodes.len() == 1 {
            fusevm::Value::str(nodes.into_iter().next().unwrap())
        } else {
            fusevm::Value::Array(nodes.into_iter().map(fusevm::Value::str).collect())
        }
    });

    // `a[i]=(elements)` / `a[i,j]=(elements)` / `a[i]=()`
    // — subscripted-array assign with array RHS. Stack pushed by
    // compile_assign as: [elem0, elem1, …, elemN-1, name, key].
    vm.register_builtin(BUILTIN_SET_SUBSCRIPT_RANGE, |vm, argc| {
        let n = argc as usize;
        let mut popped: Vec<fusevm::Value> = Vec::with_capacity(n);
        for _ in 0..n {
            popped.push(vm.pop());
        }
        popped.reverse();
        if popped.len() < 2 {
            return fusevm::Value::Status(1);
        }
        let key = popped.pop().unwrap().to_str();
        let name = popped.pop().unwrap().to_str();
        let mut values: Vec<String> = Vec::new();
        for v in popped {
            match v {
                fusevm::Value::Array(items) => {
                    for it in items {
                        values.push(it.to_str());
                    }
                }
                other => values.push(other.to_str()),
            }
        }
        with_executor(|exec| {
            // Read paramtab-first, mutate, write back via canonical
            // set_array so subscript-slice/index assignments are
            // visible to both the paramtab single source and the
            // legacy cache.
            let mut arr = exec.array(&name).unwrap_or_default();
            // Slice form `a[i,j]=(values)` — replace the inclusive
            // slice. Negative bounds count from end. Out-of-range high
            // bound clamps to len; low bound below 1 clamps to 1.
            if let Some((s_str, e_str)) = key.split_once(',') {
                let len = arr.len() as i64;
                let resolve = |s: &str| -> i64 { s.trim().parse::<i64>().unwrap_or_default() };
                let s_raw = resolve(s_str);
                let e_raw = resolve(e_str);
                let lo = if s_raw < 0 {
                    (len + s_raw + 1).max(1)
                } else {
                    s_raw.max(1)
                };
                let hi = if e_raw < 0 {
                    (len + e_raw + 1).max(0)
                } else {
                    e_raw.max(0)
                };
                let lo_idx = (lo - 1) as usize;
                let hi_idx = ((hi as usize).min(arr.len())).max(lo_idx);
                let _: Vec<String> = arr.splice(lo_idx..hi_idx, values).collect();
                exec.set_array(name, arr);
                return;
            }
            // Single-int key. `a[i]=()` (empty values) removes the
            // element at that index. Otherwise treat as a multi-element
            // splice starting at i.
            let i: i64 = match key.trim().parse::<i64>() {
                Ok(n) => n,
                Err(_) => return,
            };
            let len = arr.len() as i64;
            let idx = if i > 0 {
                (i - 1) as usize
            } else if i < 0 {
                let off = len + i;
                if off < 0 {
                    return;
                }
                off as usize
            } else {
                return;
            };
            if values.is_empty() {
                if idx < arr.len() {
                    arr.remove(idx);
                }
            } else {
                let end = (idx + 1).min(arr.len());
                let _: Vec<String> = arr.splice(idx..end, values).collect();
            }
            exec.set_array(name, arr);
        });
        fusevm::Value::Status(0)
    });

    // BUILTIN_CONCAT_SPLICE — word-segment concat with first/last
    // sticking (default zsh splice semantics for `${arr[@]}`, `$@`).
    vm.register_builtin(BUILTIN_CONCAT_SPLICE, |vm, _argc| {
        let rhs = vm.pop();
        let lhs = vm.pop();
        match (lhs, rhs) {
            (fusevm::Value::Array(mut la), fusevm::Value::Array(ra)) => {
                if la.is_empty() {
                    return fusevm::Value::Array(ra);
                }
                if ra.is_empty() {
                    return fusevm::Value::Array(la);
                }
                // Last of la merges with first of ra; rest unchanged.
                let last_l = la.pop().unwrap();
                let mut ra_iter = ra.into_iter();
                let first_r = ra_iter.next().unwrap();
                let l_s = last_l.as_str_cow();
                let r_s = first_r.as_str_cow();
                let mut merged = String::with_capacity(l_s.len() + r_s.len());
                merged.push_str(&l_s);
                merged.push_str(&r_s);
                la.push(fusevm::Value::str(merged));
                la.extend(ra_iter);
                fusevm::Value::Array(la)
            }
            (fusevm::Value::Array(mut la), rhs_scalar) => {
                if la.is_empty() {
                    return fusevm::Value::str(rhs_scalar.as_str_cow().to_string());
                }
                let last = la.pop().unwrap();
                let l_s = last.as_str_cow();
                let r_s = rhs_scalar.as_str_cow();
                let mut s = String::with_capacity(l_s.len() + r_s.len());
                s.push_str(&l_s);
                s.push_str(&r_s);
                la.push(fusevm::Value::str(s));
                fusevm::Value::Array(la)
            }
            (lhs_scalar, fusevm::Value::Array(mut ra)) => {
                if ra.is_empty() {
                    return fusevm::Value::str(lhs_scalar.as_str_cow().to_string());
                }
                let first = ra.remove(0);
                let l_s = lhs_scalar.as_str_cow();
                let r_s = first.as_str_cow();
                let mut s = String::with_capacity(l_s.len() + r_s.len());
                s.push_str(&l_s);
                s.push_str(&r_s);
                let mut out = Vec::with_capacity(ra.len() + 1);
                out.push(fusevm::Value::str(s));
                out.extend(ra);
                fusevm::Value::Array(out)
            }
            (lhs_s, rhs_s) => {
                let l = lhs_s.as_str_cow();
                let r = rhs_s.as_str_cow();
                let mut s = String::with_capacity(l.len() + r.len());
                s.push_str(&l);
                s.push_str(&r);
                fusevm::Value::str(s)
            }
        }
    });

    // BUILTIN_CONCAT_DISTRIBUTE — word-segment concat. With
    // rcexpandparam (zsh option), distributes element-wise (cartesian
    // product). Default mode: joins arrays with IFS first char to a
    // single scalar before concat, matching zsh's default unquoted
    // and DQ semantics. Direct port of Src/subst.c sepjoin path
    // (line ~1813) which gates element-vs-join on the rc_expand_param
    // option, defaulting to join.
    // BUILTIN_CONCAT_DISTRIBUTE_FORCED — same shape as
    // CONCAT_DISTRIBUTE, but always cartesian-distributes when one
    // side is Array. Used for compile-time-detected explicit
    // distribution forms (`${^arr}` etc.) where the source flag
    // overrides the rcexpandparam option default.
    vm.register_builtin(BUILTIN_CONCAT_DISTRIBUTE_FORCED, |vm, _argc| {
        let rhs = vm.pop();
        let lhs = vm.pop();
        match (lhs, rhs) {
            (fusevm::Value::Array(la), fusevm::Value::Array(ra)) => {
                if ra.is_empty() {
                    return fusevm::Value::Array(la);
                }
                if la.is_empty() {
                    return fusevm::Value::Array(ra);
                }
                let mut out = Vec::with_capacity(la.len() * ra.len());
                for a in &la {
                    let a_s = a.as_str_cow();
                    for b in &ra {
                        let b_s = b.as_str_cow();
                        let mut s = String::with_capacity(a_s.len() + b_s.len());
                        s.push_str(&a_s);
                        s.push_str(&b_s);
                        out.push(fusevm::Value::str(s));
                    }
                }
                fusevm::Value::Array(out)
            }
            (fusevm::Value::Array(la), rhs_scalar) => {
                let r = rhs_scalar.as_str_cow();
                let out: Vec<fusevm::Value> = la
                    .into_iter()
                    .map(|a| {
                        let a_s = a.as_str_cow();
                        let mut s = String::with_capacity(a_s.len() + r.len());
                        s.push_str(&a_s);
                        s.push_str(&r);
                        fusevm::Value::str(s)
                    })
                    .collect();
                fusevm::Value::Array(out)
            }
            (lhs_scalar, fusevm::Value::Array(ra)) => {
                let l = lhs_scalar.as_str_cow();
                let out: Vec<fusevm::Value> = ra
                    .into_iter()
                    .map(|b| {
                        let b_s = b.as_str_cow();
                        let mut s = String::with_capacity(l.len() + b_s.len());
                        s.push_str(&l);
                        s.push_str(&b_s);
                        fusevm::Value::str(s)
                    })
                    .collect();
                fusevm::Value::Array(out)
            }
            (lhs_s, rhs_s) => {
                let l = lhs_s.as_str_cow();
                let r = rhs_s.as_str_cow();
                let mut s = String::with_capacity(l.len() + r.len());
                s.push_str(&l);
                s.push_str(&r);
                fusevm::Value::str(s)
            }
        }
    });

    vm.register_builtin(BUILTIN_CONCAT_DISTRIBUTE, |vm, _argc| {
        let rhs = vm.pop();
        let lhs = vm.pop();
        let rc_expand = with_executor(|exec| {
            crate::ported::options::opt_state_get("rcexpandparam").unwrap_or(false)
        });
        let ifs_first = || -> String {
            with_executor(|exec| {
                exec.get_variable("IFS")
                    .chars()
                    .next()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| " ".to_string())
            })
        };
        // Helper: join an Array to scalar via IFS-first.
        let join_arr = |arr: Vec<fusevm::Value>| -> String {
            let sep = ifs_first();
            arr.iter()
                .map(|v| v.as_str_cow().into_owned())
                .collect::<Vec<_>>()
                .join(&sep)
        };
        if !rc_expand {
            // Default: join any Array side to scalar, then concat.
            let l = match lhs {
                fusevm::Value::Array(a) => join_arr(a),
                other => other.as_str_cow().into_owned(),
            };
            let r = match rhs {
                fusevm::Value::Array(a) => join_arr(a),
                other => other.as_str_cow().into_owned(),
            };
            let mut s = String::with_capacity(l.len() + r.len());
            s.push_str(&l);
            s.push_str(&r);
            return fusevm::Value::str(s);
        }
        match (lhs, rhs) {
            (fusevm::Value::Array(la), fusevm::Value::Array(ra)) => {
                // Cartesian product: [a + b for a in la for b in ra].
                let mut out = Vec::with_capacity(la.len() * ra.len().max(1));
                if ra.is_empty() {
                    return fusevm::Value::Array(la);
                }
                if la.is_empty() {
                    return fusevm::Value::Array(ra);
                }
                for a in &la {
                    let a_s = a.as_str_cow();
                    for b in &ra {
                        let b_s = b.as_str_cow();
                        let mut s = String::with_capacity(a_s.len() + b_s.len());
                        s.push_str(&a_s);
                        s.push_str(&b_s);
                        out.push(fusevm::Value::str(s));
                    }
                }
                fusevm::Value::Array(out)
            }
            (fusevm::Value::Array(la), rhs_scalar) => {
                let r = rhs_scalar.as_str_cow();
                let out: Vec<fusevm::Value> = la
                    .into_iter()
                    .map(|a| {
                        let a_s = a.as_str_cow();
                        let mut s = String::with_capacity(a_s.len() + r.len());
                        s.push_str(&a_s);
                        s.push_str(&r);
                        fusevm::Value::str(s)
                    })
                    .collect();
                fusevm::Value::Array(out)
            }
            (lhs_scalar, fusevm::Value::Array(ra)) => {
                let l = lhs_scalar.as_str_cow();
                let out: Vec<fusevm::Value> = ra
                    .into_iter()
                    .map(|b| {
                        let b_s = b.as_str_cow();
                        let mut s = String::with_capacity(l.len() + b_s.len());
                        s.push_str(&l);
                        s.push_str(&b_s);
                        fusevm::Value::str(s)
                    })
                    .collect();
                fusevm::Value::Array(out)
            }
            (lhs_s, rhs_s) => {
                // Fast path: both scalar → identical to Op::Concat.
                let l = lhs_s.as_str_cow();
                let r = rhs_s.as_str_cow();
                let mut s = String::with_capacity(l.len() + r.len());
                s.push_str(&l);
                s.push_str(&r);
                fusevm::Value::str(s)
            }
        }
    });

    // `[[ a -ef b ]]` — same-inode test. Resolves both paths via fs::metadata
    // (follows symlinks the way zsh's -ef does) and compares (dev, inode).
    // Returns false on any I/O error (path missing, permission denied, etc.).
    vm.register_builtin(BUILTIN_SAME_FILE, |vm, _argc| {
        let b = vm.pop().to_str();
        let a = vm.pop().to_str();
        let same = match (std::fs::metadata(&a), std::fs::metadata(&b)) {
            (Ok(ma), Ok(mb)) => ma.dev() == mb.dev() && ma.ino() == mb.ino(),
            _ => false,
        };
        fusevm::Value::Bool(same)
    });

    // `[[ -c path ]]` — character device.
    vm.register_builtin(BUILTIN_IS_CHARDEV, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.file_type().is_char_device())
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    // `[[ -b path ]]` — block device.
    vm.register_builtin(BUILTIN_IS_BLOCKDEV, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.file_type().is_block_device())
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    // `[[ -p path ]]` — FIFO (named pipe).
    vm.register_builtin(BUILTIN_IS_FIFO, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.file_type().is_fifo())
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    // `[[ -S path ]]` — socket.
    vm.register_builtin(BUILTIN_IS_SOCKET, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = std::fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_socket())
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });

    // `[[ -k path ]]` / `-u` / `-g` — sticky / setuid / setgid bit.
    vm.register_builtin(BUILTIN_HAS_STICKY, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.permissions().mode() & libc::S_ISVTX as u32 != 0)
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    vm.register_builtin(BUILTIN_HAS_SETUID, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.permissions().mode() & libc::S_ISUID as u32 != 0)
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    vm.register_builtin(BUILTIN_HAS_SETGID, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.permissions().mode() & libc::S_ISGID as u32 != 0)
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    vm.register_builtin(BUILTIN_OWNED_BY_USER, |vm, _argc| {
        let path = vm.pop().to_str();
        let euid = unsafe { libc::geteuid() };
        let result = std::fs::metadata(&path)
            .map(|m| m.uid() == euid)
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    vm.register_builtin(BUILTIN_OWNED_BY_GROUP, |vm, _argc| {
        let path = vm.pop().to_str();
        let egid = unsafe { libc::getegid() };
        let result = std::fs::metadata(&path)
            .map(|m| m.gid() == egid)
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });

    // `[[ -N path ]]` — file's access time is NOT newer than its
    // modification time (zsh man: "true if file exists and its
    // access time is not newer than its modification time"). Used
    // by zsh's mailbox-watching code. The semantic is `atime <=
    // mtime` (equivalent to `mtime >= atime`) — equal counts as
    // true, which a strict `mtime > atime` check missed for newly
    // created files where both stamps are identical.
    vm.register_builtin(BUILTIN_FILE_MODIFIED_SINCE_ACCESS, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.atime() <= m.mtime())
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });

    // `[[ a -nt b ]]` — true if `a`'s mtime is strictly later than `b`'s.
    // BOTH files must exist; if either is missing the result is false.
    // (Earlier behavior was bash's "missing == infinitely-old"; zsh
    // strictly requires both files to exist.)
    vm.register_builtin(BUILTIN_FILE_NEWER, |vm, _argc| {
        let b = vm.pop().to_str();
        let a = vm.pop().to_str();
        // Use SystemTime modified() for nanosecond precision —
        // MetadataExt::mtime() returns seconds only, so two files
        // touched within the same second compared equal even when
        // 500ms apart. zsh tracks ns and uses `>=` for ties (touching
        // a then b in quick succession should still report b newer).
        let ta = std::fs::metadata(&a).and_then(|m| m.modified()).ok();
        let tb = std::fs::metadata(&b).and_then(|m| m.modified()).ok();
        let result = match (ta, tb) {
            (Some(ta), Some(tb)) => ta > tb,
            _ => false,
        };
        fusevm::Value::Bool(result)
    });

    // `[[ a -ot b ]]` — mirror of -nt. Same both-must-exist contract.
    vm.register_builtin(BUILTIN_FILE_OLDER, |vm, _argc| {
        let b = vm.pop().to_str();
        let a = vm.pop().to_str();
        let ta = std::fs::metadata(&a).and_then(|m| m.modified()).ok();
        let tb = std::fs::metadata(&b).and_then(|m| m.modified()).ok();
        let result = match (ta, tb) {
            (Some(ta), Some(tb)) => ta < tb,
            _ => false,
        };
        fusevm::Value::Bool(result)
    });

    // `set -e` / `setopt errexit` post-command check. Compiler emits
    // this after each top-level command's SetStatus (skipped inside
    // conditionals/pipelines/&&||/`!`). If errexit is on AND the last
    // command exited non-zero AND it's not a `return` from a function,
    // exit the shell with that status.
    // `set -x` / `setopt xtrace` — print each command before it runs.
    // The compiler emits this BEFORE the actual builtin/external call
    // with the command's literal text as a single string arg. We
    // print to stderr if xtrace is on. Honors `$PS4` (default `+ `).
    //
    // ── XTRACE flow control ────────────────────────────────────────
    // Mirror of C zsh's `doneps4` flag in execcmd_exec (Src/exec.c).
    // When an assignment trace fires (XTRACE_ASSIGN), it emits PS4
    // and sets this flag so the subsequent XTRACE_ARGS skips its own
    // PS4 emission — the assignment + command end up on the SAME
    // line: `<PS4>a=1 echo hello\n`. XTRACE_ARGS / XTRACE_NEWLINE
    // reset the flag after emitting the trailing `\n`.
    vm.register_builtin(BUILTIN_XTRACE_LINE, |vm, _argc| {
        let cmd_text = vm.pop().to_str();
        // Sync exec.last_status with the live vm.last_status BEFORE
        // the next command runs. Direct port of the zsh exec.c
        // contract — `$?` reads the exit status of the *most recent*
        // command. XTRACE_LINE is emitted by the compiler BEFORE
        // every simple command, so it's the natural sync point.
        let live = vm.last_status;
        with_executor(|exec| {
            exec.set_last_status(live);
        });
        // C zsh emits xtrace for `(( … ))` / `[[ … ]]` / `case` /
        // `if/while/until/for/repeat` head expressions via
        // `printprompt4(); fprintf(xtrerr, "%s\n", expr)` at
        // Src/exec.c:5240 (math), c:5286 (cond), c:4117 (for), etc.
        // The compiler emits BUILTIN_XTRACE_LINE only at those
        // construct boundaries (compile_arith / compile_cond /
        // compile_if / compile_while / compile_for / compile_case);
        // simple commands route to BUILTIN_XTRACE_ARGS instead. So
        // this handler always emits when xtrace is on — no prefix-
        // string heuristic.
        let on =
            with_executor(|exec| crate::ported::options::opt_state_get("xtrace").unwrap_or(false));
        if on {
            let already = XTRACE_DONE_PS4.with(|f| f.get());
            if !already {
                printprompt4();
            }
            eprintln!("{}", cmd_text);
            XTRACE_DONE_PS4.with(|f| f.set(false));
        }
        fusevm::Value::Status(0)
    });

    // Like XTRACE_LINE but reads the top `argc - 1` values from the
    // VM stack WITHOUT consuming them (peek), then pops a prefix
    // string at the top. Joins prefix + peeked args with spaces using
    // zsh's quotedzputs-equivalent quoting. Direct port of
    // Src/exec.c:2055-2066 — emit AFTER expansion, with each arg
    // shell-quoted, so `for i in a b; echo for $i` traces as
    // `echo for a` / `echo for b`, not `echo for $i`.
    //
    // Stack contract on entry: [arg1, arg2, ..., argN, prefix].
    // Pops prefix; peeks argN..arg1 below. argc = N + 1.
    vm.register_builtin(BUILTIN_XTRACE_ARGS, |vm, argc| {
        let prefix = vm.pop().to_str();
        let live = vm.last_status;
        with_executor(|exec| {
            exec.set_last_status(live);
        });
        let on =
            with_executor(|exec| crate::ported::options::opt_state_get("xtrace").unwrap_or(false));
        if on {
            let n_args = argc.saturating_sub(1) as usize;
            let len = vm.stack.len();
            let arg_strs: Vec<String> = if n_args > 0 && len >= n_args {
                vm.stack[len - n_args..]
                    .iter()
                    .map(|v| quotedzputs(&v.to_str()))
                    .collect()
            } else {
                Vec::new()
            };
            // Builtins dispatch through `execbuiltin` (Src/builtin.c:442)
            // which emits its own PS4 + name + args xtrace. To avoid
            // double-emission, skip our emission here when the first
            // arg is a known builtin with a registered HandlerFunc —
            // those go through execbuiltin and will trace themselves.
            //
            // Externals + builtins without HandlerFunc (still pending
            // canonical port) keep our emission as a stand-in until
            // they migrate over.
            // The `prefix` IS the command name (first whitespace-token
            // of the original cmd text). If a BUILTIN entry with a
            // HandlerFunc matches, execbuiltin will emit xtrace there.
            let goes_through_execbuiltin = crate::ported::builtin::BUILTINS
                .iter()
                .any(|b| b.node.nam == prefix && b.handlerfunc.is_some());
            if !goes_through_execbuiltin {
                let line = if arg_strs.is_empty() {
                    prefix
                } else {
                    format!("{} {}", prefix, arg_strs.join(" "))
                };
                // Mirrors Src/exec.c:2055 xtrace emission. C does:
                //   if (!doneps4) printprompt4();
                //   ... emit args + spaces ...
                //   fputc('\n', xtrerr); fflush(xtrerr);
                // We honor doneps4 via XTRACE_DONE_PS4 — if a prior
                // XTRACE_ASSIGN this line already emitted PS4, skip
                // it. Then reset the flag after the trailing newline
                // so the next command starts fresh.
                let already_ps4 = XTRACE_DONE_PS4.with(|f| f.get());
                if !already_ps4 {
                    printprompt4();
                }
                eprintln!("{}", line);
            }
            XTRACE_DONE_PS4.with(|f| f.set(false));
        }
        fusevm::Value::Status(0)
    });

    // BUILTIN_XTRACE_ASSIGN — direct port of the per-assignment
    // trace block at Src/exec.c:2517-2582. C body excerpt:
    //   xtr = isset(XTRACE);
    //   if (xtr) { printprompt4(); doneps4 = 1; }
    //   while (assign) {
    //       if (xtr) fprintf(xtrerr, "%s+=" or "%s=", name);
    //       ... eval value into `val` ...
    //       if (xtr) { quotedzputs(val, xtrerr); fputc(' ', xtrerr); }
    //       ...
    //   }
    //
    // Stack on entry: [..., name, value]. PEEKS both (they're left
    // on stack for SET_VAR to pop). Emits `name=<quoted-val> ` with
    // no newline; trailing `\n` comes from XTRACE_ARGS (cmd path)
    // or XTRACE_NEWLINE (assignment-only path).
    vm.register_builtin(BUILTIN_XTRACE_ASSIGN, |vm, _argc| {
        let on =
            with_executor(|exec| crate::ported::options::opt_state_get("xtrace").unwrap_or(false));
        if on {
            // PEEK [..., name, value] — argc==2 by contract.
            let len = vm.stack.len();
            if len >= 2 {
                let name = vm.stack[len - 2].to_str();
                let value = vm.stack[len - 1].to_str();
                let already_ps4 = XTRACE_DONE_PS4.with(|f| f.get());
                if !already_ps4 {
                    printprompt4();
                    XTRACE_DONE_PS4.with(|f| f.set(true));
                }
                // C: `fprintf(xtrerr, "%s=", name)` then `quotedzputs
                // (val); fputc(' ', xtrerr);`. Emit no newline.
                eprint!("{}={} ", name, quotedzputs(&value));
            }
        }
        fusevm::Value::Status(0)
    });

    // BUILTIN_XTRACE_NEWLINE — emit trailing `\n` + flush iff a
    // prior XTRACE_ASSIGN this line already emitted PS4. Mirrors
    // C's `fputc('\n', xtrerr); fflush(xtrerr);` at exec.c:3398
    // (the assignment-only path through execcmd_exec).
    vm.register_builtin(BUILTIN_XTRACE_NEWLINE, |_vm, _argc| {
        let on =
            with_executor(|exec| crate::ported::options::opt_state_get("xtrace").unwrap_or(false));
        if on {
            let already_ps4 = XTRACE_DONE_PS4.with(|f| f.get());
            if already_ps4 {
                eprintln!();
                XTRACE_DONE_PS4.with(|f| f.set(false));
            }
        }
        fusevm::Value::Status(0)
    });

    vm.register_builtin(BUILTIN_ERREXIT_CHECK, |vm, _argc| {
        let last = vm.last_status;
        if last == 0 {
            return fusevm::Value::Status(0);
        }
        // ZERR / ERR trap fires whenever a command exits non-zero
        // (zsh signals.c handle_signals path). Read the trap body
        // BEFORE the errexit check so a trap on the failing
        // command's last command can run before we exit.
        let zerr_body = with_executor(|exec| {
            exec.traps
                .get("ZERR")
                .cloned()
                .or_else(|| exec.traps.get("ERR").cloned())
        });
        if let Some(body) = zerr_body {
            // Run the trap. Don't recurse on the trap's own failure
            // (clear last_status during the run).
            with_executor(|exec| {
                let saved = exec.last_status();
                exec.set_last_status(0);
                let _ = exec.execute_script(&body);
                exec.set_last_status(saved);
            });
        }
        let should_exit = with_executor(|exec| {
            // zsh stores the option as `errexit` (default OFF). Honor
            // both keys (`errexit=true` from `setopt errexit` /
            // `set -o errexit`, and `set -e` which currently writes
            // `errexit=true` too). Also suppress when inside a function
            // call — zsh's errexit lets functions handle their own
            // failures unless ERR_RETURN is also set. Also suppress
            // when inside a subshell — the in-process snapshot/restore
            // doesn't have a process-isolation boundary, so a real
            // `process::exit` would tear down the parent shell. Match
            // zsh's "errexit aborts the subshell only" by leaving the
            // parent alive (subshell continues until natural end).
            // errexit lives in two stores. `set -e` / `setopt errexit`
            // write through bin_setopt → OPTS_LIVE (canonical
            // `opts[ERREXIT]` per Src/options.c:46). Older paths still
            // populate `exec.options`. Check both — agree when EITHER
            // says on.
            let on_canonical = isset(ERREXIT);
            let on_legacy = opt_state_get("errexit").unwrap_or(false);
            (on_canonical || on_legacy)
                && exec.local_scope_depth == 0
                && exec.subshell_snapshots.is_empty()
        });
        if should_exit {
            std::process::exit(last);
        }
        fusevm::Value::Status(last)
    });

    // `${var:-default}` / `${var:=default}` / `${var:?error}` / `${var:+alt}`
    // Pops [name, op_byte, rhs] (rhs popped first). Returns the modified
    // value as Value::Str. Handles unset/empty distinction (`:-` etc.
    // treat empty same as unset, matching POSIX).
    // BUILTIN_PARAM_DEFAULT_FAMILY — `${var-x}` / `${var:-x}` / `${var=x}` /
    // `${var:=x}` / `${var?x}` / `${var:?x}` / `${var+x}` / `${var:+x}`.
    // PURE PASSTHRU: pop name + op + rhs, reconstruct the canonical
    // brace expression, hand to `subst::paramsubst` (C port of
    // `Src/subst.c::paramsubst`). All "missing vs empty" gating,
    // nounset suppression, default-evaluation, and elide-empty-words
    // semantics live inside paramsubst.
    vm.register_builtin(BUILTIN_PARAM_DEFAULT_FAMILY, |vm, _argc| {
        let rhs = vm.pop().to_str();
        let op = vm.pop().to_int() as u8;
        let name = vm.pop().to_str();
        let op_str = match op {
            0 => ":-",
            1 => ":=",
            2 => ":?",
            3 => ":+",
            4 => "-",
            5 => "=",
            6 => "?",
            7 => "+",
            _ => "-",
        };
        let body = format!("${{{}{}{}}}", name, op_str, rhs);
        let mut ret_flags: i32 = 0;
        let (_full, _pos, nodes) =
            crate::ported::subst::paramsubst(&body, 0, false, 0i32, &mut ret_flags);
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            with_executor(|exec| exec.set_last_status(1));
        }
        if nodes.is_empty() {
            fusevm::Value::Array(Vec::new())
        } else if nodes.len() == 1 {
            fusevm::Value::str(nodes.into_iter().next().unwrap())
        } else {
            fusevm::Value::Array(nodes.into_iter().map(fusevm::Value::str).collect())
        }
    });

    // `${var:offset[:length]}` — substring. Pops [name, offset, length].
    // length == -1 means "rest of string". Negative offset counts from end.
    // BUILTIN_PARAM_SUBSTRING — `${var:offset:length}` literal-int form.
    // PURE PASSTHRU: reconstruct `${name:offset:length}` and route
    // through `subst::paramsubst`. Length sentinel `i64::MIN` =
    // "no length given" (omit the `:length` portion).
    vm.register_builtin(BUILTIN_PARAM_SUBSTRING, |vm, _argc| {
        let length = vm.pop().to_int();
        let offset = vm.pop().to_int();
        let name = vm.pop().to_str();
        let body = if length == i64::MIN {
            format!("${{{}:{}}}", name, offset)
        } else {
            format!("${{{}:{}:{}}}", name, offset, length)
        };
        let mut ret_flags: i32 = 0;
        let (_full, _pos, nodes) =
            crate::ported::subst::paramsubst(&body, 0, false, 0i32, &mut ret_flags);
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            with_executor(|exec| exec.set_last_status(1));
        }
        if nodes.is_empty() {
            fusevm::Value::Array(Vec::new())
        } else if nodes.len() == 1 {
            fusevm::Value::str(nodes.into_iter().next().unwrap())
        } else {
            fusevm::Value::Array(nodes.into_iter().map(fusevm::Value::str).collect())
        }
    });

    // BUILTIN_PARAM_SUBSTRING_EXPR — `${var:offset_expr[:length_expr]}` form.
    // PURE PASSTHRU: rebuild `${name:offset:length}` using the
    // expression text verbatim (paramsubst's offset/length
    // parser evaluates arith / param refs itself).
    vm.register_builtin(BUILTIN_PARAM_SUBSTRING_EXPR, |vm, _argc| {
        let has_len = vm.pop().to_int() != 0;
        let len_expr = vm.pop().to_str();
        let off_expr = vm.pop().to_str();
        let name = vm.pop().to_str();
        let body = if has_len {
            format!("${{{}:{}:{}}}", name, off_expr, len_expr)
        } else {
            format!("${{{}:{}}}", name, off_expr)
        };
        let mut ret_flags: i32 = 0;
        let (_full, _pos, nodes) =
            crate::ported::subst::paramsubst(&body, 0, false, 0i32, &mut ret_flags);
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            with_executor(|exec| exec.set_last_status(1));
        }
        if nodes.is_empty() {
            fusevm::Value::Array(Vec::new())
        } else if nodes.len() == 1 {
            fusevm::Value::str(nodes.into_iter().next().unwrap())
        } else {
            fusevm::Value::Array(nodes.into_iter().map(fusevm::Value::str).collect())
        }
    });

    // `${var#pat}` / `${var##pat}` / `${var%pat}` / `${var%%pat}`
    // Pops [name, pattern, op_byte]. op: 0=`#` short-prefix, 1=`##` long,
    // 2=`%` short-suffix, 3=`%%` long. Glob-pattern matching via the
    // existing glob_match_static helper.
    // BUILTIN_PARAM_STRIP — `${var#pat}` / `${var##pat}` / `${var%pat}` /
    // `${var%%pat}`. PURE PASSTHRU: reconstruct the brace expression
    // and route through `subst::paramsubst`. (M)/(S) flags arrive
    // through SUB_FLAGS (already inside paramsubst's scope), so we
    // just clear the bridge-side cached read.
    vm.register_builtin(BUILTIN_PARAM_STRIP, |vm, _argc| {
        let _dq_flag = vm.pop().to_int() != 0;
        let op = vm.pop().to_int() as u8;
        let pattern = vm.pop().to_str();
        let name = vm.pop().to_str();
        let op_str = match op {
            0 => "#",
            1 => "##",
            2 => "%",
            3 => "%%",
            _ => "#",
        };
        let body = format!("${{{}{}{}}}", name, op_str, pattern);
        let mut ret_flags: i32 = 0;
        let (_full, _pos, nodes) =
            crate::ported::subst::paramsubst(&body, 0, false, 0i32, &mut ret_flags);
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            with_executor(|exec| exec.set_last_status(1));
        }
        if nodes.is_empty() {
            fusevm::Value::Array(Vec::new())
        } else if nodes.len() == 1 {
            fusevm::Value::str(nodes.into_iter().next().unwrap())
        } else {
            fusevm::Value::Array(nodes.into_iter().map(fusevm::Value::str).collect())
        }
    });

    // `$((expr))` — pops [expr_string], evaluates via MathEval which
    // honors integer-vs-float distinction (zsh-compatible). Returns
    // the result as Value::Str so it can be Concat'd into surrounding
    // word context.
    vm.register_builtin(BUILTIN_ARITH_EVAL, |vm, _argc| {
        let expr = vm.pop().to_str();
        let result = crate::ported::subst::arithsubst(&expr, "", "");
        fusevm::Value::str(result)
    });

    // `$(cmd)` — pops [cmd_string], routes through
    // run_command_substitution which performs an in-process pipe-capture.
    // Avoids the Op::CmdSubst sub-chunk word-emit bug
    // (`printf "a\nb"` produced "anb" via that path). Returns trimmed
    // output (trailing newlines stripped per POSIX cmd-sub semantics).
    vm.register_builtin(BUILTIN_CMD_SUBST_TEXT, |vm, _argc| {
        let cmd = vm.pop().to_str();
        // Inherit live $? into the inner shell so cmd-subst sees the
        // parent's most recent exit. Same rationale as the mode-3
        // backtick path above.
        let live_status = vm.last_status;
        let result = with_executor(|exec| {
            exec.set_last_status(live_status);
            exec.run_command_substitution(&cmd)
        });
        // Mirror run_command_substitution's exec.last_status side
        // effect into the VM's live counter so a containing
        // assignment's BUILTIN_SET_VAR — which reads vm.last_status
        // — sees the cmd-subst's exit. Without this, `a=$(false);
        // echo $?` reads stale 0 (vm.last_status was zeroed by
        // compile_assign's prelude SetStatus, and run_cmd_subst only
        // updated exec.last_status). Pull the value back through
        // exec since it owns the canonical post-subst record.
        let cs_status = with_executor(|exec| exec.last_status());
        vm.last_status = cs_status;
        fusevm::Value::str(result)
    });

    // Text-based word expansion. Pops [preserved_text, mode_byte].
    // mode_byte:
    //   0 = Default — expand_string + xpandbraces + expand_glob
    //   1 = DoubleQuoted — strip outer `"…"`, expand_string only
    //         (no brace, no glob — DQ semantics)
    //   2 = SingleQuoted — strip outer `'…'`, no expansion
    //         (kept for symmetry; Snull early-return covers most SQ)
    //   3 = AltBackquote — strip backticks, run as cmd-sub
    // Single result → Value::str; multi → Value::Array.
    vm.register_builtin(BUILTIN_EXPAND_TEXT, |vm, _argc| {
        let mode = vm.pop().to_int() as u8;
        let text = vm.pop().to_str();
        // Sync vm.last_status → exec.last_status so cmd-subst (mode 3)
        // and any nested $? reads inside singsub see the live `$?`
        // from the most recent VM op. Without this, cmd-subst inside
        // arg-eval saw a stale exec.last_status that was zeroed at
        // the start of the current statement. Direct port of zsh's
        // pre-cmdsubst lastval propagation per Src/exec.c:4770.
        let live_status = vm.last_status;
        with_executor(|exec| match mode {
            // Mode 1 = DoubleQuoted (argument context).
            // Mode 5 = DoubleQuoted in scalar-assignment context.
            // Both share the same DQ unescape pre-processing; mode 5
            // additionally bumps `in_scalar_assign` so subst_port's
            // paramsubst sees ssub=true and suppresses split flags
            // `(f)` / `(s:STR:)` / `(0)` per Src/subst.c:1759 +
            // Src/exec.c::addvars line 2546 (the PREFORK_SINGLE bit
            // C zsh sets when prefork-ing the assignment RHS).
            1 | 5 => {
                // DoubleQuoted: strip outer `"…"` if present. In DQ
                // context, `\` escapes the DQ-special chars `$`, `` ` ``,
                // `"`, `\`. zsh's expand_string expects the lexer's
                // `\0X` literal-marker for an already-escaped char, so
                // we pre-process: `\$` → `\0$`, `\\` → `\0\`, etc. Then
                // expand_string handles the rest.
                let inner = if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
                    &text[1..text.len() - 1]
                } else {
                    text.as_str()
                };
                let mut prepped = String::with_capacity(inner.len());
                let mut chars = inner.chars().peekable();
                while let Some(c) = chars.next() {
                    if c == '\\' {
                        match chars.peek() {
                            Some('$') | Some('`') | Some('"') | Some('\\') => {
                                prepped.push('\x00');
                                prepped.push(chars.next().unwrap());
                            }
                            _ => prepped.push(c),
                        }
                    } else {
                        prepped.push(c);
                    }
                }
                // Tell parameter-flag application that we're inside
                // double quotes — array-only flags ((o), (O), (n),
                // (i), (M), (u)) must be no-ops here per zsh.
                exec.in_dq_context += 1;
                if mode == 5 {
                    exec.in_scalar_assign += 1;
                }
                let out = crate::ported::subst::singsub(&prepped);
                if mode == 5 {
                    exec.in_scalar_assign -= 1;
                }
                exec.in_dq_context -= 1;
                fusevm::Value::str(out)
            }
            2 => {
                // SingleQuoted: pure literal, strip outer `'…'`.
                let inner = if text.len() >= 2 && text.starts_with('\'') && text.ends_with('\'') {
                    &text[1..text.len() - 1]
                } else {
                    text.as_str()
                };
                fusevm::Value::str(inner.to_string())
            }
            3 => {
                // Backquote command sub: strip outer backticks.
                // Word-split the result on IFS when the surrounding
                // word is unquoted — zsh: `print -l \`echo a b c\``
                // emits one arg per word. The $(…) path applies the
                // same split via BUILTIN_WORD_SPLIT after capture; do
                // the equivalent here for the `…` form.
                let inner = if text.len() >= 2 && text.starts_with('`') && text.ends_with('`') {
                    &text[1..text.len() - 1]
                } else {
                    text.as_str()
                };
                // Apply the live VM status before running the inner
                // shell so the inherited $? matches zsh's lastval
                // propagation.
                exec.set_last_status(live_status);
                let captured = exec.run_command_substitution(inner);
                let trimmed = captured.trim_end_matches('\n');
                if exec.in_dq_context > 0 {
                    fusevm::Value::str(trimmed.to_string())
                } else {
                    let ifs = exec.scalar("IFS").unwrap_or_else(|| " \t\n".to_string());
                    let parts: Vec<fusevm::Value> = trimmed
                        .split(|c: char| ifs.contains(c))
                        .filter(|s| !s.is_empty())
                        .map(|s| fusevm::Value::str(s.to_string()))
                        .collect();
                    if parts.is_empty() {
                        fusevm::Value::str(String::new())
                    } else if parts.len() == 1 {
                        parts.into_iter().next().unwrap()
                    } else {
                        fusevm::Value::Array(parts)
                    }
                }
            }
            4 => {
                // HeredocBody: expand variables / command-subst / arith
                // but NOT glob or brace. Heredoc lines like `[42]` must
                // pass through verbatim — running them through the
                // default pipeline triggers NOMATCH on the literal.
                fusevm::Value::str(crate::ported::subst::singsub(&text))
            }
            _ => {
                // Default: full expansion pipeline.
                // Pre-process backslash-escapes to the `\x00X` literal-
                // marker form so expand_string suppresses variable
                // expansion on escaped specials: `\$` → literal `$`,
                // `\\` → literal `\`, `\`` → literal `` ` ``. Without
                // this, `echo \$a` ran `\` literally then expanded
                // `$a`, leaving a stray `\` that echo's escape
                // interpreter then turned into form-feed when followed
                // by `f`-like content.
                let mut prepped = String::with_capacity(text.len());
                let mut it = text.chars().peekable();
                while let Some(c) = it.next() {
                    if c == '\\' {
                        match it.peek() {
                            Some('$') | Some('`') | Some('"') | Some('\'') | Some('\\') => {
                                prepped.push('\x00');
                                prepped.push(it.next().unwrap());
                            }
                            // Don't preprocess `\{` / `\}` here — the
                            // brace-expansion stage has its own
                            // has_balanced_escaped_braces detector that
                            // strips the backslashes when both sides
                            // are escaped. Touching them here would
                            // hide them from that detector.
                            _ => prepped.push(c),
                        }
                    } else {
                        prepped.push(c);
                    }
                }
                let expanded = crate::ported::subst::singsub(&prepped);
                let brace_expanded = vec![expanded.to_string()];
                // zsh stores the option as `glob` (default ON);
                // `setopt noglob` writes `glob=false`. Honor either
                // form so the dispatcher behaves the same as zsh.
                let noglob = crate::ported::options::opt_state_get("noglob").unwrap_or(false)
                    || crate::ported::options::opt_state_get("GLOB")
                        .map(|v| !v)
                        .unwrap_or(false)
                    || !crate::ported::options::opt_state_get("glob").unwrap_or(true);
                let parts: Vec<String> = brace_expanded
                    .into_iter()
                    .flat_map(|s| {
                        // The lexer leaves glob metacharacters in their
                        // META-encoded form: `*` → `\u{87}`, `?` →
                        // `\u{86}`, `[` → `\u{91}`, etc. expand_string
                        // doesn't untokenize them, so the literal-char
                        // checks below (`s.contains('*')`) would miss
                        // every real glob and skip expand_glob — that
                        // bug let `echo *.toml` print the literal
                        // `*.toml` because the META `\u{87}` never
                        // matched the literal `*`. Untokenize once so
                        // the metacharacter checks see the canonical
                        // form. zsh's pattern.c expects `*` etc. as
                        // bare chars at the glob layer.
                        let s = crate::lex::untokenize(&s);
                        // Skip glob expansion for assignment-shaped
                        // words (`NAME=value`). zsh doesn't expand the
                        // RHS of an assignment as a path glob unless
                        // `setopt globassign` is set, and feeding such
                        // words through expand_glob makes NOMATCH
                        // (default ON) fire spuriously on
                        // `integer i=2*3+1`, `path=*.rs`, etc.
                        let is_assignment_shape = {
                            let bytes = s.as_bytes();
                            let mut i = 0;
                            if !bytes.is_empty()
                                && (bytes[0] == b'_' || bytes[0].is_ascii_alphabetic())
                            {
                                i += 1;
                                while i < bytes.len()
                                    && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric())
                                {
                                    i += 1;
                                }
                                i < bytes.len() && bytes[i] == b'='
                            } else {
                                false
                            }
                        };
                        // Also trigger expand_glob when the word ends
                        // with a `(...)` qualifier suffix even without
                        // any other glob metachar — `/etc/hosts(mh-100)`,
                        // `path(.)`, etc.
                        let has_qual_suffix =
                            s.ends_with(')') && s.contains('(') && !s.contains('|');
                        // extendedglob `^pat` (negation) and `pat~excl`
                        // (exclusion). Trigger expand_glob so the runtime
                        // can apply the appropriate filter. Both require
                        // `setopt extendedglob` — runtime falls through
                        // to literal if that's off.
                        let extglob_meta = crate::ported::options::opt_state_get("extendedglob")
                            .unwrap_or(false)
                            && (s.starts_with('^') || s.contains('~') || s.contains("/^"));
                        let has_numeric_range = s.contains('<')
                            && s.contains('>')
                            && !crate::ported::pattern::extract_numeric_ranges(&s).is_empty();
                        // Glob alternation `(a|b|c)` is a primary
                        // zsh feature — `/etc/(passwd|hostname)`
                        // should expand to file matches. Detected
                        // by `(` ... `|` ... `)` shape; the actual
                        // top-level-vs-nested check happens in
                        // expand_glob_alternation.
                        let has_alternation = s.contains('(') && s.contains('|') && s.contains(')');
                        if !noglob
                            && !is_assignment_shape
                            && (s.contains('*')
                                || s.contains('?')
                                || s.contains('[')
                                || has_qual_suffix
                                || extglob_meta
                                || has_numeric_range
                                || has_alternation)
                        {
                            exec.expand_glob(&s)
                        } else {
                            vec![s]
                        }
                    })
                    .collect();
                if parts.len() == 1 {
                    let only = parts.into_iter().next().unwrap_or_default();
                    // Empty unquoted expansion → drop the arg entirely
                    // (zsh "remove empty unquoted words" rule). Returning
                    // an empty Value::Array makes pop_args contribute zero
                    // items. Direct port of subst.c's empty-elide pass at
                    // the end of multsub which removes empty linknodes
                    // from unquoted contexts. Quoted DQ/SQ paths (modes
                    // 1/2/5) take separate arms above and always emit
                    // Value::Str so the empty arg survives.
                    if only.is_empty() {
                        fusevm::Value::Array(Vec::new())
                    } else {
                        fusevm::Value::str(only)
                    }
                } else {
                    fusevm::Value::Array(parts.into_iter().map(fusevm::Value::str).collect())
                }
            }
        })
    });

    // `${#name}` — pops [name]. Returns the value's element count for
    // arrays (indexed and assoc) or character length for scalars.
    // BUILTIN_PARAM_LENGTH — `${#name}`. PURE PASSTHRU.
    vm.register_builtin(BUILTIN_PARAM_LENGTH, |vm, _argc| {
        let name = vm.pop().to_str();
        let body = format!("${{#{}}}", name);
        let mut ret_flags: i32 = 0;
        let (_full, _pos, nodes) =
            crate::ported::subst::paramsubst(&body, 0, false, 0i32, &mut ret_flags);
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            with_executor(|exec| exec.set_last_status(1));
        }
        if nodes.is_empty() {
            fusevm::Value::str("0")
        } else if nodes.len() == 1 {
            fusevm::Value::str(nodes.into_iter().next().unwrap())
        } else {
            fusevm::Value::Array(nodes.into_iter().map(fusevm::Value::str).collect())
        }
    });

    // `${var/pat/repl}` / `${var//pat/repl}` / `${var/#pat/repl}` /
    // `${var/%pat/repl}` — Pops [name, pattern, replacement, op_byte].
    // op: 0=first, 1=all, 2=anchor-prefix (`/#`), 3=anchor-suffix (`/%`).
    // BUILTIN_PARAM_REPLACE — `${var/pat/repl}` / `${var//pat/repl}` /
    // `${var/#pat/repl}` / `${var/%pat/repl}`. PURE PASSTHRU.
    vm.register_builtin(BUILTIN_PARAM_REPLACE, |vm, _argc| {
        let _dq_flag = vm.pop().to_int() != 0;
        let op = vm.pop().to_int() as u8;
        let repl = vm.pop().to_str();
        let pattern = vm.pop().to_str();
        let name = vm.pop().to_str();
        // op encoding: 0 = first `/`, 1 = all `//`, 2 = anchor-prefix
        // `/#`, 3 = anchor-suffix `/%`. The brace form distinguishes
        // first-vs-all by single vs doubled slash, and anchored by
        // a `#` or `%` immediately after the slash(es).
        let body = match op {
            0 => format!("${{{}/{}/{}}}", name, pattern, repl),
            1 => format!("${{{}//{}/{}}}", name, pattern, repl),
            2 => format!("${{{}/#{}/{}}}", name, pattern, repl),
            3 => format!("${{{}/%{}/{}}}", name, pattern, repl),
            _ => format!("${{{}/{}/{}}}", name, pattern, repl),
        };
        let mut ret_flags: i32 = 0;
        let (_full, _pos, nodes) =
            crate::ported::subst::paramsubst(&body, 0, false, 0i32, &mut ret_flags);
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            with_executor(|exec| exec.set_last_status(1));
        }
        if nodes.is_empty() {
            fusevm::Value::Array(Vec::new())
        } else if nodes.len() == 1 {
            fusevm::Value::str(nodes.into_iter().next().unwrap())
        } else {
            fusevm::Value::Array(nodes.into_iter().map(fusevm::Value::str).collect())
        }
    });

    vm.register_builtin(BUILTIN_REGISTER_COMPILED_FN, |vm, argc| {
        let args = pop_args(vm, argc);
        let mut iter = args.into_iter();
        let name = iter.next().unwrap_or_default();
        let body_b64 = iter.next().unwrap_or_default();
        let body_source = iter.next().unwrap_or_default();
        let line_base_str = iter.next().unwrap_or_default();
        let line_base: i64 = line_base_str.parse().unwrap_or(0);
        let bytes = base64_decode(&body_b64);
        let status = match bincode::deserialize::<fusevm::Chunk>(&bytes) {
            Ok(chunk) => with_executor(|exec| {
                let def_file = exec.scriptfilename.clone();
                if !body_source.is_empty() {
                    exec.function_source
                        .insert(name.clone(), body_source.clone());
                }
                exec.function_line_base.insert(name.clone(), line_base);
                exec.function_def_file.insert(name.clone(), def_file);
                // PFA-SMR aspect: every `name() {}` / `function name { }`
                // funnels through here at compile time. Emit one record
                // with the function name + raw body source.
                #[cfg(feature = "recorder")]
                if crate::recorder::is_enabled() {
                    let ctx = exec.recorder_ctx();
                    let body = if body_source.is_empty() {
                        None
                    } else {
                        Some(body_source.as_str())
                    };
                    crate::recorder::emit_function(&name, body, ctx);
                }
                // Mirror into canonical shfunctab so scanfunctions /
                // ${(k)functions} / functions builtin see user defs.
                // C: exec.c:funcdef → shfunctab->addnode(ztrdup(name),shf).
                if let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().write() {
                    let shf = crate::ported::hashtable::shfunc_with_body(&name, &body_source);
                    tab.add(shf);
                }
                exec.functions_compiled.insert(name, chunk);
                0
            }),
            Err(_) => 1,
        };
        Value::Status(status)
    });

    // Wire the ShellHost so direct shell ops (Op::Glob, Op::TildeExpand,
    // Op::ExpandParam, Op::CmdSubst, Op::CallFunction, etc.) route through
    // ZshrsHost back into the executor.
    vm.set_shell_host(Box::new(ZshrsHost));
}

impl ZshrsHost {
    /// True iff `c` can be a `(j:…:)` / `(s:…:)` delimiter — non-alphanumeric,
    /// non-underscore. Restricting to punctuation avoids `(jL)` consuming `L`
    /// as a delim instead of as the next flag.
    fn is_zsh_flag_delim(c: char) -> bool {
        !c.is_ascii_alphanumeric() && c != '_'
    }
}

fn pop_args(vm: &mut fusevm::VM, argc: u8) -> Vec<String> {
    let mut popped: Vec<fusevm::Value> = Vec::with_capacity(argc as usize);
    for _ in 0..argc {
        popped.push(vm.pop());
    }
    popped.reverse();
    let mut args: Vec<String> = Vec::with_capacity(popped.len());
    for v in popped {
        match v {
            fusevm::Value::Array(items) => {
                for item in items {
                    args.push(item.to_str());
                }
            }
            other => args.push(other.to_str()),
        }
    }
    // `expand_glob` set the glob-failed cell when a no-match glob in
    // this command's argv triggered the `nomatch` error. C zsh
    // (Src/glob.c:1877) calls `zerr("no matches found: %s", ostr)`
    // which sets errflag|=ERRFLAG_ERROR, then unwinds out of the
    // execpline path; the next top-level event clears errflag and
    // the script continues. Previously zshrs called
    // `process::exit(1)` here — killing the WHOLE shell on the first
    // failed glob and breaking any plugin script that uses optional
    // patterns (`ls /no_match*; echo after` lost the "after" line).
    // Now just signal the failure via last_status + the per-command
    // glob_failed cell so the downstream builtin handler /
    // host_exec_external (line ~10245) short-circuits and returns
    // status 1 without running the command body. The flag is left
    // set for the dispatcher to consume + clear.
    if with_executor(|exec| exec.current_command_glob_failed.get()) {
        with_executor(|exec| exec.set_last_status(1));
    }
    // `$_` tracks the last argument of the PREVIOUSLY executed
    // command (zsh / bash convention). Promote the deferred value
    // into `$_` BEFORE this command runs (so `echo $_` reads the
    // prior command's last arg) then stash THIS command's last arg
    // for the next dispatch.
    let new_last = args.last().cloned();
    with_executor(|exec| {
        if let Some(prev) = exec.pending_underscore.take() {
            exec.set_scalar("_".to_string(), prev);
        }
        if let Some(last) = new_last {
            exec.pending_underscore = Some(last);
        }
    });
    args
}

/// zsh dispatch order is alias → function → builtin → external. The
/// compiler emits direct CallBuiltin ops for known builtin names for
/// perf, which silently skips a user function that shadows the same
/// name (e.g. `echo() { ... }; echo hi` would run the C builtin
/// without this check). Returns Some(status) when the call is routed
/// to the user function; the builtin handler should fall through to
/// its native impl when None.
fn try_user_fn_override(name: &str, args: &[String]) -> Option<i32> {
    let has_fn = with_executor(|exec| {
        exec.functions_compiled.contains_key(name) || exec.function_exists(name)
    });
    if !has_fn {
        return None;
    }
    Some(with_executor(|exec| {
        exec.dispatch_function_call(name, args).unwrap_or(127)
    }))
}

// IDs 281 (was BUILTIN_EXPAND_WORD_RUNTIME) and 282 (was
// BUILTIN_REGISTER_FUNCTION) were legacy JSON-AST bridges. ZshCompiler
// emits BUILTIN_EXPAND_TEXT (314) and BUILTIN_REGISTER_COMPILED_FN
// (305) instead. The IDs stay reserved in this gap so future builtins
// don't reuse them.

/// Builtin ID for `${name}` reads — routes through `ShellExecutor::get_variable`
/// which knows about special params (`$?`, `$@`, `$#`, `$1..$9`), shell vars
/// (`self.variables`), arrays, and env. Replaces emission of `Op::GetVar` for
/// shell variable names so nested VMs (function calls) see the same storage.
pub const BUILTIN_GET_VAR: u16 = 283;

/// Builtin ID for `name=value` assignments — pops [name, value] and stores
/// into `executor.variables`. Replaces `Op::SetVar` emission for the same
/// reason: the storage must be visible to both bytecode and tree-walker code,
/// across nested VM boundaries.
pub const BUILTIN_SET_VAR: u16 = 284;

/// Builtin ID for pipeline execution. Pops N sub-chunk indices from the stack;
/// each index points into `vm.chunk.sub_chunks` (compiled stage bodies). Forks
/// N children, wires stdin/stdout between them via pipes, runs each stage's
/// bytecode on a fresh VM in its child, parent waits for all and pushes the
/// last stage's exit status. This is bytecode-native pipeline execution —
/// no tree-walker delegation.
pub const BUILTIN_RUN_PIPELINE: u16 = 285;

/// Builtin ID for `Array → String` joining. Pops one value: if it's an Array,
/// joins its string-coerced elements with a single space; otherwise passes
/// through. Used after `Op::Glob` to convert the pattern's matched paths into
/// the single argv-token form the bytecode word model expects (no per-word
/// splitting yet — that's a future phase).
pub const BUILTIN_ARRAY_JOIN: u16 = 286;

/// Builtin ID for `cmd &` background execution. IDs 287/288/289 are reserved
/// for the planned array work in Phase G1 (SET_ARRAY/SET_ASSOC/ARRAY_INDEX),
/// so this lands at 290. Pops one sub-chunk index; forks; child detaches
/// (`setsid`), runs the sub-chunk on a fresh VM, exits with last_status; parent
/// returns Status(0) immediately. Job-table registration (so `jobs`/`fg`/`wait`
/// can see the pid) is deferred to Phase G6 — fire-and-forget for now.
pub const BUILTIN_RUN_BG: u16 = 290;

/// Indexed-array assignment: `arr=(a b c)`. Compile_simple emits N element
/// pushes followed by name push, then `CallBuiltin(BUILTIN_SET_ARRAY, N+1)`.
/// The handler pops args (last popped = name in our pushing order) and stores
/// `Vec<String>` into `executor.arrays`. Tree-walker callers see the same
/// storage. Any prior scalar binding in `executor.variables` for `name` is
/// removed so `${name}` (scalar context) consistently reflects the array's
/// first element via `get_variable`.
pub const BUILTIN_SET_ARRAY: u16 = 287;

/// Single-key set on an associative array: `foo[key]=val`. Stack (top-down):
/// [name, key, value]. Stores `value` into `executor.assoc_arrays[name][key]`,
/// creating the outer entry if missing. compile_simple detects `var[...]=...`
/// in assignments and emits this builtin.
pub const BUILTIN_SET_ASSOC: u16 = 288;

/// `${arr[idx]}` — single-element array index. Pops two args:
///   stack: [name, idx_str]
/// Returns the indexed element as Value::str. Indexing semantics: zsh is
/// 1-based by default; bash is 0-based. We follow zsh.
/// Special idx values: `@` and `*` return the whole array as Value::Array
/// (which fuses correctly via the Op::Exec splice for argv splice).
pub const BUILTIN_ARRAY_INDEX: u16 = 289;

/// `${#arr[@]}` and `${#arr}` (when arr is an array name) — array length.
/// Pops one arg: name. Returns Value::str of len.

/// `${arr[@]}` — splice all elements as a Value::Array. Pops one arg: name.
/// The Array gets flattened by Op::Exec/ExecBg/CallFunction into argv.
pub const BUILTIN_ARRAY_ALL: u16 = 292;

/// Flatten one level of Value::Array nesting. Pops N values; for each, if it's
/// a Value::Array, its elements are appended directly; otherwise the value is
/// appended as-is. Pushes a single Value::Array of the flattened result. Used
/// by the for-loop word-list compile path: when a word like `${arr[@]}`
/// produces a nested Array, this lets `for i in ${arr[@]}` iterate over the
/// inner elements rather than the outer single-element array.
pub const BUILTIN_ARRAY_FLATTEN: u16 = 293;

/// `coproc [name] { body }` — bidirectional pipe to async child. Pops a name
/// (optional, "" for default) and a sub-chunk index. Creates two pipes, forks,
/// child redirects its fd 0/1 to the inner ends and runs the body, parent
/// stores [write_fd, read_fd] into the named array (default `COPROC`). Caller
/// closes the fds and `wait`s when done. Job-table integration deferred to
/// Phase G6 alongside the bg `&` work.
pub const BUILTIN_RUN_COPROC: u16 = 294;

/// `arr+=(d e f)` — append N elements to an existing indexed array. Compile
/// emits N element pushes + name push, then `CallBuiltin(295, N+1)`. Handler
/// drains args (last popped = name), extends `executor.arrays[name]` (creates
/// the entry if missing). Mirrors zsh's `+=` semantics for indexed arrays.
pub const BUILTIN_APPEND_ARRAY: u16 = 295;

/// `select var in words; do body; done` — interactive numbered-menu loop.
/// Compile emits N word pushes + var-name push + sub-chunk index push, then
/// `CallBuiltin(296, N+2)`. Handler prints `1) word1\n2) word2\n...` to
/// stderr, prints `$PROMPT3` (default `?# `) to stderr, reads a line from
/// stdin. On EOF returns 0. On a valid 1-based number, sets `var` to the
/// chosen word, runs the sub-chunk, then redisplays the menu and loops. On
/// invalid input redraws the menu without running the body. `break` from
/// inside the body exits the loop (handled by the body's own bytecode).
pub const BUILTIN_RUN_SELECT: u16 = 296;

/// `m[k]+=value` — append onto an existing assoc-array value (string concat).
/// If the key doesn't exist, behaves like SET_ASSOC. Stack: [name, key, value].

/// `break` from inside a body that runs on a sub-VM (select, future loop-via-
/// builtin constructs). Sets `executor.loop_signal = Some(LoopSignal::Break)`.
/// Outer-loop builtins drain the flag after each body run and exit early.
pub const BUILTIN_SET_BREAK: u16 = 299;

/// `continue` from inside a sub-VM body. Sets the signal to Continue. Outer
/// loop builtins drain + skip-to-next-iteration.
pub const BUILTIN_SET_CONTINUE: u16 = 300;

/// Brace expansion: `{a,b,c}` → 3 values, `{1..5}` → 5 values, `{01..05}` →
/// zero-padded numerics, `{a..e}` → letter range. Pops one string, returns
/// Value::Array of expansions (empty array → original string preserved).
pub const BUILTIN_BRACE_EXPAND: u16 = 301;

/// Glob qualifier filter: `*(qualifier)` filters glob results by predicate.
/// Pops [pattern, qualifier_string]. Returns Value::Array of matching paths.

/// Re-export the regex_match host method as a builtin so `[[ s =~ pat ]]`
/// works even when fusevm's Op::RegexMatch isn't routed (compat fallback).

/// Word-split a string on IFS (default: whitespace). Pops one string,
/// returns Value::Array of fields. Used in array-literal context where
/// `arr=($(cmd))` should expand cmd's stdout into multiple elements.
pub const BUILTIN_WORD_SPLIT: u16 = 304;

/// Register a pre-compiled fusevm chunk as a function. Stack: [name,
/// base64-bincode-of-Chunk]. Used by compile_zsh's compile_funcdef to
/// register functions parsed via parse_init+parse without going through the
/// ShellCommand JSON serialization path.
pub const BUILTIN_REGISTER_COMPILED_FN: u16 = 305;
pub const BUILTIN_VAR_EXISTS: u16 = 306;
/// Phase 1 native param-modifier builtins. Each takes a fixed argv shape
/// and returns the modified value as Value::Str. Replaces the runtime
/// ShellWord round-trip via BUILTIN_EXPAND_WORD_RUNTIME for the common
/// shapes.
///
/// `${var:-default}` / `${var:=default}` / `${var:?error}` / `${var:+alt}`
/// — pop [name, op_byte, rhs]. op_byte: 0=`:-`, 1=`:=`, 2=`:?`, 3=`:+`.
pub const BUILTIN_PARAM_DEFAULT_FAMILY: u16 = 307;
/// `${var:offset[:length]}` — pop [name, offset, length] (length=-1 means
/// "rest of value"; negative offset counts from end).
pub const BUILTIN_PARAM_SUBSTRING: u16 = 308;
/// `${var#pat}` / `${var##pat}` / `${var%pat}` / `${var%%pat}` — pop
/// [name, pattern, op_byte]. op_byte: 0=`#`, 1=`##`, 2=`%`, 3=`%%`.
pub const BUILTIN_PARAM_STRIP: u16 = 309;
/// `${var/pat/repl}` / `${var//pat/repl}` / `${var/#pat/repl}` /
/// `${var/%pat/repl}` — pop [name, pattern, replacement, op_byte].
/// op_byte: 0=first, 1=all, 2=anchor-prefix, 3=anchor-suffix.
pub const BUILTIN_PARAM_REPLACE: u16 = 310;
/// `${#name}` — character length of a scalar value, or element count
/// of an indexed/assoc array. Pops \[name\], returns count as Value::Str.
pub const BUILTIN_PARAM_LENGTH: u16 = 311;
/// `$((expr))` arithmetic substitution. Pops \[expr_string\], evaluates
/// via the executor's MathEval (integer-aware), returns result as
/// Value::Str. Bypasses ArithCompiler's float-only Op::Div path so
/// `$((10/3))` returns "3" not "3.333...".
pub const BUILTIN_ARITH_EVAL: u16 = 312;
/// `$(cmd)` command substitution. Pops \[cmd_string\], runs through
/// `run_command_substitution` which compiles via parse_init+parse + ZshCompiler
/// and captures stdout via an in-process pipe. Returns trimmed output
/// as Value::Str. Avoids the sub-chunk word-emit quoting bug in the
/// raw Op::CmdSubst path.
pub const BUILTIN_CMD_SUBST_TEXT: u16 = 313;
/// Text-based word expansion. Pops \[preserved_text\]: the word with
/// quotes preserved (Dnull→`"`, Snull→`'`, Bnull→`\`), runs
/// `expand_string` (variable + cmd-sub + arith) then `xpandbraces`
/// then `expand_glob`. Returns Value::str (single match) or
/// Value::Array (multi-match brace/glob).
pub const BUILTIN_EXPAND_TEXT: u16 = 314;

/// `[[ a -ef b ]]` — same-inode test. Stack: [a, b]. Pushes Bool true iff
/// both paths resolve to the same `(dev, inode)` pair (zsh + bash semantics).
pub const BUILTIN_SAME_FILE: u16 = 315;

/// `[[ a -nt b ]]` — file `a` newer than file `b` (mtime strict).
/// Stack: [path_a, path_b]. Pushes Bool. zsh-compatible "missing"
/// rules: if both exist, compare mtime; if only `a` exists → true;
/// otherwise false.
pub const BUILTIN_FILE_NEWER: u16 = 324;

/// `[[ a -ot b ]]` — mirror of `-nt`. If both exist, compare mtime;
/// if only `b` exists → true; otherwise false.
pub const BUILTIN_FILE_OLDER: u16 = 325;

/// `[[ -k path ]]` — sticky bit (S_ISVTX) set on path.
pub const BUILTIN_HAS_STICKY: u16 = 326;
/// `[[ -u path ]]` — setuid bit (S_ISUID).
pub const BUILTIN_HAS_SETUID: u16 = 327;
/// `[[ -g path ]]` — setgid bit (S_ISGID).
pub const BUILTIN_HAS_SETGID: u16 = 328;
/// `[[ -O path ]]` — owned by effective UID.
pub const BUILTIN_OWNED_BY_USER: u16 = 329;
/// `[[ -G path ]]` — owned by effective GID.
pub const BUILTIN_OWNED_BY_GROUP: u16 = 330;
/// `[[ -N path ]]` — file modified since last accessed (atime <= mtime).
pub const BUILTIN_FILE_MODIFIED_SINCE_ACCESS: u16 = 341;

/// `name+=val` (no parens) — runtime-dispatched append.
/// If name is an indexed array → push val as element.
/// If name is an assoc array → error (zsh requires `(k v)` form).
/// Else → scalar concat (existing SET_VAR behavior).
pub const BUILTIN_APPEND_SCALAR_OR_PUSH: u16 = 331;

/// `[[ -c path ]]` — character device.
pub const BUILTIN_IS_CHARDEV: u16 = 332;
/// `[[ -b path ]]` — block device.
pub const BUILTIN_IS_BLOCKDEV: u16 = 333;
/// `[[ -p path ]]` — FIFO / named pipe.
pub const BUILTIN_IS_FIFO: u16 = 334;
/// `[[ -S path ]]` — socket.
pub const BUILTIN_IS_SOCKET: u16 = 335;
pub const BUILTIN_ERREXIT_CHECK: u16 = 336;
pub const BUILTIN_PARAM_SUBSTRING_EXPR: u16 = 337;
pub const BUILTIN_XTRACE_LINE: u16 = 338;
pub const BUILTIN_ARRAY_JOIN_STAR: u16 = 339;
pub const BUILTIN_SET_RAW_OPT: u16 = 340;

/// `time { compound; ... }` — wall-clock-time the sub-chunk and print
/// elapsed seconds. Stack: [sub_chunk_idx as Int]. Runs the sub-chunk
/// on the current VM (so positional/local state is shared) and prints
/// the timing summary to stderr in zsh's format. Pushes Status.
pub const BUILTIN_TIME_SUBLIST: u16 = 316;

/// `{name}>file` / `{name}<file` / `{name}>>file` — named-fd allocation.
/// Stack: [path, varid, op_byte]. Opens `path` per `op_byte`, gets the
/// new fd (≥10 in zsh; we use libc::open with O_CLOEXEC bit cleared so
/// the inherited fd survives Command::new spawns), stores the fd number
/// as a string in `$varid`. Pushes Status (0 success, 1 error).
pub const BUILTIN_OPEN_NAMED_FD: u16 = 317;

/// Word-segment concat that does cartesian-product distribution over
/// arrays. Stack: [lhs, rhs]. Used for RC_EXPAND_PARAM `${arr}` and
/// explicit-distribute forms (`${^arr}`, `${(@)…}`).
///
/// - both scalar: `Value::str(a + b)` (fast path, identical to Op::Concat)
/// - lhs Array, rhs scalar: `Value::Array([a + rhs for a in lhs])`
/// - lhs scalar, rhs Array: `Value::Array([lhs + b for b in rhs])`
/// - both Array: cartesian product `[a + b for a in lhs for b in rhs]`
pub const BUILTIN_CONCAT_DISTRIBUTE: u16 = 318;

/// Forced-distribute concat — like `BUILTIN_CONCAT_DISTRIBUTE` but
/// always distributes cartesian regardless of the `rcexpandparam`
/// option. Emitted by the segments fast-path when an
/// `is_distribute_expansion` segment is present (`${^arr}`,
/// `${(@)arr}`, `${(s.…)arr}` etc.) per zsh: the source-level
/// distribution flag overrides the option default.
/// Direct port of Src/subst.c:1875 `case Hat: nojoin = 1` and the
/// `rcexpandparam` test bypass for the explicit-distribute flags.
pub const BUILTIN_CONCAT_DISTRIBUTE_FORCED: u16 = 522;

/// Capture current `last_status` into the `TRY_BLOCK_ERROR` variable.
/// Emitted between the try block and the always block of `{ … } always
/// { … }` so the finally arm can read $TRY_BLOCK_ERROR.
pub const BUILTIN_SET_TRY_BLOCK_ERROR: u16 = 320;
pub const BUILTIN_RESTORE_TRY_BLOCK_STATUS: u16 = 432;
pub const BUILTIN_BEGIN_INLINE_ENV: u16 = 433;
pub const BUILTIN_END_INLINE_ENV: u16 = 434;

/// `[[ -o option ]]` — shell-option-set test. Stack: \[option_name\].
/// Normalizes the name (strip underscores, lowercase) and reads
/// `exec.options`. Pushes Bool.
pub const BUILTIN_OPTION_SET: u16 = 321;

/// `${var:#pattern}` — array filter: remove elements matching `pattern`.
/// Stack: [name, pattern]. For scalar `var`, returns empty if it matches
/// the pattern, else the value. For array `var`, returns Array of
/// non-matching elements.
pub const BUILTIN_PARAM_FILTER: u16 = 322;

/// `a[i]=(elements)` / `a[i,j]=(elements)` / `a[i]=()` —
/// subscripted-array assign with array-literal RHS. Stack:
/// [...elements, name, key]. Empty elements + single-int key `a[i]=()`
/// removes that element. Comma-key `a[i,j]=(...)` splices.
pub const BUILTIN_SET_SUBSCRIPT_RANGE: u16 = 323;

/// `[[ -X file ]]` for unknown unary test op `-X`. Stack: \[op_name\].
/// Emits zsh's `unknown condition: -X` diagnostic to stderr and
/// pushes Bool(false). Without this, unknown conditions silently
/// returned false matching neither zsh's error format nor the
/// expected status code (zsh returns 2 for parse error).

/// `[[ -t fd ]]` — fd-is-a-tty check. Stack: \[fd_string\].
/// Routes through libc::isatty. Pushes Bool.
pub const BUILTIN_IS_TTY: u16 = 325;

/// Update `$LINENO` to track the source line of the next statement.
/// Stack: \[n\] (the line number from `ZshPipe.lineno`). Direct port
/// of zsh's `lineno` global tracking (Src/input.c:330) — the
/// compiler emits one of these per top-level pipe so `$LINENO`
/// reflects the source position at runtime. ID 342 picked because
/// the previous `326` collided with `BUILTIN_HAS_STICKY` (the file
/// has several other duplicate IDs — 325 has two as well — but
/// fixing those is out of scope for this port).
pub const BUILTIN_SET_LINENO: u16 = 342;

/// Pop a scalar from the VM stack, run expand_glob on it, push the
/// result as Value::Array. Used by the segment-concat compile path
/// when var refs concatenate with glob meta literals (`$D/*`,
/// `${prefix}*`, etc.) — those skip the bridge's pathname-expansion
/// pass and would otherwise leak the glob meta to argv as a literal.
pub const BUILTIN_GLOB_EXPAND: u16 = 343;

/// Push a `CmdState` token onto the command-context stack. Direct
/// port of zsh's `cmdpush(int cmdtok)` (Src/prompt.c:1623). The
/// stack is consulted by `%_` in PS4/prompt expansion to produce
/// the cumulative control-flow-context labels (`if`, `then`,
/// `cmdand`, `cmdor`, `cmdsubst`, …) that `zsh -x` xtrace shows
/// in the trace prefix. Compile_zsh emits push/pop pairs around
/// each compound command (if/while/[[…]]/((…))/$(…) etc.).
/// Token is a `CmdState as u8`.
pub const BUILTIN_CMD_PUSH: u16 = 344;

/// Pop the top of the command-context stack. Direct port of zsh's
/// `cmdpop(void)` (Src/prompt.c:1631).
pub const BUILTIN_CMD_POP: u16 = 345;

/// Emit an xtrace line built from the top `argc` values on the VM
/// stack, peeked WITHOUT consuming. Used to trace simple commands
/// AFTER expansion, so `echo for $i` shows as `echo for a` / `echo
/// for b`. Direct port of Src/exec.c:2055-2066.
pub const BUILTIN_XTRACE_ARGS: u16 = 346;

/// Trace one assignment: emits `name=<quoted-value> ` (no newline)
/// to xtrerr if XTRACE is on. Coalesces with subsequent
/// XTRACE_ASSIGN / XTRACE_ARGS calls onto the SAME line via the
/// `XTRACE_DONE_PS4` flag so `a=1 b=2 echo $a $b` produces:
///   `<PS4>a=1 b=2 echo 1 2\n`
/// matching C zsh's `execcmd_exec` body (Src/exec.c:2517-2582):
///   xtr = isset(XTRACE);
///   if (xtr) { printprompt4(); doneps4 = 1; }
///   while (assign) {
///       if (xtr) fprintf(xtrerr, "%s=", name);
///       ... eval value ...
///       if (xtr) { quotedzputs(val, xtrerr); fputc(' ', xtrerr); }
///   }
///
/// Stack contract on entry: [..., name, value]. Both peeked, NOT
/// consumed (the matching SET_VAR call pops them after). argc = 2.
pub const BUILTIN_XTRACE_ASSIGN: u16 = 525;

/// Emit a trailing `\n` + flush iff XTRACE is on AND PS4 was
/// emitted by an earlier XTRACE_ASSIGN this line. Used at the end
/// of compile_simple's assignment-only path so the trace line gets
/// terminated. Mirrors C's exec.c:3397-3399 (the assign-only return
/// path through execcmd_exec which does `fputc('\n', xtrerr);
/// fflush(xtrerr)`).
///
/// Stack: untouched. argc = 0.
pub const BUILTIN_XTRACE_NEWLINE: u16 = 526;

/// Bridge into subst_port::substitute_brace_array for nested forms
/// that need to PRESERVE array shape across the expand_string
/// boundary. Stack: `[content_string]`. Returns Value::Array of the
/// per-element words. Used by the compile path for
/// `${(@)<nested>...##pat}` shapes — the standard substitute_brace
/// returns String which collapses array→scalar; this builtin
/// preserves the multi-word output via paramsubst's third return
/// (`nodes` vec, the C source's `aval` thread).
pub const BUILTIN_BRIDGE_BRACE_ARRAY: u16 = 347;

/// Word-segment concat with FIRST/LAST sticking. Stack: [lhs, rhs].
/// Used for default unquoted splice forms (`${arr[@]}`, `$@`, `$*`)
/// where prefix sticks to first element only and suffix to last only.
///
/// Distribution table:
/// - both scalar: `Value::str(a + b)` (fast path)
/// - lhs scalar, rhs Array(b₀..bₙ): `Value::Array([lhs+b₀, b₁, …, bₙ])`
/// - lhs Array(a₀..aₙ), rhs scalar: `Value::Array([a₀, …, aₙ₋₁, aₙ+rhs])`
/// - both Array: `Value::Array([a₀, …, aₙ₋₁, aₙ+b₀, b₁, …, bₙ])`
///   (last of lhs merges with first of rhs; the rest stay separate)
///
/// This is the default zsh semantics for `print -l X${arr[@]}Y` →
/// "Xa", "b", "cY" — three distinct args, surrounding text only on ends.
pub const BUILTIN_CONCAT_SPLICE: u16 = 319;

/// `${(flags)name}` — zsh parameter expansion flags. Stack: [name, flags].
/// Flags applied left-to-right. Supported subset (high-value, used by zpwr):
///
///   `L` — lowercase the value (scalar; or each element if array)
///   `U` — uppercase
///   `j:sep:` — join array with `sep` (delim is the char after `j`)
///   `s:sep:` — split scalar on `sep` (returns Value::Array)
///   `f` — split on newlines (shorthand for `s.\n.`)
///   `o` — sort array ascending
///   `O` — sort array descending
///   `P` — indirect: read name's value as another var name, return that's value
///   `@` — keep as array (returns Value::Array — useful before `j` etc.)
///   `k` — keys of assoc array
///   `v` — values of assoc array
///   `#` — word count (array length as scalar)
///
/// Flags can stack: `(jL)` joins then lowercases; `(s.,.U)` splits on `,`
/// then uppercases each element. The long-tail flags (`q`, `qq`, `qqq` for
/// quoting, `A` for assoc, `%` for prompt expansion, `e`/`g` for re-eval,
/// `n`/`p` for numeric, `t` for type, etc.) are deferred — they hit the
/// runtime fallback via the catch-all expansion path.
pub const BUILTIN_PARAM_FLAG: u16 = 297;

/// `ShellHost` implementation that delegates to the current `ShellExecutor`
/// via the `with_executor` thread-local.
///
/// Construct fresh on each VM run (it carries no state itself). The VM
/// dispatches host method calls during `vm.run()`, and `with_executor`
/// resolves to the executor pointer set by `ExecutorContext::enter`.
/// fusevm-host implementation tying bytecode ops to the
/// shell executor.
/// zshrs-original — no C counterpart. C zsh has no bytecode VM
/// to host; everything runs through `execlist()`/`execpline()`
/// directly (Src/exec.c lines 1349/1668).
pub struct ZshrsHost;

impl fusevm::ShellHost for ZshrsHost {
    fn glob(&mut self, pattern: &str, _recursive: bool) -> Vec<String> {
        with_executor(|exec| exec.expand_glob(pattern))
    }

    fn tilde_expand(&mut self, s: &str) -> String {
        with_executor(|exec| s.to_string())
    }

    fn brace_expand(&mut self, s: &str) -> Vec<String> {
        // Direct call to the canonical brace expander
        // (Src/glob.c::xpandbraces port at glob.rs:1678). Was
        // routing through singsub which uses PREFORK_SINGLE — that
        // flag explicitly suppresses brace expansion in subst.c:166,
        // so `print X{1,2,3}Y` returned the literal string.
        //
        // brace_ccl: respect the BRACE_CCL option which the bracket-
        // class form `{a-z}` requires. Pull from executor options.
        let brace_ccl = with_executor(|exec| {
            crate::ported::options::opt_state_get("braceccl").unwrap_or(false)
        });
        crate::ported::glob::xpandbraces(s, brace_ccl)
    }

    fn str_match(&mut self, s: &str, pattern: &str) -> bool {
        // Shell glob match — `*`, `?`, `[...]`, alternation. Used by `[[ x = pat ]]`,
        // `case` arms, and any other point that compares against a glob pattern.
        // PURE PASSTHRU to canonical pattern::patmatch (C port of
        // Src/pattern.c::pattry); args reversed since patmatch takes
        // (pattern, text).
        crate::ported::pattern::patmatch(pattern, s)
    }

    fn expand_param(
        &mut self,
        name: &str,
        _modifier: u8,
        _args: &[fusevm::Value],
    ) -> fusevm::Value {
        // Sole funnel: route through `getsparam` matching C zsh's
        // `getsparam(name)` → `getvalue` → `getstrvalue` →
        // `Param.gsu->getfn` dispatch (Src/params.c:3076 / 2335).
        //
        // The lookup chain (GSU dispatch + variables + env + array-
        // join) lives in `params::getsparam`; subst.rs and this
        // bridge both call into it so the logic is in exactly one
        // place — mirroring C's "every read goes through getsparam"
        // architecture. fuseVM bytecode triggers this bridge when
        // the VM hits a PARAM opcode, equivalent to C's wordcode VM
        // resolving a parameter read during `exec.c` execution.
        //
        // Modifier handling: the `_modifier` / `_args` parameters
        // are populated by the bytecode compiler but applied by
        // separate VM opcodes (LENGTH/STRIP/SUBST/etc.) downstream
        // of this fetch — matching C's split between getsparam
        // (value fetch) and paramsubst's modifier-walk loop. This
        // bridge is the value-fetch step only.
        let val_str = crate::ported::params::getsparam(name).unwrap_or_default();
        fusevm::Value::str(val_str)
    }

    fn process_sub_in(&mut self, sub: &fusevm::Chunk) -> String {
        // Run the sub-chunk synchronously (in the current executor context),
        // capture stdout into a temp file, return the path. Synchronous is
        // simpler and avoids the thread-local-executor limitation that
        // spawned threads can't see. Common consumers (`diff`, `cat`,
        // `comm`) read the file once anyway.
        let fifo_path = format!(
            "/tmp/zshrs_psub_{}_{}",
            std::process::id(),
            with_executor(|e| {
                let n = e.process_sub_counter;
                e.process_sub_counter += 1;
                n
            })
        );
        let _ = std::fs::remove_file(&fifo_path);
        let f = match std::fs::File::create(&fifo_path) {
            Ok(f) => f,
            Err(_) => return fifo_path,
        };
        let saved = unsafe { libc::dup(libc::STDOUT_FILENO) };
        unsafe {
            libc::dup2(f.as_raw_fd(), libc::STDOUT_FILENO);
        }
        crate::fusevm_disasm::maybe_print_stdout("process_subst_in", sub);
        let mut vm = fusevm::VM::new(sub.clone());
        register_builtins(&mut vm);
        vm.set_shell_host(Box::new(ZshrsHost));
        let _ = vm.run();
        let _ = std::io::stdout().flush();
        unsafe {
            libc::dup2(saved, libc::STDOUT_FILENO);
            libc::close(saved);
        }
        fifo_path
    }

    fn process_sub_out(&mut self, sub: &fusevm::Chunk) -> String {
        // `>(cmd)` — consumer reads stdin from a FIFO that the parent
        // writes to. Create a real named pipe, fork a child that
        // dup2s the read end onto stdin and runs the sub-chunk; return
        // the FIFO path to the parent so it writes there.
        let fifo_path = format!(
            "/tmp/zshrs_psub_out_{}_{}",
            std::process::id(),
            with_executor(|e| {
                let n = e.process_sub_counter;
                e.process_sub_counter += 1;
                n
            })
        );
        let _ = std::fs::remove_file(&fifo_path);
        let cpath = match CString::new(fifo_path.clone()) {
            Ok(c) => c,
            Err(_) => return fifo_path,
        };
        if unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) } != 0 {
            // Fall back to plain file if mkfifo fails.
            let _ = std::fs::write(&fifo_path, "");
            return fifo_path;
        }
        let sub = sub.clone();
        let fifo_for_child = fifo_path.clone();
        match unsafe { libc::fork() } {
            -1 => {
                let _ = std::fs::remove_file(&fifo_path);
            }
            0 => {
                // Child: open FIFO for read, dup onto stdin, run sub-chunk, exit.
                if let Ok(f) = std::fs::OpenOptions::new().read(true).open(&fifo_for_child) {
                    let fd = f.as_raw_fd();
                    unsafe {
                        libc::dup2(fd, libc::STDIN_FILENO);
                    }
                }
                crate::fusevm_disasm::maybe_print_stdout("process_subst_out:child", &sub);
                let mut vm = fusevm::VM::new(sub);
                register_builtins(&mut vm);
                vm.set_shell_host(Box::new(ZshrsHost));
                let _ = vm.run();
                unsafe { libc::_exit(0) };
            }
            _ => {
                // Parent — return path; child handles cleanup of the FIFO
                // once stdin EOFs. (The path may leak if the parent never
                // writes; acceptable for common `>(cmd)` idioms.)
            }
        }
        fifo_path
    }

    fn subshell_begin(&mut self) {
        with_executor(|exec| {
            // libc::umask returns the previous mask AND sets the new
            // one; call with current value to read without changing.
            let cur_umask = unsafe {
                let m = libc::umask(0o022);
                libc::umask(m);
                m as u32
            };
            // Snapshot paramtab + hashed-storage too (step 1 of the
            // store unification mirrors writes there; restoring only
            // the HashMaps leaks subshell-scoped writes to the parent
            // via paramtab readers like `paramsubst → vars_get`).
            let paramtab_snap = crate::ported::params::paramtab()
                .read()
                .ok()
                .map(|t| t.clone())
                .unwrap_or_default();
            let paramtab_hashed_snap = crate::ported::params::paramtab_hashed_storage()
                .lock()
                .ok()
                .map(|m| m.clone())
                .unwrap_or_default();
            exec.subshell_snapshots.push(SubshellSnapshot {
                paramtab: paramtab_snap,
                paramtab_hashed_storage: paramtab_hashed_snap,
                positional_params: exec.pparams(),
                env_vars: std::env::vars().collect(),
                // Save the LOGICAL pwd ($PWD env), not `current_dir()`'s
                // symlink-resolved path. zsh's subshell isolation per
                // Src/exec.c at the `entersubsh` path treats `pwd` (the
                // shell-tracked logical PWD) as the carrier — see
                // `Src/builtin.c:1239-1242` where cd writes the logical
                // dest into `pwd`. Falling back to current_dir() only
                // when PWD is unset matches `setupvals` at
                // `Src/init.c:1100+`.
                cwd: std::env::var("PWD")
                    .ok()
                    .map(std::path::PathBuf::from)
                    .or_else(|| std::env::current_dir().ok()),
                umask: cur_umask,
                traps: exec.traps.clone(),
            });
            // Subshell starts with EXIT trap cleared so the parent's
            // EXIT handler doesn't fire when the subshell ends. zsh:
            // each subshell has its own trap context. Other signals
            // are inherited (well, parent's are still in place — but
            // a trap set INSIDE the subshell shouldn't leak out).
            exec.traps.remove("EXIT");
            let level = exec
                .scalar("ZSH_SUBSHELL")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0);
            exec.set_scalar("ZSH_SUBSHELL".to_string(), (level + 1).to_string());
        });
    }

    fn subshell_end(&mut self) {
        // Fire subshell's EXIT trap BEFORE restoring parent state so
        // the trap body sees the subshell's vars and exit status. zsh
        // forks for `(...)` so the trap runs in the child process,
        // before exit. We mirror by running it here, just before the
        // pop+restore. REMOVE the trap before firing so the inner
        // execute_script doesn't fire it again at its own end.
        let exit_trap_body = with_executor(|exec| exec.traps.remove("EXIT"));
        if let Some(body) = exit_trap_body {
            // Execute the trap body. Errors during trap execution
            // don't bubble — zsh ignores trap-body errors.
            with_executor(|exec| {
                let _ = exec.execute_script(&body);
            });
        }
        with_executor(|exec| {
            if let Some(snap) = exec.subshell_snapshots.pop() {
                // Restore paramtab + hashed storage so subshell-scoped
                // writes via setsparam/setaparam/sethparam don't leak
                // to the parent via paramtab readers.
                if let Some(tab) = crate::ported::params::paramtab()
                    .write()
                    .ok()
                    .as_deref_mut()
                {
                    *tab = snap.paramtab;
                }
                if let Some(m) = crate::ported::params::paramtab_hashed_storage()
                    .lock()
                    .ok()
                    .as_deref_mut()
                {
                    *m = snap.paramtab_hashed_storage;
                }
                exec.set_pparams(snap.positional_params);
                // Restore the OS env to its pre-subshell state.
                // Removes any `export` writes the subshell made, and
                // restores any vars the subshell unset. Without this
                // `(export y=sub)` would leak `y` to the parent shell.
                let current: HashMap<String, String> = std::env::vars().collect();
                for k in current.keys() {
                    if !snap.env_vars.contains_key(k) {
                        std::env::remove_var(k);
                    }
                }
                for (k, v) in &snap.env_vars {
                    if current.get(k) != Some(v) {
                        std::env::set_var(k, v);
                    }
                }
                if let Some(cwd) = snap.cwd {
                    let _ = std::env::set_current_dir(&cwd);
                    // Resync $PWD env so a parent `pwd` doesn't read
                    // the cwd the subshell `cd`'d into.
                    std::env::set_var("PWD", &cwd);
                }
                // Restore umask. zsh's `(umask 077)` doesn't leak to
                // parent because the subshell forks; we run in-process
                // so we manually reset.
                unsafe {
                    libc::umask(snap.umask as libc::mode_t);
                }
                // Restore parent's traps (the subshell's own traps die
                // with it). zsh: `(trap "X" USR1)` doesn't leak the
                // USR1 trap out of the subshell.
                exec.traps = snap.traps;
            }
        });
    }

    fn redirect(&mut self, fd: u8, op: u8, target: &str) {
        // Apply a redirection at the OS level for the next command/builtin.
        // The host tracks saved fds in a per-executor stack so a future
        // `with_redirects_end` can restore. For now, this is a thin wrapper
        // that performs the dup2; pairing with explicit save/restore is
        // delivered by `with_redirects_begin/end`.
        with_executor(|exec| exec.host_apply_redirect(fd, op, target));
    }

    fn with_redirects_begin(&mut self, count: u8) {
        with_executor(|exec| exec.host_redirect_scope_begin(count));
    }

    fn with_redirects_end(&mut self) {
        with_executor(|exec| exec.host_redirect_scope_end());
    }

    fn heredoc(&mut self, content: &str) {
        // C `Src/exec.c:4641` — `parsestr(&buf)` runs parameter +
        // command substitution on the heredoc body. The lexer's
        // quoted-delimiter detection (`<<'EOF'`) routes through the
        // `Op::HereDoc` path in `compile_zsh.rs` which short-circuits
        // before reaching here; unquoted forms route through the
        // BUILTIN_EXPAND_TEXT mode-4 emit path that calls singsub.
        // This handler covers the verbatim/quoted case.
        with_executor(|exec| exec.host_set_pending_stdin(content.to_string()));
    }

    fn herestring(&mut self, content: &str) {
        // Shell semantics: herestring appends a newline. `<<<` body
        // substitution (`Src/exec.c:4655 getherestr` calls
        // `quotesubst` + `untokenize`) lands here verbatim; the
        // upstream compiler routes through `Op::HereString` after
        // BUILTIN_EXPAND_TEXT for the substitution pass, so callers
        // of `host.herestring` see the already-expanded form.
        let mut s = content.to_string();
        s.push('\n');
        with_executor(|exec| exec.host_set_pending_stdin(s));
    }

    fn exec(&mut self, args: Vec<String>) -> i32 {
        // Track `$_` as the last argument of the last command (zsh /
        // bash convention). Empty arglists leave it untouched.
        if let Some(last) = args.last() {
            with_executor(|exec| {
                exec.set_scalar("_".to_string(), last.clone());
            });
        }
        // Route external command spawning through `executor.execute_external`
        // so intercepts (AOP before/after/around), command_hash lookups,
        // pre/postexec hooks, and zsh-specific fork-then-exec all apply.
        // Without this override, fusevm's default `host.exec` calls
        // `Command::new` directly, bypassing zshrs's dispatch logic.
        with_executor(|exec| exec.host_exec_external(&args))
    }

    fn cmd_subst(&mut self, sub: &fusevm::Chunk) -> String {
        // Run the sub-chunk on a nested VM with the same host wired up,
        // capturing stdout. The current executor remains active via the
        // thread-local — the nested VM uses CallBuiltin to dispatch shell
        // ops back through `with_executor`.
        let (read_end, write_end) = match os_pipe::pipe() {
            Ok(p) => p,
            Err(_) => return String::new(),
        };
        let saved_stdout = unsafe { libc::dup(libc::STDOUT_FILENO) };
        if saved_stdout < 0 {
            return String::new();
        }
        let write_fd = std::os::unix::io::AsRawFd::as_raw_fd(&write_end);
        unsafe {
            libc::dup2(write_fd, libc::STDOUT_FILENO);
        }
        drop(write_end);

        crate::fusevm_disasm::maybe_print_stdout("host.cmd_subst", sub);
        let mut vm = fusevm::VM::new(sub.clone());
        register_builtins(&mut vm);
        vm.set_shell_host(Box::new(ZshrsHost));
        let _ = vm.run();
        let cmd_status = vm.last_status;

        unsafe {
            libc::dup2(saved_stdout, libc::STDOUT_FILENO);
            libc::close(saved_stdout);
        }

        // Inner cmd's status not propagated for the same reason as
        // run_command_substitution — see GAPS.md.
        let _ = cmd_status;

        let mut buf = String::new();
        let mut reader = read_end;
        let _ = reader.read_to_string(&mut buf);
        // Strip trailing newlines (POSIX command substitution semantics)
        while buf.ends_with('\n') {
            buf.pop();
        }
        buf
    }

    fn call_function(&mut self, name: &str, args: Vec<String>) -> Option<i32> {
        // zsh-bundled rename helpers + zcalc: short-circuit BEFORE the
        // function/autoload lookup so the autoloaded zsh source (which
        // can hang zshrs's parser on zsh-specific syntax) never runs.
        // Native Rust impls live in builtin_zmv / builtin_zcalc.
        match name {
            "zmv" => {
                return Some(crate::extensions::ext_builtins::zmv(&args, "mv"));
            }
            "zcp" => {
                return Some(crate::extensions::ext_builtins::zmv(&args, "cp"));
            }
            "zln" => {
                return Some(crate::extensions::ext_builtins::zmv(&args, "ln"));
            }
            "zcalc" => {
                return Some(crate::extensions::ext_builtins::zcalc(&args));
            }
            // Daemon-managed z* builtins — thin IPC wrappers. Short-circuit BEFORE
            // the function-lookup path so a missing daemon doesn't fall through to
            // "command not found". The name list is owned by the daemon crate
            // (zshrs_daemon::builtins::ZSHRS_BUILTIN_NAMES); routing through
            // try_dispatch keeps this site zero-touch as new z* builtins land.
            n if crate::daemon::builtins::is_zshrs_builtin(n) => {
                let argv: Vec<String> = std::iter::once(name.to_string()).chain(args).collect();
                return Some(crate::daemon::builtins::try_dispatch(n, &argv).unwrap_or(1));
            }
            _ => {}
        }

        // Alias check first: `alias g='echo hi'; g` rewrites to `echo hi`
        // before normal function/external dispatch. The expansion is
        // re-parsed + compiled + run on a nested VM with `args` appended.
        // Without this branch, aliases would be silently ignored at
        // run-time and `g` would fall through to "command not found".
        // Skip when this alias is mid-expansion already — zsh's lexer
        // disables an alias inside its own body (so `alias ls='ls -la'`
        // works without recursion). We do the same via a HashSet guard
        // since we expand at run time, not parse time.
        // C uses the `alias.inuse` field on the alias node itself
        // (`Src/zsh.h:1256` `struct alias { ... int inuse; }`) — the
        // lexer bumps it before splicing the body and clears it after,
        // so a recursive use within the body sees `inuse != 0` and
        // refuses to re-expand. Mirror that here against the canonical
        // `aliastab` instead of a side HashSet on ShellExecutor.
        let already_expanding = crate::ported::hashtable::aliastab_lock()
            .read()
            .ok()
            .and_then(|tab| tab.get(name).map(|a| a.inuse != 0))
            .unwrap_or(false);
        let alias_body = if already_expanding {
            None
        } else {
            with_executor(|exec| exec.alias(name))
        };
        if let Some(body) = alias_body {
            let combined = if args.is_empty() {
                body
            } else {
                let quoted: Vec<String> = args
                    .iter()
                    .map(|a| {
                        let escaped = a.replace('\'', "'\\''");
                        format!("'{}'", escaped)
                    })
                    .collect();
                format!("{} {}", body, quoted.join(" "))
            };
            // Bump inuse → run → clear, matching C's lexer behavior.
            if let Ok(mut tab) = crate::ported::hashtable::aliastab_lock().write() {
                if let Some(a) = tab.get_mut(name) {
                    a.inuse += 1;
                }
            }
            let status = with_executor(|exec| exec.execute_script(&combined).unwrap_or(1));
            if let Ok(mut tab) = crate::ported::hashtable::aliastab_lock().write() {
                if let Some(a) = tab.get_mut(name) {
                    a.inuse = (a.inuse - 1).max(0);
                }
            }
            return Some(status);
        }

        // Resolve to a compiled Chunk:
        //   1. Already in functions_compiled → use as-is
        //   2. AST-only (sourced / defined earlier) → compile on demand
        //   3. Pending autoload → trigger autoload, then retry the AST path
        //   4. Available via fpath ZWC scan → autoload via that, then AST path
        //   5. Not a function → None so fusevm falls back to host.exec
        let chunk = with_executor(|exec| {
            // Autoload pending: the legacy stub in self.functions makes
            // maybe_autoload / autoload_function were deleted with
            // the old exec.c stubs (they were return-false / no-op).
            // The autoload dispatch needs a proper port of
            // `Src/builtin.c:bin_autoload` + `Src/exec.c:loadautofn`.
            // Until that lands, skip the autoload trigger — the eager
            // fpath scan below covers the common interactive case.
            if let Some(c) = exec.functions_compiled.get(name) {
                return Some(c.clone());
            }
            exec.functions_compiled.get(name).cloned()
        });

        let chunk = chunk?;

        // FUNCNEST recursion guard. zsh enforces a max depth
        // (default 500) — past that the call is refused with
        // `<name>: maximum nested function level reached; increase
        // FUNCNEST?` and exit 1. Without this, `foo() { foo; }; foo`
        // overflowed the Rust stack instead of erroring gracefully.
        // zshrs's effective ceiling is lower than zsh's: each
        // `call_function` recursion consumes ~40KB of Rust stack
        // (the bytecode VM is recursive at the host level), so the
        // 8MB default stack tops out around ~150 frames. Cap at 100
        // by default — users with deeper need can raise FUNCNEST
        // explicitly AND run with a larger stack (RUST_MIN_STACK).
        let funcnest_limit = with_executor(|exec| {
            exec.scalar("FUNCNEST")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(100)
        });
        let cur_depth = with_executor(|exec| exec.local_scope_depth);
        if cur_depth >= funcnest_limit {
            eprintln!(
                "{}: maximum nested function level reached; increase FUNCNEST?",
                name
            );
            return Some(1);
        }

        // Save and replace positional params, mirror local-scope save/restore
        // from the tree-walker `call_function`. The thread-local executor
        // pointer set by the outer VM remains valid for the nested VM —
        // nested CallBuiltin handlers and host callbacks all see the same
        // executor.
        let fn_name = name.to_string();
        // Snapshot options at function entry. zsh restores these on
        // exit when `local_options` is set at that time (per zshmisc
        // LOCAL_OPTIONS — `setopt local_options` and `emulate -L
        // ...` both arm the restore). Without this, a function that
        // does `setopt no_glob` to scope an option leaked the change
        // to the caller, breaking p10k/zinit's per-function emulate
        // -L sticky-mode pattern.
        let saved_options = crate::ported::options::opt_state_snapshot();
        let (
            saved_params,
            saved_zero,
            saved_scriptname,
            saved_funcstack,
            saved_exit_trap,
            saved_argzero_global,
        ) = with_executor(|exec| {
                let prev = exec.pparams();
                exec.set_pparams(args.clone());
                exec.local_scope_depth += 1;
                // c:Src/exec.c doshfunc startparamscope() — bump
                // canonical locallevel before the function body runs
                // so any inner `local`/`typeset` writes Params at the
                // right scope. endparamscope at exit restores.
                crate::ported::params::locallevel
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // Save and clear EXIT trap before function body
                // runs. Direct port of zsh's exec.c
                // `dotrapargs(SIGEXIT, ...)` deferred-fire pattern
                // — an EXIT trap set INSIDE a function fires on
                // function return (NOT shell exit), and the outer
                // EXIT trap is preserved across the call. Without
                // this save/restore, `foo() { trap "echo X" EXIT; }`
                // either fired X at SHELL exit (if no outer trap)
                // or polluted the parent's EXIT trap.
                let saved = exec.traps.remove("EXIT");
                // zsh's `$0` inside a function returns the function name
                // (under the FUNCTION_ARGZERO option, default on). Save
                // the previous `$0` and install the function name.
                // Anonymous functions get the cosmetic name `(anon)` —
                // zshrs's parser synthesizes `_zshrs_anon_N` /
                // `_zshrs_anon_kw_N` for `() { … }` and `function { … }`
                // so users would see the internal name otherwise.
                let display_name = if fn_name.starts_with("_zshrs_anon_") {
                    "(anon)".to_string()
                } else {
                    fn_name.clone()
                };
                let prev_zero = crate::ported::params::getsparam("0");
                exec.set_scalar("0".to_string(), display_name.clone());
                // c:Src/exec.c:5903 doshfunc — when FUNCTION_ARGZERO is
                // set (default-on under zsh emulation) the global
                // `argzero` is overwritten with the function name so
                // every `$0` read (which routes through
                // lookup_special_var("0") → argzero()) returns the
                // function name. set_scalar above writes to paramtab,
                // but lookup_special_var short-circuits to argzero()
                // BEFORE consulting paramtab — so without this
                // mirror, `$0` inside `f() { echo $0; }` returned the
                // shell binary path instead of `f`.
                let saved_argzero_global = if crate::ported::zsh_h::isset(
                    crate::ported::zsh_h::FUNCTIONARGZERO,
                ) {
                    let prev = crate::ported::utils::argzero();
                    crate::ported::utils::set_argzero(Some(display_name.clone()));
                    Some(prev)
                } else {
                    None
                };
                // scriptname: PS4's `%N` and error-message prefix both
                // read `exec.scriptname`. Inside a function, C zsh sets
                // `scriptname = dupstring(name)` at Src/exec.c:5903 so
                // `%N` shows the function name. Save the outer
                // scriptname before overwrite; restored on return.
                let prev_scriptname =
                    std::mem::replace(&mut exec.scriptname, Some(display_name.clone()));
                // funcstack: prepend the function name; outermost call
                // is at the END of the stack per zsh.
                let prev_stack = exec.array("funcstack");
                let mut new_stack = vec![fn_name.clone()];
                if let Some(ref s) = prev_stack {
                    new_stack.extend_from_slice(s);
                }
                exec.set_array("funcstack".to_string(), new_stack);
                let line_base = exec.function_line_base.get(&fn_name).copied().unwrap_or(0);
                let def_file = exec.function_def_file.get(&fn_name).cloned().flatten();
                exec.prompt_funcstack
                    .push((fn_name.clone(), line_base, def_file));
                // Set `$_` BEFORE the function body runs. zsh: inside
                // a function, `echo $_` reads the function name (when
                // called with no args) or the last call-arg.
                // Without this, internal builtins that ran before
                // (like REGISTER_COMPILED_FN) leaked their last arg
                // (the function body source!) as $_.
                let dollar_underscore = args.last().cloned().unwrap_or_else(|| fn_name.clone());
                exec.set_scalar("_".to_string(), dollar_underscore.clone());
                exec.pending_underscore = Some(dollar_underscore);
                (prev, prev_zero, prev_scriptname, prev_stack, saved, saved_argzero_global)
            });

        crate::fusevm_disasm::maybe_print_stdout(&format!("host.call_function:{fn_name}"), &chunk);
        let mut vm = fusevm::VM::new(chunk);
        register_builtins(&mut vm);
        // Seed the function-body VM with the parent's `$?` so a
        // function that reads `$?` BEFORE running any command sees
        // the caller's last status. Direct port of zsh's exec.c
        // `execfuncdef`/`doshfunc` semantics — function entry does
        // NOT reset `$?`. Without this, `false; foo() { echo $?; }; foo`
        // printed 0 instead of 1 because the fresh VM defaulted
        // last_status to 0.
        vm.last_status = with_executor(|exec| exec.last_status());
        let _ = vm.run();
        let status = vm.last_status;

        // Fire any EXIT trap set INSIDE the function body, then
        // restore the outer EXIT trap. zsh fires the function-
        // scope EXIT trap BEFORE control returns to the caller,
        // so `foo() { trap "echo X" EXIT; }; foo; echo done`
        // outputs `X` then `done`. Without this, X never fired
        // (or fired at shell exit, polluting unrelated commands).
        let inner_exit = with_executor(|exec| exec.traps.remove("EXIT"));
        if let Some(action) = inner_exit {
            // Run the trap in the current (still-inside-function)
            // scope so it sees `$0 == fn_name` etc. Errors are
            // swallowed — zsh's trap dispatch tolerates body
            // failures.
            let _ = with_executor(|exec| {
                exec.set_last_status(status);
                exec.execute_script_zsh_pipeline(&action)
            });
        }
        // Restore outer EXIT trap (if any).
        if let Some(outer) = saved_exit_trap {
            with_executor(|exec| {
                exec.traps.insert("EXIT".to_string(), outer);
            });
        }

        with_executor(|exec| {
            // Set `$_` to the last arg the function was called with
            // (or the function name when called with no args). zsh:
            // `$_` after `foo arg` is `arg`; after `foo` (no args) is
            // `foo`. The function-internal `pop_args` calls polluted
            // pending_underscore with internal command args; clear and
            // overwrite here so the caller sees the function's call
            // form, not internal `return 42` arg.
            let last_call_arg = args.last().cloned().unwrap_or_else(|| fn_name.clone());
            exec.set_scalar("_".to_string(), last_call_arg.clone());
            exec.pending_underscore = Some(last_call_arg);
            exec.set_pparams(saved_params);
            exec.local_scope_depth -= 1;
            // LOCAL_OPTIONS: when set at function exit, restore all
            // options to the snapshot taken at entry. `emulate -L`
            // arms this; plugin code uses both forms to scope option
            // changes inside helpers without leaking to callers.
            // Without it, `setopt no_glob` inside a helper polluted
            // the caller's option state.
            if crate::ported::options::opt_state_get("localoptions").unwrap_or(false) {
                // Walk all options touched since entry; reset to snapshot.
                let current = crate::ported::options::opt_state_snapshot();
                for (k, _) in &current {
                    if !saved_options.contains_key(k) {
                        crate::ported::options::opt_state_unset(k);
                    }
                }
                for (k, v) in &saved_options {
                    crate::ported::options::opt_state_set(k, *v);
                }
            }
            let _ = exec; // exec still used below for other restores
                          // Restore `$0`, scriptname, and `$funcstack` to their
                          // pre-call values. scriptname mirrors C exec.c:5907
                          // `scriptname = oldscriptname;` after execode returns.
            match saved_zero {
                Some(v) => {
                    exec.set_scalar("0".to_string(), v);
                }
                None => {
                    exec.unset_scalar("0");
                }
            }
            // c:Src/exec.c:5907 doshfunc — restore global argzero to
            // the caller's value when FUNCTION_ARGZERO was honored at
            // entry. Mirrors the `argzero = old0;` line. When the
            // option was NOT set at entry, saved_argzero_global is
            // None and we leave argzero untouched.
            if let Some(prev) = saved_argzero_global {
                crate::ported::utils::set_argzero(prev);
            }
            exec.scriptname = saved_scriptname;
            exec.prompt_funcstack.pop();
            match saved_funcstack {
                Some(s) => {
                    exec.set_array("funcstack".to_string(), s);
                }
                None => {
                    exec.unset_array("funcstack");
                }
            }
            // c:Src/exec.c doshfunc → endparamscope(). Walks paramtab
            // restoring Param.old chain for every local declaration
            // made during the call.
            crate::ported::params::endparamscope();
        });

        Some(status)
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Host-routed shell ops: ShellExecutor methods invoked by ZshrsHost from the
// fusevm VM. Not a port of Src/exec.c (see file-level docs above) — they're
// the bridge between fusevm opcodes and ShellExecutor state.
// ───────────────────────────────────────────────────────────────────────────
impl crate::ported::vm_helper::ShellExecutor {
    // ─── Host-routed shell ops (called by ZshrsHost from fusevm) ────────────

    /// Apply a single redirection. The current scope's saved-fd vec gets a
    /// dup of the original fd so it can be restored by `host_redirect_scope_end`.
    /// `op_byte` matches `fusevm::op::redirect_op::*`.
    /// Apply a file-open result to a redirect fd; on error, emit
    /// zsh-format diagnostic, set redirect_failed, sink fd to /dev/null.
    /// Shared between WRITE/APPEND/READ/CLOBBER arms in
    /// host_apply_redirect to keep the error-handling identical.
    fn redir_open_or_fail(
        fd: i32,
        result: std::io::Result<std::fs::File>,
        target: &str,
        redirect_failed: &mut bool,
    ) -> bool {
        match result {
            Ok(file) => {
                let new_fd = file.into_raw_fd();
                unsafe {
                    libc::dup2(new_fd, fd);
                    libc::close(new_fd);
                }
                true
            }
            Err(e) => {
                let msg = match e.kind() {
                    std::io::ErrorKind::PermissionDenied => "permission denied",
                    std::io::ErrorKind::NotFound => "no such file or directory",
                    std::io::ErrorKind::IsADirectory => "is a directory",
                    _ => "redirect failed",
                };
                eprintln!("zshrs:1: {}: {}", msg, target);
                *redirect_failed = true;
                if let Ok(devnull) = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open("/dev/null")
                {
                    let new_fd = devnull.into_raw_fd();
                    unsafe {
                        libc::dup2(new_fd, fd);
                        libc::close(new_fd);
                    }
                }
                false
            }
        }
    }

    pub fn host_apply_redirect(&mut self, fd: u8, op_byte: u8, target: &str) {
        // `&>` / `&>>` always target both fd 1 and fd 2 regardless of the
        // fd byte the parser supplied (the lexer's tokfd clamp makes the
        // raw value unreliable for these forms).
        let fd: i32 = if matches!(op_byte, r::WRITE_BOTH | r::APPEND_BOTH) {
            1
        } else {
            fd as i32
        };
        let saved = unsafe { libc::dup(fd) };
        if saved >= 0 {
            if let Some(top) = self.redirect_scope_stack.last_mut() {
                top.push((fd, saved));
            } else {
                // No scope — leave saved fd open and let the next scope
                // reclaim it. (Caller without a scope leaks the dup; this
                // matches `WithRedirects` parser construction always wrapping.)
                unsafe { libc::close(saved) };
            }
        }
        // For `&>` / `&>>` also save fd 2 so the scope restores it after
        // the body. Otherwise stderr stays redirected past the command.
        if matches!(op_byte, r::WRITE_BOTH | r::APPEND_BOTH) {
            let saved2 = unsafe { libc::dup(2) };
            if saved2 >= 0 {
                if let Some(top) = self.redirect_scope_stack.last_mut() {
                    top.push((2, saved2));
                } else {
                    unsafe { libc::close(saved2) };
                }
            }
        }
        match op_byte {
            r::WRITE => {
                // Honor `setopt noclobber`: refuse to overwrite an
                // existing regular file unless `>!` / `>|` (CLOBBER).
                // zsh internally stores the inverted-name `clobber`
                // (default ON); `setopt noclobber` writes
                // `clobber=false`. Honor both keys.
                let noclobber = crate::ported::options::opt_state_get("noclobber").unwrap_or(false)
                    || !crate::ported::options::opt_state_get("clobber").unwrap_or(true);
                if noclobber && std::path::Path::new(target).exists() {
                    eprintln!("zshrs:1: file exists: {}", target);
                    self.set_last_status(1);
                    // Sink the upcoming command's stdout to /dev/null
                    // so we don't leak its output to the terminal.
                    // zsh skips the command entirely; we approximate by
                    // discarding the output (the redirect target was
                    // the user's chosen sink, but with noclobber the
                    // file is protected — discarding matches the
                    // user's intent better than printing to terminal).
                    if let Ok(file) = std::fs::OpenOptions::new().write(true).open("/dev/null") {
                        let new_fd = file.into_raw_fd();
                        unsafe {
                            libc::dup2(new_fd, fd);
                            libc::close(new_fd);
                        }
                    }
                    return;
                }
                if !Self::redir_open_or_fail(
                    fd,
                    std::fs::File::create(target),
                    target,
                    &mut self.redirect_failed,
                ) {
                    self.set_last_status(1);
                }
            }
            r::CLOBBER => {
                if !Self::redir_open_or_fail(
                    fd,
                    std::fs::File::create(target),
                    target,
                    &mut self.redirect_failed,
                ) {
                    self.set_last_status(1);
                }
            }
            r::APPEND => {
                if !Self::redir_open_or_fail(
                    fd,
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(target),
                    target,
                    &mut self.redirect_failed,
                ) {
                    self.set_last_status(1);
                }
            }
            r::READ => {
                if !Self::redir_open_or_fail(
                    fd,
                    std::fs::File::open(target),
                    target,
                    &mut self.redirect_failed,
                ) {
                    self.set_last_status(1);
                }
            }
            r::READ_WRITE => {
                if let Ok(file) = std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false) // <> opens existing-or-new without truncating
                    .read(true)
                    .write(true)
                    .open(target)
                {
                    let new_fd = file.into_raw_fd();
                    unsafe {
                        libc::dup2(new_fd, fd);
                        libc::close(new_fd);
                    }
                }
            }
            r::DUP_READ | r::DUP_WRITE => {
                // Target is a numeric fd reference like `&3`. The parser
                // strips the `&` prefix before we get here in some paths,
                // others retain it — accept both. Also support `-` for
                // close-fd (`<&-` / `>&-`) per POSIX.
                let n = target.trim_start_matches('&');
                if n == "-" {
                    unsafe { libc::close(fd) };
                } else if let Ok(src_fd) = n.parse::<i32>() {
                    unsafe { libc::dup2(src_fd, fd) };
                } else {
                    tracing::warn!(target = %target, "DUP redir: target not parseable as fd");
                }
            }
            r::WRITE_BOTH => {
                if let Ok(file) = std::fs::File::create(target) {
                    let new_fd = file.into_raw_fd();
                    unsafe {
                        libc::dup2(new_fd, 1);
                        libc::dup2(new_fd, 2);
                        libc::close(new_fd);
                    }
                }
            }
            r::APPEND_BOTH => {
                if let Ok(file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(target)
                {
                    let new_fd = file.into_raw_fd();
                    unsafe {
                        libc::dup2(new_fd, 1);
                        libc::dup2(new_fd, 2);
                        libc::close(new_fd);
                    }
                }
            }
            _ => {}
        }
    }

    /// Push a fresh redirect scope. `_count` is informational — the actual
    /// saved fds are appended by host_apply_redirect into the top scope.
    pub fn host_redirect_scope_begin(&mut self, _count: u8) {
        self.redirect_scope_stack.push(Vec::new());
    }

    /// Pop the top redirect scope, restoring saved fds.
    pub fn host_redirect_scope_end(&mut self) {
        if let Some(saved) = self.redirect_scope_stack.pop() {
            for (fd, saved_fd) in saved.into_iter().rev() {
                unsafe {
                    libc::dup2(saved_fd, fd);
                    libc::close(saved_fd);
                }
            }
        }
    }

    /// Set up `content` as stdin (fd 0) for the next command via a real pipe.
    /// Used by `Op::HereDoc(idx)` and `Op::HereString`.
    ///
    /// The pattern: dup2 the read end of a fresh pipe onto fd 0, save the
    /// original fd 0 into the active redirect scope so `WithRedirectsEnd`
    /// restores it, and spawn a thread that writes `content` to the write end
    /// and closes it (so the consumer sees EOF after the body). A thread is
    /// needed because writing could block on a finite pipe buffer.
    pub fn host_set_pending_stdin(&mut self, content: String) {
        let (read_end, write_end) = match os_pipe::pipe() {
            Ok(p) => p,
            Err(_) => return,
        };
        let saved = unsafe { libc::dup(libc::STDIN_FILENO) };
        if saved >= 0 {
            if let Some(top) = self.redirect_scope_stack.last_mut() {
                top.push((libc::STDIN_FILENO, saved));
            } else {
                unsafe { libc::close(saved) };
            }
        }
        let read_fd = std::os::unix::io::AsRawFd::as_raw_fd(&read_end);
        unsafe { libc::dup2(read_fd, libc::STDIN_FILENO) };
        drop(read_end);
        std::thread::spawn(move || {
            let mut w = write_end;
            let _ = w.write_all(content.as_bytes());
        });
    }

    /// Spawn an external command using zshrs's full dispatch logic
    /// (intercepts, command_hash, redirect handling). Used by
    /// `ZshrsHost::exec` so the bytecode VM's `Op::Exec` and
    /// `Op::CallFunction` external fallback get the same semantics as
    /// the tree-walker's `execute_external` rather than a plain
    /// `Command::new` shortcut. Returns the exit status.
    pub fn host_exec_external(&mut self, args: &[String]) -> i32 {
        // If a glob expansion in this command's argv triggered the
        // nomatch error path, suppress the actual exec and return
        // status 1 — mirrors zsh's command-aborted-on-glob-error
        // behaviour. The flag is reset BEFORE returning so the next
        // command starts clean.
        if self.current_command_glob_failed.get() {
            self.current_command_glob_failed.set(false);
            self.set_last_status(1);
            return 1;
        }
        let Some((cmd, rest)) = args.split_first() else {
            return 0;
        };
        // Empty command name (e.g. result of an empty `$(false)`
        // command-sub being the only word) — zsh: no command runs,
        // exit status preserved from prior step. Was hitting the
        // "command not found: " path with empty name.
        if cmd.is_empty() && rest.is_empty() {
            return self.last_status();
        }
        let rest_vec: Vec<String> = rest.to_vec();
        // Update `$_` with the just-arriving argv so the next command
        // reads `_=<last_arg>`. Mirrors C zsh's writeback in
        // `execcmd_exec` (Src/exec.c). Per `args.last()` semantics,
        // when invoked as `cmd a b c`, `$_` becomes "c" — for a bare
        // command with no args, `$_` becomes the command name itself.
        crate::ported::params::set_zunderscore(args);

        // Builtins not in fusevm's name→id table fall through to
        // host.exec. Catch them here before the OS-level exec attempts
        // to spawn a non-existent binary.
        match cmd.as_str() {
            "sched" => return self.bin_sched(&rest_vec),
            "echotc" => return crate::fusevm_bridge::dispatch_builtin("echotc", rest_vec.clone()),
            "echoti" => return crate::fusevm_bridge::dispatch_builtin("echoti", rest_vec.clone()),
            // "getln" handler deleted with its stub.
            "zpty" => return crate::fusevm_bridge::dispatch_builtin("zpty", rest_vec.clone()),
            "ztcp" => return crate::fusevm_bridge::dispatch_builtin("ztcp", rest_vec.clone()),
            "zsocket" => {
                // Shim — parses the BUILTIN spec "ad:ltv" from
                // socket.c:276 into a real `options` struct, then
                // invokes the canonical free-fn port at
                // crate::ported::modules::socket::bin_zsocket whose
                // signature matches C `bin_zsocket(nam, args, ops,
                // func)` exactly.
                let mut ops = options {
                    ind: [0u8; MAX_OPS],
                    args: Vec::new(),
                    argscount: 0,
                    argsalloc: 0,
                };
                let mut positional: Vec<String> = Vec::new();
                let mut i = 0;
                while i < rest_vec.len() {
                    let a = &rest_vec[i];
                    if a == "--" {
                        i += 1;
                        positional.extend_from_slice(&rest_vec[i..]);
                        break;
                    }
                    if let Some(rest) = a.strip_prefix('-') {
                        if rest.is_empty() {
                            positional.push(a.clone());
                            i += 1;
                            continue;
                        }
                        let chars: Vec<char> = rest.chars().collect();
                        let mut j = 0;
                        while j < chars.len() {
                            let c = chars[j] as u8;
                            if c == b'd' {
                                ops.ind[c as usize] = (ops.args.len() + 1) as u8;
                                let rest_after = &rest[j + 1..];
                                if !rest_after.is_empty() {
                                    ops.args.push(rest_after.to_string());
                                } else {
                                    i += 1;
                                    ops.args.push(rest_vec.get(i).cloned().unwrap_or_default());
                                }
                                ops.argscount = ops.args.len() as i32;
                                break;
                            }
                            if c.is_ascii_alphabetic() {
                                ops.ind[c as usize] = 1;
                            }
                            j += 1;
                        }
                    } else {
                        positional.push(a.clone());
                    }
                    i += 1;
                }
                return bin_zsocket("zsocket", &positional, &ops, 0);
            }
            "private" => {
                // bin_private now takes the canonical C signature
                // (name, args, ops, func, assigns) per Src/Modules/
                // param_private.c:217.
                let mut ops = options {
                    ind: [0u8; MAX_OPS],
                    args: Vec::new(),
                    argscount: 0,
                    argsalloc: 0,
                };
                return crate::modules::param_private::bin_private(
                    "private",
                    &rest_vec,
                    &ops,
                    0,
                );
            }
            "zformat" => {
                return crate::fusevm_bridge::dispatch_builtin("zformat", rest_vec.clone())
            }
            "zregexparse" => {
                return crate::fusevm_bridge::dispatch_builtin("zregexparse", rest_vec.clone())
            }
            // `unalias`/`unhash`/`unfunction` share `bin_unhash` but
            // each carries its own funcid (BIN_UNALIAS / BIN_UNHASH /
            // BIN_UNFUNCTION) in the BUILTINS table. Route through
            // execbuiltin so the correct funcid + optstr propagate —
            // without this `unalias` was a silent no-op.
            "unalias" | "unhash" | "unfunction" => {
                // Fallback when fusevm doesn't have a BUILTIN_*
                // opcode registered for the name (e.g. shell-builtin
                // table mismatch). Route through execbuiltin with the
                // correct entry from BUILTINS.
                let bn_idx = crate::ported::builtin::BUILTINS
                    .iter()
                    .position(|b| b.node.nam == cmd.as_str());
                if let Some(idx) = bn_idx {
                    let bn_static: &'static crate::ported::zsh_h::builtin =
                        &crate::ported::builtin::BUILTINS[idx];
                    let bn_ptr = bn_static as *const _ as *mut _;
                    return crate::ported::builtin::execbuiltin(rest_vec, Vec::new(), bn_ptr);
                }
                return 1;
            }
            // zsh-bundled rename helpers — implemented natively in
            // Rust so `autoload -U zmv` works without shipping the
            // function source. (Without this, the autoload path hangs.)
            "zmv" => return crate::extensions::ext_builtins::zmv(&rest_vec, "mv"),
            "zcp" => return crate::extensions::ext_builtins::zmv(&rest_vec, "cp"),
            "zln" => return crate::extensions::ext_builtins::zmv(&rest_vec, "ln"),
            "zcalc" => return crate::extensions::ext_builtins::zcalc(&rest_vec),
            "zselect" => {
                // Canonical bin_zselect per zselect.c:65 takes
                // (nam, args, ops, func); the C source parses its
                // own option string inline, so an empty Options is
                // sufficient at this call site.
                let ops = options {
                    ind: [0u8; MAX_OPS],
                    args: Vec::new(),
                    argscount: 0,
                    argsalloc: 0,
                };
                return crate::ported::modules::zselect::bin_zselect("zselect", &rest_vec, &ops, 0);
            }
            "cap" => return crate::fusevm_bridge::dispatch_builtin("cap", rest_vec.clone()),
            "getcap" => return crate::fusevm_bridge::dispatch_builtin("getcap", rest_vec.clone()),
            "setcap" => return crate::fusevm_bridge::dispatch_builtin("setcap", rest_vec.clone()),
            "yes" => return self.builtin_yes(&rest_vec),
            "nl" => return self.builtin_nl(&rest_vec),
            "env" => return self.builtin_env(&rest_vec),
            "printenv" => return self.builtin_printenv(&rest_vec),
            "tty" => return self.builtin_tty(&rest_vec),
            "chgrp" => {
                // Canonical bin_chown per files.c:725 with func=BIN_CHGRP
                // per the bintab entry at c:805. BUILTIN spec "hRs".
                let mut ops = options {
                    ind: [0u8; MAX_OPS],
                    args: Vec::new(),
                    argscount: 0,
                    argsalloc: 0,
                };
                let mut positional: Vec<String> = Vec::new();
                let mut i = 0;
                while i < rest_vec.len() {
                    let a = &rest_vec[i];
                    if a == "--" {
                        i += 1;
                        positional.extend_from_slice(&rest_vec[i..]);
                        break;
                    }
                    if let Some(rest) = a.strip_prefix('-') {
                        if rest.is_empty() {
                            positional.push(a.clone());
                            i += 1;
                            continue;
                        }
                        for c in rest.chars() {
                            let cb = c as u8;
                            if cb.is_ascii_alphabetic() {
                                ops.ind[cb as usize] = 1;
                            }
                        }
                    } else {
                        positional.push(a.clone());
                    }
                    i += 1;
                }
                return crate::ported::modules::files::bin_chown(
                    "chgrp",
                    &positional,
                    &ops,
                    crate::ported::modules::files::BIN_CHGRP,
                );
            }
            "nproc" => return self.builtin_nproc(&rest_vec),
            "expr" => return self.builtin_expr(&rest_vec),
            "sha256sum" => return self.builtin_sha256sum(&rest_vec),
            "base64" => return self.builtin_base64(&rest_vec),
            "tac" => return self.builtin_tac(&rest_vec),
            "expand" => return self.builtin_expand(&rest_vec),
            "unexpand" => return self.builtin_unexpand(&rest_vec),
            "paste" => return self.builtin_paste(&rest_vec),
            "fold" => return self.builtin_fold(&rest_vec),
            "shuf" => return self.builtin_shuf(&rest_vec),
            "comm" => return self.builtin_comm(&rest_vec),
            "cksum" => return self.builtin_cksum(&rest_vec),
            "factor" => return self.builtin_factor(&rest_vec),
            "tsort" => return self.builtin_tsort(&rest_vec),
            "sum" => return self.builtin_sum(&rest_vec),
            "mkfifo" => return self.builtin_mkfifo(&rest_vec),
            "link" => return self.builtin_link(&rest_vec),
            "unlink" => return self.builtin_unlink(&rest_vec),
            "dircolors" => return self.builtin_dircolors(&rest_vec),
            "groups" => return self.builtin_groups(&rest_vec),
            "arch" => return self.builtin_arch(&rest_vec),
            "nice" => return self.builtin_nice(&rest_vec),
            "logname" => return self.builtin_logname(&rest_vec),
            "tput" => return self.builtin_tput(&rest_vec),
            "users" => return self.builtin_users(&rest_vec),
            // "sync" => return self.bin_sync(&rest_vec),
            "zbuild" => return self.builtin_zbuild(&rest_vec),
            // `zf_*` aliases from `zsh/files` (Src/Modules/files.c
            // BUILTIN table at line 816-824). The C source binds
            // both unprefixed (`chmod`) and prefixed (`zf_chmod`)
            // names to the SAME `bin_chmod` etc. handlers — the
            // prefixed forms exist so a script can portably reach
            // the builtin even when a function or alias has shadowed
            // the bare name. Each arm routes through the canonical
            // free-fn port of Src/Modules/files.c, parsing the BUILTIN
            // optstr inline since the framework doesn't pre-parse.
            "zf_mkdir" | "mkdir" => {
                let mut ops = options {
                    ind: [0u8; MAX_OPS],
                    args: Vec::new(),
                    argscount: 0,
                    argsalloc: 0,
                };
                let mut positional: Vec<String> = Vec::new();
                let mut i = 0;
                while i < rest_vec.len() {
                    let a = &rest_vec[i];
                    if a == "--" {
                        i += 1;
                        positional.extend_from_slice(&rest_vec[i..]);
                        break;
                    }
                    if let Some(rest) = a.strip_prefix('-') {
                        if rest.is_empty() {
                            positional.push(a.clone());
                            i += 1;
                            continue;
                        }
                        let chars: Vec<char> = rest.chars().collect();
                        let mut j = 0;
                        while j < chars.len() {
                            let c = chars[j] as u8;
                            if c == b'm' {
                                ops.ind[c as usize] = (ops.args.len() + 1) as u8;
                                let rest_after = &rest[j + 1..];
                                if !rest_after.is_empty() {
                                    ops.args.push(rest_after.to_string());
                                } else {
                                    i += 1;
                                    ops.args.push(rest_vec.get(i).cloned().unwrap_or_default());
                                }
                                ops.argscount = ops.args.len() as i32;
                                break;
                            }
                            if c.is_ascii_alphabetic() {
                                ops.ind[c as usize] = 1;
                            }
                            j += 1;
                        }
                    } else {
                        positional.push(a.clone());
                    }
                    i += 1;
                }
                return crate::ported::modules::files::bin_mkdir(cmd, &positional, &ops, 0);
            }
            "zf_rm" => {
                let mut ops = options {
                    ind: [0u8; MAX_OPS],
                    args: Vec::new(),
                    argscount: 0,
                    argsalloc: 0,
                };
                let mut positional: Vec<String> = Vec::new();
                let mut i = 0;
                while i < rest_vec.len() {
                    let a = &rest_vec[i];
                    if a == "--" {
                        i += 1;
                        positional.extend_from_slice(&rest_vec[i..]);
                        break;
                    }
                    if let Some(rest) = a.strip_prefix('-') {
                        if rest.is_empty() {
                            positional.push(a.clone());
                            i += 1;
                            continue;
                        }
                        for c in rest.chars() {
                            let cb = c as u8;
                            if cb.is_ascii_alphabetic() {
                                ops.ind[cb as usize] = 1;
                            }
                        }
                    } else {
                        positional.push(a.clone());
                    }
                    i += 1;
                }
                return crate::ported::modules::files::bin_rm("zf_rm", &positional, &ops, 0);
            }
            "zf_rmdir" => {
                let ops = options {
                    ind: [0u8; MAX_OPS],
                    args: Vec::new(),
                    argscount: 0,
                    argsalloc: 0,
                };
                return crate::ported::modules::files::bin_rmdir("zf_rmdir", &rest_vec, &ops, 0);
            }
            // `zstat` — port of zsh/stat module (Src/Modules/stat.c
            // BUILTIN("zstat", …)). Returns file metadata as
            // `field value` pairs / an assoc / a plus-separated
            // list depending on flags. zsh ALSO registers `stat`
            // bound to the same handler, but that name conflicts
            // with the system `stat(1)` binary (every script that
            // calls `stat -f '%Lp' …` would break). zsh resolves
            // this through opt-in `zmodload`; zshrs's modules are
            // statically linked so we keep `stat` routing to the
            // external command and only intercept the unambiguous
            // `zstat` name.
            "zstat" => {
                // bin_stat now takes the canonical C signature
                // (name, args, ops, func) per Src/Modules/stat.c:368.
                let ops = options {
                    ind: [0u8; MAX_OPS],
                    args: Vec::new(),
                    argscount: 0,
                    argsalloc: 0,
                };
                return crate::modules::stat::bin_stat("zstat", &rest_vec, &ops, 0);
            }
            _ => {}
        }

        // AOP intercepts: when an `intercept :before/:around/:after foo` block
        // is registered, dynamic-command-name dispatch must consult it before
        // spawning. Without this, `cmd=ls; $cmd` bypasses every intercept that
        // a literal `ls` would trigger. The full_cmd string mirrors what the
        // tree-walker era passed (cmd + args joined by space) so existing
        // pattern matchers continue to work.
        if !self.intercepts.is_empty() {
            let full_cmd = if rest_vec.is_empty() {
                cmd.clone()
            } else {
                format!("{} {}", cmd, rest_vec.join(" "))
            };
            if let Some(intercept_result) = self.run_intercepts(cmd, &full_cmd, &rest_vec) {
                return intercept_result.unwrap_or(127);
            }
        }

        // User-defined function lookup before OS-level exec. zsh's
        // dynamic-command-name dispatch (`cmd=hook1; $cmd`) checks
        // the function table FIRST — without this, `$f` for a
        // function-name `f` was always falling through to
        // `execute_external` and erroring "command not found".
        // Plugin code uses this pattern constantly:
        //   for f in "${precmd_functions[@]}"; do "$f"; done
        if self.function_exists(cmd) {
            if let Some(status) = self.dispatch_function_call(cmd, &rest_vec) {
                return status;
            }
        }

        self.execute_external(cmd, &rest_vec, &[]).unwrap_or(127)
    }
}
