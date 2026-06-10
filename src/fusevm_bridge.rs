//! fusevm bytecode-VM bridge for ShellExecutor.
//!
//! ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//! !!! LAST-RESORT FILE — NOT FOR NEW LOGIC !!!
//!
//! This file is a **bridge**, not a port. It exists ONLY because zshrs uses
//! a fusevm bytecode VM where C zsh uses its own wordcode walker (Src/exec.c
//! `execlist`). Every line here is plumbing that hooks fusevm opcodes onto
//! the canonical ports in `src/ported/`.
//!
//! **Before adding code to this file, STOP and ask:**
//!
//!   1. Is this logic that already lives in `src/ported/`?
//!      → Call the canonical fn. Don't reinline.
//!
//!   2. Is this logic that SHOULD live in `src/ported/` but isn't ported yet?
//!      → Port it. Add it to `src/ported/<file>.rs` with a `c:` citation.
//!        Then call the canonical fn from here.
//!
//!   3. Is this purely fusevm/bytecode plumbing (Op decode, Value conversion,
//!      VM-stack manipulation, thread-local executor pointer, etc.)?
//!      → OK to put it here. Cite the closest C analog in the comment.
//!
//! **NEVER:** reinvent paramsubst/expansion/glob/typeset/redirect logic here.
//! Those have canonical ports in `src/ported/subst.rs`, `src/ported/glob.rs`,
//! `src/ported/builtin.rs`, etc. The bridge should be SHRINKING over time,
//! not growing.
//!
//! See also: memory `feedback_no_shortcuts_in_porting` (port C bodies
//! faithfully, no structural shells), `feedback_no_exec_script_from_ported`
//! (the inverse direction — src/ported must not call back into the bridge).
//! ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
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

use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;

use crate::exec_jobs::JobState;
use crate::intercepts::Intercept;
use crate::ported::vm_helper::*;
use std::io::Write;

// ═══════════════════════════════════════════════════════════════════════════
// Thread-local executor context for VM builtin dispatch
// ═══════════════════════════════════════════════════════════════════════════

use crate::ported::options::opt_state_get;
use crate::ported::zsh_h::{isset, options, ERREXIT, MAX_OPS};
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

thread_local! {
    /// Mirror of C zsh's `doneps4` local in execcmd_exec
    /// (Src/exec.c:2517+). Tracks whether PS4 has been emitted
    /// for the current xtrace line so a coalesced sequence of
    /// XTRACE_ASSIGN + XTRACE_ARGS produces ONE line:
    ///   `<PS4>a=1 b=2 echo 1 2\n`
    /// instead of three. Reset to false by XTRACE_ARGS /
    /// XTRACE_NEWLINE after emitting the trailing `\n`.
    static XTRACE_DONE_PS4: Cell<bool> = const { Cell::new(false) };

    /// Stack of (RETFLAG, BREAKS, CONTFLAG, EXIT_PENDING) tuples saved
    /// at try-block exit so the always-arm body can run cleanly even
    /// when the try-block fired `return` / `break` / `continue` /
    /// `exit`. Restored right before the post-always re-jump so the
    /// escape resumes propagation past the construct.
    /// c:Src/exec.c WC_TRYBLOCK — zsh's wordcode walker handles this
    /// inline; the zshrs port lifts it into a paired SET / RESTORE
    /// pair around the always-arm.
    static TRY_ESCAPE_SAVE: RefCell<Vec<(i32, i32, i32, i32)>> =
        const { RefCell::new(Vec::new()) };
    /// Re-entry guard for BUILTIN_DEBUG_TRAP. While the DEBUG trap
    /// body is running, the per-statement DEBUG_TRAP dispatch in the
    /// trap body must NOT re-fire (otherwise infinite recursion +
    /// stack overflow). zsh's in_trap counter at Src/signals.c
    /// serves the same purpose.
    static DEBUG_TRAP_REENTRY: Cell<bool> = const { Cell::new(false) };
    /// Stack of (saved_stdout, saved_stderr) tuples pushed by
    /// `cmd_subst` around its nested-VM run. RUST-ONLY: zsh forks
    /// each cmdsub so trap output during the cmdsub naturally
    /// lands on the PARENT's stdout. zshrs's in-process cmdsub
    /// dups fd 1 → pipe, so a trap firing during cmdsub would
    /// emit into the captured value. Traps consult this stack
    /// to route their body output to the topmost saved_stdout
    /// instead of the cmdsub's fd 1. Bug #56 in docs/BUGS.md.
    pub static CMDSUBST_OUTER_FDS: RefCell<Vec<(i32, i32)>> =
        const { RefCell::new(Vec::new()) };
}

/// Peek the outermost cmdsub-saved (stdout, stderr) fds, if any.
/// Returns None when no cmdsub is currently capturing. Used by the
/// trap dispatcher in `src/ported/signals.rs::dotrap` to route trap
/// body output to the parent's real stdout (matching zsh's forked
/// cmdsub behaviour) instead of the cmdsub's pipe-bound fd 1.
/// Bug #56 in docs/BUGS.md.
pub fn cmdsubst_outer_stdout() -> Option<i32> {
    CMDSUBST_OUTER_FDS.with(|s| s.borrow().last().map(|(o, _)| *o))
}

// Thread-local pointer to the current ShellExecutor.
// Set before VM execution, cleared after. Used by builtin handlers.
thread_local! {
    static CURRENT_EXECUTOR: RefCell<Option<*mut ShellExecutor>> = const { RefCell::new(None) };
    /// Set by subshell_end after a deferred subshell `exit N` lands.
    /// Read + cleared by the next GET_VAR sync_status path so the
    /// vm.last_status → LASTVAL sync doesn't clobber the deferred
    /// exit status. RUST-ONLY: needed because zshrs runs subshells
    /// in-process (no fork) so vm.last_status doesn't track the
    /// subshell's exit; C zsh's subshell forks and the child's
    /// process::exit(N) becomes $? in the parent automatically.
    static SUBSHELL_EXIT_STATUS_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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

/// Look up a canonical builtin by name in `BUILTINS` and dispatch
/// via `execbuiltin` (Src/builtin.c:250). NO shadow check — calls the
/// builtin even if a user function with the same name exists. Used by
/// the `builtin foo` prefix opcode (which explicitly bypasses function
/// lookup per zsh semantics) and by internal call sites where shadowing
/// is unwanted. For zsh's normal name-resolution order (function shadows
/// builtin), use `dispatch_builtin` instead.
/// Shell-identifier prefix for diagnostic lines. Reads the canonical
/// scriptname (`zsh` in `--zsh` parity mode, `zshrs` otherwise) so a
/// single helper replaces hardcoded `"zshrs:"` literals across the
/// file's eprintln paths.
fn shname() -> String {
    crate::ported::utils::scriptname_get().unwrap_or_else(|| "zshrs".to_string())
}

pub(crate) fn dispatch_builtin_raw(name: &str, args: Vec<String>) -> i32 {
    // c:Bugs #475/#504/#555 — bash-only builtins (`mapfile`,
    // `readarray`, `compopt`) should emit "command not found" in
    // `--zsh` mode matching zsh's external-command-lookup miss.
    // The per-opcode closures for caller/help/complete/compgen
    // already gate via IS_ZSH_MODE at their registration sites;
    // names without dedicated opcodes (compopt/mapfile/readarray)
    // route through this generic builtintab lookup and need the
    // gate here.
    if crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed)
        && matches!(name, "compopt" | "mapfile" | "readarray")
    {
        eprintln!("zsh:1: command not found: {}", name);
        let _ = args;
        return 127;
    }
    // c:Src/exec.c:3050-3068 — builtin lookup hits `builtintab` (the
    // merged table containing module-provided builtins). The previous
    // port walked only the core `BUILTINS` slice, so per-module
    // entries like `log` (Src/Modules/watch.c:693 `BUILTIN("log", …,
    // bin_log, …)`) were registered into builtintab via
    // createbuiltintable but never reached at dispatch — `log` fell
    // through to PATH and ran `/usr/bin/log`. Bug #72 in docs/BUGS.md.
    let tab = crate::ported::builtin::createbuiltintable();
    if let Some(bn_static) = tab.get(name) {
        let bn_ptr = *bn_static as *const _ as *mut _;
        return crate::ported::builtin::execbuiltin(args, Vec::new(), bn_ptr);
    }
    1
}

/// Shadow-aware dispatch matching zsh's name-resolution order:
/// alias → reserved word → **function (shadows builtin)** → builtin →
/// external. All `BUILTIN_X` opcode handlers route through here so a
/// user-defined `cd () { … }` (or `r`, `fc`, `which`, … anything in
/// fusevm's name→opcode map) takes precedence over the C builtin —
/// matching `Src/exec.c:execcmd_exec`'s dispatch at c:3050-3068.
/// Without this, compile-time builtin resolution silently ignored
/// user wrappers (e.g. ZPWR's `cd () { builtin cd "$@"; … }`).
/// True for builtins that are bound by zsh/files's boot_/setup_
/// chain (Src/Modules/files.c:806-824). These are the bare-name
/// `mkdir`/`rm`/`mv`/`ln`/`chmod`/`chown`/`chgrp`/`sync`/`rmdir`
/// AND their `zf_*` aliases at c:816-824. Without explicit
/// `zmodload zsh/files`, the names fall through to PATH lookup
/// (zsh's `type rm` reports `/bin/rm`). Bug #28.
fn module_gated_files_builtin(name: &str) -> bool {
    matches!(
        name,
        "mkdir" | "rmdir" | "rm" | "mv" | "ln" | "chmod" | "chown" | "chgrp" | "sync"
            | "zf_mkdir" | "zf_rmdir" | "zf_rm" | "zf_mv" | "zf_ln" | "zf_chmod"
            | "zf_chown" | "zf_chgrp" | "zf_sync"
    )
}

pub(crate) fn dispatch_builtin(name: &str, args: Vec<String>) -> i32 {
    // c:Src/exec.c — when any redirect in the current scope failed
    // (e.g. noclobber blocked a `>` overwrite), zsh refuses to
    // execute the command and exits with status 1. The Rust port
    // still applied the command (writing to the /dev/null sink
    // installed by host_apply_redirect's noclobber arm) so the
    // success status overwrote the intended 1. Short-circuit here
    // for builtins (the external-exec equivalent lives in
    // ZshrsHost::exec).
    let redir_failed = with_executor(|exec| {
        let f = exec.redirect_failed;
        exec.redirect_failed = false;
        f
    });
    if redir_failed {
        return 1;
    }
    // c:Src/glob.c:1876-1880 NOMATCH path — when expand_glob() failed
    // on a no-match glob, zsh aborts the simple command after zerr()
    // printed "no matches found". In C, this works because zerr()
    // sets ERRFLAG_ERROR (Src/utils.c) and execcmd_exec()
    // (Src/exec.c:3050+) checks errflag before invoking the builtin
    // table. Rust's builtin dispatch doesn't sit on the same errflag
    // gate, so we explicitly consume the per-command glob-fail cell
    // and short-circuit with status 1. Mirrors the external-path
    // guard at host_exec_external (line 5167). Without this:
    // `echo /never/*` would print empty (silently rolled back to ""
    // by the empty glob expansion). Parity bug #13.
    let glob_failed = with_executor(|exec| {
        let f = exec.current_command_glob_failed.get();
        exec.current_command_glob_failed.set(false); // c:1879 cleanup
        f
    });
    if glob_failed {
        // c:Src/glob.c:1876-1880 + Src/exec.c — NOMATCH zerr sets
        // ERRFLAG_ERROR (via utils.c:184); the C execlist loop's per-
        // sublist post-exec path then resets the bit so subsequent
        // sublists continue (verified: `zsh -fc 'ls /nope_*; echo
        // after'` prints `after`). zshrs's vm dispatch doesn't have
        // C's central execlist loop — the post-command-boundary
        // equivalent is right HERE, where the dispatcher consumes
        // `current_command_glob_failed` and surfaces status 1 for THIS
        // command. Clear ERRFLAG_ERROR at the same boundary so the
        // next command runs (while leaving ERRFLAG_INT etc. alone so
        // ctrl-c still propagates).
        crate::ported::utils::errflag.fetch_and(
            !crate::ported::zsh_h::ERRFLAG_ERROR,
            std::sync::atomic::Ordering::Relaxed,
        );
        return 1; // c:1880 — command aborted, status 1
    }
    if let Some(status) = try_user_fn_override(name, &args) {
        // c:Src/jobs.c:1748 waitonejob — canonical single-command
        // pipestats update via the no-procs else-branch.
        crate::ported::builtin::LASTVAL.store(status, std::sync::atomic::Ordering::Relaxed);
        let mut synth = crate::ported::zsh_h::job::default();
        crate::ported::jobs::waitonejob(&mut synth);
        return status;
    }
    // c:Src/builtin.c:587 + Src/exec.c:3056 — a builtin disabled via
    // `disable <name>` has its `DISABLED` flag set in `builtintab`;
    // `builtintab->getnode` (the DISABLED-filtering accessor) returns
    // NULL for it at lookup time, so execcmd_exec falls through to
    // PATH lookup and runs the external. The Rust port stores the
    // disabled set in `BUILTINS_DISABLED`; the previous dispatcher
    // only checked the immutable `createbuiltintable` HashMap which
    // never reflects disablement — so `disable echo; echo hi` kept
    // running the bin_echo builtin. Bug #106 in docs/BUGS.md.
    //
    // dispatch_builtin (the high-level wrapper used by the BUILTIN_*
    // opcode handlers and reg_passthru! callsites) is the correct
    // gate: `dispatch_builtin_raw` is the low-level entry point
    // used by `bin_builtin` itself which MUST bypass the disabled
    // set (man zshbuiltins: `builtin name` runs the builtin
    // regardless of disable state). Place the check here so the
    // bypass path stays clean.
    let disabled = crate::ported::builtin::BUILTINS_DISABLED
        .lock()
        .map(|s| s.contains(name))
        .unwrap_or(false);
    if disabled {
        let status = with_executor(|exec| exec.execute_external(name, &args, &[]))
            .unwrap_or(127);
        crate::ported::builtin::LASTVAL
            .store(status, std::sync::atomic::Ordering::Relaxed);
        let mut synth = crate::ported::zsh_h::job::default();
        crate::ported::jobs::waitonejob(&mut synth);
        return status;
    }
    // c:Src/Modules/files.c:806-814 — `mkdir`, `rm`, `mv`, `ln`, `chmod`,
    // `chown`, `chgrp`, `sync`, `rmdir` are bound by the `zsh/files`
    // module's boot_/setup_ chain. Without explicit `zmodload zsh/files`,
    // these bare names fall through to PATH (`/bin/rm`, `/usr/bin/chmod`,
    // etc.) in zsh; `type rm` reports `rm is /bin/rm`. The `zf_*`
    // aliases (`zf_rm`, `zf_chmod`, …) are bound by the same module
    // and gated the same way. Bug #28 in docs/BUGS.md.
    if module_gated_files_builtin(name) {
        if !crate::ported::module::MODULESTAB.lock().unwrap().is_loaded("zsh/files") {
            // Strip the `zf_` prefix when routing to PATH so `zf_rm`
            // (when zsh/files isn't loaded) still finds /bin/rm.
            let path_name = name.strip_prefix("zf_").unwrap_or(name);
            let status = with_executor(|exec| exec.execute_external(path_name, &args, &[]))
                .unwrap_or(127);
            crate::ported::builtin::LASTVAL
                .store(status, std::sync::atomic::Ordering::Relaxed);
            let mut synth = crate::ported::zsh_h::job::default();
            crate::ported::jobs::waitonejob(&mut synth);
            return status;
        }
    }
    let status = dispatch_builtin_raw(name, args);
    // c:Src/jobs.c:1748 waitonejob — canonical single-command pipestats update.
    crate::ported::builtin::LASTVAL.store(status, std::sync::atomic::Ordering::Relaxed);
    let mut synth = crate::ported::zsh_h::job::default();
    crate::ported::jobs::waitonejob(&mut synth);
    status
}

/// Install the `crate::ported::exec_hooks` fn-pointer registry so
/// code under `src/ported/` can dispatch to operations owned by
/// `ShellExecutor` (array/assoc storage, script eval, function
/// dispatch, command substitution) WITHOUT a direct executor
/// reference or `with_executor` call from inside src/ported/.
///
/// Idempotent — each hook uses `OnceLock::set`, so calling this
/// multiple times (once per `ShellExecutor::new`) is safe; the second
/// and later calls are no-ops.
///
/// **Extension** — no C analog. Bridges the Rust-only `src/ported/`
/// → executor boundary that the user pinned as forbidden via memory
/// `feedback_no_exec_script_from_ported` /
/// `feedback_no_shellexecutor_in_ported`.
pub(crate) fn install_exec_hooks() {
    use crate::ported::exec_hooks as h;
    h::install_array_get(|name| with_executor(|exec| exec.array(name)));
    h::install_assoc_get(|name| with_executor(|exec| exec.assoc(name)));
    h::install_array_set(|name, val| {
        with_executor(|exec| exec.set_array(name.to_string(), val));
    });
    h::install_assoc_set(|name, val| {
        with_executor(|exec| exec.set_assoc(name.to_string(), val));
    });
    h::install_scalar_unset(|name| {
        with_executor(|exec| exec.unset_scalar(name));
    });
    h::install_array_unset(|name| {
        with_executor(|exec| exec.unset_array(name));
    });
    h::install_assoc_unset(|name| {
        with_executor(|exec| exec.unset_assoc(name));
    });
    h::install_dispatch_function_call(|name, args| {
        with_executor(|exec| exec.dispatch_function_call(name, args))
    });
    h::install_run_function_body(|name, args| {
        with_executor(|exec| exec.run_function_body_only(name, args))
    });
    h::install_execute_script(|src| with_executor(|exec| exec.execute_script(src)));
    h::install_execute_script_zsh_pipeline(|src| {
        with_executor(|exec| exec.execute_script_zsh_pipeline(src))
    });
    h::install_run_command_substitution(|cmd| {
        with_executor(|exec| exec.run_command_substitution(cmd))
    });
    h::install_pparams_get(|| with_executor(|exec| exec.pparams()));
    h::install_pparams_set(|v| {
        with_executor(|exec| exec.set_pparams(v));
    });
    h::install_unregister_function(|name| {
        with_executor(|exec| {
            let a = exec.functions_compiled.remove(name).is_some();
            let b = exec.function_source.remove(name).is_some();
            a || b
        })
    });
}

/// Register all zsh builtins with the VM.
pub(crate) fn register_builtins(vm: &mut fusevm::VM) {
    // exec_hooks fn-ptrs MUST be installed before any builtin can
    // reach into src/ported/ code that consults them (e.g.
    // `BUILTIN_ERREXIT_CHECK` → `dotrap` → `exec_hooks::dispatch_function_call`).
    // OnceLock makes the call idempotent — repeated invocations from
    // every `ShellExecutor::new` are no-ops.
    install_exec_hooks();
    // Engage fusevm's tiered JIT (block + tracing) so hot, fully-eligible
    // numeric chunks run in native code and — with the `jit-disk-cache`
    // feature (on by default) — persist that native code to
    // `~/.cache/fusevm-jit`, letting repeated zsh invocations skip Cranelift
    // codegen. fusevm gates the JIT on per-chunk eligibility and warms up by
    // an invocation threshold, falling back to the interpreter for any chunk
    // it cannot compile (e.g. host-builtin/`Extended` command dispatch), so
    // enabling it here never changes observable behaviour — it only caches
    // the numeric hot path. Idempotent: re-enabling on each VM is a no-op.
    vm.enable_tracing_jit();
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
                // c:Src/exec.c — redirect failure in the current
                // scope means the command must NOT run. coreutils
                // shadows (cat / head / tail / etc.) take a separate
                // dispatch path from dispatch_builtin, so they need
                // their own gate. Without this `cat <&3` after a
                // closed-fd diagnostic still ran the shadow and
                // overwrote $? from the forced 1.
                let redir_failed = with_executor(|exec| {
                    let f = exec.redirect_failed;
                    exec.redirect_failed = false;
                    f
                });
                if redir_failed {
                    return Value::Status(1);
                }
                // `[builtins].coreutils_shadows = off` in
                // ~/.zshrs/zshrs.toml (or `ZSHRS_NO_COREUTILS_SHADOWS=1`
                // env override) bypasses the in-process shadow and
                // fork-execs the real /bin/X. Safety valve for any
                // script that hits an edge-case divergence between
                // the zshrs shadow and system coreutils. Cached
                // after first call, so the hot path is one atomic
                // load per shadowed-builtin invocation.
                if !crate::daemon_presence::coreutils_shadows_enabled() {
                    return Value::Status(exec_system_command($name, &args));
                }
                let status = with_executor(|exec| exec.$method(&args));
                Value::Status(status)
            });
        };
    }

    // Pure-passthru builtin: pops args, routes to canonical
    // `dispatch_builtin(name, args)` (which goes via execbuiltin →
    // BUILTINS[name] → bin_X). No pre/post bridge work. Used by
    // ~25 handlers that were 4-line copy-paste boilerplate.
    macro_rules! reg_passthru {
        ($vm:expr, $id:expr, $name:literal) => {
            $vm.register_builtin($id, |vm, argc| {
                Value::Status(dispatch_builtin($name, pop_args(vm, argc)))
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
        // c:Src/builtin.c:1258 — `callhookfunc("chpwd", NULL, 1, NULL)`
        // after cd succeeds. The canonical port at
        // src/ported/utils.rs:1532 handles both the `chpwd` shfunc
        // dispatch AND the `chpwd_functions` array walk.
        if status == 0 {
            crate::ported::utils::callhookfunc("chpwd", None, 1, std::ptr::null_mut());
        }
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

    reg_passthru!(vm, BUILTIN_PRINTF, "printf");
    reg_passthru!(vm, BUILTIN_EXPORT, "export");
    reg_passthru!(vm, BUILTIN_UNSET, "unset");
    // `source` (Src/builtin.c c:116) wired to bin_dot via BUILTINS.
    reg_passthru!(vm, BUILTIN_SOURCE, "source");
    reg_passthru!(vm, BUILTIN_DOT, ".");
    reg_passthru!(vm, BUILTIN_LOGOUT, "logout");

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
        // c:Src/exec.c:1257 — zsh sets `zunderscore` AT THE END of
        // each command (the `if (!noerrs)` block runs `zsfree(prev_argv0); …;
        // zunderscore = …`). For no-arg `true`, $_ becomes the
        // command name itself. Set DIRECTLY (not via pending_underscore)
        // so the NEXT command's argv-expansion of `$_` reads "true",
        // not the stale prior value — pending_underscore is consumed
        // by pop_args which runs AFTER argv expansion, too late.
        // c:Src/exec.c:1257 — `zunderscore = …` at end-of-command.
        // With args, $_ = args.last(). Without args, $_ = command name.
        // Write DIRECTLY to the canonical zunderscore static (the
        // underscoregetfn at params.rs:7003 reads from there); the
        // paramtab "_" slot is shadowed by lookup_special_var so
        // set_scalar on it has no effect on `$_` reads.
        if args.is_empty() {
            crate::ported::params::set_zunderscore(&["true".to_string()]);
        } else {
            crate::ported::params::set_zunderscore(&args);
        }
        // Route through canonical execbuiltin so PS4 xtrace fires
        // via the c:442 printprompt4 path.
        Value::Status(dispatch_builtin("true", args))
    });
    vm.register_builtin(BUILTIN_FALSE, |vm, argc| {
        let args = pop_args(vm, argc);
        if let Some(s) = try_user_fn_override("false", &args) {
            return Value::Status(s);
        }
        // Direct set; see BUILTIN_TRUE above for rationale.
        if args.is_empty() {
            crate::ported::params::set_zunderscore(&["false".to_string()]);
        } else {
            crate::ported::params::set_zunderscore(&args);
        }
        // Route through canonical execbuiltin — see BUILTIN_TRUE
        // above for the same rationale (xtrace + fast-path removal).
        let status = dispatch_builtin("false", args);
        Value::Status(status)
    });
    vm.register_builtin(BUILTIN_COLON, |vm, argc| {
        let args = pop_args(vm, argc);
        // Direct set; see BUILTIN_TRUE above for rationale.
        if args.is_empty() {
            crate::ported::params::set_zunderscore(&[":".to_string()]);
        } else {
            crate::ported::params::set_zunderscore(&args);
        }
        let status = dispatch_builtin(":", args);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_TEST, |vm, argc| {
        let args = pop_args(vm, argc);
        // Distinguish `[ … ]` from `test …` by sniffing the trailing
        // `]` — `[` requires it (c:Src/builtin.c:7241), `test` rejects
        // it. The compile path emits BUILTIN_TEST for both, so the
        // dispatch name carries the `[` vs `test` semantic for
        // execbuiltin's funcid (BIN_BRACKET=21 vs BIN_TEST=20). Without
        // this, bin_test's `if func == BIN_BRACKET` arm (which pops
        // the trailing `]`) never fired for `[` calls, so the `]`
        // leaked into evalcond as a positional and silently changed
        // the result. Bug surfaced via test_test_dashdash_unknown_condition.
        let name = if args.last().map(|s| s.as_str()) == Some("]") {
            "["
        } else {
            "test"
        };
        let status = dispatch_builtin(name, args);
        Value::Status(status)
    });

    // Variable declaration. `local` (Src/builtin.c bin_local) handles
    // the scope chain (`pm->old = oldpm` at Src/params.c:1137 inside
    // createparam, `pm->level = locallevel` at Src/builtin.c:2576).
    // `typeset` / `declare` are aliases — fusevm maps both to
    // BUILTIN_TYPESET; compile_zsh special-cases `declare` to keep
    // the `declare:` error prefix.
    reg_passthru!(vm, BUILTIN_LOCAL, "local");
    reg_passthru!(vm, BUILTIN_TYPESET, "typeset");

    reg_passthru!(vm, BUILTIN_DECLARE, "declare");
    reg_passthru!(vm, BUILTIN_READONLY, "readonly");
    reg_passthru!(vm, BUILTIN_INTEGER, "integer");
    reg_passthru!(vm, BUILTIN_FLOAT, "float");
    reg_passthru!(vm, BUILTIN_READ, "read");
    // c:Bug #504 — fusevm reserves BUILTIN_MAPFILE for the bash
    // mapfile/readarray builtins. Neither exists in zsh; in --zsh
    // parity mode the dispatch must emit "command not found" + rc=127
    // matching zsh's external-command-lookup miss. The previous wiring
    // left BUILTIN_MAPFILE unregistered, so fusevm's VM treated the op
    // as a no-op rc=0 — `mapfile` (and `readarray`) silently succeeded
    // in --zsh mode. The host gate in `dispatch_builtin_raw` never
    // fired because the compile path emitted `Op::CallBuiltin(31, ..)`
    // directly. Register the slot so the gate runs (or a future
    // non-zsh mode can wire in a real impl).
    vm.register_builtin(fusevm::shell_builtins::BUILTIN_MAPFILE, |vm, argc| {
        let args = pop_args(vm, argc);
        // The fusevm name→id map collapses both `mapfile` and
        // `readarray` to the same opcode; pick the right diagnostic
        // by sniffing the user's actual invocation. The xtrace ARGS
        // push earlier records the cmd-prefix as the bottom of the
        // popped argv, but `args` here excludes the prefix — so we
        // can't recover the user-typed name from the stack. Default
        // to `mapfile` (the more-common spelling); both produce
        // identical diagnostics in any case.
        if crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("zsh:1: command not found: mapfile");
            let _ = args;
            return Value::Status(127);
        }
        // Non-zsh modes (bash drop-in) get a passthru to the canonical
        // dispatcher in case a future port adds a real mapfile builtin
        // — until then the rc=1 unknown-command default applies.
        Value::Status(dispatch_builtin("mapfile", args))
    });
    reg_passthru!(vm, BUILTIN_BREAK, "break");
    reg_passthru!(vm, BUILTIN_CONTINUE, "continue");
    reg_passthru!(vm, BUILTIN_SHIFT, "shift");

    vm.register_builtin(BUILTIN_EVAL, |vm, argc| {
        // Direct port of `bin_eval(UNUSED(char *nam), char **argv, UNUSED(Options ops), UNUSED(int func))` body from Src/builtin.c:6151:
        //   `if (!*argv) return 0;`
        //   `prog = parse_string(zjoin(argv, ' ', 1), 1);`
        //   `execode(prog, 1, 0, "eval");`
        // The execode invocation lives here (not in the canonical
        // free-fn) because it must run through the bytecode VM's
        // current executor — the same VM that's mid-dispatch.
        let mut args = pop_args(vm, argc);
        // c:Src/builtin.c:407-411 — generic `--` end-of-options
        // strip applied by `execbuiltin` for builtins that have
        // NULL optstr AND no BINF_HANDLES_OPTS. `eval` qualifies
        // (Src/builtin.c:65 `BUILTIN("eval", BINF_PSPECIAL, ...,
        // NULL, NULL)`). The BUILTIN_EVAL fast-path bypasses
        // execbuiltin, so we mirror the strip inline. Bug #319.
        if args.first().is_some_and(|s| s == "--") {
            args.remove(0);
        }
        if args.is_empty() {
            return Value::Status(0); // c:6160
        }
        let src = args.join(" "); // c:6166
        // c:Src/builtin.c:6164-6165 — `if (!ineval) scriptname =
        // "(eval)";`. Diagnostics emitted while the eval body runs
        // (command-not-found, parse errors, etc.) use scriptname as
        // the source-context prefix. Without setting it here the
        // BUILTIN_EVAL fast-path leaked the outer "zsh" prefix
        // through, breaking the `(eval):N:` convention zsh uses
        // for in-eval errors. Bug #420.
        let oscriptname = crate::ported::utils::scriptname_get();
        crate::ported::utils::set_scriptname(Some("(eval)".to_string()));
        let status = with_executor(|exec| {
            // c:6175 execode
            exec.execute_script(&src).unwrap_or(1)
        });
        crate::ported::utils::set_scriptname(oscriptname);
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
        // c:Src/exec.c:3435-3436 — `builtin NAME` with NAME not in
        // builtintab emits `zwarn("no such builtin: %s", cmdarg)`
        // and returns 1. zshrs's dispatch_builtin_raw bare-returned 1
        // silently. Probe the table here so the diagnostic fires
        // before dispatch.
        let tab = crate::ported::builtin::createbuiltintable();
        if !tab.contains_key(name.as_str()) {
            eprintln!("zshrs:1: no such builtin: {}", name);
            return Value::Status(1);
        }
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
    // to `exec::execcmd_compile_head` (the fusevm-bytecode-time head
    // resolver mirroring `Src/exec.c::execcmd_exec` precommand-modifier
    // walk at c:3104-3187). That helper already does the -p / -v / -V
    // option parsing, surfaces `has_command_vv` for the whence
    // redirect, and reports the dispatch shape (is_builtin vs external).
    vm.register_builtin(BUILTIN_COMMAND, |vm, argc| {
        let args = pop_args(vm, argc);
        let mut full = Vec::with_capacity(args.len() + 1);
        full.push("command".to_string());
        full.extend(args.clone());
        let dispatch =
            crate::ported::exec::execcmd_compile_head(&full, crate::ported::zsh_h::WC_SIMPLE);
        let post = &full[dispatch.precmd_skip..];
        // c:Src/builtin.c:4500 — `command -p` resets PATH for the
        // exec to the POSIX-defined default (`getconf PATH`), so
        // standard utilities resolve even when the caller has
        // emptied $PATH. zsh restores the original PATH after the
        // command returns. Mirror via a scoped env::set_var.
        let dash_p = args.iter().any(|a| {
            a == "-p"
                || a == "-pv"
                || a == "-pV"
                || (a.starts_with('-') && a.contains('p') && !a.starts_with("--"))
        });
        // The post slice from execcmd_compile_head may still contain
        // `-p` as the first element because precmd-modifier opt
        // parsing isn't wired here. Strip it manually so the dispatch
        // below sees the real command name.
        let mut post: Vec<String> = if dash_p {
            post.iter()
                .filter(|a| {
                    let s = a.as_str();
                    !(s.starts_with('-')
                        && s.len() >= 2
                        && s[1..].chars().all(|c| c == 'p' || c == 'v' || c == 'V'))
                })
                .cloned()
                .collect()
        } else {
            post.to_vec()
        };
        // c:Src/exec.c:3176-3177 — `BINF_COMMAND` arm strips a single
        // leading `--` end-of-options marker.
        // `execcmd_compile_head` (src/ported/exec.rs:1042) performs
        // this removal on its LOCAL `preargs` Vec but doesn't surface
        // the modified args; the caller still sees `--` in `full` and
        // tried to dispatch it as the command name. Bug #251. Mirror
        // the C strip here so `command -- echo hi` and
        // `command -p -- echo hi` route correctly.
        if let Some(first) = post.first() {
            if first == "--" {
                post.remove(0);
            }
        }
        let post = post.as_slice();
        let _path_guard = if dash_p {
            let saved = env::var("PATH").ok();
            let default_path = std::process::Command::new("getconf")
                .arg("PATH")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "/usr/bin:/bin:/usr/sbin:/sbin".to_string());
            env::set_var("PATH", &default_path);
            crate::ported::params::setsparam("PATH", &default_path);
            Some(saved)
        } else {
            None
        };
        struct PathGuard {
            saved: Option<String>,
            active: bool,
        }
        impl Drop for PathGuard {
            fn drop(&mut self) {
                if !self.active {
                    return;
                }
                match self.saved.take() {
                    Some(p) => {
                        env::set_var("PATH", &p);
                        crate::ported::params::setsparam("PATH", &p);
                    }
                    None => {
                        env::remove_var("PATH");
                        crate::ported::params::setsparam("PATH", "");
                    }
                }
            }
        }
        let _restore = PathGuard {
            saved: _path_guard.unwrap_or(None),
            active: dash_p,
        };
        if dispatch.has_command_vv {
            // `-v` / `-V` → bin_whence with BIN_COMMAND funcid.
            let mut ops = options {
                ind: [0u8; MAX_OPS],
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
        // c:Src/exec.c:3275-3278 — `execcmd_compile_head` cleared
        // hn for the BINF_COMMAND + !POSIXBUILTINS case, surfacing
        // is_builtin=false. Run as external. Under POSIXBUILTINS
        // dispatch.is_builtin would be true; honour it.
        let n = name.clone();
        let r = rest.to_vec();
        if dispatch.is_builtin
            && crate::ported::builtin::BUILTINS
                .iter()
                .any(|b| b.node.nam == n.as_str())
        {
            return Value::Status(dispatch_builtin_raw(&n, r));
        }
        Value::Status(with_executor(|exec| exec.execute_external(&n, &r, &[])).unwrap_or(127))
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
        // c:Src/builtin.c:1075-1080 — track if any flag was consumed.
        // `exec -c`, `exec -l`, `exec -a NAME` without a following
        // command emit "exec requires a command to execute" rc=1.
        // Bare `exec` (no args at all) is the silent-redirect-apply
        // form per POSIX.
        let mut saw_flag = false;
        while i < args.len() {
            let a = &args[i];
            if a == "--" {
                args.remove(i);
                break;
            }
            // c:Src/builtin.c:42 `BIN_PREFIX("-", BINF_DASH)`. A bare
            // `-` is its own BINF_PREFIX builtin (BINF_DASH flag —
            // "login shell, prepend `-` to argv[0]"). In the canonical
            // precmd-walk at Src/exec.c:3056-3091 a bare `-` after
            // `exec` is recognized AS a builtin and stripped from
            // preargs (precmd_skip++), accumulating BINF_DASH into
            // cflags. The fast-path here bypasses execcmd_compile_head,
            // so we mirror the strip locally: bare `-` → set login,
            // remove, continue. Without this `exec -` (with no command
            // following) tried to exec `-` as a literal command and
            // exited the shell. Bug #252.
            if a == "-" {
                saw_flag = true;
                login = true;
                args.remove(i);
                continue;
            }
            if !a.starts_with('-') || a.len() < 2 {
                break;
            }
            match a.as_str() {
                "-a" => {
                    saw_flag = true;
                    args.remove(i);
                    if i < args.len() {
                        argv0_override = Some(args.remove(i));
                    }
                }
                "-c" => {
                    saw_flag = true;
                    clean_env = true;
                    args.remove(i);
                }
                "-l" => {
                    saw_flag = true;
                    login = true;
                    args.remove(i);
                }
                _ => {
                    // c:Src/exec.c:3196-3208 — when an unrecognized
                    // `-X`-style arg has NO following arg, the lexer's
                    // IS_DASH walk hits the "no next node" branch at
                    // c:3199 before the unknown-flag-letter switch at
                    // c:3249, so the canonical message is "exec
                    // requires a command to execute" rc=1 (verified vs
                    // `/opt/homebrew/bin/zsh -fc 'exec --bad'`).
                    // Consume the lone flag so the post-loop check
                    // fires. When a following arg exists, leave the
                    // unknown-flag arg in place — that arg becomes
                    // the command name and execution proceeds.
                    if args.len() == 1 {
                        saw_flag = true;
                        args.remove(i);
                        continue;
                    }
                    break;
                }
            }
        }
        let Some(cmd) = args.first().cloned() else {
            if saw_flag {
                // c:Src/builtin.c:1078-1080 — flags consumed but no
                // command follows → "exec requires a command to
                // execute" rc=1.
                eprintln!("zshrs:1: exec requires a command to execute");
                return Value::Status(1);
            }
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
        // c:Src/exec.c::execcmd — `exec funcname` runs the function
        // in-process as the shell's last act, then exits with the
        // function's status. zsh's dispatcher falls through from the
        // BINF_EXEC prefix into the normal Builtin/External/Function
        // resolution and only execvp's if the target ISN'T a
        // function. Bug #101 in docs/BUGS.md: zshrs's exec went
        // straight to execvp and errored `not found` for shell
        // functions.
        //
        // For both subshell and top-level contexts: dispatch through
        // the function/builtin lookup first; only fall through to
        // execvp/spawn if the name isn't shell-resolvable.
        let has_user_fn = with_executor(|exec| exec.functions_compiled.contains_key(&cmd));
        if has_user_fn {
            let status = with_executor(|exec| {
                exec.dispatch_function_call(&cmd, &rest).unwrap_or(127)
            });
            // Top-level `exec funcname` — exit the shell with the
            // function's status (mirrors C's "exec replaces shell as
            // last act"). Subshell `(exec funcname)` — return through
            // the EXIT_PENDING path so the subshell body aborts and
            // the parent resumes via subshell_end.
            let in_subshell_now =
                with_executor(|exec| !exec.subshell_snapshots.is_empty());
            if in_subshell_now {
                crate::ported::builtin::EXIT_VAL
                    .store(status, std::sync::atomic::Ordering::Relaxed);
                crate::ported::builtin::EXIT_PENDING
                    .store(1, std::sync::atomic::Ordering::Relaxed);
                return Value::Status(status);
            }
            std::process::exit(status);
        }
        // c:Src/exec.c — builtin path: `exec builtin` runs the
        // builtin in-process and exits.
        let bn_in_tab = crate::ported::builtin::createbuiltintable().contains_key(&cmd);
        if bn_in_tab {
            let status = dispatch_builtin_raw(&cmd, rest.clone());
            let in_subshell_now =
                with_executor(|exec| !exec.subshell_snapshots.is_empty());
            if in_subshell_now {
                crate::ported::builtin::EXIT_VAL
                    .store(status, std::sync::atomic::Ordering::Relaxed);
                crate::ported::builtin::EXIT_PENDING
                    .store(1, std::sync::atomic::Ordering::Relaxed);
                return Value::Status(status);
            }
            std::process::exit(status);
        }
        // c:Src/exec.c — `exec` inside a subshell (`(exec cmd)`)
        // replaces ONLY the subshell child process; the parent shell
        // continues. C zsh always forks for `(...)`, so the actual
        // execvp lands in the forked child. zshrs runs subshells via
        // a snapshot/restore pattern in the SAME process — calling
        // execvp here would replace the parent too. Bug #94 in
        // docs/BUGS.md.
        //
        // Detect subshell context via the non-empty
        // `subshell_snapshots` stack. When in a subshell: spawn the
        // command as a child, wait for it, then signal the subshell
        // body to abort (return Status(N) and the caller's
        // subshell_end will pop the snapshot and resume the parent).
        let in_subshell = with_executor(|exec| !exec.subshell_snapshots.is_empty());
        if in_subshell {
            let mut command = std::process::Command::new(&cmd);
            command.arg0(&display_argv0);
            command.args(&rest);
            if clean_env {
                command.env_clear();
            }
            let status = match command.spawn() {
                Ok(mut child) => match child.wait() {
                    Ok(s) => s.code().unwrap_or(127),
                    Err(_) => 127,
                },
                Err(e) => {
                    // c:Src/exec.c:797 — `zerr("%e: %s", lerrno, arg0)`
                    //                     when arg0 contains `/`.
                    // c:872-876 — when arg0 has no `/` (PATH search
                    //              path), C tracks the "good" errno
                    //              via `isgooderr`; if all PATH entries
                    //              were ENOENT-not-good, eno stays 0
                    //              and C emits `command not found: %s`
                    //              instead of strerror.
                    // %e expands to strerror(errno) with the first
                    // letter lowercased (unless errno == EIO; see
                    // Src/utils.c:362-368). `zerr` prepends the
                    // scriptname:lineno: prefix — matching zsh's
                    // canonical `zsh:N: <errmsg>: <cmd>` pattern.
                    // Previously emitted `zshrs: exec: {}: not found`
                    // (wrong prefix, hardcoded message, missing
                    // lineno). Bug #140 in docs/BUGS.md.
                    let errno = e.raw_os_error().unwrap_or(libc::ENOENT);
                    let has_slash = cmd.contains('/');
                    if !has_slash && errno == libc::ENOENT {
                        // c:876 — PATH search exhausted with no good
                        // errno → `command not found: arg0`.
                        crate::ported::utils::zerr(&format!(
                            "command not found: {}",
                            cmd
                        ));
                    } else {
                        let mut errmsg = crate::ported::compat::strerror(errno);
                        if errno != libc::EIO {
                            if let Some(c) = errmsg.chars().next() {
                                errmsg = format!(
                                    "{}{}",
                                    c.to_ascii_lowercase(),
                                    &errmsg[c.len_utf8()..]
                                );
                            }
                        }
                        crate::ported::utils::zerr(&format!("{}: {}", errmsg, cmd));
                    }
                    // c:881 — `_exit((eno == EACCES || eno == ENOEXEC) ? 126 : 127);`
                    if errno == libc::EACCES || errno == libc::ENOEXEC {
                        126
                    } else {
                        127
                    }
                }
            };
            // Mark the subshell as exec-replaced so subsequent body
            // commands skip — mirrors the post-execvp "child process
            // is gone" reality in C. EXIT_PENDING + EXIT_VAL drive
            // the next ERREXIT_CHECK to unwind to the subshell-end
            // patch.
            crate::ported::builtin::EXIT_VAL
                .store(status, std::sync::atomic::Ordering::Relaxed);
            crate::ported::builtin::EXIT_PENDING
                .store(1, std::sync::atomic::Ordering::Relaxed);
            return Value::Status(status);
        }
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
        // c:Src/exec.c:797 / c:872-876 — same format as in-subshell
        // branch. arg0-has-/ → `<strerror>: <cmd>`; arg0-no-/ +
        // ENOENT → `command not found: <cmd>`. Lowercase strerror
        // first letter unless EIO. Bug #140 in docs/BUGS.md.
        let errno = err.raw_os_error().unwrap_or(libc::ENOENT);
        let has_slash = cmd.contains('/');
        if !has_slash && errno == libc::ENOENT {
            crate::ported::utils::zerr(&format!("command not found: {}", cmd));
        } else {
            let mut errmsg = crate::ported::compat::strerror(errno);
            if errno != libc::EIO {
                if let Some(c) = errmsg.chars().next() {
                    errmsg = format!(
                        "{}{}",
                        c.to_ascii_lowercase(),
                        &errmsg[c.len_utf8()..]
                    );
                }
            }
            crate::ported::utils::zerr(&format!("{}: {}", errmsg, cmd));
        }
        // c:881 — `_exit((eno == EACCES || eno == ENOEXEC) ? 126 : 127);`
        let code = if errno == libc::EACCES || errno == libc::ENOEXEC {
            126
        } else {
            127
        };
        std::process::exit(code);
    });

    reg_passthru!(vm, BUILTIN_LET, "let");

    // Job control
    reg_passthru!(vm, BUILTIN_JOBS, "jobs");
    reg_passthru!(vm, BUILTIN_FG, "fg");
    reg_passthru!(vm, BUILTIN_BG, "bg");
    reg_passthru!(vm, BUILTIN_KILL, "kill");
    reg_passthru!(vm, BUILTIN_DISOWN, "disown");
    reg_passthru!(vm, BUILTIN_WAIT, "wait");
    reg_passthru!(vm, BUILTIN_SUSPEND, "suspend");

    // History — `fc` / `history` / `r` all route to `bin_fc` (zsh
    // registers them as aliases of the same builtin per Src/builtin.c).
    reg_passthru!(vm, BUILTIN_FC, "fc");
    reg_passthru!(vm, BUILTIN_HISTORY, "history");
    reg_passthru!(vm, BUILTIN_R, "r");

    // Aliases
    reg_passthru!(vm, BUILTIN_ALIAS, "alias");

    // Options. `setopt` (BIN_SETOPT=0) / `unsetopt` (BIN_UNSETOPT=1)
    // share bin_setopt (options.c:580) — funcid bit discriminates
    // the polarity via BUILTINS table entries.
    reg_passthru!(vm, BUILTIN_SET, "set");
    reg_passthru!(vm, BUILTIN_SETOPT, "setopt");
    reg_passthru!(vm, BUILTIN_UNSETOPT, "unsetopt");

    vm.register_builtin(BUILTIN_SHOPT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = crate::extensions::ext_builtins::shopt(&args);
        Value::Status(status)
    });

    reg_passthru!(vm, BUILTIN_EMULATE, "emulate");
    reg_passthru!(vm, BUILTIN_GETOPTS, "getopts");
    reg_passthru!(vm, BUILTIN_AUTOLOAD, "autoload");
    reg_passthru!(vm, BUILTIN_FUNCTIONS, "functions");
    reg_passthru!(vm, BUILTIN_TRAP, "trap");
    reg_passthru!(vm, BUILTIN_DIRS, "dirs");
    // pushd / popd dispatch through canonical bin_cd via execbuiltin
    // — the BUILTINS table at src/ported/builtin.rs:9298 wires
    // `pushd` to bin_cd with funcid=BIN_PUSHD, and `popd` similarly
    // with BIN_POPD. Without these reg_passthru lines the fusevm
    // BUILTIN_PUSHD/POPD opcodes had no handler installed, so the
    // emitted CallBuiltin(110, …) silently returned a no-op and the
    // dirstack/$dirstack/pwd all stayed unchanged.
    reg_passthru!(vm, BUILTIN_PUSHD, "pushd");
    reg_passthru!(vm, BUILTIN_POPD, "popd");
    // type / whence / where / which all route through `bin_whence`
    // (canonical port at `src/ported/builtin.rs:3734` of
    // `Src/builtin.c:3975`). Each gets its own opcode so funcid +
    // defopts come from the BUILTINS table entry — execbuiltin
    // applies them correctly via the module-level dispatch_builtin.
    reg_passthru!(vm, BUILTIN_WHENCE, "whence");
    reg_passthru!(vm, BUILTIN_TYPE, "type");
    reg_passthru!(vm, BUILTIN_WHICH, "which");
    reg_passthru!(vm, BUILTIN_WHERE, "where");
    reg_passthru!(vm, BUILTIN_HASH, "hash");
    reg_passthru!(vm, BUILTIN_REHASH, "rehash");

    // `unhash`/`unalias`/`unfunction` share `bin_unhash` (Src/builtin.c
    // c:4350) but each carries its own funcid (BIN_UNHASH /
    // BIN_UNALIAS / BIN_UNFUNCTION) in the BUILTINS table.
    reg_passthru!(vm, BUILTIN_UNHASH, "unhash");
    vm.register_builtin(BUILTIN_UNALIAS, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(dispatch_builtin("unalias", args))
    });
    vm.register_builtin(BUILTIN_UNFUNCTION, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(dispatch_builtin("unfunction", args))
    });

    // Completion
    vm.register_builtin(BUILTIN_COMPGEN, |vm, argc| {
        let args = pop_args(vm, argc);
        // c:Bug #475/#555 — `compgen` is a bash-only builtin. In
        // `--zsh` mode emit "command not found" matching zsh's
        // external-command lookup miss.
        if crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("zsh:1: command not found: compgen");
            let _ = args;
            return Value::Status(127);
        }
        let status = with_executor(|exec| exec.builtin_compgen(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_COMPLETE, |vm, argc| {
        let args = pop_args(vm, argc);
        // c:Bug #475 — `complete` is a bash-only builtin. Same gate
        // as BUILTIN_COMPGEN above.
        if crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("zsh:1: command not found: complete");
            let _ = args;
            return Value::Status(127);
        }
        let status = with_executor(|exec| exec.builtin_complete(&args));
        Value::Status(status)
    });

    reg_passthru!(vm, BUILTIN_COMPADD, "compadd");
    reg_passthru!(vm, BUILTIN_COMPSET, "compset");

    vm.register_builtin(BUILTIN_COMPDEF, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(with_executor(|exec| exec.builtin_compdef(&args)))
    });

    vm.register_builtin(BUILTIN_COMPINIT, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(with_executor(|exec| exec.builtin_compinit(&args)))
    });

    vm.register_builtin(BUILTIN_CDREPLAY, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(with_executor(|exec| exec.builtin_cdreplay(&args)))
    });

    // Zsh-specific
    reg_passthru!(vm, BUILTIN_ZSTYLE, "zstyle");
    reg_passthru!(vm, BUILTIN_ZMODLOAD, "zmodload");
    reg_passthru!(vm, BUILTIN_BINDKEY, "bindkey");
    reg_passthru!(vm, BUILTIN_ZLE, "zle");
    reg_passthru!(vm, BUILTIN_VARED, "vared");
    reg_passthru!(vm, BUILTIN_ZCOMPILE, "zcompile");
    reg_passthru!(vm, BUILTIN_ZFORMAT, "zformat");
    reg_passthru!(vm, BUILTIN_ZPARSEOPTS, "zparseopts");
    reg_passthru!(vm, BUILTIN_ZREGEXPARSE, "zregexparse");

    // Resource limits
    reg_passthru!(vm, BUILTIN_ULIMIT, "ulimit");
    reg_passthru!(vm, BUILTIN_LIMIT, "limit");
    reg_passthru!(vm, BUILTIN_UNLIMIT, "unlimit");
    reg_passthru!(vm, BUILTIN_UMASK, "umask");

    // Misc
    reg_passthru!(vm, BUILTIN_TIMES, "times");

    vm.register_builtin(BUILTIN_CALLER, |vm, argc| {
        let args = pop_args(vm, argc);
        // c:Bug #475 — `caller` is a bash-only builtin. In `--zsh`
        // mode emit the canonical "command not found" diagnostic
        // and rc=127 matching zsh's external-command-lookup miss.
        if crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("zsh:1: command not found: caller");
            let _ = args;
            return Value::Status(127);
        }
        Value::Status(with_executor(|exec| exec.builtin_caller(&args)))
    });

    vm.register_builtin(BUILTIN_HELP, |vm, argc| {
        let args = pop_args(vm, argc);
        // c:Bug #475 — `help` is a bash-only builtin. Same gate as
        // BUILTIN_CALLER above.
        if crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("zsh:1: command not found: help");
            let _ = args;
            return Value::Status(127);
        }
        Value::Status(with_executor(|exec| exec.builtin_help(&args)))
    });

    reg_passthru!(vm, BUILTIN_ENABLE, "enable");
    reg_passthru!(vm, BUILTIN_DISABLE, "disable");
    reg_passthru!(vm, BUILTIN_TTYCTL, "ttyctl");
    reg_passthru!(vm, BUILTIN_SYNC, "sync");
    reg_passthru!(vm, BUILTIN_MKDIR, "mkdir");
    reg_passthru!(vm, BUILTIN_STRFTIME, "strftime");

    vm.register_builtin(BUILTIN_ZSLEEP, |vm, argc| {
        Value::Status(crate::extensions::ext_builtins::zsleep(&pop_args(vm, argc)))
    });

    reg_passthru!(vm, BUILTIN_ZSYSTEM, "zsystem");

    // PCRE
    reg_passthru!(vm, BUILTIN_PCRE_COMPILE, "pcre_compile");
    reg_passthru!(vm, BUILTIN_PCRE_MATCH, "pcre_match");
    reg_passthru!(vm, BUILTIN_PCRE_STUDY, "pcre_study");

    // Database (GDBM)
    reg_passthru!(vm, BUILTIN_ZTIE, "ztie");
    reg_passthru!(vm, BUILTIN_ZUNTIE, "zuntie");
    reg_passthru!(vm, BUILTIN_ZGDBMPATH, "zgdbmpath");

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

    reg_passthru!(vm, BUILTIN_ZPROF, "zprof");

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
    // `cp` — zshrs extension (NOT in upstream zsh; upstream's
    // zsh/files module ships `ln`/`mv`/`rm`/`chmod`/`chown` but no
    // `cp`). In-process implementation in
    // `ext_builtins::cp_impl` — recursive copy with -r/-R, -f, -i,
    // -n, -p (chown + utimensat), -v. ID 263 is the first slot
    // past fusevm's built-in range (260-262) and before BUILTIN_MAX
    // (280).
    /// `BUILTIN_CP` constant.
    pub const BUILTIN_CP: u16 = 263;
    reg_overridable!(vm, BUILTIN_CP, "cp", builtin_cp);

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
                    // c:Src/exec.c — pipeline children are forked
                    // subshells; their EXIT trap context is reset so
                    // the parent's `trap '...' EXIT` doesn't fire when
                    // the child exits. Mirror by dropping EXIT from
                    // the inherited traps_table inside the child.
                    if let Ok(mut t) = crate::ported::builtin::traps_table().lock() {
                        t.remove("EXIT");
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
        let pipefail_on = with_executor(|exec| opt_state_get("pipefail").unwrap_or(false));
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

        // c:Src/params.c:265,438 — only `pipestatus` (lowercase) is the
        // zsh special parameter; bash's `PIPESTATUS` doesn't exist in
        // zsh's special-params table. Prior port also populated
        // `PIPESTATUS` "for portability" — but that's a real divergence
        // from zsh: a script doing `[[ -z $PIPESTATUS ]]` to detect
        // zsh-vs-bash would mis-classify. Bug #64 in docs/BUGS.md.
        with_executor(|exec| {
            let strs: Vec<String> = pipestatus.iter().map(|s| s.to_string()).collect();
            exec.set_array("pipestatus".to_string(), strs);
        });

        Value::Status(last_status)
    });

    // Array→String join. Pops one value; if it's an Array (e.g. from Op::Glob),
    // joins string-coerced elements with a single space. Pass-through for
    // non-arrays so the op is safe to chain after any String-or-Array producer.
    vm.register_builtin(BUILTIN_ARRAY_JOIN, |vm, _argc| {
        let val = vm.pop();
        match val {
            Value::Array(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_str()).collect();
                Value::str(parts.join(" "))
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
                // c:Src/jobs.c:73 — `lastpid = pid;` after a
                // background fork. zshrs's `$!` getter
                // (params.rs::lookup_special_var "!") reads from
                // the same atomic, so a single store here is the
                // canonical writer.
                crate::ported::modules::clone::lastpid
                    .store(pid, std::sync::atomic::Ordering::Relaxed);
                with_executor(|exec| {
                    exec.jobs.add_pid_job(pid, String::new(), JobState::Running);
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
        let mut popped: Vec<Value> = Vec::with_capacity(n);
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
                Value::Array(items) => {
                    for it in items {
                        values.push(it.to_str());
                    }
                }
                other => values.push(other.to_str()),
            }
        }
        let blocked = with_executor(|exec| {
            // Assoc init `typeset -A m; m=(k v k v ...)` — route to
            // canonical sethparam (Src/params.c:3602) which parses the
            // flat (k,v) pair list internally.
            if exec.assoc(&name).is_some() {
                if !values.len().is_multiple_of(2) {
                    eprintln!(
                        "{}:1: bad set of key/value pairs for associative array",
                        shname()
                    );
                    return true;
                }
                let _ = crate::ported::params::sethparam(&name, values.clone());
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
            // Indexed-array: setaparam (Src/params.c:3766) wraps
            // assignaparam with ASSPM_WARN — handles PM_UNIQUE dedupe,
            // type-flag flip, PM_READONLY rejection.
            //
            // The tied-array mirror to a PM_TIED scalar
            // (`typeset -T PATH path`) lives canonically in
            // setarrvalue's dispatch in C zsh; until that wires
            // through assignaparam, mirror here so PATH stays in sync
            // after `path=(/x)`.
            if let Some((scalar_name, sep)) = exec.tied_array_to_scalar.get(&name).cloned() {
                let joined = values.join(&sep);
                exec.set_scalar(scalar_name, joined);
            }
            let _ = crate::ported::params::setaparam(&name, values.clone());
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
        let mut popped: Vec<Value> = Vec::with_capacity(n);
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
                Value::Array(items) => {
                    for it in items {
                        values.push(it.to_str());
                    }
                }
                other => values.push(other.to_str()),
            }
        }
        with_executor(|exec| {
            // Assoc append `m+=(k1 v1 ...)`: merge the (k,v) pairs into
            // the existing map and write back via canonical sethparam
            // (Src/params.c:3602). The canonical C path would go
            // assignaparam(ASSPM_AUGMENT) → arrhashsetfn(ASSPM_AUGMENT)
            // at Src/params.c:3850, but the zshrs port of
            // arrhashsetfn doesn't yet implement value-storage
            // (pending Param.u_hash backend wireup) — until that
            // lands, do the augment + write here so the storage
            // actually mutates.
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
            // Indexed-array append `arr+=(d e f)` — route directly
            // through canonical assignaparam with ASSPM_AUGMENT
            // (`Src/params.c:3570-3585` append-on-array branch).
            // assignaparam reads the prior array internally and
            // appends the new values, so the bridge no longer needs
            // to pre-concat manually.
            let _ = crate::ported::params::assignaparam(
                &name,
                values.clone(),
                crate::ported::zsh_h::ASSPM_AUGMENT,
            );
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                let ctx = exec.recorder_ctx();
                let attrs = exec.recorder_attrs_for(&name);
                emit_path_or_assign(&name, &values, attrs, true, &ctx);
            }
            // Tied-scalar mirror — TODO faithful: should live in
            // setarrvalue's gsu dispatch once boot_ paramtab wiring
            // lands (Task #16). Re-read the canonical post-augment
            // array so the joined scalar matches.
            let tied_scalar = exec.tied_array_to_scalar.get(&name).cloned();
            if let Some((scalar_name, sep)) = tied_scalar {
                let merged = exec.array(&name).unwrap_or_default();
                let joined = merged.join(&sep);
                exec.set_scalar(scalar_name.clone(), joined.clone());
                env::set_var(&scalar_name, &joined);
            }
        });
        Value::Status(0)
    });
    vm.register_builtin(BUILTIN_RUN_SELECT, |vm, argc| {
        if argc < 2 {
            return Value::Status(1);
        }
        let n = argc as usize;
        let mut popped: Vec<Value> = Vec::with_capacity(n);
        for _ in 0..n {
            popped.push(vm.pop());
        }
        // popped: [sub_idx, name, word_N, ..., word_1] (popping from top)
        let sub_idx_val = popped.remove(0);
        let name_val = popped.remove(0);
        // c:Src/loop.c — `select` flattens Array values (from `$@`,
        // `${arr[@]}`, etc.) into the menu. Without per-element
        // splice, `select x do ... done` (bare, iterating $@)
        // collapsed all positionals into one joined entry.
        let mut words: Vec<String> = Vec::new();
        for v in popped.into_iter().rev() {
            match v {
                Value::Array(items) => {
                    for item in items {
                        words.push(item.to_str());
                    }
                }
                other => words.push(other.to_str()),
            }
        }

        let sub_idx = sub_idx_val.to_int() as usize;
        let name = name_val.to_str();

        // c:Src/loop.c:248-252 — `if (!args || empty(args)) {
        // state->pc = end; ... return 0; }`. An empty option list
        // skips the body entirely; without this gate the prompt loop
        // runs indefinitely (or twice on the EOF stdin case before
        // exiting). Bug #401.
        if words.is_empty() {
            return Value::Status(0);
        }

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
            let term_width: usize = env::var("COLUMNS")
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
                Ok(0) => {
                    // EOF — emit the final newline that zsh prints
                    // after the prompt-then-EOF sequence (c:Src/loop.c
                    // selectlist falls through to fputc('\n', stderr)
                    // at the end of the read failure path). Without
                    // this the next process's output runs directly
                    // after `-->>>> ` on the same line.
                    let _ = writeln!(std::io::stderr());
                    let _ = std::io::stderr().flush();
                    break;
                }
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

            // Reset canonical BREAKS/CONTFLAG before running the body
            // so a stale value from a sibling construct doesn't leak in.
            crate::ported::builtin::BREAKS.store(0, SeqCst);
            crate::ported::builtin::CONTFLAG.store(0, SeqCst);

            // c:Src/loop.c — `select` increments LOOPS for the body so
            // `break` / `continue` inside the body see loops > 0 and
            // don't emit `not in while, until, select, or repeat loop`.
            // Mirrors execwhile/execrepeat's `LOOPS.fetch_add` pattern.
            // The decrement happens after the body call so a body that
            // explicitly returns / errors still leaves the counter
            // balanced for the next iteration.
            crate::ported::builtin::LOOPS.fetch_add(1, SeqCst);

            crate::fusevm_disasm::maybe_print_stdout("select:body", &chunk);
            let mut body_vm = fusevm::VM::new(chunk.clone());
            register_builtins(&mut body_vm);
            let _ = body_vm.run();
            last_status = body_vm.last_status;

            crate::ported::builtin::LOOPS.fetch_sub(1, SeqCst);

            // Drain the canonical BREAKS/CONTFLAG counters. Mirrors
            // loop.c:529-534's `if (breaks) { breaks--; if (breaks ||
            // !contflag) break; contflag = 0; }` drain pattern.
            // The legacy `BREAK_SELECT=1` env-var sentinel is still
            // honored for backward compat.
            let break_legacy = with_executor(|exec| {
                let v = exec.scalar("BREAK_SELECT");
                exec.unset_scalar("BREAK_SELECT");
                v.map(|s| s != "0" && !s.is_empty()).unwrap_or(false)
            });
            use std::sync::atomic::Ordering::SeqCst;
            let breaks = crate::ported::builtin::BREAKS.load(SeqCst);
            if breaks > 0 {
                let cont = crate::ported::builtin::CONTFLAG.load(SeqCst);
                crate::ported::builtin::BREAKS.fetch_sub(1, SeqCst);
                if breaks - 1 > 0 || cont == 0 {
                    break;
                }
                crate::ported::builtin::CONTFLAG.store(0, SeqCst);
                continue;
            }
            if break_legacy {
                break;
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
    // Outer-flag dispatch (`(@)` / `(@k)` / `(v)NAME[(I)pat]` / etc.)
    // routes through BUILTIN_BRIDGE_BRACE_ARRAY at the compile path
    // (canonical paramsubst flag parser owns dispatch at Src/subst.c:2147+),
    // so BUILTIN_ARRAY_INDEX receives clean name+key with no sentinel
    // prefixes.
    vm.register_builtin(BUILTIN_ARRAY_INDEX, |vm, _argc| {
        let idx = vm.pop().to_str();
        let name = vm.pop().to_str();
        // c:Src/subst.c subscript parsing — when paramsubst re-parses
        // the synthesized `${name[idx]}` body, characters like `'`
        // `"` `\` `$` etc. are LEXER-active inside the `[…]` and get
        // reinterpreted (quote-strip, paramsubst recursion, …). For
        // PRE-EVALUATED key strings (the dynamic-key fast path at
        // compile_zsh.rs:3234 already expanded `$k` via EXPAND_TEXT),
        // the idx is a literal string that must match the stored key
        // byte-for-byte — no further reinterpretation. Direct assoc
        // lookup bypasses the lexer for this case, avoiding the
        // quote-strip bug where `h[a'b]` failed to resolve because
        // paramsubst's subscript lexer treated the `'` as a quote.
        // Bug #338. Only fires for simple assoc-name + non-flag idx
        // (no outer-flag sentinels, no `(…)` flag prefix on idx, no
        // splat operator). Other paths (slice, splat, flag-based
        // search, magic-assoc) still flow through paramsubst.
        let idx_is_simple = !idx.starts_with('(')
            && idx != "@"
            && idx != "*"
            && !idx.contains(',');
        if idx_is_simple {
            if let Some(v) = with_executor(|exec| {
                exec.assoc(&name).and_then(|m| m.get(&idx).cloned())
            }) {
                return Value::str(v);
            }
        }
        let body = format!("${{{}[{}]}}", name, idx);
        paramsubst_to_value(&body)
    });
    // BUILTIN_ASSOC_HAS_KEY — `${(k)assoc[name]}` key-existence query.
    // Pops [assoc_name, key]; returns key (Str) if present in the
    // assoc, empty Str otherwise. Mirrors zsh's `${(k)h[name]}`
    // documented semantics in zshparam(1) "Parameter Expansion Flags".
    // Distinct from BUILTIN_ARRAY_INDEX (which returns the VALUE) and
    // from `${+h[name]}` (which returns "0"/"1"). Bug #145.
    vm.register_builtin(BUILTIN_ASSOC_HAS_KEY, |vm, _argc| {
        let key = vm.pop().to_str();
        let name = vm.pop().to_str();
        let exists = crate::ported::params::gethkparam(&name)
            .map(|keys| keys.iter().any(|k| k == &key))
            .unwrap_or(false);
        if exists {
            Value::str(key)
        } else {
            Value::str("")
        }
    });
    vm.register_builtin(BUILTIN_BRIDGE_BRACE_ARRAY, |vm, _argc| {
        // Inner body of `${(...)...}` (already stripped of `${`/`}` by
        // the caller). The compiler optionally prefixes Qstring
        // (\u{8c}) to signal "expanded in DQ context" — strip it
        // here and bump in_dq_context for the paramsubst call so the
        // SUB_ZIP and other qt-aware paths fire.
        let body = vm.pop().to_str();
        let (dq, inner) = if let Some(rest) = body.strip_prefix('\u{8c}') {
            (true, rest.to_string())
        } else {
            (false, body)
        };
        if dq {
            with_executor(|exec| exec.in_dq_context += 1);
        }
        let v = paramsubst_to_value(&format!("${{{}}}", inner));
        if dq {
            with_executor(|exec| exec.in_dq_context -= 1);
        }
        v
    });

    // BUILTIN_PARAM_FLAG — `${(flags)name}` paramsubst dispatch.
    // PURE PASSTHRU: pops sentinel-tagged flags + name, hands the
    // canonical `${(flags)name}` form to `subst::paramsubst` (C port
    // of `Src/subst.c::paramsubst`). The bridge does no flag
    // walking, no DQ-context branching, no array/scalar shape
    // selection — all of that lives inside paramsubst. Compile-time
    // context (DQ / scalar-assign-RHS) flows through executor cells
    // (in_dq_context, in_scalar_assign) bumped by BUILTIN_EXPAND_TEXT.
    vm.register_builtin(BUILTIN_PARAM_FLAG, |vm, _argc| {
        let flags = vm.pop().to_str();
        let name = vm.pop().to_str();
        let body = format!("${{({}){}}}", flags, name);
        paramsubst_to_value(&body)
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
            let is_assoc = exec.assoc(&name).is_some();
            let is_scalar = !is_indexed && !is_assoc && exec.scalar(&name).is_some();
            // c:Src/params.c::getindex — `(i)pat` / `(I)pat` / `(R)pat`
            // / `(r)pat` subscript flags on an indexed array LHS resolve
            // to a numeric index (first / last match of pat). On a
            // SCALAR LHS the same flags resolve to a CHAR position
            // (1-based first/last match of pat in the scalar string)
            // for the c:2748+ char-splice assignment. zshrs's
            // read-form `${a[(i)pat]}` already implements both shapes;
            // the LHS assignment path silently stored the literal
            // "(i)pat" as an assoc key (for scalar: auto-vivified to
            // PM_HASHED via the assignsparam unknown-subscript
            // fallback). Bug #293 (array) / scalar sibling.
            //
            // Detect the `(flags)pat` shape and resolve to a numeric
            // index before assignsparam.
            if is_indexed || is_scalar {
                if let Some(rest) = key.strip_prefix('(') {
                    if let Some(close) = rest.find(')') {
                        let flags = &rest[..close];
                        let pat = &rest[close + 1..];
                        if !flags.is_empty()
                            && flags
                                .chars()
                                .all(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'n' | 'e'))
                        {
                            // Resolve via the array's contents.
                            if let Some(arr) = exec.array(&name) {
                                let return_index = true; // LHS write — index needed
                                let down = flags.contains('I') || flags.contains('R');
                                let exact = flags.contains('e');
                                let iter: Box<dyn Iterator<Item = (usize, &String)>> = if down {
                                    Box::new(arr.iter().enumerate().rev())
                                } else {
                                    Box::new(arr.iter().enumerate())
                                };
                                let mut found: Option<usize> = None;
                                for (idx, elem) in iter {
                                    let matched = if exact {
                                        elem == pat
                                    } else {
                                        crate::ported::pattern::patcompile(
                                            pat,
                                            crate::ported::zsh_h::PAT_HEAPDUP as i32,
                                            None,
                                        )
                                        .map_or(false, |p| {
                                            crate::ported::pattern::pattry(&p, elem)
                                        })
                                    };
                                    if matched {
                                        found = Some(idx);
                                        break;
                                    }
                                }
                                let _ = return_index;
                                // (i)/(r) return 1-based index of match,
                                // arr.len()+1 (or 1 for I/R) on miss
                                // per zsh docs. We mirror the read-form
                                // semantics from subst.rs.
                                let idx_1based = match found {
                                    Some(i) => (i + 1) as i64,
                                    None => (arr.len() + 1) as i64,
                                };
                                return idx_1based.to_string();
                            }
                            // Scalar LHS — resolve to a CHAR position
                            // (1-based first/last match of pat in the
                            // string). c:Src/params.c:1411-1418 — the
                            // scalar path returns the char index from
                            // sliding-window pattern match against
                            // pm.u_str. Same algorithm as the read-form
                            // at subst.rs:5283-5306. Bug (scalar
                            // sibling of #293): `a=hello; a[(I)l]=X`
                            // previously auto-vivified `a` into
                            // PM_HASHED with key "(I)l" instead of
                            // splicing X at the last 'l' position
                            // (yielding "helXo").
                            if is_scalar {
                                let s = exec.scalar(&name).unwrap_or_default();
                                let s_chars: Vec<char> = s.chars().collect();
                                let n = s_chars.len();
                                let want_last = flags.contains('I') || flags.contains('R');
                                let exact = flags.contains('e');
                                let mut found: Option<usize> = None;
                                'outer: for start in 0..=n {
                                    let lengths: Box<dyn Iterator<Item = usize>> = if want_last {
                                        Box::new((1..=(n - start)).rev())
                                    } else {
                                        Box::new(1..=(n - start))
                                    };
                                    for len in lengths {
                                        let cand: String =
                                            s_chars[start..start + len].iter().collect();
                                        let matched = if exact {
                                            cand == pat
                                        } else {
                                            crate::ported::pattern::patcompile(
                                                pat,
                                                crate::ported::zsh_h::PAT_HEAPDUP as i32,
                                                None,
                                            )
                                            .map_or(false, |p| {
                                                crate::ported::pattern::pattry(&p, &cand)
                                            })
                                        };
                                        if matched {
                                            found = Some(start);
                                            if !want_last {
                                                break 'outer;
                                            }
                                            break;
                                        }
                                    }
                                }
                                // (I)/(R): scan again to find LAST.
                                if want_last {
                                    let mut last_found: Option<usize> = found;
                                    for start in (0..=n).rev() {
                                        for len in 1..=(n - start) {
                                            let cand: String =
                                                s_chars[start..start + len].iter().collect();
                                            let matched = if exact {
                                                cand == pat
                                            } else {
                                                crate::ported::pattern::patcompile(
                                                    pat,
                                                    crate::ported::zsh_h::PAT_HEAPDUP as i32,
                                                    None,
                                                )
                                                .map_or(false, |p| {
                                                    crate::ported::pattern::pattry(&p, &cand)
                                                })
                                            };
                                            if matched {
                                                last_found = Some(start);
                                                break;
                                            }
                                        }
                                        if last_found.is_some()
                                            && last_found.unwrap() >= start
                                        {
                                            break;
                                        }
                                    }
                                    found = last_found;
                                }
                                let idx_1based = match found {
                                    Some(i) => (i + 1) as i64,
                                    // (i) miss → len+1 (one past end).
                                    None => (n + 1) as i64,
                                };
                                return idx_1based.to_string();
                            }
                        }
                    }
                }
            }
            if is_indexed && key.trim().parse::<i64>().is_err() {
                crate::ported::math::mathevali(&crate::ported::subst::singsub(&key))
                    .map(|n| n.to_string())
                    .unwrap_or(key.clone())
            } else {
                key.clone()
            }
        });
        let subscripted = format!("{}[{}]", name, resolved_key);
        crate::ported::params::assignsparam(&subscripted, &value, crate::ported::zsh_h::ASSPM_WARN);
        Value::Status(0)
    });

    // Brace expansion. Routes through executor.xpandbraces (already
    // implemented for the pre-fusevm executor). Returns Value::Array.
    // BUILTIN_ARRAY_DROP_EMPTY — filter out empty Value::Str entries
    // from a Value::Array on the stack. Used by `for x in $@` /
    // `for x in $*` unquoted forms which drop empty positionals
    // (POSIX-like) but do NOT IFS-split each element internally
    // (zsh-specific — scalar word splitting is off by default).
    // Distinct from BUILTIN_WORD_SPLIT which routes through
    // multsub PREFORK_SPLIT (full IFS-split). Bug #166.
    vm.register_builtin(BUILTIN_ARRAY_DROP_EMPTY, |vm, _argc| {
        let v = vm.pop();
        match v {
            Value::Array(items) => {
                let filtered: Vec<Value> = items
                    .into_iter()
                    .filter(|x| !x.to_str().is_empty())
                    .collect();
                Value::Array(filtered)
            }
            Value::Str(s) if s.is_empty() => Value::Array(Vec::new()),
            other => other,
        }
    });

    // BUILTIN_QUOTEDZPUTS — re-wrap top-of-stack scalar via the
    // canonical quotedzputs (Src/utils.c:6464). Non-printable bytes
    // come back as `$'…'` C-string form so the cond xtrace prefix
    // line preserves the source-quoting form for `[[ -n $'\C-[OP' ]]`
    // instead of leaking raw ESC + "OP" bytes through the terminal.
    vm.register_builtin(BUILTIN_QUOTEDZPUTS, |vm, _argc| {
        let s = vm.pop().to_str();
        Value::str(crate::ported::utils::quotedzputs(&s))
    });

    // BUILTIN_QUOTE_TOKENIZED_OUTPUT — char-aware mirror of
    // c:Src/exec.c:2114 `quote_tokenized_output`. The canonical
    // port at exec::quote_tokenized_output operates on bytes
    // (zsh's metafied encoding); zshrs strings are UTF-8 so
    // `\u{87}` Star is `[0xC2, 0x87]`, and a byte walk writes
    // 0xC2 raw (invalid UTF-8 lead → U+FFFD on lossy decode).
    // Walk by char and dispatch the same switch the byte port
    // uses, but with the token chars matching the UTF-8 form.
    vm.register_builtin(BUILTIN_QUOTE_TOKENIZED_OUTPUT, |vm, _argc| {
        let s = vm.pop().to_str();
        let mut out = String::with_capacity(s.len());
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            // c:2120 — Meta-quoted byte: emit `*++s ^ 32`.
            // In UTF-8 strings Meta is `\u{83}`; the next char is
            // the metafied payload.
            if c == '\u{83}' {
                if let Some(&n) = chars.get(i + 1) {
                    if (n as u32) < 0x80 {
                        out.push(((n as u8) ^ 32) as char);
                    } else {
                        out.push(n);
                    }
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }
            // c:2124 — Nularg: skip.
            if c == '\u{a1}' {
                i += 1;
                continue;
            }
            // c:2128-2143 — ASCII specials get backslash-prefixed
            // then fall through to emit the literal char.
            match c {
                '\\' | '<' | '>' | '(' | '|' | ')' | '^' | '#' | '~'
                | '[' | ']' | '*' | '?' | '$' | ' ' => {
                    out.push('\\');
                    out.push(c);
                    i += 1;
                    continue;
                }
                '\t' => {
                    out.push_str("$'\\t'");
                    i += 1;
                    continue;
                }
                '\n' => {
                    out.push_str("$'\\n'");
                    i += 1;
                    continue;
                }
                '\r' => {
                    out.push_str("$'\\r'");
                    i += 1;
                    continue;
                }
                '=' => {
                    if i == 0 {
                        out.push('\\');
                    }
                    out.push(c);
                    i += 1;
                    continue;
                }
                _ => {}
            }
            // c:2163 — `if (itok(*s)) putc(ztokens[*s - Pound]);`
            // Map zsh token chars (`\u{84}`..`\u{a1}` range, the
            // ones the lexer emits for `#$^*()…`) back to their
            // source ASCII via the `ztokens` table.
            let cp = c as u32;
            if (0x84..=0xa1).contains(&cp) {
                let idx = (cp - 0x84) as usize;
                let ztokens = crate::ported::lex::ztokens.as_bytes();
                if idx < ztokens.len() {
                    out.push(ztokens[idx] as char);
                    i += 1;
                    continue;
                }
            }
            out.push(c);
            i += 1;
        }
        Value::str(out)
    });

    // BUILTIN_WORD_SPLIT — `${=var}` IFS-split runtime.
    // PURE PASSTHRU: route through canonical `subst::multsub` with
    // PREFORK_SPLIT flag (C port of `Src/subst.c::multsub` at c:544
    // — the IFS-split walker with whitespace-vs-non-whitespace
    // gating, quote-aware parsing, and empty-field handling).
    vm.register_builtin(BUILTIN_WORD_SPLIT, |vm, _argc| {
        let s = vm.pop().to_str();
        let (_joined, parts, _isarr, _flags) =
            crate::ported::subst::multsub(&s, crate::ported::zsh_h::PREFORK_SPLIT);
        // Empty single-string special case → empty Array (drop empty arg).
        if parts.len() == 1 && parts[0].is_empty() {
            return Value::Array(Vec::new());
        }
        nodes_to_value(parts)
    });

    vm.register_builtin(BUILTIN_BRACE_EXPAND, |vm, _argc| {
        // c:Src/glob.c::xpandbraces — brace expansion runs per word.
        // When the upstream produced an array (e.g. `${a:e}` splat),
        // expand braces on each element separately so the splat
        // survives. `pop().to_str()` would join with space and lose
        // the array shape. Parity bug #28 cousin: the BRACE_EXPAND
        // emit always fires for any word containing `{` (including
        // `${...}` param-expansion braces), so its collapse hit even
        // pure-paramsubst args.
        let raw = vm.pop();
        // c:Src/options.c — `no_brace_expand` (negated braceexpand)
        // disables brace expansion entirely. When set, `{a,b}` stays
        // literal. Mirror by short-circuiting xpandbraces; pass the
        // input through unchanged.
        let brace_expand = opt_state_get("braceexpand").unwrap_or(true);
        let brace_ccl = opt_state_get("braceccl").unwrap_or(false);
        let inputs: Vec<String> = match raw {
            Value::Array(items) => items.into_iter().map(|v| v.to_str()).collect(),
            other => vec![other.to_str()],
        };
        if !brace_expand {
            return nodes_to_value(inputs);
        }
        let mut out: Vec<String> = Vec::with_capacity(inputs.len());
        for s in inputs {
            for w in crate::ported::glob::xpandbraces(&s, brace_ccl) {
                out.push(w);
            }
        }
        nodes_to_value(out)
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
        // c:Src/glob.c:1872 — `zglob` runs per-word in the argv
        // pipeline. When the upstream EXPAND_TEXT returned an array
        // (e.g. `${a:e}` splat → ["txt","md"]), we must glob each
        // element separately, not collapse to a sepjoin'd scalar.
        // Without this, `print -l ${a:e}` saw the array stringified
        // by `pop().to_str()` and emitted one joined arg.
        let raw = vm.pop();
        let patterns: Vec<String> = match raw {
            Value::Array(items) => items.into_iter().map(|v| v.to_str()).collect(),
            other => vec![other.to_str()],
        };
        // c:Src/glob.c:1872 — honour `setopt noglob` / `noglob CMD`
        // precommand. When the option is on, the word stays literal
        // (zsh skips the glob expansion entirely). Without this, the
        // segment-fast-path BUILTIN_GLOB_EXPAND fired even after
        // `noglob` set the option, so `noglob echo *.xyz` saw the
        // NOMATCH error instead of the literal pass-through.
        let noglob =
            opt_state_get("noglob").unwrap_or(false) || !opt_state_get("glob").unwrap_or(true);
        if noglob {
            return if patterns.is_empty() {
                Value::Array(Vec::new())
            } else if patterns.len() == 1 {
                Value::str(patterns.into_iter().next().unwrap())
            } else {
                Value::Array(patterns.into_iter().map(Value::str).collect())
            };
        }
        let mut out: Vec<String> = Vec::with_capacity(patterns.len());
        for pattern in &patterns {
            let matches = with_executor(|exec| exec.expand_glob(pattern));
            if matches.is_empty() {
                // c:1872 nullglob — drop this word, don't emit a hole
                continue;
            }
            for m in matches {
                out.push(m);
            }
        }
        if out.is_empty() {
            return Value::Array(Vec::new());
        }
        if patterns.len() == 1 && out.len() == 1 && out[0] == patterns[0] {
            // No real matches; expand_glob returned the literal. Pass
            // back as scalar so downstream ops don't re-flatten.
            return Value::str(out.into_iter().next().unwrap());
        }
        Value::Array(out.into_iter().map(Value::str).collect())
    });

    // `break`/`continue` from a sub-VM body. The compile path emits
    // these when the keyword appears at chunk top-level (no enclosing
    // for/while in the current chunk's patch lists). Outer-loop
    // builtins (BUILTIN_RUN_SELECT and any future loop-via-builtin
    // construct) drain canonical BREAKS/CONTFLAG after each iteration.
    //
    // Writes match `bin_break`'s c:5836+ pattern:
    //   continue: contflag = 1; breaks++   (Src/builtin.c::bin_break)
    //   break:    breaks++
    vm.register_builtin(BUILTIN_SET_BREAK, |_vm, _argc| {
        use std::sync::atomic::Ordering::SeqCst;
        crate::ported::builtin::BREAKS.fetch_add(1, SeqCst);
        Value::Status(0)
    });
    vm.register_builtin(BUILTIN_SET_CONTINUE, |_vm, _argc| {
        use std::sync::atomic::Ordering::SeqCst;
        crate::ported::builtin::CONTFLAG.store(1, SeqCst);
        crate::ported::builtin::BREAKS.fetch_add(1, SeqCst);
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
        // Pure passthru: canonical port lives in
        // src/ported/options.rs::opt_state_set_via_alias and
        // handles negation-alias resolution per c:Src/options.c.
        crate::ported::options::opt_state_set_via_alias(&opt, on);
        Value::Status(0)
    });

    // c:Src/options.c GLOB_SUBST — runtime glob expansion of
    // substituted words. Pop a Value (Str or Array); when
    // GLOB_SUBST is ON, run expand_glob on each string element;
    // when OFF, pass through unchanged. Bug #119 in docs/BUGS.md.
    vm.register_builtin(BUILTIN_GLOB_SUBST_EXPAND, |vm, _argc| {
        let raw = vm.pop();
        let glob_subst =
            crate::ported::zsh_h::isset(crate::ported::zsh_h::GLOBSUBST);
        if !glob_subst {
            return raw;
        }
        // Collect input strings (Str → vec![s]; Array → multiple).
        let inputs: Vec<String> = match raw {
            Value::Array(items) => items.into_iter().map(|v| v.to_str()).collect(),
            other => vec![other.to_str()],
        };
        // Run expand_glob on each. Empty matches collapse to a
        // single literal pass-through to mirror nullglob-off default.
        let mut out: Vec<String> = Vec::with_capacity(inputs.len());
        for pattern in inputs {
            let matches = with_executor(|exec| exec.expand_glob(&pattern));
            if matches.is_empty() {
                // No match: keep the literal (like nullglob off).
                out.push(pattern);
            } else {
                for m in matches {
                    out.push(m);
                }
            }
        }
        if out.len() == 1 {
            Value::str(out.into_iter().next().unwrap())
        } else {
            Value::Array(out.into_iter().map(Value::str).collect())
        }
    });

    // c:Src/math.c:337 — `getmathparam` for ArithCompiler pre-load.
    // Pop a variable name, return its math-coerced value. Mirrors
    // the routing in math::getmathparam: try i64, then f64, then
    // recursive arith-eval, else 0. Bug #118 in docs/BUGS.md.
    vm.register_builtin(BUILTIN_GET_MATH_VAR, |vm, _argc| {
        let name = vm.pop().to_str();
        let raw = crate::ported::params::getsparam(&name).unwrap_or_default();
        // Empty / unset → 0.
        if raw.is_empty() {
            return Value::Int(0);
        }
        // Direct int / float parse.
        if let Ok(n) = raw.parse::<i64>() {
            return Value::Int(n);
        }
        if let Ok(f) = raw.parse::<f64>() {
            return Value::Float(f);
        }
        // Recursive arith eval (matches getmathparam fallback at
        // Src/math.c:337). If that fails too, return 0 — C's
        // mathevall returns 0 with errflag set on parse failure.
        match crate::ported::math::mathevali(&raw) {
            Ok(n) => Value::Int(n),
            Err(_) => Value::Int(0),
        }
    });

    // c:Src/options.c GLOB_SUBST + Src/cond.c:552 cond_match.
    // Pop pattern string; when GLOB_SUBST is OFF, escape every glob
    // metachar with `\` so the downstream StrMatch + patcompile
    // treat them as literals (matching C's tokenization-based
    // gate). When GLOB_SUBST is ON, pass through unchanged.
    // See BUILTIN_GLOB_SUBST_GUARD docs above for full rationale.
    vm.register_builtin(BUILTIN_GLOB_SUBST_GUARD, |vm, _argc| {
        let p = vm.pop().to_str();
        let glob_subst =
            crate::ported::zsh_h::isset(crate::ported::zsh_h::GLOBSUBST);
        if glob_subst {
            return Value::str(p);
        }
        let mut out = String::with_capacity(p.len() * 2);
        for c in p.chars() {
            match c {
                '*' | '?' | '[' | ']' | '(' | ')' | '|' | '<' | '>' | '#' | '^'
                | '~' | '\\' => {
                    out.push('\\');
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        Value::str(out)
    });

    vm.register_builtin(BUILTIN_ARRAY_JOIN_STAR, |vm, _argc| {
        let name = vm.pop().to_str();
        let (joined, ifs_full, in_dq) = with_executor(|exec| {
            // c:Src/params.c — `"$*"` joins by IFS[0]. zsh
            // distinguishes IFS=unset (→ default `" "`) from
            // IFS="" (→ EMPTY separator → fields concatenate).
            // chars().next() collapsed both into the default, so
            // IFS="" was treated as IFS=" ".
            let ifs_full = exec
                .scalar("IFS")
                .unwrap_or_else(|| " \t\n".to_string());
            let sep = ifs_full
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_default();
            let in_dq = exec.in_dq_context > 0;
            let joined = if name == "@" || name == "*" || name == "argv" {
                exec.pparams().join(&sep)
            } else if let Some(assoc_map) = exec.assoc(&name) {
                // c:Src/params.c — assoc-splat values for
                // `"${h[@]}"` / `"${h[*]}"`. Bug #109 in
                // docs/BUGS.md.
                assoc_map.values().cloned().collect::<Vec<_>>().join(&sep)
            } else if let Some(arr) = exec.array(&name) {
                arr.join(&sep)
            } else {
                exec.get_variable(&name)
            };
            (joined, ifs_full, in_dq)
        });
        // c:Src/subst.c — UNQUOTED `${name[*]}` (or `$*`) goes
        // through the canonical "join via IFS[0], then word-split
        // via IFS" pipeline. The fast-path bypassed paramsubst
        // entirely so it never word-split, producing one joined
        // string instead of N argv entries. Bug #428.
        //
        // In QUOTED (`"${name[*]}"`) context, the result IS a
        // single scalar — return it as Str without splitting.
        if in_dq {
            return Value::str(joined);
        }
        if joined.is_empty() {
            return Value::Array(Vec::new());
        }
        // IFS word-split — every IFS char is a separator. Empty
        // resulting fields are dropped (the canonical
        // "remove empty unquoted words" pass from
        // Src/subst.c::prefork c:184-187).
        let parts: Vec<String> = joined
            .split(|c: char| ifs_full.contains(c))
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        if parts.is_empty() {
            Value::Array(Vec::new())
        } else if parts.len() == 1 {
            Value::str(parts.into_iter().next().unwrap())
        } else {
            Value::Array(parts.into_iter().map(Value::str).collect())
        }
    });

    vm.register_builtin(BUILTIN_ARRAY_ALL, |vm, _argc| {
        let name = vm.pop().to_str();
        with_executor(|exec| {
            // Special positional names — splice the positional list.
            if name == "@" || name == "*" || name == "argv" {
                return Value::Array(exec.pparams().iter().map(Value::str).collect());
            }
            // c:Src/Modules/parameter.c — funcstack/funcfiletrace/
            // funcsourcetrace/functrace are PM_ARRAY|PM_READONLY
            // specials backed by the canonical FUNCSTACK Vec.
            // `${funcstack[@]}` inside a function call should splat
            // the innermost-first names; without this branch the
            // runtime fell to the scalar fallback (get_variable
            // returns empty for these specials) and `[@]` came out
            // empty. Bug #276 in docs/BUGS.md. Mirrors the parallel
            // arrays_get handler at src/ported/subst.rs ~10685.
            // c:Src/Modules/datetime.c:256 — `epochtime` PM_ARRAY|
            // PM_READONLY backed by getcurrenttime(). Same parallel
            // arrangement as the FUNCSTACK-backed specials below.
            if name == "epochtime" {
                let arr = crate::ported::modules::datetime::getcurrenttime();
                return Value::Array(arr.into_iter().map(Value::str).collect());
            }
            if matches!(
                name.as_str(),
                "funcstack" | "funcfiletrace" | "funcsourcetrace" | "functrace"
            ) {
                if let Ok(f) = crate::ported::modules::parameter::FUNCSTACK.lock() {
                    let vals: Vec<Value> = f
                        .iter()
                        .rev()
                        .map(|fs| {
                            let s = match name.as_str() {
                                "funcstack" => fs.name.clone(),
                                "funcfiletrace" => fs.filename.clone().unwrap_or_default(),
                                // funcsourcetrace / functrace
                                _ => format!("{}:{}", fs.name, fs.lineno),
                            };
                            Value::str(s)
                        })
                        .collect();
                    return Value::Array(vals);
                }
            }
            // c:Src/params.c — `${assoc[@]}` enumerates VALUES (per
            // params.c:1696-1750 hashparam splat). Check assoc
            // storage BEFORE the scalar fallback so an associative
            // array named X resolves `${X[@]}` to the values, not
            // empty. Bug #109 in docs/BUGS.md: `${h[@]}` on an
            // assoc routed through BUILTIN_ARRAY_ALL, which only
            // consulted `exec.array(name)` (the indexed-array map)
            // — that lookup missed for assocs, fell through to
            // `get_variable("h")` (also empty for an assoc-only
            // name), and returned `Array(vec![])`. zsh's expected
            // behavior is to enumerate values.
            if let Some(assoc_map) = exec.assoc(&name) {
                return Value::Array(
                    assoc_map.values().cloned().map(Value::str).collect(),
                );
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
                    if val.is_empty() && !exec.has_scalar(&name) && env::var(&name).is_err() {
                        Value::Array(vec![])
                    } else if opt_state_get("shwordsplit").unwrap_or(false) {
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
                // c:Src/exec.c — `coprocin`/`coprocout` are the
                // canonical globals that bin_read's `-p` arm
                // (Src/builtin.c:6510) and bin_print's `-p` arm
                // (Src/builtin.c:4827) read to find the
                // coprocess fds. The Rust port has the atomic
                // declarations at src/ported/modules/clone.rs:262
                // but the coproc-launch path never updated them,
                // so `read -p` / `print -p` always errored with
                // "-p: no coprocess" even when a coproc was
                // running. Bug #388 in docs/BUGS.md. Update them
                // here so the canonical builtins find the live
                // pipe.
                crate::ported::modules::clone::coprocin
                    .store(read_fd, std::sync::atomic::Ordering::Relaxed);
                crate::ported::modules::clone::coprocout
                    .store(write_fd, std::sync::atomic::Ordering::Relaxed);
                Value::Status(0)
            }
        }
    });

    vm.register_builtin(BUILTIN_ARRAY_FLATTEN, |vm, argc| {
        let n = argc as usize;
        let start = vm.stack.len().saturating_sub(n);
        let raw: Vec<Value> = vm.stack.drain(start..).collect();
        let mut flat: Vec<Value> = Vec::with_capacity(raw.len());
        for v in raw {
            match v {
                Value::Array(items) => flat.extend(items),
                other => flat.push(other),
            }
        }
        let len = flat.len() as i64;
        // Push the array first; the Int(len) becomes the builtin's return
        // value (which CallBuiltin already pushes). Caller consumes in
        // reverse: SetSlot(len_slot) pops Int, SetSlot(arr_slot) pops Array.
        vm.push(Value::Array(flat));
        Value::Int(len)
    });

    // Shell variable get/set — routes through executor.variables so nested
    // VMs (function calls) and tree-walker callers see the same storage.
    vm.register_builtin(BUILTIN_GET_VAR, |vm, argc| {
        let args = pop_args(vm, argc);
        let name = args.into_iter().next().unwrap_or_default();
        let live_status = vm.last_status;
        // Suppress sync when a deferred subshell exit just landed:
        // LASTVAL holds the correct deferred status, vm.last_status
        // is stale (post-subshell vm doesn't propagate status). See
        // SUBSHELL_EXIT_STATUS_PENDING TLS declaration for rationale.
        let suppress_sync = SUBSHELL_EXIT_STATUS_PENDING.with(|c| {
            let prev = c.get();
            c.set(false);
            prev
        });
        // `$@` and `$*` need splice semantics — return Value::Array of
        // positional params so for-loop's BUILTIN_ARRAY_FLATTEN spreads them
        // and pop_args splits them into argv slots. zsh's `"$@"` bslashquote-each-
        // word semantics matches: each pos-param becomes its own arg.
        // Same for arrays accessed by name (e.g. `$arr` in some contexts).
        let sync_status = |exec: &mut ShellExecutor| {
            if !suppress_sync {
                exec.set_last_status(live_status);
            }
        };
        if name == "@" || name == "*" {
            return with_executor(|exec| {
                sync_status(exec);
                Value::Array(exec.pparams().iter().map(Value::str).collect())
            });
        }
        // RC_EXPAND_PARAM: when the option is set and `name` refers to
        // an array, return Value::Array so the enclosing word's
        // BUILTIN_CONCAT_DISTRIBUTE distributes element-wise. Without
        // the option, arrays still join to a space-separated scalar
        // (zsh's default unquoted-array-as-scalar semantics).
        let rc_expand = with_executor(|exec| opt_state_get("rcexpandparam").unwrap_or(false));
        if rc_expand {
            let arr_val = with_executor(|exec| {
                sync_status(exec);
                exec.array(&name)
            });
            if let Some(arr) = arr_val {
                return Value::Array(arr.into_iter().map(Value::str).collect());
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
            // Canonical PARTAB dispatch (Src/Modules/parameter.c:2235-
            // 2298 + SPECIALPMDEFs in mapfile/terminfo/termcap/system/
            // zleparameter): PARTAB_ARRAY entries → whole-array getfn;
            // PARTAB entries → scan keys + per-key getpm/scanpm fn
            // pointers.
            let _ = exec;
            if let Some(values) = partab_array_get(&name) {
                Some(values)
            } else if let Some(keys) = partab_scan_keys(&name) {
                Some(
                    keys.iter()
                        .map(|k| partab_get(&name, k).unwrap_or_default())
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            }
        });
        if let Some(vals) = magic_vals {
            // Distinguish "name IS a magic-assoc with no entries"
            // (return Array(empty)) from "name is unknown — fall
            // through to get_variable".
            return Value::Array(vals.into_iter().map(Value::str).collect());
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
            let ksh_arrays = opt_state_get("ksharrays").unwrap_or(false);
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
            return Value::Array(items.into_iter().map(Value::str).collect());
        }
        let (val, in_dq, is_known) = with_executor(|exec| {
            sync_status(exec);
            let v = exec.get_variable(&name);
            // For nounset detection: a name is "known" when it has a
            // paramtab/array/assoc/env entry. Special chars ($?, $#,
            // $@, $*, $-, $$, $!, $_, $0) always count as known
            // regardless of value. Pure-digit positional params
            // count as known iff index <= $# (set -- has populated
            // that slot). c:Src/subst.c:1689 — NOUNSET fires on
            // unset positional param too: `set --; echo "$1"` with
            // nounset must diagnose.
            let is_special_single = name.len() == 1
                && matches!(
                    name.chars().next().unwrap(),
                    '?' | '#' | '@' | '*' | '-' | '$' | '!' | '_' | '0'
                );
            let is_pure_digit = !name.is_empty() && name.chars().all(|c| c.is_ascii_digit());
            let positional_known = if is_pure_digit {
                let idx: usize = name.parse().unwrap_or(0);
                if idx == 0 {
                    true // $0 always set
                } else {
                    idx <= exec.pparams().len()
                }
            } else {
                false
            };
            let known = !v.is_empty()
                || name.is_empty()
                || is_special_single
                || positional_known
                || crate::ported::params::paramtab()
                    .read()
                    .ok()
                    .map(|t| t.contains_key(&name))
                    .unwrap_or(false)
                || env::var(&name).is_ok();
            (v, exec.in_dq_context > 0, known)
        });
        // c:Src/subst.c:1689 — NO_UNSET / nounset: reading an unset
        // parameter fires "parameter not set" diagnostic and aborts
        // the substitution. Direct port of the noerrs gate at c:1689
        // (zerr + errflag). Matches `set -u` POSIX semantics.
        if !is_known && opt_state_get("nounset").unwrap_or(false) {
            crate::ported::utils::zerr(&format!("{}: parameter not set", name));
            crate::ported::utils::errflag.fetch_or(
                crate::ported::zsh_h::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            with_executor(|exec| exec.set_last_status(1));
            return Value::str("");
        }
        // Empty unquoted scalar → drop the arg (zsh "remove empty
        // unquoted words" rule). Returning empty Value::Array makes
        // pop_args contribute zero items. DQ context keeps the empty
        // string so "$a" stays a single empty arg. Direct port of
        // subst.c's elide-empty pass.
        if val.is_empty() && !in_dq {
            return Value::Array(Vec::new());
        }
        // c:Src/subst.c:1759 SH_WORD_SPLIT — when shwordsplit is set and
        // we're in unquoted command-arg position (not DQ), split scalar
        // value on IFS into multiple words. Matches BUILTIN_ARRAY_ALL's
        // shwordsplit arm (fusevm_bridge.rs:2200). Without this, bare
        // `$s` in `print $s` stayed a single arg even with the option
        // set, breaking POSIX-style scalar word-splitting.
        if !in_dq && opt_state_get("shwordsplit").unwrap_or(false) {
            let ifs =
                with_executor(|exec| exec.scalar("IFS").unwrap_or_else(|| " \t\n".to_string()));
            let parts: Vec<Value> = val
                .split(|c: char| ifs.contains(c))
                .filter(|s| !s.is_empty())
                .map(|s| Value::str(s.to_string()))
                .collect();
            if parts.is_empty() {
                return Value::Array(Vec::new());
            } else if parts.len() == 1 {
                return parts.into_iter().next().unwrap();
            } else {
                return Value::Array(parts);
            }
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
            // Array form: `arr+=elem` pushes a single element.
            // Routes through canonical assignaparam(name, [value],
            // ASSPM_AUGMENT) — Src/params.c:3357 c:3402-3412 augment
            // path prepends prior scalar / appends to existing array.
            if exec.array(&name).is_some() {
                let _ = crate::ported::params::assignaparam(
                    &name,
                    vec![value.clone()],
                    crate::ported::zsh_h::ASSPM_AUGMENT,
                );
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
            // Scalar / integer / float form: route through canonical
            // assignsparam(name, value, ASSPM_AUGMENT) which
            // dispatches PM_TYPE — PM_SCALAR concats, PM_INTEGER
            // arith-adds (c:2775-2778), PM_FLOAT float-adds.
            let _ = crate::ported::params::assignsparam(
                &name,
                &value,
                crate::ported::zsh_h::ASSPM_AUGMENT,
            );
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                let ctx = exec.recorder_ctx();
                let attrs = exec.recorder_attrs_for(&name);
                // Re-read the canonical value via get_variable for the
                // recorder bundle (assignsparam may have transformed it
                // through integer/float arithmetic).
                let final_val = exec.get_variable(&name);
                let lower = name.to_ascii_lowercase();
                if matches!(
                    lower.as_str(),
                    "path" | "fpath" | "manpath" | "module_path" | "cdpath"
                ) {
                    emit_path_or_assign(&name, std::slice::from_ref(&final_val), attrs, true, &ctx);
                } else {
                    crate::recorder::emit_assign_typed(&name, &final_val, attrs, ctx);
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
        // Snapshot the raw Values BEFORE pop_args's to_str
        // flattening — needed to distinguish Int (arith assignment,
        // integer-typed param) from Str (scalar assignment).
        let mut raw_values: Vec<fusevm::Value> = Vec::with_capacity(argc as usize);
        for _ in 0..argc {
            raw_values.push(vm.pop());
        }
        raw_values.reverse();
        let name = raw_values.first().map(|v| v.to_str()).unwrap_or_default();
        let value_raw = raw_values.get(1).cloned();
        let value = value_raw.as_ref().map(|v| v.to_str()).unwrap_or_default();
        // c:Src/params.c — when the bytecode hands us an Int value
        // (only the arith assignment paths emit this — `(( X = N ))`
        // is the canonical site), route through setiparam so the
        // param ends up PM_INTEGER + inherits the math layer's
        // `lastbase` for display formatting (`(( X = 16#ff ));
        // echo \$X` → `16#FF`). Scalar `X=val` and `$((expr))`
        // assignments still take the setsparam path below.
        let int_assign = matches!(value_raw, Some(fusevm::Value::Int(_)));
        let float_assign = matches!(value_raw, Some(fusevm::Value::Float(_)));
        with_executor(|exec| {
            // c:Src/params.c assignsparam — PM_READONLY rejection
            // BEFORE any env mutation. The inline-env-prefix path
            // (`X=2 env`) called env::set_var unconditionally before
            // the readonly check fired in setsparam, so the OS env
            // got X=2 even though the assignment errored. env then
            // inherited the polluted env from fork, leaking the
            // attempted override past the readonly guard. Mirror
            // C's order: readonly check → zerr → bail; only mutate
            // env when the assignment is admissible. Bug #551
            // (security-relevant).
            if exec.is_readonly_param(&name) {
                crate::ported::utils::zerr(&format!("read-only variable: {}", name));
                return;
            }
            // Inline-assignment frame tracking (`X=foo cmd` reverts on
            // command return).
            if !exec.inline_env_stack.is_empty() {
                let prev_var = crate::ported::params::getsparam(&name);
                let prev_env = env::var(&name).ok();
                exec.inline_env_stack
                    .last_mut()
                    .unwrap()
                    .push((name.clone(), prev_var, prev_env));
                env::set_var(&name, &value);
            }
            // Canonical setsparam handles readonly, integer math, case
            // fold, GSU dispatch. For Int values (arith assigns) route
            // through setiparam so the param is PM_INTEGER + inherits
            // the math layer's lastbase for display formatting. For
            // Float (arith assigns producing MN_FLOAT) route through
            // setnparam so the param is PM_FFLOAT — `(( b = a * 2 ))`
            // with scalar `a="3.14"` should create b as typeset -F,
            // not a scalar holding "6.28".
            if int_assign {
                if let Some(fusevm::Value::Int(i)) = value_raw {
                    crate::ported::params::setiparam(&name, i);
                } else {
                    crate::ported::params::setsparam(&name, &value);
                }
            } else if float_assign {
                if let Some(fusevm::Value::Float(f)) = value_raw {
                    // ArithCompiler returns Value::Float whenever any
                    // operand came through Str (BUILTIN_GET_VAR yields
                    // Value::Str even for integer-shaped scalars). To
                    // avoid forcing every `(( b = a + 3 ))` to PM_FFLOAT
                    // when `a="5"` (integer-shaped), detect integer-
                    // valued floats and route through setiparam instead.
                    // True floats (non-integral) reach setnparam →
                    // PM_FFLOAT so `typeset -p b` shows `typeset -F …`.
                    if f.fract() == 0.0 && f.is_finite() && f.abs() <= i64::MAX as f64 {
                        crate::ported::params::setiparam(&name, f as i64);
                    } else {
                        let mnval = crate::ported::math::mnumber {
                            l: 0,
                            d: f,
                            type_: crate::ported::math::MN_FLOAT,
                        };
                        crate::ported::params::setnparam(&name, mnval);
                    }
                } else {
                    crate::ported::params::setsparam(&name, &value);
                }
            } else {
                crate::ported::params::setsparam(&name, &value);
            }
            // PM_EXPORTED / allexport env mirror — read AFTER setsparam
            // so the flag bit reflects any GSU setfn side-effects.
            let allexport = opt_state_get("allexport").unwrap_or(false);
            let already_exported =
                (exec.param_flags(&name) as u32 & crate::ported::zsh_h::PM_EXPORTED) != 0;
            if allexport || already_exported {
                env::set_var(&name, &value);
            }
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled()
                && exec.local_scope_depth == 0
                && !matches!(
                    name.as_str(),
                    "PPID" | "LINENO" | "ZSH_ARGZERO" | "argv0" | "ARGC" | "?" | "_" | "RANDOM"
                )
            {
                let ctx = exec.recorder_ctx();
                let attrs = exec.recorder_attrs_for(&name);
                crate::recorder::emit_assign_typed(&name, &value, attrs, ctx);
            }
        });
        Value::Status(vm.last_status)
    });

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
        Value::Bool(result == "1")
    });

    // `time { compound; ... }` — runs the sub-chunk and prints elapsed
    // wall-clock time. zsh's full `time` also tracks user/system CPU via
    // getrusage on the *child*; we approximate via wall-time only since
    // the sub-chunk runs in-process (no fork). Output format matches
    // `time simple-cmd` (already implemented elsewhere via exectime).
    vm.register_builtin(BUILTIN_TIME_SUBLIST, |vm, argc| {
        let sub_idx = vm.pop().to_int() as usize;
        // c:Src/jobs.c:1028-1029 — `pn->text` arg to printtime. argc==2
        // means the compiler also pushed a desc string (bug #66 fix);
        // older callers with argc==1 push only sub_idx and we synthesize
        // an empty desc for backward compat with cached bytecode that
        // predates the desc-threading patch.
        let desc = if argc >= 2 {
            vm.pop().to_str().to_string()
        } else {
            String::new()
        };
        let chunk_opt = vm.chunk.sub_chunks.get(sub_idx).cloned();
        let Some(chunk) = chunk_opt else {
            return Value::Status(0);
        };
        // c:Src/jobs.c:1968 — `getrusage(RUSAGE_CHILDREN, &ti)` before
        // and after the timed sublist gives accurate per-stage user/sys
        // CPU. Wall-time-only approximation (0.7×/0.1× fudge factors)
        // produced bogus user/sys columns and ignored TIMEFMT. Bug #66
        // in docs/BUGS.md.
        let ru_before: libc::rusage = unsafe {
            let mut r: libc::rusage = std::mem::zeroed();
            libc::getrusage(libc::RUSAGE_CHILDREN, &mut r);
            r
        };
        let start = Instant::now();
        crate::fusevm_disasm::maybe_print_stdout("time_sublist", &chunk);
        let mut sub_vm = fusevm::VM::new(chunk);
        register_builtins(&mut sub_vm);
        let _ = sub_vm.run();
        let status = sub_vm.last_status;
        let elapsed = start.elapsed();
        let ru_after: libc::rusage = unsafe {
            let mut r: libc::rusage = std::mem::zeroed();
            libc::getrusage(libc::RUSAGE_CHILDREN, &mut r);
            r
        };
        // Delta children rusage = timed work's CPU.
        let mut delta = ru_after;
        let sub = |a: libc::timeval, b: libc::timeval| -> libc::timeval {
            let mut sec = a.tv_sec - b.tv_sec;
            let mut usec = a.tv_usec as i64 - b.tv_usec as i64;
            if usec < 0 {
                sec -= 1;
                usec += 1_000_000;
            }
            libc::timeval {
                tv_sec: sec,
                tv_usec: usec as libc::suseconds_t,
            }
        };
        delta.ru_utime = sub(ru_after.ru_utime, ru_before.ru_utime);
        delta.ru_stime = sub(ru_after.ru_stime, ru_before.ru_stime);
        let ti = crate::ported::zsh_h::timeinfo::from_rusage(&delta);
        // c:Src/jobs.c:808-809 — `s = getsparam("TIMEFMT"); s ||
        // DEFAULT_TIMEFMT`. Honor user-set TIMEFMT, fall back to the
        // canonical default.
        let fmt = crate::ported::params::getsparam("TIMEFMT")
            .unwrap_or_else(|| crate::ported::zsh_system_h::DEFAULT_TIMEFMT.to_string());
        // c:Src/jobs.c:768 `desc` arg — for the `time { sublist }` /
        // `time simple-cmd` keyword path, zsh passes the sublist's
        // source text (used by %J via printtime). The compiler now
        // threads the rendered source text through as the desc operand
        // (compile_zsh.rs Time arm, argc==2 form). Bug #66.
        let line = crate::ported::jobs::printtime(elapsed.as_secs_f64(), &ti, &fmt, &desc);
        eprintln!("{}", line);
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
        let path_c = match CString::new(path.clone()) {
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

    // BUILTIN_SET_TRY_BLOCK_ERROR — capture the try-block's exit
    // status into `__zshrs_try_block_saved_status` (a scratch
    // scalar) so the always-arm can later restore it. Also set
    // `TRY_BLOCK_ERROR` per zsh semantics: it stays at -1 unless
    // the try-block fired an explicit error (errflag), per
    // c:Src/exec.c execlist's WC_TRYBLOCK arm.
    vm.register_builtin(BUILTIN_SET_TRY_BLOCK_ERROR, |vm, _argc| {
        use std::sync::atomic::Ordering;
        let vm_status = vm.last_status;
        let errored = (crate::ported::utils::errflag.load(Ordering::Relaxed)
            & crate::ported::zsh_h::ERRFLAG_ERROR)
            != 0;
        // c:Src/exec.c WC_TRYBLOCK — the always-arm runs with a
        // clean escape state. Snapshot RETFLAG / BREAKS / CONTFLAG /
        // EXIT_PENDING here and clear them; RESTORE_TRY_BLOCK_STATUS
        // re-applies them at always-arm exit so the propagation jump
        // emitted by compile_zsh fires correctly.
        let ret_save = crate::ported::builtin::RETFLAG.swap(0, Ordering::Relaxed);
        let brk_save = crate::ported::builtin::BREAKS.swap(0, Ordering::Relaxed);
        let cont_save = crate::ported::builtin::CONTFLAG.swap(0, Ordering::Relaxed);
        let exit_save = crate::ported::builtin::EXIT_PENDING.swap(0, Ordering::Relaxed);
        TRY_ESCAPE_SAVE.with(|s| {
            s.borrow_mut()
                .push((ret_save, brk_save, cont_save, exit_save));
        });
        with_executor(|exec| {
            exec.set_scalar(
                "__zshrs_try_block_saved_status".to_string(),
                vm_status.to_string(),
            );
            // c:Src/exec.c WC_TRYBLOCK — TRY_BLOCK_ERROR reflects
            // the errflag state at try-block exit. zsh leaves it
            // at -1 (sentinel) when the block completed normally,
            // and sets to last_status when errflag triggered the
            // unwind. The always-arm can reset it to 0 to
            // SWALLOW the error.
            if errored {
                exec.set_scalar("TRY_BLOCK_ERROR".to_string(), vm_status.to_string());
                // Clear errflag so always-arm runs cleanly.
                crate::ported::utils::errflag
                    .fetch_and(!crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
            } else {
                // c:Src/Modules/parameter.c — TRY_BLOCK_ERROR reads
                // as 0 inside an always-arm when no error fired
                // (per zsh's PM_INTEGER default-zero). The previous
                // port set -1 (the C internal "no try yet" sentinel)
                // which leaked to user-visible reads.
                exec.set_scalar("TRY_BLOCK_ERROR".to_string(), "0".to_string());
            }
        });
        Value::Status(0)
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
        Value::Status(0)
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
                        Some(v) => env::set_var(&name, &v),
                        None => env::remove_var(&name),
                    }
                }
            }
        });
        Value::Status(0)
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
        use std::sync::atomic::Ordering;
        // c:Src/exec.c — the entire `{try} always {…}` construct's
        // exit status is the try-block's last status. Per zsh
        // semantics this carries through regardless of what the
        // always-arm did (including reads/writes of TRY_BLOCK_ERROR
        // — those affect later commands' visible value but don't
        // override the construct's exit). The "swallow" idiom in
        // C is gated on errflag state at always-arm exit, not on
        // TBE's literal value; full fidelity needs more state and
        // is deferred.
        let saved = with_executor(|exec| {
            exec.scalar("__zshrs_try_block_saved_status")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0)
        });
        // Re-apply the escape flags captured by SET_TRY_BLOCK_ERROR.
        // If the always-arm itself fired return/break/continue/exit,
        // its handler already overwrote the canonical atomics; let
        // those win — the always-arm's own escape always takes
        // priority over the try-block's deferred one.
        if let Some((ret, brk, cont, exit_p)) = TRY_ESCAPE_SAVE.with(|s| s.borrow_mut().pop()) {
            if crate::ported::builtin::RETFLAG.load(Ordering::Relaxed) == 0 {
                crate::ported::builtin::RETFLAG.store(ret, Ordering::Relaxed);
            }
            if crate::ported::builtin::BREAKS.load(Ordering::Relaxed) == 0 {
                crate::ported::builtin::BREAKS.store(brk, Ordering::Relaxed);
            }
            if crate::ported::builtin::CONTFLAG.load(Ordering::Relaxed) == 0 {
                crate::ported::builtin::CONTFLAG.store(cont, Ordering::Relaxed);
            }
            if crate::ported::builtin::EXIT_PENDING.load(Ordering::Relaxed) == 0 {
                crate::ported::builtin::EXIT_PENDING.store(exit_p, Ordering::Relaxed);
            }
        }
        Value::Status(saved)
    });

    vm.register_builtin(BUILTIN_IS_TTY, |vm, _argc| {
        let fd_str = vm.pop().to_str();
        let fd: i32 = fd_str.trim().parse().unwrap_or(-1);
        let is_tty = if fd < 0 {
            false
        } else {
            unsafe { libc::isatty(fd) != 0 }
        };
        Value::Bool(is_tty)
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
        // c:Src/exec.c:lineno = N — direct write to the param's
        // u_val. Cannot go through setsparam because LINENO carries
        // PM_READONLY (so `(t)LINENO` reads `integer-readonly-special`
        // per zsh); setsparam → assignstrvalue's PM_READONLY guard
        // would reject the internal write. C zsh handles this via the
        // PM_SPECIAL GSU vtable's setfn callback which bypasses the
        // generic readonly check; the Rust port writes the canonical
        // field directly instead.
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            if let Some(pm) = tab.get_mut("LINENO") {
                pm.u_val = n;
                pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
            }
        }
        // Mirror to the file-static `lineno` (utils.c:121) that
        // zerrmsg reads at utils.c:301 for the `:N: msg` prefix.
        crate::ported::utils::set_lineno(n as i32);
        // DAP hook — checks breakpoints / step mode / pause-request
        // for the line we just landed on. O(1) no-op when DAP is off
        // (single atomic load on a OnceLock). Inside `--dap` mode
        // this is the call that blocks the executor on a Condvar
        // until the IDE sends `continue`. Mirrors strykelang's
        // `debugger.should_stop(line) → debugger.prompt(...)` flow.
        crate::extensions::dap::check_line(n as u32);
        Value::Status(0)
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
        Value::Status(0)
    });

    // Direct port of Src/prompt.c:1631 cmdpop.
    vm.register_builtin(BUILTIN_CMD_POP, |_vm, _argc| {
        crate::ported::prompt::cmdpop();
        Value::Status(0)
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
            0 => Value::Bool(true),  // c:cond.c:520 set
            1 => Value::Bool(false), // c:cond.c:518/520 unset
            _ => {
                // c:cond.c:514 — unknown option: zwarnnam emitted by
                // optison itself when POSIXBUILTINS is unset; mirror to
                // stderr here for parity with the earlier diagnostic.
                eprintln!("{}:1: no such option: {}", shname(), name);
                Value::Bool(false)
            }
        }
    });
    // Tri-state `-o` for compile_cond's direct status path. Returns
    // 0 / 1 / 3 as a Value::Int that compile_cond consumes via
    // Op::SetStatus. Mirrors zsh's `[[ -o invalid ]]` returning $?=3.
    vm.register_builtin(BUILTIN_OPTION_CHECK_TRISTATE, |vm, _argc| {
        let name = vm.pop().to_str();
        let r = crate::ported::cond::optison("test", &name); // c:cond.c:502
                                                             // optison itself prints the diagnostic via zwarnnam when r=3
                                                             // and POSIXBUILTINS is unset (the canonical path). Don't
                                                             // double-emit here. r is already 0/1/3.
        Value::Int(r as i64)
    });

    // BUILTIN_PARAM_FILTER — `${var:#pat}` / `${var:|name}` etc.
    // PURE PASSTHRU: rebuild `${name:#pat}` and route to paramsubst.
    vm.register_builtin(BUILTIN_PARAM_FILTER, |vm, _argc| {
        let pattern = vm.pop().to_str();
        let name = vm.pop().to_str();
        let body = format!("${{{}:#{}}}", name, pattern);
        paramsubst_to_value(&body)
    });

    // `a[i]=(elements)` / `a[i,j]=(elements)` / `a[i]=()`
    // — subscripted-array assign with array RHS. Stack pushed by
    // compile_assign as: [elem0, elem1, …, elemN-1, name, key].
    vm.register_builtin(BUILTIN_SET_SUBSCRIPT_RANGE, |vm, argc| {
        let n = argc as usize;
        let mut popped: Vec<Value> = Vec::with_capacity(n);
        for _ in 0..n {
            popped.push(vm.pop());
        }
        popped.reverse();
        if popped.len() < 2 {
            return Value::Status(1);
        }
        let key = popped.pop().unwrap().to_str();
        let name = popped.pop().unwrap().to_str();
        let mut values: Vec<String> = Vec::new();
        for v in popped {
            match v {
                Value::Array(items) => {
                    for it in items {
                        values.push(it.to_str());
                    }
                }
                other => values.push(other.to_str()),
            }
        }
        with_executor(|exec| {
            // Parse subscript: slice `lo,hi` or single index `i`.
            // setarrvalue (Src/params.c:2895) expects 1-based start/
            // end inclusive where start==end means replace one
            // element. Negative bounds translate to len+n+1 (1-based).
            //
            // c:Src/params.c — the END side accepts 0 as a valid value
            // that signals "insert BEFORE start position" (the canonical
            // `a[N,N-1]=val` prepend / mid-insert idiom). Bug #275 in
            // docs/BUGS.md: the previous Rust port clamped end up to 1,
            // collapsing `a[1,0]=(X Y)` into `a[1,1]=(X Y)` which
            // OVERWRITES position 1 instead of prepending. Provide two
            // translators — start_translate clamps to 1 (1-based);
            // end_translate keeps 0 intact so the splice in
            // setarrvalue (start_idx=0..end_idx=0) inserts at the front.
            // Bug #589: for scalars (no array), use the scalar's char
            // count as `len` so negative-index translation (`a[2,-1]`)
            // computes against the actual string length, not 0.
            let len = exec
                .array(&name)
                .map(|a| a.len() as i64)
                .or_else(|| {
                    crate::ported::params::paramtab()
                        .read()
                        .ok()
                        .and_then(|t| {
                            t.get(&name).and_then(|pm| {
                                if crate::ported::zsh_h::PM_TYPE(pm.node.flags as u32)
                                    == crate::ported::zsh_h::PM_SCALAR
                                {
                                    pm.u_str
                                        .as_ref()
                                        .map(|s| s.chars().count() as i64)
                                } else {
                                    None
                                }
                            })
                        })
                })
                .unwrap_or(0);
            let start_translate = |raw: i64| -> i32 {
                if raw < 0 {
                    (len + raw + 1).max(1) as i32
                } else {
                    raw.max(1) as i32
                }
            };
            let end_translate = |raw: i64| -> i32 {
                if raw < 0 {
                    (len + raw + 1).max(0) as i32
                } else {
                    raw.max(0) as i32
                }
            };
            // c:Src/params.c — KSH_ARRAYS option flips array subscripts
            // from 1-based to 0-based. setarrvalue expects 1-based
            // inclusive bounds, so under KSH_ARRAYS we shift positive
            // inputs by +1 before translation. Negative bounds left
            // alone (count from end). Sibling of #610/#611/#612.
            // Bug #613.
            let ksh_arrays = crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS);
            let ksh_shift = |raw: i64| -> i64 {
                if ksh_arrays && raw >= 0 { raw + 1 } else { raw }
            };
            let (start, end) = if let Some((s_str, e_str)) = key.split_once(',') {
                let s = ksh_shift(s_str.trim().parse::<i64>().unwrap_or(0));
                let e = ksh_shift(e_str.trim().parse::<i64>().unwrap_or(0));
                (start_translate(s), end_translate(e))
            } else {
                let i = ksh_shift(key.trim().parse::<i64>().unwrap_or(0));
                if i == 0 {
                    return;
                }
                let n = start_translate(i);
                (n, n)
            };
            // Route through canonical setarrvalue (Src/params.c:2895).
            // It handles PM_READONLY rejection, PM_HASHED slice-error,
            // PM_ARRAY splice + bounds clamp + padding (c:2980+).
            let taken = match crate::ported::params::paramtab().write() {
                Ok(mut tab) => tab.remove(&name),
                Err(_) => None,
            };
            // c:Src/params.c:2748+ — PM_SCALAR with subscript range
            // SPLICES the value into the scalar's char string. Bug
            // #589: zshrs's slice handler always called setarrvalue,
            // erroring "attempt to assign array value to non-array"
            // for `a=hello; a[2,3]=XYZ`. Detect PM_SCALAR and route
            // through assignstrvalue (which does scalar splice via
            // the PM_SCALAR arm at params.rs:3709-3789).
            let is_scalar = taken.as_ref().map_or(false, |pm| {
                crate::ported::zsh_h::PM_TYPE(pm.node.flags as u32)
                    == crate::ported::zsh_h::PM_SCALAR
            });
            let mut v = crate::ported::zsh_h::value {
                pm: taken,
                arr: Vec::new(),
                scanflags: 0,
                valflags: 0,
                start,
                end,
            };
            if is_scalar {
                // Scalar splice — concat values, route through
                // assignstrvalue which dispatches by PM_TYPE.
                // start_translate returns 1-based positions; assignstrvalue's
                // PM_SCALAR arm at params.rs:3735+ expects 0-based start
                // (chars before start are kept) and 0-based end-exclusive
                // (chars from end are kept). Convert: start-=1.
                if v.start > 0 {
                    v.start -= 1;
                }
                let val: String = values.join("");
                crate::ported::params::assignstrvalue(Some(&mut v), Some(val), 0);
            } else {
                crate::ported::params::setarrvalue(&mut v, values);
            }
            // Write the mutated Param back to paramtab — setarrvalue
            // mutated v.pm in-place; the prior `tab.remove(&name)` at
            // the top of this handler took ownership, so we re-insert
            // here. setarrvalue + this re-insert IS the canonical
            // store (Src/params.c:2895). No further mirror needed.
            if let Some(pm) = v.pm {
                if let Ok(mut tab) = crate::ported::params::paramtab().write() {
                    tab.insert(name, pm);
                }
            }
        });
        Value::Status(0)
    });

    // BUILTIN_CONCAT_SPLICE — word-segment concat with first/last
    // sticking (default zsh splice semantics for `${arr[@]}`, `$@`).
    vm.register_builtin(BUILTIN_CONCAT_SPLICE, |vm, _argc| {
        let rhs = vm.pop();
        let lhs = vm.pop();
        match (lhs, rhs) {
            (Value::Array(mut la), Value::Array(ra)) => {
                if la.is_empty() {
                    return Value::Array(ra);
                }
                if ra.is_empty() {
                    return Value::Array(la);
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
                la.push(Value::str(merged));
                la.extend(ra_iter);
                Value::Array(la)
            }
            (Value::Array(mut la), rhs_scalar) => {
                // c:Src/subst.c paramsubst splice — empty array on
                // either side preserves the empty (zero words),
                // doesn't collapse into a single-empty-string scalar.
                // Bug #120 in docs/BUGS.md: empty array slice
                // concatenated with empty literal returned
                // Value::str("") which surfaced as one empty arg
                // instead of zero args.
                let rhs_s = rhs_scalar.as_str_cow();
                if la.is_empty() {
                    if rhs_s.is_empty() {
                        return Value::Array(Vec::new());
                    }
                    return Value::str(rhs_s.to_string());
                }
                let last = la.pop().unwrap();
                let l_s = last.as_str_cow();
                let mut s = String::with_capacity(l_s.len() + rhs_s.len());
                s.push_str(&l_s);
                s.push_str(&rhs_s);
                la.push(Value::str(s));
                Value::Array(la)
            }
            (lhs_scalar, Value::Array(mut ra)) => {
                let lhs_s = lhs_scalar.as_str_cow();
                if ra.is_empty() {
                    // Empty-array RHS — preserve emptiness when the
                    // LHS is also empty (no prefix to attach). Bug
                    // #120 in docs/BUGS.md.
                    if lhs_s.is_empty() {
                        return Value::Array(Vec::new());
                    }
                    return Value::str(lhs_s.to_string());
                }
                let first = ra.remove(0);
                let r_s = first.as_str_cow();
                let mut s = String::with_capacity(lhs_s.len() + r_s.len());
                s.push_str(&lhs_s);
                s.push_str(&r_s);
                let mut out = Vec::with_capacity(ra.len() + 1);
                out.push(Value::str(s));
                out.extend(ra);
                Value::Array(out)
            }
            (lhs_s, rhs_s) => {
                let l = lhs_s.as_str_cow();
                let r = rhs_s.as_str_cow();
                let mut s = String::with_capacity(l.len() + r.len());
                s.push_str(&l);
                s.push_str(&r);
                Value::str(s)
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
            (Value::Array(la), Value::Array(ra)) => {
                if ra.is_empty() {
                    return Value::Array(la);
                }
                if la.is_empty() {
                    return Value::Array(ra);
                }
                let mut out = Vec::with_capacity(la.len() * ra.len());
                for a in &la {
                    let a_s = a.as_str_cow();
                    for b in &ra {
                        let b_s = b.as_str_cow();
                        let mut s = String::with_capacity(a_s.len() + b_s.len());
                        s.push_str(&a_s);
                        s.push_str(&b_s);
                        out.push(Value::str(s));
                    }
                }
                Value::Array(out)
            }
            (Value::Array(la), rhs_scalar) => {
                let r = rhs_scalar.as_str_cow();
                let out: Vec<Value> = la
                    .into_iter()
                    .map(|a| {
                        let a_s = a.as_str_cow();
                        let mut s = String::with_capacity(a_s.len() + r.len());
                        s.push_str(&a_s);
                        s.push_str(&r);
                        Value::str(s)
                    })
                    .collect();
                Value::Array(out)
            }
            (lhs_scalar, Value::Array(ra)) => {
                let l = lhs_scalar.as_str_cow();
                let out: Vec<Value> = ra
                    .into_iter()
                    .map(|b| {
                        let b_s = b.as_str_cow();
                        let mut s = String::with_capacity(l.len() + b_s.len());
                        s.push_str(&l);
                        s.push_str(&b_s);
                        Value::str(s)
                    })
                    .collect();
                Value::Array(out)
            }
            (lhs_s, rhs_s) => {
                let l = lhs_s.as_str_cow();
                let r = rhs_s.as_str_cow();
                let mut s = String::with_capacity(l.len() + r.len());
                s.push_str(&l);
                s.push_str(&r);
                Value::str(s)
            }
        }
    });

    vm.register_builtin(BUILTIN_CONCAT_DISTRIBUTE, |vm, argc| {
        let rhs = vm.pop();
        let lhs = vm.pop();
        // c:Src/options.c — RC_EXPAND_PARAM applies to UNQUOTED
        // expansions only; inside DQ `"$foo${arr}bar"` joins via
        // $IFS[0] regardless of the option. The compiler emits
        // CallBuiltin(BUILTIN_CONCAT_DISTRIBUTE, 1) when the parent
        // word is DQ-wrapped (compile_zsh.rs parent_is_dq); the
        // default UNQUOTED path emits argc=2 (lhs + rhs). Treat
        // argc==1 as "force rc_expand off." Bug #246 in docs/BUGS.md.
        let dq_suppress = argc == 1;
        let rc_expand = !dq_suppress
            && with_executor(|exec| opt_state_get("rcexpandparam").unwrap_or(false));
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
        let join_arr = |arr: Vec<Value>| -> String {
            let sep = ifs_first();
            arr.iter()
                .map(|v| v.as_str_cow().into_owned())
                .collect::<Vec<_>>()
                .join(&sep)
        };
        if !rc_expand {
            // Default: join any Array side to scalar, then concat.
            let l = match lhs {
                Value::Array(a) => join_arr(a),
                other => other.as_str_cow().into_owned(),
            };
            let r = match rhs {
                Value::Array(a) => join_arr(a),
                other => other.as_str_cow().into_owned(),
            };
            let mut s = String::with_capacity(l.len() + r.len());
            s.push_str(&l);
            s.push_str(&r);
            return Value::str(s);
        }
        match (lhs, rhs) {
            (Value::Array(la), Value::Array(ra)) => {
                // Cartesian product: [a + b for a in la for b in ra].
                let mut out = Vec::with_capacity(la.len() * ra.len().max(1));
                if ra.is_empty() {
                    return Value::Array(la);
                }
                if la.is_empty() {
                    return Value::Array(ra);
                }
                for a in &la {
                    let a_s = a.as_str_cow();
                    for b in &ra {
                        let b_s = b.as_str_cow();
                        let mut s = String::with_capacity(a_s.len() + b_s.len());
                        s.push_str(&a_s);
                        s.push_str(&b_s);
                        out.push(Value::str(s));
                    }
                }
                Value::Array(out)
            }
            (Value::Array(la), rhs_scalar) => {
                let r = rhs_scalar.as_str_cow();
                let out: Vec<Value> = la
                    .into_iter()
                    .map(|a| {
                        let a_s = a.as_str_cow();
                        let mut s = String::with_capacity(a_s.len() + r.len());
                        s.push_str(&a_s);
                        s.push_str(&r);
                        Value::str(s)
                    })
                    .collect();
                Value::Array(out)
            }
            (lhs_scalar, Value::Array(ra)) => {
                let l = lhs_scalar.as_str_cow();
                let out: Vec<Value> = ra
                    .into_iter()
                    .map(|b| {
                        let b_s = b.as_str_cow();
                        let mut s = String::with_capacity(l.len() + b_s.len());
                        s.push_str(&l);
                        s.push_str(&b_s);
                        Value::str(s)
                    })
                    .collect();
                Value::Array(out)
            }
            (lhs_s, rhs_s) => {
                // Fast path: both scalar → identical to Op::Concat.
                let l = lhs_s.as_str_cow();
                let r = rhs_s.as_str_cow();
                let mut s = String::with_capacity(l.len() + r.len());
                s.push_str(&l);
                s.push_str(&r);
                Value::str(s)
            }
        }
    });

    // `[[ a -ef b ]]` — same-inode test. Resolves both paths via fs::metadata
    // (follows symlinks the way zsh's -ef does) and compares (dev, inode).
    // Returns false on any I/O error (path missing, permission denied, etc.).
    vm.register_builtin(BUILTIN_SAME_FILE, |vm, _argc| {
        let b = vm.pop().to_str();
        let a = vm.pop().to_str();
        let same = match (fs::metadata(&a), fs::metadata(&b)) {
            (Ok(ma), Ok(mb)) => ma.dev() == mb.dev() && ma.ino() == mb.ino(),
            _ => false,
        };
        Value::Bool(same)
    });

    // `[[ -c path ]]` — character device.
    vm.register_builtin(BUILTIN_IS_CHARDEV, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = fs::metadata(&path)
            .map(|m| m.file_type().is_char_device())
            .unwrap_or(false);
        Value::Bool(result)
    });
    // `[[ -b path ]]` — block device.
    vm.register_builtin(BUILTIN_IS_BLOCKDEV, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = fs::metadata(&path)
            .map(|m| m.file_type().is_block_device())
            .unwrap_or(false);
        Value::Bool(result)
    });
    // `[[ -p path ]]` — FIFO (named pipe).
    vm.register_builtin(BUILTIN_IS_FIFO, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = fs::metadata(&path)
            .map(|m| m.file_type().is_fifo())
            .unwrap_or(false);
        Value::Bool(result)
    });
    // `[[ -S path ]]` — socket.
    vm.register_builtin(BUILTIN_IS_SOCKET, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_socket())
            .unwrap_or(false);
        Value::Bool(result)
    });

    // `[[ -k path ]]` / `-u` / `-g` — sticky / setuid / setgid bit.
    vm.register_builtin(BUILTIN_HAS_STICKY, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = fs::metadata(&path)
            .map(|m| m.permissions().mode() & libc::S_ISVTX as u32 != 0)
            .unwrap_or(false);
        Value::Bool(result)
    });
    vm.register_builtin(BUILTIN_HAS_SETUID, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = fs::metadata(&path)
            .map(|m| m.permissions().mode() & libc::S_ISUID as u32 != 0)
            .unwrap_or(false);
        Value::Bool(result)
    });
    vm.register_builtin(BUILTIN_HAS_SETGID, |vm, _argc| {
        let path = vm.pop().to_str();
        let result = fs::metadata(&path)
            .map(|m| m.permissions().mode() & libc::S_ISGID as u32 != 0)
            .unwrap_or(false);
        Value::Bool(result)
    });
    vm.register_builtin(BUILTIN_OWNED_BY_USER, |vm, _argc| {
        let path = vm.pop().to_str();
        let euid = unsafe { libc::geteuid() };
        let result = fs::metadata(&path)
            .map(|m| m.uid() == euid)
            .unwrap_or(false);
        Value::Bool(result)
    });
    vm.register_builtin(BUILTIN_OWNED_BY_GROUP, |vm, _argc| {
        let path = vm.pop().to_str();
        let egid = unsafe { libc::getegid() };
        let result = fs::metadata(&path)
            .map(|m| m.gid() == egid)
            .unwrap_or(false);
        Value::Bool(result)
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
        let result = fs::metadata(&path)
            .map(|m| m.atime() <= m.mtime())
            .unwrap_or(false);
        Value::Bool(result)
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
        let ta = fs::metadata(&a).and_then(|m| m.modified()).ok();
        let tb = fs::metadata(&b).and_then(|m| m.modified()).ok();
        let result = match (ta, tb) {
            (Some(ta), Some(tb)) => ta > tb,
            _ => false,
        };
        Value::Bool(result)
    });

    // `[[ a -ot b ]]` — mirror of -nt. Same both-must-exist contract.
    vm.register_builtin(BUILTIN_FILE_OLDER, |vm, _argc| {
        let b = vm.pop().to_str();
        let a = vm.pop().to_str();
        let ta = fs::metadata(&a).and_then(|m| m.modified()).ok();
        let tb = fs::metadata(&b).and_then(|m| m.modified()).ok();
        let result = match (ta, tb) {
            (Some(ta), Some(tb)) => ta < tb,
            _ => false,
        };
        Value::Bool(result)
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
    vm.register_builtin(BUILTIN_XTRACE_IS_ON, |_vm, _argc| {
        // Push live xtrace state. Caller pairs this with JumpIfFalse
        // to skip the trace-string-building block when xtrace is off,
        // avoiding side-effectful operand re-evaluation. Bug #159 in
        // docs/BUGS.md.
        let on = with_executor(|_| opt_state_get("xtrace").unwrap_or(false));
        Value::Int(if on { 1 } else { 0 })
    });

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
        let on = with_executor(|exec| opt_state_get("xtrace").unwrap_or(false));
        if on {
            let already = XTRACE_DONE_PS4.with(|f| f.get());
            if !already {
                printprompt4();
            }
            eprintln!("{}", cmd_text);
            XTRACE_DONE_PS4.with(|f| f.set(false));
        }
        Value::Status(0)
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
        let on = with_executor(|exec| opt_state_get("xtrace").unwrap_or(false));
        if on {
            let n_args = argc.saturating_sub(1) as usize;
            let len = vm.stack.len();
            // c:Src/exec.c:2055 — argv is the POST-expansion word
            // list, so an arg that expanded to multiple words splats
            // into multiple trace tokens AND an arg that expanded to
            // zero words (empty unquoted `${UNSET}`) emits nothing.
            // pop_args (line 6243) already does this splat for the
            // real handler; mirror the same Array → splat / empty →
            // drop logic here so xtrace renders `echo ${UNSET}` as
            // `echo` (zsh) instead of `echo ''` (the previous
            // single-arg stringify path returned "" and then
            // quotedzputs wrapped it in `''`).
            let arg_strs: Vec<String> = if n_args > 0 && len >= n_args {
                let mut out = Vec::new();
                for v in &vm.stack[len - n_args..] {
                    match v {
                        Value::Array(items) => {
                            for item in items {
                                out.push(quotedzputs(&item.to_str()));
                            }
                        }
                        other => out.push(quotedzputs(&other.to_str())),
                    }
                }
                out
            } else {
                Vec::new()
            };
            // Builtins dispatch through `execbuiltin` (Src/builtin.c:442)
            // which emits its own PS4 + name + args xtrace. To avoid
            // double-emission, skip our emission here when the first
            // arg is a known builtin with a registered HandlerFunc —
            // those go through execbuiltin and will trace themselves.
            // Externals + builtins-not-yet-routed-through-execbuiltin
            // keep our emission as a stand-in.
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
                let already_ps4 = XTRACE_DONE_PS4.with(|f| f.get());
                if !already_ps4 {
                    printprompt4();
                }
                eprintln!("{}", line);
            }
            XTRACE_DONE_PS4.with(|f| f.set(false));
        }
        Value::Status(0)
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
        let on = with_executor(|exec| opt_state_get("xtrace").unwrap_or(false));
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
        Value::Status(0)
    });

    // BUILTIN_XTRACE_NEWLINE — emit trailing `\n` + flush iff a
    // prior XTRACE_ASSIGN this line already emitted PS4. Mirrors
    // C's `fputc('\n', xtrerr); fflush(xtrerr);` at exec.c:3398
    // (the assignment-only path through execcmd_exec).
    vm.register_builtin(BUILTIN_XTRACE_NEWLINE, |_vm, _argc| {
        let on = with_executor(|exec| opt_state_get("xtrace").unwrap_or(false));
        if on {
            let already_ps4 = XTRACE_DONE_PS4.with(|f| f.get());
            if already_ps4 {
                eprintln!();
                XTRACE_DONE_PS4.with(|f| f.set(false));
            }
        }
        Value::Status(0)
    });

    // c:Src/exec.c WC_TRYBLOCK — post-always re-jump probes. Each
    // returns 1 + consumes the atomic when the corresponding
    // escape flag is set; the try-block compile pairs each with
    // a JumpIfFalse + Jump → outer scope's return / break /
    // continue patches.
    vm.register_builtin(BUILTIN_RETFLAG_CHECK, |_vm, _argc| {
        use std::sync::atomic::Ordering;
        let r = crate::ported::builtin::RETFLAG.load(Ordering::Relaxed);
        if r != 0 {
            // Don't clear here — doshfunc owns the clear at c:6047
            // when the function unwinds. Leaving it set propagates
            // through nested `eval`/`source` callers correctly.
            Value::Int(1)
        } else {
            Value::Int(0)
        }
    });
    vm.register_builtin(BUILTIN_BREAKS_CHECK, |_vm, _argc| {
        use std::sync::atomic::Ordering;
        let b = crate::ported::builtin::BREAKS.load(Ordering::Relaxed);
        let c = crate::ported::builtin::CONTFLAG.load(Ordering::Relaxed);
        // `break` sets BREAKS but NOT CONTFLAG; `continue` sets both.
        // Filter out the continue path here so the two checks are
        // mutually exclusive.
        if b != 0 && c == 0 {
            // Consume BREAKS so the outer loop's break_patches
            // landing doesn't double-decrement.
            crate::ported::builtin::BREAKS.store(0, Ordering::Relaxed);
            Value::Int(1)
        } else {
            Value::Int(0)
        }
    });
    vm.register_builtin(BUILTIN_CONTFLAG_CHECK, |_vm, _argc| {
        use std::sync::atomic::Ordering;
        let c = crate::ported::builtin::CONTFLAG.load(Ordering::Relaxed);
        if c != 0 {
            crate::ported::builtin::CONTFLAG.store(0, Ordering::Relaxed);
            crate::ported::builtin::BREAKS.store(0, Ordering::Relaxed);
            Value::Int(1)
        } else {
            Value::Int(0)
        }
    });
    vm.register_builtin(BUILTIN_NOEXEC_CHECK, |_vm, _argc| {
        // c:Src/exec.c:1390 — `set -n` / `noexec` option: parse but
        // don't execute. Returns Int(1) when noexec is set so the
        // emit-side JumpIfTrue skips the statement body.
        if opt_state_get("noexec").unwrap_or(false) {
            Value::Int(1)
        } else {
            Value::Int(0)
        }
    });
    vm.register_builtin(BUILTIN_DONETRAP_RESET, |_vm, _argc| {
        // c:Src/exec.c:1455 — `donetrap = 0;` at sublist start.
        // Reset before each top-level statement so the next
        // sublist's ERREXIT_CHECK fires the ZERR trap on its FIRST
        // non-zero command. Carries the "already fired" state
        // across function-call returns within the SAME outer
        // sublist (per C semantics — donetrap is process-global).
        // Bug #303 in docs/BUGS.md.
        crate::ported::exec::DONETRAP.store(0, std::sync::atomic::Ordering::Relaxed);
        Value::Status(0)
    });

    // `[[ -z X ]]` / `[[ -n X ]]` — pop one Value, route through
    // canonical `src/ported/cond.rs::evalcond` so the actual
    // empty/non-empty test reuses the C-port at `cond.rs:270-271`
    // (`'n' => !arg.is_empty()`, `'z' => arg.is_empty()`).
    //
    // The Array→args conversion lives at the bridge because cond.rs
    // expects `&[&str]` (C `cond_str` signature equivalent). For
    // `"${arr[@]}"` in DQ context the splice yields `Value::Array`
    // — an empty array still expands to one implicit empty word
    // (per zsh's "${arr[@]}" splat preserving at least one slot
    // in cond context), so:
    //   - Array(0)   → ["-z", ""]            → evalcond → 0 (true)
    //   - Array(1)   → ["-z", word]          → evalcond → 0/1
    //   - Array(2+)  → ["-z", w1, w2, ...]   → evalcond → 2 (parse
    //                                          error: too many ops)
    //                                          → coerced to false
    //   - Str(s)     → ["-z", s]             → evalcond → 0/1
    //
    // Bug #185 in docs/BUGS.md.
    fn run_cond_str_empty(v: Value, op: &str) -> Value {
        let words: Vec<String> = match v {
            Value::Array(arr) => arr.into_iter().map(|x| x.to_str()).collect(),
            Value::Str(s) => vec![s.to_string()],
            other => vec![other.to_str()],
        };
        let mut args: Vec<&str> = vec![op];
        if words.is_empty() {
            args.push("");
        } else {
            args.extend(words.iter().map(|s| s.as_str()));
        }
        let opts: std::collections::HashMap<String, bool> =
            std::collections::HashMap::new();
        let vars: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // c:Src/cond.c:62-66 — `evalcond` returns 0=true, 1=false,
        // 2=syntax-error. Coerce error to false (observable behavior
        // in zsh: `[[ -z a b ]]` errors and the test as a whole
        // returns non-zero).
        // `[[ ]]` dispatch — C's `evalcond(state, NULL)` calling convention.
        // `None` for from_test → mathevali integer-compare coercion path.
        let ret = crate::ported::cond::evalcond(&args, &opts, &vars, false, None);
        Value::Int(if ret == 0 { 1 } else { 0 })
    }
    vm.register_builtin(BUILTIN_COND_STR_EMPTY, |vm, _argc| {
        let v = vm.pop();
        run_cond_str_empty(v, "-z")
    });
    vm.register_builtin(BUILTIN_COND_STR_NONEMPTY, |vm, _argc| {
        let v = vm.pop();
        run_cond_str_empty(v, "-n")
    });

    // `exec N<<<"str"` — herestring redirect to explicit fd, applied
    // permanently. Direct port of `Src/exec.c:4655 getherestr` +
    // `addfd(forked, save, mfds, fn->fd1, fil, 0, ...)` at c:3766-
    // 3780 for the nullexec=1 bare-exec-redir path. Bug #205 in
    // docs/BUGS.md.
    vm.register_builtin(BUILTIN_EXEC_HERESTR_FD, |vm, _argc| {
        let fd = vm.pop().to_int() as i32;
        let content = vm.pop().to_str();
        // c:4671-4672 — append `\n` for "real" herestrings (not
        // heredoc-derived). zshrs's bare-exec path only fires for
        // the `<<<` syntax (REDIR_HERESTR), so always append.
        let body = format!("{}\n", content);
        // c:4673-4679 — gettempfile → write_loop → close → reopen
        // read-only → unlink. Rust equivalent via tempfile crate or
        // explicit O_TMPFILE; use mkstemp + unlink-immediately to
        // mirror C exactly.
        use std::ffi::CString;
        let mut tmpl: Vec<u8> = b"/tmp/zshrs_hs_XXXXXX\0".to_vec();
        let write_fd = unsafe { libc::mkstemp(tmpl.as_mut_ptr() as *mut libc::c_char) };
        if write_fd < 0 {
            crate::ported::utils::zwarn(&format!(
                "can't create temp file for here document: {}",
                std::io::Error::last_os_error()
            ));
            return Value::Status(1);
        }
        // c:4675 — write_loop(fd, t, len)
        let bytes = body.as_bytes();
        let mut off = 0;
        while off < bytes.len() {
            let n = unsafe {
                libc::write(
                    write_fd,
                    bytes[off..].as_ptr() as *const libc::c_void,
                    bytes.len() - off,
                )
            };
            if n <= 0 {
                unsafe { libc::close(write_fd) };
                return Value::Status(1);
            }
            off += n as usize;
        }
        unsafe { libc::close(write_fd) }; // c:4676
        // Path null-terminated by mkstemp; reopen for reading.
        let read_fd = unsafe { libc::open(tmpl.as_ptr() as *const libc::c_char, libc::O_RDONLY) };
        // c:4678 — unlink immediately so the file disappears on
        // close, leaving only the fd reference.
        unsafe { libc::unlink(tmpl.as_ptr() as *const libc::c_char) };
        if read_fd < 0 {
            return Value::Status(1);
        }
        // c:3779 addfd → dup2 to target fd, close intermediate.
        let r = unsafe { libc::dup2(read_fd, fd) };
        unsafe { libc::close(read_fd) };
        if r < 0 {
            return Value::Status(1);
        }
        Value::Status(0)
    });
    // c:Src/exec.c:2418 + addfd splice — MULTIOS fan-out. Stack
    // layout pushed by compile_zsh's coalescing pass:
    //   [target_1, op_byte_1, target_2, op_byte_2, …, target_N,
    //    op_byte_N, fd]
    // argc = 2N + 1. Pops, opens every target, sets up a pipe +
    // splitter thread that reads pipe → writes every chunk to
    // every opened target, dup2's pipe-write-end onto fd. The
    // splitter is closed + joined by host_redirect_scope_end.
    // Bug #36 in docs/BUGS.md.
    vm.register_builtin(BUILTIN_MULTIOS_REDIRECT, |vm, argc| {
        if argc < 3 || argc % 2 == 0 {
            // Bad shape — bail.
            return Value::Status(1);
        }
        // Pop fd first (top of stack).
        let fd = vm.pop().to_int() as i32;
        // Then pop (op, target) pairs in reverse compile order.
        let n_targets = ((argc - 1) / 2) as usize;
        let mut pairs: Vec<(u8, String)> = Vec::with_capacity(n_targets);
        for _ in 0..n_targets {
            let op_byte = vm.pop().to_int() as u8;
            let target = vm.pop().to_str();
            pairs.push((op_byte, target));
        }
        // Restore compile order (target_1 first).
        pairs.reverse();

        // Open every target per its op_byte.
        let mut target_fds: Vec<i32> = Vec::with_capacity(pairs.len());
        for (op_byte, target) in &pairs {
            let open_result = match *op_byte {
                r::WRITE | r::CLOBBER => fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(target),
                r::APPEND => fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .append(true)
                    .open(target),
                _ => fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(target),
            };
            match open_result {
                Ok(file) => target_fds.push(file.into_raw_fd()),
                Err(e) => {
                    let msg = match e.kind() {
                        std::io::ErrorKind::PermissionDenied => "permission denied",
                        std::io::ErrorKind::NotFound => "no such file or directory",
                        std::io::ErrorKind::IsADirectory => "is a directory",
                        _ => "redirect failed",
                    };
                    eprintln!("{}:1: {}: {}", shname(), msg, target);
                    // Close already-opened fds to avoid leaks.
                    for prev in &target_fds {
                        unsafe {
                            libc::close(*prev);
                        }
                    }
                    with_executor(|exec| {
                        exec.redirect_failed = true;
                    });
                    return Value::Status(1);
                }
            }
        }

        // Save current fd state for scope-end restoration.
        let saved = unsafe { libc::dup(fd) };
        if saved >= 0 {
            with_executor(|exec| {
                if let Some(top) = exec.redirect_scope_stack.last_mut() {
                    top.push((fd, saved));
                } else {
                    unsafe { libc::close(saved) };
                }
            });
        }

        // Create the splitter pipe.
        let (read_end, write_end) = match os_pipe::pipe() {
            Ok(p) => p,
            Err(_) => {
                for f in &target_fds {
                    unsafe {
                        libc::close(*f);
                    }
                }
                return Value::Status(1);
            }
        };
        let pipe_write_raw = AsRawFd::as_raw_fd(&write_end);
        // Spawn the splitter thread: read pipe → write every chunk
        // to every target fd. Each write inside the thread uses
        // libc::write directly on the raw fd (no Rust File ownership
        // so the splitter can close after EOF without racing main).
        let target_fds_for_thread = target_fds.clone();
        let handle = std::thread::spawn(move || {
            let mut r = read_end;
            let mut buf = [0u8; 8192];
            loop {
                match std::io::Read::read(&mut r, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        for &tfd in &target_fds_for_thread {
                            let mut off = 0;
                            while off < n {
                                let w = unsafe {
                                    libc::write(
                                        tfd,
                                        buf[off..n].as_ptr() as *const libc::c_void,
                                        n - off,
                                    )
                                };
                                if w <= 0 {
                                    break;
                                }
                                off += w as usize;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            // Close every target so file contents flush.
            for tfd in target_fds_for_thread {
                unsafe {
                    libc::close(tfd);
                }
            }
        });

        // Dup the pipe write-end onto the target fd; close the
        // original write_end so EOF arrives when host_redirect_scope_end
        // closes our tracked pipe_write_fd.
        let write_dup = unsafe { libc::dup(pipe_write_raw) };
        drop(write_end);
        if write_dup < 0 {
            return Value::Status(1);
        }
        unsafe {
            libc::dup2(write_dup, fd);
            libc::close(write_dup);
        }
        // Track the running splitter so scope-end can drain + join.
        // The "write_fd" we store is the user-visible fd (e.g. 1).
        // Closing that fd at scope-end isn't quite right; we need a
        // way to send EOF. Solution: track the write_dup we just
        // closed; instead keep a second dup for the close-on-end.
        let close_on_end = unsafe { libc::dup(fd) };
        with_executor(|exec| {
            if let Some(top) = exec.multios_scope_stack.last_mut() {
                top.push((close_on_end, handle));
            } else {
                // No scope — leak the dup; thread will keep running
                // until process exit. Should not happen because
                // host_redirect_scope_begin pushed a frame.
                unsafe { libc::close(close_on_end) };
            }
        });
        Value::Status(0)
    });
    // c:Src/exec.c:2418 input-arm — MULTIOS read fan-in. Stack
    // layout pushed by compile_zsh:
    //   [source_1, source_2, …, source_N, fd]
    // argc = N + 1. Opens every source, sets up a pipe + producer
    // thread that reads each source in order and writes to the
    // pipe write-end, then closes its write-end so the consumer
    // gets EOF. dup2 the pipe read-end onto fd. Bug #36 input
    // side in docs/BUGS.md.
    vm.register_builtin(BUILTIN_MULTIOS_READ, |vm, argc| {
        if argc < 2 {
            return Value::Status(1);
        }
        let fd = vm.pop().to_int() as i32;
        let n_sources = (argc - 1) as usize;
        let mut sources: Vec<String> = Vec::with_capacity(n_sources);
        for _ in 0..n_sources {
            sources.push(vm.pop().to_str());
        }
        sources.reverse();

        // Open every source.
        let mut source_fds: Vec<i32> = Vec::with_capacity(sources.len());
        for path in &sources {
            match fs::File::open(path) {
                Ok(f) => source_fds.push(f.into_raw_fd()),
                Err(e) => {
                    let msg = match e.kind() {
                        std::io::ErrorKind::PermissionDenied => "permission denied",
                        std::io::ErrorKind::NotFound => "no such file or directory",
                        _ => "open failed",
                    };
                    eprintln!("{}:1: {}: {}", shname(), msg, path);
                    for prev in &source_fds {
                        unsafe {
                            libc::close(*prev);
                        }
                    }
                    with_executor(|exec| {
                        exec.redirect_failed = true;
                    });
                    return Value::Status(1);
                }
            }
        }

        // Save current fd state for scope-end restoration.
        let saved = unsafe { libc::dup(fd) };
        if saved >= 0 {
            with_executor(|exec| {
                if let Some(top) = exec.redirect_scope_stack.last_mut() {
                    top.push((fd, saved));
                } else {
                    unsafe { libc::close(saved) };
                }
            });
        }

        // Create the concatenator pipe.
        let (read_end, write_end) = match os_pipe::pipe() {
            Ok(p) => p,
            Err(_) => {
                for f in &source_fds {
                    unsafe {
                        libc::close(*f);
                    }
                }
                return Value::Status(1);
            }
        };
        // dup the pipe read-end onto fd before spawning the
        // producer; close the original read_end so the consumer
        // (reading via fd) is the sole reference until scope-end.
        let read_dup = unsafe { libc::dup(AsRawFd::as_raw_fd(&read_end)) };
        drop(read_end);
        if read_dup < 0 {
            for f in &source_fds {
                unsafe {
                    libc::close(*f);
                }
            }
            return Value::Status(1);
        }
        unsafe {
            libc::dup2(read_dup, fd);
            libc::close(read_dup);
        }
        // Spawn the producer.
        let source_fds_for_thread = source_fds.clone();
        let handle = std::thread::spawn(move || {
            let mut w = write_end;
            let mut buf = [0u8; 8192];
            for sfd in source_fds_for_thread {
                loop {
                    let n = unsafe {
                        libc::read(
                            sfd,
                            buf.as_mut_ptr() as *mut libc::c_void,
                            buf.len(),
                        )
                    };
                    if n <= 0 {
                        break;
                    }
                    let n = n as usize;
                    if std::io::Write::write_all(&mut w, &buf[..n]).is_err() {
                        break;
                    }
                }
                unsafe {
                    libc::close(sfd);
                }
            }
            // Closing w (the write_end) at scope drop signals EOF
            // to the consumer.
        });
        with_executor(|exec| {
            // Track using a closed-write sentinel — the producer
            // owns write_end so we just need to join. Use -1 fd
            // marker meaning "no fd to close".
            if let Some(top) = exec.multios_scope_stack.last_mut() {
                top.push((-1, handle));
            } else {
                let _ = handle.join();
            }
        });
        Value::Status(0)
    });
    // c:Src/exec.c — block-level redirect-failure gate. When a
    // compound command (`{ … } < file`, `( … ) > file`, etc.) has a
    // failing redirect (e.g. `< /nonexistent`), zsh skips the entire
    // body AND sets lastval to 1. The simple-command path's
    // redirect_failed check (line 215-221 above) only catches the
    // failure when a builtin dispatches and is consumed by that
    // single builtin call — so a multi-statement block kept running
    // its remaining statements after the redir error. Emit-side at
    // compile_zsh.rs::compile_command's Redirected arm pairs this
    // with a JumpIfTrue → WithRedirectsEnd to abandon the body.
    vm.register_builtin(BUILTIN_REDIRECT_FAILED_CHECK, |vm, _argc| {
        let failed = with_executor(|exec| {
            let f = exec.redirect_failed;
            exec.redirect_failed = false;
            f
        });
        if failed {
            vm.last_status = 1;
            Value::Int(1)
        } else {
            Value::Int(0)
        }
    });
    // c:Src/exec.c — drop-in replacement for fusevm's Op::Exec used by
    // the dynamic-first-word path (`$cmd`, `$(cmd)`, glob-named cmds).
    // fusevm's Op::Exec returns Value::Status(0) when post-expansion
    // argv is empty (vm.rs:1722) — that clobbers \$? for the
    // `\$(exit 1); echo \$?` case where the cmd-subst left
    // last_status = 1 but the empty expansion gets exec'd to 0.
    // Mirror C zsh: when the word list is empty after expansion,
    // \$? becomes whatever the inner cmd-subst's last_status is
    // (preserved here by returning Value::Status(last_status)).
    vm.register_builtin(BUILTIN_EXEC_DYNAMIC, |vm, argc| {
        let raw = pop_args(vm, argc);
        // Flatten Array entries into argv slots (matches fusevm
        // Op::Exec's flatten at vm.rs:1660-1665) so `${arr[@]}` /
        // splice expansions produce one argv slot per element.
        let args: Vec<String> = raw.into_iter().collect();
        // c:Src/subst.c paramsubst — when `${var:?msg}` or
        // `${var?msg}` set errflag, the expansion may produce empty
        // argv[0] which would fall into the EACCES/permission-denied
        // path below, masking the real paramsubst diagnostic with a
        // spurious "permission denied:" line and rc=126. Honour
        // errflag so the simple command ends with the paramsubst
        // error as the sole diagnostic, rc=1. Bug #86.
        if (crate::ported::utils::errflag.load(std::sync::atomic::Ordering::SeqCst)
            & crate::ported::zsh_h::ERRFLAG_ERROR)
            != 0
        {
            return Value::Status(1);
        }
        if args.is_empty() {
            // c:Src/exec.c — empty argv preserves prior \$?. The
            // cmd-subst inside the word already set last_status; just
            // round-trip it back through SetStatus.
            return Value::Status(vm.last_status);
        }
        if args[0].is_empty() {
            // Explicit empty command word — exec returns EACCES.
            let script_name =
                crate::ported::utils::scriptname_get().unwrap_or_else(|| "zshrs".to_string());
            let lineno: u64 = with_executor(|exec| {
                exec.scalar("LINENO")
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(1)
            });
            eprintln!("{}:{}: permission denied: ", script_name, lineno);
            return Value::Status(126);
        }
        // c:Src/exec.c:2900 execcmd_exec — canonical simple-command
        // dispatcher. Runs precmd-modifier walk (c:3013-3091), then
        // dispatches to execbuiltin (c:4233) / runshfunc (c:3431+) /
        // execute (c:4314) per the resolved head. zshrs's bytecode VM
        // expanded the args before reaching here; we feed them in via
        // eparams.args and let execcmd_exec do the rest exactly as C
        // does for static heads. Without this, `c=builtin; $c source X`
        // skipped the precmd walk and emitted "command not found:
        // builtin".
        let mut state = crate::ported::zsh_h::estate {
            prog: Box::<crate::ported::zsh_h::eprog>::default(),
            pc: 0,
            strs: None,
            strs_offset: 0,
        };
        let mut eparams = crate::ported::zsh_h::execcmd_params {
            args: Some(args),
            redir: None,
            beg: 0,
            varspc: None,
            assignspc: None,
            typ: crate::ported::zsh_h::WC_SIMPLE as i32,
            postassigns: 0,
            htok: 0,
        };
        // input/output=0 → no pipe redirection (use shell stdio
        // directly); `output != 0` at c:2988 forks immediately. last1=1
        // marks this as the last/only command (no further pipe stages).
        crate::ported::exec::execcmd_exec(
            &mut state,
            &mut eparams,
            0,                                    // input  (c:2989)
            0,                                    // output (c:2988)
            crate::ported::zsh_h::Z_SYNC as i32,  // how
            1,                                    // last1=1 last/only
            -1,                                   // close_if_forked
        );
        let status = crate::ported::builtin::LASTVAL
            .load(std::sync::atomic::Ordering::Relaxed);
        let mut synth = crate::ported::zsh_h::job::default();
        crate::ported::jobs::waitonejob(&mut synth);
        Value::Status(status)
    });
    // c:Src/exec.c:3340-3364 — `< file` / `> file` with no command
    // word. Resolves NULLCMD/READNULLCMD at runtime then routes
    // through host_exec_external. Redirects are already applied by
    // the surrounding WithRedirectsBegin scope.
    vm.register_builtin(BUILTIN_NULLCMD_EXEC, |vm, argc| {
        let args = pop_args(vm, argc);
        let is_single_read = args
            .first()
            .map(|s| s != "0" && !s.is_empty())
            .unwrap_or(false);
        // c:Src/exec.c — when the surrounding redir-open failed
        // (e.g. `< /nonexistent`), zerr already printed the diag
        // and set redirect_failed. Don't invoke NULLCMD — return
        // status 1 like the wordcode path does.
        let redir_failed = with_executor(|exec| {
            let f = exec.redirect_failed;
            exec.redirect_failed = false;
            f
        });
        if redir_failed {
            crate::ported::builtin::LASTVAL.store(1, std::sync::atomic::Ordering::Relaxed);
            return Value::Status(1);
        }
        let nullcmd = crate::ported::params::getsparam("NULLCMD");
        let nc_str = nullcmd.as_deref().unwrap_or("");
        let nc_empty = nc_str.is_empty();
        // c:3340-3344 — CSHNULLCMD or no NULLCMD set → diagnostic.
        if nc_empty || crate::ported::zsh_h::isset(crate::ported::zsh_h::CSHNULLCMD) {
            let script_name =
                crate::ported::utils::scriptname_get().unwrap_or_else(|| "zshrs".to_string());
            let lineno: u64 = with_executor(|exec| {
                exec.scalar("LINENO")
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(1)
            });
            eprintln!("{}:{}: redirection with no command", script_name, lineno);
            return Value::Status(1);
        }
        // c:3350 — SHNULLCMD → run `:`.
        let cmd: String = if crate::ported::zsh_h::isset(crate::ported::zsh_h::SHNULLCMD) {
            ":".to_string()
        } else if is_single_read {
            // c:3354-3359 — single REDIR_READ + READNULLCMD set → readnullcmd.
            let rnc = crate::ported::params::getsparam("READNULLCMD");
            let rnc_str = rnc.as_deref().unwrap_or("");
            if !rnc_str.is_empty() {
                rnc_str.to_string()
            } else {
                nc_str.to_string() // c:3360-3363 fallback
            }
        } else {
            nc_str.to_string() // c:3360-3363
        };
        let status = with_executor(|exec| exec.host_exec_external(&[cmd]));
        crate::ported::builtin::LASTVAL.store(status, std::sync::atomic::Ordering::Relaxed);
        Value::Status(status)
    });
    // c:Src/exec.c:3342 — `zerr("redirection with no command")`.
    // Bare prefix-keyword (`builtin`, `command`, `exec`, `noglob`,
    // `nocorrect`) with a redirect but no command word. Emits the
    // canonical diagnostic via zerr (which sets errflag) and
    // returns Status(1). Bug #534.
    vm.register_builtin(BUILTIN_REDIR_NO_CMD, |_vm, _argc| {
        crate::ported::utils::zerr("redirection with no command");
        Value::Status(1)
    });
    vm.register_builtin(BUILTIN_DEBUG_TRAP, |vm, _argc| {
        // c:Src/signals.c:1245 dotrap(SIGDEBUG) — fires the DEBUG
        // trap body once per statement. The body sees the parent
        // shell's $? (LASTVAL). Guard against re-entry: commands
        // inside the DEBUG trap body would otherwise trigger
        // DEBUG_TRAP recursively → stack overflow. zsh guards via
        // its in_trap counter; we mirror with a thread-local Cell.
        //
        // c:Src/exec.c::trapcmd — before dotrap, the C source sets
        // `ZSH_DEBUG_CMD` to the about-to-run command text via
        // `dupstring(text)`. The trap body reads the parameter;
        // C unsets it after the trap returns. compile_list emits
        // the rendered statement text as the single arg here so the
        // shell-visible parameter reflects the command. Bug #263 in
        // docs/BUGS.md.
        let cmd_text = vm.pop().to_str();
        DEBUG_TRAP_REENTRY.with(|c| {
            if c.get() {
                return Value::Status(0);
            }
            // c:Src/exec.c:1423 — `if (sigtrapped[SIGDEBUG] &&
            // isset(DEBUGBEFORECMD) && !intrap)`. Bug #573: without
            // this gate, every sublist boundary called
            // setsparam("ZSH_DEBUG_CMD", ...) even when no DEBUG trap
            // was set, polluting the param table and (under
            // WARN_CREATE_GLOBAL) emitting a spurious
            // `scalar parameter ZSH_DEBUG_CMD created globally`
            // warning at every function call.
            //
            // Two trap registries exist (per signals.rs:1481-1511 dotrap):
            //   - settrap path → sigtrapped[SIGDEBUG] bits set
            //   - bin_trap path → traps_table["DEBUG"] populated, sigtrapped untouched
            // Mirror the dotrap dispatch decision: skip only when BOTH
            // are absent.
            let sig_debug = crate::ported::signals_h::SIGDEBUG as usize;
            let debug_trapped = crate::ported::signals::sigtrapped
                .lock()
                .map(|v| v.get(sig_debug).copied().unwrap_or(0))
                .unwrap_or(0);
            let debug_in_table = crate::ported::builtin::traps_table()
                .lock()
                .map(|t| t.contains_key("DEBUG"))
                .unwrap_or(false);
            if debug_trapped == 0 && !debug_in_table {
                return Value::Status(0);
            }
            c.set(true);
            // c:Src/exec.c — set ZSH_DEBUG_CMD scalar (PM_READONLY
            // is NOT set on ZSH_DEBUG_CMD, so the canonical
            // setsparam path is fine here — no direct paramtab
            // mutation needed).
            crate::ported::params::setsparam("ZSH_DEBUG_CMD", &cmd_text);
            let _ = crate::ported::signals::dotrap(crate::ported::signals_h::SIGDEBUG);
            // c:Src/exec.c::trapcmd — `unsetparam("ZSH_DEBUG_CMD")`
            // after the trap returns. Mirror that.
            crate::ported::params::unsetparam("ZSH_DEBUG_CMD");
            c.set(false);
            Value::Status(0)
        })
    });

    vm.register_builtin(BUILTIN_ERREXIT_CHECK, |vm, _argc| {
        // Returns Value::Int(1) when the caller should jump to the
        // current scope's return-patch landing (subshell-end / func-
        // end / chunk-end). Returns Value::Int(0) otherwise. Emit
        // side at `emit_errexit_check` pairs this with a JumpIfTrue
        // → return_patches pattern so the caller can short-circuit.
        //
        // Four triggers:
        //   1. RETFLAG set by a nested `return` / `exit` (eval,
        //      sourced file, called function). Unwind THIS scope so
        //      the flag propagates outward until something clears it.
        //   2. EXIT_PENDING set (mostly subshell-context exits). Same
        //      propagation logic.
        //   3. `set -e` + nonzero status — the classic errexit path.
        //   4. errflag set in non-interactive mode — readonly
        //      reassign, bad redirect, parse error mid-expansion etc.
        //      Aborts the script (c:Src/init.c loop()).
        use std::sync::atomic::Ordering;
        let retflag = crate::ported::builtin::RETFLAG.load(Ordering::Relaxed);
        let exit_pending = crate::ported::builtin::EXIT_PENDING.load(Ordering::Relaxed);
        if retflag != 0 || exit_pending != 0 {
            return Value::Int(1);
        }
        let errflag_set = (crate::ported::utils::errflag.load(Ordering::Relaxed)
            & crate::ported::zsh_h::ERRFLAG_ERROR)
            != 0;
        if errflag_set && !crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE) {
            // Clear errflag so the abort doesn't keep re-triggering;
            // the script-end last_status gives the caller the
            // failing status. Update BOTH executor's last_status
            // (LASTVAL) AND the VM's last_status so run_chunk's
            // post-script sync sees the failing value.
            crate::ported::utils::errflag
                .fetch_and(!crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
            with_executor(|exec| exec.set_last_status(1));
            vm.last_status = 1;
            // c:Src/init.c loop() — a non-interactive errflag-fired
            // abort propagates to the SHELL, not just the current
            // function/sourced file. Inside a function, the local
            // BUILTIN_ERREXIT_CHECK unwinds the function scope; but
            // the caller's next ERREXIT_CHECK only sees errflag if we
            // didn't clear it — and we did (above). Set EXIT_PENDING
            // so the outer ERREXIT_CHECK at script-level takes the
            // EXIT_PENDING arm and aborts. Bug #74 in docs/BUGS.md:
            // `f() { local -r x=5; x=10; }; f; echo after` printed
            // `after` because errflag-clear above let the script-level
            // check see a clean state.
            crate::ported::builtin::EXIT_VAL.store(1, Ordering::Relaxed);
            crate::ported::builtin::EXIT_PENDING.store(1, Ordering::Relaxed);
            return Value::Int(1);
        }
        let last = vm.last_status;
        if last == 0 {
            return Value::Int(0);
        }
        // c:Src/exec.c:1598 `if (!this_noerrexit && !donetrap &&
        // !this_donetrap)` — gate the ZERR trap fire on DONETRAP so
        // an inner sublist (e.g. `false` inside a function) that
        // already fired ZERR doesn't fire it AGAIN at the outer
        // sublist's post-command check (after the function
        // returned non-zero). Bug #303 in docs/BUGS.md. DONETRAP
        // is reset at top-level statement boundaries via
        // BUILTIN_DONETRAP_RESET (compile_list emit at
        // compile_zsh.rs).
        let already_done =
            crate::ported::exec::DONETRAP.load(Ordering::Relaxed) != 0;
        if !already_done {
            // c:Src/signals.c:1245 dotrap(SIGZERR) — canonical ZERR
            // trap dispatch. Fires whenever a command exits
            // non-zero.
            let _ = crate::ported::signals::dotrap(crate::ported::signals_h::SIGZERR);
            // c:1602 — `donetrap = 1;` after firing.
            crate::ported::exec::DONETRAP.store(1, Ordering::Relaxed);
        }
        // c:Src/exec.c:1605-1610 — compute errreturn / errexit.
        //   errreturn = ERRRETURN && (INTERACTIVE || locallevel || sourcelevel)
        //               && !(noerrexit & NOERREXIT_RETURN)
        //   errexit   = (ERREXIT || (ERRRETURN && !errreturn))
        //               && !(noerrexit & NOERREXIT_EXIT)
        let no_err = crate::ported::exec::noerrexit.load(Ordering::Relaxed);
        let locallvl = crate::ported::params::locallevel.load(Ordering::Relaxed);
        let sourcelvl = crate::ported::init::sourcelevel.load(Ordering::Relaxed);
        let errreturn_opt = isset(crate::ported::zsh_h::ERRRETURN);
        let in_unwindable_scope = isset(crate::ported::zsh_h::INTERACTIVE)
            || locallvl != 0
            || sourcelvl != 0;
        let errreturn = errreturn_opt
            && in_unwindable_scope
            && (no_err & crate::ported::zsh_h::NOERREXIT_RETURN) == 0;
        if errreturn {
            // c:1620-1623 — `retflag = 1; breaks = loops;` — unwind to
            // function boundary without exiting the shell.
            crate::ported::builtin::RETFLAG.store(1, Ordering::Relaxed);
            let loops = crate::ported::builtin::LOOPS.load(Ordering::Relaxed);
            crate::ported::builtin::BREAKS.store(loops, Ordering::Relaxed);
            return Value::Int(1);
        }
        let (errexit_on, in_subshell) = with_executor(|exec| {
            let on_canonical = isset(ERREXIT)
                || (errreturn_opt && !errreturn); // c:1608-1609
            let on_legacy = opt_state_get("errexit").unwrap_or(false);
            (
                (on_canonical || on_legacy)
                    && (no_err & crate::ported::zsh_h::NOERREXIT_EXIT) == 0,
                !exec.subshell_snapshots.is_empty(),
            )
        });
        if !errexit_on {
            return Value::Int(0);
        }
        // c:Src/exec.c — `set -e` fires shell exit, NOT a function-only
        // unwind. When LOCAL_OPTIONS restores the option mid-fn, the
        // restoration would otherwise mask the trigger and let the
        // outer scope continue. Setting EXIT_PENDING + EXIT_VAL here
        // (for ALL scope kinds, not just subshells) makes the fn-exit
        // path propagate to the shell-exit boundary at c:6135-6155.
        crate::ported::builtin::EXIT_VAL.store(last, Ordering::Relaxed);
        crate::ported::builtin::EXIT_PENDING.store(1, Ordering::Relaxed);
        let _ = in_subshell;
        // Function scope and top-level scope both branch to their
        // respective return_patches; top-level lands at chunk-end,
        // so execute_script returns `last` as the script's exit
        // status (same observable behavior as a process::exit).
        Value::Int(1)
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
        // op=8 is the `${+name}` set-test prefix form (distinct from the
        // `${name+rhs}` substitute-if-set suffix form which is op=7).
        // Per compile_zsh.rs::parse_param_modifier: the `+` is emitted as
        // a leading sigil and `rhs` is empty.
        let body = if op == 8 {
            format!("${{+{}}}", name)
        } else {
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
            format!("${{{}{}{}}}", name, op_str, rhs)
        };
        paramsubst_to_value(&body)
    });

    // `${var:offset[:length]}` — substring. Pops [name, offset, length].
    // length == -1 means "rest of string". Negative offset counts from end.
    // BUILTIN_PARAM_SUBSTRING — `${var:offset:length}` literal-int form.
    // PURE PASSTHRU: reconstruct `${name:offset:length}` and route
    // through `subst::paramsubst`. Length sentinel `i64::MIN` =
    // "no length given" (omit the `:length` portion).
    //
    // c:Src/subst.c:1571,3781 — `${name:-N}` is the colon-default
    // operator, NOT a substring with negative offset. zsh's lexical
    // rule disambiguates via a literal space: `${name: -N}` (space
    // before `-`) is the substring form. The reconstructed body MUST
    // preserve that space when offset < 0; otherwise paramsubst's
    // `:-` dispatch fires on the synthesized `${name:-N}` body and
    // returns N as the unset-default instead of slicing the last N
    // chars. Length-form `${name:-N:M}` has the same trap.
    vm.register_builtin(BUILTIN_PARAM_SUBSTRING, |vm, _argc| {
        let length = vm.pop().to_int();
        let offset = vm.pop().to_int();
        let name = vm.pop().to_str();
        let off_sep = if offset < 0 { " " } else { "" };
        let body = if length == i64::MIN {
            format!("${{{}:{}{}}}", name, off_sep, offset)
        } else {
            format!("${{{}:{}{}:{}}}", name, off_sep, offset, length)
        };
        paramsubst_to_value(&body)
    });

    // BUILTIN_PARAM_SUBSTRING_EXPR — `${var:offset_expr[:length_expr]}` form.
    // PURE PASSTHRU: rebuild `${name:offset:length}` using the
    // expression text verbatim (paramsubst's offset/length
    // parser evaluates arith / param refs itself).
    //
    // c:Src/subst.c:1571,3781 — same `:-` disambiguation trap as
    // BUILTIN_PARAM_SUBSTRING. The expression text may itself start
    // with `-` (e.g. `${VAR:$((-1))}` arith resolves at the body-
    // assembly layer in some upstream paths, leaving `-1` in
    // off_expr). Insert a leading space when off_expr starts with
    // `-` so paramsubst's check_colon_subscript (subst.c:1571)
    // accepts the operand as a math expression instead of the
    // `:-` operator catching it.
    vm.register_builtin(BUILTIN_PARAM_SUBSTRING_EXPR, |vm, _argc| {
        let has_len = vm.pop().to_int() != 0;
        let len_expr = vm.pop().to_str();
        let off_expr = vm.pop().to_str();
        let name = vm.pop().to_str();
        let off_sep = if off_expr.starts_with('-') { " " } else { "" };
        let body = if has_len {
            format!("${{{}:{}{}:{}}}", name, off_sep, off_expr, len_expr)
        } else {
            format!("${{{}:{}{}}}", name, off_sep, off_expr)
        };
        paramsubst_to_value(&body)
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
        paramsubst_to_value(&body)
    });

    // `$((expr))` — pops [expr_string], evaluates via MathEval which
    // honors integer-vs-float distinction (zsh-compatible). Returns
    // the result as Value::Str so it can be Concat'd into surrounding
    // word context.
    vm.register_builtin(BUILTIN_ARITH_EVAL, |vm, _argc| {
        // Pure path: evaluate expr, return string. errflag may be
        // set by arithsubst on math error; the caller decides
        // whether to clear it. For `(( ... ))` (math command) the
        // compile_arith path clears via BUILTIN_ARITH_CMD_FINISH;
        // for `$((... ))` (substitution inside another command)
        // errflag stays set so the surrounding command aborts —
        // matches c:Src/math.c "math errors propagate as errflag
        // through the containing word expansion".
        let expr = vm.pop().to_str();
        let result = crate::ported::subst::arithsubst(&expr, "", "");
        let _ = vm; // silence unused warning when no math error path mutates
        Value::str(result)
    });

    // After-call hook used by compile_arith's `(( ... ))` path: when
    // arithsubst set errflag (math error), clear it and signal
    // status=2 in vm.last_status — matches zsh's c:exec.c arith-
    // failure: the math command exits 2 and the script continues.
    vm.register_builtin(BUILTIN_ARITH_CMD_FINISH, |vm, _argc| {
        use std::sync::atomic::Ordering;
        let live = crate::ported::utils::errflag.load(Ordering::Relaxed);
        let err = live & crate::ported::zsh_h::ERRFLAG_ERROR;
        let hard = live & crate::ported::zsh_h::ERRFLAG_HARD;
        if err != 0 {
            // c:Src/subst.c:3344 — when `${var:?msg}` fires, errflag
            // is OR'd with ERRFLAG_HARD to signal a script-abort
            // error (vs a recoverable math error like `$((1/0))`).
            // Clear only the ERRFLAG_ERROR bit; preserve
            // ERRFLAG_HARD so the next ERREXIT_CHECK aborts the
            // script. Bug #193 in docs/BUGS.md.
            if hard != 0 {
                // Keep ERRFLAG_HARD AND ERRFLAG_ERROR set so the
                // script-abort gate downstream still fires.
                vm.last_status = 2;
                Value::Status(2)
            } else {
                crate::ported::utils::errflag
                    .fetch_and(!crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
                vm.last_status = 2;
                Value::Status(2)
            }
        } else {
            Value::Status(vm.last_status)
        }
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
        Value::str(result)
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
        with_executor(|exec| exec.set_last_status(live_status));
        let result_value = with_executor(|exec| match mode {
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
                // The lexer's dquote_parse (Src/lex.c) already tokenized
                // DQ contents: `$` → Qstring (\u{8c}), `\$`/`\\`/`\"`/
                // `` \` `` → Bnull (\u{9f}) + literal. Stringsubst /
                // multsub recognize these markers natively. We pass
                // `inner` through verbatim — no re-tokenization needed.
                let prepped: String = inner.to_string();
                // Tell parameter-flag application that we're inside
                // double quotes — array-only flags ((o), (O), (n),
                // (i), (M), (u)) must be no-ops here per zsh.
                exec.in_dq_context += 1;
                if mode == 5 {
                    exec.in_scalar_assign += 1;
                }
                // Mode 1 = argv DQ word; mode 5 = scalar-assign RHS.
                // In C zsh, the corresponding prefork-on-list paths
                // are: argv → `prefork(argv_list, 0)` returns multi-
                // word LinkList (Src/exec.c::execcmd), assignment →
                // `prefork(rhs_list, PREFORK_SINGLE|PREFORK_ASSIGN)`
                // returns single-word (Src/exec.c::addvars line
                // 2546). zshrs's `multsub` (Src/subst.c:544) is the
                // multi-result variant; `singsub` (Src/subst.c:514)
                // asserts ≤1 node. Mode 5 keeps singsub; mode 1
                // switches to multsub so `"${(@)arr}"`/`"$@"`/
                // `"${arr[@]}"` in argv context emit multiple words
                // as the C path would.
                let result_value = if mode == 5 {
                    let out = crate::ported::subst::singsub(&prepped);
                    Value::str(out)
                } else {
                    let (_first, nodes, _ms_ws, _ret) = crate::ported::subst::multsub(&prepped, 0);
                    // c:Src/subst.c:655 — multsub returns Vec::new()
                    // for zero-word results (quoted array splat that
                    // resolved to empty array). Surface as
                    // Value::Array(vec![]) so the downstream array
                    // assignment / argv flattening sees ZERO args.
                    // Previous Rust port returned Value::str("") which
                    // surfaced as ONE empty arg. Bug #120 in
                    // docs/BUGS.md.
                    if nodes.is_empty() {
                        Value::Array(Vec::new())
                    } else if nodes.len() == 1 {
                        Value::str(nodes.into_iter().next().unwrap())
                    } else {
                        Value::Array(nodes.into_iter().map(Value::str).collect())
                    }
                };
                if mode == 5 {
                    exec.in_scalar_assign -= 1;
                }
                exec.in_dq_context -= 1;
                result_value
            }
            2 => {
                // SingleQuoted: pure literal, strip outer `'…'`.
                let inner = if text.len() >= 2 && text.starts_with('\'') && text.ends_with('\'') {
                    &text[1..text.len() - 1]
                } else {
                    text.as_str()
                };
                Value::str(inner.to_string())
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
                    Value::str(trimmed.to_string())
                } else {
                    let ifs = exec.scalar("IFS").unwrap_or_else(|| " \t\n".to_string());
                    let parts: Vec<Value> = trimmed
                        .split(|c: char| ifs.contains(c))
                        .filter(|s| !s.is_empty())
                        .map(|s| Value::str(s.to_string()))
                        .collect();
                    if parts.is_empty() {
                        Value::str(String::new())
                    } else if parts.len() == 1 {
                        parts.into_iter().next().unwrap()
                    } else {
                        Value::Array(parts)
                    }
                }
            }
            4 => {
                // HeredocBody: expand variables / command-subst / arith
                // but NOT glob or brace. Heredoc lines like `[42]` must
                // pass through verbatim — running them through the
                // default pipeline triggers NOMATCH on the literal.
                Value::str(crate::ported::subst::singsub(&text))
            }
            _ => {
                // Default (unquoted): the lexer's gettokstr already
                // tokenized backslash-escapes (`\$` → Bnull+$, etc).
                // Pass `text` through verbatim — multsub/stringsubst
                // recognize the markers natively. No bridge-side
                // re-tokenization needed.
                //
                // Mode 6 = unquoted RHS in scalar-assign context.
                // Pass PREFORK_ASSIGN so prefork's filesub colon-walk
                // fires per c:Src/exec.c:2546.
                let prepped: String = text.clone();
                if std::env::var("ZSHRS_TRACE_DEFP").is_ok() {
                    eprintln!(
                        "[TRACE_DEFP] text={:?} prepped={:?} mode={}",
                        text, prepped, mode
                    );
                }
                let pf_flags = if mode == 6 {
                    crate::ported::zsh_h::PREFORK_ASSIGN
                } else {
                    0
                };
                // c:Src/subst.c:544+ — `multsub(&prepped, 0)` is the
                // unquoted-argv equivalent of zsh's `prefork(list,
                // 0, NULL)` for a single-element list. Returns the
                // post-expansion node list (Vec<String>) so array-
                // shape results (e.g. `${a:e}`, `${a[@]}`,
                // `${(s::)str}`) splat into multiple argv words.
                // singsub() collapses to one string and discards the
                // splat — parity bug #28 (whole-array modifier).
                let (_first, nodes, _ms_ws, _ret) =
                    crate::ported::subst::multsub(&prepped, pf_flags);
                if std::env::var("ZSHRS_TRACE_MULTSUB").is_ok() {
                    eprintln!("[TRACE_MULTSUB] prepped={:?} nodes={:?}", prepped, nodes);
                }
                // c:Src/subst.c:166 — xpandbraces runs AFTER prefork's
                // substitution pass and BEFORE untokenize/glob. Per
                // word, scan for Inbrace TOKEN and expand. Words that
                // don't contain Inbrace TOKEN pass through unchanged.
                // Brace expansion is done here (inside the bridge
                // default arm) instead of via a post-EXPAND_TEXT
                // BRACE_EXPAND emit because untokenize (line below)
                // strips TOKEN bytes, after which the strict-TOKEN
                // xpandbraces gate would no longer match.
                let brace_ccl = opt_state_get("braceccl").unwrap_or(false);
                // c:Src/options.c — `no_brace_expand` (negated
                // `braceexpand`) gates brace expansion entirely.
                // When off, `{a,b}` stays literal.
                let brace_expand = opt_state_get("braceexpand").unwrap_or(true);
                let pre_brace: Vec<String> = if nodes.is_empty() {
                    vec![String::new()]
                } else {
                    nodes
                };
                let brace_expanded: Vec<String> = pre_brace
                    .into_iter()
                    .flat_map(|w| {
                        if brace_expand && w.contains('\u{8f}') {
                            crate::ported::glob::xpandbraces(&w, brace_ccl)
                        } else {
                            vec![w]
                        }
                    })
                    .collect();
                // zsh stores the option as `glob` (default ON);
                // `setopt noglob` writes `glob=false`. Honor either
                // form so the dispatcher behaves the same as zsh.
                let noglob = opt_state_get("noglob").unwrap_or(false)
                    || opt_state_get("GLOB").map(|v| !v).unwrap_or(false)
                    || !opt_state_get("glob").unwrap_or(true);
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
                        // c:Src/pattern.c:4306 haswilds — token-only
                        // gate matching C verbatim. C's haswilds checks
                        // ONLY the META-TOKEN codes (Inpar `\u{88}`,
                        // Bar `\u{89}`, Star `\u{87}`, Inbrack `\u{91}`,
                        // Inang `\u{94}`, Quest `\u{86}`, Pound `\u{84}`
                        // /EXTENDEDGLOB, Hat `\u{8a}`/EXTENDEDGLOB),
                        // never their literal ASCII counterparts. The
                        // lexer tokenizes source-level `[abc]` →
                        // Inbrack/Outbrack, source-level `*.toml` → Star
                        // — those reach haswilds with tokens and fire.
                        // Bare literal `[`/`*`/`?` from `$'...'` decode,
                        // `:-` default values, or variable expansion
                        // never get shtokenize'd (C subst.c:3231 sets
                        // globsubst=0 in the `:-` arm) so haswilds
                        // skips them. Bug #625: `${${X}:-$'\e[hi]'}`
                        // returned bare literal `[hi]` from the nested
                        // paramsubst; the previous post-untokenize
                        // haswilds saw literal `[` and globbed → NOMATCH
                        // fired. The TOKEN-only check has to happen
                        // PRE-untokenize so source-level Star tokens
                        // (`*.toml`) survive while substituted bare
                        // glob chars stay literal.
                        let is_glob_pre = if noglob {
                            false
                        } else {
                            use crate::ported::zsh_h::{
                                isset, Bar, Bang, EXTENDEDGLOB, Hat, Inang, Inbrack, Inpar,
                                KSHGLOB, Outbrack, Pound, Quest, SHGLOB, Star,
                            };
                            // c:Src/pattern.c:4310-4312 — single-byte
                            // bare Inbrack/Outbrack is legal pattern.
                            let bytes = s.as_bytes();
                            let len = bytes.len();
                            let single_bracket = len == 1
                                && (bytes[0] == Inbrack as u8 || bytes[0] == Outbrack as u8);
                            // c:4317-4318 — `%?foo` job-ref skip.
                            let skip_pos_1 = len >= 2
                                && bytes[0] == b'%'
                                && bytes[1] == Quest as u8;
                            let mut found = false;
                            if !single_bracket {
                                let disp = crate::ported::pattern::zpc_disables
                                    .lock()
                                    .unwrap();
                                for i in 0..len {
                                    if skip_pos_1 && i == 1 {
                                        continue;
                                    }
                                    let b = bytes[i];
                                    // c:4326-4335 Inpar — KSHGLOB
                                    // prev-char gating mirrors C.
                                    if b == Inpar as u8 {
                                        let prev = if i > 0 { bytes[i - 1] } else { 0 };
                                        if (!isset(SHGLOB)
                                            && disp[crate::ported::zsh_h::ZPC_INPAR as usize] == 0)
                                            || (i > 0
                                                && isset(KSHGLOB)
                                                && ((prev == Quest as u8
                                                    && disp[crate::ported::zsh_h::ZPC_KSH_QUEST as usize] == 0)
                                                    || (prev == Star as u8
                                                        && disp[crate::ported::zsh_h::ZPC_KSH_STAR as usize] == 0)
                                                    || (prev == b'+'
                                                        && disp[crate::ported::zsh_h::ZPC_KSH_PLUS as usize] == 0)
                                                    || (prev == Bang as u8
                                                        && disp[crate::ported::zsh_h::ZPC_KSH_BANG as usize] == 0)
                                                    || (prev == b'!'
                                                        && disp[crate::ported::zsh_h::ZPC_KSH_BANG2 as usize] == 0)
                                                    || (prev == b'@'
                                                        && disp[crate::ported::zsh_h::ZPC_KSH_AT as usize] == 0)))
                                        {
                                            found = true;
                                            break;
                                        }
                                    } else if b == Bar as u8 {
                                        if disp[crate::ported::zsh_h::ZPC_BAR as usize] == 0 {
                                            found = true;
                                            break;
                                        }
                                    } else if b == Star as u8 {
                                        if disp[crate::ported::zsh_h::ZPC_STAR as usize] == 0 {
                                            found = true;
                                            break;
                                        }
                                    } else if b == Inbrack as u8 {
                                        if disp[crate::ported::zsh_h::ZPC_INBRACK as usize] == 0 {
                                            found = true;
                                            break;
                                        }
                                    } else if b == Inang as u8 {
                                        if disp[crate::ported::zsh_h::ZPC_INANG as usize] == 0 {
                                            found = true;
                                            break;
                                        }
                                    } else if b == Quest as u8 {
                                        if disp[crate::ported::zsh_h::ZPC_QUEST as usize] == 0 {
                                            found = true;
                                            break;
                                        }
                                    } else if b == Pound as u8 {
                                        if isset(EXTENDEDGLOB)
                                            && disp[crate::ported::zsh_h::ZPC_HASH as usize] == 0
                                        {
                                            found = true;
                                            break;
                                        }
                                    } else if b == Hat as u8 {
                                        if isset(EXTENDEDGLOB)
                                            && disp[crate::ported::zsh_h::ZPC_HAT as usize] == 0
                                        {
                                            found = true;
                                            break;
                                        }
                                    }
                                }
                            }
                            found
                        };
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
                        // Glob-trigger decision: pre-untokenize
                        // haswilds_tokens_only result (computed above
                        // before the untokenize that collapses META
                        // tokens to their ASCII forms). The TOKEN-only
                        // gate matches C `Src/pattern.c:4306-4376`
                        // exactly — only Inbrack/Star/Quest/Inpar/Bar/
                        // Inang/Pound/Hat token codes count as wild,
                        // not their literal ASCII counterparts. Source-
                        // level `*.toml` carries Star token so globs;
                        // `$'…'`-decoded `[abc]` carries bare `[` so
                        // stays literal. Bug #625.
                        if is_glob_pre && !is_assignment_shape {
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
                        Value::Array(Vec::new())
                    } else {
                        Value::str(only)
                    }
                } else {
                    Value::Array(parts.into_iter().map(Value::str).collect())
                }
            }
        });
        // Pull any inner cmd-subst (`` `cmd` `` via mode 3 or via
        // mode 0/6 multsub → getoutput, `$(cmd)` via the default
        // arm's multsub path, nested `$()`s reached through
        // stringsubst) back into vm.last_status so a containing
        // assignment's BUILTIN_SET_VAR — which reads vm.last_status —
        // sees the cmd-subst's exit. Without this, backtick
        // assignments (`a=\`false\`; echo $?`) reported 0 because the
        // ported LASTVAL update never reached the VM-side counter.
        let cs_status = with_executor(|exec| exec.last_status());
        vm.last_status = cs_status;
        result_value
    });

    // `${#name}` — pops [name]. Returns the value's element count for
    // arrays (indexed and assoc) or character length for scalars.
    // BUILTIN_PARAM_LENGTH — `${#name}`. PURE PASSTHRU.
    vm.register_builtin(BUILTIN_PARAM_LENGTH, |vm, _argc| {
        let name = vm.pop().to_str();
        // PARAM_LENGTH's empty-result semantics differ from
        // paramsubst_to_value: 0 nodes → "0" (numeric length), not
        // empty array. paramsubst on `${#X}` always returns at least
        // one node in practice (the length string); the empty case
        // is defensive.
        let mut ret_flags: i32 = 0;
        let (_full, _pos, nodes) = crate::ported::subst::paramsubst(
            &format!("${{#{}}}", name),
            0,
            false,
            0i32,
            &mut ret_flags,
        );
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            with_executor(|exec| exec.set_last_status(1));
        }
        if nodes.is_empty() {
            Value::str("0")
        } else {
            nodes_to_value(nodes)
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
        paramsubst_to_value(&body)
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
                // c:Src/exec.c:5383 — `shf->filename =
                // ztrdup(scriptfilename);` — the function's
                // definition-file is read from the canonical
                // file-scope `scriptfilename` global at compile
                // time, NOT from a per-executor struct field.
                // exec.scriptfilename is seeded once at
                // bins/zshrs.rs:1717 to the bin basename ("zsh")
                // and never updates on source/dot, so reading from
                // it left every user function's def_file as "zsh".
                // Route through scriptfilename_get() so source /
                // dot's set_scriptfilename calls propagate.
                let def_file = crate::ported::utils::scriptfilename_get()
                    .or_else(|| exec.scriptfilename.clone());
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
                    let mut shf = crate::ported::hashtable::shfunc_with_body(&name, &body_source);
                    // c:Src/exec.c:5409 — `shf->lineno = lineno;`. Use
                    // the same max(1, line_base) clamp as the synth_shf
                    // in vm_helper::dispatch_function_call. Bug #396.
                    shf.lineno = std::cmp::max(1, line_base);
                    tab.add(shf);
                }
                // c:Src/exec.c:5460-5475 — `TRAP<SIG>() { ... }` is the
                // function-named trap install. zsh detects the `TRAP`
                // prefix at func-def time and calls
                // `settrap(signum, NULL, ZSIG_FUNC)` so the next
                // dispatch of that signal routes to the named shfunc.
                // Bug #157 in docs/BUGS.md — fusevm_bridge's funcdef
                // opcode skipped this dispatch entirely, so TRAPEXIT /
                // TRAPUSR1 / TRAPZERR / TRAPDEBUG never fired.
                if name.len() > 4 && name.starts_with("TRAP") {
                    if let Some(sn) = crate::ported::jobs::getsigidx(&name[4..]) {
                        let _ = crate::ported::signals::settrap(
                            sn,
                            None,
                            crate::ported::zsh_h::ZSIG_FUNC as i32,
                        );
                    }
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

/// Run `body` through `crate::ported::subst::paramsubst` and convert
/// the resulting node list into a fusevm `Value`. Centralises the
/// pattern duplicated across ~10 BUILTIN_* handlers:
///   - build a `${...}` body string from opcode operands
///   - paramsubst the body
///   - propagate errflag to `exec.last_status`
///   - delegate the LinkList → Value conversion to `nodes_to_value`
///
/// **Extension** — Rust-only helper. No direct C analog because C
/// zsh uses LinkList everywhere; the conversion happens at the
/// boundary back into the VM's stack.
fn paramsubst_to_value(body: &str) -> Value {
    // c:Src/subst.c:1625 paramsubst's `qt` flag is the C signal that
    // the current expansion is inside `"…"`. The fast-path bridges
    // (BUILTIN_PARAM_*, BUILTIN_BRIDGE_BRACE_ARRAY) used to hardcode
    // qt=false, which silently broke DQ-only semantics inside
    // `${arr:^other}` / `${arr:^^other}` (Src/subst.c:3456-3520).
    // The executor's `in_dq_context` counter is bumped by EXPAND_TEXT
    // mode 1 / mode 5 before the bridge fires, so reading it here
    // propagates the DQ flag without changing every bridge call site.
    let qt = with_executor(|exec| exec.in_dq_context > 0);
    let mut ret_flags: i32 = 0;
    let (_full, _pos, nodes) = crate::ported::subst::paramsubst(body, 0, qt, 0i32, &mut ret_flags);
    if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
        with_executor(|exec| exec.set_last_status(1));
    }
    nodes_to_value(nodes)
}

/// Wrap a `Vec<String>` (e.g. paramsubst nodes, multsub parts,
/// xpandbraces output) into a fusevm `Value`: 0 → empty Array, 1 →
/// Str, >1 → Array. Same unwrap idiom every handler that calls a
/// canonical Vec-returning fn does.
fn nodes_to_value(nodes: Vec<String>) -> Value {
    // c:Src/glob.c:3649 remnulargs — strip the Nularg (`\u{a1}`)
    //   sentinel and other INULL bytes that paramsubst's splat block
    //   emits for empty array elements (so prefork's empty-node-delete
    //   pass doesn't drop them). Downstream consumers (cond `-z`/`-n`,
    //   command args, etc.) must see the post-remnulargs strings. Bug
    //   #185 in docs/BUGS.md: `[[ -z "${b[@]}" ]]` for b=("") returned
    //   false because the leftover `\u{a1}` had StringLen=1.
    let stripped: Vec<String> = nodes
        .into_iter()
        .map(|mut s| {
            crate::ported::glob::remnulargs(&mut s);
            s
        })
        .collect();
    if stripped.is_empty() {
        Value::Array(Vec::new())
    } else if stripped.len() == 1 {
        let only = stripped.into_iter().next().unwrap();
        // c:Src/subst.c:183-186 — `else if (!(flags & PREFORK_SINGLE)
        // && !(*ret_flags & PREFORK_KEY_VALUE) && !keep)
        //   uremnode(list, node);`
        // C zsh's prefork removes empty linknodes from the result
        // list when in non-SINGLE (argv-context) mode. The ported
        // prefork at subst.rs:388-396 honors the same delete-empty
        // pass, but some paramsubst paths land here with a single-
        // empty-string Vec instead of an empty Vec (paramsubst's
        // slice / substring / parameter-flag branches allocate a
        // result before checking emptiness). Mirror the prefork
        // drop at this layer: single-empty under !in_dq_context
        // collapses to Value::Array(empty), and pop_args (line 6243)
        // splats the empty Array → zero argv words. DQ context
        // (in_dq_context > 0) keeps the empty string so
        // `echo "${UNSET}"` still produces an empty arg per zsh's
        // quoting rules (c:Src/subst.c:1650-1656 isarr comment).
        if only.is_empty() {
            let in_dq = with_executor(|exec| exec.in_dq_context > 0);
            if !in_dq {
                return Value::Array(Vec::new());
            }
        }
        Value::str(only)
    } else {
        Value::Array(stripped.into_iter().map(Value::str).collect())
    }
}

fn pop_args(vm: &mut fusevm::VM, argc: u8) -> Vec<String> {
    let mut popped: Vec<Value> = Vec::with_capacity(argc as usize);
    for _ in 0..argc {
        popped.push(vm.pop());
    }
    popped.reverse();
    let mut args: Vec<String> = Vec::with_capacity(popped.len());
    for v in popped {
        match v {
            Value::Array(items) => {
                for item in items {
                    args.push(item.to_str());
                }
            }
            other => args.push(other.to_str()),
        }
    }
    // `expand_glob` set the glob-failed cell when a no-match glob
    // triggered nomatch (c:Src/glob.c:1877). Signal the failure via
    // last_status + the per-command glob_failed cell; the dispatcher
    // (`host_exec_external`) consumes + clears it and returns status 1
    // without running the command body.
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
/// Fork+exec a system binary by name. Used by `reg_overridable!` as
/// the fall-through path when `[builtins].coreutils_shadows = off`
/// (the default) — runs the canonical `/bin/X` instead of zshrs's
/// in-process shadow so old scripts hit zero behavioral divergence.
///
/// Inherits stdin/stdout/stderr from the parent so pipelines work
/// transparently. Resolves the binary via PATH; mirrors what zsh's
/// own external-command dispatch would do. Returns the child's exit
/// status (or 127 if PATH lookup fails — the standard "command not
/// found" code).
fn exec_system_command(name: &str, args: &[String]) -> i32 {
    let status = std::process::Command::new(name)
        .args(args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();
    match status {
        Ok(s) => s.code().unwrap_or(if s.success() { 0 } else { 1 }),
        Err(e) => {
            eprintln!("zshrs: {}: {}", name, e);
            127
        }
    }
}

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

/// Builtin ID for `${name}` reads — routes through canonical
/// `getsparam` (Src/params.c:3076) via paramtab + env walk so nested
/// VMs (function calls) see the same storage.
pub const BUILTIN_GET_VAR: u16 = 283;

/// Builtin ID for `name=value` assignments — pops [name, value] and
/// routes through canonical `setsparam` (Src/params.c:3350).
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

/// `break` from inside a body that runs on a sub-VM (select, future
/// loop-via-builtin constructs). Writes the canonical
/// `crate::ported::builtin::BREAKS` atomic (port of `Src/loop.c:46
/// breaks`). Outer-loop builtins drain BREAKS/CONTFLAG after each
/// body run, matching the loop.c:529-534 drain pattern.
pub const BUILTIN_SET_BREAK: u16 = 299;

/// `continue` from inside a sub-VM body. Sets CONTFLAG=1 + bumps
/// BREAKS, matching `bin_break`'s WC_CONTINUE arm at Src/builtin.c
/// c:5836 `contflag = 1; FALLTHROUGH; breaks++;`.
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
/// `BUILTIN_VAR_EXISTS` constant.
pub const BUILTIN_VAR_EXISTS: u16 = 306;
/// Native param-modifier builtins. Each takes a fixed argv shape and
/// returns the modified value as Value::Str.
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
/// `(( ... ))` math command post-eval status hook. Pops nothing,
/// pushes Value::Status. If errflag is set (math error in the
/// preceding BUILTIN_ARITH_EVAL call), clears it and emits status=2
/// matching c:Src/math.c arith-failure semantics. Otherwise emits
/// the current vm.last_status. Used by compile_arith's `(( ... ))`
/// path so the math command swallows errors without halting the
/// script — `$((... ))` substitutions skip this hook so their
/// errflag propagates up to the containing command.
pub const BUILTIN_ARITH_CMD_FINISH: u16 = 527;
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
/// `BUILTIN_ERREXIT_CHECK` constant.
pub const BUILTIN_ERREXIT_CHECK: u16 = 336;
/// Post-`always`-arm checks for the canonical RETFLAG / BREAKS /
/// CONTFLAG atomics that mark try-block escapes. Each returns
/// Value::Int(1) when the corresponding atomic is set (and consumes
/// it so the next escape doesn't re-fire) and Value::Int(0) otherwise.
/// Paired with JumpIfFalse + Jump to outer return_patches /
/// break_patches / continue_patches by compile_zsh's `Try` arm.
pub const BUILTIN_RETFLAG_CHECK: u16 = 600;
/// `BUILTIN_BREAKS_CHECK` constant.
pub const BUILTIN_BREAKS_CHECK: u16 = 601;
/// `BUILTIN_CONTFLAG_CHECK` constant.
pub const BUILTIN_CONTFLAG_CHECK: u16 = 602;
/// Fire the DEBUG trap (SIGDEBUG) before each statement.
/// c:Src/exec.c:1357-1500 DEBUGBEFORECMD — when a "DEBUG" entry is
/// installed via `trap '...' DEBUG`, run the body just before the
/// next command. Cheap when no DEBUG trap is set (one hashmap lookup
/// returns None and we early-out).
pub const BUILTIN_DEBUG_TRAP: u16 = 603;
/// `set -n` / `set -o noexec` — parse but don't execute. Returns
/// Value::Int(1) when the noexec option is set so the caller's
/// JumpIfTrue skips the statement body. c:Src/exec.c:1390 main loop
/// check.
pub const BUILTIN_NOEXEC_CHECK: u16 = 604;
/// Block-level redirect-failure gate. Reads exec.redirect_failed
/// (set by host.redirect when a redirect open fails); returns
/// Value::Int(1) AND clears the flag if set, else 0. Emit-side at
/// compile_zsh.rs::compile_command's Redirected arm pairs with a
/// JumpIfTrue → WithRedirectsEnd to abandon the body. Without this,
/// a multi-statement block after a failed redir kept running every
/// statement after the first (the first builtin consumed the flag,
/// subsequent statements ran unimpeded).
pub const BUILTIN_REDIRECT_FAILED_CHECK: u16 = 605;
/// Drop-in replacement for fusevm's Op::Exec for the dynamic-first-
/// word path (`$cmd`, `$(cmd)`, `~/bin/foo`). Returns
/// Value::Status(vm.last_status) when post-expansion argv is empty
/// (preserves the inner cmd-subst's exit), Value::Status(126) with
/// "permission denied" when `argv[0]` is empty, otherwise routes
/// through executor.host_exec_external like Op::Exec did.
pub const BUILTIN_EXEC_DYNAMIC: u16 = 606;
/// `< file` / `> file` with no command word (NULLCMD path).
/// Resolves NULLCMD (default "cat") / READNULLCMD (default "more")
/// at runtime per Src/exec.c:3340-3364 and exec's it through
/// host_exec_external. Argc is 1: the int (0 or 1) on the stack
/// indicates whether this is a single REDIR_READ redirect
/// (selects READNULLCMD when set + non-empty).
pub const BUILTIN_NULLCMD_EXEC: u16 = 607;
/// `.` (dot) — alias of source/bin_dot but dispatches with the
/// literal name "." so the diagnostic prefix matches zsh's
/// (`zsh:.:1: …` vs source's `zsh:source:1: …`).
/// c:Src/builtin.c:9308 — `BUILTIN(".", BINF_PSPECIAL, bin_dot, …)`.
pub const BUILTIN_DOT: u16 = 608;
/// `logout` — fusevm maps this to BUILTIN_EXIT alongside `exit`/`bye`,
/// which drops the name and dispatches with BIN_EXIT funcid. zsh's
/// `logout` outside a login shell must emit "not login shell" + exit 1,
/// which only fires when bin_break sees BIN_LOGOUT funcid. Dedicated
/// opcode dispatches via BUILTINS table by literal name "logout".
pub const BUILTIN_LOGOUT: u16 = 610;
/// `BUILTIN_PARAM_SUBSTRING_EXPR` constant.
pub const BUILTIN_PARAM_SUBSTRING_EXPR: u16 = 337;
/// `BUILTIN_XTRACE_LINE` constant.
pub const BUILTIN_XTRACE_LINE: u16 = 338;
/// `BUILTIN_ARRAY_JOIN_STAR` constant.
pub const BUILTIN_ARRAY_JOIN_STAR: u16 = 339;
/// `BUILTIN_SET_RAW_OPT` constant.
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
/// `BUILTIN_RESTORE_TRY_BLOCK_STATUS` constant.
pub const BUILTIN_RESTORE_TRY_BLOCK_STATUS: u16 = 432;
/// `BUILTIN_BEGIN_INLINE_ENV` constant.
pub const BUILTIN_BEGIN_INLINE_ENV: u16 = 433;
/// `BUILTIN_END_INLINE_ENV` constant.
pub const BUILTIN_END_INLINE_ENV: u16 = 434;

/// `[[ -o option ]]` — shell-option-set test. Stack: \[option_name\].
/// Normalizes the name (strip underscores, lowercase) and reads
/// `exec.options`. Pushes Bool.
pub const BUILTIN_OPTION_SET: u16 = 321;
/// Tri-state `[[ -o NAME ]]` — same lookup as BUILTIN_OPTION_SET
/// but returns a Value::Int (0=set, 1=unset, 3=invalid-name). The
/// 3-state code matches zsh's `[[ -o invalid ]]` exit (Src/cond.c
/// :502 `optison()`). Used by compile_cond's `-o` arm to skip the
/// generic bool→status conversion and preserve the invalid-name
/// signal in `$?`.
pub const BUILTIN_OPTION_CHECK_TRISTATE: u16 = 609;

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

/// Push the live `xtrace` opt-state as `Value::Int(1)` (on) or
/// `Value::Int(0)` (off). Used by `compile_cond` to gate the
/// trace-string-building block on xtrace state at runtime — without
/// this the trace path's `compile_word_str` on each operand re-
/// evaluates side-effectful expressions (`$((i++))`) once for the
/// trace string and once for the actual condition, doubling the
/// effective increment. Bug #159 in docs/BUGS.md.
///
/// Stack: pushes Int(0|1). argc = 0.
pub const BUILTIN_XTRACE_IS_ON: u16 = 611;

/// Reset the `DONETRAP` flag at the start of each top-level statement
/// (sublist boundary). Mirrors C `Src/exec.c:1455` — `donetrap = 0`.
/// Stack: untouched. argc = 0. Bug #303 in docs/BUGS.md.
pub const BUILTIN_DONETRAP_RESET: u16 = 612;

/// `[[ -z X ]]` / `[[ -n X ]]` operand-empty test that honours zsh's
/// array-splice semantics. C zsh evaluates `[[ -z X ]]` per
/// `Src/cond.c:347` (case 'z'): `s` is the SCALAR operand passed
/// through `cond_str`'s singsub. For `"${arr[@]}"` zsh expands per
/// `Src/subst.c:multsub` which yields each element as its own word
/// list node; cond.c then sees the joined-or-single-element form.
///
/// The compile-side `-z` shortcut at `compile_zsh.rs:5371` used
/// `Op::StringLen` which calls `Value::len` — for `Value::Array`
/// that returns ARRAY LENGTH, not string length. `b=("")` produced
/// `Value::Array([""])` → `len = 1` → `-z` returned false.
///
/// This builtin pops one `Value` and pushes `1` (empty) or `0`
/// (non-empty) per the cond context:
///   - `Value::Str(s)` → s.is_empty()
///   - `Value::Array([])` → true (zero words → vacuous-empty)
///   - `Value::Array([s])` → s.is_empty() (single-word case)
///   - `Value::Array([_; n>=2])` → false (multiple non-empty
///     words; zsh would raise "unknown condition" but the
///     observable test result is non-empty/false)
///
/// Companion to BUILTIN_COND_STR_NONEMPTY (#185 in docs/BUGS.md).
pub const BUILTIN_COND_STR_EMPTY: u16 = 613;

/// `[[ -n X ]]` operand-non-empty test (logical complement of
/// BUILTIN_COND_STR_EMPTY).
pub const BUILTIN_COND_STR_NONEMPTY: u16 = 614;

/// `exec N<<<"str"` — herestring redirect to explicit fd, applied
/// permanently to the shell (no scope restoration). Pops `[content,
/// fd]` from the stack; creates a temp file, writes
/// `content + "\n"`, reopens read-only, dup2's to `fd`, unlinks the
/// temp path so it disappears on close. Mirrors C `Src/exec.c:4655
/// getherestr` + `addfd(forked, save, mfds, fn->fd1, fil, 0, ...)`
/// at c:3766-3780 for the bare-exec-redir code path (nullexec=1).
/// Bug #205 in docs/BUGS.md.
///
/// Stack: pushes `Value::Status(0)` on success, `Status(1)` on
/// failure. argc = 2.
pub const BUILTIN_EXEC_HERESTR_FD: u16 = 615;

/// MULTIOS write/append fan-out for `cmd > a > b` / `cmd > a >> b`
/// style redirects (Bug #36 in docs/BUGS.md). zsh's MULTIOS option
/// (Src/exec.c:2418 `mfds[fd1]` check + addfd splice) creates a
/// pipe at fd1, spawns an internal "tee" process that copies
/// stdin → every collected target file. Without this, only the
/// LAST redirect target survives because each dup2 overwrites the
/// previous binding.
///
/// Stack layout (pushed by compile_zsh's compile_redirs coalescing
/// pass): `[target_1, op_byte_1, target_2, op_byte_2, …, target_N,
/// op_byte_N, fd]`. Pops 2N+1 elements; `argc = 2*N + 1`.
///
/// Runtime:
///   1. Open all targets per their op_byte (WRITE truncate /
///      APPEND).
///   2. Save `dup(fd)` onto the active redirect_scope_stack so
///      `host_redirect_scope_end` restores the original fd.
///   3. Create a pipe; spawn a thread that reads from the pipe
///      read-end and writes every chunk to every opened target.
///   4. dup2 the pipe write-end onto `fd` so the command's writes
///      go through the splitter.
///   5. Track `(pipe_write_fd, JoinHandle)` so scope-end can close
///      the pipe (draining the thread) and join before restoring.
pub const BUILTIN_MULTIOS_REDIRECT: u16 = 617;

/// MULTIOS input-side concatenation for `cmd < a < b` shapes
/// (Bug #36 input arm). C zsh's `Src/exec.c:2418` mfds dispatch
/// also covers the read direction — when multiple `<` redirects
/// target the same fd, mfds[fd] grows and addfd splices a
/// concatenating cat into the pipe.
///
/// Stack layout: `[source_1, source_2, …, source_N, fd]`. Pops
/// N + 1 elements (argc = N + 1). All sources are file paths; the
/// op_byte is implicitly READ.
///
/// Runtime:
///   1. Open every source file for reading.
///   2. Save `dup(fd)` onto the redirect_scope_stack.
///   3. Create a pipe; spawn a thread that reads each source in
///      order and writes every chunk to the pipe write-end. Close
///      write-end when done so the consumer sees EOF.
///   4. dup2 the pipe read-end onto `fd`.
///   5. Track the JoinHandle so scope-end joins (no fd-close needed
///      here — the producer thread closes its own pipe write-end
///      on exit).
pub const BUILTIN_MULTIOS_READ: u16 = 618;

/// `redirection with no command` parse-time error for bare
/// `builtin 2>&1` / `command < file` / `exec >&-` precmd-keyword
/// shapes with a redirect but no following command. Direct port
/// of `Src/exec.c:3342 zerr("redirection with no command")`.
/// argc=0; pushes Value::Status(1).
pub const BUILTIN_REDIR_NO_CMD: u16 = 616;

/// GLOB_SUBST guard for `[[ x == $pat ]]` pattern RHS coming from
/// parameter / command substitution. C-zsh's `[[ == ]]` semantics
/// (Src/options.c GLOB_SUBST default OFF + Src/cond.c:552
/// `cond_match` + Src/pattern.c patcompile tokenization-based
/// meta detection) treat chars from substitution as LITERAL
/// unless GLOB_SUBST is on. The Rust patcompile accepts both
/// tokenized and raw-ASCII meta chars, losing the distinction,
/// so `pat="h*"; [[ hello == $pat ]]` matched in zshrs but not
/// in zsh. Bug #116 in docs/BUGS.md.
///
/// Compile-time signal: emitted by `compile_cond_expr` ONLY when
/// the RHS contains `$` or backtick. Runtime checks the live
/// option state. If GLOB_SUBST is OFF, the popped string has
/// its glob metachars escaped with `\` so the downstream StrMatch
/// → patcompile treats them as literals. If GLOB_SUBST is ON,
/// the value passes through unchanged so `setopt glob_subst`
/// restores zsh's pattern-on-expansion behavior.
///
/// Stack: pops one string, pushes the (possibly escaped) result.
/// argc = 1.
pub const BUILTIN_GLOB_SUBST_GUARD: u16 = 528;

/// Coerce a string parameter value to a math number (Int or Float)
/// for arithmetic-context reads, mirroring C-zsh's `getmathparam`
/// (Src/math.c:337). When the variable holds a string like "hello"
/// that isn't numeric, C falls back to recursively evaluating the
/// raw string as an arith expression; if that fails too, returns 0.
///
/// Used by the ArithCompiler pre-load path so `(( y = x ))` with
/// `x="hello"` reads `x` as integer 0, then assigns y as integer 0
/// — matching zsh's behaviour. The previous Rust port used
/// BUILTIN_GET_VAR which returned the raw string "hello"; the
/// ArithCompiler stored it verbatim in y's slot, and the post-sync
/// BUILTIN_SET_VAR wrote y="hello" as scalar instead of y=0 as
/// integer. Bug #118 in docs/BUGS.md.
///
/// Stack: pops `name` (string), pushes coerced numeric Value.
/// argc = 1.
pub const BUILTIN_GET_MATH_VAR: u16 = 529;

/// GLOB_SUBST runtime gate for words containing parameter / command
/// substitution. C-zsh's `prefork` (Src/subst.c) runs `shtokenize`
/// on the substituted value when `GLOB_SUBST` is set, making the
/// substituted chars eligible for filename generation. With the
/// option off, substituted chars stay literal.
///
/// The Rust port's compile_zsh emits `compile_word_str` for words
/// like `/tmp/X/$pat`, which returns the post-expansion string but
/// never runs glob expansion (no path here triggers
/// BUILTIN_GLOB_EXPAND). Bug #119 in docs/BUGS.md: with `setopt
/// glob_subst`, `for f in /tmp/X/$pat` (pat="*.txt") never matched
/// `*.txt` files.
///
/// This opcode wraps the substitution result and dispatches at
/// runtime: when GLOB_SUBST is OFF, return unchanged; when ON,
/// pass the value through `expand_glob` so glob metas become
/// active. Emitted by `compile_for_words` (and similar sites)
/// after WORD_SPLIT for words with unquoted expansion.
///
/// Stack: pops a Value (Str or Array of Str), pushes the glob-
/// expanded result (still Str or Array depending on input shape).
/// argc = 1.
pub const BUILTIN_GLOB_SUBST_EXPAND: u16 = 530;
/// `BUILTIN_ASSOC_HAS_KEY` constant — `${(k)assoc[name]}` key-existence
/// query. Returns the key text on hit, empty string on miss. Bug #145.
pub const BUILTIN_ASSOC_HAS_KEY: u16 = 531;
/// `BUILTIN_ARRAY_DROP_EMPTY` constant — filter empty elements from
/// an Array on the stack. Used by `for x in $@` / `for x in $*`
/// unquoted forms. Bug #166.
pub const BUILTIN_ARRAY_DROP_EMPTY: u16 = 532;
/// `BUILTIN_QUOTEDZPUTS` constant — run top-of-stack value through
/// `crate::ported::utils::quotedzputs` and push the quoted result.
/// Used by the cond xtrace path so non-printable bytes (e.g.
/// `$'\C-[OP'` expanded ESC+OP) get re-wrapped in `$'…'` form for
/// the trace prefix line, matching zsh's `Src/exec.c` cond trace
/// which calls `quotedzputs(operand, xtrerr)` on each side. Bug
/// surfaced when `[[ -n $'\C-[OP' ]]` traced as `[[ -n OP ]]`
/// (raw bytes leaked through the terminal) vs zsh's
/// `[[ -n $'\C-[OP' ]]` source-form preservation.
pub const BUILTIN_QUOTEDZPUTS: u16 = 533;
/// `BUILTIN_QUOTE_TOKENIZED_OUTPUT` — port of
/// `crate::ported::exec::quote_tokenized_output` (Src/exec.c:2114)
/// applied to top-of-stack scalar. Used by cond xtrace for the RHS
/// of pattern-context comparisons (`=` / `==` / `!=`) where C zsh
/// emits the SOURCE form: untokenize lexer tokens (Star → `*`,
/// Inpar → `(`, …) and backslash-escape special chars, but
/// preserve literal ASCII unchanged. Distinct from quotedzputs
/// which wraps the whole string in `'…'` / `$'…'` based on
/// non-printability — that's wrong for `[[ x = a* ]]` which must
/// render as `[[ x = a* ]]`, not `'a*'`.
pub const BUILTIN_QUOTE_TOKENIZED_OUTPUT: u16 = 534;

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
        let brace_ccl = with_executor(|exec| opt_state_get("braceccl").unwrap_or(false));
        crate::ported::glob::xpandbraces(s, brace_ccl)
    }

    fn str_match(&mut self, s: &str, pattern: &str) -> bool {
        // Shell glob match — `*`, `?`, `[...]`, alternation. Used by `[[ x = pat ]]`,
        // `case` arms, and any other point that compares against a glob pattern.
        glob_match_static(s, pattern)
    }

    fn expand_param(&mut self, name: &str, _modifier: u8, _args: &[Value]) -> Value {
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
        Value::str(val_str)
    }

    fn process_sub_in(&mut self, sub: &fusevm::Chunk) -> String {
        // c:Src/exec.c::getproc — `<(cmd)` uses pipe + fork + the
        // `/dev/fd/N` filesystem entry (where N is the read end of
        // the pipe held open in the parent). Consumer opens
        // `/dev/fd/N`, reads the cmd's stdout through the pipe.
        // Both macOS and Linux expose `/dev/fd` for held-open file
        // descriptors. Previous Rust port captured stdout into
        // `/tmp/zshrs_psub_*` tempfiles synchronously — works for
        // `diff <(a) <(b)` style readers that scan once but diverges
        // from zsh's observable path string and breaks any consumer
        // that introspects the path or expects a non-seekable pipe.
        let mut fds: [libc::c_int; 2] = [-1, -1];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            // Pipe creation failed — fall back to tempfile so we at
            // least return SOMETHING.
            let fifo_path = format!(
                "/tmp/zshrs_psub_fallback_{}_{}",
                std::process::id(),
                with_executor(|e| {
                    let n = e.process_sub_counter;
                    e.process_sub_counter += 1;
                    n
                })
            );
            let _ = fs::remove_file(&fifo_path);
            return fifo_path;
        }
        let (read_end, write_end) = (fds[0], fds[1]);
        let sub_for_child = sub.clone();
        match unsafe { libc::fork() } {
            -1 => {
                unsafe {
                    libc::close(read_end);
                    libc::close(write_end);
                }
                return String::from("/dev/null");
            }
            0 => {
                // Child: close read end, dup write end to stdout,
                // run the sub-chunk, exit. The exit closes the
                // write end automatically, so the parent's reader
                // gets EOF when the cmd finishes.
                unsafe {
                    libc::close(read_end);
                    libc::dup2(write_end, libc::STDOUT_FILENO);
                    libc::close(write_end);
                }
                crate::fusevm_disasm::maybe_print_stdout("process_subst_in", &sub_for_child);
                let mut vm = fusevm::VM::new(sub_for_child);
                register_builtins(&mut vm);
                vm.set_shell_host(Box::new(ZshrsHost));
                let _ = vm.run();
                let _ = std::io::stdout().flush();
                unsafe { libc::_exit(0) };
            }
            _ => {
                // Parent: close write end, keep read end open under
                // the same fd value so `/dev/fd/N` resolves to the
                // pipe's read side. NOTE: FD_CLOEXEC must STAY clear
                // — consumers like `cat <(cmd)` and `diff <(a) <(b)`
                // discover the fd via exec inheritance, so closing
                // on exec defeats the whole point. C zsh's getproc
                // (Src/exec.c:5045+) leaves the fd open across exec.
                unsafe {
                    libc::close(write_end);
                }
            }
        }
        format!("/dev/fd/{}", read_end)
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
        let _ = fs::remove_file(&fifo_path);
        let cpath = match CString::new(fifo_path.clone()) {
            Ok(c) => c,
            Err(_) => return fifo_path,
        };
        if unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) } != 0 {
            // Fall back to plain file if mkfifo fails.
            let _ = fs::write(&fifo_path, "");
            return fifo_path;
        }
        let sub = sub.clone();
        let fifo_for_child = fifo_path.clone();
        match unsafe { libc::fork() } {
            -1 => {
                let _ = fs::remove_file(&fifo_path);
            }
            0 => {
                // Child: open FIFO for read, dup onto stdin, run sub-chunk, exit.
                if let Ok(f) = fs::OpenOptions::new().read(true).open(&fifo_for_child) {
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
                env_vars: env::vars().collect(),
                // Save the LOGICAL pwd ($PWD env), not `current_dir()`'s
                // symlink-resolved path. zsh's subshell isolation per
                // Src/exec.c at the `entersubsh` path treats `pwd` (the
                // shell-tracked logical PWD) as the carrier — see
                // `Src/builtin.c:1239-1242` where cd writes the logical
                // dest into `pwd`. Falling back to current_dir() only
                // when PWD is unset matches `setupvals` at
                // `Src/init.c:1100+`.
                cwd: env::var("PWD")
                    .ok()
                    .map(PathBuf::from)
                    .or_else(|| env::current_dir().ok()),
                umask: cur_umask,
                // Snapshot canonical `traps_table` — bin_trap writes
                // there (`Src/builtin.c`).
                traps: crate::ported::builtin::traps_table()
                    .lock()
                    .map(|t| t.clone())
                    .unwrap_or_default(),
                // Snapshot option store so `(set -e)` /
                // `(setopt extendedglob)` don't leak to parent.
                opts: crate::ported::options::opt_state_snapshot(),
                // c:Src/exec.c — fork() copies the alias table to
                // the subshell. `(alias x=y)` inside the subshell
                // dies with the child; the parent doesn't see x.
                // Snapshot here so subshell_end can restore.
                // Bug #209 in docs/BUGS.md.
                aliases: crate::ported::hashtable::aliastab_lock()
                    .read()
                    .ok()
                    .map(|t| {
                        t.iter()
                            .map(|(k, v)| (k.clone(), v.text.clone()))
                            .collect()
                    })
                    .unwrap_or_default(),
                // c:Src/exec.c::entersubsh — same fork-copy
                //   semantics for shfunctab. `(f() { ... })` defined
                //   inside the subshell dies with the child; parent's
                //   `type f` reports "not found". Bug #208 in
                //   docs/BUGS.md.
                shfuncs: crate::ported::hashtable::shfunctab_lock()
                    .read()
                    .ok()
                    .map(|t| t.snapshot())
                    .unwrap_or_default(),
                functions_compiled: exec.functions_compiled.clone(),
                function_source: exec.function_source.clone(),
                // c:Src/exec.c::entersubsh — subshell forks its own
                // modulestab. A `(zmodload zsh/X)` inside the
                // subshell flips MOD_INIT_B on the CHILD's
                // modulestab; when the child exits the change
                // dies with it. zshrs's in-process subshell would
                // otherwise leak the load to the parent.
                // Bug #210 in docs/BUGS.md. Snapshot just the
                // (name → flags) pairs since the only mutating
                // field is the flags bitmask (MOD_INIT_B for
                // loaded, MOD_UNLOAD for unloaded).
                modules: crate::ported::module::MODULESTAB
                    .lock()
                    .ok()
                    .map(|t| {
                        t.modules
                            .iter()
                            .map(|(k, v)| (k.clone(), v.node.flags))
                            .collect()
                    })
                    .unwrap_or_default(),
                // c:Src/exec.c::entersubsh — fork-copy semantics for
                // THINGYTAB (ZLE widget registry). A subshell `zle -N`
                // / `zle -D` mutation dies with the child in C zsh;
                // mirror via in-process snapshot. Bug #453.
                thingytab: crate::ported::zle::zle_thingy::thingytab()
                    .lock()
                    .ok()
                    .map(|t| t.clone())
                    .unwrap_or_default(),
                // c:Src/exec.c::entersubsh — same fork-copy for the
                // KEYMAPNAMTAB (named keymap registry). `bindkey -N km`
                // / `bindkey -D km` inside a subshell dies with the
                // child. Bug #454.
                keymapnamtab: crate::ported::zle::zle_keymap::keymapnamtab()
                    .lock()
                    .ok()
                    .map(|t| t.clone())
                    .unwrap_or_default(),
            });
            // Subshell starts with EXIT trap cleared so the parent's
            // EXIT handler doesn't fire when the subshell ends. zsh:
            // each subshell has its own trap context. Other signals
            // are inherited (well, parent's are still in place — but
            // a trap set INSIDE the subshell shouldn't leak out).
            if let Ok(mut t) = crate::ported::builtin::traps_table().lock() {
                t.remove("EXIT");
            }
            let level = exec
                .scalar("ZSH_SUBSHELL")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0);
            // c:Src/exec.c — ZSH_SUBSHELL carries PM_READONLY (declared
            // in params.rs special_params); setsparam would be rejected
            // by assignstrvalue's PM_READONLY guard. Write u_val
            // directly — same bypass pattern as BUILTIN_SET_LINENO at
            // line 2784. C zsh's PM_SPECIAL GSU vtable handles this
            // implicitly via the setfn callback.
            let new_level = (level + 1) as i64;
            if let Ok(mut tab) = crate::ported::params::paramtab().write() {
                if let Some(pm) = tab.get_mut("ZSH_SUBSHELL") {
                    pm.u_val = new_level;
                    pm.u_str = Some(new_level.to_string());
                    pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                }
            }
        });
        // Bump SUBSHELL_DEPTH so zexit defers process::exit (see
        // SUBSHELL_DEPTH declaration in src/ported/builtin.rs for
        // rationale).
        crate::ported::builtin::SUBSHELL_DEPTH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // c:Src/exec.c::entersubsh — C zsh's subshell is a forked
        // child process: signals sent to the parent (via `kill $$`
        // inside the subshell, where `$$` is the parent's pid)
        // never reach the child's signal handlers. zshrs's
        // in-process subshell shares the process pid with the
        // parent, so without queueing the subshell's trap handler
        // fires for signals that zsh would deliver only to the
        // parent. Queue signals across the subshell body so the
        // parent's restored trap table sees them after
        // subshell_end's unqueue drain. Bug #450.
        crate::ported::signals_h::queue_signals();
    }

    fn subshell_end(&mut self) -> Option<i32> {
        // Fire subshell's EXIT trap BEFORE restoring parent state so
        // the trap body sees the subshell's vars and exit status. zsh
        // forks for `(...)` so the trap runs in the child process,
        // before exit. We mirror by running it here, just before the
        // pop+restore. REMOVE the trap before firing so the inner
        // execute_script doesn't fire it again at its own end.
        let exit_trap_body = crate::ported::builtin::traps_table()
            .lock()
            .ok()
            .and_then(|mut t| t.remove("EXIT"));
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
                let current: HashMap<String, String> = env::vars().collect();
                for k in current.keys() {
                    if !snap.env_vars.contains_key(k) {
                        env::remove_var(k);
                    }
                }
                for (k, v) in &snap.env_vars {
                    if current.get(k) != Some(v) {
                        env::set_var(k, v);
                    }
                }
                if let Some(cwd) = snap.cwd {
                    let _ = env::set_current_dir(&cwd);
                    // Resync $PWD env so a parent `pwd` doesn't read
                    // the cwd the subshell `cd`'d into.
                    env::set_var("PWD", &cwd);
                }
                // Restore umask. zsh's `(umask 077)` doesn't leak to
                // parent because the subshell forks; we run in-process
                // so we manually reset.
                unsafe {
                    libc::umask(snap.umask as libc::mode_t);
                }
                // Restore parent's traps (the subshell's own traps die
                // with it). zsh: `(trap "X" USR1)` doesn't leak the
                // USR1 trap out of the subshell. Write back to the
                // canonical `traps_table` (bin_trap writes there).
                if let Ok(mut t) = crate::ported::builtin::traps_table().lock() {
                    *t = snap.traps;
                }
                // Restore parent's option store so `(set -e)` /
                // `(setopt extendedglob)` don't leak. zsh forks
                // subshells so child option changes die with the
                // child; we run in-process and must restore.
                crate::ported::options::opt_state_restore(snap.opts);
                // c:Src/exec.c — fork() means alias mutations in a
                // subshell die with the child. Restore parent's
                // alias table from snapshot. Clear current entries
                // then re-add parent's. Bug #209 in docs/BUGS.md.
                if let Ok(mut tab) = crate::ported::hashtable::aliastab_lock().write() {
                    tab.clear();
                    for (name, text) in snap.aliases {
                        tab.add(crate::ported::zsh_h::alias {
                            node: crate::ported::zsh_h::hashnode {
                                next: None,
                                nam: name,
                                flags: 0,
                            },
                            text,
                            inuse: 0,
                        });
                    }
                }
                // c:Src/exec.c::entersubsh — same fork-copy
                //   semantics for shfunctab. Restore parent's function
                //   table from snapshot so `(f() { ... })` definitions
                //   inside the subshell don't leak to the parent.
                //   Bug #208 in docs/BUGS.md.
                if let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().write() {
                    tab.restore(snap.shfuncs);
                }
                // Restore the runtime dispatch tables (compiled chunks
                // + source). Without these, a subshell-defined
                // override leaves its bytecode in place even after
                // shfunctab is restored — `g` after the subshell would
                // still run the override.
                exec.functions_compiled = snap.functions_compiled;
                exec.function_source = snap.function_source;
                // c:Src/exec.c::entersubsh — restore parent's
                // modulestab so a subshell `(zmodload zsh/X)` doesn't
                // leak to the parent. Bug #210 in docs/BUGS.md.
                // Restore via per-module flag write since the
                // snapshot is `(name → flags)` only.
                if let Ok(mut t) = crate::ported::module::MODULESTAB.lock() {
                    for (name, saved_flags) in &snap.modules {
                        if let Some(m) = t.modules.get_mut(name) {
                            m.node.flags = *saved_flags;
                        }
                    }
                }
                // c:Src/exec.c::entersubsh — restore parent's THINGYTAB
                // so a subshell's `zle -N w f` / `zle -D w` doesn't
                // affect the parent's widget registry. Bug #453.
                if let Ok(mut t) = crate::ported::zle::zle_thingy::thingytab().lock() {
                    *t = snap.thingytab;
                }
                // Same for KEYMAPNAMTAB. Bug #454.
                if let Ok(mut t) = crate::ported::zle::zle_keymap::keymapnamtab().lock() {
                    *t = snap.keymapnamtab;
                }
            }
        });
        // Decrement SUBSHELL_DEPTH. If a deferred subshell exit
        // landed inside (EXIT_PENDING set with depth > 0), promote
        // the deferred status into the subshell's exit status now
        // that we're at the boundary, then clear so the parent
        // continues. Matches C zsh's "subshell-exit-via-fork"
        // boundary where the child's process::exit(N) becomes
        // $WAITSTATUS / $? in the parent.
        crate::ported::builtin::SUBSHELL_DEPTH.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        // c:Src/exec.c — drain the signal queue against the now-
        // restored parent trap table. Pairs with the
        // queue_signals() call at the end of subshell_begin.
        // Any `kill $$` from inside the subshell is processed
        // here against OUTER's trap, matching C zsh's
        // signal-delivery-to-parent semantics. Bug #450.
        crate::ported::signals_h::unqueue_signals();
        let exit_pending =
            crate::ported::builtin::EXIT_PENDING.load(std::sync::atomic::Ordering::Relaxed);
        if exit_pending != 0 {
            // c:Src/builtin.c — `exit N` masks N to 8 bits because
            // POSIX _exit takes the low byte as status. `(exit 256)`
            // and `(exit 0)` are indistinguishable to the parent;
            // `(exit 257)` exits with 1. Without the mask zshrs's
            // in-process subshell propagated the full i32 (256) into
            // the parent's $?, diverging from zsh.
            let raw = crate::ported::builtin::EXIT_VAL.load(std::sync::atomic::Ordering::Relaxed);
            let val = raw & 0xFF;
            with_executor(|exec| exec.set_last_status(val));
            crate::ported::builtin::EXIT_PENDING.store(0, std::sync::atomic::Ordering::Relaxed);
            crate::ported::builtin::RETFLAG.store(0, std::sync::atomic::Ordering::Relaxed);
            crate::ported::builtin::BREAKS.store(0, std::sync::atomic::Ordering::Relaxed);
            // Set the post-subshell-exit guard. The next GET_VAR
            // sync_status path consults this to skip its
            // vm.last_status→LASTVAL sync (which would overwrite the
            // deferred-exit status we just set with stale vm state
            // since SubshellEnd doesn't propagate status into the
            // VM). Cleared as soon as the next sync_status sees it.
            SUBSHELL_EXIT_STATUS_PENDING.with(|c| c.set(true));
            // Return the deferred-exit status so the VM updates its
            // own `last_status`. Otherwise run_chunk's post-script
            // `set_last_status(vm.last_status)` would clobber LASTVAL
            // back to the stale pre-subshell value.
            return Some(val);
        }
        None
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

    fn regex_match(&mut self, s: &str, regex: &str) -> bool {
        // c:Src/Modules/regex.c:54 `zcond_regex_match` — POSIX ERE
        // matching + populate `$MATCH` / `$MBEGIN` / `$MEND` /
        // `$match[]` / `$mbegin[]` / `$mend[]` (or `$BASH_REMATCH`
        // under BASHREMATCH). Direct delegation to the canonical
        // port at src/ported/modules/regex.rs:58.
        //
        // The bridge passthru path delivers TOKEN-form bytes here
        // (Inbrack \u{91}, Outbrack \u{92}, Star \u{87}, Quest
        // \u{86}, etc.) since the lexer tokenizes regex meta chars
        // inside `[[ ]]`. The host regex engine expects ASCII, so
        // untokenize the pattern (and subject, for safety) once at
        // this boundary. zsh C reaches its POSIX-ERE engine through
        // the same untokenize path inside zcond_regex_match.
        let s_clean = crate::lex::untokenize(s);
        let regex_clean = crate::lex::untokenize(regex);
        crate::ported::modules::regex::zcond_regex_match(
            &[s_clean.as_str(), regex_clean.as_str()],
            crate::ported::modules::regex::ZREGEX_EXTENDED,
        ) != 0
    }

    fn with_redirects_end(&mut self) {
        with_executor(|exec| exec.host_redirect_scope_end());
        // c:Src/exec.c:5172 — if any redirect in this scope failed
        // (noclobber-blocked, ENOENT for read, etc.), the command's
        // exit status is forced to 1 regardless of what the (still-
        // executed) command's own exit was. C zsh prevents the
        // command from running at all when a redirect fails; the
        // Rust port still runs it (sinking output to /dev/null in
        // the noclobber arm at host_apply_redirect:5481) and then
        // overrides $? here. Same observable effect for the common
        // pattern `echo x > existing-file` under noclobber.
        let failed = with_executor(|exec| {
            let f = exec.redirect_failed;
            exec.redirect_failed = false;
            f
        });
        if failed {
            with_executor(|exec| exec.set_last_status(1));
        }
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
        // c:Src/subst.c paramsubst — when `${var:?msg}` or `${var?msg}`
        // triggered the "parameter null or not set" error, errflag
        // is raised and zsh aborts the simple command without
        // attempting exec. The expansion may have produced empty
        // argv[0] which falls into the c:?/permission-denied path
        // below, masking the real diagnostic with a spurious
        // "permission denied:" line and rc=126 instead of rc=1.
        // Honour errflag here so the script ends with the
        // paramsubst error as the sole diagnostic. Bug #86.
        //
        // c:Src/exec.c — C's execlist loop clears ERRFLAG_ERROR
        // between sublists when the error came from a NOMATCH-style
        // command failure (glob no-match, etc.) so subsequent
        // sublists run. zshrs's vm dispatch handles this at the
        // post-command-boundary HERE: if THIS command has its
        // `current_command_glob_failed` cell set (meaning the glob
        // NOMATCH happened during this command's argv prep), surface
        // status 1 and clear BOTH the cell AND ERRFLAG_ERROR so the
        // NEXT exec call sees a clean state. The errflag from
        // genuine script-fatal errors (parse, redirect, paramsubst
        // `${:?msg}`) does NOT come paired with glob_failed, so
        // those still short-circuit + propagate.
        let glob_failed = with_executor(|exec| {
            let f = exec.current_command_glob_failed.get();
            exec.current_command_glob_failed.set(false);
            f
        });
        if glob_failed {
            crate::ported::utils::errflag.fetch_and(
                !crate::ported::zsh_h::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            with_executor(|exec| exec.set_last_status(1));
            return 1;
        }
        if (crate::ported::utils::errflag.load(std::sync::atomic::Ordering::SeqCst)
            & crate::ported::zsh_h::ERRFLAG_ERROR)
            != 0
        {
            return 1;
        }
        // c:Src/exec.c — two distinct empty-command cases:
        //
        // 1. args=[""]  — an explicit empty-string command word
        //    (`""`, `"\$unset"`, `\$'\$x'`). zsh attempts exec(2)
        //    on the empty path → EACCES → "permission denied", \$?
        //    = 126.
        //
        // 2. args=[]    — the WORD LIST is empty (unquoted \$(\$cmd)
        //    that produced empty, or an unquoted unset \$var that
        //    elided). zsh: no exec is attempted; \$? becomes the
        //    last cmd-subst's exit status (the inner sub-VM
        //    already set last_status), and the line completes
        //    silently. Critically NOT 126.
        if args.is_empty() {
            // c:Src/exec.c — empty word list passes through to a
            // no-op; preserve whatever the inner cmd-subst's exit
            // is. Return last_status so the caller's SetStatus
            // round-trips correctly.
            return with_executor(|exec| exec.last_status());
        }
        if args[0].is_empty() {
            let script_name =
                crate::ported::utils::scriptname_get().unwrap_or_else(|| "zshrs".to_string());
            let lineno: u64 = with_executor(|exec| {
                exec.scalar("LINENO")
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(1)
            });
            eprintln!("{}:{}: permission denied: ", script_name, lineno);
            return 126;
        }
        // c:Src/exec.c — when any redirect in the current scope
        // failed (e.g. noclobber blocked a `>` overwrite), zsh
        // refuses to execute the command and exits with status 1.
        // The Rust port still applied the command (writing to the
        // /dev/null sink installed by host_apply_redirect's
        // noclobber arm), but the success status overwrote the
        // intended `1`. Short-circuit here so the exec returns 1
        // without running the body.
        let redir_failed = with_executor(|exec| {
            let f = exec.redirect_failed;
            exec.redirect_failed = false;
            f
        });
        if redir_failed {
            return 1;
        }
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
        let status = with_executor(|exec| exec.host_exec_external(&args));
        // c:Src/jobs.c:1748 waitonejob (no-procs else-branch). zshrs's
        // exec model routes external commands through host_exec_external
        // (which already waitpid'd in-line); the canonical waitonejob
        // expects a Job to derive lastval, but here we already know
        // it. Synthesize a procs-less job so waitonejob's no-procs
        // branch fires the `pipestats[0]=lastval; numpipestats=1;`
        // update via the canonical port.
        crate::ported::builtin::LASTVAL.store(status, std::sync::atomic::Ordering::Relaxed);
        let mut synth = crate::ported::zsh_h::job::default();
        crate::ported::jobs::waitonejob(&mut synth);
        status
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
        let saved_stderr = unsafe { libc::dup(libc::STDERR_FILENO) };
        let write_fd = AsRawFd::as_raw_fd(&write_end);
        unsafe {
            libc::dup2(write_fd, libc::STDOUT_FILENO);
        }
        drop(write_end);

        // c:Bug #56 — publish the saved outer fds so a trap firing
        // during the nested VM run can route its body output to the
        // PARENT's stdout instead of the cmdsub's pipe-bound fd 1.
        // zsh's forked cmdsub gets this for free (trap runs in the
        // parent process whose fd 1 is untouched). zshrs's
        // in-process cmdsub needs this thread-local stack so the
        // trap dispatcher can find the right destination fd.
        CMDSUBST_OUTER_FDS.with(|s| s.borrow_mut().push((saved_stdout, saved_stderr)));

        crate::fusevm_disasm::maybe_print_stdout("host.cmd_subst", sub);
        let mut vm = fusevm::VM::new(sub.clone());
        register_builtins(&mut vm);
        vm.set_shell_host(Box::new(ZshrsHost));
        let _ = vm.run();
        let cmd_status = vm.last_status;

        CMDSUBST_OUTER_FDS.with(|s| {
            s.borrow_mut().pop();
        });

        unsafe {
            libc::dup2(saved_stdout, libc::STDOUT_FILENO);
            libc::close(saved_stdout);
            if saved_stderr >= 0 {
                libc::close(saved_stderr);
            }
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
        // c:Src/exec.c — when the command word is empty (e.g. `""`
        // or `"$nonexistent"`), zsh attempts the exec(2) which
        // returns EACCES ("permission denied") and exits 126. The
        // Rust port silently treated empty as a no-op (status 0).
        // Match zsh by emitting the diagnostic and returning 126.
        if name.is_empty() {
            let script_name =
                crate::ported::utils::scriptname_get().unwrap_or_else(|| "zshrs".to_string());
            let lineno: u64 = with_executor(|exec| {
                exec.scalar("LINENO")
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(1)
            });
            eprintln!("{}:{}: permission denied: ", script_name, lineno);
            with_executor(|exec| exec.set_last_status(126));
            return Some(126);
        }
        // c:Src/exec.c — redirect failure in this scope means the
        // command should NOT run. The Host::exec path already has
        // this gate (at fn exec above); call_function takes external
        // commands like `cat <&3` through a different code path, so
        // gate here too. Without this, bad-fd redirects produced
        // the diagnostic but the external command still ran, so $?
        // came out from the command's natural exit instead of the
        // forced 1.
        let redir_failed = with_executor(|exec| {
            let f = exec.redirect_failed;
            exec.redirect_failed = false;
            f
        });
        if redir_failed {
            with_executor(|exec| exec.set_last_status(1));
            return Some(1);
        }
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
            // ztest framework (src/extensions/ztest.rs — port of
            // ../strykelang's unit-test framework). All zassert_*/
            // ztest_* names route through the single try_dispatch
            // helper so adding/removing assertions only touches
            // ztest.rs.
            n if crate::extensions::ztest::try_dispatch_known(n) => {
                let status = with_executor(|exec| {
                    crate::extensions::ztest::try_dispatch(exec, n, &args).unwrap_or(1)
                });
                return Some(status);
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

        // c:Src/exec.c:3050-3068 — module-provided builtins (registered
        // via each module's `bintab` and folded into the canonical
        // `builtintab` by `createbuiltintable`) must dispatch BEFORE
        // PATH lookup. fusevm's `shell_builtins::builtin_id` doesn't
        // know about per-module entries like `log`
        // (Src/Modules/watch.c:693) — they reach call_function as
        // CallFunction ops. Consult the merged builtintab here so
        // `log` runs the canonical `bin_log` instead of falling
        // through to `/usr/bin/log` on macOS. Bug #72 in docs/BUGS.md.
        //
        // User-defined functions still take precedence over builtins
        // (zsh's `alias → function → builtin → external` resolution
        // order, c:Src/exec.c:3038-3068). Check `functions_compiled`
        // first so a user `log() { ... }` shadows the module bin_log.
        // c:Src/exec.c — shfunctab->getnode (the DISABLED-filtering
        // accessor) returns NULL for entries flipped to DISABLED via
        // `disable -f NAME`. functions_compiled holds the body
        // independently of the DISABLED flag, so check shfunctab first
        // and mask the lookup when the entry is disabled. Bug #221
        // in docs/BUGS.md.
        let user_fn_disabled = crate::ported::hashtable::shfunctab_lock()
            .read()
            .ok()
            .and_then(|t| {
                let entry = t.get_including_disabled(name)?;
                Some((entry.node.flags as u32 & crate::ported::zsh_h::DISABLED as u32) != 0)
            })
            .unwrap_or(false);
        let has_user_fn = !user_fn_disabled
            && with_executor(|exec| exec.functions_compiled.contains_key(name));
        if !has_user_fn {
            // c:Src/exec.c:3056 — `builtintab->getnode(builtintab,
            // cmdarg)` returns NULL for DISABLED entries, falling
            // execcmd through to PATH lookup. Mirror by gating the
            // bn_in_tab match on the BUILTINS_DISABLED set. Bug #106
            // in docs/BUGS.md.
            let disabled = crate::ported::builtin::BUILTINS_DISABLED
                .lock()
                .map(|s| s.contains(name))
                .unwrap_or(false);
            let bn_in_tab = !disabled
                && crate::ported::builtin::createbuiltintable().contains_key(name);
            if bn_in_tab {
                return Some(dispatch_builtin_raw(name, args));
            }
        }

        // c:Src/lex.c — alias expansion is a LEXER-TIME pass, not a
        // run-time lookup. zsh parses the whole `-c` argument (or
        // script) before executing, so aliases defined in the same
        // parse unit don't apply to commands parsed earlier. Only at
        // an INTERACTIVE prompt does each line parse separately with
        // the latest aliastab visible.
        //
        // Gate the run-time alias-rewrite path on `interactive` so
        // `alias hi='echo hello'; hi` in `-c` mode falls through to
        // "command not found" (matching zsh) while interactive REPL
        // input still re-parses with the live aliastab.
        let interactive = crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE);
        let already_expanding = if interactive {
            crate::ported::hashtable::aliastab_lock()
                .read()
                .ok()
                .and_then(|tab| tab.get(name).map(|a| a.inuse != 0))
                .unwrap_or(false)
        } else {
            true // suppress lookup entirely in non-interactive mode
        };
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

        // $_ pre-body bump and pending-underscore tracking are
        // ZshrsHost-only concerns (prompt rendering). Apply BEFORE
        // delegating to dispatch_function_call so the body sees the
        // bumped value.
        //
        // c:Src/exec.c:3491 — `setunderscore((args && nonempty(args))
        // ? ((char *) getdata(lastnode(args))) : "")`. C sets $_ to
        // the LAST node of the WHOLE args list (which includes argv[0]
        // == the function name). So for a no-arg `f`, $_ becomes "f"
        // inside the function body. The Rust port at the CallFunction
        // op-handler receives `args` WITHOUT the command name
        // (compile_zsh.rs:1571 only pushes simple.words[1..]). The
        // last() fallback `|| fn_name.clone()` already covers the
        // no-arg case, but `exec.set_scalar("_", ...)` writes paramtab
        // — the canonical `$_` read goes through `underscoregetfn`
        // (params.rs:7836) which reads the `zunderscore` Mutex.
        // setsparam("_") doesn't update that mutex, so the body's
        // `${_}` returned empty. Bug #279 in docs/BUGS.md. Mirror the
        // C `setunderscore` by writing via `set_zunderscore` directly.
        let fn_name = name.to_string();
        with_executor(|exec| {
            let dollar_underscore = args.last().cloned().unwrap_or_else(|| fn_name.clone());
            exec.set_scalar("_".to_string(), dollar_underscore.clone());
            crate::ported::params::set_zunderscore(std::slice::from_ref(&dollar_underscore));
            exec.pending_underscore = Some(dollar_underscore);
        });

        // Delegate the actual function dispatch to the canonical
        // `dispatch_function_call` (which itself wraps the canonical
        // `doshfunc` port from `Src/exec.c:5823`). Single doshfunc
        // call-site keeps scope-mgmt invariants in one place.
        let status = with_executor(|exec| exec.dispatch_function_call(&fn_name, &args));

        // $_ post-body — last call-arg or function name. Mirrors the
        // C `setunderscore` invocation after the body returns.
        with_executor(|exec| {
            let last_call_arg = args.last().cloned().unwrap_or_else(|| fn_name.clone());
            exec.set_scalar("_".to_string(), last_call_arg.clone());
            exec.pending_underscore = Some(last_call_arg);
        });

        status
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Host-routed shell ops: ShellExecutor methods invoked by ZshrsHost from the
// fusevm VM. Not a port of Src/exec.c (see file-level docs above) — they're
// the bridge between fusevm opcodes and ShellExecutor state.
// ───────────────────────────────────────────────────────────────────────────
impl ShellExecutor {
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
        result: std::io::Result<fs::File>,
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
                eprintln!("{}:1: {}: {}", shname(), msg, target);
                *redirect_failed = true;
                if let Ok(devnull) = fs::OpenOptions::new()
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
    /// `host_apply_redirect` — see implementation.
    pub fn host_apply_redirect(&mut self, fd: u8, op_byte: u8, target: &str) {
        // `&>` / `&>>` always target both fd 1 and fd 2 regardless of the
        // fd byte the parser supplied (the lexer's tokfd clamp makes the
        // raw value unreliable for these forms).
        let fd: i32 = if matches!(op_byte, r::WRITE_BOTH | r::APPEND_BOTH) {
            1
        } else {
            fd as i32
        };
        // c:Src/exec.c — for DUP_READ / DUP_WRITE forms (<&N / >&N),
        // validate the source fd is open BEFORE the save-and-dup
        // dance below. The save's `dup(fd)` reclaims the lowest free
        // fd, which on closed-fd reuse would let dup2(src=N, …)
        // succeed against the freshly-claimed slot — masking the
        // user's "bad file descriptor" error. Check src_fd first.
        if matches!(op_byte, r::DUP_READ | r::DUP_WRITE) {
            let n_check = target.trim_start_matches('&');
            if n_check != "-" {
                if let Ok(src_fd) = n_check.parse::<i32>() {
                    if unsafe { libc::fcntl(src_fd, libc::F_GETFD) } == -1 {
                        eprintln!("{}:1: {}: bad file descriptor", shname(), src_fd);
                        self.set_last_status(1);
                        self.redirect_failed = true;
                        return;
                    }
                }
            }
        }
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
                //
                // c:Src/exec.c:2241-2245 clobber_open recover path:
                // after O_EXCL fails, reopen and `if (!S_ISREG(...))
                // return fd;` — non-regular targets (char/block-
                // special, FIFO, socket) bypass the noclobber check.
                // Bug #30 in docs/BUGS.md: this bridge-side check did
                // a bare `Path::exists()` and treated `/dev/null` as
                // a protected file, breaking `setopt no_clobber; echo
                // hi > /dev/null` and every `2> /dev/null` idiom.
                // Add a regular-file stat gate that matches the C
                // semantic. The canonical clobber_open at
                // src/ported/exec.rs:2123 already handles this; the
                // bridge duplicates a stripped-down version here and
                // must mirror the same check.
                let noclobber = opt_state_get("noclobber").unwrap_or(false)
                    || !opt_state_get("clobber").unwrap_or(true);
                let target_is_regular_file = std::fs::metadata(target)
                    .map(|m| m.file_type().is_file())
                    .unwrap_or(false);
                if noclobber && target_is_regular_file {
                    eprintln!("{}:1: file exists: {}", shname(), target);
                    self.set_last_status(1);
                    // c:Src/exec.c — set redirect_failed so the scope-end
                    // hook (`with_redirects_end` in this file) forces
                    // $? to 1 regardless of the still-running command's
                    // own exit. Without this the next command (e.g.
                    // `echo x` writing to /dev/null below) succeeds
                    // and overwrites the redirect-failure status,
                    // making noclobber unobservable from $?.
                    self.redirect_failed = true;
                    // Sink the upcoming command's stdout to /dev/null
                    // so we don't leak its output to the terminal.
                    // zsh skips the command entirely; we approximate by
                    // discarding the output (the redirect target was
                    // the user's chosen sink, but with noclobber the
                    // file is protected — discarding matches the
                    // user's intent better than printing to terminal).
                    if let Ok(file) = fs::OpenOptions::new().write(true).open("/dev/null") {
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
                    fs::File::create(target),
                    target,
                    &mut self.redirect_failed,
                ) {
                    self.set_last_status(1);
                }
            }
            r::CLOBBER => {
                if !Self::redir_open_or_fail(
                    fd,
                    fs::File::create(target),
                    target,
                    &mut self.redirect_failed,
                ) {
                    self.set_last_status(1);
                }
            }
            r::APPEND => {
                // c:Src/exec.c:3924-3927 — `>>` honors NO_CLOBBER+!APPENDCREATE
                // by opening O_APPEND|O_WRONLY WITHOUT O_CREAT, so missing
                // files yield ENOENT. zsh source:
                //   if (!isset(CLOBBER) && !isset(APPENDCREATE) &&
                //       !IS_CLOBBER_REDIR(fn->type))
                //       mode = O_WRONLY|O_APPEND|O_NOCTTY;
                //   else mode = O_WRONLY|O_APPEND|O_CREAT|O_NOCTTY;
                // (IS_CLOBBER_REDIR — `>>!`/`>>|` — is currently flattened
                // to plain APPEND at compile time in
                // src/extensions/compile_zsh.rs:1654-1655, so the bang/pipe
                // forms can't be distinguished here yet.)
                let noclobber = opt_state_get("noclobber").unwrap_or(false)
                    || !opt_state_get("clobber").unwrap_or(true);
                let append_create = opt_state_get("appendcreate").unwrap_or(false)
                    || opt_state_get("append_create").unwrap_or(false);
                let open_result = if noclobber && !append_create {
                    fs::OpenOptions::new().append(true).open(target) // no create
                } else {
                    fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(target)
                };
                if !Self::redir_open_or_fail(
                    fd,
                    open_result,
                    target,
                    &mut self.redirect_failed,
                ) {
                    self.set_last_status(1);
                }
            }
            r::READ => {
                if !Self::redir_open_or_fail(
                    fd,
                    fs::File::open(target),
                    target,
                    &mut self.redirect_failed,
                ) {
                    self.set_last_status(1);
                }
            }
            r::READ_WRITE => {
                if let Ok(file) = fs::OpenOptions::new()
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
                // close-fd (`<&-` / `>&-`) per POSIX. The src_fd
                // validity check ran above before the save-and-dup.
                let n = target.trim_start_matches('&');
                if n == "-" {
                    unsafe { libc::close(fd) };
                } else if n == "p" {
                    // c:Src/exec.c — `<&p` / `>&p` route through the
                    // coprocin / coprocout globals. zsh's `coproc CMD`
                    // launch publishes those fds; the canonical
                    // bin_print / bin_read `-p` arms already consume
                    // them. The DUP redirect form is the third
                    // consumer: it must dup the coproc fd onto the
                    // target slot so the next command's stdin/stdout
                    // is wired to the running coprocess. Bug #388.
                    let coproc_fd = if op_byte == r::DUP_READ {
                        crate::ported::modules::clone::coprocin
                            .load(std::sync::atomic::Ordering::Relaxed)
                    } else {
                        crate::ported::modules::clone::coprocout
                            .load(std::sync::atomic::Ordering::Relaxed)
                    };
                    if coproc_fd < 0 {
                        eprintln!("{}:1: no coprocess", shname());
                        self.set_last_status(1);
                        self.redirect_failed = true;
                    } else {
                        unsafe {
                            libc::dup2(coproc_fd, fd);
                        }
                    }
                } else if let Ok(src_fd) = n.parse::<i32>() {
                    unsafe { libc::dup2(src_fd, fd) };
                } else {
                    tracing::warn!(target = %target, "DUP redir: target not parseable as fd");
                }
            }
            r::WRITE_BOTH => {
                if let Ok(file) = fs::File::create(target) {
                    let new_fd = file.into_raw_fd();
                    unsafe {
                        libc::dup2(new_fd, 1);
                        libc::dup2(new_fd, 2);
                        libc::close(new_fd);
                    }
                }
            }
            r::APPEND_BOTH => {
                if let Ok(file) = fs::OpenOptions::new()
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
        self.multios_scope_stack.push(Vec::new());
    }

    /// Pop the top redirect scope, restoring saved fds.
    pub fn host_redirect_scope_end(&mut self) {
        // c:Src/exec.c — restore saved fds FIRST so the multios
        // pipe-write end is released from `fd`, then close our
        // tracked close_on_end (the last surviving writer dup), then
        // join the splitter thread. If we closed close_on_end before
        // restoring saved, `fd` would still hold a pipe writer and
        // the thread would block forever waiting for EOF.
        if let Some(saved) = self.redirect_scope_stack.pop() {
            for (fd, saved_fd) in saved.into_iter().rev() {
                unsafe {
                    libc::dup2(saved_fd, fd);
                    libc::close(saved_fd);
                }
            }
        }
        if let Some(scope) = self.multios_scope_stack.pop() {
            for (write_fd, handle) in scope {
                if write_fd >= 0 {
                    unsafe {
                        libc::close(write_fd);
                    }
                }
                let _ = handle.join();
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
        let read_fd = AsRawFd::as_raw_fd(&read_end);
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
        //
        // c:Src/glob.c:1876-1880 + Src/exec.c — NOMATCH sets
        // ERRFLAG_ERROR but C's execlist clears the bit per-sublist
        // so subsequent commands run. Symmetric with the builtin
        // dispatcher's clear at fusevm_bridge.rs:299 — clear it here
        // too at the external-command post-command-boundary.
        if self.current_command_glob_failed.get() {
            self.current_command_glob_failed.set(false);
            crate::ported::utils::errflag.fetch_and(
                !crate::ported::zsh_h::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
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
        //
        // c:Src/Modules/*.c boot_/setup_ chain — module-bound builtins
        // (zsh/net/tcp → ztcp, zsh/net/socket → zsocket, zsh/zftp →
        // zftp, zsh/zpty → zpty) are only registered into `builtintab`
        // when the module is loaded via `zmodload`. Without a load,
        // execcmd_exec falls through to PATH and reports "command not
        // found" with exit 127. The Rust dispatcher previously routed
        // these names directly to `dispatch_builtin` regardless of
        // module state, so `zftp` etc. ran the builtin even with no
        // zmodload. Gate by `MODULESTAB.is_loaded(module)` so the
        // not-loaded case falls through to the PATH path below
        // (which then reports "command not found").
        let module_bound = |modname: &str| -> bool {
            crate::ported::module::MODULESTAB
                .lock()
                .map(|t| t.is_loaded(modname))
                .unwrap_or(false)
        };
        match cmd.as_str() {
            "sched" => return dispatch_builtin("sched", rest_vec.clone()),
            "echotc" => return dispatch_builtin("echotc", rest_vec.clone()),
            "echoti" => return dispatch_builtin("echoti", rest_vec.clone()),
            "zpty" if module_bound("zsh/zpty") => {
                return dispatch_builtin("zpty", rest_vec.clone())
            }
            "ztcp" if module_bound("zsh/net/tcp") => {
                return dispatch_builtin("ztcp", rest_vec.clone())
            }
            "zsocket" if module_bound("zsh/net/socket") => {
                // c:Src/Modules/socket.c:276 BUILTIN spec — BUILTINS["zsocket"]
                // optstr "ad:ltv" parsed by execbuiltin.
                return dispatch_builtin("zsocket", rest_vec.clone());
            }
            "zftp" if module_bound("zsh/zftp") => {
                return dispatch_builtin("zftp", rest_vec.clone())
            }
            "private" => {
                // c:Src/Modules/param_private.c:217 — bin_private via
                // BUILTINS["private"].
                return dispatch_builtin("private", rest_vec.clone());
            }
            "zformat" => return dispatch_builtin("zformat", rest_vec.clone()),
            "zregexparse" => return dispatch_builtin("zregexparse", rest_vec.clone()),
            // `unalias`/`unhash`/`unfunction` share `bin_unhash` but
            // each carries its own funcid (BIN_UNALIAS / BIN_UNHASH /
            // BIN_UNFUNCTION) — dispatch_builtin handles the BUILTINS
            // lookup + funcid propagation via execbuiltin.
            "unalias" | "unhash" | "unfunction" => {
                return dispatch_builtin(cmd.as_str(), rest_vec.clone());
            }
            // zsh-bundled rename helpers — implemented natively in
            // Rust so `autoload -U zmv` works without shipping the
            // function source. (Without this, the autoload path hangs.)
            "zmv" => return crate::extensions::ext_builtins::zmv(&rest_vec, "mv"),
            "zcp" => return crate::extensions::ext_builtins::zmv(&rest_vec, "cp"),
            "zln" => return crate::extensions::ext_builtins::zmv(&rest_vec, "ln"),
            "zcalc" => return crate::extensions::ext_builtins::zcalc(&rest_vec),
            "zselect" => {
                // Route through canonical dispatch_builtin which goes
                // via execbuiltin → BUILTINS["zselect"] (zselect.c:272).
                return dispatch_builtin("zselect", rest_vec.clone());
            }
            "cap" => return dispatch_builtin("cap", rest_vec.clone()),
            "getcap" => return dispatch_builtin("getcap", rest_vec.clone()),
            "setcap" => return dispatch_builtin("setcap", rest_vec.clone()),
            "yes" => return self.builtin_yes(&rest_vec),
            "nl" => return self.builtin_nl(&rest_vec),
            "env" => return self.builtin_env(&rest_vec),
            "printenv" => return self.builtin_printenv(&rest_vec),
            "tty" => return self.builtin_tty(&rest_vec),
            // c:Src/Modules/files.c:806 — BUILTINS["chgrp"] with
            // BIN_CHGRP funcid + "hRs" optstr.
            "chgrp" => return dispatch_builtin("chgrp", rest_vec.clone()),
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
            // zf_* aliases route through canonical BUILTINS entries
            // (files.c:816-824) — execbuiltin parses each fn's optstr
            // automatically.
            "mkdir" | "zf_mkdir" | "zf_rm" | "zf_rmdir" | "zf_chmod" | "zf_chown" | "zf_chgrp"
            | "zf_ln" | "zf_mv" | "zf_sync" => {
                return dispatch_builtin(cmd.as_str(), rest_vec.clone());
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
                // Canonical bin_stat per stat.c:638 via BUILTINS["zstat"].
                return dispatch_builtin("zstat", rest_vec.clone());
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
