//! fusevm bytecode-VM bridge for ShellExecutor.
//!
//! **Extension** — has no Src/exec.c counterpart. C zsh uses a
//! tree-walking interpreter (`Src/exec.c::execlist`). zshrs compiles
//! the parsed AST to fusevm bytecode and runs it on a stack VM; this
//! file holds the bridge between fusevm's `ShellHost` trait and our
//! `ShellExecutor` state, the thread-local executor pointer, all
//! `BUILTIN_*` opcode constants, and the giant `register_builtins`
//! handler table that wires zsh builtins onto fusevm CallBuiltin
//! opcodes.

#![allow(unused_imports)]

use crate::history::HistoryEngine;
// MathState is private to math.rs (no public state struct in math.c).
use crate::options::ZSH_OPTIONS_SET;
use crate::prompt::{expand_prompt, PromptContext};
// TcpSessions struct deleted — modules/tcp.rs uses ZTCP_SESSIONS thread_local.
use crate::zftp::Zftp;
// `Profiler` deleted — zprof state is module-level statics now.
use crate::zutil::StyleTable;
use compsys::cache::CompsysCache;
use compsys::CompInitResult;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::sync::LazyLock;
use indexmap::IndexMap;

use crate::ported::exec::*;
use crate::ported::jobs::JobState;
use crate::intercepts::{AdviceKind, Intercept, intercept_matches};
use std::io::Write;

// ═══════════════════════════════════════════════════════════════════════════
// Thread-local executor context for VM builtin dispatch
// ═══════════════════════════════════════════════════════════════════════════

use std::cell::{Cell, RefCell};
use crate::socket::bin_zsocket;

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

/// Fallible variant of `with_executor` — returns `None` when called
/// outside VM context (e.g. from a unit test that exercises subst_port
/// directly via `mk_state` without setting up an executor). Used by
/// pure-paramsubst code paths that have a fallback when the executor
/// isn't available, so they can run in unit tests without panicking.
pub(crate) fn try_with_executor<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut ShellExecutor) -> R,
{
    CURRENT_EXECUTOR.with(|cell| {
        let ptr = (*cell.borrow())?;
        // SAFETY: same as with_executor.
        let executor = unsafe { &mut *ptr };
        Some(f(executor))
    })
}



/// Register all zsh builtins with the VM.
pub(crate) fn register_builtins(vm: &mut fusevm::VM) {
    use fusevm::shell_builtins::*;
    use fusevm::Value;

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
        let status = with_executor(|exec| exec.bin_cd(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PWD, |vm, argc| {
        let args = pop_args(vm, argc);
        if let Some(s) = try_user_fn_override("pwd", &args) {
            return Value::Status(s);
        }
        let status = with_executor(|exec| exec.builtin_pwd_with_args(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ECHO, |vm, argc| {
        let args = pop_args(vm, argc);
        if let Some(s) = try_user_fn_override("echo", &args) {
            return Value::Status(s);
        }
        let status = with_executor(|exec| exec.builtin_echo(&args, &[]));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PRINT, |vm, argc| {
        let args = pop_args(vm, argc);
        if let Some(s) = try_user_fn_override("print", &args) {
            return Value::Status(s);
        }
        let status = with_executor(|exec| exec.bin_print(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PRINTF, |vm, argc| {
        let args = pop_args(vm, argc);
        if let Some(s) = try_user_fn_override("printf", &args) {
            return Value::Status(s);
        }
        let status = with_executor(|exec| exec.builtin_printf(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_EXPORT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_export(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_UNSET, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_unset(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_SOURCE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_dot(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_EXIT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_break("exit", &args));
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
        let status = with_executor(|exec| {
            // Sync executor.last_status to the VM's view BEFORE
            // bin_break("return") reads it for the no-arg fallback.
            exec.last_status = live_status;
            exec.bin_break("return", &args)
        });
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
        let status = with_executor(|exec| exec.bin_test(&args));
        Value::Status(status)
    });

    // Variable declaration
    vm.register_builtin(BUILTIN_LOCAL, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_local(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_TYPESET, |vm, argc| {
        let args = pop_args(vm, argc);
        // fusevm's builtin_id maps both `declare` and `typeset` to
        // BUILTIN_TYPESET, so this handler must default to the
        // typeset error-prefix. compile_zsh special-cases `declare`
        // to register BUILTIN_DECLARE explicitly so that path keeps
        // the `declare:` prefix in error messages.
        let status = with_executor(|exec| exec.bin_typeset(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_DECLARE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_declare(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_READONLY, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_readonly(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_INTEGER, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_integer(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_FLOAT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_float(&args));
        Value::Status(status)
    });

    // I/O
    vm.register_builtin(BUILTIN_READ, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_read(&args));
        Value::Status(status)
    });

    // Control flow
    vm.register_builtin(BUILTIN_BREAK, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_break("break", &args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_CONTINUE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_break("continue", &args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_SHIFT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_shift(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_EVAL, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_eval(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_EXEC, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_exec(&args, &[]));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_COMMAND, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_command(&args, &[]));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_BUILTIN, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_builtin(&args, &[]));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_LET, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_let(&args));
        Value::Status(status)
    });

    // Job control
    vm.register_builtin(BUILTIN_JOBS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_jobs(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_FG, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_fg(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_BG, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_bg(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_KILL, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_kill(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_DISOWN, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_disown(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_WAIT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_wait(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_SUSPEND, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_suspend(&args));
        Value::Status(status)
    });

    // History
    vm.register_builtin(BUILTIN_HISTORY, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_history(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_FC, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_fc(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_R, |vm, argc| {
        let args = pop_args(vm, argc);
        if let Some(s) = try_user_fn_override("r", &args) {
            return Value::Status(s);
        }
        let status = with_executor(|exec| exec.builtin_r(&args));
        Value::Status(status)
    });

    // Aliases
    vm.register_builtin(BUILTIN_ALIAS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_alias(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_UNALIAS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_unalias(&args));
        Value::Status(status)
    });

    // Options
    vm.register_builtin(BUILTIN_SET, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_set(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_SETOPT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_setopt("setopt", &args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_UNSETOPT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_setopt("unsetopt", &args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_SHOPT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_shopt(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_EMULATE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_emulate(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_GETOPTS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_getopts(&args));
        Value::Status(status)
    });

    // Functions / Autoload
    vm.register_builtin(BUILTIN_AUTOLOAD, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_autoload(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_FUNCTIONS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_functions(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_UNFUNCTION, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_unfunction(&args));
        Value::Status(status)
    });

    // Traps
    vm.register_builtin(BUILTIN_TRAP, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_trap(&args));
        Value::Status(status)
    });

    // Directory stack
    vm.register_builtin(BUILTIN_PUSHD, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_pushd(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_POPD, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_popd(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_DIRS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_dirs(&args));
        Value::Status(status)
    });

    // Type / Which / Hash
    vm.register_builtin(BUILTIN_TYPE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_type(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_WHENCE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_whence(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_WHERE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_where(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_WHICH, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_which(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_HASH, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_hash("hash", &args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_REHASH, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_hash("rehash", &args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_UNHASH, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_unhash(&args));
        Value::Status(status)
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
        let status = with_executor(|exec| exec.bin_compadd(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_COMPSET, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_compset(&args));
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
        let status = with_executor(|exec| exec.bin_zstyle(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZMODLOAD, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_zmodload(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_BINDKEY, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_bindkey(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZLE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_zle(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_VARED, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_vared(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZCOMPILE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_zcompile(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZFORMAT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_zformat(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZPARSEOPTS, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_zparseopts(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZREGEXPARSE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_zregexparse(&args));
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
        let status = with_executor(|exec| exec.bin_umask(&args));
        Value::Status(status)
    });

    // Misc
    vm.register_builtin(BUILTIN_TIMES, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_times(&args));
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
        let status = with_executor(|exec| exec.bin_enable("enable", &args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_DISABLE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_enable("disable", &args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_NOGLOB, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_noglob(&args, &[]));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_TTYCTL, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_ttyctl(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_SYNC, |vm, argc| {
        let args = pop_args(vm, argc);
        // Canonical bin_sync per files.c:53 — `sync(); return 0;`.
        use crate::ported::zsh_h::{options, MAX_OPS};
        let ops = options { ind: [0u8; MAX_OPS], args: Vec::new(),
                            argscount: 0, argsalloc: 0 };
        let status = crate::ported::modules::files::bin_sync(
            "sync", &args, &ops, 0);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_MKDIR, |vm, argc| {
        let args = pop_args(vm, argc);
        // Canonical bin_mkdir per files.c:63 — parses -m/-p inline
        // via the BUILTIN spec "pm:".
        use crate::ported::zsh_h::{options, MAX_OPS};
        let mut ops = options { ind: [0u8; MAX_OPS], args: Vec::new(),
                                argscount: 0, argsalloc: 0 };
        let mut positional: Vec<String> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if a == "--" { i += 1; positional.extend_from_slice(&args[i..]); break; }
            if let Some(rest) = a.strip_prefix('-') {
                if rest.is_empty() { positional.push(a.clone()); i += 1; continue; }
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
                            ops.args.push(args.get(i).cloned().unwrap_or_default());
                        }
                        ops.argscount = ops.args.len() as i32;
                        break;
                    }
                    if c.is_ascii_alphabetic() { ops.ind[c as usize] = 1; }
                    j += 1;
                }
            } else {
                positional.push(a.clone());
            }
            i += 1;
        }
        let status = crate::ported::modules::files::bin_mkdir(
            "mkdir", &positional, &ops, 0);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_STRFTIME, |vm, argc| {
        let args = pop_args(vm, argc);
        // Canonical bin_strftime takes (nam, argv, ops, func) per
        // Src/Modules/datetime.c:187. Adapt &[String] → &[&str] +
        // empty options inline (datetime parses no flags).
        use crate::ported::zsh_h::{options, MAX_OPS};
        let ops = options { ind: [0u8; MAX_OPS], args: Vec::new(),
                            argscount: 0, argsalloc: 0 };
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let status = crate::ported::modules::datetime::bin_strftime(
            "strftime", &argv, &ops, 0);
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZSLEEP, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_zsleep(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZSYSTEM, |vm, argc| {
        let args = pop_args(vm, argc);
        // bin_zsystem now takes the canonical C signature
        // (name, args, ops, func) per Src/Modules/system.c:806.
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(), argscount: 0, argsalloc: 0,
        };
        let _ = with_executor(|_exec| ());
        let status = crate::modules::system::bin_zsystem("zsystem", &args, &ops, 0);
        Value::Status(status)
    });

    // PCRE
    vm.register_builtin(BUILTIN_PCRE_COMPILE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_pcre_compile(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PCRE_MATCH, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_pcre_match(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PCRE_STUDY, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_pcre_study(&args));
        Value::Status(status)
    });

    // Database (GDBM)
    vm.register_builtin(BUILTIN_ZTIE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_ztie(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZUNTIE, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_zuntie(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_ZGDBMPATH, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.bin_zgdbmpath(&args));
        Value::Status(status)
    });

    // Prompt
    vm.register_builtin(BUILTIN_PROMPTINIT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_promptinit(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_PROMPT, |vm, argc| {
        let args = pop_args(vm, argc);
        let status = with_executor(|exec| exec.builtin_prompt(&args));
        Value::Status(status)
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
        use crate::ported::zsh_h::{options, MAX_OPS};
        let mut ops = options { ind: [0u8; MAX_OPS], args: Vec::new(),
                                argscount: 0, argsalloc: 0 };
        if args.iter().any(|a| a == "-c") { ops.ind[b'c' as usize] = 1; }
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
            let mut stage_vm = fusevm::VM::new(stages.into_iter().next().unwrap());
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
                    let mut stage_vm = fusevm::VM::new(chunk.clone());
                    register_builtins(&mut stage_vm);
                    let _ = stage_vm.run();
                    // Flush any buffered output before exiting
                    use std::io::Write;
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
            let mut stage_vm = fusevm::VM::new(last_chunk);
            register_builtins(&mut stage_vm);
            stage_vm.set_shell_host(Box::new(ZshrsHost));
            let _ = stage_vm.run();
            use std::io::Write;
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
        let pipefail_on =
            with_executor(|exec| exec.options.get("pipefail").copied().unwrap_or(false));
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
            exec.arrays.insert("pipestatus".to_string(), strs.clone());
            exec.arrays.insert("PIPESTATUS".to_string(), strs);
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
                    exec.variables.insert("!".to_string(), pid.to_string());
                    exec.jobs.add_pid_job(
                        pid,
                        String::new(),
                        crate::ported::jobs::JobState::Running,
                    );
                });
                Value::Status(0)
            }
        }
    });

    // ── Indexed-array storage and access ──────────────────────────────────
    //
    // Two calling conventions:
    //   1. `arr=(a b c)` → push "a", "b", "c", "arr"; CallBuiltin(SET_ARRAY, 4).
    //   2. `arr=($(cmd))` → push FlatArray, "arr"; CallBuiltin(SET_ARRAY, 2)
    //      where FlatArray is a Value::Array of words after BUILTIN_ARRAY_FLATTEN
    //      + WORD_SPLIT processing.
    // Both end with name as the LAST arg. Values may be a single Value::Array
    // (in which case we extract its elements) or a sequence of strings.
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
            // Refuse to mutate read-only arrays (declare -ra / typeset
            // -ra). zsh prints `read-only variable: NAME` and exits 1
            // in -c mode. Mirror that fatal behavior.
            let is_ro = exec.readonly_vars.contains(&name)
                || exec
                    .var_attrs
                    .get(&name)
                    .map(|a| a.readonly)
                    .unwrap_or(false);
            if is_ro {
                eprintln!("zshrs:1: read-only variable: {}", name);
                std::process::exit(1);
            }
            // Two-statement assoc init: `typeset -A m; m=(k v k v ...)`.
            if exec.assoc_arrays.contains_key(&name) {
                // zsh: odd number of values -> `bad set of key/value
                // pairs for associative array` exit 1, no
                // assignment. zshrs's `if let Some(v) = it.next()`
                // silently dropped the orphaned key.
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
                exec.assoc_arrays.insert(name.clone(), map);
                // PFA-SMR aspect: assoc bulk init `h=(k1 v1 k2 v2 ...)`.
                // Recorder emits a structured assoc event with the
                // ordered (key, value) pairs preserved in
                // `value_assoc` so replay can reconstruct the assoc
                // exactly — insertion order matters because zsh
                // associative arrays are insertion-ordered (via
                // IndexMap on the executor side).
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
            // Mirror array→scalar if name is the array side of a typeset -T tie.
            // `typeset -U arr` dedupes; first-wins per zsh.
            let is_unique = exec.var_attrs.get(&name).map(|a| a.unique).unwrap_or(false);
            if is_unique {
                let mut seen = std::collections::HashSet::new();
                values.retain(|v| seen.insert(v.clone()));
            }
            if let Some((scalar_name, sep)) = exec.tied_array_to_scalar.get(&name).cloned() {
                let joined = values.join(&sep);
                exec.variables.insert(scalar_name, joined);
                exec.arrays.insert(name.clone(), values.clone());
            } else {
                exec.variables.remove(&name);
                exec.arrays.insert(name.clone(), values.clone());
            }
            // PFA-SMR aspect: array SET (`name=(...)`). emit_path_or_assign
            // routes path-family names to per-element path_mod events
            // and everything else to one structured array `assign`
            // event with value_array = ordered elements (replay-safe).
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

    // `arr+=(d e f)` — append. Same calling conventions as SET_ARRAY.
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
            // Refuse appends on read-only arrays (declare -ra).
            let is_ro = exec.readonly_vars.contains(&name)
                || exec
                    .var_attrs
                    .get(&name)
                    .map(|a| a.readonly)
                    .unwrap_or(false);
            if is_ro {
                eprintln!("zshrs:1: read-only variable: {}", name);
                std::process::exit(1);
            }
            // Assoc-aware append: `typeset -A m; m+=(k1 v1 k2 v2 ...)`
            // adds key/value pairs. Without this, the values were
            // appended to a parallel array and `${m[k]}` lookup missed
            // the new keys entirely.
            if exec.assoc_arrays.contains_key(&name) {
                let map = exec.assoc_arrays.entry(name).or_default();
                let mut it = values.into_iter();
                while let Some(k) = it.next() {
                    if let Some(v) = it.next() {
                        map.insert(k, v);
                    }
                }
                return;
            }
            exec.variables.remove(&name);
            // `typeset -U arr` dedupes — append must respect existing
            // elements too. Skip values that are already present.
            // PFA-SMR aspect: array APPEND (`name+=(...)`). Same
            // routing as SET_ARRAY but with is_append=true so the
            // event carries the APPEND attr bit for replay.
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                let ctx = exec.recorder_ctx();
                let attrs = exec.recorder_attrs_for(&name);
                emit_path_or_assign(&name, &values, attrs, true, &ctx);
            }
            let is_unique = exec.var_attrs.get(&name).map(|a| a.unique).unwrap_or(false);
            // Mirror the post-append result back to a tied scalar
            // (`typeset -T PATH path :` — `path+=(/x)` must update
            // `PATH` too). Without this, zinit / OMZ patterns like
            // `path+=(/some/dir)` left $PATH stale, so `command -v`
            // / pathprog lookups missed newly-added dirs.
            let tied_scalar = exec.tied_array_to_scalar.get(&name).cloned();
            let target = exec.arrays.entry(name.clone()).or_insert_with(Vec::new);
            if is_unique {
                let existing: std::collections::HashSet<String> = target.iter().cloned().collect();
                for v in values {
                    if !existing.contains(&v) {
                        target.push(v);
                    }
                }
            } else {
                target.extend(values);
            }
            if let Some((scalar_name, sep)) = tied_scalar {
                let joined = exec
                    .arrays
                    .get(&name)
                    .map(|a| a.join(&sep))
                    .unwrap_or_default();
                exec.variables.insert(scalar_name.clone(), joined.clone());
                // Keep the env var (PATH / FPATH / MANPATH / …) in
                // sync with the scalar so child processes see the
                // change.
                std::env::set_var(&scalar_name, &joined);
            }
        });
        Value::Status(0)
    });

    // `select var in words; do body; done` — interactive menu loop. Stack
    // discipline (top-down): sub_chunk_idx (Int), var_name (str), word_N..word_1.
    // Argc = words_count + 2. We pop in reverse order: idx first, then name,
    // then words back to source order via reverse().
    //
    // Loop body:
    //   1. Print numbered menu to stderr.
    //   2. Print PROMPT3 (default "?# ") to stderr.
    //   3. Read line from stdin.
    //   4. EOF (read fails) → break, return Status(0).
    //   5. Empty line → redraw menu, loop.
    //   6. Numeric input in 1..=N → set var, run sub-chunk, capture status,
    //      redraw menu, loop.
    //   7. Anything else → set var to "" (zsh convention), run sub-chunk,
    //      redraw menu, loop. The body sees REPLY = the raw input.
    //
    // `break` inside the body short-circuits via the sub-chunk's own bytecode
    // (the break_patches mechanism). When the sub-chunk halts via break it
    // returns from VM::run; we treat any non-zero status as "loop should
    // exit"? No — break sets a flag in the chunk-level patches. Since we're
    // running the body in a fresh VM each iteration, break needs a different
    // signaling mechanism. For now: the body's bytecode can do `return 99`
    // which we recognize as a "user wants out" signal. zsh's `break` works
    // in select via the same loop-control mechanism as for/while. Phase G6
    // follow-up.
    vm.register_builtin(BUILTIN_RUN_SELECT, |vm, argc| {
        use std::io::{BufRead, Write};

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

        let prompt = with_executor(|exec| {
            exec.variables
                .get("PROMPT3")
                .cloned()
                .unwrap_or_else(|| "?# ".to_string())
        });

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
                exec.variables.insert("REPLY".to_string(), trimmed.clone());
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
                exec.variables.insert(name.clone(), chosen);
            });

            // Reset the loop signal before running the body so a stale
            // value from a sibling construct doesn't leak in.
            with_executor(|exec| exec.loop_signal = None);

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
                exec.variables
                    .remove("BREAK_SELECT")
                    .map(|v| v != "0" && !v.is_empty())
                    .unwrap_or(false)
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
    fn magic_assoc_lookup(name: &str, idx: &str) -> Option<Value> {
        // Subscript-flag lookup `(r)pat` / `(R)pat` / `(i)pat` /
        // `(I)pat` on a magic-assoc — synthesize the (key,value)
        // pair list from get_special_array_value and route through
        // the assoc-flag matcher (same path real assocs use).
        // Direct port of Src/params.c getarg's hash-aware index/
        // match handling — without this, `${aliases[(I)foo*]}` and
        // friends were passing the literal `(I)foo*` text through
        // as the key.
        // Magic-assoc subscript flags (I)/(R)/(i)/(r): parse the
        // leading `(...)` flag tag and dispatch by-key (I/i) or
        // by-value (R/r) glob match. Capital = return all matches
        // joined by space; lowercase = return first only.
        // Direct port of Src/params.c getarg path which routes
        // hash subscripts through pattern matching when the flag
        // tag is present.
        let parsed_flags: Option<(String, String)> = (|s: &str| {
            let s = s.trim_start();
            let rest = s.strip_prefix('(')?;
            let close = rest.find(')')?;
            let flags = rest[..close].to_string();
            let pat = rest[close + 1..].to_string();
            if flags.chars().next().is_some_and(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'k' | 'K' | 'n' | 'e' | 'b' | 'w' | 'f' | 'p' | 's')) {
                Some((flags, pat))
            } else { None }
        })(idx);
        if let Some((flags, pat)) = parsed_flags.clone() {
            let pairs = with_executor(|exec| -> Option<Vec<(String, String)>> {
                let keys = crate::exec_shims::scan_magic_assoc_keys(name)?;
                Some(keys
                    .into_iter()
                    .map(|k| {
                        let v = exec
                            .get_special_array_value(name, &k)
                            .unwrap_or_default();
                        (k, v)
                    })
                    .collect())
            });
            if let Some(pairs) = pairs {
                let by_key = flags.contains('I') || flags.contains('i');
                let return_all = flags.contains('I') || flags.contains('R');
                let mut out: Vec<String> = Vec::new();
                for (k, v) in &pairs {
                    let hay = if by_key { k } else { v };
                    if ShellExecutor::glob_match_static(hay, &pat) {
                        out.push(if by_key { k.clone() } else { v.clone() });
                        if !return_all { break; }
                    }
                }
                return Some(Value::str(out.join(" ")));
            }
        }
        with_executor(|exec| -> Option<Value> {
            match name {
                "commands" => {
                    if idx == "@" || idx == "*" {
                        return Some(Value::Array(
                            exec.command_hash.values().map(Value::str).collect(),
                        ));
                    }
                    Some(Value::str(
                        exec.command_hash.get(idx).cloned().unwrap_or_else(|| {
                            // Fall back to PATH scan for first match
                            for dir in env::var("PATH").unwrap_or_default().split(':') {
                                let p = std::path::PathBuf::from(dir).join(idx);
                                if p.is_file() {
                                    return p.to_string_lossy().into_owned();
                                }
                            }
                            String::new()
                        }),
                    ))
                }
                "aliases" => Some(Value::str(
                    exec.aliases.get(idx).cloned().unwrap_or_default(),
                )),
                "galiases" => Some(Value::str(
                    exec.global_aliases.get(idx).cloned().unwrap_or_default(),
                )),
                "saliases" => Some(Value::str(
                    exec.suffix_aliases.get(idx).cloned().unwrap_or_default(),
                )),
                "functions" => {
                    if let Some(text) = exec.function_definition_text(idx) {
                        // zsh's `$functions[name]` returns the function
                        // body with each statement on its own line and a
                        // leading TAB on every line (no trailing `;`).
                        // Was returning the raw user-typed source which
                        // diverges on indent and terminator. Direct port
                        // of Src/exec.c's `getfn_functions` formatter.
                        let formatted = FuncBodyFmt::render(text.trim());
                        Some(Value::str(format!("\t{}", formatted)))
                    } else {
                        Some(Value::str(""))
                    }
                }
                "dis_functions" => {
                    // Disabled functions table — zshrs tracks via autoload_pending
                    // for the autoload-but-not-loaded case; full disable list
                    // would need a separate table. For now: empty unless
                    // explicitly disabled.
                    Some(Value::str(""))
                }
                "builtins" => {
                    // Return "defined" for known builtins; empty for unknown
                    let known = matches!(
                        idx,
                        "echo"
                            | "print"
                            | "printf"
                            | "cd"
                            | "pwd"
                            | "exit"
                            | "return"
                            | "true"
                            | "false"
                            | ":"
                            | "test"
                            | "["
                            | "local"
                            | "private"
                            | "declare"
                            | "typeset"
                            | "read"
                            | "shift"
                            | "eval"
                            | "alias"
                            | "unalias"
                            | "set"
                            | "unset"
                            | "export"
                            | "source"
                            | "."
                            | "history"
                            | "fc"
                            | "jobs"
                            | "fg"
                            | "bg"
                            | "kill"
                            | "wait"
                            | "trap"
                            | "ulimit"
                            | "umask"
                            | "hash"
                            | "unhash"
                            | "type"
                            | "whence"
                            | "which"
                            | "where"
                            | "command"
                            | "builtin"
                            | "exec"
                            | "getopts"
                            | "let"
                            | "setopt"
                            | "unsetopt"
                            | "emulate"
                            | "zstyle"
                            | "compdef"
                            | "compadd"
                            | "compinit"
                            | "compset"
                    );
                    if known {
                        Some(Value::str("defined"))
                    } else {
                        Some(Value::str(""))
                    }
                }
                "reswords" => {
                    let known = matches!(
                        idx,
                        "if" | "then"
                            | "elif"
                            | "else"
                            | "fi"
                            | "for"
                            | "do"
                            | "done"
                            | "while"
                            | "until"
                            | "case"
                            | "esac"
                            | "in"
                            | "function"
                            | "select"
                            | "time"
                            | "{"
                            | "}"
                            | "[["
                            | "]]"
                            | "!"
                            | "coproc"
                            | "always"
                            | "foreach"
                            | "end"
                            | "repeat"
                            | "nocorrect"
                            | "noglob"
                            | "declare"
                            | "typeset"
                            | "local"
                            | "readonly"
                            | "export"
                            | "integer"
                            | "float"
                    );
                    if known {
                        Some(Value::str("reserved"))
                    } else {
                        Some(Value::str(""))
                    }
                }
                "options" => {
                    let opt_name = idx.to_lowercase().replace('_', "");
                    Some(Value::str(
                        if exec.options.get(&opt_name).copied().unwrap_or(false) {
                            "on"
                        } else {
                            "off"
                        },
                    ))
                }
                "parameters" => {
                    // ${parameters[name]} returns the type with all
                    // attributes joined by `-` (zsh `paramtypes` per
                    // VarAttr::format_zsh). Falls back to base kind
                    // when there's no attr entry yet (e.g. inherited
                    // env or implicit assignment).
                    if let Some(attr) = exec.var_attrs.get(idx) {
                        Some(Value::str(attr.format_zsh()))
                    } else if exec.assoc_arrays.contains_key(idx) {
                        Some(Value::str("association"))
                    } else if exec.arrays.contains_key(idx) {
                        Some(Value::str("array"))
                    } else if exec.variables.contains_key(idx) || env::var(idx).is_ok() {
                        Some(Value::str("scalar"))
                    } else {
                        Some(Value::str(""))
                    }
                }
                "jobtexts" => {
                    let job_id: usize = idx.parse().ok()?;
                    Some(Value::str(
                        exec.jobs
                            .get(job_id)
                            .map(|j| j.command.clone())
                            .unwrap_or_default(),
                    ))
                }
                "jobdirs" => {
                    let _job_id: usize = idx.parse().ok()?;
                    // Per-job working dir not tracked; return current cwd as
                    // a useful approximation (zsh tracks it; we don't yet).
                    Some(Value::str(
                        std::env::current_dir()
                            .ok()
                            .and_then(|p| p.to_str().map(String::from))
                            .unwrap_or_default(),
                    ))
                }
                "jobstates" => {
                    let job_id: usize = idx.parse().ok()?;
                    Some(Value::str(
                        exec.jobs
                            .get(job_id)
                            .map(|j| match j.state {
                                JobState::Running => "running".to_string(),
                                JobState::Stopped => "stopped".to_string(),
                                JobState::Done => "done".to_string(),
                            })
                            .unwrap_or_default(),
                    ))
                }
                "nameddirs" => Some(Value::str(
                    exec.named_dirs
                        .get(idx)
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )),
                "userdirs" => {
                    // ~user → home dir lookup via /etc/passwd. No caching;
                    // each lookup hits getpwnam.
                    let c_user = match std::ffi::CString::new(idx) {
                        Ok(c) => c,
                        Err(_) => return Some(Value::str("")),
                    };
                    let pw = unsafe { libc::getpwnam(c_user.as_ptr()) };
                    if pw.is_null() {
                        Some(Value::str(""))
                    } else {
                        let home_ptr = unsafe { (*pw).pw_dir };
                        if home_ptr.is_null() {
                            return Some(Value::str(""));
                        }
                        let home = unsafe { std::ffi::CStr::from_ptr(home_ptr) };
                        Some(Value::str(home.to_string_lossy().into_owned()))
                    }
                }
                "modules" => {
                    // Loaded modules — compiled-in always-loaded plus
                    // anything zmodload registered via the
                    // `_module_<name>` option flag (see
                    // bin_zmodload). Same source as the
                    // magic_assoc_lookup path so both `${modules[X]}`
                    // and `${(t)modules[X]}` agree.
                    const ALWAYS_LOADED: &[&str] = &[
                        "zsh/datetime",
                        "zsh/sched",
                        "zsh/zutil",
                        "zsh/parameter",
                        "zsh/files",
                        "zsh/complete",
                        "zsh/complist",
                        "zsh/regex",
                        "zsh/system",
                        "zsh/stat",
                        "zsh/net/tcp",
                        "zsh/net/socket",
                        "zsh/private",
                        "zsh/zftp",
                        "zsh/zselect",
                        "zsh/zle",
                        "zsh/random",
                        "zsh/pcre",
                        "zsh/db/gdbm",
                        "zsh/cap",
                        "zsh/clone",
                        "zsh/curses",
                        "zsh/mapfile",
                        "zsh/nearcolor",
                        "zsh/newuser",
                        "zsh/mathfunc",
                        "zsh/termcap",
                        "zsh/terminfo",
                        "zsh/profiler",
                    ];
                    let loaded = ALWAYS_LOADED.contains(&idx)
                        || exec
                            .options
                            .get(&format!("_module_{}", idx))
                            .copied()
                            .unwrap_or(false);
                    Some(Value::str(if loaded { "loaded" } else { "" }))
                }
                "patchars" => Some(Value::str("*?[]<>(){}|^&;")),
                "widgets" => {
                    // ${widgets[name]} → 'builtin' or 'user:func' per
                    // zleparameter.c widgets_*. Mirrors the
                    // magic_assoc_lookup path so both lookup sites
                    // agree.
                    use crate::zle::zle;
                    let zle = zle();
                    if let Some(target) = zle.get_widget(idx) {
                        if target == idx {
                            Some(Value::str("builtin"))
                        } else {
                            Some(Value::str(format!("user:{}", target)))
                        }
                    } else {
                        Some(Value::str(""))
                    }
                }
                "keymaps" => {
                    // ${keymaps[name]} → "1" or "" per zleparameter.c
                    // keymaps_*. Same canonical seven names as the
                    // magic_assoc path.
                    let known = matches!(
                        idx,
                        "main" | "emacs" | "viins" | "vicmd" | "isearch" | "command" | "menuselect"
                    );
                    if known {
                        Some(Value::str("1"))
                    } else {
                        Some(Value::str(""))
                    }
                }
                "mapfile" => {
                    // zsh/mapfile module: `${mapfile[/path]}` reads a
                    // file's bytes verbatim. Trailing newline is
                    // preserved (verified against real zsh: a one-line
                    // "test\n" file gives len=5, not 4). Downstream
                    // (f)/(@f) flags handle the trailing-newline split.
                    if idx == "@" || idx == "*" {
                        // Splice: not meaningful for mapfile (the whole
                        // filesystem isn't enumerable). Return empty.
                        return Some(Value::Array(vec![]));
                    }
                    match std::fs::read_to_string(idx) {
                        Ok(s) => Some(Value::str(s)),
                        Err(_) => Some(Value::str("")),
                    }
                }
                "sysparams" => {
                    // zsh/system module: `${sysparams[KEY]}` magic
                    // assoc with three keys per zshmodules(1): `pid`,
                    // `ppid`, `procsubstpid`. Returns the appropriate
                    // process ID. Splice form returns the value list.
                    let pid_str = std::process::id().to_string();
                    let ppid_str = unsafe { libc::getppid() }.to_string();
                    if idx == "@" || idx == "*" {
                        return Some(Value::Array(vec![
                            Value::str(pid_str),
                            Value::str(ppid_str),
                        ]));
                    }
                    match idx {
                        "pid" => Some(Value::str(pid_str)),
                        "ppid" => Some(Value::str(ppid_str)),
                        "procsubstpid" => Some(Value::str("0")),
                        _ => Some(Value::str("")),
                    }
                }
                "epochtime" => {
                    // zsh/datetime — `${epochtime}` is a 2-element
                    // indexed array: [seconds, nanoseconds] from
                    // clock_gettime(CLOCK_REALTIME). Direct port of
                    // the `epochtimegetfn` accessor in
                    // Src/Modules/datetime.c (struct gsu_array).
                    use std::time::{SystemTime, UNIX_EPOCH};
                    let (secs, nsecs) = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| (d.as_secs() as i64, d.subsec_nanos() as i64))
                        .unwrap_or((0, 0));
                    if idx == "@" || idx == "*" {
                        return Some(Value::Array(vec![
                            Value::str(secs.to_string()),
                            Value::str(nsecs.to_string()),
                        ]));
                    }
                    if let Ok(n) = idx.parse::<i64>() {
                        let pos = if n > 0 {
                            (n - 1) as usize
                        } else if n < 0 {
                            let p = 2 + n;
                            if p < 0 {
                                return Some(Value::str(""));
                            }
                            p as usize
                        } else {
                            return Some(Value::str(""));
                        };
                        return match pos {
                            0 => Some(Value::str(secs.to_string())),
                            1 => Some(Value::str(nsecs.to_string())),
                            _ => Some(Value::str("")),
                        };
                    }
                    Some(Value::str(""))
                }
                "termcap" => {
                    // `${termcap[cap]}` — direct port of
                    // `gettermcap()` from Src/Modules/termcap.c:144.
                    // Backed by ncurses' termcap-emulation API
                    // (`tgetent`/`tgetstr`/`tgetnum`/`tgetflag`)
                    // which resolves from the same database
                    // `${terminfo[…]}` uses but with the legacy
                    // 2-letter cap names.
                    Some(Value::str(
                        crate::modules::termcap::gettermcap(idx).unwrap_or_default(),
                    ))
                }
                "terminfo" => {
                    // `${terminfo[capname]}` — direct port of
                    // `getterminfo()` from Src/Modules/terminfo.c:135.
                    // Lazy ncurses tigetstr/tigetnum/tigetflag lookup
                    // for any capability the script names. The
                    // executor also pre-seeds the common subset
                    // into `assoc_arrays["terminfo"]` so
                    // `${(k)terminfo}` enumerates the seeded names —
                    // but the magic-assoc path runs FIRST (per the
                    // `user_defined_assoc` gate at line 3108), so
                    // for INDEX lookups we always reach `lookup()`
                    // and uncommon caps like `bel` resolve correctly.
                    Some(Value::str(
                        crate::modules::terminfo::getterminfo(idx).unwrap_or_default(),
                    ))
                }
                "errnos" => {
                    // zsh/system module: `${errnos[N]}` is an INDEXED
                    // array of errno-name strings, 1-based. Direct
                    // port of the `SPECIALPMDEF("errnos", PM_ARRAY|
                    // PM_READONLY, …)` entry at
                    // Src/Modules/system.c:902 + the `errnosgetfn`
                    // accessor at line 832 (which returns
                    // `arrdup((char **)sys_errnames)`). Splice (`@`/
                    // `*`) returns the whole platform-specific list
                    // as a Value::Array; numeric subscript returns
                    // the matching name (or "" for unknown).
                    let table = crate::modules::system::ERRNO_NAMES;
                    if idx == "@" || idx == "*" {
                        return Some(Value::Array(
                            table.iter().map(|(n, _)| Value::str(*n)).collect(),
                        ));
                    }
                    if let Ok(n) = idx.parse::<i64>() {
                        // 1-based. Negative indices count from end.
                        let len = table.len() as i64;
                        let pos = if n > 0 {
                            (n - 1) as usize
                        } else if n < 0 {
                            let p = len + n;
                            if p < 0 {
                                return Some(Value::str(""));
                            }
                            p as usize
                        } else {
                            return Some(Value::str(""));
                        };
                        if let Some((name, _)) = table.get(pos) {
                            return Some(Value::str(*name));
                        }
                    }
                    Some(Value::str(""))
                }
                // `langinfo` — port of zsh/langinfo module
                // (src/zsh/Src/Modules/langinfo.c:402-449). Read-
                // only assoc keyed by nl_item names (CODESET,
                // D_FMT, RADIXCHAR, etc.); each lookup goes through
                // nl_langinfo(3). Splice (`@`/`*`) returns all the
                // names known to the module's static table.
                "langinfo" => {
                    if idx == "@" || idx == "*" {
                        return Some(Value::Array(
                            crate::langinfo::NL_NAMES
                                .iter()
                                .map(|s| Value::str(*s))
                                .collect(),
                        ));
                    }
                    let val = crate::langinfo::getlanginfo(idx).unwrap_or_default();
                    Some(Value::str(val))
                }
                // `.zle.esc` and `.zle.sgr` — port of zsh/hlgroup
                // module (src/zsh/Src/Modules/hlgroup.c:81-165).
                // Both back into the user's `.zle.hlgroups` assoc.
                // `.zle.esc[name]` returns the FULL escape sequence
                // for the highlight-group; `.zle.sgr[name]` returns
                // just the digit run (after stripping `\033[` and
                // trailing `m`). hlgroup.c:39-78 convertattr does
                // both modes.
                ".zle.esc" | ".zle.sgr" => {
                    let sgr = name == ".zle.sgr";
                    // Look up `.zle.hlgroups[idx]` — the user's
                    // attribute string per hlgroup.c:96-99 (var =
                    // GROUPVAR i.e. ".zle.hlgroups").
                    let attr = exec
                        .assoc_arrays
                        .get(".zle.hlgroups")
                        .and_then(|m| m.get(idx))
                        .cloned()
                        .unwrap_or_default();
                    if attr.is_empty() {
                        // Per hlgroup.c:101-103, missing/unset entry
                        // returns an empty string (PM_UNSET).
                        return Some(Value::str(""));
                    }
                    let converted = crate::hlgroup::convertattr(&attr, sgr);
                    Some(Value::str(converted))
                }
                _ => None,
            }
        })
    }

    // `${arr[idx]}` — pop name, then idx_str. zsh is 1-based for positive
    // indices; we honor that. `@`/`*` return the whole array as Value::Array
    // so Op::Exec splice produces N argv slots. For `${foo[key]}` where foo
    // is an assoc, the idx is a string key — we check assoc_arrays first
    // when the idx isn't `@`/`*` and the name has an assoc binding.
    vm.register_builtin(BUILTIN_ARRAY_INDEX, |vm, _argc| {
        let mut idx = vm.pop().to_str();
        let name = vm.pop().to_str();
        // `\u{02}` prefix on idx = "compile-time DQ context" — set by
        // the compile_zsh fast path when the ${arr[KEY]} appeared
        // inside `"…"`. The runtime needs this to decide whether
        // a `[N,M]` range slice should join (DQ) or stay as array
        // (unquoted). The mode-1 BUILTIN_EXPAND_TEXT bridge already
        // bumps `exec.in_dq_context`, so detect either signal.
        let dq_compile = idx.starts_with('\u{02}');
        if dq_compile {
            idx = idx[1..].to_string();
        }
        // `\u{05}` prefix on idx = "(@) flag is set in surrounding
        // flag chain" — emitted by parse_zsh_flag_subscript when the
        // outer flag chain contains `@`. Direct port of zsh's
        // nojoin behavior: `(@)` overrides the DQ-join even inside
        // `"…"`. When this sentinel is present, force array shape
        // for slices regardless of in_dq_context.
        let force_array = idx.starts_with('\u{05}');
        if force_array {
            idx = idx[1..].to_string();
        }
        // `\u{06}` prefix = "outer (v) flag wants values for matching
        // assoc keys" — flip the (I)/(i) subscript-flag from
        // returning keys to returning the corresponding values.
        // Direct port of zsh's (v)+(I) combo.
        let flip_to_values = idx.starts_with('\u{06}');
        if flip_to_values {
            idx = idx[1..].to_string();
        }
        // `\u{07}` prefix = "outer (k) flag wants keys for matching
        // assoc values" — flip the (R)/(r) subscript-flag from
        // returning values to returning the corresponding keys.
        let flip_to_keys = idx.starts_with('\u{07}');
        if flip_to_keys {
            idx = idx[1..].to_string();
        }
        // Pre-expand `$((arith))` / `$VAR` / `$(cmd)` references in
        // the subscript text so downstream slice / index logic sees
        // numeric literals it can parse. The compile path passes the
        // raw subscript text as a constant; without expansion, a key
        // like `$((1+1)),-1` failed `parse::<i64>()` for the lower
        // bound and the whole slice fell back to scalar concat.
        // Special-flag keys `(I)pat` / `(R)pat` skip this — those
        // already get their `$VAR` resolution inside the matchers.
        if idx.contains('$')
            && !idx.starts_with("(I)")
            && !idx.starts_with("(i)")
            && !idx.starts_with("(R)")
            && !idx.starts_with("(r)")
            && !idx.starts_with("(K)")
            && !idx.starts_with("(k)")
        {
            idx = with_executor(|exec| exec.singsub(&idx));
        }
        // `${pipestatus[N]}` / `${PIPESTATUS[N]}` — pipeline exit
        // status array. Populated by BUILTIN_PIPELINE_EXEC after a
        // real pipeline; for single commands fall back to a synthetic
        // [last_status] list so `true; echo $pipestatus[1]` prints 0.
        // After a non-pipeline command runs, the prior pipestatus
        // array becomes stale (zsh resets pipestatus to a single-
        // element array on every command). Detect by comparing the
        // last element to last_status; if they diverge, fall back
        // to the synthetic [last_status] form so e.g.
        //   true | false; echo "$?"; echo "$pipestatus"
        // prints "0" (just the echo's status), not "0 1".
        if name == "pipestatus" || name == "PIPESTATUS" {
            let arr = with_executor(|exec| {
                let cached = exec.arrays.get(&name).cloned();
                let last = exec.last_status.to_string();
                match cached {
                    Some(arr)
                        if arr.last().map(|s| s.as_str()) == Some(last.as_str()) =>
                    {
                        arr
                    }
                    _ => vec![last],
                }
            });
            if let Ok(i) = idx.parse::<i64>() {
                let len = arr.len() as i64;
                let resolved = if i > 0 {
                    (i - 1) as usize
                } else if i < 0 {
                    let off = len + i;
                    if off < 0 {
                        return Value::str("");
                    }
                    off as usize
                } else {
                    return Value::str("");
                };
                return Value::str(arr.get(resolved).cloned().unwrap_or_default());
            }
            if idx == "@" || idx == "*" {
                return Value::Array(arr.into_iter().map(Value::str).collect());
            }
        }

        // Special-name positional-param indexing. `${@[N]}`, `${@[N,M]}`,
        // `${*[N]}`, `${argv[N]}` all index the positional-param array
        // 1-based (zsh semantics). Without this, `@`/`*`/`argv` fall
        // through to the scalar-slice path which slices the joined
        // string instead.
        if matches!(name.as_str(), "@" | "*" | "argv") {
            let arr = with_executor(|exec| exec.positional_params.clone());
            // Slice form `N,M`.
            if let Some((s_str, e_str)) = idx.split_once(',') {
                let s_opt: Option<i64> = s_str.trim().parse().ok();
                let e_opt: Option<i64> = e_str.trim().parse().ok();
                if let (Some(s), Some(e)) = (s_opt, e_opt) {
                    return Value::Array(
                        getarrvalue(&arr, s, e)
                            .into_iter()
                            .map(Value::str)
                            .collect(),
                    );
                }
            }
            // Single index.
            if let Ok(i) = idx.parse::<i64>() {
                let len = arr.len() as i64;
                let resolved = if i > 0 {
                    (i - 1) as usize
                } else if i < 0 {
                    let off = len + i;
                    if off < 0 {
                        return Value::str("");
                    }
                    off as usize
                } else {
                    return Value::str("");
                };
                return Value::str(arr.get(resolved).cloned().unwrap_or_default());
            }
            // Subscript-flag form on positional params: route through
            // getarg with positional_params as the array. Matches
            // zsh's `${@[(I)pat]}` / `${@[(r)pat]}` semantics.
            if idx.starts_with('(') {
                if let Some(crate::ported::params::GetargOut::Value(v)) =
                    crate::ported::params::getarg(&idx, Some(&arr), None, None)
                {
                    return v;
                }
            }
        }
        // Magic special-parameter assoc lookups — synthesized from shell
        // state on access. zsh exposes shell-introspection assocs like
        // `${commands[ls]}`, `${aliases[ll]}`, `${functions[foo]}`,
        // `${options[interactive]}`, etc. None of these are stored in
        // `assoc_arrays`; we generate the value at lookup time.
        //
        // BUT: if the user declared `typeset -A NAME` and assigned
        // values, their declaration wins. This matches zsh's actual
        // module behavior (verified against /bin/zsh): `typeset -A
        // langinfo; langinfo[CODESET]=UTF-8; echo $langinfo[CODESET]`
        // prints `UTF-8` even though `zsh/langinfo` would normally
        // shadow it with nl_langinfo(3). The C source enforces this
        // via the module loader: `bin_zmodload` only registers the
        // special-parameter table entry when no existing assoc with
        // that name exists. Mirroring: skip the magic path if
        // `name` is already in `assoc_arrays`.
        let user_defined_assoc =
            with_executor(|exec| exec.assoc_arrays.contains_key(&name));
        if !user_defined_assoc {
            if let Some(v) = magic_assoc_lookup(&name, &idx) {
                // Magic-assoc with `(I)pat` glob-match returned an
                // Array of matching keys. In DQ context (the user
                // wrote `"${aliases[(I)foo*]}"`), zsh joins array
                // results with the first IFS char per Src/subst.c
                // paramsubst's `nojoin` gating. Without this the
                // outer DQ-string was treating the array as a
                // splice and emitting one arg per matching key.
                if dq_compile {
                    if let Value::Array(items) = &v {
                        let strs: Vec<String> =
                            items.iter().map(|i| i.to_str()).collect();
                        let sep = with_executor(|exec| {
                            exec.variables
                                .get("IFS")
                                .and_then(|s| s.chars().next())
                                .unwrap_or(' ')
                        });
                        return Value::str(strs.join(&sep.to_string()));
                    }
                }
                return v;
            }
        }
        with_executor(|exec| match idx.as_str() {
            "@" | "*" => {
                // Splice: assoc → values list (zsh's `${foo[@]}` for assoc);
                // indexed → element list. For assoc the order of values is
                // implementation-defined (matches HashMap iteration).
                if let Some(map) = exec.assoc_arrays.get(&name) {
                    return Value::Array(map.values().map(Value::str).collect());
                }
                match exec.arrays.get(&name) {
                    Some(v) => Value::Array(v.iter().map(Value::str).collect()),
                    None => Value::Array(vec![]),
                }
            }
            _ => {
                // Assoc lookup wins if the name is in assoc_arrays — the user
                // declared it via `typeset -A` or assigned via foo[key]=val.
                // Subscript flags (k)/(v) on assoc handled separately by
                // BUILTIN_PARAM_FLAG; (r)/(R)/(i)/(I) on assoc would search
                // values/keys, supported below.
                if let Some(map) = exec.assoc_arrays.get(&name) {
                    if let Some((flags, pat)) = (|s: &str| -> Option<(String, String)> {
                        // Port of subst.c subscript-flag parser:
                        // `(I)pat` / `(R)pat` / `(i)pat` / `(r)pat`.
                        // Returns (flags_chars, pattern_after).
                        let s = s.trim_start();
                        let rest = s.strip_prefix('(')?;
                        let close = rest.find(')')?;
                        let flags = rest[..close].to_string();
                        let pat = rest[close + 1..].to_string();
                        if flags.chars().next().is_some_and(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'k' | 'K' | 'n' | 'e' | 'b' | 'w' | 'f' | 'p' | 's')) {
                            Some((flags, pat))
                        } else { None }
                    })(&idx) {
                        // (v)+(I)/(i): subscript searches keys but
                        // outer wants values. Iterate the assoc and
                        // return values for keys that match `pat`.
                        if flip_to_values
                            && (flags.contains('I') || flags.contains('i'))
                        {
                            let return_all = flags.contains('I');
                            let mut out: Vec<String> = Vec::new();
                            for (k, v) in map.iter() {
                                if ShellExecutor::glob_match_static(k, &pat) {
                                    out.push(v.clone());
                                    if !return_all {
                                        break;
                                    }
                                }
                            }
                            return Value::str(out.join(" "));
                        }
                        // (k)+(R)/(r): subscript searches values but
                        // outer wants keys. Iterate the assoc and
                        // return keys whose values match.
                        if flip_to_keys
                            && (flags.contains('R') || flags.contains('r'))
                        {
                            let return_all = flags.contains('R');
                            let mut out: Vec<String> = Vec::new();
                            for (k, v) in map.iter() {
                                if ShellExecutor::glob_match_static(v, &pat) {
                                    out.push(k.clone());
                                    if !return_all {
                                        break;
                                    }
                                }
                            }
                            return Value::str(out.join(" "));
                        }
                        // Default flag handling — route to getarg's
                        // hash-search arm (params.c:1581-1660).
                        match crate::ported::params::getarg(&idx, None, Some(map), None) {
                            Some(crate::ported::params::GetargOut::Value(v)) => return v,
                            _ => {}
                        }
                    }
                    return Value::str(map.get(&idx).cloned().unwrap_or_default());
                }

                let arr = match exec.arrays.get(&name) {
                    Some(a) => a.clone(),
                    None => {
                        // Fall back to scalar subscripting on `variables`.
                        // zsh treats `${str[N]}` and `${str[N,M]}` as
                        // 1-based char indexing. Subscript flags
                        // `(w)`/`(s/sep/)` on scalars split before
                        // indexing — direct port of zsh's
                        // zshparam(1) "Subscript Flags" `w` and `s`.
                        let scalar = exec.get_variable(&name);
                        if scalar.is_empty() {
                            return Value::str("");
                        }
                        // `(w)N` on scalar: split by IFS into words,
                        // return the Nth (1-based). zsh's word
                        // separator defaults to IFS whitespace.
                        // `(s/sep/)` overrides the separator. zsh
                        // also accepts `(ws[chars])` — `s` followed
                        // by a `[chars]` set treated as IFS for this
                        // operation.
                        if let Some((flags, pat)) = (|s: &str| -> Option<(String, String)> {
                        // Port of subst.c subscript-flag parser:
                        // `(I)pat` / `(R)pat` / `(i)pat` / `(r)pat`.
                        // Special-case `(s<delim>...<delim>)` per
                        // params.c:1458-1476 — `s` introduces a
                        // delimited separator block.
                        // Returns (flags_chars, pattern_after).
                        let s = s.trim_start();
                        let rest = s.strip_prefix('(')?;
                        let close = rest.find(')')?;
                        let flags = rest[..close].to_string();
                        let pat = rest[close + 1..].to_string();
                        if flags.starts_with('s') {
                            return Some((flags, pat));
                        }
                        if flags.chars().next().is_some_and(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'k' | 'K' | 'n' | 'e' | 'b' | 'w' | 'f' | 'p' | 's')) {
                            Some((flags, pat))
                        } else { None }
                    })(&idx) {
                            if flags.contains('w') {
                                if let Ok(n) = pat.parse::<i64>() {
                                    let words: Vec<&str> = scalar.split_whitespace().collect();
                                    let len = words.len() as i64;
                                    let i = if n > 0 {
                                        (n - 1) as usize
                                    } else if n < 0 {
                                        let off = len + n;
                                        if off < 0 {
                                            return Value::str("");
                                        }
                                        off as usize
                                    } else {
                                        return Value::str("");
                                    };
                                    return Value::str(
                                        words.get(i).map(|s| s.to_string()).unwrap_or_default(),
                                    );
                                }
                            }
                            // `(s/sep/)N` is a NO-OP for scalar `[N]`
                            // indexing — confirmed by testing zsh
                            // (`a=hello; ${a[(s/l/)1]}` returns "h",
                            // same as `${a[1]}`). The `(s)` flag
                            // only affects splitting in word-list
                            // contexts (`${(s/sep/)var}` without
                            // index, or `[@]` form). Strip the
                            // flag, parse the index normally, fall
                            // through to char slicing.
                            if flags.starts_with('s') {
                                if let Ok(i) = pat.parse::<i64>() {
                                    let s_chars: Vec<String> = scalar.chars().map(|c| c.to_string()).collect();
                                    return Value::str(crate::ported::params::getarrvalue(&s_chars, i, i).concat());
                                }
                            }
                            // (i)/(I)/(r)/(R) on scalar — route
                            // through getarg's scalar char-search
                            // arm (params.c:1798-1980). Faithful
                            // port lives in src/ported/params.rs;
                            // this branch defers to it to avoid
                            // duplicated drift.
                            if flags.chars().all(|c| matches!(c, 'i' | 'I' | 'r' | 'R' | 'e')) {
                                let _ = &pat;
                                if let Some(crate::ported::params::GetargOut::Value(v)) =
                                    crate::ported::params::getarg(&idx, None, None, Some(&scalar))
                                {
                                    return v;
                                }
                            }
                        }
                        // Build a per-char pseudo-array and route slice/index
                        // through getarrvalue so 1-based inclusive semantics
                        // and negative-from-end indexing match
                        // Src/params.c::getstrvalue's char-arm.
                        let s_chars: Vec<String> = scalar.chars().map(|c| c.to_string()).collect();
                        if let Some((start_s, end_s)) = idx.split_once(',') {
                            let parse_one = |s: &str, exec: &mut ShellExecutor| -> Option<i64> {
                                let t = s.trim();
                                if t.is_empty() { return None; }
                                if let Ok(i) = t.parse::<i64>() { return Some(i); }
                                Some(exec.eval_arith_expr(t))
                            };
                            let s_opt = parse_one(start_s, exec);
                            let e_opt = parse_one(end_s, exec);
                            let s_i = s_opt.unwrap_or(1);
                            let e_i = e_opt.unwrap_or(s_chars.len() as i64);
                            return Value::str(crate::ported::params::getarrvalue(&s_chars, s_i, e_i).concat());
                        }
                        let i = match idx.parse::<i64>() {
                            Ok(i) => i,
                            Err(_) => exec.eval_arith_expr(&idx),
                        };
                        return Value::str(crate::ported::params::getarrvalue(&s_chars, i, i).concat());
                    }
                };

                // Subscript flag form: (r)pat / (R)pat / (i)pat / (I)pat
                // / (e)str / (n:N:)pat. Returns first/last matching value
                // or first/last matching index per zsh semantics.
                if let Some((flags, pat)) = (|s: &str| -> Option<(String, String)> {
                    let s = s.trim_start();
                    let rest = s.strip_prefix('(')?;
                    let close = rest.find(')')?;
                    let flags = rest[..close].to_string();
                    let pat = rest[close + 1..].to_string();
                    if flags.chars().next().is_some_and(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'k' | 'K' | 'n' | 'e' | 'b' | 'w' | 'f' | 'p' | 's')) {
                        Some((flags, pat))
                    } else { None }
                })(&idx) {
                    // Route to getarg's array-search arm
                    // (params.c:1672-1719).
                    let _ = (&flags, &pat); // silence unused if any
                    match crate::ported::params::getarg(&idx, Some(&arr), None, None) {
                        Some(crate::ported::params::GetargOut::Value(v)) => return v,
                        _ => {}
                    }
                    return Value::str("");
                }

                // Slice form `N,M`: comma separator with int-or-arith
                // operands on each side. Negative indices count from
                // end. Direct port of zsh's getindex() N,M slice.
                //
                // Return shape depends on context: in DQ (`"${arr[2,4]}"`)
                // zsh joins the slice with the first IFS char into a
                // single scalar (Src/subst.c sepjoin path with nojoin=0);
                // in unquoted (`${arr[2,4]}`) or `[@]`-style context it
                // remains an array. Detect via in_dq_context which the
                // BUILTIN_EXPAND_TEXT mode-1 wrapper bumps.
                if let Some((start_s, end_s)) = idx.split_once(',') {
                    // Inline subscript-int parse — mirrors getarg's
                    // mathevalarg fallback (params.c:1567).
                    let parse_one = |s: &str, exec: &mut ShellExecutor| -> Option<i64> {
                        let t = s.trim();
                        if t.is_empty() { return None; }
                        if let Ok(i) = t.parse::<i64>() { return Some(i); }
                        Some(exec.eval_arith_expr(t))
                    };
                    let start = parse_one(start_s, exec);
                    let end = parse_one(end_s, exec);
                    if let (Some(s), Some(e)) = (start, end) {
                        // KSH_ARRAYS: indices are 0-based, so shift
                        // positive values up by 1 before the (1-based)
                        // slicer runs. zsh: `setopt ksh_arrays;
                        // a=(a b c d); echo $a[1,2]` → `b c`.
                        let ksh = exec.options.get("ksharrays").copied().unwrap_or(false);
                        let s = if ksh && s >= 0 { s + 1 } else { s };
                        let e = if ksh && e >= 0 { e + 1 } else { e };
                        let sliced = getarrvalue(&arr, s, e);
                        // (@) flag in surrounding chain overrides DQ-join
                        // — always splat to Value::Array so the caller's
                        // (@)-aware splat path emits each element as its
                        // own word.
                        if !force_array && (exec.in_dq_context > 0 || dq_compile) {
                            let ifs_first = exec
                                .get_variable("IFS")
                                .chars()
                                .next()
                                .unwrap_or(' ')
                                .to_string();
                            return Value::str(sliced.join(&ifs_first));
                        }
                        return Value::Array(
                            sliced.into_iter().map(Value::str).collect(),
                        );
                    }
                }

                // Single index — try literal int first (fast), then fall
                // back to arithmetic eval which handles bare variable
                // names (`arr[i]`), expressions (`arr[i+1]`), etc.
                // KSH_ARRAYS: 0-based, so a 0 means first element and
                // valid indices are 0..len-1. Without this, `setopt
                // ksh_arrays; a[0]` returned empty (treating 0 as
                // "before first" per the standard 1-based path).
                let i = match idx.parse::<i64>() {
                    Ok(i) => i,
                    Err(_) => exec.eval_arith_expr(&idx),
                };
                let len = arr.len() as i64;
                let ksh = exec.options.get("ksharrays").copied().unwrap_or(false);
                let resolved = if ksh {
                    if i < 0 {
                        let off = len + i;
                        if off < 0 {
                            return Value::str("");
                        }
                        off as usize
                    } else if i >= len {
                        return Value::str("");
                    } else {
                        i as usize
                    }
                } else if i > 0 {
                    (i - 1) as usize
                } else if i < 0 {
                    let off = len + i;
                    if off < 0 {
                        return Value::str("");
                    }
                    off as usize
                } else {
                    return Value::str("");
                };
                Value::str(arr.get(resolved).cloned().unwrap_or_default())
            }
        })
    });

    // `${(flags)name}` — apply zsh parameter flags. See BUILTIN_PARAM_FLAG
    // doc comment for the supported flag set. Algorithm: load `name` as a
    // current-value (scalar from variables/env, array from arrays, or assoc
    // from assoc_arrays), then walk `flags` char-by-char applying each
    // transformation. Final state is either Value::str or Value::Array
    // depending on the last flag.
    // Bridge entry that preserves array shape — see the const's doc.
    // Pops [content] (the brace body without the outer ${...}) and
    // returns Value::Array of per-element words.
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
            let mut state = crate::ported::subst::SubstState::from_executor(exec);
            let mut ret_flags = 0u32;
            let (_full_str, _new_pos, nodes) = crate::ported::subst::paramsubst(
                &full,
                0,
                false,
                0,
                &mut ret_flags,
                &mut state,
            );
            state.commit_to_executor(exec);
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

    vm.register_builtin(BUILTIN_PARAM_FLAG, |vm, _argc| {
        let mut flags = vm.pop().to_str();
        let name = vm.pop().to_str();

        // Compile path tags DQ-wrapped expressions with a leading
        // `\u{02}` sentinel. In DQ context, array-only flags are
        // no-ops per zsh: `(o)`/`(O)`/`(n)`/`(i)`/`(M)`/`(u)` only
        // fire in array context. Strip those flag chars before
        // processing so the join-as-scalar path returns the original
        // element order.
        let dq_compile = flags.starts_with('\u{02}');
        if dq_compile {
            flags = flags[1..].to_string();
        }
        // `\u{03}` sentinel = the original name had `[@]`/`[*]` suffix.
        // The compile path strips the suffix from name (fast-path
        // requires identifier-only), but encodes the splice context
        // through this sentinel so DQ flag-stripping still respects it.
        let had_at_subscript = flags.starts_with('\u{03}');
        if had_at_subscript {
            flags = flags[1..].to_string();
        }
        // `\u{04}` sentinel = scalar-assignment context (compile-time
        // detected via `scalar_assign_depth`). Direct port of zsh's
        // PREFORK_SINGLE bit (Src/exec.c::addvars line 2546). Strip
        // the sentinel and remember it for the split-flag gate
        // below.
        let ssub_compile = flags.starts_with('\u{04}');
        if ssub_compile {
            flags = flags[1..].to_string();
        }
        let dq_runtime = with_executor(|exec| exec.in_dq_context > 0);
        // PREFORK_SINGLE equivalent — set when the BUILTIN_PARAM_FLAG
        // is being evaluated as the RHS of a scalar assignment.
        // Direct port of Src/subst.c:1759 `int ssub = (pf_flags &
        // PREFORK_SINGLE)`. Per Src/subst.c:3902 `force_split = !ssub
        // && (spbreak || spsep)` — when ssub, the force-split path
        // is gated off, so split flags `(f)` / `(s:STR:)` / `(0)` /
        // `(z)` produce the original scalar verbatim. Consulted at
        // each split flag's effect site below (the flag char itself
        // is not removed; instead the split is skipped).
        let ssub_runtime = ssub_compile
            || with_executor(|exec| exec.in_scalar_assign > 0);
        // `[@]` / `[*]` subscript on the name overrides the DQ
        // strip — explicit `[@]` marks the array as splice-
        // expanded so array-only flags (`o`/`O`/`n`/`i`/`u`)
        // still fire on the per-element list. Direct port of
        // zsh's subst.c nojoin/spbreak path. Without this,
        // `"${(o)a[@]}"` skipped the sort in DQ.
        // The explicit `@` flag is also an array-context marker — zsh
        // treats `${(@o)a}` same as `${(o)a[@]}` (both keep array-only
        // sort flags active in DQ). Without checking flags too, the DQ
        // strip dropped `o` for the bare-name `(@o)` case.
        let has_at_subscript = had_at_subscript
            || name.ends_with("[@]")
            || name.ends_with("[*]")
            || flags.contains('@');
        if (dq_compile || dq_runtime) && !has_at_subscript {
            // Strip array-only flag CHARS (sort/unique/index variants)
            // from the flag chain — but only when they appear as
            // bare flag chars, not as part of a flag-arg like
            // `(r:NAME::pad:)` where NAME may contain `n`/`o`/etc.
            // Direct port of zsh's nojoin gating in Src/subst.c:1813
            // which gates these flags off in DQ context. The C source
            // walks the flag chain as a state machine; we mirror that
            // by tracking arg-region depth: when we hit `(j:`, `(s:`,
            // `(l:`, `(r:` etc., switch into "in-arg" mode and copy
            // chars verbatim until the closing delim. Without this
            // careful skip, `(r:hlen:: :)` lost the `n` inside the
            // identifier, so width parsing returned a truncated name.
            let bytes = flags.as_bytes();
            let mut out = String::with_capacity(bytes.len());
            let mut i = 0;
            while i < bytes.len() {
                let b = bytes[i] as char;
                // Flag chars that take a delimited argument:
                // `j:STR:` join, `s:STR:` split, `l:N::pad:`,
                // `r:N::pad:`, `Z:STR:`, `g:STR:`. The arg is
                // bracket-delimited by the next char.
                if matches!(b, 'j' | 's' | 'l' | 'r' | 'Z' | 'g')
                    && i + 1 < bytes.len()
                    && !(bytes[i + 1] as char).is_ascii_alphanumeric()
                    && bytes[i + 1] != b'_'
                {
                    let delim_open = bytes[i + 1] as char;
                    let delim_close = match delim_open {
                        '[' => ']',
                        '{' => '}',
                        '(' => ')',
                        '<' => '>',
                        c => c,
                    };
                    out.push(b);
                    out.push(delim_open);
                    i += 2;
                    // For `l:N::pad:` and `r:N::pad:`, the format has
                    // TWO arg sections: `:N:` then `:pad:`. Walk
                    // through both, plus any further sections until
                    // we run out of immediate-`delim_close+delim_open`
                    // pairs. This matches zsh subst.c get_strarg
                    // which is called in a loop.
                    loop {
                        while i < bytes.len() && bytes[i] as char != delim_close {
                            out.push(bytes[i] as char);
                            i += 1;
                        }
                        if i < bytes.len() {
                            out.push(delim_close);
                            i += 1;
                        }
                        // Continue if the next char is the same
                        // open-delim (another arg section).
                        if i < bytes.len() && bytes[i] as char == delim_open {
                            out.push(delim_open);
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    continue;
                }
                if matches!(b, 'o' | 'O' | 'n' | 'i' | 'u') {
                    i += 1;
                    continue;
                }
                out.push(b);
                i += 1;
            }
            flags = out;
        }

        // Initial state: prefer assoc → array → scalar lookup. If `P` flag
        // is in the chain, we'll re-fetch with the indirected name later.
        enum St {
            S(String),
            A(Vec<String>),
        }

        // Detect (k) flag PRESENCE early — we need to seed
        // magic-assoc lookups with the key set before the flag
        // walker re-orders things. Use `flags` (the post-sentinel-
        // strip string) since the `chars` Vec is built later.
        let want_keys = flags.contains('k');
        let want_values = flags.contains('v');

        // Literal-string operand sentinel: `${(flags)"text"}` compiles to a
        // name prefixed with `\u{01}` followed by the literal value. Skip
        // the lookup and seed state with the literal scalar.
        let mut state = if let Some(literal) = name.strip_prefix('\u{01}') {
            St::S(literal.to_string())
        } else {
            with_executor(|exec| {
                if let Some(map) = exec.assoc_arrays.get(&name) {
                    // For assoc, default to value list (no flag) — `(k)`/`(v)`
                    // override.
                    St::A(map.values().cloned().collect())
                } else if let Some(arr) = exec.arrays.get(&name) {
                    St::A(arr.clone())
                } else if want_keys {
                    // `${(k)<magic-assoc>}` — names like `aliases`,
                    // `functions`, `options`, `commands`, `terminfo`,
                    // `errnos` etc. are not in `assoc_arrays` (they're
                    // synthesized via magic-getfn). When the flag set
                    // includes `k`, return the SCANFN-equivalent key
                    // list. Direct port of paramsubst's per-special
                    // scanfn dispatch (Src/Modules/parameter.c +
                    // system.c + terminfo.c et al.).
                    if let Some(keys) =
                        crate::exec_shims::scan_magic_assoc_keys(&name)
                    {
                        St::A(keys)
                    } else {
                        St::S(exec.get_variable(&name))
                    }
                } else if want_values {
                    // `${(v)<magic-assoc>}` — values for the same
                    // magic-getfn list above. zinit/p10k both use
                    // `${(v)aliases}`-style introspection; the
                    // earlier (k) branch covered the keys but the
                    // (v) symmetry was missing, so plugin code that
                    // looped over alias bodies got an empty list.
                    if let Some(keys) =
                        crate::exec_shims::scan_magic_assoc_keys(&name)
                    {
                        let values: Vec<String> = keys
                            .iter()
                            .map(|k| exec.get_special_array_value(&name, k).unwrap_or_default())
                            .collect();
                        St::A(values)
                    } else {
                        St::S(exec.get_variable(&name))
                    }
                } else {
                    St::S(exec.get_variable(&name))
                }
            })
        };

        let chars: Vec<char> = flags.chars().collect();
        // Pre-scan for `(P)` — indirect: zsh's bin_zmodload-style
        // P flag is special. It applies BEFORE all per-char
        // transforms regardless of position in the flag string,
        // because zsh's paramsubst sets `aspar` early and the
        // INITIAL value is the indirected lookup. Without this
        // pre-resolve, `${(UP)ref}` first uppercases ref's value
        // ("target" → "TARGET") then tries to indirect on "TARGET"
        // which is unset, returning empty. zsh produces "HELLO"
        // because it indirects FIRST (ref→target, lookup target =
        // "hello") then uppercases.
        let want_indirect = chars.iter().any(|&c| c == 'P');
        // `(Pt)` is a special pairing — type-of-the-target, not
        // value-of-the-target. Direct port of Src/subst.c:2807-2854
        // `wantt` arm: zsh's `wantt` runs AFTER `aspar` has resolved
        // the pm pointer to the target's Param struct, then reads
        // `pm->node.flags` for type. Doing the value pre-walker here
        // discards the target name and the (t) handler ends up
        // introspecting the original pointer ("n" → scalar). Skip
        // the value-walker for (Pt); the (t) handler resolves the
        // target name itself via `target_for_type` below.
        let want_type = chars.iter().any(|&c| c == 't');
        let pt_combo = want_indirect && want_type;
        if want_indirect && !pt_combo && !matches!(state, St::S(ref s) if s.is_empty()) {
            // The state at this point holds the (P) TARGET reference,
            // not the original pointer name — the param-flag dispatch
            // upstream initialized state to `exec.get_variable(name)`.
            // Resolve that target. Two shapes:
            //   - bare name: `${(P)n}` with `n=foo` → state="foo",
            //     look up `foo` directly.
            //   - subscripted name: `${(P)n2}` with `n2="arr[-1]"` →
            //     state="arr[-1]", split into base="arr" + sub="-1"
            //     and route through expand_string. Direct port of
            //     Src/subst.c:2799-2806 where `fetchvalue(&vbuf, &ov, …)`
            //     parses both name and any trailing `[…]` subscript
            //     from the same input pointer. Without this split,
            //     a subscripted target was looked up as a literal
            //     parameter named "arr[-1]" (always unset → empty).
            fn resolve_indirect_target(target: &str, exec: &mut ShellExecutor) -> St {
                let (base, sub) = match target.find('[') {
                    Some(b) if target.ends_with(']') => {
                        let n = &target[..b];
                        let s = &target[b + 1..target.len() - 1];
                        (n.to_string(), Some(s.to_string()))
                    }
                    _ => (target.to_string(), None),
                };
                // Bare-name path.
                if sub.is_none() {
                    if let Some(arr) = exec.arrays.get(&base) {
                        return St::A(arr.clone());
                    }
                    return St::S(exec.get_variable(&base));
                }
                let sub_str = sub.unwrap();
                // Assoc lookup: `${(P)"map[key]"}` — single value for
                // the given key.
                if let Some(m) = exec.assoc_arrays.get(&base).cloned() {
                    return St::S(m.get(&sub_str).cloned().unwrap_or_default());
                }
                // Indexed-array subscript. Direct port of getindex()
                // (Src/params.c) handling for negative indices and
                // `lo,hi` slice. expand_string() can't be used here —
                // it routes the subscripted form through compile-time
                // paths that re-fetch the WHOLE array on the bridge
                // back from subst_port. Apply the subscript here
                // directly.
                if let Some(arr) = exec.arrays.get(&base).cloned() {
                    let n = arr.len() as i64;
                    let to_zero = |i: i64| -> i64 {
                        if i > 0 {
                            i - 1
                        } else if i < 0 {
                            n + i
                        } else {
                            0
                        }
                    };
                    if let Some((lo_s, hi_s)) = sub_str.split_once(',') {
                        let lo = lo_s.trim().parse::<i64>().unwrap_or(1);
                        let hi = hi_s.trim().parse::<i64>().unwrap_or(n);
                        let lo_i = to_zero(lo).max(0);
                        let hi_i = to_zero(hi);
                        if hi_i < lo_i || lo_i >= n {
                            return St::A(Vec::new());
                        }
                        let hi_clamped = (hi_i + 1).min(n) as usize;
                        return St::A(arr[lo_i as usize..hi_clamped].to_vec());
                    }
                    if sub_str == "@" || sub_str == "*" {
                        return St::A(arr);
                    }
                    if let Ok(idx) = sub_str.parse::<i64>() {
                        let real = to_zero(idx);
                        if real < 0 || real >= n {
                            return St::S(String::new());
                        }
                        return St::S(arr[real as usize].clone());
                    }
                }
                // Fallback: scalar with subscript = char-range.
                let val = exec.get_variable(&base);
                let chars: Vec<char> = val.chars().collect();
                let n = chars.len() as i64;
                let to_zero = |i: i64| -> i64 {
                    if i > 0 {
                        i - 1
                    } else if i < 0 {
                        n + i
                    } else {
                        0
                    }
                };
                if let Some((lo_s, hi_s)) = sub_str.split_once(',') {
                    let lo = lo_s.trim().parse::<i64>().unwrap_or(1);
                    let hi = hi_s.trim().parse::<i64>().unwrap_or(n);
                    let lo_i = to_zero(lo).max(0);
                    let hi_i = to_zero(hi);
                    if hi_i < lo_i || lo_i >= n {
                        return St::S(String::new());
                    }
                    let hi_clamped = (hi_i + 1).min(n) as usize;
                    return St::S(chars[lo_i as usize..hi_clamped].iter().collect());
                }
                if let Ok(idx) = sub_str.parse::<i64>() {
                    let real = to_zero(idx);
                    if real < 0 || real >= n {
                        return St::S(String::new());
                    }
                    return St::S(chars[real as usize].to_string());
                }
                St::S(String::new())
            }
            state = match state {
                St::S(name) => with_executor(|exec| resolve_indirect_target(&name, exec)),
                St::A(names) => with_executor(|exec| {
                    let resolved: Vec<String> = names
                        .into_iter()
                        .map(|n| exec.get_variable(&n))
                        .collect();
                    St::A(resolved)
                }),
            };
        }
        // Pre-scan for `(p)` — print-style escape interpretation for
        // any subsequent `(s::)`, `(j::)`, `(l::)`, `(r::)` argument
        // strings. Direct port of src/zsh/Src/subst.c:2381-2382 which
        // sets `escapes = 1` and then `untok_and_escape` performs the
        // print-escape on those flag args. Order in zsh: only flags
        // that appear AFTER `p` get their args escaped; we approximate
        // by detecting `p` at the start of the flag string. The exact
        // C semantics rely on left-to-right state, but `(ps:..:)` is
        // by far the dominant idiom and a position-aware pre-scan is
        // the simplest faithful match.
        let print_escapes = chars
            .iter()
            .take_while(|&&c| c != 's' && c != 'j' && c != 'l' && c != 'r')
            .any(|&c| c == 'p');
        // print_escape_str — interpret \n, \t, \r, \\, \xNN, \NNN
        // (octal) per zsh's untok_and_escape behavior. Returns the
        // decoded string. Used inline below when print_escapes is set.
        fn print_escape_str(s: &str) -> String {
            let mut out = String::with_capacity(s.len());
            let mut chars = s.chars().peekable();
            while let Some(c) = chars.next() {
                if c != '\\' {
                    out.push(c);
                    continue;
                }
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('\\') => out.push('\\'),
                    Some('\'') => out.push('\''),
                    Some('"') => out.push('"'),
                    Some('a') => out.push('\x07'),
                    Some('b') => out.push('\x08'),
                    Some('e') | Some('E') => out.push('\x1b'),
                    Some('f') => out.push('\x0c'),
                    Some('v') => out.push('\x0b'),
                    Some('0') => out.push('\0'),
                    Some('x') => {
                        let mut hex = String::new();
                        for _ in 0..2 {
                            match chars.peek() {
                                Some(&h) if h.is_ascii_hexdigit() => {
                                    hex.push(h);
                                    chars.next();
                                }
                                _ => break,
                            }
                        }
                        if let Ok(n) = u32::from_str_radix(&hex, 16) {
                            if let Some(c) = char::from_u32(n) {
                                out.push(c);
                            }
                        }
                    }
                    Some(d) if d.is_ascii_digit() => {
                        let mut oct = String::from(d);
                        for _ in 0..2 {
                            match chars.peek() {
                                Some(&h) if h.is_digit(8) => {
                                    oct.push(h);
                                    chars.next();
                                }
                                _ => break,
                            }
                        }
                        if let Ok(n) = u32::from_str_radix(&oct, 8) {
                            if let Some(c) = char::from_u32(n) {
                                out.push(c);
                            }
                        }
                    }
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            }
            out
        }
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            i += 1;
            match c {
                '#' => {
                    // `(#)` — evaluate each element as an arithmetic
                    // expression, then output the character with that
                    // code point. Direct port of substevalchar in
                    // src/zsh/Src/subst.c:1490-1520. zsh's flow:
                    //   ires = mathevali(ptr);     // line 1497
                    //   if (errflag) return "";    // 1499-1502
                    //   if (ires < 0) zerr("character not in range");  // 1504-1506
                    //   if MULTIBYTE && ires>127: ucs4tomb           // 1508-1511
                    //   else: single-byte sprintf                    // 1514-1518
                    let to_char = |s: &str| -> String {
                        let n = with_executor(|exec| exec.eval_arith_expr(s));
                        // zsh subst.c:1504-1518 — negative WARNS but
                        // STILL outputs the low byte (truncated cast
                        // through `(int)ires` + `%c` sprintf at line
                        // 1514-1517). The zerr at line 1505 just sets
                        // errflag without aborting the function. We
                        // skip the error message (matches zsh's
                        // observed silent behavior under -f -c) and
                        // mirror the low-byte fallback.
                        if !(0..=0x10FFFF).contains(&n) {
                            // Truncated cast: low 8 bits as Latin-1
                            // byte (zsh's `%c` sprintf on `(int)ires`).
                            let byte = (n as i32 as u32) & 0xFF;
                            // Encode the byte as raw — for high bytes
                            // (0x80-0xFF), wrap with the same UTF-8
                            // promotion zsh's pastebuf() uses.
                            return char::from_u32(byte)
                                .map(|c| c.to_string())
                                .unwrap_or_default();
                        }
                        // Valid Unicode scalar — char::from_u32 returns
                        // the right multi-byte UTF-8 sequence in Rust.
                        char::from_u32(n as u32)
                            .map(|c| c.to_string())
                            .unwrap_or_default()
                    };
                    state = match state {
                        St::S(s) => St::S(to_char(&s)),
                        St::A(a) => St::A(a.into_iter().map(|s| to_char(&s)).collect()),
                    };
                }
                'L' => {
                    state = match state {
                        St::S(s) => St::S(s.to_lowercase()),
                        St::A(a) => St::A(a.into_iter().map(|s| s.to_lowercase()).collect()),
                    };
                }
                'U' => {
                    state = match state {
                        St::S(s) => St::S(s.to_uppercase()),
                        St::A(a) => St::A(a.into_iter().map(|s| s.to_uppercase()).collect()),
                    };
                }
                'l' | 'r' => {
                    // (l:N:) — left-pad to width N (truncate if longer).
                    // (l:N::fill:) — pad with `fill` instead of space.
                    // (r:N:) — right-pad to width N (truncate if longer).
                    // Width must be followed by `:` (or `(` etc.) delim.
                    let pad_left = c == 'l';
                    if i >= chars.len() || !ZshrsHost::is_zsh_flag_delim(chars[i]) {
                        // Bare `l`/`r` without delim — skip (only the
                        // padded form takes a width).
                        continue;
                    }
                    let delim = chars[i];
                    i += 1;
                    let mut width_str = String::new();
                    while i < chars.len() && chars[i] != delim {
                        width_str.push(chars[i]);
                        i += 1;
                    }
                    if i < chars.len() {
                        i += 1; // skip closing delim
                    }
                    // Width may be a literal number, `$VAR`, or a bare
                    // identifier (zsh evaluates `(r:hlen:: :)` by
                    // running `mathevali("hlen")` which reads the
                    // parameter table). Direct port of Src/subst.c
                    // `get_intarg()` (line 1428) which does
                    // `parsestr` → `singsub` → `mathevali`. Fast path:
                    // if the arg parses as a literal usize, use it
                    // directly. Otherwise expand `$`-references and
                    // route through evaluate_arithmetic so bare
                    // identifiers resolve to their variable values.
                    let width: usize = if let Ok(n) = width_str.parse() {
                        n
                    } else {
                        with_executor(|exec| {
                            let arith_str = exec.evaluate_arithmetic(&width_str);
                            arith_str.parse::<i64>().map(|v| v.unsigned_abs() as usize).unwrap_or(0)
                        })
                    };
                    // Optional `:fill:` after the width.
                    let mut fill = String::from(" ");
                    if i < chars.len() && ZshrsHost::is_zsh_flag_delim(chars[i]) {
                        let d2 = chars[i];
                        i += 1;
                        let mut f = String::new();
                        while i < chars.len() && chars[i] != d2 {
                            f.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() {
                            i += 1; // skip closing delim
                        }
                        if !f.is_empty() {
                            fill = if print_escapes {
                                print_escape_str(&f)
                            } else {
                                f
                            };
                        }
                    }
                    let pad_one = |s: String| -> String {
                        let len = s.chars().count();
                        if len >= width {
                            return s.chars().take(width).collect();
                        }
                        let need = width - len;
                        let mut filler = String::new();
                        while filler.chars().count() < need {
                            filler.push_str(&fill);
                        }
                        let filler: String = filler.chars().take(need).collect();
                        if pad_left {
                            format!("{}{}", filler, s)
                        } else {
                            format!("{}{}", s, filler)
                        }
                    };
                    state = match state {
                        St::S(s) => St::S(pad_one(s)),
                        St::A(a) => St::A(a.into_iter().map(pad_one).collect()),
                    };
                }
                'j' | 's' => {
                    // zsh syntax: `(j:sep:)` and `(s:sep:)` use the char
                    // following the flag as the delimiter. The delimiter must
                    // be a non-alphanumeric, non-underscore char so subsequent
                    // flags (alphabetic) aren't accidentally swallowed —
                    // `(jL)` should be `j` (no delim, default IFS) followed
                    // by `L`, not `j` with delim `L`. Recognized delim chars
                    // mirror what zsh allows: punctuation only. zsh subst.c
                    // get_strarg also accepts matched bracket pairs:
                    // `[`/`]`, `{`/`}`, `(`/`)`, `<`/`>`.
                    let mut sep = String::new();
                    if i < chars.len() && ZshrsHost::is_zsh_flag_delim(chars[i]) {
                        let delim = chars[i];
                        let close = match delim {
                            '[' => ']',
                            '{' => '}',
                            '(' => ')',
                            '<' => '>',
                            c => c,
                        };
                        i += 1;
                        while i < chars.len() && chars[i] != close {
                            sep.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() {
                            i += 1; // skip closing delim
                        }
                    } else if c == 'j' {
                        // `j` with no delim → join with space (IFS-default).
                        sep = " ".to_string();
                    }
                    // `(p)` print-escape interpretation per
                    // src/zsh/Src/subst.c:2381-2382 — `\n`, `\t`,
                    // `\xNN`, `\NNN` (octal) etc. become the actual
                    // characters in the separator. Additionally,
                    // (p) enables \$VAR / \${VAR} / \$(cmd) /
                    // \$((expr)) expansion in the separator string
                    // (zsh's parsestr+singsub treatment of get_strarg
                    // results when the (p) flag is present). Without
                    // (p), these stay literal — confirmed via
                    // /opt/homebrew/bin/zsh -fc.
                    if print_escapes && !sep.is_empty() {
                        sep = print_escape_str(&sep);
                        if sep.contains('$') || sep.contains('`') {
                            sep = with_executor(|exec| exec.singsub(&sep));
                        }
                    }
                    if c == 'j' {
                        state = match state {
                            St::A(a) => St::S(a.join(&sep)),
                            St::S(s) => St::S(s),
                        };
                    } else {
                        // (s) splits both scalars and array elements per
                        // zsh semantics. `(@s:,:)` runs `@` first which
                        // wraps a scalar in a 1-elem array; `s` must
                        // still split that element. Same goes for true
                        // arrays — flat-map split each element.
                        //
                        // Empty-field handling — verified against zsh's
                        // C source (utils.c sepsplit + subst.c around
                        // line 3273). The actual rule is NOT "drop all
                        // empties" but more nuanced:
                        //   - Boundary empties (leading or trailing
                        //     run of separators) collapse to ONE empty
                        //     each, regardless of how many separators.
                        //   - Middle empties (consecutive separators
                        //     between non-empties) drop ENTIRELY.
                        //   - `(@)` flag preserves all empties verbatim.
                        // Examples (no @):
                        //   "a,,b,,c"   → [a,b,c]      (3)
                        //   ",a,b"      → ["",a,b]     (3)
                        //   "a,b,"      → [a,b,""]     (3)
                        //   ",,a,,b,,"  → ["",a,b,""]  (4)
                        //   "a,,,b"     → [a,b]        (2, 3 middle empties)
                        let keep_empty = chars.contains(&'@');
                        let collapse = |s: &str, sep: &str| -> Vec<String> {
                            let parts: Vec<String> = s.split(sep).map(String::from).collect();
                            if keep_empty {
                                return parts;
                            }
                            // Find first and last non-empty positions.
                            let first_nonempty = parts.iter().position(|p| !p.is_empty());
                            let last_nonempty = parts.iter().rposition(|p| !p.is_empty());
                            match (first_nonempty, last_nonempty) {
                                (None, _) => {
                                    // All-empty input. Collapse to a
                                    // single empty if input had any
                                    // separator (parts.len() > 1) and
                                    // therefore had a "boundary";
                                    // empty input → empty output.
                                    if parts.len() > 1 {
                                        vec![String::new()]
                                    } else {
                                        Vec::new()
                                    }
                                }
                                (Some(fi), Some(li)) => {
                                    let mut out: Vec<String> = Vec::new();
                                    if fi > 0 {
                                        out.push(String::new());
                                    }
                                    // Push only non-empty middles; drop
                                    // every internal empty.
                                    for p in &parts[fi..=li] {
                                        if !p.is_empty() {
                                            out.push(p.clone());
                                        }
                                    }
                                    if li < parts.len() - 1 {
                                        out.push(String::new());
                                    }
                                    out
                                }
                                _ => parts,
                            }
                        };
                        state = match state {
                            St::S(s) if sep.is_empty() => {
                                St::A(s.chars().map(|c| c.to_string()).collect())
                            }
                            St::S(s) => St::A(collapse(&s, sep.as_str())),
                            St::A(a) => {
                                let mut out: Vec<String> = Vec::with_capacity(a.len());
                                for elem in a {
                                    if sep.is_empty() {
                                        for c in elem.chars() {
                                            out.push(c.to_string());
                                        }
                                    } else {
                                        out.extend(collapse(&elem, sep.as_str()));
                                    }
                                }
                                St::A(out)
                            }
                        };
                    }
                }
                'f' => {
                    // Suppress the split entirely in scalar-assignment
                    // context per Src/subst.c:3902 ssub gate. The
                    // value passes through unchanged (preserves
                    // original `\n` separators in `y="${(f)x}"`).
                    if !ssub_runtime {
                        state = match state {
                            St::S(s) => St::A(s.split('\n').map(String::from).collect()),
                            St::A(a) => {
                                // Same flat-map rule as (s): split each element.
                                let mut out: Vec<String> = Vec::with_capacity(a.len());
                                for elem in a {
                                    for line in elem.split('\n') {
                                        out.push(line.to_string());
                                    }
                                }
                                St::A(out)
                            }
                        };
                    }
                }
                '0' => {
                    // `(0)` — split on NUL byte. Direct port of
                    // src/zsh/Src/subst.c:2292-2297 which sets `spsep`
                    // to a meta-encoded NUL. We split on the literal
                    // `\0` character. Same flat-map behaviour as `(f)`.
                    // Same ssub gate.
                    if !ssub_runtime { state = match state {
                        St::S(s) => St::A(s.split('\0').map(String::from).collect()),
                        St::A(a) => {
                            let mut out: Vec<String> = Vec::with_capacity(a.len());
                            for elem in a {
                                for piece in elem.split('\0') {
                                    out.push(piece.to_string());
                                }
                            }
                            St::A(out)
                        }
                    }; }
                }
                'F' => {
                    // (F) — join array elements with newlines (mirror
                    // of (j:\n:) but as a one-letter shorthand).
                    state = match state {
                        St::A(a) => St::S(a.join("\n")),
                        s => s,
                    };
                }
                'Q' => {
                    // (Q) — full shell-quoting reversal. Direct port of
                    // Src/utils.c::dequotestring which scans the entire
                    // string, handling SQ-spans (`'…'`), DQ-spans
                    // (`"…"`) with backslash escapes, and standalone
                    // `\X` escapes — NOT just outer-bslashquote strip. The
                    // canonical roundtrip is `(qq)` → `(Q)` for strings
                    // containing single quotes: `(qq)` of `a'b` produces
                    // `'a'\''b'` and `(Q)` must reverse the four
                    // close/escape/open transitions to recover `a'b`.
                    // Earlier outer-bslashquote-strip left `a'\''b` literal.
                    let dequote = |s: &str| -> String {
                        let mut out = String::with_capacity(s.len());
                        let mut chars = s.chars().peekable();
                        while let Some(c) = chars.next() {
                            match c {
                                '\\' => {
                                    if let Some(&nx) = chars.peek() {
                                        out.push(nx);
                                        chars.next();
                                    }
                                }
                                '\'' => {
                                    while let Some(&inner) = chars.peek() {
                                        chars.next();
                                        if inner == '\'' {
                                            break;
                                        }
                                        out.push(inner);
                                    }
                                }
                                '"' => {
                                    while let Some(&inner) = chars.peek() {
                                        chars.next();
                                        if inner == '"' {
                                            break;
                                        }
                                        if inner == '\\' {
                                            if let Some(&esc) = chars.peek() {
                                                out.push(esc);
                                                chars.next();
                                                continue;
                                            }
                                        }
                                        out.push(inner);
                                    }
                                }
                                _ => out.push(c),
                            }
                        }
                        out
                    };
                    state = match state {
                        St::S(s) => St::S(dequote(&s)),
                        St::A(a) => St::A(a.into_iter().map(|s| dequote(&s)).collect()),
                    };
                }
                'z' => {
                    // (z) — split by shell-token rules: whitespace
                    // boundaries, BUT also split out shell metacharacters
                    // like `;`, `&`, `|`, `(`, `)`, `<`, `>` as their
                    // own tokens. Honors single/double quotes (treat
                    // contents as one token, strip outer quotes from
                    // the result). Matches zsh's `(z)` flag.
                    state = match state {
                        St::S(s) => St::A(bufferwords_z(&s)),
                        St::A(a) => St::A(a),
                    };
                }
                'w' => {
                    // (w) — count words; in the array sense, just split
                    // on whitespace and let downstream consumers count.
                    state = match state {
                        St::S(s) => St::A(s.split_whitespace().map(String::from).collect()),
                        St::A(a) => St::A(a),
                    };
                }
                'o' | 'O' => {
                    // Optional sub-flag: `n` numeric, `i` case-insensitive,
                    // `a` array-order (i.e. don't sort, just reverse for O).
                    // Also detect `n`/`i` BEFORE the `o`/`O` (zsh's
                    // `(no)` and `(io)` shapes — order-agnostic).
                    let sub = chars.get(i).copied();
                    let consume = matches!(sub, Some('n') | Some('i') | Some('a'));
                    if consume {
                        i += 1;
                    }
                    // Look back: was `n` or `i` already in the flags
                    // string before this `o`? zsh treats `(no)` same
                    // as `(on)` — numeric sort applied to the
                    // ascending order. Only relevant if no inline sub
                    // was found.
                    let sub = if consume {
                        sub
                    } else {
                        let prefix = &chars[..i.saturating_sub(1)];
                        if prefix.contains(&'n') {
                            Some('n')
                        } else if prefix.contains(&'i') {
                            Some('i')
                        } else {
                            None
                        }
                    };
                    let consume = consume || matches!(sub, Some('n') | Some('i') | Some('a'));
                    let descending = c == 'O';
                    state = match state {
                        St::A(mut a) => {
                            match sub {
                                Some('a') if consume => {
                                    if descending {
                                        a.reverse();
                                    }
                                    // ascending + array-order = no-op
                                }
                                Some('n') if consume => {
                                    // Natural sort: compare by chunks of
                                    // digits-vs-non-digits so "file10"
                                    // sorts after "file2".
                                    a.sort_by(|x, y| {
                                        let cmp = natural_cmp(x, y);
                                        if descending {
                                            cmp.reverse()
                                        } else {
                                            cmp
                                        }
                                    });
                                }
                                Some('i') if consume => {
                                    a.sort_by(|x, y| {
                                        let xl = x.to_lowercase();
                                        let yl = y.to_lowercase();
                                        if descending {
                                            yl.cmp(&xl)
                                        } else {
                                            xl.cmp(&yl)
                                        }
                                    });
                                }
                                _ => {
                                    if descending {
                                        a.sort_by(|x, y| y.cmp(x));
                                    } else {
                                        a.sort();
                                    }
                                }
                            }
                            St::A(a)
                        }
                        s => s,
                    };
                }
                'u' => {
                    // Unique: preserve first occurrence, drop later dupes.
                    state = match state {
                        St::A(a) => {
                            let mut seen = std::collections::HashSet::new();
                            let unique: Vec<String> =
                                a.into_iter().filter(|s| seen.insert(s.clone())).collect();
                            St::A(unique)
                        }
                        s => s,
                    };
                }
                'C' => {
                    // `(C)` — capitalize. Direct port of
                    // src/zsh/Src/hist.c:2239-2256 CASMOD_CAPS via
                    // crate::ported::hist::casemodify. Treats any non-
                    // alphanumeric (including punctuation, control
                    // chars, NOT just whitespace) as a word boundary
                    // and lowercases mid-word uppercase letters.
                    state = match state {
                        St::S(s) => {
                            St::S(crate::ported::hist::casemodify(&s, crate::ported::hist::CaseMod::Caps))
                        }
                        St::A(a) => St::A(
                            a.into_iter()
                                .map(|s| crate::ported::hist::casemodify(&s, crate::ported::hist::CaseMod::Caps))
                                .collect(),
                        ),
                    };
                }
                'V' => {
                    // Make non-printable characters visible. zsh:
                    // `^X` for control chars (X = char + 64); `\M-X`
                    // for high-bit chars; backslash escapes for
                    // common forms (\n, \t, \r). zshrs's separate
                    // ZshParamFlag::Visible path implements this for
                    // the multi-flag dispatcher, but the inline state
                    // machine had no `V` arm so `${(V)x}` left
                    // control chars raw.
                    let visible = |s: &str| -> String {
                        let mut out = String::with_capacity(s.len());
                        for c in s.chars() {
                            match c {
                                '\n' => out.push_str("\\n"),
                                '\t' => out.push_str("\\t"),
                                '\r' => out.push_str("\\r"),
                                c if c.is_control() => {
                                    out.push('^');
                                    out.push((c as u8 + 64) as char);
                                }
                                _ => out.push(c),
                            }
                        }
                        out
                    };
                    state = match state {
                        St::S(s) => St::S(visible(&s)),
                        St::A(a) => St::A(a.into_iter().map(|s| visible(&s)).collect()),
                    };
                }
                'D' => {
                    // (D) named-directory substitution per
                    // Src/subst.c:4155 (`mods & 1`) → substnamedir.
                    // Replace $HOME prefix with `~` and any longer
                    // named-dir match with `~name`. Per-element on
                    // arrays, longest-prefix-first to avoid shallow
                    // shadowing (a `~zpwr=/Users/wizard/zpwr`
                    // override beats the bare `~=/Users/wizard`).
                    let render_d = |s: &str| -> String {
                        with_executor(|exec| {
                            let mut out = s.to_string();
                            // First the longer named dirs.
                            let mut entries: Vec<(String, std::path::PathBuf)> = exec
                                .named_dirs
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect();
                            entries.sort_by_key(|(_, p)| std::cmp::Reverse(p.as_os_str().len()));
                            for (name, path) in &entries {
                                let path_s = path.to_string_lossy();
                                if !path_s.is_empty() && out.starts_with(path_s.as_ref()) {
                                    return format!(
                                        "~{}{}",
                                        name,
                                        &out[path_s.len()..]
                                    );
                                }
                            }
                            // Then $HOME — only if no named-dir matched.
                            if let Some(home) = exec.variables.get("HOME").cloned() {
                                if !home.is_empty() && out.starts_with(&home) {
                                    out = format!("~{}", &out[home.len()..]);
                                }
                            } else if let Ok(home) = std::env::var("HOME") {
                                if !home.is_empty() && out.starts_with(&home) {
                                    out = format!("~{}", &out[home.len()..]);
                                }
                            }
                            out
                        })
                    };
                    state = match state {
                        St::S(s) => St::S(render_d(&s)),
                        St::A(a) => St::A(a.into_iter().map(|s| render_d(&s)).collect()),
                    };
                }
                'P' => {
                    // (P) was already applied as the pre-walker
                    // initial-state transform — see `want_indirect`
                    // above. The walker pass is a no-op for P.
                    state = match state {
                        St::S(s) => St::S(s),
                        St::A(a) => St::A(a),
                    };
                }
                '@' => {
                    // Force array shape (scalar → 1-elem array).
                    state = match state {
                        St::S(s) => St::A(vec![s]),
                        a => a,
                    };
                }
                'k' => {
                    // Keys of assoc. If immediately followed by 'v' (or
                    // earlier state was already 'v'-set), interleave key/value
                    // pairs (zsh's `(kv)` form). For regular arrays zsh
                    // returns the values themselves (a quirk: docs say
                    // "integer subscripts" but the actual implementation
                    // returns array contents — verified against /bin/zsh).
                    if i < chars.len() && chars[i] == 'v' {
                        i += 1; // consume the 'v'
                        let pairs = with_executor(|exec| {
                            if let Some(m) = exec.assoc_arrays.get(&name) {
                                let mut out = Vec::with_capacity(m.len() * 2);
                                for (k, v) in m {
                                    out.push(k.clone());
                                    out.push(v.clone());
                                }
                                out
                            } else if let Some(arr) = exec.arrays.get(&name) {
                                arr.clone()
                            } else {
                                // Magic-assoc fallback for (kv): emit
                                // alternating [key, value] pairs by
                                // pairing magic_assoc_keys with
                                // get_special_array_value lookups.
                                if let Some(keys) = crate::exec_shims::scan_magic_assoc_keys(&name) {
                                    let mut out = Vec::with_capacity(keys.len() * 2);
                                    for k in keys {
                                        let v = exec
                                            .get_special_array_value(&name, &k)
                                            .unwrap_or_default();
                                        out.push(k);
                                        out.push(v);
                                    }
                                    out
                                } else {
                                    Vec::new()
                                }
                            }
                        });
                        state = St::A(pairs);
                    } else {
                        let keys = with_executor(|exec| {
                            if let Some(m) = exec.assoc_arrays.get(&name) {
                                m.keys().cloned().collect::<Vec<_>>()
                            } else if let Some(arr) = exec.arrays.get(&name) {
                                // zsh quirk: `(k)` on a regular array
                                // returns the array values themselves.
                                arr.clone()
                            } else {
                                // `${(k)<magic-assoc>}` — names like
                                // `aliases`, `functions`, `options`,
                                // `commands`, `terminfo`, `errnos`,
                                // etc. Direct port of the per-special
                                // scanfn dispatch (Src/Modules/
                                // parameter.c et al.). Returns the
                                // sorted key set the C source builds
                                // by walking each magic table.
                                crate::exec_shims::scan_magic_assoc_keys(&name)
                                    .unwrap_or_default()
                            }
                        });
                        state = St::A(keys);
                    }
                }
                'v' => {
                    // Values of assoc. If immediately followed by 'k',
                    // interleave value/key pairs (zsh's `(vk)` form, less
                    // common than `(kv)` but supported for symmetry).
                    // Magic-assoc fallback when name isn't in
                    // assoc_arrays (`aliases`, `functions`, `commands`,
                    // `options`, `parameters`, `terminfo`, `errnos`,
                    // `sysparams`) — synthesize the value list from the
                    // executor's get_special_array_value scanfn-equivalent.
                    if i < chars.len() && chars[i] == 'k' {
                        i += 1; // consume the 'k'
                        let pairs = with_executor(|exec| {
                            if let Some(m) = exec.assoc_arrays.get(&name) {
                                let mut out = Vec::with_capacity(m.len() * 2);
                                for (k, v) in m {
                                    out.push(v.clone());
                                    out.push(k.clone());
                                }
                                out
                            } else if let Some(keys) =
                                crate::exec_shims::scan_magic_assoc_keys(&name)
                            {
                                let mut out = Vec::with_capacity(keys.len() * 2);
                                for k in keys {
                                    let v = exec
                                        .get_special_array_value(&name, &k)
                                        .unwrap_or_default();
                                    out.push(v);
                                    out.push(k);
                                }
                                out
                            } else {
                                Vec::new()
                            }
                        });
                        state = St::A(pairs);
                    } else {
                        let vals = with_executor(|exec| {
                            if let Some(m) = exec.assoc_arrays.get(&name) {
                                m.values().cloned().collect::<Vec<_>>()
                            } else if let Some(keys) =
                                crate::exec_shims::scan_magic_assoc_keys(&name)
                            {
                                keys.iter()
                                    .map(|k| {
                                        exec.get_special_array_value(&name, k)
                                            .unwrap_or_default()
                                    })
                                    .collect()
                            } else {
                                Vec::new()
                            }
                        });
                        state = St::A(vals);
                    }
                }
                '#' => {
                    state = match state {
                        St::A(a) => St::S(a.len().to_string()),
                        St::S(s) => St::S(s.len().to_string()),
                    };
                }
                'q' => {
                    // (q) quoting flag — direct port of `case 'q':` in
                    // Src/subst.c:2235-2253. zsh accepts ONLY:
                    //   q     backslash-escape (QT_BACKSLASH)
                    //   qq    single-bslashquote   (QT_SINGLE)
                    //   qqq   double-bslashquote   (QT_DOUBLE)
                    //   qqqq  $'…' ANSI-C   (QT_DOLLARS)
                    //   q-    QT_SINGLE_OPTIONAL (single-bslashquote if needed)
                    //   q+    QT_QUOTEDZPUTS    (quotedzputs() format)
                    // No `q*`, no `q!`, and crucially no `q:str:` delimiter
                    // form — those were bot-invented extensions. The
                    // `q:str:` arm in particular treated `@` as a delimiter
                    // (since `@` is non-alphanumeric so `is_zsh_flag_delim`
                    // returned true), capturing `explicit_delim=Some("")`
                    // and then `s.replace("", "\\")` inserted `\` between
                    // every char. That broke `${(qqqq@)arr}` and any other
                    // q-flag combined with a flag-letter that's also non-
                    // alphanumeric. Reference: zsh has no q-delimiter form.
                    let mut level = 1;
                    while i < chars.len() && chars[i] == 'q' && level < 4 {
                        level += 1;
                        i += 1;
                    }
                    let mut strip_trailing_newlines = false;
                    let mut wrap_only_if_needed = false;
                    let escape_glob_chars = false;     // c:2235 (no q* in zsh)
                    let explicit_delim: Option<String> = None; // c:2235 (no q:str: in zsh)
                    while i < chars.len() {
                        match chars[i] {
                            '+' => {
                                // c:2245-2246 — q+ → QT_QUOTEDZPUTS. Mapped
                                // to wrap-only-if-needed pending a faithful
                                // QT_QUOTEDZPUTS port.
                                wrap_only_if_needed = true;
                                i += 1;
                            }
                            '-' => {
                                // c:2245-2246 — q- → QT_SINGLE_OPTIONAL.
                                // Currently mapped to strip_trailing_newlines
                                // pending a faithful QT_SINGLE_OPTIONAL port.
                                strip_trailing_newlines = true;
                                i += 1;
                            }
                            _ => break,
                        }
                    }
                    let needs_quoting = |s: &str| -> bool {
                        s.is_empty()
                            || s.chars().any(|c| {
                                c.is_whitespace()
                                    || matches!(
                                        c,
                                        '\'' | '"'
                                            | '\\'
                                            | '$'
                                            | '`'
                                            | '*'
                                            | '?'
                                            | '['
                                            | ']'
                                            | '{'
                                            | '}'
                                            | '('
                                            | ')'
                                            | '|'
                                            | '&'
                                            | ';'
                                            | '<'
                                            | '>'
                                            | '#'
                                            | '~'
                                    )
                            })
                    };
                    let quote_one = |raw: &str| -> String {
                        let s_owned: String;
                        let s = if strip_trailing_newlines {
                            s_owned = raw.trim_end_matches('\n').to_string();
                            s_owned.as_str()
                        } else {
                            raw
                        };
                        if wrap_only_if_needed {
                            // q+: skip quoting if the value is "shell-safe";
                            // otherwise wrap with single-quotes (zsh's q+
                            // promotes to single-bslashquote level when needed).
                            if !needs_quoting(s) {
                                return s.to_string();
                            }
                            return format!("'{}'", s.replace('\'', "'\\''"));
                        }
                        if let Some(ref d) = explicit_delim {
                            // q:str: form — wrap value with the explicit
                            // delimiter on each side, escaping inner d's
                            // with backslash.
                            let escaped = s.replace(d.as_str(), &format!("\\{}", d));
                            return format!("{}{}{}", d, escaped, d);
                        }
                        match level {
                            1 => {
                                // q: backslash-escape every shell-special
                                // char without surrounding quotes. zsh
                                // special-cases the empty string: `${(q)x}`
                                // for empty `x` outputs `''` (a real
                                // single-quoted empty pair) so the
                                // value survives word-splitting in the
                                // consumer.
                                if s.is_empty() {
                                    return "''".to_string();
                                }
                                let mut out = String::with_capacity(s.len() + 4);
                                for c in s.chars() {
                                    if matches!(
                                        c,
                                        ' ' | '\t'
                                            | '\''
                                            | '"'
                                            | '\\'
                                            | '$'
                                            | '`'
                                            | '*'
                                            | '?'
                                            | '['
                                            | ']'
                                            | '{'
                                            | '}'
                                            | '('
                                            | ')'
                                            | '|'
                                            | '&'
                                            | ';'
                                            | '<'
                                            | '>'
                                            | '#'
                                            | '~'
                                    ) {
                                        out.push('\\');
                                    }
                                    out.push(c);
                                }
                                out
                            }
                            2 => {
                                // qq: single-bslashquote, escape inner ' as '\''.
                                let mut escaped = s.replace('\'', "'\\''");
                                if escape_glob_chars {
                                    escaped = escaped.replace('*', "\\*").replace('?', "\\?");
                                }
                                format!("'{}'", escaped)
                            }
                            3 => {
                                // qqq: double-bslashquote, escape $ ` " \\.
                                let mut out = String::with_capacity(s.len() + 2);
                                out.push('"');
                                for c in s.chars() {
                                    match c {
                                        '$' | '`' | '"' | '\\' => {
                                            out.push('\\');
                                            out.push(c);
                                        }
                                        '*' | '?' if escape_glob_chars => {
                                            out.push('\\');
                                            out.push(c);
                                        }
                                        _ => out.push(c),
                                    }
                                }
                                out.push('"');
                                out
                            }
                            _ => {
                                // qqqq: ANSI-C $'…' style.
                                let mut out = String::with_capacity(s.len() + 4);
                                out.push_str("$'");
                                for c in s.chars() {
                                    match c {
                                        '\\' => out.push_str("\\\\"),
                                        '\'' => out.push_str("\\'"),
                                        '\n' => out.push_str("\\n"),
                                        '\t' => out.push_str("\\t"),
                                        '\r' => out.push_str("\\r"),
                                        c if (c as u32) < 0x20 => {
                                            out.push_str(&format!("\\x{:02x}", c as u32));
                                        }
                                        c => out.push(c),
                                    }
                                }
                                out.push('\'');
                                out
                            }
                        }
                    };
                    state = match state {
                        St::S(s) => St::S(quote_one(&s)),
                        St::A(a) => {
                            // Empty array under `(q)`/`(qq)` flag emits a
                            // single empty quoted pair (`''`) — zsh treats
                            // the empty array as `[""]` for quoting so the
                            // result still occupies a slot. Without this
                            // special case, `${(qq)a}` for an empty `a`
                            // produced an actually-empty string.
                            if a.is_empty() {
                                St::A(vec![quote_one("")])
                            } else {
                                St::A(a.into_iter().map(|s| quote_one(&s)).collect())
                            }
                        }
                    };
                }
                'g' => {
                    // Process backslash escapes (`\n`, `\t`, `\r`, `\\`,
                    // `\xNN`, `\NNN` octal). Applied to the current scalar
                    // or each array element.
                    let unescape = |s: &str| -> String {
                        let mut out = String::with_capacity(s.len());
                        let mut chars = s.chars().peekable();
                        while let Some(c) = chars.next() {
                            if c != '\\' {
                                out.push(c);
                                continue;
                            }
                            match chars.next() {
                                Some('n') => out.push('\n'),
                                Some('t') => out.push('\t'),
                                Some('r') => out.push('\r'),
                                Some('\\') => out.push('\\'),
                                Some('\'') => out.push('\''),
                                Some('"') => out.push('"'),
                                Some('0') => out.push('\0'),
                                Some('a') => out.push('\x07'),
                                Some('b') => out.push('\x08'),
                                Some('f') => out.push('\x0c'),
                                Some('v') => out.push('\x0b'),
                                Some('x') => {
                                    let mut hex = String::new();
                                    for _ in 0..2 {
                                        if let Some(&h) = chars.peek() {
                                            if h.is_ascii_hexdigit() {
                                                hex.push(h);
                                                chars.next();
                                            } else {
                                                break;
                                            }
                                        }
                                    }
                                    if let Ok(b) = u8::from_str_radix(&hex, 16) {
                                        out.push(b as char);
                                    }
                                }
                                Some(other) => {
                                    out.push('\\');
                                    out.push(other);
                                }
                                None => out.push('\\'),
                            }
                        }
                        out
                    };
                    state = match state {
                        St::S(s) => St::S(unescape(&s)),
                        St::A(a) => St::A(a.into_iter().map(|s| unescape(&s)).collect()),
                    };
                }
                'n' => {
                    // Numeric sort. Direct port of src/zsh/Src/sort.c:137-172
                    // (eltpcmp's `if (sortnumeric)` block) and subst.c:2217
                    // (case 'n' sets SORTIT_NUMERICALLY).
                    //
                    // Two flavors per zsh — controlled by sortnumeric value:
                    //   1  (positive)  — unsigned. A leading `-` is just
                    //                   another non-digit char and is
                    //                   compared lexicographically. (n)
                    //                   alone takes this path.
                    //   -1 (negative) — signed. A `-` immediately preceding
                    //                   digits flips the comparison so that
                    //                   `-5 < -3 < 1`. Triggered by the
                    //                   `-` flag char per subst.c:2220-2222
                    //                   (case '-': sortit |= NUMERICALLY_SIGNED).
                    //
                    // We pre-scan the flag string for a literal `-` after
                    // the `n` to enable signed mode. This matches the order-
                    // independent behavior of zsh's flag dispatch (any
                    // `-` in the (...) group enables signed mode for the
                    // numeric sort).
                    let signed = chars.contains(&'-');
                    fn natural_cmp(a: &str, b: &str, signed: bool) -> std::cmp::Ordering {
                        use std::cmp::Ordering;
                        if signed {
                            // Strip a leading sign and compare numerically
                            // when both look like signed integers. Falls
                            // back to per-char compare when not numeric.
                            let parse_signed = |s: &str| -> Option<i128> {
                                let bytes = s.as_bytes();
                                if bytes.is_empty() {
                                    return None;
                                }
                                let (neg, rest) = match bytes[0] {
                                    b'-' if bytes.len() > 1 && bytes[1].is_ascii_digit() => {
                                        (true, &s[1..])
                                    }
                                    b'+' if bytes.len() > 1 && bytes[1].is_ascii_digit() => {
                                        (false, &s[1..])
                                    }
                                    c if c.is_ascii_digit() => (false, s),
                                    _ => return None,
                                };
                                rest.parse::<i128>().ok().map(|n| if neg { -n } else { n })
                            };
                            if let (Some(va), Some(vb)) = (parse_signed(a), parse_signed(b)) {
                                return va.cmp(&vb);
                            }
                            // fall through to natural compare below
                        }
                        let mut ai = a.chars().peekable();
                        let mut bi = b.chars().peekable();
                        loop {
                            match (ai.peek(), bi.peek()) {
                                (None, None) => return Ordering::Equal,
                                (None, _) => return Ordering::Less,
                                (_, None) => return Ordering::Greater,
                                (Some(ca), Some(cb))
                                    if ca.is_ascii_digit() && cb.is_ascii_digit() =>
                                {
                                    let mut na = String::new();
                                    while let Some(&c) = ai.peek() {
                                        if c.is_ascii_digit() {
                                            na.push(c);
                                            ai.next();
                                        } else {
                                            break;
                                        }
                                    }
                                    let mut nb = String::new();
                                    while let Some(&c) = bi.peek() {
                                        if c.is_ascii_digit() {
                                            nb.push(c);
                                            bi.next();
                                        } else {
                                            break;
                                        }
                                    }
                                    let va: u128 = na.parse().unwrap_or(0);
                                    let vb: u128 = nb.parse().unwrap_or(0);
                                    match va.cmp(&vb) {
                                        Ordering::Equal => continue,
                                        ord => return ord,
                                    }
                                }
                                (Some(&ca), Some(&cb)) => {
                                    ai.next();
                                    bi.next();
                                    match ca.cmp(&cb) {
                                        Ordering::Equal => continue,
                                        ord => return ord,
                                    }
                                }
                            }
                        }
                    }
                    state = match state {
                        St::A(mut a) => {
                            a.sort_by(|x, y| natural_cmp(x, y, signed));
                            St::A(a)
                        }
                        s => s,
                    };
                }
                '-' => {
                    // `(-)` — signed-numeric sort modifier per
                    // src/zsh/Src/subst.c:2220-2222. The actual sort
                    // happens in the `n` arm above; this arm just
                    // consumes the flag char so unrecognized-flag
                    // paths don't trip on it.
                }
                'i' => {
                    // Case-insensitive sort. Re-applies sort using lowercase
                    // comparison; if the array isn't sorted, this is the
                    // sort-key.
                    state = match state {
                        St::A(mut a) => {
                            a.sort_by_key(|x| x.to_lowercase());
                            St::A(a)
                        }
                        s => s,
                    };
                }
                't' => {
                    // Type query. zsh's `(t)` flag returns the base
                    // type plus any attribute markers separated by `-`.
                    // Examples: `integer`, `float`, `scalar-readonly`,
                    // `scalar-export`, `scalar-left` (typeset -L N),
                    // `scalar-right_blanks`, `array`, `association`.
                    //
                    // `(Pt)` combo: direct port of Src/subst.c:2807-2854.
                    // zsh's `wantt` reads `v->pm->node.flags` AFTER
                    // `aspar` has resolved the indirect target's Param.
                    // We mirror that: for (Pt), look up `name`'s scalar
                    // value to get the target name, then introspect
                    // THAT parameter's type. The value pre-walker was
                    // skipped above for the Pt combo.
                    let target = if pt_combo {
                        with_executor(|exec| exec.get_variable(&name))
                    } else {
                        name.clone()
                    };
                    let kind = with_executor(|exec| {
                        if let Some(attr) = exec.var_attrs.get(&target) {
                            return attr.format_zsh();
                        }
                        if exec.assoc_arrays.contains_key(&target) {
                            "association".to_string()
                        } else if exec.arrays.contains_key(&target) {
                            "array".to_string()
                        } else if exec.variables.contains_key(&target)
                            || std::env::var(&target).is_ok()
                        {
                            "scalar".to_string()
                        } else {
                            String::new()
                        }
                    });
                    state = St::S(kind);
                }
                '%' => {
                    // Prompt expansion: process %F %B %f %{ %} etc. via the
                    // executor's expand_prompt. Useful for building prompts
                    // out of stored fragments.
                    state = match state {
                        St::S(s) => St::S(with_executor(|exec| exec.expand_prompt_string(&s))),
                        St::A(a) => St::A(
                            a.into_iter()
                                .map(|s| with_executor(|exec| exec.expand_prompt_string(&s)))
                                .collect(),
                        ),
                    };
                }
                'e' => {
                    // Per zshexpn(1): "perform parameter expansion,
                    // command substitution and arithmetic expansion
                    // on the resulting word". Apply expand_string so
                    // `\$var` (literal `$var` in the value) becomes
                    // the value of $var, `\$(cmd)` runs the cmd, etc.
                    let eval_one =
                        |s: &str| -> String { with_executor(|exec| exec.singsub(s)) };
                    state = match state {
                        St::S(s) => St::S(eval_one(&s)),
                        St::A(a) => St::A(a.into_iter().map(|s| eval_one(&s)).collect()),
                    };
                }
                'p' => {
                    // Print-style escape processing (mirrors print -e). Same
                    // as `g` for the escape set we support — they differ in
                    // zsh on some niche `\c` and `\E` forms, which we map
                    // identically.
                    let unescape = |s: &str| -> String {
                        let mut out = String::with_capacity(s.len());
                        let mut chars = s.chars().peekable();
                        while let Some(c) = chars.next() {
                            if c != '\\' {
                                out.push(c);
                                continue;
                            }
                            match chars.next() {
                                Some('n') => out.push('\n'),
                                Some('t') => out.push('\t'),
                                Some('r') => out.push('\r'),
                                Some('\\') => out.push('\\'),
                                Some('e') | Some('E') => out.push('\x1b'),
                                Some(other) => {
                                    out.push('\\');
                                    out.push(other);
                                }
                                None => out.push('\\'),
                            }
                        }
                        out
                    };
                    state = match state {
                        St::S(s) => St::S(unescape(&s)),
                        St::A(a) => St::A(a.into_iter().map(|s| unescape(&s)).collect()),
                    };
                }
                'A' => {
                    // Coerce to array shape (alias of @). Mostly affects
                    // downstream flags that treat scalar vs array
                    // differently.
                    state = match state {
                        St::S(s) => St::A(vec![s]),
                        a => a,
                    };
                }
                '~' => {
                    // Pattern-toggle: in zsh this enables glob-pattern
                    // interpretation of the value in subsequent matches. The
                    // bytecode dispatch already glob-matches via `Op::StrMatch`
                    // when relevant; without a stateful match-context this
                    // flag is a no-op pass-through. tracing::debug records
                    // the request.
                    tracing::debug!("PARAM_FLAG ~ — no-op pass-through (no match-context state)");
                }
                'p' => {
                    // `(p)` — print-style escapes for OTHER flag args.
                    // Already detected by the pre-scan above; here we
                    // just consume the flag char without mutating
                    // state (no-op on the value itself). Matches
                    // src/zsh/Src/subst.c:2381-2382.
                }
                'g' => {
                    // `(g)` — apply print-style escape decoding to
                    // the operand value itself, with sub-flags
                    // selecting which escape conventions to honor.
                    // Sub-flags from src/zsh/Src/subst.c:2409-2436:
                    //   e — emacs-style: \C-x, \M-x, \e
                    //   o — octal: \NNN
                    //   c — caret notation: ^X for control chars
                    // We honor any combination by running the same
                    // C-style interpreter that `(p)` uses on `(s::)`
                    // args; sub-flags currently widen but do not
                    // narrow the escape set.
                    if i < chars.len() && ZshrsHost::is_zsh_flag_delim(chars[i]) {
                        let d = chars[i];
                        i += 1;
                        // Consume the sub-flag chars (e/o/c) — recorded
                        // for documentation; the escape interpreter
                        // below already handles all three cases.
                        while i < chars.len() && chars[i] != d {
                            i += 1;
                        }
                        if i < chars.len() {
                            i += 1; // skip closing delim
                        }
                    }
                    state = match state {
                        St::S(s) => St::S(print_escape_str(&s)),
                        St::A(a) => St::A(a.into_iter().map(|s| print_escape_str(&s)).collect()),
                    };
                }
                '_' => {
                    // `(_)` — reserved for future use per
                    // src/zsh/Src/subst.c:2485-2502. Consume the
                    // delim-bracketed arg if present so we don't
                    // mis-parse subsequent flags.
                    if i < chars.len() && ZshrsHost::is_zsh_flag_delim(chars[i]) {
                        let d = chars[i];
                        i += 1;
                        while i < chars.len() && chars[i] != d {
                            i += 1;
                        }
                        if i < chars.len() {
                            i += 1;
                        }
                    }
                }
                'b' | 'B' => {
                    // (b)/(B) — backslash-escape shell + pattern metas
                    // (whitespace, glob/redirect/bslashquote/expansion specials).
                    let escape = |s: &str| -> String {
                        let mut r = String::new();
                        for c in s.chars() {
                            if "\\*?[]{}()<>&|;\"'$`!#~ \t\n".contains(c) {
                                r.push('\\');
                            }
                            r.push(c);
                        }
                        r
                    };
                    state = match state {
                        St::S(s) => St::S(escape(&s)),
                        St::A(a) => St::A(a.iter().map(|s| escape(s)).collect()),
                    };
                }
                _ => {
                    // Unknown flag — silently skip. The maintainer's "no
                    // friendly nags" rule means we don't print "unsupported
                    // flag X"; tracing::debug records it in the log.
                    tracing::debug!(flag = %c, "BUILTIN_PARAM_FLAG: unknown flag");
                }
            }
        }

        // Direct port of Src/subst.c:3901-3933. When the caller is in
        // DQ context AND the state landed in `St::A` (e.g. via `(f)`
        // line-split, `(s:…:)` arbitrary split, or assoc/array seed
        // with no `[@]` splice), zsh's paramsubst joins the array back
        // into a single scalar via `sepjoin(aval, sep, 1)`:
        //
        //   • If `sep` is non-NULL (set by `(F)` / `(j:…:)`), join
        //     with that exact separator.
        //   • Else if `spsep` is non-NULL (set by `(f)` / `(s:…:)`),
        //     `sepjoin` falls back to the first IFS char (space by
        //     default for `IFS=$' \t\n'`).
        //
        // Without this, `echo "[${(f)x}]"` (DQ) would word-split the
        // array into 3 separate echo args (`[line1] [line2] [line3]`)
        // instead of zsh's `[line1 line2 line3]`. The explicit `[@]`
        // splice operator OR `(@)` flag suppresses this collapse —
        // both already covered by `has_at_subscript` above.
        //
        // Skip the collapse when nested inside ANOTHER `${...}` —
        // `${${(f)x}[2]}` needs the inner `(f)` to keep its array
        // shape so the outer `[2]` can subscript element-2. C zsh
        // tracks this through paramsubst's recursion (the inner call
        // returns aval; outer operates on aval before any sepjoin).
        // We detect the same condition via `in_paramsubst_nest`,
        // bumped by every BUILTIN_PARAM_FLAG / BUILTIN_PARAM_*
        // recursion entry.
        // The DQ collapse fires only for "bare" arrays — those that
        // came from `${arr}` / `${assoc}` without a split flag. When
        // any split flag (`(z)`, `(f)`, `(s:STR:)`, `(0)`, `(=)`) was
        // applied the array shape is INTENTIONAL: zsh keeps it
        // multi-word inside DQ. Direct port of Src/subst.c's
        // `nojoin` behavior — the split flags set nojoin=1 which
        // causes paramsubst to skip sepjoin even in DQ.
        let split_flag_active = flags.contains('z')
            || flags.contains('f')
            || flags.contains('s')
            || flags.contains('0')
            || flags.contains('=');
        let is_nested = with_executor(|exec| exec.in_paramsubst_nest > 1);
        if (dq_compile || dq_runtime) && !has_at_subscript && !is_nested && !split_flag_active {
            if let St::A(a) = state {
                // Pick the join separator. `(F)` (the last F seen) is
                // tracked via `flags.contains('F')`; `(j:str:)` runs
                // earlier in the loop and stores the result already
                // joined as `St::S(_)`, so we only see `St::A` here
                // for split-style flags. The default is the first
                // char of $IFS (space when IFS is the zsh default).
                let sep = if flags.contains('F') {
                    "\n".to_string()
                } else {
                    with_executor(|exec| {
                        let ifs = exec.get_variable("IFS");
                        ifs.chars().next().map(|c| c.to_string()).unwrap_or_else(|| " ".to_string())
                    })
                };
                return Value::str(a.join(&sep));
            }
        }

        match state {
            St::S(s) => Value::str(s),
            St::A(a) => Value::Array(a.into_iter().map(Value::str).collect()),
        }
    });

    // `foo[key]=val` — single-key set on an assoc array. Stack: [name, key, value].
    vm.register_builtin(BUILTIN_SET_ASSOC, |vm, _argc| {
        let value = vm.pop().to_str();
        let key = vm.pop().to_str();
        let name = vm.pop().to_str();
        with_executor(|exec| {
            // PFA-SMR aspect: subscript assignment `arr[N]=val` /
            // `assoc[key]=val`. Recorded as a structured assoc/array
            // event with the (key, value) pair preserved in
            // `value_assoc` so replay can reconstruct the exact slot.
            // Path-family arrays come through SET_ARRAY / APPEND_ARRAY,
            // never here, so no path_mod routing.
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                let ctx = exec.recorder_ctx();
                let attrs = exec.recorder_attrs_for(&name);
                crate::recorder::emit_assoc_assign(
                    &name,
                    vec![(key.clone(), value.clone())],
                    attrs,
                    true, // element-add semantics, not full replace
                    ctx,
                );
            }
            // Indexed array element assign `a[N]=val`. Routes here when
            // `name` is already an indexed array. For unset names, only
            // treat as indexed if the key is unambiguously numeric (a
            // literal int) — `foo[key]=val` with no prior storage and
            // a string key should create an assoc (zsh default), not an
            // indexed array. zsh's rule: numeric subscript on an
            // indexed array (or new var with numeric key) assigns to
            // the 1-based slot, growing the array if needed. Negative
            // indices count from the end.
            let is_indexed = exec.arrays.contains_key(&name);
            let is_assoc = exec.assoc_arrays.contains_key(&name);
            let key_literal_int = key.trim().parse::<i64>().ok();
            // For an existing indexed array, fall back to arith eval so
            // `a[i+1]=v` works when `i` is set.
            let key_int_for_indexed = if is_indexed {
                key_literal_int.or_else(|| Some(exec.eval_arith_expr(&key)))
            } else {
                key_literal_int
            };
            let route_indexed = if is_assoc {
                false
            } else if is_indexed {
                key_int_for_indexed.is_some()
            } else {
                key_literal_int.is_some()
            };
            if let (true, Some(i)) = (route_indexed, key_int_for_indexed) {
                let len = exec.arrays.get(&name).map(|a| a.len() as i64).unwrap_or(0);
                let idx = if i > 0 {
                    (i - 1) as usize
                } else if i < 0 {
                    let off = len + i;
                    if off < 0 {
                        return;
                    }
                    off as usize
                } else {
                    // zsh: `a[0]=v` is "assignment to invalid subscript
                    // range" (positionals/arrays are 1-based). Mirror
                    // the diagnostic and abort with status 1.
                    eprintln!("zshrs:1: {}: assignment to invalid subscript range", name);
                    std::process::exit(1);
                };
                let arr = exec.arrays.entry(name.clone()).or_insert_with(Vec::new);
                while arr.len() <= idx {
                    arr.push(String::new());
                }
                arr[idx] = value;
                exec.variables.remove(&name);
                return;
            }
            // Default: assoc set.
            exec.variables.remove(&name);
            exec.assoc_arrays
                .entry(name)
                .or_insert_with(IndexMap::new)
                .insert(key, value);
        });
        Value::Status(0)
    });

    // Brace expansion. Routes through executor.xpandbraces (already
    // implemented for the tree-walker era). Returns Value::Array.
    vm.register_builtin(BUILTIN_WORD_SPLIT, |vm, _argc| {
        let s = vm.pop().to_str();
        let ifs = with_executor(|exec| {
            exec.variables
                .get("IFS")
                .cloned()
                .unwrap_or_else(|| " \t\n".to_string())
        });
        // Direct port of multsub's IFS-split path (src/zsh/Src/subst.c:
        // 567-680). zsh distinguishes WHITESPACE IFS (default) from
        // NON-WHITESPACE IFS:
        //   - whitespace IFS chars (space/tab/newline): runs of separator
        //     collapse and empty fields are SUPPRESSED
        //   - non-whitespace IFS chars: every separator boundary creates a
        //     field, including empties between adjacent separators
        // Mixed IFS treats whitespace runs as collapsing, but a single
        // non-whitespace IFS character creates a field boundary regardless.
        // zsh's default IFS is " \t\n\0" (space, tab, newline, NUL).
        // Treat NUL as whitespace-class so the default-IFS path
        // collapses runs and suppresses empties; without this the
        // NUL char triggered the non-whitespace branch and emitted
        // empty fields between every separator.
        let only_ws = ifs.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\0'));
        let parts: Vec<fusevm::Value> = if only_ws {
            s.split(|c: char| ifs.contains(c))
                .filter(|p| !p.is_empty())
                .map(fusevm::Value::str)
                .collect()
        } else {
            // Non-whitespace IFS: preserve every separator boundary,
            // including empty fields. Matches zsh's behaviour for
            // `IFS=:; ${=a}` on `x:y::z` -> [x, y, "", z].
            s.split(|c: char| ifs.contains(c))
                .map(fusevm::Value::str)
                .collect()
        };
        // zsh: word-splitting an empty value yields ZERO words, not one
        // empty word. `unset b; for w in ${=b}` iterates zero times.
        // Whitespace-IFS path filtered out the empties already; the
        // non-whitespace path may have produced a single-empty Vec from
        // `"".split(...)` which still iterates once — collapse to an
        // empty Array so for-loops and arg expansion see no words.
        if parts.is_empty() || (parts.len() == 1 && parts[0].to_str().is_empty()) {
            fusevm::Value::Array(Vec::new())
        } else if parts.len() == 1 {
            parts.into_iter().next().unwrap()
        } else {
            fusevm::Value::Array(parts)
        }
    });

    vm.register_builtin(BUILTIN_BRACE_EXPAND, |vm, _argc| {
        let s = vm.pop().to_str();
        // Direct call to the canonical brace expander (port of
        // Src/glob.c::xpandbraces at glob.rs:1678). Was stubbed
        // as `vec![s]` — every `print X{1,2,3}Y` returned literal.
        let brace_ccl = with_executor(|exec|
            exec.options.get("braceccl").copied().unwrap_or(false));
        let parts = crate::ported::glob::xpandbraces(&s, brace_ccl);
        if parts.len() == 1 {
            fusevm::Value::str(parts.into_iter().next().unwrap_or_default())
        } else {
            fusevm::Value::Array(parts.into_iter().map(fusevm::Value::str).collect())
        }
    });

    // `[[ s =~ pat ]]` regex match — extra-builtin fallback path so the
    // conditional grammar can route here when Op::RegexMatch isn't wired.
    // Uses the same regex cache as the host method.
    vm.register_builtin(BUILTIN_REGEX_MATCH, |vm, _argc| {
        let pat = vm.pop().to_str();
        let s = vm.pop().to_str();
        // Same untokenize before regex compile as ZshrsHost::regex_match
        // — SNULL/DQ markers from quoted patterns must be stripped
        // before the regex engine sees them. Direct port of
        // bin_test/cond_match's untokenize() call.
        let pat = crate::lex::untokenize(&pat);
        let s = crate::lex::untokenize(&s);
        let mut cache = REGEX_CACHE.lock();
        let matched = if let Some(re) = cache.get(&pat) {
            re.is_match(&s)
        } else {
            match regex::Regex::new(&pat) {
                Ok(re) => {
                    let m = re.is_match(&s);
                    cache.insert(pat.clone(), re);
                    m
                }
                Err(_) => false,
            }
        };
        if matched {
            Value::Status(0)
        } else {
            Value::Status(1)
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

    vm.register_builtin(BUILTIN_GLOB_QUALIFIED, |vm, _argc| {
        let qual = vm.pop().to_str();
        let pattern = vm.pop().to_str();
        let nullglob = qual.contains('N');
        let mut matches = with_executor(|exec| exec.expand_glob(&pattern));
        if matches.is_empty() && !nullglob {
            // Default: keep the unmatched pattern (zsh's default unless N is set)
            return fusevm::Value::Array(vec![fusevm::Value::str(pattern)]);
        }
        // Filter by predicates that require stat
        matches.retain(|path| {
            use std::fs;
            use std::os::unix::fs::PermissionsExt;
            // zsh's `-` modifier in glob qualifiers (`*(-.)`) means
            // "follow symlinks before applying the test". Without
            // `-`, `(.)` uses lstat (skipping symlinks even when
            // they target a regular file). Direct port of zsh's
            // pattern.c qualifier parser — the QUAL_NULL bit is set
            // by `-` and switches stat→lstat-vs-stat. Default Rust
            // `fs::metadata` follows symlinks; use `symlink_metadata`
            // by default, switch to `metadata` when `-` is in the
            // qualifier set.
            let follow_symlinks = qual.contains('-');
            let meta_res = if follow_symlinks {
                fs::metadata(path)
            } else {
                fs::symlink_metadata(path)
            };
            let meta = match meta_res {
                Ok(m) => m,
                Err(_) => return qual.contains('N'),
            };
            let mut keep = true;
            for c in qual.chars() {
                match c {
                    '.' => keep &= meta.is_file(),
                    '/' => keep &= meta.is_dir(),
                    '@' => {
                        // is_symlink requires fs::symlink_metadata for the
                        // path itself, not the target.
                        keep &= fs::symlink_metadata(path)
                            .map(|m| m.file_type().is_symlink())
                            .unwrap_or(false);
                    }
                    'x' => {
                        keep &= meta.permissions().mode() & 0o111 != 0;
                    }
                    'r' => {
                        keep &= meta.permissions().mode() & 0o444 != 0;
                    }
                    'w' => {
                        keep &= meta.permissions().mode() & 0o222 != 0;
                    }
                    _ => {}
                }
                if !keep {
                    break;
                }
            }
            keep
        });
        // Sort modifiers
        if qual.contains("on") || qual.contains('o') && !qual.contains("om") && !qual.contains("oL")
        {
            matches.sort();
        }
        if qual.contains("On")
            || (qual.contains('O') && !qual.contains("Om") && !qual.contains("OL"))
        {
            matches.sort();
            matches.reverse();
        }
        if qual.contains("oL") {
            matches.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0));
        }
        if qual.contains("OL") {
            matches.sort_by_key(|p| {
                std::cmp::Reverse(std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            });
        }
        if qual.contains("om") {
            matches.sort_by_key(|p| {
                std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .map(|t| {
                        std::cmp::Reverse(
                            t.duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0),
                        )
                    })
                    .unwrap_or(std::cmp::Reverse(0))
            });
        }
        if qual.contains("Om") {
            matches.sort_by_key(|p| {
                std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .map(|t| {
                        t.duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                    })
                    .unwrap_or(0)
            });
        }
        // (M) mark-dirs / (T) list-types qualifiers — direct port of
        // zsh/Src/glob.c:1557-1566 (case 'M' / case 'T'). zsh appends
        // a single char to each output (or only to dirs for `M`):
        //   /  directory      *  executable regular file
        //   @  symlink        |  fifo
        //   =  socket         #  block device   %  char device
        //
        // M alone marks ONLY directories with `/`; T marks every
        // file with its type char. Both sourced from glob.c:355,372
        // emit-side logic on gf_markdirs / gf_listtypes flags.
        let mark_dirs = qual.contains('M');
        let list_types = qual.contains('T');
        if mark_dirs || list_types {
            matches = matches
                .into_iter()
                .map(|p| {
                    use std::os::unix::fs::PermissionsExt;
                    let meta = match std::fs::symlink_metadata(&p) {
                        Ok(m) => m,
                        Err(_) => return p,
                    };
                    let mode = meta.permissions().mode();
                    let ch = crate::glob::file_type(mode);
                    if list_types || (mark_dirs && ch == '/') {
                        format!("{}{}", p, ch)
                    } else {
                        p
                    }
                })
                .collect();
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

    // `m[k]+=tail` — append onto the existing value (string concat). Mirrors
    // zsh's += behavior on assoc-array entries. Missing key creates it with
    // just `tail`, matching SET_ASSOC's create-on-demand.
    vm.register_builtin(BUILTIN_APPEND_ASSOC, |vm, _argc| {
        let tail = vm.pop().to_str();
        let key = vm.pop().to_str();
        let name = vm.pop().to_str();
        with_executor(|exec| {
            exec.variables.remove(&name);
            let map = exec.assoc_arrays.entry(name.clone()).or_insert_with(IndexMap::new);
            match map.get_mut(&key) {
                Some(existing) => existing.push_str(&tail),
                None => {
                    map.insert(key.clone(), tail.clone());
                }
            }
            // PFA-SMR aspect: assoc subscript-append `m[k]+=tail`.
            // Recorder emits a structured assoc event with the
            // POST-append value so replay reconstructs end state
            // directly (no need to model the +=tail concat).
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                let ctx = exec.recorder_ctx();
                let attrs = exec.recorder_attrs_for(&name);
                let new_val = exec
                    .assoc_arrays
                    .get(&name)
                    .and_then(|m| m.get(&key))
                    .cloned()
                    .unwrap_or_default();
                crate::recorder::emit_assoc_assign(
                    &name,
                    vec![(key.clone(), new_val)],
                    attrs,
                    true,
                    ctx,
                );
            }
        });
        Value::Status(0)
    });

    vm.register_builtin(BUILTIN_ARRAY_LENGTH, |vm, _argc| {
        let name = vm.pop().to_str();
        let len = with_executor(|exec| exec.arrays.get(&name).map(|a| a.len()).unwrap_or(0));
        Value::str(len.to_string())
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
                exec.options.insert(opt, true);
            } else {
                exec.options.remove(&opt);
            }
        });
        Value::Status(0)
    });

    vm.register_builtin(BUILTIN_ARRAY_JOIN_STAR, |vm, _argc| {
        let name = vm.pop().to_str();
        let result = with_executor(|exec| {
            let sep = exec
                .variables
                .get("IFS")
                .and_then(|s| s.chars().next())
                .map(|c| c.to_string())
                .unwrap_or_else(|| " ".to_string());
            if name == "@" || name == "*" || name == "argv" {
                return exec.positional_params.join(&sep);
            }
            if let Some(arr) = exec.arrays.get(&name) {
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
                return Value::Array(exec.positional_params.iter().map(Value::str).collect());
            }
            match exec.arrays.get(&name) {
                Some(v) => Value::Array(v.iter().map(Value::str).collect()),
                None => {
                    // Fall back to scalar lookup. zsh (unlike bash)
                    // does NOT IFS-split a scalar variable in a for
                    // list — `for w in $scalar` iterates ONCE with the
                    // scalar value. Word-splitting requires either
                    // sh_word_split option or explicit `${(s.,.)scalar}`.
                    let val = exec.get_variable(&name);
                    if val.is_empty()
                        && !exec.variables.contains_key(&name)
                        && std::env::var(&name).is_err()
                    {
                        Value::Array(vec![])
                    } else if exec.options.get("shwordsplit").copied().unwrap_or(false) {
                        // bash-compat: under setopt sh_word_split, do
                        // split scalars on IFS chars.
                        let ifs = exec
                            .variables
                            .get("IFS")
                            .cloned()
                            .unwrap_or_else(|| " \t\n".to_string());
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
                    exec.variables.remove(&name);
                    exec.arrays
                        .insert(name, vec![read_fd.to_string(), write_fd.to_string()]);
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
            exec.last_status = live_status;
        };
        if name == "@" || name == "*" {
            return with_executor(|exec| {
                sync_status(exec);
                fusevm::Value::Array(
                    exec.positional_params
                        .iter()
                        .map(fusevm::Value::str)
                        .collect(),
                )
            });
        }
        // RC_EXPAND_PARAM: when the option is set and `name` refers to
        // an array, return Value::Array so the enclosing word's
        // BUILTIN_CONCAT_DISTRIBUTE distributes element-wise. Without
        // the option, arrays still join to a space-separated scalar
        // (zsh's default unquoted-array-as-scalar semantics).
        let rc_expand =
            with_executor(|exec| exec.options.get("rcexpandparam").copied().unwrap_or(false));
        if rc_expand {
            let arr_val = with_executor(|exec| {
                sync_status(exec);
                exec.arrays.get(&name).cloned()
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
            crate::exec_shims::scan_magic_assoc_keys(&name).map(|keys| {
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
            let ksh_arrays = exec.options.get("ksharrays").copied().unwrap_or(false);
            if let Some(arr) = exec.arrays.get(&name) {
                if ksh_arrays {
                    return Some((vec![arr.first().cloned().unwrap_or_default()], in_dq));
                }
                return Some((arr.clone(), in_dq));
            }
            if let Some(map) = exec.assoc_arrays.get(&name) {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let values: Vec<String> = keys
                    .iter()
                    .filter_map(|k| map.get(*k).cloned())
                    .collect();
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
            if let Some(arr) = exec.arrays.get_mut(&name) {
                arr.push(value.clone());
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
            if exec.assoc_arrays.contains_key(&name) {
                eprintln!("zshrs: {}: cannot use += on assoc without (key val)", name);
                return;
            }
            // typeset -i: `+=` is arithmetic add, not string concat.
            // `typeset -i x=42; x+=8` must store 50, not "428".
            let is_integer = exec
                .var_attrs
                .get(&name)
                .map(|a| matches!(a.kind, VarKind::Integer))
                .unwrap_or(false);
            if is_integer {
                let prev = exec.get_variable(&name);
                let prev_n: i64 = prev.parse().unwrap_or(0);
                let added = exec.eval_arith_expr(&value);
                let new_val = (prev_n + added).to_string();
                exec.variables.insert(name.clone(), new_val.clone());
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
            exec.variables.insert(name.clone(), combined.clone());
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
                    emit_path_or_assign(
                        &name,
                        std::slice::from_ref(&combined),
                        attrs,
                        true,
                        &ctx,
                    );
                } else {
                    crate::recorder::emit_assign_typed(&name, &combined, attrs, ctx);
                }
            }
        });
        Value::Status(0)
    });

    vm.register_builtin(BUILTIN_SET_VAR, |vm, argc| {
        let args = pop_args(vm, argc);
        let mut iter = args.into_iter();
        let name = iter.next().unwrap_or_default();
        let value = iter.next().unwrap_or_default();
        let blocked = with_executor(|exec| {
            // zsh has a fixed set of intrinsic read-only specials that
            // can never be assigned to from script. This is a hard
            // wired list (params.c `ROVAR` flag) — not user-settable.
            // NOTE: `_` is NOT readonly — zsh allows assignments to
            // and `unset` of it (it's just the last-arg auto-update).
            // ZSH_ARGZERO is also writable in zsh per Src/params.c
            // (uses PM_SCALAR without PM_READONLY); zinit's startup
            // line `ZSH_ARGZERO=$0` relies on this.
            let is_intrinsic_ro = matches!(
                name.as_str(),
                "PPID" | "LINENO" | "argv0" | "ARGC"
            );
            let is_ro = is_intrinsic_ro
                || exec.readonly_vars.contains(&name)
                || exec
                    .var_attrs
                    .get(&name)
                    .map(|a| a.readonly)
                    .unwrap_or(false);
            if is_ro {
                eprintln!("zshrs:1: read-only variable: {}", name);
                // Mirror zsh -c: read-only assignment failure aborts
                // the shell with status 1, not just the command.
                std::process::exit(1);
            }
            // If the variable was previously declared `integer` (or
            // `typeset -i`), arith-evaluate the value before storing.
            // zsh: `integer i; i=5*3` stores 15.
            let attrs = exec.var_attrs.get(&name).cloned();
            let is_integer = attrs
                .as_ref()
                .map(|a| matches!(a.kind, VarKind::Integer))
                .unwrap_or(false);
            let int_base = attrs.as_ref().and_then(|a| a.int_base);
            let stored = if is_integer && !value.is_empty() {
                let evaluated = exec.eval_arith_expr(&value).to_string();
                // Apply `typeset -i N` base-formatting at storage time.
                // Without this, `typeset -i 16 x; x=255` stored `255`
                // instead of zsh's `16#FF` form.
                if let Some(base) = int_base {
                    evaluated
                        .parse::<i64>()
                        .map(|n| format_int_in_base(n, base))
                        .unwrap_or(evaluated)
                } else {
                    evaluated
                }
            } else {
                value.clone()
            };
            // Mirror scalar→array if name is the scalar side of a
            // typeset -T tie. Direct port of Src/params.c PM_TIED:
            // assigning to PATH must update both `path` (the array
            // mirror) and the process env (so child execs see the
            // new value, and so find_in_path / external lookups
            // resolve correctly). Without the env::set_var step
            // here, `PATH=/nope; ls` continued to find ls via the
            // shell's startup-time env PATH.
            if let Some((arr_name, sep)) = exec.tied_scalar_to_array.get(&name).cloned() {
                let parts: Vec<String> = if stored.is_empty() {
                    Vec::new()
                } else {
                    stored.split(&sep).map(String::from).collect()
                };
                exec.arrays.insert(arr_name, parts);
                std::env::set_var(&name, &stored);
                // Clear the command hash on PATH change so subsequent
                // command lookups walk the new PATH instead of
                // returning stale absolute paths from before the
                // assignment. zsh's bin_set rehashes lazily; this is
                // the simplest equivalent.
                if name == "PATH" {
                    exec.command_hash.clear();
                }
            }
            // zsh enforces a minimum of 1 on `HISTSIZE` — `HISTSIZE=0`
            // and `HISTSIZE=-5` both clamp to `1`. Mirror at storage
            // time so subsequent reads return the clamped value.
            let stored = if name == "HISTSIZE" {
                stored
                    .parse::<i64>()
                    .map(|n| n.max(1).to_string())
                    .unwrap_or_else(|_| stored.clone())
            } else {
                stored
            };
            // If we're inside an inline-assignment frame (`X=foo cmd`
            // is currently exec'ing the prefix), record the previous
            // value so END_INLINE_ENV can restore it after the command
            // returns. Then export the new value to the env so the
            // child sees it. zsh's `X=foo cmd` semantics: shell
            // variable AND env entry both vanish after cmd returns.
            let in_inline_env = !exec.inline_env_stack.is_empty();
            if in_inline_env {
                let prev_var = exec.variables.get(&name).cloned();
                let prev_env = std::env::var(&name).ok();
                exec.inline_env_stack
                    .last_mut()
                    .unwrap()
                    .push((name.clone(), prev_var, prev_env));
                std::env::set_var(&name, &stored);
            }
            exec.variables.insert(name.clone(), stored.clone());
            // `set -o allexport`: every assignment auto-exports the var.
            // zsh: `setopt allexport; a=42; env | grep ^a=` prints `a=42`.
            // Without this, env didn't see user-set scalars.
            let allexport = exec.options.get("allexport").copied().unwrap_or(false);
            let already_exported = exec.var_attrs.get(&name).map(|a| a.export).unwrap_or(false);
            if allexport || already_exported {
                std::env::set_var(&name, &stored);
            }
            // PFA-SMR aspect: every top-level scalar assignment
            // (`VAR=value`) compiles to BUILTIN_SET_VAR, so this is the
            // chokepoint. Skip the recorder when inside a function scope
            // (those are runtime locals, not config state) and skip the
            // intrinsic specials zsh maintains itself.
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
                crate::recorder::emit_assign_typed(&name, &stored, attrs, ctx);
            }
            false
        });
        if blocked {
            return Value::Status(1);
        }
        // Propagate cmd-subst's exit status to $?. zsh: `a=$(false);
        // echo $?` → 1. run_command_substitution sets last_status
        // before returning; we pick it up here so the assignment's
        // status reflects the cmd-subst result.
        //
        // CRITICAL: read `vm.last_status` (live), NOT
        // `exec.last_status` (stale — only synced at statement
        // boundaries; see the BUILTIN_RETURN handler ~line 1003).
        // compile_assign emits LoadInt(0) + SetStatus BEFORE the
        // RHS is evaluated specifically to clear the live status,
        // so a plain assignment (no cmd-subst) reads back 0 and a
        // `$(...)` value reads back the subst's exit. Reading the
        // stale exec field here would always propagate the previous
        // command's status, breaking `false; a=plain; echo $?` → 1
        // (should be 0).
        let captured = vm.last_status;
        Value::Status(captured)
    });

    // BUILTIN_REGISTER_FUNCTION (id 282) was a legacy JSON-AST body
    // bridge. ZshCompiler emits BUILTIN_REGISTER_COMPILED_FN (id 305)
    // instead, which carries a base64 bincode of an already-compiled
    // Chunk. The constant + handler are removed; the ID stays reserved.

    // Pre-compiled function registration — used by compile_zsh.rs's
    // FuncDef path. Stack: [name, base64-bincode-of-Chunk]. We decode
    // the base64, deserialize the Chunk, and store directly in
    // executor.functions_compiled. Bypasses the ShellCommand JSON layer.
    // `[[ -v name ]]` — true iff `name` is a set variable (incl. set-empty,
    // arrays, assoc arrays, and exported env vars). Pops one string, pushes
    // Bool. Matches bash's -v semantics; zsh's `(t)` flag overlaps.
    vm.register_builtin(BUILTIN_VAR_EXISTS, |vm, _argc| {
        let name = vm.pop().to_str();
        // `[[ -v a[N] ]]` checks element existence, not just the array.
        // Split on `[`, look up the array, and verify the resolved
        // index falls within the populated range. `[[ -v h[key] ]]`
        // checks an associative array key.
        if let Some(open) = name.find('[') {
            if name.ends_with(']') {
                let arr_name = &name[..open];
                let key = &name[open + 1..name.len() - 1];
                let exists = with_executor(|exec| {
                    if let Some(arr) = exec.arrays.get(arr_name) {
                        // 1-based index, supports negatives.
                        let parsed = key.parse::<i64>().ok();
                        if let Some(i) = parsed {
                            let len = arr.len() as i64;
                            let resolved = if i < 0 { len + i + 1 } else { i };
                            return resolved >= 1 && resolved <= len;
                        }
                        return false;
                    }
                    if let Some(h) = exec.assoc_arrays.get(arr_name) {
                        return h.contains_key(key);
                    }
                    false
                });
                return fusevm::Value::Bool(exists);
            }
        }
        let exists = with_executor(|exec| {
            // Positional parameter test: `[[ -v N ]]` for an integer N
            // checks whether `$N` is set — i.e. there are at least N
            // positional params. The digit name otherwise won't exist
            // in `variables` unless explicitly assigned.
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(n) = name.parse::<usize>() {
                    if n == 0 {
                        return exec.variables.contains_key("0");
                    }
                    return n <= exec.positional_params.len();
                }
            }
            exec.variables.contains_key(&name)
                || exec.arrays.contains_key(&name)
                || exec.assoc_arrays.contains_key(&name)
                || std::env::var(&name).is_ok()
        });
        fusevm::Value::Bool(exists)
    });

    // `time { compound; ... }` — runs the sub-chunk and prints elapsed
    // wall-clock time. zsh's full `time` also tracks user/system CPU via
    // getrusage on the *child*; we approximate via wall-time only since
    // the sub-chunk runs in-process (no fork). Output format matches
    // `time simple-cmd` (already implemented elsewhere via exectime).
    vm.register_builtin(BUILTIN_TIME_SUBLIST, |vm, _argc| {
        use std::time::Instant;
        let sub_idx = vm.pop().to_int() as usize;
        let chunk_opt = vm.chunk.sub_chunks.get(sub_idx).cloned();
        let Some(chunk) = chunk_opt else {
            return Value::Status(0);
        };
        let start = Instant::now();
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
            exec.variables.insert(varid, final_fd.to_string());
        });
        Value::Status(0)
    });

    // BUILTIN_SET_TRY_BLOCK_ERROR — capture the try-block's exit status
    // into $TRY_BLOCK_ERROR so the always-arm can read it.
    vm.register_builtin(BUILTIN_SET_TRY_BLOCK_ERROR, |vm, _argc| {
        let vm_status = vm.last_status;
        with_executor(|exec| {
            exec.variables
                .insert("TRY_BLOCK_ERROR".to_string(), vm_status.to_string());
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
                            exec.variables.insert(name.clone(), v);
                        }
                        None => {
                            exec.variables.remove(&name);
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
            exec.variables
                .get("TRY_BLOCK_ERROR")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0)
        });
        fusevm::Value::Status(try_status)
    });

    vm.register_builtin(BUILTIN_UNKNOWN_COND, |vm, _argc| {
        // Unused — the diagnostic is emitted at compile time
        // (BUILTIN dispatch wasn't reliably firing for this path).
        // Kept registered as a no-op placeholder.
        let _ = vm.pop();
        fusevm::Value::Bool(false)
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
            exec.variables.insert("LINENO".to_string(), n.to_string());
        });
        fusevm::Value::Status(0)
    });

    // Direct port of Src/prompt.c:1623 cmdpush. Token is a
    // `crate::prompt::CmdState as u8` — emitted by compile_zsh
    // around each compound command (if/while/[[…]]/((…))/$(…))
    // and consumed by `%_` in PS4 / prompt expansion.
    vm.register_builtin(BUILTIN_CMD_PUSH, |vm, _argc| {
        let token = vm.pop().to_int() as u8;
        with_executor(|exec| {
            if let Some(state) = crate::prompt::CmdState::from_u8(token) {
                exec.cmd_stack.push(state);
            }
        });
        fusevm::Value::Status(0)
    });

    // Direct port of Src/prompt.c:1631 cmdpop.
    vm.register_builtin(BUILTIN_CMD_POP, |_vm, _argc| {
        with_executor(|exec| {
            exec.cmd_stack.pop();
        });
        fusevm::Value::Status(0)
    });

    vm.register_builtin(BUILTIN_OPTION_SET, |vm, _argc| {
        let name = vm.pop().to_str();
        // zsh strips a leading `no` (e.g. `[[ -o nounset ]]` and
        // `[[ -o nonounset ]]` both query the `nounset` option, with
        // the latter inverted). Strip any underscores/hyphens too —
        // user-typed names like `extended_glob` should match the
        // canonical `extendedglob`.
        let normalized = name.to_lowercase().replace(['_', '-'], "");
        let (canonical, invert) = if let Some(stripped) = normalized.strip_prefix("no") {
            if ZSH_OPTIONS_SET.contains(stripped) {
                (stripped.to_string(), true)
            } else {
                (normalized.clone(), false)
            }
        } else {
            (normalized.clone(), false)
        };
        // Unknown option: zsh emits "no such option: NAME" to stderr
        // (and the test result is false). Match the diagnostic so
        // scripts probing `[[ -o opt ]]` for unknowns get the same
        // signal in stderr.
        if !ZSH_OPTIONS_SET.contains(canonical.as_str()) {
            eprintln!("zshrs:1: no such option: {}", name);
            return fusevm::Value::Bool(false);
        }
        let is_set = with_executor(|exec| exec.options.get(&canonical).copied().unwrap_or(false));
        fusevm::Value::Bool(if invert { !is_set } else { is_set })
    });

    vm.register_builtin(BUILTIN_PARAM_FILTER, |vm, _argc| {
        let pattern_raw = vm.pop().to_str();
        let name = vm.pop().to_str();
        // Expand `$VAR` / `${VAR}` / `$(cmd)` / `$((expr))` references in
        // the pattern before matching. Direct port of Src/subst.c:3192
        // case '#' arm which calls singsub on the operand. zinit's
        // `${(@)region_highlight:#$_LAST_HIGHLIGHT}` and similar idioms
        // rely on the pattern being expanded first.
        let pattern = if pattern_raw.contains('$') || pattern_raw.contains('`') {
            with_executor(|exec| exec.singsub(&pattern_raw))
        } else {
            pattern_raw
        };
        let arr_val = with_executor(|exec| exec.arrays.get(&name).cloned());
        // Inline of the deleted extendedglob_match helper (Src/glob.c
        // pattern_match path): leading `^` inverts when extendedglob is
        // set; otherwise falls through to glob_match_static. Plain
        // literal-equal path retained for the no-meta-char case
        // (cheaper than running a regex compile on every element).
        let matches_glob = |s: &str, pat: &str| -> bool {
            let starts_neg = pat.starts_with('^');
            if pat.contains('*') || pat.contains('?') || pat.contains('[') || starts_neg {
                let extendedglob = with_executor(|exec| {
                    exec.options.get("extendedglob").copied().unwrap_or(false)
                });
                if extendedglob {
                    if let Some(neg) = pat.strip_prefix('^') {
                        return !crate::ported::exec::ShellExecutor::glob_match_static(s, neg);
                    }
                }
                crate::ported::exec::ShellExecutor::glob_match_static(s, pat)
            } else {
                s == pat
            }
        };
        // (M) flag inverts the filter: keep matching elements, drop
        // non-matching (vs default which drops matches). Direct port
        // of subst.c's SUB_MATCH bit which getmatch consults to
        // pick the "matched" disposition over the "rest" default.
        let invert = with_executor(|exec| {
            let inv = (exec.sub_flags & 0x0008) != 0;       // c:2171 SUB_MATCH
            exec.sub_flags = 0;                              // c:2169 (consume)
            inv
        });
        if let Some(arr) = arr_val {
            let kept: Vec<fusevm::Value> = arr
                .into_iter()
                .filter(|elem| {                             // c:2171
                    let m = matches_glob(elem, &pattern);   // c:2171
                    if invert { m } else { !m }              // c:2171
                })
                .map(fusevm::Value::str)
                .collect();
            return fusevm::Value::Array(kept);
        }
        let val = with_executor(|exec| exec.get_variable(&name));
        let m = matches_glob(&val, &pattern);
        if invert {                                          // c:2171
            if m { fusevm::Value::str(val) } else { fusevm::Value::str(String::new()) } // c:2171
        } else if m {
            fusevm::Value::str(String::new())
        } else {
            fusevm::Value::str(val)
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
            let arr = exec.arrays.entry(name.clone()).or_insert_with(Vec::new);
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
                exec.variables.remove(&name);
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
            exec.variables.remove(&name);
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
            exec.options.get("rcexpandparam").copied().unwrap_or(false)
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
        use std::os::unix::fs::MetadataExt;
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
        use std::os::unix::fs::FileTypeExt;
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.file_type().is_char_device())
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    // `[[ -b path ]]` — block device.
    vm.register_builtin(BUILTIN_IS_BLOCKDEV, |vm, _argc| {
        use std::os::unix::fs::FileTypeExt;
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.file_type().is_block_device())
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    // `[[ -p path ]]` — FIFO (named pipe).
    vm.register_builtin(BUILTIN_IS_FIFO, |vm, _argc| {
        use std::os::unix::fs::FileTypeExt;
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.file_type().is_fifo())
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    // `[[ -S path ]]` — socket.
    vm.register_builtin(BUILTIN_IS_SOCKET, |vm, _argc| {
        use std::os::unix::fs::FileTypeExt;
        let path = vm.pop().to_str();
        let result = std::fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_socket())
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });

    // `[[ -k path ]]` / `-u` / `-g` — sticky / setuid / setgid bit.
    vm.register_builtin(BUILTIN_HAS_STICKY, |vm, _argc| {
        use std::os::unix::fs::PermissionsExt;
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.permissions().mode() & libc::S_ISVTX as u32 != 0)
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    vm.register_builtin(BUILTIN_HAS_SETUID, |vm, _argc| {
        use std::os::unix::fs::PermissionsExt;
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.permissions().mode() & libc::S_ISUID as u32 != 0)
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    vm.register_builtin(BUILTIN_HAS_SETGID, |vm, _argc| {
        use std::os::unix::fs::PermissionsExt;
        let path = vm.pop().to_str();
        let result = std::fs::metadata(&path)
            .map(|m| m.permissions().mode() & libc::S_ISGID as u32 != 0)
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    vm.register_builtin(BUILTIN_OWNED_BY_USER, |vm, _argc| {
        use std::os::unix::fs::MetadataExt;
        let path = vm.pop().to_str();
        let euid = unsafe { libc::geteuid() };
        let result = std::fs::metadata(&path)
            .map(|m| m.uid() == euid)
            .unwrap_or(false);
        fusevm::Value::Bool(result)
    });
    vm.register_builtin(BUILTIN_OWNED_BY_GROUP, |vm, _argc| {
        use std::os::unix::fs::MetadataExt;
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
        use std::os::unix::fs::MetadataExt;
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
        // command, including across function boundaries. zshrs's
        // function-entry path reads exec.last_status to seed the
        // child VM's `$?`, but exec.last_status was only updated at
        // top-level script boundaries, leaking 0 into every nested
        // CallFunction. XTRACE_LINE is emitted by the compiler
        // BEFORE every simple command (right after the previous
        // command's SetStatus), so it's the natural sync point.
        let live = vm.last_status;
        with_executor(|exec| {
            exec.last_status = live;
        });
        let on = with_executor(|exec| exec.options.get("xtrace").copied().unwrap_or(false));
        if on {
            // Mirrors Src/exec.c xtrace emission: printprompt4() writes
            // the PS4 prefix to xtrerr (no newline), then the caller
            // emits the line text + newline. Without the `on` guard,
            // every command still printed its text to stderr — zsh does
            // not (BUILTIN_XTRACE_ARGS already gated the same way).
            printprompt4();
            eprintln!("{}", cmd_text);
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
            exec.last_status = live;
        });
        let on = with_executor(|exec| exec.options.get("xtrace").copied().unwrap_or(false));
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
            // XTRACE_ASSIGN this line already emitted PS4, skip it.
            // Then reset the flag after the trailing newline so the
            // next command starts fresh.
            let already_ps4 = XTRACE_DONE_PS4.with(|f| f.get());
            if !already_ps4 {
                printprompt4();
            }
            eprintln!("{}", line);
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
        let on = with_executor(|exec| exec.options.get("xtrace").copied().unwrap_or(false));
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
        let on = with_executor(|exec| exec.options.get("xtrace").copied().unwrap_or(false));
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
                let saved = exec.last_status;
                exec.last_status = 0;
                let _ = exec.execute_script(&body);
                exec.last_status = saved;
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
            let on = exec.options.get("errexit").copied().unwrap_or(false);
            on && exec.local_scope_depth == 0 && exec.subshell_snapshots.is_empty()
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
    vm.register_builtin(BUILTIN_PARAM_DEFAULT_FAMILY, |vm, _argc| {
        let rhs = vm.pop().to_str();
        let op = vm.pop().to_int() as u8;
        let name = vm.pop().to_str();
        // Op codes:
        //   0 :-  1 :=  2 :?  3 :+   (treat-empty-as-unset variants)
        //   4 -   5 =   6 ?   7 +    (no-colon: only fire if truly unset)
        // The default/alt modifiers handle missing-var themselves, so
        // suppress the nounset (set -u) abort during the value lookup —
        // otherwise `${unset:-fb}` exits the shell instead of returning
        // "fb". Save/restore nounset around the lookup.
        let val = with_executor(|exec| {
            let saved_nounset = exec.options.get("nounset").copied();
            let saved_unset = exec.options.get("unset").copied();
            exec.options.insert("nounset".to_string(), false);
            exec.options.insert("unset".to_string(), true);
            let v = exec.get_variable(&name);
            match saved_nounset {
                Some(b) => {
                    exec.options.insert("nounset".to_string(), b);
                }
                None => {
                    exec.options.remove("nounset");
                }
            }
            match saved_unset {
                Some(b) => {
                    exec.options.insert("unset".to_string(), b);
                }
                None => {
                    exec.options.remove("unset");
                }
            }
            v
        });
        let is_set = with_executor(|exec| {
            // Positional params ($1, $2, ...): set iff index <= $#.
            if name.chars().all(|c| c.is_ascii_digit()) && !name.is_empty() {
                if let Ok(idx) = name.parse::<usize>() {
                    if idx == 0 {
                        return true; // $0 always set
                    }
                    return idx <= exec.positional_params.len();
                }
            }
            // zsh-special "always set" params: their getter computes
            // a dynamic value, but the contains_key check fails. Treat
            // them as set so `${SECONDS-default}` returns the seconds,
            // not "default".
            let is_zsh_special = matches!(
                name.as_str(),
                "SECONDS"
                    | "EPOCHSECONDS"
                    | "EPOCHREALTIME"
                    | "RANDOM"
                    | "LINENO"
                    | "HISTCMD"
                    | "PPID"
                    | "UID"
                    | "EUID"
                    | "GID"
                    | "EGID"
                    | "SHLVL"
            );
            exec.variables.contains_key(&name)
                || exec.arrays.contains_key(&name)
                || exec.assoc_arrays.contains_key(&name)
                || std::env::var(&name).is_ok()
                || is_zsh_special
        });
        let is_empty = val.is_empty();
        // For colon variants, "missing" = unset OR empty.
        // For no-colon variants, "missing" = unset only.
        let missing = match op {
            0..=3 => is_empty,
            _ => !is_set,
        };
        // Empty-unquoted-elide for default-family results. When the
        // resulting expansion is empty AND we're unquoted, drop the
        // arg. Direct port of zsh's elide-empty-words pass which
        // applies to ALL paramsubst results, including default-family.
        let in_dq = with_executor(|exec| exec.in_dq_context > 0);
        let maybe_elide = |s: String| -> fusevm::Value {
            if s.is_empty() && !in_dq {
                fusevm::Value::Array(Vec::new())
            } else {
                fusevm::Value::str(s)
            }
        };
        // The default/alt operand may contain `$var` / `$(cmd)` /
        // `$((expr))` — zsh expands these before substitution. Apply
        // expand_string lazily (only when we'll actually use rhs).
        let expand_rhs = |s: &str| -> String { with_executor(|exec| exec.singsub(s)) };
        match op {
            0 | 4 => {
                // `:-` / `-` use default if missing
                if missing {
                    maybe_elide(expand_rhs(&rhs))
                } else {
                    maybe_elide(val)
                }
            }
            1 | 5 => {
                // `:=` / `=` assign default if missing, then use it
                if missing {
                    let expanded = expand_rhs(&rhs);
                    with_executor(|exec| {
                        exec.variables.insert(name, expanded.clone());
                    });
                    maybe_elide(expanded)
                } else {
                    maybe_elide(val)
                }
            }
            2 | 6 => {
                // `:?` / `?` error if missing — zsh in -c mode prints
                // `zsh:LINE: NAME: msg` and exits 1. Mirror that: emit
                // diagnostic on stderr and abort the shell.
                if missing {
                    let expanded = expand_rhs(&rhs);
                    let msg = if expanded.is_empty() {
                        "parameter not set".to_string()
                    } else {
                        expanded
                    };
                    eprintln!("zshrs:1: {}: {}", name, msg);
                    std::process::exit(1);
                } else {
                    fusevm::Value::str(val)
                }
            }
            3 | 7 => {
                // `:+` / `+` use alt if NOT missing (set-and-non-empty
                // for colon variant; just set for no-colon variant).
                if missing {
                    maybe_elide(String::new())
                } else {
                    maybe_elide(expand_rhs(&rhs))
                }
            }
            8 => {
                // `${+name}` set-test — emits "1" if name is set,
                // "0" if unset. Direct port of subst.c case '+' at
                // the leading-flag position (different from `${name+rhs}`).
                // is_set was computed above and includes positional
                // params, zsh-special vars, regular vars, arrays,
                // assocs. Subscripted form `${+arr[i]}` checks if
                // that specific element is set — get_variable doesn't
                // parse subscripts, so resolve the lookup by hand:
                // numeric N → arr[N-1] is set iff N <= len; (r)PAT /
                // (R)PAT / KEY → resolve via the same subscript
                // engine as plain `${arr[i]}`.
                if let Some(lb) = name.find('[') {
                    if name.ends_with(']') {
                        let arr_name = &name[..lb];
                        let key = &name[lb + 1..name.len() - 1];
                        let direct_set = with_executor(|exec| {
                            // Numeric index: 1-based, must be in range.
                            if let Ok(n) = key.parse::<i64>() {
                                let len = exec
                                    .arrays
                                    .get(arr_name)
                                    .map(|a| a.len() as i64)
                                    .unwrap_or(0);
                                if n > 0 && n <= len {
                                    return Some(true);
                                }
                                if n < 0 {
                                    let resolved = len + n;
                                    return Some(resolved >= 0);
                                }
                                return Some(false);
                            }
                            if let Some(map) = exec.assoc_arrays.get(arr_name) {
                                return Some(map.contains_key(key));
                            }
                            if let Some(arr) = exec.arrays.get(arr_name) {
                                let pat = if let Some(p) = key
                                    .strip_prefix("(r)")
                                    .or_else(|| key.strip_prefix("(R)"))
                                {
                                    p
                                } else {
                                    key
                                };
                                return Some(arr.iter().any(|el| {
                                    crate::exec::ShellExecutor::glob_match_static(el, pat)
                                }));
                            }
                            None
                        });
                        // Magic-assoc fallback (commands, aliases,
                        // functions, options, etc.) — `${+commands[ls]}`
                        // walks PATH to answer "is ls a command". Direct
                        // port of zsh's getindex routing through the
                        // special-parameter getfn (Src/params.c
                        // SPECIAL_PARAMS) when the named assoc isn't
                        // user-declared. Re-uses the same magic_assoc_lookup
                        // dispatcher BUILTIN_ARRAY_INDEX consults; called
                        // outside the with_executor closure so the lookup
                        // itself can re-enter the executor lock safely.
                        let element_set = direct_set.unwrap_or_else(|| {
                            magic_assoc_lookup(arr_name, key)
                                .map(|v| !v.to_str().is_empty())
                                .unwrap_or(false)
                        });
                        return fusevm::Value::str(if element_set { "1" } else { "0" });
                    }
                    fusevm::Value::str(if !val.is_empty() { "1" } else { "0" })
                } else {
                    fusevm::Value::str(if is_set { "1" } else { "0" })
                }
            }
            _ => fusevm::Value::str(val),
        }
    });

    // `${var:offset[:length]}` — substring. Pops [name, offset, length].
    // length == -1 means "rest of string". Negative offset counts from end.
    vm.register_builtin(BUILTIN_PARAM_SUBSTRING, |vm, _argc| {
        let length = vm.pop().to_int();
        let offset = vm.pop().to_int();
        let name = vm.pop().to_str();
        // `${@:offset:length}` / `${*:offset:length}` — slice
        // positional parameters as ARRAY elements (not chars). zsh's
        // semantics: 1-based, inclusive offset; length counts elems.
        // For arrays/assoc-values arrays, same array semantics.
        // `[@]`/`[*]` suffix preserved by the compile path indicates
        // the user wrote `${arr[@]:n}` and expects splice; return
        // Value::Array so downstream array-init keeps element
        // boundaries.
        let (lookup_name, force_array) = if let Some(stripped) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
        {
            (stripped.to_string(), true)
        } else {
            (name.clone(), false)
        };
        if lookup_name == "@" || lookup_name == "*" {
            let result = with_executor(|exec| slice_positionals(exec, offset, length));
            return fusevm::Value::Array(result.into_iter().map(fusevm::Value::str).collect());
        }
        let array_slice = with_executor(|exec| exec.arrays.get(&lookup_name).cloned());
        if let Some(arr) = array_slice {
            let result = slice_array_zero_based(&arr, offset, length);
            return if force_array {
                fusevm::Value::Array(result.into_iter().map(fusevm::Value::str).collect())
            } else {
                fusevm::Value::str(result.join(" "))
            };
        }
        let name = lookup_name;
        let val = with_executor(|exec| exec.get_variable(&name));
        let chars: Vec<char> = val.chars().collect();
        let len = chars.len() as i64;
        let start = if offset < 0 {
            (len + offset).max(0) as usize
        } else {
            (offset as usize).min(chars.len())
        };
        // length sentinels:
        //   i64::MIN → no length given, take rest of string
        //   negative → "stop N chars before end" (bash/zsh)
        //   positive → take exactly N chars
        let take = if length == i64::MIN {
            chars.len().saturating_sub(start)
        } else if length < 0 {
            // Stop |length| chars before end.
            let end = (len + length).max(start as i64) as usize;
            end.saturating_sub(start)
        } else {
            (length as usize).min(chars.len().saturating_sub(start))
        };
        let result: String = chars.iter().skip(start).take(take).collect();
        fusevm::Value::str(result)
    });

    // `${var:offset[:length]}` with arith/var-based offset/length —
    // the literal-int variant above can't represent `${s:$n:2}`.
    // Stack layout (top→bottom): has_length, length_expr, offset_expr,
    // name. has_length distinguishes "no length given" from
    // "length=0".
    vm.register_builtin(BUILTIN_PARAM_SUBSTRING_EXPR, |vm, _argc| {
        let has_len = vm.pop().to_int() != 0;
        let len_expr = vm.pop().to_str();
        let off_expr = vm.pop().to_str();
        let name = vm.pop().to_str();
        // Match BUILTIN_PARAM_SUBSTRING's array-aware dispatch:
        // `${@:n:m}` / `${arr[@]:n:m}` slice positionals/array
        // ELEMENTS, not chars. Without this, the expr-form fell
        // back to scalar char-slicing on the IFS-joined value.
        let (lookup_name, force_array) = if let Some(stripped) = name
            .strip_suffix("[@]")
            .or_else(|| name.strip_suffix("[*]"))
        {
            (stripped.to_string(), true)
        } else {
            (name.clone(), false)
        };
        // Use a dual-result: Array when force_array, Str otherwise.
        // zsh: `${a[@]:1}` keeps array splice for downstream array
        // assignment (`b=("${a[@]:1}")` should give 2 elements, not
        // a single space-joined string).
        enum Result {
            Str(String),
            Arr(Vec<String>),
        }
        let result = with_executor(|exec| {
            let offset = exec.eval_arith_expr(&off_expr);
            let length_opt: Option<i64> = if has_len {
                Some(exec.eval_arith_expr(&len_expr))
            } else {
                None
            };
            // Positional-param slice (`${@:1:2}`).
            if lookup_name == "@" || lookup_name == "*" {
                let parts = slice_positionals(exec, offset, length_opt.unwrap_or(i64::MIN));
                return Result::Arr(parts);
            }
            // Array slice (`${arr:1:2}` or `${arr[@]:1:2}`).
            if let Some(arr) = exec.arrays.get(&lookup_name).cloned() {
                let sliced = slice_array_zero_based(&arr, offset, length_opt.unwrap_or(i64::MIN));
                return if force_array {
                    Result::Arr(sliced)
                } else {
                    Result::Str(sliced.join(" "))
                };
            }
            // Scalar fallback.
            let val = exec.get_variable(&lookup_name);
            let chars: Vec<char> = val.chars().collect();
            let len = chars.len() as i64;
            let start = if offset < 0 {
                (len + offset).max(0) as usize
            } else {
                (offset as usize).min(chars.len())
            };
            let take = match length_opt {
                None => chars.len().saturating_sub(start),
                Some(length) if length < 0 => chars.len().saturating_sub(start),
                Some(length) => (length as usize).min(chars.len().saturating_sub(start)),
            };
            Result::Str(chars.iter().skip(start).take(take).collect::<String>())
        });
        match result {
            Result::Str(s) => fusevm::Value::str(s),
            Result::Arr(parts) => {
                fusevm::Value::Array(parts.into_iter().map(fusevm::Value::str).collect())
            }
        }
    });

    // `${var#pat}` / `${var##pat}` / `${var%pat}` / `${var%%pat}`
    // Pops [name, pattern, op_byte]. op: 0=`#` short-prefix, 1=`##` long,
    // 2=`%` short-suffix, 3=`%%` long. Glob-pattern matching via the
    // existing glob_match_static helper.
    vm.register_builtin(BUILTIN_PARAM_STRIP, |vm, _argc| {
        // The compiler now passes `dq_flag` as a 4th arg so the
        // runtime can distinguish DQ-wrapped (join-then-strip)
        // from unquoted (per-element) on array-valued names.
        // Mirrors zsh's pattern.c split between `getmatch` (joined
        // scalar) and `getmatcharr` (per-element).
        let dq_flag = vm.pop().to_int() != 0;
        let op = vm.pop().to_int() as u8;
        let pattern_raw = vm.pop().to_str();
        let name = vm.pop().to_str();
        // SUB_M / SUB_S flags. M = return matched portion (vs strip
        // result). S = search anywhere instead of anchored to start
        // (#/##) or end (%/%%). Direct port of subst.c:2171/2186
        // SUB_MATCH / SUB_SUBSTR bits + getmatch dispatch.
        let (sub_match, sub_substr) = with_executor(|exec| {
            let m = (exec.sub_flags & 0x0008) != 0;
            let s = (exec.sub_flags & 0x0004) != 0;
            exec.sub_flags = 0;
            (m, s)
        });
        // Pattern may contain `$var` / `$(cmd)` / `$((expr))` — zsh
        // expands these before applying the strip. Was emitted as-is.
        let pattern = with_executor(|exec| exec.singsub(&pattern_raw));
        // Delegate to the shared `strip_match_op` helper (also used
        // by the flag-aware `expand_braced_variable` path so M-flag
        // inversion works consistently). The compile-time fast path
        // never carries (M) since `parse_param_modifier` rejects
        // flag forms and routes them through the bridge — so always
        // pass `m_flag=false` here.
        // strip_match_op port — direct inline of subst.c:3540's
        // SUB_MATCH dispatch on the # / ## / % / %% pattern strip
        // ops. Op codes per ParamModifierKind::Strip:
        //   0 = `#`  shortest prefix
        //   1 = `##` longest prefix
        //   2 = `%`  shortest suffix
        //   3 = `%%` longest suffix
        // Pattern matching is currently glob-via-fnmatch from
        // crate::ported::glob::glob_match_static (handles ?, *, [...]).
        let strip_one = |v: &str, op: u8, pattern: &str| -> String {
            let chars: Vec<char> = v.chars().collect();
            let n = chars.len();
            // (S) substring search: instead of anchoring to start
            // (#/##) or end (%/%%), find the shortest/longest match
            // ANYWHERE in v, and either return it (sub_match) or
            // remove it (default — keep parts before+after the match).
            // Direct port of subst.c:2186 SUB_SUBSTR bit which
            // getmatch routes through pat_substr_match.
            if sub_substr {                                  // c:2186
                let longest = matches!(op, 1 | 3);          // c:2186 (## / %% want longest)
                let mut best: Option<(usize, usize)> = None; // c:2186 (start, end in chars)
                // Slide a window across v; for each start index
                // try every (longest|shortest) length that matches.
                for start in 0..=n {                        // c:2186
                    let end_iter: Box<dyn Iterator<Item = usize>> = if longest { // c:2186
                        Box::new((start..=n).rev())          // c:2186
                    } else {                                 // c:2186
                        Box::new(start..=n)                  // c:2186
                    };                                       // c:2186
                    for end in end_iter {                    // c:2186
                        let sub: String = chars[start..end].iter().collect(); // c:2186
                        if ShellExecutor::glob_match_static(&sub, pattern) { // c:2186
                            // (S) prefers the leftmost match
                            // for # / ##, and the rightmost for
                            // % / %%. # / ## scan left-to-right;
                            // % / %% mirror by walking start
                            // backward at the outer level — but
                            // since the outer loop is L-to-R, we
                            // record EVERY match and pick the
                            // last one for %/%%, first for #/##.
                            let suffix_op = matches!(op, 2 | 3); // c:2186
                            if best.is_none() || suffix_op {  // c:2186
                                best = Some((start, end));   // c:2186
                            }                                 // c:2186
                            if !suffix_op { break; }          // c:2186 (#/## stop at first)
                        }                                    // c:2186
                    }                                        // c:2186
                    if best.is_some() && !matches!(op, 2 | 3) { break; } // c:2186
                }                                            // c:2186
                if let Some((s, e)) = best {                 // c:2186
                    let matched: String = chars[s..e].iter().collect(); // c:2186
                    if sub_match {                           // c:2171
                        return matched;                      // c:2171
                    }                                        // c:2171
                    let mut out = String::new();             // c:2186
                    out.extend(chars[..s].iter());           // c:2186
                    out.extend(chars[e..].iter());           // c:2186
                    return out;                              // c:2186
                }                                            // c:2186
                return if sub_match { String::new() } else { v.to_string() }; // c:2186
            }                                                // c:2186
            // (M) inverted-disposition helper: when sub_match is set,
            // return the MATCHED portion instead of the post-strip
            // string. Used by zsh idioms like \${(M)path#*/} which
            // returns the leading "/segment" rather than the rest.
            // Direct port of getmatch's SUB_MATCH branch — it picks
            // the matched-portion view from the same scan.
            match op {
                0 => {
                    // shortest prefix strip — try k = 0, 1, ...
                    for k in 0..=n {
                        let prefix: String = chars[..k].iter().collect();
                        if ShellExecutor::glob_match_static(&prefix, pattern) {
                            return if sub_match {            // c:2171
                                prefix                       // c:2171
                            } else {                         // c:2171
                                chars[k..].iter().collect()
                            };
                        }
                    }
                    if sub_match { String::new() } else { v.to_string() } // c:2171
                }
                1 => {
                    // longest prefix strip — try k = n down to 0
                    for k in (0..=n).rev() {
                        let prefix: String = chars[..k].iter().collect();
                        if ShellExecutor::glob_match_static(&prefix, pattern) {
                            return if sub_match {            // c:2171
                                prefix                       // c:2171
                            } else {                         // c:2171
                                chars[k..].iter().collect()
                            };
                        }
                    }
                    if sub_match { String::new() } else { v.to_string() } // c:2171
                }
                2 => {
                    // shortest suffix strip
                    for k in 0..=n {
                        let suffix: String = chars[n - k..].iter().collect();
                        if ShellExecutor::glob_match_static(&suffix, pattern) {
                            return if sub_match {            // c:2171
                                suffix                       // c:2171
                            } else {                         // c:2171
                                chars[..n - k].iter().collect()
                            };
                        }
                    }
                    if sub_match { String::new() } else { v.to_string() } // c:2171
                }
                3 => {
                    // longest suffix strip
                    for k in (0..=n).rev() {
                        let suffix: String = chars[n - k..].iter().collect();
                        if ShellExecutor::glob_match_static(&suffix, pattern) {
                            return if sub_match {            // c:2171
                                suffix                       // c:2171
                            } else {                         // c:2171
                                chars[..n - k].iter().collect()
                            };
                        }
                    }
                    if sub_match { String::new() } else { v.to_string() } // c:2171
                }
                _ => v.to_string(),
            }
        };
        // `${arr#pat}` / `${arr%pat}` / etc. on an array:
        //   - Unquoted form: iterate per element, preserve array
        //     shape so `print -l` emits one line per element. Direct
        //     port of Src/subst.c:3422-3433 `if (!vunset && isarr)`
        //     branch which calls `getmatcharr(&aval, …)` — modifies
        //     each element of the array in-place, leaves isarr=1.
        //   - DQ-wrapped form (`"${arr%pat}"`): zsh joins as scalar
        //     first then strips. So `(/tmp/foo /etc/bar)` with `%/*`
        //     gives `/tmp/foo /etc` (last `/bar` stripped from
        //     joined), not `/tmp /etc` (per-element).
        enum StripResult {
            Scalar(String),
            Array(Vec<String>),
        }
        let result: StripResult = with_executor(|exec| {
            let in_dq = dq_flag || exec.in_dq_context > 0;
            if name == "@" || name == "*" {
                if in_dq {
                    let joined = exec.positional_params.join(" ");
                    return StripResult::Scalar(strip_one(&joined, op, &pattern));
                }
                let stripped: Vec<String> = exec
                    .positional_params
                    .iter()
                    .map(|e| strip_one(e, op, &pattern))
                    .collect();
                return StripResult::Array(stripped);
            }
            if let Some(arr) = exec.arrays.get(&name) {
                if in_dq {
                    let joined = arr.join(" ");
                    return StripResult::Scalar(strip_one(&joined, op, &pattern));
                }
                let stripped: Vec<String> = arr
                    .iter()
                    .map(|e| strip_one(e, op, &pattern))
                    .collect();
                return StripResult::Array(stripped);
            }
            let val = exec.get_variable(&name);
            StripResult::Scalar(strip_one(&val, op, &pattern))
        });
        match result {
            StripResult::Scalar(s) => fusevm::Value::str(s),
            StripResult::Array(arr) => {
                let mapped: Vec<fusevm::Value> = arr.into_iter().map(fusevm::Value::str).collect();
                fusevm::Value::Array(mapped)
            }
        }
    });

    // `$((expr))` — pops [expr_string], evaluates via MathEval which
    // honors integer-vs-float distinction (zsh-compatible). Returns
    // the result as Value::Str so it can be Concat'd into surrounding
    // word context.
    vm.register_builtin(BUILTIN_ARITH_EVAL, |vm, _argc| {
        let expr = vm.pop().to_str();
        let result = with_executor(|exec| exec.evaluate_arithmetic(&expr));
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
            exec.last_status = live_status;
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
        let cs_status = with_executor(|exec| exec.last_status);
        vm.last_status = cs_status;
        fusevm::Value::str(result)
    });

    // Text-based word expansion. Pops [preserved_text, mode_byte].
    // mode_byte:
    //   0 = Default — expand_string + xpandbraces + expand_glob
    //   1 = DoubleQuoted — strip outer `"…"`, expand_string only
    //         (no brace, no glob — DQ semantics)
    //   2 = SingleQuoted — strip outer `'…'`, no expansion
    //         (kept for symmetry; SNULL early-return covers most SQ)
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
                let out = exec.singsub(&prepped);
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
                exec.last_status = live_status;
                let captured = exec.run_command_substitution(inner);
                let trimmed = captured.trim_end_matches('\n');
                if exec.in_dq_context > 0 {
                    fusevm::Value::str(trimmed.to_string())
                } else {
                    let ifs = exec
                        .variables
                        .get("IFS")
                        .cloned()
                        .unwrap_or_else(|| " \t\n".to_string());
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
                fusevm::Value::str(exec.singsub(&text))
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
                let expanded = exec.singsub(&prepped);
                let brace_expanded = vec![expanded.to_string()];
                // zsh stores the option as `glob` (default ON);
                // `setopt noglob` writes `glob=false`. Honor either
                // form so the dispatcher behaves the same as zsh.
                let noglob = exec.options.get("noglob").copied().unwrap_or(false)
                    || exec.options.get("GLOB").map(|v| !v).unwrap_or(false)
                    || !exec.options.get("glob").copied().unwrap_or(true);
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
                        let extglob_meta =
                            exec.options.get("extendedglob").copied().unwrap_or(false)
                                && (s.starts_with('^') || s.contains('~') || s.contains("/^"));
                        let has_numeric_range = s.contains('<')
                            && s.contains('>')
                            && !crate::ported::pattern::NumericRange::extract_all(&s).is_empty();
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
    vm.register_builtin(BUILTIN_PARAM_LENGTH, |vm, _argc| {
        let name_raw = vm.pop().to_str();
        // Strip `[@]` / `[*]` subscript suffix — `${#arr[@]}` and
        // `${#m[@]}` are element-count forms, same as `${#arr}` /
        // `${#m}`. Fast paths sometimes hand us the bare name and
        // sometimes leave the subscript attached.
        let name = name_raw
            .strip_suffix("[@]")
            .or_else(|| name_raw.strip_suffix("[*]"))
            .unwrap_or(&name_raw)
            .to_string();
        // `${#arr[N]}` — length of the Nth ELEMENT, not the array
        // count. Verified empirically: arr=(aa bb ccc); ${#arr[2]} → 2
        // in real zsh. Resolve the bare name + bracketed subscript
        // (with embedded `$VAR` references expanded) to a single
        // value, then count its chars. Skip `[@]` / `[*]` — those
        // were stripped above as splice forms.
        if let Some(open) = name.find('[') {
            if name.ends_with(']') && &name[open..] != "[@]" && &name[open..] != "[*]" {
                let bare = &name[..open];
                let raw_idx = &name[open + 1..name.len() - 1];
                let elem = with_executor(|exec| {
                    // Expand `$VAR` / `${VAR}` references inside the
                    // subscript before lookup (single dollar pass).
                    let resolved_idx = expand_dollar_refs(raw_idx, exec);
                    if let Some(arr) = exec.arrays.get(bare) {
                        if let Ok(n) = resolved_idx.trim().parse::<i64>() {
                            let len = arr.len() as i64;
                            let idx = if n > 0 { n - 1 } else if n < 0 { len + n } else { -1 };
                            if idx >= 0 && (idx as usize) < arr.len() {
                                return arr[idx as usize].clone();
                            }
                        }
                        return String::new();
                    }
                    if let Some(map) = exec.assoc_arrays.get(bare) {
                        return map.get(resolved_idx.as_str()).cloned().unwrap_or_default();
                    }
                    String::new()
                });
                return fusevm::Value::str(elem.chars().count().to_string());
            }
        }
        let count = with_executor(|exec| {
            // ${#@} / ${#*} → count of positional params (= $#).
            // Without this, `@`/`*` fell through to `get_variable`
            // which returned the IFS-joined positional string and
            // we counted chars (5 for "a b c" instead of 3).
            if name == "@" || name == "*" || name == "argv" {
                return exec.positional_params.len();
            }
            // Magic-array specials whose length is data-driven, not
            // taken from `exec.arrays`/`exec.assoc_arrays`. Direct
            // ports of the relevant `SPECIALPMDEF` entries:
            //   - `errnos`     → Src/Modules/system.c:902
            //   - `commands`   → Src/Modules/parameter.c
            //   - `aliases`    → Src/Modules/parameter.c
            //   - `functions`  → Src/Modules/parameter.c
            //   - `parameters` → Src/Modules/parameter.c
            //   - `options`    → Src/Modules/parameter.c
            //   - `sysparams`  → Src/Modules/system.c:904
            match name.as_str() {
                "errnos" => return crate::modules::system::ERRNO_NAMES.len(),
                "epochtime" => return 2, // [seconds, nanoseconds]
                "commands" => return exec.command_hash.len(),
                "aliases" => return exec.aliases.len(),
                "galiases" => return exec.global_aliases.len(),
                "saliases" => return exec.suffix_aliases.len(),
                "functions" => return exec.function_names().len(),
                "options" => return exec.options.len(),
                "sysparams" => return 3, // pid, ppid, procsubstpid
                _ => {}
            }
            if let Some(arr) = exec.arrays.get(&name) {
                arr.len()
            } else if let Some(assoc) = exec.assoc_arrays.get(&name) {
                assoc.len()
            } else {
                exec.get_variable(&name).chars().count()
            }
        });
        fusevm::Value::str(count.to_string())
    });

    // `${var/pat/repl}` / `${var//pat/repl}` / `${var/#pat/repl}` /
    // `${var/%pat/repl}` — Pops [name, pattern, replacement, op_byte].
    // op: 0=first, 1=all, 2=anchor-prefix (`/#`), 3=anchor-suffix (`/%`).
    vm.register_builtin(BUILTIN_PARAM_REPLACE, |vm, _argc| {
        let dq_flag = vm.pop().to_int() != 0;
        let op = vm.pop().to_int() as u8;
        let repl_raw = vm.pop().to_str();
        let pattern_raw = vm.pop().to_str();
        let name = vm.pop().to_str();
        // SUB_* flag bits set by the (M)/(R)/(B)/(E)/(N)/(S) flag-loop
        // arms. Direct port of zsh's getmatch() flag dispatch — these
        // alter the disposition of the match result:
        //   M=0x08 — return matched portion
        //   R=0x10 — return rest after match
        //   B=0x20 — return 1-based start index
        //   E=0x40 — return 1-based end index
        //   N=0x80 — return match length
        //   S=0x04 — substring search (anywhere) instead of anchored
        // Read once and consume so subsequent paramsubst calls see
        // a clean slate — direct port of subst.c flag-loop pattern.
        let (sub_match, sub_rest, sub_bind, sub_eind, sub_len, _sub_substr) =
            with_executor(|exec| {
                let f = exec.sub_flags;
                exec.sub_flags = 0;
                (
                    (f & 0x0008) != 0,                       // c:2171 M
                    (f & 0x0010) != 0,                       // c:2174 R
                    (f & 0x0020) != 0,                       // c:2177 B
                    (f & 0x0040) != 0,                       // c:2180 E
                    (f & 0x0080) != 0,                       // c:2183 N
                    (f & 0x0004) != 0,                       // c:2186 S
                )
            });
        // Both pattern and replacement get parameter / cmd-subst /
        // arith expansion before use (zsh semantics — `${s/$pat/X}`
        // resolves $pat).
        // Untokenize before pattern compile — zsh's lexer leaves
        // SNULL/DQ markers and meta-encoded metachars in the
        // pattern stream. regex::Regex::new errors on those bytes,
        // and even when it compiles, it matches against tokenized
        // text rather than the user's literal pattern. Direct port
        // of bin_test's `untokenize(pattern)` call before patcompile.
        let pattern = with_executor(|exec| exec.singsub(&pattern_raw));
        let pattern = crate::lex::untokenize(&pattern);
        // Replacement: full singsub with skip_filesub so a literal
        // leading `~` in the replacement reaches the output as-is
        // (per zsh, `${var/#pat/~}` keeps the tilde — the
        // p10k / oh-my-zsh idiom of replacing `$HOME` with `~` for
        // display). Was using a hand-rolled `expand_no_tilde` that
        // only handled `$VAR` / `${VAR}` references, missing
        // `$(cmd)` and `$((expr))` in templates like
        // `\${var//foo/$(date +%s)}`.
        // Inline `singsub-with-skip_filesub` — C zsh sets the flag
        // inline before calling singsub rather than wrapping in a
        // helper. Direct port of the prefork SUB_FLAG | SKIP_FILESUB
        // pattern. PORT.md: no helpers without C counterpart.
        let repl = with_executor(|exec| {
            let mut state = crate::ported::subst::SubstState::from_executor(exec);
            state.skip_filesub = true;
            let r = crate::ported::subst::singsub(&repl_raw, &mut state);
            state.commit_to_executor(exec);
            r
        });
        let repl = crate::lex::untokenize(&repl);
        // Strip backslash escapes from the pattern. zsh: `\X` in a
        // ${var/pat/repl} pattern means "literal X" — the backslash
        // is removed and X is used as a literal char (regardless of
        // whether X is a pattern metachar). Without this, `${a//\:/-}`
        // tried to match the literal "\:" in $a which never matched.
        // We preserve `\\` (literal backslash) and `\X` for X in the
        // pattern-meta set, since regex compile expects those raw.
        let pattern = {
            let mut out = String::with_capacity(pattern.len());
            let mut it = pattern.chars().peekable();
            while let Some(c) = it.next() {
                if c == '\\' {
                    if let Some(&nx) = it.peek() {
                        // For non-meta chars, drop the backslash.
                        // For metas keep the escape so regex still
                        // matches them literally below.
                        // Keep escape only for actual zsh pattern
                        // metachars (the ones that have special pattern
                        // meaning). `.` is regex-meta but NOT zsh-meta,
                        // so `\.` drops the backslash → literal `.`.
                        if matches!(nx, '?' | '*' | '[' | ']' | '(' | ')' | '|' | '\\') {
                            out.push(c);
                        } else {
                            out.push(nx);
                            it.next();
                        }
                    } else {
                        out.push(c);
                    }
                } else {
                    out.push(c);
                }
            }
            out
        };
        // Inline pattern flags `(#i)` / `(#l)` / `(#I)` / `(#b)` apply
        // to ${var//pat/repl}. `(#b)` enables backref capture: each
        // `(...)` group in the pattern becomes accessible via
        // `${match[N]}` (1-based) in the replacement. Per
        // Src/pattern.c — the C source uses `pat_pure` flags +
        // `pat_subme` arrays; the Rust port plumbs through
        // `regex::Captures` and writes `state.arrays["match"]`
        // before each replacement-string expansion.
        let (pattern, case_insensitive_repl, _l_flag_repl, _approx_repl, backref_mode) =
            crate::ported::pattern::PatternFlags::parse(&pattern);
        // zsh patterns in ${var/pat/repl} support `?`, `*`, `[...]`,
        // anchored `#`/`%` (handled via op codes 2/3). Compile to a
        // regex for the actual matching; falls back to plain string
        // when the pattern has no glob metas (faster).
        // Include `(` as a glob trigger — zsh's `(...)` is a grouping
        // (with `|` for alternation). `${a/(?)/X}` should match like
        // `${a/?/X}` (paren is the group). Without `(` in the trigger
        // set, paren patterns fell into the literal-string path and
        // matched nothing.
        // `#` (and its `##` repetition pair) is an extendedglob
        // postfix metachar — `a##` = one-or-more `a`. Include it
        // in the trigger set so `${var//a##/X}` routes through the
        // regex compile path instead of the literal-string fallback.
        // Bare `#` alone is non-meta — but it's safe to over-trigger
        // here because the regex compiler escapes literals it can't
        // interpret as quantifier postfix anyway.
        let has_glob = pattern
            .chars()
            .any(|c| matches!(c, '?' | '*' | '[' | ']' | '(' | '#'));
        // backref_mode (set by `(#b)` / `(#m)` / `(#M)` flags) needs
        // per-match capture iteration so `$match[N]` / `$MATCH` /
        // `$MBEGIN` / `$MEND` resolve PER-replacement against the
        // current capture. The literal-string replace path skips
        // captures entirely, so MATCH stays empty. Force the regex
        // path when backref_mode is set even for literal patterns.
        let glob_re: Option<regex::Regex> = if has_glob || case_insensitive_repl || backref_mode {
            // Convert the glob pattern to a regex string:
            //   ? → . (any single char)
            //   * → .* (any seq)
            //   [...] → kept as-is (regex char class)
            //   ( ) → kept as regex group; | as alternation
            //   other regex metas → escaped
            let mut re = String::with_capacity(pattern.len() * 2);
            let mut chars = pattern.chars().peekable();
            // `#` / `##` extendedglob postfix detector for the
            // BUILTIN_PARAM_REPLACE pattern compile. Matches the
            // same handling in subst_port::glob_to_regex_capturing
            // and exec.rs::glob_match_static — direct port of zsh's
            // pattern.c POUND/POUND2 cases. Used by zinit's
            // main-message-formatter pattern `[^\}]##` (one-or-
            // more non-`}`).
            let consume_postfix = |chars: &mut std::iter::Peekable<std::str::Chars>| -> Option<&'static str> {
                if chars.peek() == Some(&'#') {
                    chars.next();
                    if chars.peek() == Some(&'#') {
                        chars.next();
                        Some("+")
                    } else {
                        Some("*")
                    }
                } else {
                    None
                }
            };
            while let Some(c) = chars.next() {
                match c {
                    '?' => {
                        re.push('.');
                        if let Some(q) = consume_postfix(&mut chars) { re.push_str(q); }
                    }
                    '*' => {
                        re.push_str(".*");
                        if let Some(q) = consume_postfix(&mut chars) { re.push_str(q); }
                    }
                    '[' => {
                        // Pass through to the closing ']' (already
                        // valid regex syntax for most char classes).
                        // zsh uses BOTH `[!...]` and `[^...]` for class
                        // negation; regex only accepts `^`. Translate
                        // a leading `!` after `[` to `^`. Track escape
                        // state so `[\]…]` (escaped `]` inside class)
                        // doesn't terminate the class on the FIRST `]`.
                        // Direct port of zsh's pattern.c P_BRACT_END:
                        // a backslash-quoted `]` inside a class stays
                        // literal. Used by hist-substring's
                        // `[\][()|\\*?#<>~^]` pattern.
                        re.push('[');
                        if chars.peek() == Some(&'!') {
                            chars.next();
                            re.push('^');
                        }
                        // First-char `]` is literal in zsh and regex
                        // (POSIX rule), so allow it without closing.
                        let mut first = true;
                        let mut escaped = false;
                        while let Some(cc) = chars.next() {
                            if escaped {
                                re.push(cc);
                                escaped = false;
                                first = false;
                                continue;
                            }
                            if cc == '\\' {
                                re.push(cc);
                                escaped = true;
                                continue;
                            }
                            if cc == ']' && !first {
                                re.push(cc);
                                break;
                            }
                            re.push(cc);
                            first = false;
                        }
                        if let Some(q) = consume_postfix(&mut chars) { re.push_str(q); }
                    }
                    '\\' => {
                        // `\\(#e)` / `\\(#s)` — escaped backslash
                        // followed by end/start anchor. After
                        // expand_string's `\x00\` preprocessing,
                        // this arrives as `\(#e)` (one backslash
                        // already consumed as escape-marker). Per
                        // zsh's pattern.c, `\\` in a pattern is
                        // escape-backslash (literal `\`). When that
                        // literal `\` is followed by `(#e)` /
                        // `(#s)`, emit `\\$` / `\\^`. Detected
                        // here as 5-char `\(#e)` (one `\` then
                        // `(#e)` which the (#e) arm below would
                        // otherwise treat as anchor with a literal
                        // `(` — losing the backslash). Used by
                        // zinit's `(#b)((*)\\(#e)|(*))`.
                        let mut peek = chars.clone();
                        let p1 = peek.next();
                        let p2 = peek.next();
                        let p3 = peek.next();
                        let p4 = peek.next();
                        if p1 == Some('(')
                            && p2 == Some('#')
                            && (p3 == Some('e') || p3 == Some('s'))
                            && p4 == Some(')')
                        {
                            re.push_str("\\\\");
                            chars.next(); chars.next(); chars.next(); chars.next();
                            re.push(if p3 == Some('e') { '$' } else { '^' });
                            continue;
                        }
                        re.push('\\');
                        if let Some(next) = chars.next() {
                            re.push(next);
                        }
                    }
                    // `(#e)` / `(#s)` end/start anchors — direct port
                    // of zsh's pattern.c P_EOL / P_BOL tokens. 4-char
                    // lookahead detects them; emit regex `$` / `^`.
                    // Used by zinit's
                    // `(#b)((*)\\(#e)|(*))` array-replace pattern.
                    '(' if {
                        let mut peek = chars.clone();
                        let p1 = peek.next();
                        let p2 = peek.next();
                        let p3 = peek.next();
                        p1 == Some('#')
                            && (p2 == Some('e') || p2 == Some('s'))
                            && p3 == Some(')')
                    } =>
                    {
                        chars.next(); // consume '#'
                        let kind = chars.next().unwrap(); // 'e' or 's'
                        chars.next(); // consume ')'
                        re.push(if kind == 'e' { '$' } else { '^' });
                    }
                    // `(`, `|` are zsh group/alternation operators
                    // — keep them as regex equivalents. `)` may be
                    // followed by `#`/`##` postfix applied to the
                    // closed group (e.g. `(foo|bar)##` = one-or-more
                    // of foo/bar).
                    '(' | '|' => re.push(c),
                    ')' => {
                        re.push(c);
                        if let Some(q) = consume_postfix(&mut chars) { re.push_str(q); }
                    }
                    // Regex meta chars that are NOT glob metas — escape
                    // so the regex compiler treats them literally.
                    '.' | '+' | '^' | '$' | '{' | '}' => {
                        re.push('\\');
                        re.push(c);
                    }
                    _ => {
                        re.push(c);
                        if let Some(q) = consume_postfix(&mut chars) { re.push_str(q); }
                    }
                }
            }
            // Apply `(#i)` case-insensitive flag if it was present
            // in the original pattern. Same `(?i)` prefix as
            // glob_match_static uses.
            let final_re = if case_insensitive_repl {
                format!("(?i){}", re)
            } else {
                re
            };
            regex::Regex::new(&final_re).ok()
        } else {
            None
        };
        let one = |val: String| -> String {
            // SUB_M/R/B/E/N short-circuit — alter the disposition
            // before doing the actual replacement. Direct port of
            // zsh's getmatch() which returns one of these views
            // instead of the substituted string when the bit is set.
            // Matched-portion / rest / position / length variants
            // all skip the replacement template entirely.
            let any_disposition = sub_match || sub_rest || sub_bind || sub_eind || sub_len;
            if any_disposition {
                if let Some(ref rx) = glob_re {
                    if let Some(m) = rx.find(&val) {
                        if sub_match { return m.as_str().to_string(); }
                        if sub_rest  { return val[m.end()..].to_string(); }
                        if sub_bind  { return (m.start() + 1).to_string(); }
                        if sub_eind  { return m.end().to_string(); }
                        if sub_len   { return (m.end() - m.start()).to_string(); }
                    } else {
                        // No match: M/R return empty, B/E/N return 0.
                        if sub_match || sub_rest { return String::new(); }
                        return "0".to_string();
                    }
                } else if let Some(pos) = val.find(pattern.as_str()) {
                    let end = pos + pattern.len();
                    if sub_match { return pattern.clone(); }
                    if sub_rest  { return val[end..].to_string(); }
                    if sub_bind  { return (pos + 1).to_string(); }
                    if sub_eind  { return end.to_string(); }
                    if sub_len   { return pattern.len().to_string(); }
                } else {
                    if sub_match || sub_rest { return String::new(); }
                    return "0".to_string();
                }
            }
            if let Some(ref rx) = glob_re {
                // Helper that runs ONE replacement: takes the
                // captures, populates `state.arrays["match"]`
                // (1-based indexing), then expands the replacement
                // template via `expand_string` so `$match[N]` in
                // the template resolves to the just-captured group.
                // Mirrors C zsh's pat_subme + addbackref handling
                // around Src/pattern.c (pattry, patmatch).
                let expand_repl_with_caps = |caps: &regex::Captures| -> String {
                    if backref_mode {
                        with_executor(|exec| {
                            // `(#b)` — per-group captures into `match[N]`
                            // (1-based array). Also seed `MATCH` with the
                            // whole-match text so `(#m)` plus `$MATCH` in
                            // the replacement returns the matched portion.
                            // Direct port of Src/pattern.c addbackref +
                            // pat_pure_m which sets both views.
                            let mut arr = Vec::with_capacity(caps.len());
                            let mut begins = Vec::with_capacity(caps.len());
                            let mut ends = Vec::with_capacity(caps.len());
                            for i in 1..caps.len() {
                                if let Some(m) = caps.get(i) {
                                    arr.push(m.as_str().to_string());
                                    begins.push((m.start() + 1).to_string());
                                    ends.push(m.end().to_string());
                                } else {
                                    arr.push(String::new());
                                    begins.push("0".to_string());
                                    ends.push("0".to_string());
                                }
                            }
                            exec.arrays.insert("match".to_string(), arr);
                            // mbegin/mend arrays — 1-based start
                            // and end positions of each capture
                            // group. Direct port of zsh's
                            // pat_pure_m population.
                            exec.arrays.insert("mbegin".to_string(), begins);
                            exec.arrays.insert("mend".to_string(), ends);
                            if let Some(m0) = caps.get(0) {
                                exec.variables
                                    .insert("MATCH".to_string(), m0.as_str().to_string());
                                exec.variables
                                    .insert("MBEGIN".to_string(), (m0.start() + 1).to_string());
                                exec.variables
                                    .insert("MEND".to_string(), m0.end().to_string());
                            }
                        });
                        with_executor(|exec| exec.singsub(&repl_raw))
                    } else {
                        repl.clone()
                    }
                };
                match op {
                    0 => {
                        if backref_mode {
                            // `replacen` doesn't expose Captures —
                            // reimplement: find first match, expand
                            // replacement from its caps, splice.
                            if let Some(caps) = rx.captures(&val) {
                                let m = caps.get(0).unwrap();
                                let r = expand_repl_with_caps(&caps);
                                return format!("{}{}{}", &val[..m.start()], r, &val[m.end()..]);
                            }
                            val
                        } else {
                            rx.replacen(&val, 1, repl.as_str()).to_string()
                        }
                    }
                    1 => {
                        if backref_mode {
                            // Iterate each match, build output piecewise.
                            let mut out = String::with_capacity(val.len());
                            let mut last = 0usize;
                            for caps in rx.captures_iter(&val) {
                                let m = caps.get(0).unwrap();
                                out.push_str(&val[last..m.start()]);
                                let r = expand_repl_with_caps(&caps);
                                out.push_str(&r);
                                last = m.end();
                            }
                            out.push_str(&val[last..]);
                            out
                        } else {
                            rx.replace_all(&val, repl.as_str()).to_string()
                        }
                    }
                    2 => {
                        // Anchored prefix: only match at start.
                        if let Some(caps) = rx.captures(&val) {
                            let m = caps.get(0).unwrap();
                            if m.start() == 0 {
                                let r = if backref_mode {
                                    expand_repl_with_caps(&caps)
                                } else {
                                    repl.clone()
                                };
                                return format!("{}{}", r, &val[m.end()..]);
                            }
                        }
                        val
                    }
                    3 => {
                        // Anchored suffix: last match whose end is val.len().
                        let mut last_caps: Option<regex::Captures> = None;
                        for caps in rx.captures_iter(&val) {
                            let m = caps.get(0).unwrap();
                            if m.end() == val.len() {
                                last_caps = Some(caps);
                            }
                        }
                        if let Some(caps) = last_caps {
                            let m = caps.get(0).unwrap();
                            let r = if backref_mode {
                                expand_repl_with_caps(&caps)
                            } else {
                                repl.clone()
                            };
                            return format!("{}{}", &val[..m.start()], r);
                        }
                        val
                    }
                    _ => val,
                }
            } else {
                match op {
                    0 => val.replacen(&pattern, &repl, 1),
                    1 => val.replace(&pattern, &repl),
                    2 => {
                        if val.starts_with(&pattern) {
                            format!("{}{}", repl, &val[pattern.len()..])
                        } else {
                            val
                        }
                    }
                    3 => {
                        if val.ends_with(&pattern) {
                            format!("{}{}", &val[..val.len() - pattern.len()], repl)
                        } else {
                            val
                        }
                    }
                    _ => val,
                }
            }
        };
        // Array case: per-element replacement (default), or
        // join-then-replace when in DQ context. zsh: `"${a/o/O}"`
        // for `a=(one two three)` joins to "one two three", then
        // does the FIRST replacement only -> "One two three".
        // Unquoted `${a/o/O}` per-element first -> "One twO three".
        let arr_val = with_executor(|exec| exec.arrays.get(&name).cloned());
        if let Some(arr) = arr_val {
            if dq_flag {
                let joined = arr.join(" ");
                return fusevm::Value::str(one(joined));
            }
            let mapped: Vec<fusevm::Value> = arr
                .into_iter()
                .map(|s| fusevm::Value::str(one(s)))
                .collect();
            return fusevm::Value::Array(mapped);
        }
        let val = with_executor(|exec| exec.get_variable(&name));
        fusevm::Value::str(one(val))
    });

    vm.register_builtin(BUILTIN_REGISTER_COMPILED_FN, |vm, argc| {
        let args = pop_args(vm, argc);
        let mut iter = args.into_iter();
        let name = iter.next().unwrap_or_default();
        let body_b64 = iter.next().unwrap_or_default();
        let body_source = iter.next().unwrap_or_default();
        let bytes = base64_decode(&body_b64);
        let status = match bincode::deserialize::<fusevm::Chunk>(&bytes) {
            Ok(chunk) => with_executor(|exec| {
                if !body_source.is_empty() {
                    exec.function_source.insert(name.clone(), body_source.clone());
                }
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

fn expand_dollar_refs(s: &str, exec: &crate::ported::exec::ShellExecutor) -> String {
    // Single-pass `$VAR` / `${VAR}` expansion for subscript bodies.
    // Mirrors the small subset of paramsubst needed when the BUILTIN_
    // PARAM_LENGTH handler resolves `${#arr[$i]}`.
    let bytes: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != '$' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        if i + 1 >= bytes.len() {
            out.push('$');
            i += 1;
            continue;
        }
        let next = bytes[i + 1];
        if next == '{' {
            if let Some(close) = bytes[i + 2..].iter().position(|&c| c == '}') {
                let name: String = bytes[i + 2..i + 2 + close].iter().collect();
                out.push_str(&exec.get_variable(&name));
                i += 2 + close + 1;
                continue;
            }
        }
        if next.is_ascii_alphabetic() || next == '_' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == '_') {
                end += 1;
            }
            let name: String = bytes[start..end].iter().collect();
            out.push_str(&exec.get_variable(&name));
            i = end;
            continue;
        }
        out.push('$');
        i += 1;
    }
    out
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
    // this command's argv triggered the `nomatch` error. For BUILTIN
    // commands (zsh: errflag persists in the shell process), the
    // entire script aborts with status 1 — `echo /no_match_*` exits
    // before printing anything. External commands hit the same flag
    // in `host_exec_external` instead, which only fails the command
    // and lets the script continue (zsh's fork inherits-but-resets
    // errflag semantics). We only land here for builtins, so abort.
    let glob_failed = with_executor(|exec| {
        let f = exec.current_command_glob_failed.get();
        if f {
            exec.current_command_glob_failed.set(false);
            exec.last_status = 1;
        }
        f
    });
    if glob_failed {
        std::process::exit(1);
    }
    // `$_` tracks the last argument of the PREVIOUSLY executed
    // command (zsh / bash convention). Promote the deferred value
    // into `$_` BEFORE this command runs (so `echo $_` reads the
    // prior command's last arg) then stash THIS command's last arg
    // for the next dispatch.
    let new_last = args.last().cloned();
    with_executor(|exec| {
        if let Some(prev) = exec.pending_underscore.take() {
            exec.variables.insert("_".to_string(), prev);
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
pub const BUILTIN_ARRAY_LENGTH: u16 = 291;

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
pub const BUILTIN_APPEND_ASSOC: u16 = 298;

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
pub const BUILTIN_GLOB_QUALIFIED: u16 = 302;

/// Re-export the regex_match host method as a builtin so `[[ s =~ pat ]]`
/// works even when fusevm's Op::RegexMatch isn't routed (compat fallback).
pub const BUILTIN_REGEX_MATCH: u16 = 303;

/// Word-split a string on IFS (default: whitespace). Pops one string,
/// returns Value::Array of fields. Used in array-literal context where
/// `arr=($(cmd))` should expand cmd's stdout into multiple elements.
pub const BUILTIN_WORD_SPLIT: u16 = 304;

/// Register a pre-compiled fusevm chunk as a function. Stack: [name,
/// base64-bincode-of-Chunk]. Used by compile_zsh's compile_funcdef to
/// register functions parsed via ZshParser without going through the
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
/// `run_command_substitution` which compiles via ZshParser+ZshCompiler
/// and captures stdout via an in-process pipe. Returns trimmed output
/// as Value::Str. Avoids the sub-chunk word-emit quoting bug in the
/// raw Op::CmdSubst path.
pub const BUILTIN_CMD_SUBST_TEXT: u16 = 313;
/// Text-based word expansion. Pops \[preserved_text\]: the word with
/// quotes preserved (DNULL→`"`, SNULL→`'`, BNULL→`\`), runs
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
pub const BUILTIN_UNKNOWN_COND: u16 = 324;

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
        let brace_ccl = with_executor(|exec|
            exec.options.get("braceccl").copied().unwrap_or(false));
        crate::ported::glob::xpandbraces(s, brace_ccl)
    }

    fn str_match(&mut self, s: &str, pattern: &str) -> bool {
        // Shell glob match — `*`, `?`, `[...]`, alternation. Used by `[[ x = pat ]]`,
        // `case` arms, and any other point that compares against a glob pattern.
        ShellExecutor::glob_match_static(s, pattern)
    }

    fn expand_param(&mut self, name: &str, _modifier: u8, _args: &[fusevm::Value]) -> fusevm::Value {
        // Sole funnel: route through `getsparam` matching C zsh's
        // `getsparam(name)` → `getvalue` → `getstrvalue` →
        // `Param.gsu->getfn` dispatch (Src/params.c:3076 / 2335).
        //
        // The lookup chain (GSU dispatch + variables + env + array-
        // join) lives in `params::getsparam`; subst.rs and this
        // bridge both call into it so the logic is in exactly one
        // place — mirroring C's "every read goes through getsparam"
        // architecture. fuseVM bytecode triggers this bridge when
        // the VM hits a PARAM opcode, equivalent to C's tree-walker
        // hitting a `${...}` AST node.
        //
        // Modifier handling: the `_modifier` / `_args` parameters
        // are populated by the bytecode compiler but applied by
        // separate VM opcodes (LENGTH/STRIP/SUBST/etc.) downstream
        // of this fetch — matching C's split between getsparam
        // (value fetch) and paramsubst's modifier-walk loop. This
        // bridge is the value-fetch step only.
        let val_str = with_executor(|exec| {
            crate::ported::params::getsparam(&exec.variables, &exec.arrays, name)
                .unwrap_or_default()
        });
        fusevm::Value::str(val_str)
    }

    fn regex_match(&mut self, s: &str, regex: &str) -> bool {
        // Untokenize the pattern + subject before compiling. zsh's
        // lexer emits SNULL/DQ markers around quoted regions; if a
        // single-quoted regex like `'([a-z]+)([0-9]+)'` reaches us
        // with the SNULL bytes still present, regex::Regex::new
        // returns Err (the markers aren't valid pattern syntax).
        // Direct port of zsh's bin_test path which calls untokenize()
        // on both operands before handing to the regex compiler
        // (Src/cond.c:cond_match).
        let regex = crate::lex::untokenize(regex);
        let s = crate::lex::untokenize(s);
        let s = s.as_str();
        let regex = regex.as_str();
        // Compile (cached) and run captures so we can populate the
        // zsh-side magic vars: `$MATCH` (full match), `$match[N]`
        // (capture groups), and `$mbegin`/`$mend` (1-based offsets).
        let mut cache = REGEX_CACHE.lock();
        let re = if let Some(re) = cache.get(regex) {
            re.clone()
        } else {
            match regex::Regex::new(regex) {
                Ok(re) => {
                    cache.insert(regex.to_string(), re.clone());
                    re
                }
                Err(_) => return false,
            }
        };
        drop(cache);
        match re.captures(s) {
            Some(caps) => {
                let full = caps
                    .get(0)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();
                let full_begin = caps
                    .get(0)
                    .map(|m| (s[..m.start()].chars().count() + 1).to_string())
                    .unwrap_or_else(|| "0".to_string());
                let full_end = caps
                    .get(0)
                    .map(|m| s[..m.end()].chars().count().to_string())
                    .unwrap_or_else(|| "0".to_string());
                let mut group_strs: Vec<String> = Vec::new();
                let mut begins: Vec<String> = Vec::new();
                let mut ends: Vec<String> = Vec::new();
                for i in 1..caps.len() {
                    if let Some(m) = caps.get(i) {
                        group_strs.push(m.as_str().to_string());
                        begins.push((s[..m.start()].chars().count() + 1).to_string());
                        ends.push(s[..m.end()].chars().count().to_string());
                    } else {
                        group_strs.push(String::new());
                        begins.push("0".to_string());
                        ends.push("0".to_string());
                    }
                }
                with_executor(|exec| {
                    exec.variables.insert("MATCH".to_string(), full);
                    exec.variables.insert("MBEGIN".to_string(), full_begin);
                    exec.variables.insert("MEND".to_string(), full_end);
                    exec.arrays.insert("match".to_string(), group_strs);
                    exec.arrays.insert("mbegin".to_string(), begins);
                    exec.arrays.insert("mend".to_string(), ends);
                });
                true
            }
            None => false,
        }
    }

    fn process_sub_in(&mut self, sub: &fusevm::Chunk) -> String {
        // Run the sub-chunk synchronously (in the current executor context),
        // capture stdout into a temp file, return the path. Synchronous is
        // simpler and avoids the thread-local-executor limitation that
        // spawned threads can't see. Common consumers (`diff`, `cat`,
        // `comm`) read the file once anyway.
        use std::io::Write as _;
        use std::os::unix::io::AsRawFd;
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
        use std::ffi::CString;
        use std::os::unix::io::AsRawFd;
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
            exec.subshell_snapshots.push(SubshellSnapshot {
                variables: exec.variables.clone(),
                arrays: exec.arrays.clone(),
                assoc_arrays: exec.assoc_arrays.clone(),
                positional_params: exec.positional_params.clone(),
                env_vars: std::env::vars().collect(),
                cwd: std::env::current_dir().ok(),
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
                .variables
                .get("ZSH_SUBSHELL")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0);
            exec.variables
                .insert("ZSH_SUBSHELL".to_string(), (level + 1).to_string());
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
                exec.variables = snap.variables;
                exec.arrays = snap.arrays;
                exec.assoc_arrays = snap.assoc_arrays;
                exec.positional_params = snap.positional_params;
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
        with_executor(|exec| exec.host_set_pending_stdin(content.to_string()));
    }

    fn herestring(&mut self, content: &str) {
        // Shell semantics: herestring appends a newline.
        let mut s = content.to_string();
        s.push('\n');
        with_executor(|exec| exec.host_set_pending_stdin(s));
    }

    fn exec(&mut self, args: Vec<String>) -> i32 {
        // Track `$_` as the last argument of the last command (zsh /
        // bash convention). Empty arglists leave it untouched.
        if let Some(last) = args.last() {
            with_executor(|exec| {
                exec.variables.insert("_".to_string(), last.clone());
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
        use std::io::Read;
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
                return Some(with_executor(|exec| exec.builtin_zmv(&args, "mv")));
            }
            "zcp" => {
                return Some(with_executor(|exec| exec.builtin_zmv(&args, "cp")));
            }
            "zln" => {
                return Some(with_executor(|exec| exec.builtin_zmv(&args, "ln")));
            }
            "zcalc" => {
                return Some(with_executor(|exec| exec.builtin_zcalc(&args)));
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
        let already_expanding = with_executor(|exec| exec.expanding_aliases.contains(name));
        let alias_body = if already_expanding {
            None
        } else {
            with_executor(|exec| exec.aliases.get(name).cloned())
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
            with_executor(|exec| exec.expanding_aliases.insert(name.to_string()));
            let status = with_executor(|exec| exec.execute_script(&combined).unwrap_or(1));
            with_executor(|exec| exec.expanding_aliases.remove(name));
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
            // function_exists() true even though no Chunk has landed yet,
            // so trigger autoload BEFORE the existence check. maybe_autoload
            // is a no-op when autoload_pending doesn't hold the name.
            if exec.autoload_pending.contains_key(name) {
                exec.maybe_autoload(name);
            }
            if let Some(c) = exec.functions_compiled.get(name) {
                return Some(c.clone());
            }
            // Eager fpath/ZWC scan for unknown names is non-zsh-compatible
            // (zsh only autoloads when an explicit `autoload name` was
            // declared). Skip the scan in `-f` (no-rcs) mode so the user's
            // FPATH-resident wrappers — `rm`, `cd`, etc. — don't shadow
            // builtins/externals when they explicitly asked for a
            // minimal shell. With rcs enabled we keep the legacy eager
            // behavior to avoid breaking interactive sessions that rely
            // on it.
            let rcs_enabled = exec.options.get("rcs").copied().unwrap_or(true);
            if rcs_enabled && !exec.function_exists(name) {
                let _ = exec.autoload_function(name);
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
            exec.variables
                .get("FUNCNEST")
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
        let saved_options = with_executor(|exec| exec.options.clone());
        let (
            saved_params,
            saved_local_count,
            saved_local_arr_count,
            saved_local_assoc_count,
            saved_zero,
            saved_scriptname,
            saved_funcstack,
            saved_exit_trap,
        ) = with_executor(|exec| {
            let prev = std::mem::replace(&mut exec.positional_params, args.clone());
            let count = exec.local_save_stack.len();
            let arr_count = exec.local_array_save_stack.len();
            let assoc_count = exec.local_assoc_save_stack.len();
            exec.local_scope_depth += 1;
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
            let prev_zero = exec.variables.insert("0".to_string(), display_name.clone());
            // scriptname: PS4's `%N` and error-message prefix both
            // read `exec.scriptname`. Inside a function, C zsh sets
            // `scriptname = dupstring(name)` at Src/exec.c:5903 so
            // `%N` shows the function name. Save the outer
            // scriptname before overwrite; restored on return.
            let prev_scriptname = std::mem::replace(
                &mut exec.scriptname,
                Some(display_name.clone()),
            );
            // funcstack: prepend the function name; outermost call
            // is at the END of the stack per zsh.
            let prev_stack = exec.arrays.get("funcstack").cloned();
            let mut new_stack = vec![fn_name.clone()];
            if let Some(ref s) = prev_stack {
                new_stack.extend_from_slice(s);
            }
            exec.arrays.insert("funcstack".to_string(), new_stack);
            // Set `$_` BEFORE the function body runs. zsh: inside
            // a function, `echo $_` reads the function name (when
            // called with no args) or the last call-arg.
            // Without this, internal builtins that ran before
            // (like REGISTER_COMPILED_FN) leaked their last arg
            // (the function body source!) as $_.
            let dollar_underscore = args.last().cloned().unwrap_or_else(|| fn_name.clone());
            exec.variables
                .insert("_".to_string(), dollar_underscore.clone());
            exec.pending_underscore = Some(dollar_underscore);
            (
                prev,
                count,
                arr_count,
                assoc_count,
                prev_zero,
                prev_scriptname,
                prev_stack,
                saved,
            )
        });

        let mut vm = fusevm::VM::new(chunk);
        register_builtins(&mut vm);
        // Seed the function-body VM with the parent's `$?` so a
        // function that reads `$?` BEFORE running any command sees
        // the caller's last status. Direct port of zsh's exec.c
        // `execfuncdef`/`doshfunc` semantics — function entry does
        // NOT reset `$?`. Without this, `false; foo() { echo $?; }; foo`
        // printed 0 instead of 1 because the fresh VM defaulted
        // last_status to 0.
        vm.last_status = with_executor(|exec| exec.last_status);
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
                exec.last_status = status;
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
            exec.variables
                .insert("_".to_string(), last_call_arg.clone());
            exec.pending_underscore = Some(last_call_arg);
            exec.positional_params = saved_params;
            exec.local_scope_depth -= 1;
            // LOCAL_OPTIONS: when set at function exit, restore all
            // options to the snapshot taken at entry. `emulate -L`
            // arms this; plugin code uses both forms to scope option
            // changes inside helpers without leaking to callers.
            // Without it, `setopt no_glob` inside a helper polluted
            // the caller's option state.
            if exec
                .options
                .get("localoptions")
                .copied()
                .unwrap_or(false)
            {
                exec.options = saved_options.clone();
            }
            // Restore `$0`, scriptname, and `$funcstack` to their
            // pre-call values. scriptname mirrors C exec.c:5907
            // `scriptname = oldscriptname;` after execode returns.
            match saved_zero {
                Some(v) => {
                    exec.variables.insert("0".to_string(), v);
                }
                None => {
                    exec.variables.remove("0");
                }
            }
            exec.scriptname = saved_scriptname;
            match saved_funcstack {
                Some(s) => {
                    exec.arrays.insert("funcstack".to_string(), s);
                }
                None => {
                    exec.arrays.remove("funcstack");
                }
            }
            // Unwind any `local` declarations made during the function call.
            while exec.local_save_stack.len() > saved_local_count {
                if let Some((var_name, old_val)) = exec.local_save_stack.pop() {
                    match old_val {
                        Some(v) => {
                            exec.variables.insert(var_name, v);
                        }
                        None => {
                            exec.variables.remove(&var_name);
                        }
                    }
                }
            }
            // Same for `local arr=(...)` array bindings — restore the
            // outer array's elements (or remove if there was none).
            while exec.local_array_save_stack.len() > saved_local_arr_count {
                if let Some((arr_name, old_arr)) = exec.local_array_save_stack.pop() {
                    match old_arr {
                        Some(items) => {
                            exec.arrays.insert(arr_name, items);
                        }
                        None => {
                            exec.arrays.remove(&arr_name);
                        }
                    }
                }
            }
            // Same for `typeset -A h=(...)` assoc bindings — restore
            // the outer assoc (or remove if there was none).
            while exec.local_assoc_save_stack.len() > saved_local_assoc_count {
                if let Some((assoc_name, old_assoc)) = exec.local_assoc_save_stack.pop() {
                    match old_assoc {
                        Some(map) => {
                            exec.assoc_arrays.insert(assoc_name, map);
                        }
                        None => {
                            exec.assoc_arrays.remove(&assoc_name);
                        }
                    }
                }
            }
        });

        Some(status)
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Host-routed shell ops: ShellExecutor methods invoked by ZshrsHost from the
// fusevm VM. Not a port of Src/exec.c (see file-level docs above) — they're
// the bridge between fusevm opcodes and ShellExecutor state.
// ───────────────────────────────────────────────────────────────────────────
impl crate::ported::exec::ShellExecutor {
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
        last_status: &mut i32,
        redirect_failed: &mut bool,
    ) {
        use std::os::unix::io::IntoRawFd;
        match result {
            Ok(file) => {
                let new_fd = file.into_raw_fd();
                unsafe {
                    libc::dup2(new_fd, fd);
                    libc::close(new_fd);
                }
            }
            Err(e) => {
                let msg = match e.kind() {
                    std::io::ErrorKind::PermissionDenied => "permission denied",
                    std::io::ErrorKind::NotFound => "no such file or directory",
                    std::io::ErrorKind::IsADirectory => "is a directory",
                    _ => "redirect failed",
                };
                eprintln!("zshrs:1: {}: {}", msg, target);
                *last_status = 1;
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
            }
        }
    }

    pub fn host_apply_redirect(&mut self, fd: u8, op_byte: u8, target: &str) {
        use fusevm::op::redirect_op as r;
        use std::os::unix::io::IntoRawFd;
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
                let noclobber = self.options.get("noclobber").copied().unwrap_or(false)
                    || !self.options.get("clobber").copied().unwrap_or(true);
                if noclobber && std::path::Path::new(target).exists() {
                    eprintln!("zshrs:1: file exists: {}", target);
                    self.last_status = 1;
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
                Self::redir_open_or_fail(
                    fd,
                    std::fs::File::create(target),
                    target,
                    &mut self.last_status,
                    &mut self.redirect_failed,
                );
            }
            r::CLOBBER => {
                Self::redir_open_or_fail(
                    fd,
                    std::fs::File::create(target),
                    target,
                    &mut self.last_status,
                    &mut self.redirect_failed,
                );
            }
            r::APPEND => {
                Self::redir_open_or_fail(
                    fd,
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(target),
                    target,
                    &mut self.last_status,
                    &mut self.redirect_failed,
                );
            }
            r::READ => {
                Self::redir_open_or_fail(
                    fd,
                    std::fs::File::open(target),
                    target,
                    &mut self.last_status,
                    &mut self.redirect_failed,
                );
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
        use std::io::Write;
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
            self.last_status = 1;
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
            return self.last_status;
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
            "echotc" => return self.bin_echotc(&rest_vec),
            "echoti" => return self.bin_echoti(&rest_vec),
            "getln" => return self.builtin_getln(&rest_vec),
            "zpty" => return self.bin_zpty(&rest_vec),
            "ztcp" => return self.bin_ztcp(&rest_vec),
            "zsocket" => {
                // Shim — parses the BUILTIN spec "ad:ltv" from
                // socket.c:276 into a real `options` struct, then
                // invokes the canonical free-fn port at
                // crate::ported::modules::socket::bin_zsocket whose
                // signature matches C `bin_zsocket(nam, args, ops,
                // func)` exactly.
                use crate::ported::zsh_h::{options, MAX_OPS};
                let mut ops = options { ind: [0u8; MAX_OPS], args: Vec::new(),
                                        argscount: 0, argsalloc: 0 };
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
                        if rest.is_empty() { positional.push(a.clone()); i += 1; continue; }
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
                            if c.is_ascii_alphabetic() { ops.ind[c as usize] = 1; }
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
                use crate::ported::zsh_h::{options, MAX_OPS};
                let mut ops = options { ind: [0u8; MAX_OPS], args: Vec::new(),
                                        argscount: 0, argsalloc: 0 };
                let mut assigns: Vec<(String, String)> = Vec::new();
                return crate::modules::param_private::bin_private("private",
                    &rest_vec, &mut ops, 0, &mut assigns);
            }
            "zformat" => return self.bin_zformat(&rest_vec),
            "zregexparse" => return self.bin_zregexparse(&rest_vec),
            // zsh-bundled rename helpers — implemented natively in
            // Rust so `autoload -U zmv` works without shipping the
            // function source. (Without this, the autoload path hangs.)
            "zmv" => return self.builtin_zmv(&rest_vec, "mv"),
            "zcp" => return self.builtin_zmv(&rest_vec, "cp"),
            "zln" => return self.builtin_zmv(&rest_vec, "ln"),
            "zcalc" => return self.builtin_zcalc(&rest_vec),
            "zselect" => return self.bin_zselect(&rest_vec),
            "cap" => return self.bin_cap(&rest_vec),
            "getcap" => return self.bin_getcap(&rest_vec),
            "setcap" => return self.bin_setcap(&rest_vec),
            "yes" => return self.builtin_yes(&rest_vec),
            "nl" => return self.builtin_nl(&rest_vec),
            "env" => return self.builtin_env(&rest_vec),
            "printenv" => return self.builtin_printenv(&rest_vec),
            "tty" => return self.builtin_tty(&rest_vec),
            "chgrp" => {
                // Canonical bin_chown per files.c:725 with func=BIN_CHGRP
                // per the bintab entry at c:805. BUILTIN spec "hRs".
                use crate::ported::zsh_h::{options, MAX_OPS};
                let mut ops = options { ind: [0u8; MAX_OPS], args: Vec::new(),
                                        argscount: 0, argsalloc: 0 };
                let mut positional: Vec<String> = Vec::new();
                let mut i = 0;
                while i < rest_vec.len() {
                    let a = &rest_vec[i];
                    if a == "--" { i += 1; positional.extend_from_slice(&rest_vec[i..]); break; }
                    if let Some(rest) = a.strip_prefix('-') {
                        if rest.is_empty() { positional.push(a.clone()); i += 1; continue; }
                        for c in rest.chars() {
                            let cb = c as u8;
                            if cb.is_ascii_alphabetic() { ops.ind[cb as usize] = 1; }
                        }
                    } else {
                        positional.push(a.clone());
                    }
                    i += 1;
                }
                return crate::ported::modules::files::bin_chown(
                    "chgrp", &positional, &ops,
                    crate::ported::modules::files::BIN_CHGRP);
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
            // the bare name. Each arm here routes to the matching
            // `builtin_*` method already defined further down in
            // this file.
            // "zf_chmod" => return self.bin_chmod(&rest_vec),
            // "zf_chown" => return self.bin_chown("zf_chown", &rest_vec),
            // "zf_chgrp" => return self.bin_chown("zf_chgrp", &rest_vec),
            // "zf_ln" => return self.bin_ln("zf_ln", &rest_vec),
            // "zf_mkdir" => return self.bin_mkdir(&rest_vec),
            // "zf_mv" => return self.bin_ln("zf_mv", &rest_vec),
            // "zf_rm" => return self.bin_rm(&rest_vec),
            // "zf_rmdir" => return self.bin_rmdir(&rest_vec),
            // "zf_sync" => return self.bin_sync(&rest_vec),
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
                use crate::ported::zsh_h::{options, MAX_OPS};
                let ops = options { ind: [0u8; MAX_OPS], args: Vec::new(),
                                    argscount: 0, argsalloc: 0 };
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
