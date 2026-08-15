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

    /// Port of C's `FILE *xtrerr` xtrace stream (Src/exec.c:81). C builds
    /// each trace line in this stdio buffer — `printprompt4` does
    /// `fprintf(xtrerr, …)`, then args via `fputs`/`fputc` — and
    /// `fflush(xtrerr)` writes the WHOLE line to stderr in one syscall
    /// (makecline c:2122-2123, addvars c:2588, condition c:1372). That
    /// single flush is exactly why a forked pipeline stage's trace line
    /// reaches the shared stderr fd atomically and never interleaves with
    /// a concurrent stage. zshrs previously emitted PS4 and the command
    /// text as separate `eprint!` writes, which raced under load. Model
    /// the FILE buffer as this thread-local String (a forked child owns
    /// its own copy); `xtrerr_fputs` appends, `xtrerr_flush` does the
    /// single write.
    static XTRERR: RefCell<String> = const { RefCell::new(String::new()) };

    /// Stack of (RETFLAG, BREAKS, CONTFLAG, EXIT_PENDING) tuples saved
    /// at try-block exit so the always-arm body can run cleanly even
    /// when the try-block fired `return` / `break` / `continue` /
    /// `exit`. Restored right before the post-always re-jump so the
    /// escape resumes propagation past the construct.
    /// c:Src/exec.c WC_TRYBLOCK — zsh's wordcode walker handles this
    /// inline; the zshrs port lifts it into a paired SET / RESTORE
    /// pair around the always-arm.
    /// Tuple: `(retflag, breaks, contflag, exit_pending, try_errflag,
    /// try_interrupt)`. The last two are c:Src/loop.c:762-763's
    /// `save_try_errflag` / `save_try_interrupt` — the ENCLOSING try
    /// block's values, restored at c:778-779 so nested
    /// `{…} always {…}` constructs don't leak `$TRY_BLOCK_ERROR`
    /// outward.
    static TRY_ESCAPE_SAVE: RefCell<Vec<(i32, i32, i32, i32, i64, i64)>> =
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
    /// c:Src/exec.c:5025 getproc (PATH_DEV_FD branch) — the parent
    /// keeps the `>(cmd)` pipe WRITE end open under `/dev/fd/N`,
    /// parks it in the job's filelist (`fdtable[fd] =
    /// FDT_PROC_SUBST; addfilelist(NULL, fd)`), and deletefilelist
    /// closes it when the consuming job finishes — that close is
    /// what lets the `>(cmd)` child's reader see EOF. zshrs runs
    /// commands in-process, so the equivalent is: record
    /// `(scope_depth, fd)` here and drain after the consuming
    /// command (external exec or builtin dispatch) completes.
    static PSUB_PENDING_FDS: RefCell<Vec<(usize, i32)>> = const { RefCell::new(Vec::new()) };
    /// `(scope_depth, path)` for `=(cmd)` temp files, unlinked at the same
    /// job-end boundary as the pending fds (c:Src/jobs.c deletefilelist).
    static PSUB_PENDING_FILES: RefCell<Vec<(usize, String)>> = const { RefCell::new(Vec::new()) };
    /// Scope depth for PSUB_PENDING_FDS tagging. Incremented around
    /// nested execution contexts (cmd-subst bodies, shell-function
    /// bodies) so a command running INSIDE the nested context only
    /// drains its own `>(cmd)` fds, never the enclosing command's
    /// (e.g. `tee >(wc) $(print x)` — print must not close tee's
    /// fd). Mirrors C's per-job filelist ownership.
    static PSUB_SCOPE_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// Forked `<(cmd)`/`>(cmd)` child pids awaiting reap. Drained
    /// non-blockingly (WNOHANG) by note_psub_child so proc-sub children
    /// don't accumulate as zombies across a shell session.
    static PSUB_CHILDREN: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
}

/// Record a proc-sub child pid and best-effort reap any already-exited
/// proc-sub children (WNOHANG). Non-blocking: still-running children
/// stay parked and get reaped on a later call.
pub(crate) fn note_psub_child(pid: i32) {
    if pid <= 0 {
        return;
    }
    PSUB_CHILDREN.with(|v| {
        let mut v = v.borrow_mut();
        v.push(pid);
        v.retain(|&p| {
            let mut status = 0;
            // WNOHANG: reap if exited, else keep parked.
            let r = unsafe { libc::waitpid(p, &mut status, libc::WNOHANG) };
            // r == p → reaped; r == 0 → still running (keep); r < 0 →
            // already gone/not ours (drop).
            r == 0
        });
    });
}

/// Port of `fputs(s, xtrerr)` / `fprintf(xtrerr, "%s", s)` (Src/exec.c):
/// append `s` to the buffered xtrace line. Nothing reaches stderr until
/// [`xtrerr_flush`] (the port of `fflush(xtrerr)`) writes the line whole.
pub(crate) fn xtrerr_fputs(s: &str) {
    XTRERR.with(|b| b.borrow_mut().push_str(s));
}

/// Port of `fflush(xtrerr)` (Src/exec.c:1373/2123/2596): write the
/// buffered xtrace line to stderr in ONE `write` and clear the buffer, so
/// the line lands on the shared fd atomically (no interleaving across
/// concurrent pipeline stages).
pub(crate) fn xtrerr_flush() {
    XTRERR.with(|b| {
        let mut buf = b.borrow_mut();
        if !buf.is_empty() {
            use std::io::Write;
            let _ = std::io::stderr().write_all(buf.as_bytes());
            buf.clear();
        }
    });
}

/// RAII guard bumping the psub scope depth — see PSUB_SCOPE_DEPTH.
pub(crate) struct PsubScope;

impl PsubScope {
    pub(crate) fn enter() -> Self {
        PSUB_SCOPE_DEPTH.with(|d| d.set(d.get() + 1));
        PsubScope
    }
}

impl Drop for PsubScope {
    fn drop(&mut self) {
        PSUB_SCOPE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// RAII guard bumping `$ZSH_SUBSHELL` for the duration of an
/// in-process command substitution.
///
/// c:Src/exec.c:1161 — entersubsh() does `zsh_subshell++;` and zsh's
/// cmdsub FORKS, so the increment dies with the child and the parent's
/// value is untouched. zshrs runs cmdsubs in-process on a nested VM,
/// so the visible param must be bumped on entry and restored on exit.
/// Writes paramtab u_val directly because ZSH_SUBSHELL is PM_READONLY
/// (same bypass pattern as the subshell-builtin bump below); also
/// mirrors into ported::exec::zsh_subshell so exec.c:4376-style
/// `forked | zsh_subshell` reads agree.
pub(crate) struct CmdSubstSubshellBump {
    saved_val: i64,
    saved_str: Option<String>,
}

impl CmdSubstSubshellBump {
    pub(crate) fn enter() -> Self {
        let mut saved_val = 0i64;
        let mut saved_str = None;
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            if let Some(pm) = tab.get_mut("ZSH_SUBSHELL") {
                saved_val = pm.u_val;
                saved_str = pm.u_str.clone();
                pm.u_val = saved_val + 1;
                pm.u_str = Some((saved_val + 1).to_string());
                pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
            }
        }
        crate::ported::exec::zsh_subshell.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        CmdSubstSubshellBump {
            saved_val,
            saved_str,
        }
    }
}

impl Drop for CmdSubstSubshellBump {
    fn drop(&mut self) {
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            if let Some(pm) = tab.get_mut("ZSH_SUBSHELL") {
                pm.u_val = self.saved_val;
                pm.u_str = self.saved_str.take();
            }
        }
        crate::ported::exec::zsh_subshell.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Port of deletefilelist() from Src/jobs.c (the `>(cmd)` fd arm):
/// closes every pending proc-subst write end created at or inside
/// the current scope depth, exactly when C deletes the consuming
/// job's filelist (getproc parks the fd there via
/// `addfilelist(NULL, fd)`, Src/exec.c:5025+).
fn close_pending_psub_fds() {
    let depth = PSUB_SCOPE_DEPTH.with(|d| d.get());
    PSUB_PENDING_FDS.with(|v| {
        v.borrow_mut().retain(|&(d, fd)| {
            if d >= depth {
                unsafe { libc::close(fd) };
                false
            } else {
                true
            }
        });
    });
    // c:Src/jobs.c deletefilelist — `=(cmd)` temp files are unlinked at the
    // same job-end boundary. `f==(print x); [[ -f $f ]]` is false after the
    // command line ends.
    PSUB_PENDING_FILES.with(|v| {
        v.borrow_mut().retain(|(d, path)| {
            if *d >= depth {
                let _ = fs::remove_file(path);
                false
            } else {
                true
            }
        });
    });
}

/// RAII drain guard — instantiated at the top of the consuming-
/// command paths (dispatch_builtin, ZshrsHost::exec) so the pending
/// `>(cmd)` write ends close on every exit path once the command
/// finished, exactly when C's job filelist would be deleted.
struct PsubFdGuard;

impl Drop for PsubFdGuard {
    fn drop(&mut self) {
        close_pending_psub_fds();
    }
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

thread_local! {
    /// The pipeline fds a stage still has to install onto 0/1, as
    /// `(input, output)` with -1 meaning "leave this fd alone".
    ///
    /// c:Src/exec.c:3720-3724 — the pipe's read/write ends are the
    /// FIRST entries of the command's multio table:
    ///     /* Add pipeline input/output to mnodes */
    ///     if (input)  addfd(forked, save, mfds, 0, input, 0, NULL);
    ///     if (output) addfd(forked, save, mfds, 1, output, 1, NULL);
    /// and that runs AFTER prefork (c:3304) + globlist (c:3702) have
    /// expanded the command's argument words. So a `$(...)` inside a
    /// stage's ARGS reads the shell's original fd 0, not the pipe:
    /// `print -rl -- c a b | print -r -- "[$(cat)]"` prints `[]`.
    /// (The stage's own fork at c:3000 happens before the expansion,
    /// which is why `${x::=v}` in a non-last stage doesn't survive —
    /// but the fds are still installed after it.)
    ///
    /// zshrs's [`BUILTIN_RUN_PIPELINE`] forks per stage, so it parks
    /// the stage's fds here instead of dup2'ing them itself, and the
    /// compiled stage chunk installs them via
    /// [`BUILTIN_PIPE_FDS_INSTALL`] at the C-faithful point: after the
    /// arg-word ops, before the redirect scope
    /// (compile_zsh.rs::emit_stage_fds_install). A compound stage
    /// (`{ … }`, `( … )`, a function) installs at chunk entry — its
    /// body legitimately reads the pipe.
    static PENDING_STAGE_FDS: std::cell::Cell<(i32, i32)> = const { std::cell::Cell::new((-1, -1)) };
}

/// Park the current stage's `(input, output)` pipe fds for the stage
/// chunk's `BUILTIN_PIPE_FDS_INSTALL` to pick up. Returns the previous
/// value so a nested pipeline (`print -- "$(a | b)" | c`) can restore
/// the outer stage's still-uninstalled fds when it finishes.
fn stage_fds_park(input: i32, output: i32) -> (i32, i32) {
    PENDING_STAGE_FDS.with(|c| c.replace((input, output)))
}

/// Take (and clear) the parked stage fds.
fn stage_fds_take() -> (i32, i32) {
    PENDING_STAGE_FDS.with(|c| c.replace((-1, -1)))
}

// Thread-local pointer to the current ShellExecutor.
// Set before VM execution, cleared after. Used by builtin handlers.
thread_local! {
    static CURRENT_EXECUTOR: RefCell<Option<*mut ShellExecutor>> = const { RefCell::new(None) };
    /// The installed session executor, registered by
    /// `exec::install_session_executor`. Lets [`with_session_context`]
    /// establish a VM execution context for STARTUP work that runs
    /// before the loop's first `execode` enters one (rc-file sourcing in
    /// `run_init_scripts`, c:1914). Mirrors exec.rs's own
    /// `SESSION_EXECUTOR`; kept here so the context helper lives next to
    /// `ExecutorContext`/`CURRENT_EXECUTOR`.
    static SESSION_EXECUTOR_PTR: std::cell::Cell<Option<*mut ShellExecutor>> = const { std::cell::Cell::new(None) };
    /// GLOB_ASSIGN eligibility carrier. Set true by BUILTIN_MARK_GLOB_ELIGIBLE
    /// (emitted by the compiler ONLY when a scalar-assignment RHS carries an
    /// UNQUOTED glob token — Star/Quest/Inbrack), read+cleared by the next
    /// BUILTIN_SET_VAR. Matches C zsh's `GLOB_ASSIGN` (Src/exec.c:2554): only
    /// a literal unquoted glob pattern in the wordcode is globbed; values from
    /// `$param` / `$(cmd)` / quoted strings are NOT (verified against zsh).
    /// The runtime SET_VAR value arrives untokenized (the compiler DQ-wraps to
    /// suppress compile-time globbing), so quoting can no longer be recovered
    /// from the value bytes — this flag carries the compile-time decision.
    static SET_VAR_GLOB_ELIGIBLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Register the session executor pointer (called from
/// `install_session_executor`). See [`with_session_context`].
pub fn register_session_executor(exec: &mut ShellExecutor) {
    SESSION_EXECUTOR_PTR.with(|c| c.set(Some(exec as *mut ShellExecutor)));
}

/// Run `f` with the registered session executor established as
/// `CURRENT_EXECUTOR`, so code reaching the live executor via
/// `try_with_executor` works even when no per-command `execode` context
/// is active yet.
///
/// Sole caller: `zsh_main`'s `run_init_scripts()` (c:1914), which
/// sources `.zshenv`/`.zshrc`/`.zlogin` via `source()` BEFORE the loop's
/// first `execode`. Without an active context those sourced bodies
/// `try_with_executor` → `None` → no-op, so the shell silently ignored
/// the user's dotfiles. The scope is entered once around the startup
/// sourcing window and dropped before the loop begins — deliberately NOT
/// Run `f` with the registered session executor established as
/// `CURRENT_EXECUTOR`, so code reaching the live executor via
/// `try_with_executor` works even when no per-command `execode` context
/// is active yet.
///
/// Sole caller: `zsh_main`'s `run_init_scripts()` (c:1914), which
/// sources `.zshenv`/`.zshrc`/`.zlogin` via `source()` BEFORE the loop's
/// first `execode`. Without an active context those sourced bodies
/// `try_with_executor` → `None` → no-op, so the shell silently ignored
/// the user's dotfiles. The scope is entered once around the startup
/// sourcing window and dropped before the loop begins — deliberately NOT
/// a global fallback inside `execute_script_zsh_pipeline`, which would
/// re-enter the executor on nested command substitution and block on
/// input.
pub fn with_session_context<R>(f: impl FnOnce() -> R) -> R {
    let ptr = SESSION_EXECUTOR_PTR.with(|c| c.get());
    match ptr {
        // SAFETY: set by install_session_executor to an executor that
        // outlives the single-threaded interactive session.
        Some(ptr) => {
            let _ctx = ExecutorContext::enter(unsafe { &mut *ptr });
            f()
        }
        None => f(),
    }
}

/// Merge finished background-compinit results into shell state, callable
/// from any site (active VM context first, session executor otherwise —
/// the ZLE completion path runs OUTSIDE a VM frame, where
/// `try_with_executor` alone is None and $_comps stayed empty forever).
pub fn drain_compinit_bg_hook() {
    if try_with_executor(|exec| exec.drain_compinit_bg()).is_some() {
        return;
    }
    let ptr = SESSION_EXECUTOR_PTR.with(|c| c.get());
    if let Some(ptr) = ptr {
        // SAFETY: per with_session_context.
        let _ctx = ExecutorContext::enter(unsafe { &mut *ptr });
        unsafe { (*ptr).drain_compinit_bg() }
    }
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

/// Non-panicking variant of [`with_executor`]: runs `f` against the
/// current executor and returns `Some(result)`, or `None` when no
/// executor is in scope (`CURRENT_EXECUTOR` unset — e.g. unit tests /
/// compsys contexts with no fusevm bridge running).
///
/// This is the primitive the `crate::ported::exec` accessor wrappers
/// (array/assoc/dispatch_function_call/execute_script/...) use to
/// reach the live executor while preserving the exact "no executor →
/// fall back to the direct param table / default value" semantics that
/// the deleted `exec_hooks` OnceLock layer encoded via its
/// "is-the-hook-installed?" check. `CURRENT_EXECUTOR` being set is the
/// faithful equivalent of "the bridge installed the hooks".
#[inline]
pub(crate) fn try_with_executor<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut ShellExecutor) -> R,
{
    CURRENT_EXECUTOR.with(|cell| {
        let ptr = (*cell.borrow())?;
        // SAFETY: same contract as with_executor — the pointer is valid
        // for the duration of VM execution and access is single-threaded.
        let executor = unsafe { &mut *ptr };
        Some(f(executor))
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

/// c:Src/subst.c:505-507 + Src/exec.c:3378-3380 — per-command
/// CSH_NULL_GLOB outcome check. During this command's word expansion
/// `expand_glob` accumulated `badcshglob |= 1` per failed glob and
/// `|= 2` per successful one (Src/glob.c:1871-1875). Exactly 1 —
/// failures and no successes — is the csh-style error: `no match`,
/// command skipped, status 1. Any other value (0 = no globs, 2/3 =
/// at least one matched) is silent. Always resets the counter for
/// the next command (C resets at prefork entry, subst.rs:1307).
/// Returns true when the error fired; callers mirror their
/// glob_failed handling (builtins leave ERRFLAG_ERROR set so the
/// script aborts, externals clear it so the next sublist runs —
/// verified against zsh 5.9.1).
/// Restore the user's GLOB_SUBST after a `${~spec}` carrier flip
/// (see subst::TILDE_GLOBSUBST_CARRIER). Runs at the same
/// command-dispatch boundaries that consume glob_failed /
/// badcshglob — by then every glob op of the current word pipeline
/// has read the carrier.
pub(crate) fn consume_tilde_globsubst_carrier() {
    crate::ported::subst::TILDE_GLOBSUBST_CARRIER.with(|c| {
        if let Some(saved) = c.take() {
            crate::ported::options::opt_state_set("globsubst", saved);
        }
    });
}

/// Pop `argc` stack slots for a whole-array assignment: the LAST popped
/// (deepest pushed) is the param name, the rest are the values in stack
/// order, with any `Value::Array` flattened to its elements. Mirrors the
/// Flatten an array-assignment RHS value into scalar strings, descending
/// through nested `Value::Array`s. zsh arrays are always flat, so recursion
/// only collapses the wrapper layers the compiler introduces — in particular
/// the single `Value::Array` built by `Op::MakeArray` for `arr=(...)` literals
/// (used to dodge `CallBuiltin`'s u8 argc cap), whose own elements may
/// themselves be arrays from an unquoted `$other_array` expansion. A
/// one-level flatten would stringify those inner arrays into a single element.
fn flatten_array_value(v: Value, out: &mut Vec<String>) {
    match v {
        Value::Array(items) => {
            for it in items.iter() {
                flatten_array_value(it.clone(), out);
            }
        }
        other => out.push(other.to_str()),
    }
}

/// pop/flatten prologue of BUILTIN_SET_ARRAY / BUILTIN_APPEND_ARRAY.
fn pop_array_args_with_name(vm: &mut fusevm::VM, argc: u8) -> (String, Vec<String>) {
    let n = argc as usize;
    let mut popped: Vec<Value> = Vec::with_capacity(n);
    for _ in 0..n {
        popped.push(vm.pop());
    }
    popped.reverse();
    let name = popped.pop().map(|v| v.to_str()).unwrap_or_default();
    let mut values: Vec<String> = Vec::new();
    for v in popped {
        flatten_array_value(v, &mut values);
    }
    (name, values)
}

fn consume_badcshglob() -> bool {
    let v = crate::ported::glob::BADCSHGLOB.swap(0, std::sync::atomic::Ordering::Relaxed);
    if v == 1 {
        crate::ported::utils::zerr("no match"); // c:Src/subst.c:507
        true
    } else {
        false
    }
}

/// Map a builtin name to the zsh module that owns it, IFF zsh does
/// not auto-load that builtin on first use. Used by
/// `dispatch_builtin_raw` to gate `--zsh` mode dispatch behind
/// `zmodload`, mirroring `zsh -fc <name>` returning 127 for these
/// names without an explicit module load.
///
/// Returns `Some(module_name)` if `name` belongs to a non-auto-load
/// module per the per-module `Src/Modules/<x>.c` `bintab[]` plus
/// the auto-load flag set at module-build time. `None` for core
/// builtins and for auto-loaded module builtins (sched, log, echotc,
/// echoti, zformat, zparseopts, zregexparse, zstyle, strftime,
/// private, vared, zle, bindkey, comp*) which work without zmodload.
fn module_bound_builtin_module(name: &str) -> Option<&'static str> {
    match name {
        "zftp" => Some("zsh/zftp"),
        "zsocket" => Some("zsh/net/socket"),
        "ztcp" => Some("zsh/net/tcp"),
        "zstat" => Some("zsh/stat"),
        "zselect" => Some("zsh/zselect"),
        "zpty" => Some("zsh/zpty"),
        "zprof" => Some("zsh/zprof"),
        "zsystem" | "syserror" => Some("zsh/system"),
        "clone" => Some("zsh/clone"),
        "zcurses" => Some("zsh/curses"),
        "ztie" | "zuntie" | "zgdbmpath" => Some("zsh/db/gdbm"),
        "pcre_compile" | "pcre_match" | "pcre_study" => Some("zsh/pcre"),
        "example" => Some("zsh/example"),
        "cap" | "getcap" | "setcap" => Some("zsh/cap"),
        "zgetattr" | "zsetattr" | "zdelattr" | "zlistattr" => Some("zsh/attr"),
        // c:Src/Modules/datetime.c — `strftime` is registered via
        // partab[] when zsh/datetime loads. Verified by
        // `zsh -fc 'strftime -s s %Y 0'` → 127 "command not found".
        "strftime" => Some("zsh/datetime"),
        _ => None,
    }
}

/// Dispatch a zshrs-ORIGINAL builtin by NAME, argv-style. These are
/// registered as fusevm opcodes in [`register_builtins`] (async, doctor,
/// peach, …), so a *literal* name compiles to `CallBuiltin` and runs. But
/// they are absent from the static `BUILTINS` port table and the merged
/// `builtintab`, so when the command name is resolved only at run time —
/// `$var` indirection, `builtin NAME` — the ported command-resolution path
/// never finds them and reports "command not found" / "no such builtin",
/// even though `whence` (correctly) calls them builtins. The
/// `register_builtins` closures use the VM only to pop args and then call an
/// executor method, so the identical dispatch works here from any parent-side
/// resolver that has an executor — no VM re-entry (which would alias the
/// running `&mut VM`). Returns `None` for a name that is not one of them, so
/// the caller falls through to external lookup.
///
/// !!! Keep in sync with the matching `vm.register_builtin(...)` closures in
/// `register_builtins`: both must route a name to the same executor method.
pub(crate) fn try_run_registered_builtin(name: &str, argv: &[String]) -> Option<i32> {
    let s = match name {
        "async" => with_executor(|e| e.builtin_async(argv)),
        "await" => with_executor(|e| e.builtin_await(argv)),
        "barrier" => with_executor(|e| e.builtin_barrier(argv)),
        "peach" => with_executor(|e| e.builtin_peach(argv)),
        "pmap" => with_executor(|e| e.builtin_pmap(argv)),
        "pgrep" => with_executor(|e| e.builtin_pgrep(argv)),
        "intercept" => with_executor(|e| e.builtin_intercept(argv)),
        "intercept_proceed" => with_executor(|e| e.builtin_intercept_proceed(argv)),
        "doctor" => with_executor(|e| e.builtin_doctor(argv)),
        "dbview" => with_executor(|e| e.builtin_dbview(argv)),
        "profile" => with_executor(|e| e.builtin_profile(argv)),
        "caller" => with_executor(|e| e.builtin_caller(argv)),
        "help" => with_executor(|e| e.builtin_help(argv)),
        "cdreplay" => with_executor(|e| e.builtin_cdreplay(argv)),
        "zsleep" => crate::extensions::ext_builtins::zsleep(argv),
        _ => return None,
    };
    Some(s)
}

pub(crate) fn dispatch_builtin_raw(name: &str, args: Vec<String>) -> i32 {
    // !!! WARNING: RUST-ONLY — NO C COUNTERPART !!!
    // Native p10k engine intercept (src/extensions/p10k): sourcing
    // powerlevel10k.zsh-theme activates the Rust segment engine
    // instead of executing the ~13k-line zsh theme. The user's
    // `.p10k.zsh` CONFIG is NOT intercepted — it sources normally so
    // its POWERLEVEL9K_* typesets land in the paramtab, which the
    // engine reads live at every render. Placed here (the chokepoint
    // every builtin route funnels through) so `source`, `.`, and
    // `builtin source` all hit it.
    if matches!(name, "source" | ".") {
        if let Some(status) = crate::p10k::maybe_intercept_theme_source(&args) {
            // Register a `p10k` stub function so `${+functions[p10k]}`
            // guards in .zshrc templates stay truthy. The body forwards
            // to the bridge-intercepted `zshrs-p10k-api` name so
            // `p10k segment` (custom-segment protocol) and the other
            // API subcommands reach the native engine (p10k_api).
            try_with_executor(|exec| {
                let _ = exec.execute_script("function p10k() { zshrs-p10k-api \"$@\" }");
            });
            return status;
        }
    }
    // Native p10k API dispatch — the `p10k` stub function forwards
    // here (see the theme intercept above). Must run before the
    // generic builtintab lookup: the name is not a real builtin.
    if let Some(status) = crate::p10k::maybe_intercept_command(name, &args) {
        return status;
    }
    // c:Src/exec.c:2700-2717 — `private` is an autoloaded builtin in
    // zsh (autofeature b:private of zsh/param/private): first use
    // runs ensurefeature → require_module → load_module → boot_,
    // marking the module MOD_INIT_B (what `zmodload -e` reads) and
    // installing the wrap_private FuncWrap (param_private.c:712).
    // doshfunc gates the wrapper dispatch on that load state, so
    // this require_module is what activates private scoping. The
    // raw dispatcher is the chokepoint every builtin route funnels
    // through; require_module is idempotent after the first call
    // (needs_load checks MOD_INIT_B).
    if name == "private" {
        if let Ok(mut tab) = crate::ported::module::MODULESTAB.lock() {
            let _ = crate::ported::module::require_module(&mut tab, "zsh/param/private", None, 0, false);
            // c:2710 ensurefeature
        }
    }
    // c:Src/Modules/param_private.c:682-685 setup_ — loading
    // zsh/param/private REPLACES the `local` builtintab node's
    // handlerfunc + optstr with bin_private's ("Even more horrible
    // hack"), so once the module is loaded `local` IS bin_private: it
    // accepts the -P/-Pa private-scope flags, and without -P delegates
    // to bin_typeset, which already treats `local` and `private`
    // identically (is_locallike, builtin.rs:3666). Replicate the swap by
    // routing `local` through the `private` node only after the module
    // is loaded — before then, `local -P` still errors "bad option: -P"
    // exactly like stock zsh. The `private` node carries the augmented
    // optstr (with P) that the `local` node lacks.
    if name == "local"
        && crate::ported::module::MODULESTAB
            .lock()
            .map(|t| t.is_bound("zsh/param/private"))
            .unwrap_or(false)
    {
        return dispatch_builtin_raw("private", args);
    }
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
    // c:Src/exec.c:2700-2724 resolvebuiltin — autoloaded-builtin stub
    // (registered by `zmodload -ab MOD NAME`, Src/module.c:426
    // add_autobin) fires on first use: ensurefeature loads the owning
    // module, then dispatch proceeds against the real builtin. Must
    // run BEFORE the module-bound 127 gate below — `zmodload -ab
    // zsh/zselect zselect; zselect` previously died there with
    // `command not found` because the gate only checked is_loaded,
    // never the autoload ledger.
    if let Some(rc) = crate::ported::module::resolvebuiltin(name) {
        if rc != 0 {
            // Load failed or feature undefined — diagnostics already
            // printed (load_module zwarn / resolvebuiltin zerr).
            // C's execbuiltin head returns 1 (Src/builtin.c:264-267).
            return 1;
        }
        // Module loaded — fall through; the is_loaded gates below now
        // pass and the normal dispatch chain runs the real builtin.
    }
    // c:Src/Modules/<mod>.c boot_/setup_ chain — module-bound builtins
    // (zftp, zsocket, ztcp, zstat, etc.) are only registered into
    // `builtintab` when their module is loaded via `zmodload`. In
    // zsh `-fc` (the parity test harness's invocation), the modules
    // are NOT pre-loaded, so each name reports "command not found"
    // with exit 127. zshrs intentionally pre-loads all module bintabs
    // in `createbuiltintable` (builtin.rs:131-152) for the default
    // mode so users can call these without `zmodload`; that auto-load
    // diverges from zsh's gate behavior. Match zsh's stance only when
    // the user explicitly asked for parity via `--zsh`.
    //
    // The list is the union of builtins from modules that zsh does
    // NOT auto-load (verified via `zsh -fc <name>` returning 127):
    //   zsh/zftp          → zftp
    //   zsh/net/socket    → zsocket
    //   zsh/net/tcp       → ztcp
    //   zsh/stat          → zstat (NOT `stat`; that name resolves to
    //                              /bin/stat on PATH per zsh's setup)
    //   zsh/zselect       → zselect
    //   zsh/zpty          → zpty
    //   zsh/zprof         → zprof
    //   zsh/system        → zsystem, syserror
    //   zsh/clone         → clone
    //   zsh/curses        → zcurses
    //   zsh/db/gdbm       → ztie, zuntie, zgdbmpath
    //   zsh/pcre          → pcre_compile, pcre_match, pcre_study
    //   zsh/example       → example
    //   zsh/cap           → cap, getcap, setcap
    //   zsh/attr          → zgetattr, zsetattr, zdelattr, zlistattr
    if crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed)
        && module_bound_builtin_module(name)
            .map(|m| {
                !crate::ported::module::MODULESTAB
                    .lock()
                    .map(|t| t.is_loaded(m))
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    {
        eprintln!("zsh:1: command not found: {}", name);
        let _ = args;
        return 127;
    }
    // c:Src/Modules/files.c:806-824 — zsh/files registers `chmod`,
    // `chown`, `chgrp`, `ln`, `mkdir`, `mv`, `rm`, `rmdir`, `sync`
    // (plus their `zf_*` aliases) into builtintab on module load.
    // Without an explicit `zmodload zsh/files`, zsh resolves the
    // names through PATH lookup — `zsh -fc 'chmod +x f'` runs
    // `/bin/chmod`, whose argv-parser accepts symbolic modes like
    // `+x` that bin_chmod's octal-only parser rejects with
    // "invalid mode `+x'". The shadow-aware wrapper at
    // `dispatch_builtin` (line 438) already has this gate, but the
    // direct `dispatch_builtin_raw` path used by fusevm's
    // CallBuiltin opcode bypasses it. Mirror the gate here so the
    // low-level dispatch matches C's PATH-fall-through behavior.
    // The gate is NOT emulation-mode dependent. C has no `zsh/files`
    // builtins in `builtintab` until the module is loaded, in ANY mode, so a
    // bare `rm`/`mv`/`chmod` falls through to PATH — `chmod +x FILE` runs
    // /bin/chmod and succeeds. Conditioning this on `IS_ZSH_MODE` meant the
    // native binary answered `chmod +x` from `bin_chmod`'s octal-only parser
    // ("invalid mode `+x'"), and `rm -s` / `mv -s` from the module's argument
    // parser instead of the system tool's. `dispatch_builtin` at :1087 already
    // gates unconditionally; this low-level `CallBuiltin` path did not.
    if module_gated_files_builtin(name)
        && !crate::ported::module::MODULESTAB
            .lock()
            .map(|t| t.is_loaded("zsh/files"))
            .unwrap_or(false)
    {
        // PATH lookup uses the LITERAL name: bare `mkdir` finds
        // /bin/mkdir; a `zf_*` alias finds nothing and exits 127 —
        // matching zsh -fc `zf_mkdir d` → "command not found:
        // zf_mkdir" (the aliases exist ONLY in the loaded module's
        // builtintab, Src/Modules/files.c:816-824; PATH has no
        // /bin/zf_rm). The previous zf_-strip silently ran the
        // system binary instead.
        let status = with_executor(|exec| exec.execute_external(name, &args, &[])).unwrap_or(127);
        return status;
    }
    // c:Src/Modules/stat.c:637-638 — zsh/stat registers BOTH `stat`
    // and `zstat`. `zstat` is in the module_bound 127-gate above (no
    // /usr/bin/zstat exists), but the bare `stat` name must FALL
    // THROUGH to PATH when zsh/stat isn't loaded — zsh -fc
    // 'stat -f %Lp f' runs /usr/bin/stat, while bin_stat's parser
    // rejects stat(1) flags ("bad option: -c"). Same fall-through
    // shape as the zsh/files gate above.
    if crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed)
        && name == "stat"
        && !crate::ported::module::MODULESTAB
            .lock()
            .map(|t| t.is_loaded("zsh/stat"))
            .unwrap_or(false)
    {
        let status = with_executor(|exec| exec.execute_external(name, &args, &[])).unwrap_or(127);
        return status;
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
        "mkdir"
            | "rmdir"
            | "rm"
            | "mv"
            | "ln"
            | "chmod"
            | "chown"
            | "chgrp"
            | "sync"
            | "zf_mkdir"
            | "zf_rmdir"
            | "zf_rm"
            | "zf_mv"
            | "zf_ln"
            | "zf_chmod"
            | "zf_chown"
            | "zf_chgrp"
            | "zf_sync"
    )
}

pub(crate) fn dispatch_builtin(name: &str, args: Vec<String>) -> i32 {
    // c:Src/exec.c getproc + Src/jobs.c deletefilelist — close any
    // `>(cmd)` write ends owned by this command once it finishes
    // (drops on every return path below).
    let _psub_fds = PsubFdGuard;
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
        // c:Src/exec.c:4367-4386 — POSIX special-builtin escalation:
        // a failed redirect on a PSPECIAL builtin (set, readonly,
        // typeset, ...) under POSIX_BUILTINS is FATAL in a
        // non-interactive shell (`exit(1)` at c:4383). The `command`
        // prefix resets this (BINF_COMMAND, c:4369) — that path
        // dispatches through bin_command, not here.
        if crate::ported::zsh_h::isset(crate::ported::zsh_h::POSIXBUILTINS)
            && !crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE)
            && builtin_is_pspecial(name)
        {
            use std::sync::atomic::Ordering;
            crate::ported::builtin::EXIT_VAL.store(1, Ordering::Relaxed);
            crate::ported::builtin::EXIT_PENDING.store(1, Ordering::Relaxed);
        }
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
    consume_tilde_globsubst_carrier();
    let glob_failed = with_executor(|exec| {
        let f = exec.current_command_glob_failed.get();
        exec.current_command_glob_failed.set(false); // c:1879 cleanup
        f
    });
    if glob_failed {
        // c:Src/glob.c:1876-1880 + Src/exec.c — NOMATCH zerr sets
        // ERRFLAG_ERROR (via utils.c:184). For a BUILTIN command the
        // expansion runs IN the shell process, so errflag stays set
        // and the rest of the input aborts (zsh -fc 'echo /nope_*;
        // echo after' prints nothing after the error — verified
        // against zsh 5.9). The continue-after-nomatch behaviour
        // belongs ONLY to externals: C forks BEFORE expansion there,
        // so the child's zerr can't touch the parent's errflag (zsh
        // -fc 'ls /nope_*; echo after' prints `after`) — that path's
        // clear lives in fn exec / execute_external. Leave
        // ERRFLAG_ERROR set here; BUILTIN_ERREXIT_CHECK trigger 4
        // aborts the remaining script at the next command boundary.
        return 1; // c:1880 — command aborted, status 1
    }
    // c:Src/subst.c:505-507 — CSH_NULL_GLOB sibling of the NOMATCH
    // gate above: all of this command's globs failed silently (words
    // dropped, badcshglob accumulated 1s and no 2s) → `no match`,
    // skip the builtin, status 1. Like the NOMATCH path, ERRFLAG
    // from zerr stays set for builtins so the rest of the script
    // aborts (zsh -fc 'setopt cshnullglob; print *nope* x; print
    // after' prints only the error — verified zsh 5.9.1).
    if consume_badcshglob() {
        // c:Src/exec.c:3380 — `lastval = 1;` so the shell's final
        // exit status reflects the aborted command.
        with_executor(|exec| exec.set_last_status(1));
        return 1;
    }
    // c:Src/exec.c:4162-4295 — assignment-builtin (BINF_ASSIGN family:
    // typeset / declare / local / export / readonly / integer / float /
    // private) whose `name=value` postassign arg raised errflag while
    // its RHS was preforked (PREFORK_ASSIGN, c:4239-4245) — the classic
    // case is a math error in `typeset -F fv=$((1/0))`. The postassign
    // loop `break`s on errflag (c:4243) and then `if (!errflag)
    // execbuiltin(...)` (c:4287) SKIPS the builtin entirely, so `lastval`
    // is left UNCHANGED from before the command (0 fresh, 1 after
    // `false`). This differs from a PLAIN assignment `x=$((1/0))`, which
    // goes through execsimple c:1375 `lv = errflag ? errflag : cmdoutval`
    // → 1, and from a NON-assign builtin `print $((1/0))`, whose main
    // args-prefork errflag lands on c:3760 `lastval = 1`. Only the
    // assignment-BUILTIN postassign path preserves the prior status.
    // Mirror it here: the fusevm reg_passthru dispatch still calls us
    // with errflag set (unlike C's pre-invoke gate), so consume that
    // state and return the prior LASTVAL instead of running the builtin.
    {
        use std::sync::atomic::Ordering;
        let live = crate::ported::utils::errflag.load(Ordering::Relaxed);
        let ef = live & crate::ported::zsh_h::ERRFLAG_ERROR;
        let hard = live & crate::ported::zsh_h::ERRFLAG_HARD;
        // Only the SOFT recoverable error (math failure like `$((1/0))`,
        // ERRFLAG_ERROR without ERRFLAG_HARD) preserves the prior status
        // per c:4287. A HARD error (`${var?msg}`, which c:Src/subst.c
        // OR's ERRFLAG_HARD onto errflag) is a script-abort that yields
        // status 1 regardless of the prior status — leave that to the
        // normal dispatch/abort path below (which returns 1 and keeps
        // ERRFLAG_HARD set for the downstream errexit gate).
        if ef != 0 && hard == 0 && builtin_is_assign_family(name) {
            // c:4287 — execbuiltin skipped; lastval unchanged.
            return crate::ported::builtin::LASTVAL.load(Ordering::Relaxed);
        }
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
        let status = with_executor(|exec| exec.execute_external(name, &args, &[])).unwrap_or(127);
        crate::ported::builtin::LASTVAL.store(status, std::sync::atomic::Ordering::Relaxed);
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
        if !crate::ported::module::MODULESTAB
            .lock()
            .unwrap()
            .is_loaded("zsh/files")
        {
            // PATH lookup uses the literal name. In --zsh parity mode
            // `zf_rm` must 127 like zsh -fc (no /bin/zf_rm); default
            // zshrs mode keeps the convenience zf_-strip so the alias
            // still reaches the system binary.
            let path_name = if crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) {
                name
            } else {
                name.strip_prefix("zf_").unwrap_or(name)
            };
            let status =
                with_executor(|exec| exec.execute_external(path_name, &args, &[])).unwrap_or(127);
            crate::ported::builtin::LASTVAL.store(status, std::sync::atomic::Ordering::Relaxed);
            let mut synth = crate::ported::zsh_h::job::default();
            crate::ported::jobs::waitonejob(&mut synth);
            return status;
        }
    }
    // c:Src/exec.c:3997 `int q = queue_signal_level();`
    // c:Src/exec.c:4231 `dont_queue_signals();`
    // c:Src/exec.c:4243 `restore_queue_signals(q);`
    //
    // C runs EVERY builtin with signal queueing switched OFF. Two
    // consequences the zshrs port was missing:
    //
    //   1. `dont_queue_signals()` DRAINS the pending queue (it calls
    //      run_queued_signals()), so a signal that arrived while an
    //      enclosing scope held queue_signals() — doshfunc holds one
    //      for the whole call, c:Src/exec.c:5835 — fires its trap at
    //      the NEXT command boundary rather than at function exit.
    //   2. While the builtin runs, queueing stays off, so a signal the
    //      builtin sends to itself (`kill -USR1 $$`) dispatches the
    //      trap synchronously inside the builtin — which is why zsh
    //      prints pre/trap/post for
    //      `f() { print pre; kill -USR1 $$; print post }`.
    //
    // Without this bracket every trap raised inside a function was
    // deferred to the enclosing unqueue_signals() (i.e. script end).
    // c:Src/exec.c:3546 — `setunderscore((args && nonempty(args)) ?
    // ((char *) getdata(lastnode(args))) : "");`. execcmd_exec sets `$_`
    // to the last word of the command it is ABOUT to run — after the
    // words were expanded, before the builtin/external executes — so a
    // builtin that READS `_` at run time (`typeset -p _`, `${(P)…}`,
    // `$parameters[_]`) sees its own last argument, not the previous
    // command's. zshrs only did this for a handful of builtins (echo,
    // print, true, false, `:`) and for the external/function paths;
    // every reg_passthru! builtin was left reading the stale value.
    // C's `args` list carries argv[0], so a bare `cat` sets `_=cat` —
    // hence the fallback to `name` when there are no arguments.
    let underscore = args.last().cloned().unwrap_or_else(|| name.to_string()); // c:3546
    crate::ported::params::set_zunderscore(std::slice::from_ref(&underscore)); // c:3546
    let q = crate::ported::signals_h::queue_signal_level(); // c:3997
    crate::ported::signals_h::dont_queue_signals(); // c:4231
    let status = dispatch_builtin_raw(name, args);
    crate::ported::signals_h::restore_queue_signals(q); // c:4243
                                                        // c:Src/jobs.c:1748 waitonejob — canonical single-command pipestats update.
    crate::ported::builtin::LASTVAL.store(status, std::sync::atomic::Ordering::Relaxed);
    let mut synth = crate::ported::zsh_h::job::default();
    crate::ported::jobs::waitonejob(&mut synth);
    // c:Src/exec.c:4367-4386 — done: tail. A PSPECIAL builtin that
    // raised errflag under POSIX_BUILTINS exits the non-interactive
    // shell with status 1 ("hard error in POSIX" — e.g. bin_dot's
    // zerrnam at Src/builtin.c:6133). Arm the deferred-exit pair so
    // the next ERREXIT_CHECK unwinds; EXIT_VAL=1 matches C's
    // hardcoded exit(1), NOT the builtin's own status (dot returns
    // 127 but POSIX exits 1).
    if crate::ported::zsh_h::isset(crate::ported::zsh_h::POSIXBUILTINS)
        && !crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE)
        && builtin_is_pspecial(name)
        && (crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed)
            & crate::ported::zsh_h::ERRFLAG_ERROR)
            != 0
    {
        use std::sync::atomic::Ordering;
        crate::ported::builtin::EXIT_VAL.store(1, Ordering::Relaxed);
        crate::ported::builtin::EXIT_PENDING.store(1, Ordering::Relaxed);
    }
    status
}

/// c:Src/zsh.h:1467 BINF_PSPECIAL — true when `name` is a POSIX
/// special builtin per the canonical builtin table flags
/// (Src/builtin.c:48-129: `.`, `:`, break, continue, declare, eval,
/// exit, export, float, integer, local, readonly, return, set,
/// shift, source, times, trap, typeset, unset).
fn builtin_is_pspecial(name: &str) -> bool {
    crate::ported::builtin::createbuiltintable()
        .get(name)
        .map(|b| (b.node.flags as u32 & crate::ported::zsh_h::BINF_PSPECIAL) != 0)
        .unwrap_or(false)
}

/// c:Src/zsh.h:1486 BINF_ASSIGN — the assignment-builtin family
/// (typeset / declare / local / export / readonly / integer / float /
/// private). Their `name=value` args are handled as postassigns
/// (c:Src/exec.c:4162-4295), whose errflag-abort skips execbuiltin and
/// preserves the prior `lastval`. Read the flag straight from the
/// builtin table (same pattern as `builtin_is_pspecial`).
fn builtin_is_assign_family(name: &str) -> bool {
    crate::ported::builtin::createbuiltintable()
        .get(name)
        .map(|b| (b.node.flags as u32 & crate::ported::zsh_h::BINF_ASSIGN) != 0)
        .unwrap_or(false)
}

// The former `install_exec_hooks()` fn-pointer registry is gone. Code
// under `src/ported/` now reaches `ShellExecutor` operations
// (array/assoc storage, script eval, function dispatch, command
// substitution) through the `crate::ported::exec::*` accessor wrappers,
// which resolve the live executor via `try_with_executor`
// (`CURRENT_EXECUTOR`). The bridge lives in exec.rs — the sanctioned
// fusevm-access exception — per `feedback_no_exec_script_from_ported` /
// `feedback_no_shellexecutor_in_ported`.

/// Register all zsh builtins with the VM.
pub(crate) fn register_builtins(vm: &mut fusevm::VM) {
    // src/ported/ reaches the live executor (param store, function
    // dispatch, nested script/cmdsubst exec) through the
    // `crate::ported::exec::*` accessor wrappers, which read
    // `CURRENT_EXECUTOR` via `try_with_executor`. No install step is
    // needed: the executor is in scope for the duration of any VM run
    // (set by `ExecutorContext::enter`), so the wrappers resolve it
    // directly. (Replaces the former `exec_hooks` OnceLock fn-ptr
    // registry, now deleted.)
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
                // c:Src/exec.c getproc + Src/jobs.c deletefilelist —
                // close `>(cmd)` write ends owned by this command
                // once it finishes (shadows bypass dispatch_builtin
                // and ZshrsHost::exec, so they need their own guard:
                // `tee >(wc -c) </dev/null` left wc blocked).
                let _psub_fds = PsubFdGuard;
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
                // c:Src/exec.c:3545-3547 — these shadows stand in for
                // EXTERNAL commands (`cat`, `head`, …), which in zsh reach
                // execcmd_exec and set `$_` to the command's last word
                // before running. Both arms below bypass dispatch_builtin
                // AND execute_external_bg (the shadow runs in-process; the
                // opt-out arm spawns through exec_system_command), so
                // without this `cat f; print $_` reported the PREVIOUS
                // command's last argument.
                {
                    let last = args.last().cloned().unwrap_or_else(|| $name.to_string());
                    crate::ported::params::set_zunderscore(std::slice::from_ref(&last));
                    // c:3546
                }
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
                let args = pop_args(vm, argc);
                // function > builtin: a same-named user function wins over
                // the builtin on the normal (CallBuiltin) invocation path.
                // The compiler's `user_function_shadow` already routes the
                // same-compile-unit case through CallFunction; this probe
                // extends that to the cross-unit / interactive case (define
                // `zstyle() { … }` on one line, call it on the next). The
                // forced `builtin NAME` / `command NAME` paths dispatch
                // through their own handlers, not this one, so they still
                // reach the builtin as required.
                if let Some(s) = try_user_fn_override($name, &args) {
                    return Value::Status(s);
                }
                Value::Status(dispatch_builtin($name, args))
            });
        };
    }

    // zshrs-original extension builtins (async / peach / doctor / …) that
    // route to an ExecutorContext method. Like `reg_overridable!`, they
    // probe `try_user_fn_override` FIRST so a user function of the same
    // name wins — zsh's alias → function → builtin dispatch order. Without
    // the probe, `doctor() { … }; doctor` silently ran the builtin and
    // ignored the function (function > builtin violated for these).
    macro_rules! reg_ext_overridable {
        ($vm:expr, $id:expr, $name:literal, $method:ident) => {
            $vm.register_builtin($id, |vm, argc| {
                let args = pop_args(vm, argc);
                if let Some(s) = try_user_fn_override($name, &args) {
                    return Value::Status(s);
                }
                Value::Status(with_executor(|exec| exec.$method(&args)))
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
        // Non-zsh modes (bash drop-in): mapfile / readarray reads lines
        // from stdin (or `-u fd`) into an array. Handled by the ported
        // bash builtin in ext_builtins.
        Value::Status(crate::extensions::ext_builtins::readarray(&args))
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
                                  // c:Src/builtin.c:6209 — `execode(prog, 1, 0, "eval");`. execode
                                  // (c:Src/exec.c:1245-1266) APPENDS its context argument to
                                  // `zsh_eval_context` for the duration of the body, so code inside
                                  // `eval` sees `cmdarg:eval` (and `cmdarg:shfunc:eval` when the eval is
                                  // in a function). zshrs pushed "shfunc" and, since #1065, "cmdsubst",
                                  // but never "eval". Popped on every return path by the guard, matching
                                  // execode's stack discipline. Bug #1065 (eval leg).
        let _eval_ctx_guard = crate::ported::exec::EvalContextFrame::push("eval");
        // c:Src/builtin.c:6163-6178 — `eval` pushes a funcstack frame named
        // "(eval)" (tp = FS_EVAL), gated on `ineval = !isset(EVALLINENO)` /
        // `if (!ineval)` — i.e. pushed when EVAL_LINENO is SET, which is the
        // zsh default. zshrs already set `scriptname = "(eval)"` (below) but
        // never pushed the frame, so `eval '…${#funcstack}'` reported 0 where
        // zsh reports 1, and inside a function `${(j:,:)funcstack}` was `f`
        // rather than `(eval),f`. Both shells already agreed under
        // `unsetopt evallineno` (no frame), so the option gate is load-bearing
        // and is mirrored here. Popped on every return path by the guard.
        // Bug #1066.
        let eval_pushed_frame = crate::ported::zsh_h::isset(crate::ported::zsh_h::EVALLINENO);
        if eval_pushed_frame {
            let caller = {
                let stk = crate::ported::modules::parameter::FUNCSTACK
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                stk.last()
                    .map(|f| f.name.clone()) // c:6167 funcstack->name
                    .or_else(|| crate::ported::utils::argzero()) // c:6167 argzero
            };
            let frame = crate::ported::zsh_h::funcstack {
                prev: None,                 // c:6166 (Vec-stack: index encodes link)
                name: "(eval)".to_string(), // c:6166 fstack.name = scriptname
                filename: None,
                caller,
                flineno: 0,
                lineno: 0,                         // c:6169
                tp: crate::ported::zsh_h::FS_EVAL, // c:6170
            };
            crate::ported::modules::parameter::FUNCSTACK
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(frame); // c:6178 funcstack = &fstack
        }
        struct EvalFuncstackGuard(bool);
        impl Drop for EvalFuncstackGuard {
            fn drop(&mut self) {
                if self.0 {
                    crate::ported::modules::parameter::FUNCSTACK
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .pop();
                }
            }
        }
        let _eval_fs_guard = EvalFuncstackGuard(eval_pushed_frame);
        let oscriptname = crate::ported::utils::scriptname_get();
        crate::ported::utils::set_scriptname(Some("(eval)".to_string()));
        // Recursion backstop — c:Src/jobs.c:1878-1884. zsh caps eval recursion
        // via its job table (every eval'd pipeline grabs a job slot; the table
        // caps at MAX_MAXJOBS → "job table full or recursion limit exceeded").
        // The fusevm runtime allocates no job per pipeline, and nested evals
        // push no funcstack frame (INEVAL suppression, c:6164), so eval nesting
        // is invisible to both the job table AND FUNCNEST/FUNCSTACK — runaway
        // `eval`-string recursion overflowed the 256 MB main-thread stack →
        // uncatchable SIGBUS. Track eval re-entry depth (the Rust proxy for
        // held job slots) and refuse at the same MAX_MAXJOBS ceiling.
        let eval_depth = crate::vm_helper::EVAL_RECURSION_DEPTH.with(|d| {
            let v = d.get() + 1;
            d.set(v);
            v
        });
        let mut status = if eval_depth >= crate::ported::jobs::MAX_MAXJOBS {
            crate::ported::utils::zerr("job table full or recursion limit exceeded");
            1
        } else {
            with_executor(|exec| {
                // c:6175 execode
                exec.execute_script(&src).unwrap_or(1)
            })
        };
        crate::vm_helper::EVAL_RECURSION_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        // c:Src/builtin.c:6211-6212 — `if (errflag && !lastval)
        //   lastval = errflag;`
        // c:Src/builtin.c:6221 — `errflag &= ~ERRFLAG_ERROR;`
        // eval is a CONTAINMENT boundary: an error inside the eval
        // body (readonly reassign, bad assoc set, ${unset?msg}, …)
        // breaks the eval body's lists via errflag, then eval clears
        // the flag and returns lastval, and the CALLER's next list
        // runs. zsh 5.9: `eval 'assoc=(odd)'; echo "after $?"`
        // prints `after 1` in -c, script, and stdin contexts.
        {
            use std::sync::atomic::Ordering;
            let ef = crate::ported::utils::errflag.load(Ordering::Relaxed)
                & crate::ported::zsh_h::ERRFLAG_ERROR;
            if ef != 0 && status == 0 {
                status = ef; // c:6212 lastval = errflag
            }
            crate::ported::utils::errflag
                .fetch_and(!crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
        }
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
        // zshrs extension builtins (daemon z* family: zd, zcache, zjob,
        // …) are dispatched by name via try_dispatch instead of living
        // in builtintab — but they ARE builtins, so the `builtin`
        // precommand must reach them (`builtin zd ping` errored
        // "no such builtin: zd" while bare `zd ping` worked).
        if crate::daemon::builtins::is_zshrs_builtin(name) {
            let argv: Vec<String> = std::iter::once(name.to_string())
                .chain(rest.iter().cloned())
                .collect();
            return Value::Status(crate::daemon::builtins::try_dispatch(name, &argv).unwrap_or(1));
        }
        // c:Src/exec.c:3435-3436 — `builtin NAME` with NAME not in
        // builtintab emits `zwarn("no such builtin: %s", cmdarg)`
        // and returns 1. zshrs's dispatch_builtin_raw bare-returned 1
        // silently. Probe the table here so the diagnostic fires
        // before dispatch.
        let tab = crate::ported::builtin::createbuiltintable();
        if !tab.contains_key(name.as_str()) {
            // zshrs-original opcode builtins (async, doctor, peach, …) aren't
            // in builtintab; `builtin NAME` must still reach them.
            if let Some(status) = try_run_registered_builtin(name, rest) {
                return Value::Status(status);
            }
            // c:Src/exec.c:3436 — `zwarn("no such builtin: %s", cmdarg);`.
            // Route through the ported `zwarn` rather than formatting the
            // prefix by hand: zwarn emits zsh's `zsh:LINE:` prefix, and the
            // hand-rolled `eprintln!` here printed `zshrs:1:` instead. Of the
            // twelve error shapes probed this was the ONLY one carrying the
            // wrong prefix — the ported twin at exec.rs:9878 already used
            // zwarn correctly, so this was a reimplementation shadowing a
            // faithful port (same shape as #1027 / #1031 / #1044 / #1050).
            // Bug #1063.
            crate::ported::utils::zwarn(&format!("no such builtin: {}", name));
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
        //
        // command's OWN options end at the first non-flag arg —
        // everything after the command name belongs to IT. The
        // previous `.any()` scan over ALL args stole `-p` from
        // `command mkdir -p DIR` (zconvey.plugin.zsh:44), stripping
        // the flag before /bin/mkdir ran → "File exists" errors on
        // every re-source.
        let mut lead = 0usize;
        let mut dash_p = false;
        let mut kept_flags: Vec<String> = Vec::new();
        for a in post.iter() {
            let s = a.as_str();
            if s == "--" {
                lead += 1;
                break;
            }
            if s.starts_with('-')
                && s.len() >= 2
                && s[1..].chars().all(|c| c == 'p' || c == 'v' || c == 'V')
            {
                if s.contains('p') {
                    dash_p = true;
                }
                // -v / -V drive the whence-style lookup downstream —
                // keep them in post (only the PATH-reset `p` is
                // consumed here).
                let rest: String = s[1..].chars().filter(|c| *c != 'p').collect();
                if !rest.is_empty() {
                    kept_flags.push(format!("-{}", rest));
                }
                lead += 1;
                continue;
            }
            break;
        }
        let mut post: Vec<String> = {
            let mut v = kept_flags;
            v.extend(post[lead..].iter().cloned());
            v
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
            let status =
                with_executor(|exec| exec.dispatch_function_call(&cmd, &rest).unwrap_or(127));
            // Top-level `exec funcname` — exit the shell with the
            // function's status (mirrors C's "exec replaces shell as
            // last act"). Subshell `(exec funcname)` — return through
            // the EXIT_PENDING path so the subshell body aborts and
            // the parent resumes via subshell_end.
            let in_subshell_now = with_executor(|exec| !exec.subshell_snapshots.is_empty());
            if in_subshell_now {
                crate::ported::builtin::EXIT_VAL
                    .store(status, std::sync::atomic::Ordering::Relaxed);
                crate::ported::builtin::EXIT_PENDING.store(1, std::sync::atomic::Ordering::Relaxed);
                return Value::Status(status);
            }
            std::process::exit(status);
        }
        // c:Src/exec.c — builtin path: `exec builtin` runs the
        // builtin in-process and exits.
        let bn_in_tab = crate::ported::builtin::createbuiltintable().contains_key(&cmd);
        if bn_in_tab {
            let status = dispatch_builtin_raw(&cmd, rest.clone());
            let in_subshell_now = with_executor(|exec| !exec.subshell_snapshots.is_empty());
            if in_subshell_now {
                crate::ported::builtin::EXIT_VAL
                    .store(status, std::sync::atomic::Ordering::Relaxed);
                crate::ported::builtin::EXIT_PENDING.store(1, std::sync::atomic::Ordering::Relaxed);
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
            // Queue signals across spawn+wait so the SIGCHLD reaper
            // can't reap this child before child.wait() does — see
            // ForegroundWaitGuard.
            let _wait_guard = ForegroundWaitGuard::enter();
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
            crate::ported::builtin::EXIT_VAL.store(status, std::sync::atomic::Ordering::Relaxed);
            crate::ported::builtin::EXIT_PENDING.store(1, std::sync::atomic::Ordering::Relaxed);
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
                    errmsg = format!("{}{}", c.to_ascii_lowercase(), &errmsg[c.len_utf8()..]);
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

    // Aliases — alias is `BINF_MAGICEQUALS` per Src/builtin.c:50.
    // c:Src/exec.c:3298-3304 — when a builtin has BINF_MAGICEQUALS,
    // execcmd_exec sets esprefork = PREFORK_TYPESET and calls
    // `prefork(args, esprefork, NULL)` on the argv. prefork (subst.c:
    // 100) drives `filesub` on each word (c:133), which (c:677-686)
    // looks for the assignment Equals and runs `filesubstr` on the
    // VALUE side. That's how `alias bad===` triggers equalsubstr's
    // "= not found" via the inner Equals after the first `=`.
    //
    // The fusevm dispatch path doesn't go through execcmd_exec, so
    // BUILTIN_ALIAS previously passed args straight to bin_alias with
    // no expansion — `alias x=~/foo` stored literal `~/foo` (no tilde
    // expand), `alias bad===` stored a broken entry without firing
    // the "= not found" diagnostic. The prefork(PREFORK_TYPESET) runs
    // per arg word via BUILTIN_MAGIC_EQUALS_PREFORK ops that
    // compile_simple emits BEFORE the redirect scope opens (matching
    // c:3304 prefork-before-addfd order), so the dispatch here is a
    // plain passthrough — re-running prefork would double-fire the
    // "= not found" diagnostic.
    reg_passthru!(vm, BUILTIN_ALIAS, "alias");
    // c:Src/exec.c:3298-3304 — per-word magic-equals prefork; see the
    // const doc at BUILTIN_MAGIC_EQUALS_PREFORK. prefork's filesub
    // trigger (subst.c:678 `strchr(*namptr+1, Equals)`) looks for the
    // EQUALS TOKEN, not literal `=`. The fusevm path delivers args
    // already-untokenized, so re-tokenize each element via
    // `shtokenize` (the same call C's lexer makes implicitly when
    // assembling the word) so prefork sees Equals tokens at `=`
    // boundaries and Tilde tokens at `~` starts. After prefork
    // expands, untokenize for storage.
    vm.register_builtin(BUILTIN_MAGIC_EQUALS_PREFORK, |vm, _argc| {
        let raw = vm.pop();
        let inputs: Vec<String> = match raw {
            Value::Array(items) => items.iter().map(|v| v.to_str()).collect(),
            other => vec![other.to_str()],
        };
        let mut as_linklist: crate::ported::linklist::LinkList<String> = Default::default();
        for s in &inputs {
            let mut tokd = s.clone();
            crate::ported::glob::shtokenize(&mut tokd);
            as_linklist.push_back(tokd);
        }
        let mut rf = 0i32;
        crate::ported::subst::prefork(
            &mut as_linklist,
            crate::ported::zsh_h::PREFORK_TYPESET,
            &mut rf,
        );
        let mut expanded: Vec<String> = Vec::with_capacity(inputs.len());
        while let Some(s) = as_linklist.pop_front() {
            expanded.push(crate::ported::lex::untokenize(&s).to_string());
        }
        if expanded.len() == 1 {
            Value::str(expanded.into_iter().next().unwrap())
        } else {
            Value::array(expanded.into_iter().map(Value::str).collect())
        }
    });

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
        // external-command lookup miss — UNLESS a user FUNCTION of
        // that name exists: zsh has no such builtin, so bashcompinit's
        // `compgen() {...}` definition wins the dispatch there. The
        // unconditional 127 shadowed it and broke every
        // bashcompinit-style completion file (zsh-more-completions
        // _msync/_gocomplete/_qshell/_cw), spraying "command not
        // found: complete" at every deferred compinit load.
        if crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            if crate::ported::utils::getshfunc("compgen").is_some() {
                let status = with_executor(|exec| exec.dispatch_function_call("compgen", &args))
                    .unwrap_or(127);
                return Value::Status(status);
            }
            eprintln!("zsh:1: command not found: compgen");
            let _ = args;
            return Value::Status(127);
        }
        let status = with_executor(|exec| exec.builtin_compgen(&args));
        Value::Status(status)
    });

    vm.register_builtin(BUILTIN_COMPLETE, |vm, argc| {
        let args = pop_args(vm, argc);
        // c:Bug #475 — `complete` is a bash-only builtin. Same gate +
        // user-function precedence as BUILTIN_COMPGEN above.
        if crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            if crate::ported::utils::getshfunc("complete").is_some() {
                let status = with_executor(|exec| exec.dispatch_function_call("complete", &args))
                    .unwrap_or(127);
                return Value::Status(status);
            }
            eprintln!("zsh:1: command not found: complete");
            let _ = args;
            return Value::Status(127);
        }
        let status = with_executor(|exec| exec.builtin_complete(&args));
        Value::Status(status)
    });

    reg_passthru!(vm, BUILTIN_COMPADD, "compadd");
    reg_passthru!(vm, BUILTIN_COMPSET, "compset");

    // See the const's doc comment for the contract. Stack (bottom→top):
    // base, e1, …, eN — argc = N + 1.
    vm.register_builtin(BUILTIN_TYPESET_PAREN_PACK, |vm, argc| {
        let mut vals: Vec<Value> = Vec::with_capacity(argc as usize);
        for _ in 0..argc {
            vals.push(vm.pop());
        }
        vals.reverse();
        let mut it = vals.into_iter();
        let mut out = it.next().map(|v| v.to_str()).unwrap_or_default();
        for v in it {
            match v {
                // Array → splice items as separate elements (splat);
                // empty array contributes nothing (empty elision).
                Value::Array(items) => {
                    for item in items.iter() {
                        out.push('\u{1f}');
                        out.push_str(&item.to_str());
                    }
                }
                other => {
                    out.push('\u{1f}');
                    out.push_str(&other.to_str());
                }
            }
        }
        Value::str(out)
    });

    vm.register_builtin(BUILTIN_TYPESET_PAREN_CLOSE, |vm, _argc| {
        let base = vm.pop().to_str();
        Value::str(format!("{}\u{1f})", base))
    });

    vm.register_builtin(BUILTIN_COMPDEF, |vm, argc| {
        let args = pop_args(vm, argc);
        // ACTUALLY A ZSH FUNCTION: compdef is defined by `compinit`, it is
        // never a builtin. Without the completion system set up it is
        // command-not-found (127) in every mode — `zsh -f; compdef` prints
        // "command not found: compdef". A user/compsys `compdef` FUNCTION
        // (autoload compinit → compinit defines compdef) wins and runs the
        // fast native impl; otherwise it's command-not-found. Previously the
        // extension builtin ran in native mode (bare `compdef` → "I need
        // arguments"), diverging from zsh.
        // compinit installs a `compdef` function stub (see
        // NATIVE_COMPDEF_MARKER) purely so `${+functions[compdef]}` is
        // true; route that exact body to the fast native impl instead of
        // dispatching the stub. A genuine user/compsys compdef function
        // (any other body) still wins via try_user_fn_override below.
        let is_native_stub = crate::ported::hashtable::shfunctab_lock()
            .read()
            .ok()
            .and_then(|t| t.get("compdef").and_then(|shf| shf.body.clone()))
            .map(|b| b.trim() == crate::extensions::ext_builtins::NATIVE_COMPDEF_MARKER)
            .unwrap_or(false);
        if is_native_stub {
            return Value::Status(with_executor(|exec| exec.builtin_compdef(&args)));
        }
        if let Some(s) = try_user_fn_override("compdef", &args) {
            return Value::Status(s);
        }
        if with_executor(|exec| exec.function_exists("compdef")) {
            return Value::Status(with_executor(|exec| exec.builtin_compdef(&args)));
        }
        eprintln!("zsh:1: command not found: compdef");
        Value::Status(127)
    });

    vm.register_builtin(BUILTIN_COMPINIT, |vm, argc| {
        let args = pop_args(vm, argc);
        // ACTUALLY A ZSH FUNCTION: compinit is a contrib FUNCTION (autoloaded
        // from $fpath), never a builtin. Without `autoload -Uz compinit` it is
        // command-not-found
        // (127) in every mode — `zsh -f; compinit` prints
        // "command not found: compinit". zshrs previously ran its builtin
        // unconditionally, so bare `compinit` succeeded. Gate on a compinit
        // function entry existing (which `autoload -Uz compinit` creates);
        // once the user has autoloaded/defined it, run zshrs's implementation.
        if !with_executor(|exec| exec.function_exists("compinit")) {
            eprintln!("zsh:1: command not found: compinit");
            let _ = args;
            return Value::Status(127);
        }
        Value::Status(with_executor(|exec| exec.builtin_compinit(&args)))
    });

    reg_ext_overridable!(vm, BUILTIN_CDREPLAY, "cdreplay", builtin_cdreplay);

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
        let args = pop_args(vm, argc);
        // function > builtin: a user `zsleep() { … }` wins.
        if let Some(s) = try_user_fn_override("zsleep", &args) {
            return Value::Status(s);
        }
        Value::Status(crate::extensions::ext_builtins::zsleep(&args))
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
        // ACTUALLY A ZSH FUNCTION: promptinit is a contrib FUNCTION
        // (autoloaded from $fpath), never a builtin. Command-not-found until
        // `autoload -Uz promptinit`; once autoloaded, run the native impl.
        if !with_executor(|exec| exec.function_exists("promptinit")) {
            eprintln!("zsh:1: command not found: promptinit");
            let _ = args;
            return Value::Status(127);
        }
        Value::Status(crate::extensions::ext_builtins::promptinit(&args))
    });

    vm.register_builtin(BUILTIN_PROMPT, |vm, argc| {
        let args = pop_args(vm, argc);
        Value::Status(crate::extensions::ext_builtins::prompt(&args))
    });

    // Async / Parallel (zshrs extensions) — all overridable by a
    // same-named user function (function > builtin).
    reg_ext_overridable!(vm, BUILTIN_ASYNC, "async", builtin_async);
    reg_ext_overridable!(vm, BUILTIN_AWAIT, "await", builtin_await);
    reg_ext_overridable!(vm, BUILTIN_PMAP, "pmap", builtin_pmap);
    reg_ext_overridable!(vm, BUILTIN_PGREP, "pgrep", builtin_pgrep);
    reg_ext_overridable!(vm, BUILTIN_PEACH, "peach", builtin_peach);
    reg_ext_overridable!(vm, BUILTIN_BARRIER, "barrier", builtin_barrier);

    // Intercept (AOP)
    reg_ext_overridable!(vm, BUILTIN_INTERCEPT, "intercept", builtin_intercept);
    reg_ext_overridable!(
        vm,
        BUILTIN_INTERCEPT_PROCEED,
        "intercept_proceed",
        builtin_intercept_proceed
    );

    // Debug / Profile
    reg_ext_overridable!(vm, BUILTIN_DOCTOR, "doctor", builtin_doctor);
    reg_ext_overridable!(vm, BUILTIN_DBVIEW, "dbview", builtin_dbview);
    reg_ext_overridable!(vm, BUILTIN_PROFILE, "profile", builtin_profile);

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

        // c:Src/exec.c — every pipeline stage forks from the current
        // shell state, so each stage observes the pre-pipeline $? until
        // it runs its own command. Stage sub-VMs start fresh with
        // last_status=0, so seed them with the parent's lastval; without
        // this `false; echo $? | cat` prints 0 instead of zsh's 1.
        let parent_status = vm.last_status;

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
            stage_vm.last_status = parent_status;
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
                    // c:Src/exec.c:2862 → 1219 — pipeline children run
                    // entersubsh with ESUB_PGRP, which clears the job
                    // table (clearjobtab, Src/jobs.c:1780). Without
                    // this, `sleep 5 & jobs -p | wc -l` reports 1 in
                    // the forked stage where zsh reports 0. The fork
                    // already copy-isolates the statics, so mutating
                    // them here can't leak to the parent.
                    with_executor(|exec| {
                        let monitor =
                            crate::ported::zsh_h::isset(crate::ported::zsh_h::MONITOR) as i32;
                        crate::ported::jobs::clearjobtab(&mut exec.jobs, monitor);
                    });
                    *crate::ported::jobs::THISJOB
                        .get_or_init(|| std::sync::Mutex::new(-1))
                        .lock()
                        .unwrap() = -1;
                    // c:Src/exec.c:3720-3724 — the stage's own fds go
                    // onto 0/1 only AFTER its argument words have been
                    // expanded (prefork c:3304 / globlist c:3702), so
                    // park them and let the stage chunk's
                    // BUILTIN_PIPE_FDS_INSTALL do the dup2 at the
                    // C-faithful point. `print -rl -- c a b |
                    // print -r -- "[$(cat)]" | cat` therefore prints
                    // `[]` — the middle stage's `$(cat)` reads the
                    // shell's stdin, not the pipe.
                    let in_fd = if i > 0 { pipes[i - 1].0 } else { -1 };
                    let out_fd = pipes[i].1;
                    // (Pipe-output MULTIOS marking — c:Src/exec.c:3724 —
                    // is emitted INTO the stage chunk by compile_pipe
                    // via BUILTIN_PIPE_OUTPUT_MARK, gated on the stage's
                    // top-level command actually carrying redirects, so
                    // a nested `{ echo a > f; } | cat` body redirect
                    // does not wrongly join the pipe.)
                    // Close every pipe fd this stage doesn't need. The
                    // two it does keep are closed by the install op
                    // right after their dup2.
                    for (r, w) in &pipes {
                        unsafe {
                            if *r != in_fd && *r != out_fd {
                                libc::close(*r);
                            }
                            if *w != in_fd && *w != out_fd {
                                libc::close(*w);
                            }
                        }
                    }
                    stage_fds_park(in_fd, out_fd);

                    // Run this stage's bytecode on a fresh VM
                    crate::fusevm_disasm::maybe_print_stdout(
                        &format!("pipeline:child:stage:{i}"),
                        chunk,
                    );
                    let mut stage_vm = fusevm::VM::new(chunk.clone());
                    stage_vm.last_status = parent_status;
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

        // Parent runs the LAST stage inline. Save stdin, park the last
        // pipe's read end for the chunk's BUILTIN_PIPE_FDS_INSTALL
        // (c:Src/exec.c:3722 `addfd(..., 0, input, 0, NULL)` — after
        // the stage's args are expanded, so `… | print -r -- "[$(cat)]"`
        // has its `$(cat)` read the shell's stdin, not the pipe), run
        // the chunk, restore stdin. Close every other pipe fd so the
        // producer side gets EOF when the last upstream stage exits.
        // Shell-internal save — keep it out of the script's fd range (movefd,
        // c:Src/exec.c:2425).
        let saved_stdin = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_DUPFD, 10) };
        let last_in_fd = if last_idx > 0 {
            pipes[last_idx - 1].0
        } else {
            -1
        };
        // Close all pipe fds in the parent except the one the last
        // stage still has to install. (Children already have their own
        // copies; the install op closes the read end after its dup2.)
        for (r, w) in &pipes {
            unsafe {
                if *r != last_in_fd {
                    libc::close(*r);
                }
                libc::close(*w);
            }
        }
        let outer_stage_fds = stage_fds_park(last_in_fd, -1);

        // Run the last stage's bytecode on a sub-VM with the host wired up.
        // By default (zsh semantics) the sub-VM runs IN THIS PROCESS so the
        // last stage's reads/assignments update the parent's state directly
        // (`echo x | read v` sets $v; `cmd | mapfile arr` sets arr).
        //
        // !!! BASH-MODE GATE !!! bash forks EVERY pipeline stage (unless
        // `shopt -s lastpipe`), so the last stage runs in a SUBSHELL and its
        // variable/array assignments do NOT persist — `echo x | read v; echo
        // $v` prints an empty line, `cmd | mapfile arr` leaves arr unset.
        // Fork the last stage under `--bash` to match. The parent's existing
        // `stage_fds_take()` below closes its `last_in_fd` copy; the forked
        // child inherits the parked pipe fd and installs it onto stdin, and
        // the writer stages (already forked) supply its input.
        let last_stage_status = if crate::dash_mode::bash_mode() {
            let last_chunk = stages_vec.into_iter().last().unwrap();
            crate::fusevm_disasm::maybe_print_stdout("pipeline:last", &last_chunk);
            match unsafe { libc::fork() } {
                -1 => 1,
                0 => {
                    // Subshell child: run the last stage, then _exit with its
                    // status. Reset SIGPIPE + drop the EXIT trap like the
                    // other pipeline children above.
                    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
                    if let Ok(mut t) = crate::ported::builtin::traps_table().lock() {
                        t.remove("EXIT");
                    }
                    let mut stage_vm = fusevm::VM::new(last_chunk);
                    stage_vm.last_status = parent_status;
                    register_builtins(&mut stage_vm);
                    stage_vm.set_shell_host(Box::new(ZshrsHost));
                    let _ = stage_vm.run();
                    let st = stage_vm.last_status;
                    let _ = std::io::stdout().flush();
                    let _ = std::io::stderr().flush();
                    unsafe { libc::_exit(st) };
                }
                pid => {
                    let mut status: i32 = 0;
                    unsafe { libc::waitpid(pid, &mut status, 0) };
                    if libc::WIFEXITED(status) {
                        libc::WEXITSTATUS(status)
                    } else if libc::WIFSIGNALED(status) {
                        128 + libc::WTERMSIG(status)
                    } else {
                        1
                    }
                }
            }
        } else {
            let last_chunk = stages_vec.into_iter().last().unwrap();
            crate::fusevm_disasm::maybe_print_stdout("pipeline:last", &last_chunk);
            let mut stage_vm = fusevm::VM::new(last_chunk);
            stage_vm.last_status = parent_status;
            register_builtins(&mut stage_vm);
            stage_vm.set_shell_host(Box::new(ZshrsHost));
            let _ = stage_vm.run();
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            stage_vm.last_status
        };

        // Reclaim the read end if the stage chunk never reached its
        // install op (an expansion error aborted it, or the stage was
        // a shape that dispatches without one), then restore the outer
        // stage's still-pending fds for a nested pipeline.
        let (leftover_in, _) = stage_fds_take();
        if leftover_in >= 0 {
            unsafe { libc::close(leftover_in) };
        }
        stage_fds_park(outer_stage_fds.0, outer_stage_fds.1);

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
    // Scalar coercion of an assembled word: pop a Value; if it's an
    // Array (produced by a splice segment like `"$@"` / `"${arr[@]}"`),
    // IFS[0]-join it to a single scalar; a scalar passes through. This
    // is the assignment-context coercion C zsh applies in multsub when
    // the expansion is the RHS of a SCALAR assignment (Src/subst.c
    // c:3032 sepjoin under ssub) — `v="$@"` joins the positionals with
    // ${IFS[1]} rather than leaving an array whose splat would lose all
    // but the first element. Joins via sepjoin so a custom / empty IFS
    // is honored (not a hardcoded space).
    vm.register_builtin(BUILTIN_ARRAY_JOIN, |vm, _argc| {
        let val = vm.pop();
        match val {
            Value::Array(items) => {
                let strs: Vec<String> = items.iter().map(|v| v.to_str()).collect();
                Value::str(crate::ported::utils::sepjoin(&strs, None))
            }
            other => other,
        }
    });

    // `cmd &` background execution. Compile_list emits this for any item
    // followed by ListOp::Amp: the job text + the cmd's sub-chunk index are
    // pushed, then this builtin pops both, looks up the chunk, forks. The
    // child detaches via setsid (so SIGINT to the foreground job doesn't kill
    // it), runs the bytecode on a fresh VM with builtins re-registered, exits
    // with the last status. The parent registers the job in the canonical
    // JOBTAB (c:Src/exec.c::execpline Z_ASYNC arm) and returns Status(0).
    vm.register_builtin(BUILTIN_RUN_BG, |vm, _argc| {
        // `&|` / `&!` set disown → the job is dropped from the table (no
        // `[N] pid` announcement, no `[N] done`), matching C exec.c:1752-1758.
        let disown = vm.pop().to_int() != 0;
        let sub_idx = vm.pop().to_int() as usize;
        let job_text = vm.pop().to_str();
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
                // c:Src/exec.c:1700 — `thisjob = newjob = initjob()`:
                // allocate the canonical jobtab slot. c:Src/exec.c:2950
                // zfork path → addproc(pid, text, 0, &bgtime, ...) hangs
                // the proc entry (with its display text) off the job.
                // c:Src/exec.c:1744-1746 — `clearoldjobtab();
                // jobtab[thisjob].stat |= STAT_NOSTTY;` then c:1758
                // `spawnjob()` promotes it to curjob (top-level shell
                // only), marks STAT_LOCKED and resets thisjob.
                {
                    use crate::ported::jobs;
                    use std::sync::Mutex;
                    let table = jobs::JOBTAB.get_or_init(|| Mutex::new(Vec::new()));
                    let idx = {
                        let mut tab = table.lock().unwrap_or_else(|e| e.into_inner());
                        let idx = jobs::initjob(&mut tab); // c:exec.c:1700
                        jobs::addproc(
                            &mut tab[idx],
                            pid,
                            &job_text,
                            false,
                            Some(std::time::Instant::now()),
                            -1,
                            -1,
                        ); // c:exec.c:2950 addproc
                        tab[idx].stat |= crate::ported::zsh_h::STAT_NOSTTY; // c:exec.c:1746
                        idx
                    };
                    jobs::clearoldjobtab(); // c:exec.c:1744
                    if let Ok(mut tj) = jobs::THISJOB.get_or_init(|| Mutex::new(-1)).lock() {
                        *tj = idx as i32;
                    }
                    if disown {
                        // c:exec.c:1752-1755 — `pipecleanfilelist(...);
                        // deletejob(jobtab + thisjob, 1); thisjob = -1;` — a
                        // disowned job leaves the table entirely, so neither
                        // spawnjob's `[N] pid` nor the later `[N] done` prints.
                        // This is what keeps zinit-turbo's `… &|` completion
                        // jobs silent (they load inside a `zle -F` handler).
                        {
                            let mut tab = table.lock().unwrap_or_else(|e| e.into_inner());
                            jobs::pipecleanfilelist(&mut tab[idx], false); // c:1753
                            jobs::deletejob(&mut tab[idx], true); // c:1754
                        }
                        if let Ok(mut tj) = jobs::THISJOB.get_or_init(|| Mutex::new(-1)).lock() {
                            *tj = -1; // c:1755
                        }
                    } else {
                        jobs::spawnjob(); // c:exec.c:1758
                    }
                }
                with_executor(|exec| {
                    exec.jobs
                        .add_pid_job(pid, job_text.clone(), JobState::Running);
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
        // `${~spec}` carrier: an assignment statement is a word-
        // pipeline boundary too — restore the user's GLOB_SUBST
        // before the NEXT word expands (`Z[d]=${~Z[d]}; print
        // ${options[globsubst]}` must read the user value).
        consume_tilde_globsubst_carrier();
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
            flatten_array_value(v, &mut values);
        }
        // Bash sparse: a full `a=(...)` reassign resets the array to dense
        // (drops any prior holes from subscript-assign / unset).
        if crate::dash_mode::bash_mode() {
            crate::bash_arrays::clear(&name);
        }
        let blocked = with_executor(|exec| {
            // Assoc init `typeset -A m; m=(k v k v ...)` — route to
            // canonical sethparam (Src/params.c:3602) which parses the
            // flat (k,v) pair list internally.
            if exec.assoc(&name).is_some() {
                // `[k]=v` / `[k]+=v` elements arrive from the compiler
                // as Marker / key / value triples (compile_zsh's port
                // of keyvalpairelement, c:Src/subst.c:49-79).
                let marker = crate::ported::zsh_h::Marker;
                let values = if values.iter().any(|e| e.starts_with(marker)) {
                    // c:Src/params.c:3544-3560 — under ASSPM_KEY_VALUE
                    // assocs strictly enforce `[key]=value`: every
                    // stride-of-3 element must be a Marker. Mixing
                    // plain pairs with kv triads is an error.
                    let mut i = 0usize;
                    while i < values.len() {
                        if !values[i].starts_with(marker) {
                            crate::ported::utils::zerr(
                                "bad [key]=value syntax for associative array",
                            );
                            crate::ported::utils::errflag.fetch_or(
                                crate::ported::zsh_h::ERRFLAG_ERROR,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            exec.set_last_status(1);
                            return true;
                        }
                        i += 3;
                    }
                    if values.len() % 3 != 0 {
                        // c:Src/params.c:4124-4131 arrhashsetfn — a
                        // truncated triad leaves an odd non-Marker
                        // count → "bad set of key/value pairs".
                        crate::ported::utils::zerr(
                            "bad set of key/value pairs for associative array",
                        );
                        crate::ported::utils::errflag.fetch_or(
                            crate::ported::zsh_h::ERRFLAG_ERROR,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        exec.set_last_status(1);
                        return true;
                    }
                    // c:Src/params.c:4136-4168 arrhashsetfn — whole
                    // assignment builds a FRESH table; a `Marker +`
                    // triad (`[k]+=v`) appends to the value inserted
                    // EARLIER IN THIS SAME LITERAL (assignstrvalue
                    // with eltflags=ASSPM_AUGMENT against the new ht),
                    // so `h=([k]=a [k]+=b)` yields "ab". Resolve the
                    // appends here, then hand flat pairs to sethparam.
                    let mut order: Vec<String> = Vec::new();
                    let mut map: std::collections::HashMap<String, String> =
                        std::collections::HashMap::new();
                    for ch in values.chunks(3) {
                        let elt_append = ch[0].chars().nth(1) == Some('+');
                        let k = ch[1].clone();
                        let v = ch[2].clone();
                        let nv = if elt_append {
                            format!("{}{}", map.get(&k).cloned().unwrap_or_default(), v)
                        } else {
                            v
                        };
                        if !map.contains_key(&k) {
                            order.push(k.clone());
                        }
                        map.insert(k, nv);
                    }
                    order
                        .into_iter()
                        .flat_map(|k| {
                            let v = map.get(&k).cloned().unwrap_or_default();
                            [k, v]
                        })
                        .collect()
                } else {
                    values
                };
                // Odd-count rejection lives in the canonical chain:
                // sethparam → setarrvalue (c:3651/c:2920) →
                // arrhashsetfn's zerr "bad set of key/value pairs"
                // (Src/params.c:4128-4131). zerr sets ERRFLAG_ERROR,
                // which aborts the remaining list at the next command
                // boundary (BUILTIN_ERREXIT_CHECK trigger 4) —
                // matching `zsh -fc 'typeset -A m; m=(odd); print x'`
                // printing nothing after the error. Like C's sethparam
                // (c:3652-3653 returns v->pm regardless), the Rust
                // port returns Some on the odd-count path — the
                // failure travels via errflag, so check BOTH.
                let pre_err = crate::ported::utils::errflag
                    .load(std::sync::atomic::Ordering::Relaxed)
                    & crate::ported::zsh_h::ERRFLAG_ERROR;
                let res = crate::ported::params::sethparam(&name, values.clone());
                let now_err = crate::ported::utils::errflag
                    .load(std::sync::atomic::Ordering::Relaxed)
                    & crate::ported::zsh_h::ERRFLAG_ERROR;
                if res.is_none() || (pre_err == 0 && now_err != 0) {
                    // c:Src/exec.c:2632-2633 addvars — `if
                    // (!assignaparam(name, arr, myflags)) lastval = 1;`
                    // — failed assignment sets lastval so the errflag
                    // abort exits 1 (init.c loop() breaks, zsh_main
                    // returns lastval).
                    exec.set_last_status(1);
                    return true;
                }
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
            // `[k]=v` elements arrive as Marker / key / value triples
            // (compile_zsh's keyvalpairelement port). Mirror
            // c:Src/exec.c:2552-2553 — `if (prefork_ret &
            // PREFORK_KEY_VALUE) myflags |= ASSPM_KEY_VALUE;` — so
            // assignaparam runs its kv-resolution block (sparse fill
            // for PM_ARRAY, c:3447-3541; strict-triad enforcement for
            // special PM_HASHED targets like `options`, c:3544-3560).
            let values = values;
            let has_kv = values
                .iter()
                .any(|e| e.starts_with(crate::ported::zsh_h::Marker));
            // The tied-array mirror to a PM_TIED scalar
            // (`typeset -T PATH path`) lives canonically in
            // setarrvalue's dispatch in C zsh; until that wires
            // through assignaparam, mirror here so PATH stays in sync
            // after `path=(/x)`.
            //
            // The mirrored value must be the array AS STORED, which for a
            // PM_UNIQUE tie means deduped. c:4066-4076 arrsetfn fixes the
            // order:
            //     if (pm->node.flags & PM_UNIQUE) uniqarray(x);
            //     pm->u.arr = x;
            //     if (pm->ename && x) arrfixenv(pm->ename, x);
            // — the dedupe happens FIRST, so the scalar publishes the same
            // list the array holds and the two halves of a tie always agree.
            // Mirroring the raw `values` broke exactly that:
            //     typeset -U path; path=(/a /b /a)
            //       $path → /a /b        (right)
            //       $PATH → /a:/b:/a     (wrong; zsh gives /a:/b)
            // i.e. `typeset -U path`, the standard PATH-dedup idiom in
            // essentially every .zshrc. assignaparam's own arrfixenv does not
            // rescue it: that call is gated on the param having a gsu_a wired,
            // and `path` has none.
            //
            // The dedupe is applied here rather than by reading the array back
            // after assignaparam, because this mirror must stay BEFORE it.
            // `exec.set_scalar` is heavier than C's arrfixenv — arrfixenv only
            // rewrites the environment string, while set_scalar re-derives the
            // ARRAY from the scalar. Running it afterwards makes `path=()`
            // publish PATH="" and then re-split that back into a one-element
            // `path=("")`, where zsh leaves 0 elements.
            if let Some((scalar_name, sep)) = exec.tied_array_to_scalar.get(&name).cloned() {
                let uniq = crate::ported::params::paramtab()
                    .read()
                    .ok()
                    .and_then(|t| t.get(&name).map(|p| p.node.flags))
                    .map(|f| (f as u32 & crate::ported::zsh_h::PM_UNIQUE) != 0)
                    .unwrap_or(false);
                let mirror = if uniq {
                    crate::ported::params::simple_arrayuniq(values.clone()) // c:4068
                } else {
                    values.clone()
                };
                exec.set_scalar(scalar_name, mirror.join(&sep)); // c:4074-4075
            }
            // c:Src/exec.c:2632-2633 addvars — `if (!assignaparam(...))
            // lastval = 1;` — a failed assignment (bad subscript, bad
            // [key]=value syntax, readonly) exits 1 and the errflag
            // abort stops the remaining list. Track errflag pre/post
            // like the assoc branch above.
            let kv_flag = if has_kv {
                crate::ported::zsh_h::ASSPM_KEY_VALUE
            } else {
                0
            };
            let pre_err = crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed)
                & crate::ported::zsh_h::ERRFLAG_ERROR;
            let res = crate::ported::params::assignaparam(
                &name,
                values.clone(),
                crate::ported::zsh_h::ASSPM_WARN | kv_flag,
            );
            let now_err = crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed)
                & crate::ported::zsh_h::ERRFLAG_ERROR;
            if res.is_none() && pre_err == 0 && now_err != 0 {
                exec.set_last_status(1);
                return true;
            }
            // Bash sparse: an explicit-index array literal `a=([2]=x [5]=y)`
            // leaves the un-indexed slots as HOLES (bash: count 2, indices
            // {2,5}), not dense empties. assignaparam has already placed each
            // value at its 0-based index; mark every OTHER slot a hole. Gated
            // to a PURE indexed literal (every element a `[idx]=val` triple) —
            // a mixed positional/indexed literal (`a=(x [3]=y z)`) needs the
            // positional-counter replay we don't model, so it stays dense.
            if crate::dash_mode::bash_mode() && has_kv {
                let marker = crate::ported::zsh_h::Marker;
                let pure_indexed = !values.is_empty()
                    && values.len() % 3 == 0
                    && values.chunks(3).all(|ch| ch[0].starts_with(marker));
                if pure_indexed {
                    let mut explicit: std::collections::BTreeSet<usize> =
                        std::collections::BTreeSet::new();
                    for ch in values.chunks(3) {
                        if let Ok(i) = ch[1].trim().parse::<usize>() {
                            explicit.insert(i);
                        }
                    }
                    let len = exec.array(&name).map(|a| a.len()).unwrap_or(0);
                    for i in 0..len {
                        if !explicit.contains(&i) {
                            crate::bash_arrays::note_unset(&name, i);
                        }
                    }
                }
            }
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                let ctx = exec.recorder_ctx();
                let attrs = exec.recorder_attrs_for(&name);
                emit_path_or_assign(&name, &values, attrs, false, &ctx);
            }
            false
        });
        let status = if blocked { 1 } else { 0 };
        // c:Src/jobs.c:1748-1757 waitonejob — in C an array-assignment
        // simple command goes through execpline → waitjobs; with no
        // procs the else-branch stores `pipestats[0] = lastval;
        // numpipestats = 1`. Bare SCALAR assignments never create a
        // job (no waitjobs), so this clobber is array/assoc-assignment
        // specific: `false|true; x=(1 2); echo $pipestatus` → `0` in
        // zsh while `x=1` preserves `1 0`. Bug #373.
        crate::ported::builtin::LASTVAL.store(status, std::sync::atomic::Ordering::Relaxed);
        let mut synth = crate::ported::zsh_h::job::default();
        crate::ported::jobs::waitonejob(&mut synth);
        Value::Status(status)
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
            flatten_array_value(v, &mut values);
        }
        let blocked = with_executor(|exec| -> bool {
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
                // `[k]=v` / `[k]+=v` elements arrive as Marker / key /
                // value triples (compile_zsh's port of
                // keyvalpairelement, c:Src/subst.c:49-79).
                let marker = crate::ported::zsh_h::Marker;
                let mut map = exec.assoc(&name).unwrap_or_default();
                if values.iter().any(|e| e.starts_with(marker)) {
                    // c:Src/params.c:3544-3560 — strict triad rule.
                    let mut i = 0usize;
                    while i < values.len() {
                        if !values[i].starts_with(marker) {
                            crate::ported::utils::zerr(
                                "bad [key]=value syntax for associative array",
                            );
                            crate::ported::utils::errflag.fetch_or(
                                crate::ported::zsh_h::ERRFLAG_ERROR,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            exec.set_last_status(1);
                            return true;
                        }
                        i += 3;
                    }
                    if values.len() % 3 != 0 {
                        // c:Src/params.c:4124-4131 — odd pair count.
                        crate::ported::utils::zerr(
                            "bad set of key/value pairs for associative array",
                        );
                        crate::ported::utils::errflag.fetch_or(
                            crate::ported::zsh_h::ERRFLAG_ERROR,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        exec.set_last_status(1);
                        return true;
                    }
                    // c:Src/params.c:4133-4168 arrhashsetfn with
                    // ASSPM_AUGMENT — ht = the EXISTING table, so
                    // `[k]+=v` appends to the current value
                    // (assignstrvalue eltflags=ASSPM_AUGMENT,
                    // c:4144-4150) and `[k]=v` overwrites.
                    for ch in values.chunks(3) {
                        let elt_append = ch[0].chars().nth(1) == Some('+');
                        let k = ch[1].clone();
                        let v = ch[2].clone();
                        let nv = if elt_append {
                            format!("{}{}", map.get(&k).cloned().unwrap_or_default(), v)
                        } else {
                            v
                        };
                        map.insert(k, nv);
                    }
                } else {
                    let mut it = values.iter().cloned();
                    while let Some(k) = it.next() {
                        if let Some(v) = it.next() {
                            map.insert(k, v);
                        }
                    }
                }
                exec.set_assoc(name, map);
                return false;
            }
            // Indexed-array append `arr+=(d e f)` — route directly
            // through canonical assignaparam with ASSPM_AUGMENT
            // (`Src/params.c:3570-3585` append-on-array branch).
            // assignaparam reads the prior array internally and
            // appends the new values, so the bridge no longer needs
            // to pre-concat manually. Marker triples from `[k]=v`
            // elements add ASSPM_KEY_VALUE (c:Src/exec.c:2552-2553)
            // so the kv sparse-fill block (c:Src/params.c:3447-3541)
            // resolves them against the existing elements.
            let kv_flag = if values
                .iter()
                .any(|e| e.starts_with(crate::ported::zsh_h::Marker))
            {
                crate::ported::zsh_h::ASSPM_KEY_VALUE
            } else {
                0
            };
            let pre_err = crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed)
                & crate::ported::zsh_h::ERRFLAG_ERROR;
            let res = crate::ported::params::assignaparam(
                &name,
                values.clone(),
                crate::ported::zsh_h::ASSPM_AUGMENT | kv_flag,
            );
            let now_err = crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed)
                & crate::ported::zsh_h::ERRFLAG_ERROR;
            if res.is_none() && pre_err == 0 && now_err != 0 {
                // c:Src/exec.c:2632-2633 — failed assignment → lastval 1.
                exec.set_last_status(1);
                return true;
            }
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
                let _ = crate::ported::params::zputenv(&format!("{}={}", &scalar_name, &joined));
                // c:Src/params.c:5354
            }
            false
        });
        // c:Src/jobs.c:1748-1757 waitonejob — `arr+=(...)` is an
        // array-assignment simple command and clobbers pipestats to
        // `[lastval]` exactly like `arr=(...)` above. Bug #373.
        let status = if blocked { 1 } else { 0 };
        crate::ported::builtin::LASTVAL.store(status, std::sync::atomic::Ordering::Relaxed);
        let mut synth = crate::ported::zsh_h::job::default();
        crate::ported::jobs::waitonejob(&mut synth);
        Value::Status(status)
    });
    // `name[@]=(...)` / `name[*]=(...)` — whole-array SET with the assoc
    // guard (c:Src/params.c:3324-3327). Stack: [v0..vn, name].
    vm.register_builtin(BUILTIN_SET_ARRAY_AT, |vm, argc| {
        let (name, values) = pop_array_args_with_name(vm, argc);
        let status = with_executor(|exec| {
            if exec.assoc(&name).is_some() {
                // c:Src/params.c:3324-3327 — `[@]` (any slice) on a
                // PM_HASHED target is an error.
                crate::ported::utils::zerr(&format!(
                    "{}: attempt to set slice of associative array",
                    name
                ));
                crate::ported::utils::errflag.fetch_or(
                    crate::ported::zsh_h::ERRFLAG_ERROR,
                    std::sync::atomic::Ordering::Relaxed,
                );
                exec.set_last_status(1);
                return 1;
            }
            exec.set_array(name, values); // whole replace (c:3528 setarrvalue)
            0
        });
        Value::Status(status)
    });
    // `name[@]+=(...)` / `name[*]+=(...)` — whole-array APPEND (push) with
    // the same assoc guard.
    vm.register_builtin(BUILTIN_APPEND_ARRAY_AT, |vm, argc| {
        let (name, values) = pop_array_args_with_name(vm, argc);
        let status = with_executor(|exec| {
            if exec.assoc(&name).is_some() {
                crate::ported::utils::zerr(&format!(
                    "{}: attempt to set slice of associative array",
                    name
                ));
                crate::ported::utils::errflag.fetch_or(
                    crate::ported::zsh_h::ERRFLAG_ERROR,
                    std::sync::atomic::Ordering::Relaxed,
                );
                exec.set_last_status(1);
                return 1;
            }
            let mut cur = exec.array(&name).unwrap_or_default();
            cur.extend(values);
            exec.set_array(name, cur); // c:3511-3528 AUGMENT on array → push
            0
        });
        Value::Status(status)
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
                    for item in items.iter() {
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

        // c:Src/loop.c:264 — `more = selectlist(args, 0);` renders the menu
        // ONCE, BEFORE the selection loop. C reprints it ONLY when the user
        // enters an EMPTY line (c:290, inside the inner read loop) — never per
        // body iteration. This was a single conflated loop that re-rendered at
        // the top of every pass, so `printf "1\n2\n" | select x in a b; do
        // print $x; done` redrew the list before each prompt where zsh prints
        // it once.
        //
        // (`selectlist` is also ported at src/ported/loop.rs:127, but that copy
        // derives its row budget from adjustlines()/adjustcolumns() — an ioctl
        // on fd 1 — and so renders nothing when stdout is not a tty, which is
        // exactly this path. Keeping the working inline render here.)
        let render_menu = || {
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
        };
        render_menu(); // c:264 — once, before the loop

        'select: loop {
            // c:266-290 — inner read loop: prompt and read until a NON-EMPTY
            // line arrives; each empty line reprints the menu and re-reads.
            let trimmed = loop {
                let _ = write!(std::io::stderr(), "{}", prompt);
                let _ = std::io::stderr().flush();

                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        // c:277-285 — EOF (user pressed Ctrl+D): REPLY="",
                        // a newline to stderr, then leave the construct.
                        with_executor(|exec| {
                            exec.set_scalar("REPLY".to_string(), String::new());
                        });
                        let _ = writeln!(std::io::stderr());
                        let _ = std::io::stderr().flush();
                        break 'select;
                    }
                    Ok(_) => {}
                    Err(_) => break 'select,
                }
                let t = line.trim_end_matches(['\n', '\r'][..].as_ref()).to_string();
                // c:288-289 — `if (*str) break;`
                if !t.is_empty() {
                    break t;
                }
                // c:290 — `more = selectlist(args, more);` on an empty line.
                render_menu();
            };
            // c:291 `setsparam("REPLY", ztrdup(str));` — REPLY is set once the
            // inner loop yields a non-empty line. An empty line never reaches
            // here: c:290 reprints and re-reads instead.
            with_executor(|exec| {
                exec.set_scalar("REPLY".to_string(), trimmed.clone());
            });

            // c:293 `i = atoi(str);` — atoi(3) reads a LEADING integer:
            // optional blanks, optional sign, then digits, ignoring whatever
            // trails, and yields 0 when there are no digits at all.
            // `parse::<usize>()` is strict and rejected `1 2` / `1abc` / `+2`
            // / ` 1`, so a reply with anything after the number selected
            // NOTHING where zsh selects the leading number's item.
            let i: i64 = {
                let b = trimmed.as_bytes();
                let mut p = 0;
                while p < b.len() && (b[p] == b' ' || b[p] == b'\t') {
                    p += 1;
                }
                let neg = p < b.len() && b[p] == b'-';
                if p < b.len() && (b[p] == b'-' || b[p] == b'+') {
                    p += 1;
                }
                let mut v: i64 = 0;
                while p < b.len() && b[p].is_ascii_digit() {
                    v = v.saturating_mul(10).saturating_add((b[p] - b'0') as i64);
                    p += 1;
                }
                if neg {
                    -v
                } else {
                    v
                }
            };
            // c:294-301 — `if (!i) str = "";` else walk i-1 nodes and take
            // that word; running off the end leaves "". A NEGATIVE i walks
            // until the list is exhausted (`n && i`), which also lands on "".
            let chosen = if i <= 0 {
                String::new()
            } else {
                words.get((i - 1) as usize).cloned().unwrap_or_default()
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
        array_index_lookup(&name, &idx)
    });
    // BUILTIN_ARRAY_INDEX_UNBRACED — bare `$name[idx]` (no braces).
    // Same subscript dispatch as BUILTIN_ARRAY_INDEX when KSHARRAYS
    // is unset, but under KSHARRAYS the UNBRACED form does NOT
    // subscript at all:
    //   c:Src/subst.c:2800-2802 — fetchvalue's bracket-parse arg is
    //     `(unset(KSHARRAYS) || inbrace) ? 1 : -1`; -1 inhibits
    //     subscript parsing for the bare form under KSHARRAYS.
    //   c:Src/subst.c:2867 — the bracket-consuming loop only runs
    //     `while (v || ((inbrace || (unset(KSHARRAYS) && vunset)) &&
    //     isbrack(*s)))` — bare + KSHARRAYS leaves `[...]` as literal
    //     trailing text.
    // The bare `$name` expands (first element for identifier-named
    // arrays per c:Src/params.c:2293-2296 `v->end = 1, v->isarr = 0`),
    // the literal `[idx]` (+ any literal suffix) joins the last word,
    // and the word undergoes filename generation: `[...]` is a glob
    // char class, so unquoted it hits the c:Src/glob.c:1873-1886
    // nomatch/nullglob dispatch (reused via exec.expand_glob).
    // Operands: [name, idx, suffix, quoted] — `quoted` set when the
    // word carries DQ markers (no filename generation in DQ; zsh 5.9:
    // `setopt ksharrays; a=(x y z); print "$a[0]"` → `x[0]`).
    // Verbatim zsh 5.9 ground truth for the unquoted form:
    //   `setopt ksharrays; a=(x y z); print -- $a[0]` →
    //   stderr `zsh:1: no matches found: x[0]`, rc=1, empty stdout.
    vm.register_builtin(BUILTIN_ARRAY_INDEX_UNBRACED, |vm, _argc| {
        let quoted = vm.pop().to_str() == "1";
        let suffix = vm.pop().to_str();
        let idx = vm.pop().to_str();
        let name = vm.pop().to_str();
        if !crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS) {
            let v = array_index_lookup(&name, &idx);
            if suffix.is_empty() {
                return v;
            }
            // Mirrors the previous compile-shape `ARRAY_INDEX +
            // Op::Concat` exactly: Concat stringifies via as_str_cow
            // (fusevm value.rs:132-146, arrays join with " ").
            return Value::str(format!("{}{}", v.to_str(), suffix));
        }
        // KSHARRAYS bare form: no subscript. Bare-`$name` words +
        // literal `[idx]suffix` glued onto the last word.
        let mut words = ksharrays_bare_words(&name);
        let last = format!("{}[{}]{}", words.pop().unwrap_or_default(), idx, suffix);
        if quoted {
            // DQ context — no filename generation, bracket text stays
            // literal.
            words.push(last);
            return Value::str(words.join(" "));
        }
        // c:Src/glob.c:1873-1886 — expand_glob handles nullglob /
        // NOMATCH (zerr "no matches found" + errflag + the
        // current_command_glob_failed cell consumed at the command
        // dispatch boundary) / literal passthrough for glob-free text.
        let matches = with_executor(|exec| exec.expand_glob(&last));
        let mut out: Vec<Value> = words.into_iter().map(Value::str).collect();
        out.extend(matches.into_iter().map(Value::str));
        if out.len() == 1 {
            return out.pop().unwrap();
        }
        Value::array(out)
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
        // c:Src/params.c getindex — the subscript text is substituted
        // (singsub) before the lookup; `${(k)H[$k]}` must resolve $k.
        // The compiler hands this opcode the RAW subscript text, so a
        // dynamic key arrived literally ("$k") and matched nothing.
        // singsub is identity for plain keys.
        let key = if key.contains('$') || key.contains('`') || key.contains('\u{8c}') {
            crate::ported::subst::singsub(&key)
        } else {
            key
        };
        // c:Src/params.c:3131 gethkparam covers ordinary PM_HASHED
        // paramtab entries only. Special/magic hashes (`parameters`,
        // `options`, … — the zsh/parameter module's partab-backed
        // params, Src/Modules/parameter.c) aren't in that storage, so
        // a None here doesn't mean "no such assoc". Route those
        // through paramsubst, whose assoc materialization handles the
        // magic hashes and whose getarg port (c:Src/params.c:1591 +
        // Src/subst.c:2922) returns the KEY for `(k)` on a plain
        // subscript. zsh 5.9: `${(k)parameters[PATH]}` → "PATH".
        // c:Src/params.c:1602-1612 — on a hash subscript C dispatches
        // `ht->getnode(ht, s)`; it never enumerates. For the magic hashes that
        // distinction is observable: `getfunction_source` (c:Src/Modules/
        // parameter.c:549-566) answers for names its scan never lists, `mapfile`
        // answers for any readable file, and the job trio calls `getjob(name,
        // NULL)` whose `job not found` diagnostic (Src/jobs.c:2150-2151) must be
        // emitted by the read. `gethkparam` answers Some(<enumerated keys>) for
        // these names, which shortcut the dispatch entirely — `${(k)jobstates[x]}`
        // stayed silent and `${(k)functions_source[x]}` returned "" where zsh
        // returns the key. Send PARTAB names to paramsubst, which owns the
        // getnode path (c:Src/subst.c:2923-2925 — key when set, "" when unset).
        // Keys carrying `]`/`}` can't survive the flat rebuild (see
        // array_index_lookup), so those keep the enumeration answer.
        let magic_getnode = !key.contains(']')
            && !key.contains('}')
            && crate::ported::modules::parameter::PARTAB
                .iter()
                .any(|e_| e_.name == name);
        match crate::ported::params::gethkparam(&name) {
            Some(keys) if !magic_getnode => {
                if keys.iter().any(|k| k == &key) {
                    Value::str(key)
                } else {
                    Value::str("")
                }
            }
            _ => paramsubst_to_value(&format!("${{(k){}[{}]}}", name, key)),
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
    vm.register_builtin(BUILTIN_PARAM_FLAG, |vm, argc| {
        // argc 3 = the compiler flagged this expansion as the VALUE of a
        // scalar assignment (`x=…` / `local x=…`), which C preforks with
        // PREFORK_SINGLE|PREFORK_ASSIGN (c:Src/exec.c:2603 / :4239-4241).
        // PREFORK_SINGLE is paramsubst's `ssub` (c:Src/subst.c:1761) and
        // gates off c:3913's `force_split`, so `(s::)` / `(f)` / `(0)` do
        // not split there. argc 2 = ordinary word, no ssub.
        let ssub = if argc >= 3 { vm.pop().to_int() != 0 } else { false };
        let flags = vm.pop().to_str();
        let name = vm.pop().to_str();
        let body = format!("${{({}){}}}", flags, name);
        let pf_flags = if ssub {
            crate::ported::zsh_h::PREFORK_SINGLE
        } else {
            0
        };
        paramsubst_to_value_pf(&body, pf_flags)
    });

    // `foo[key]=val` — single-key set on an assoc array. Stack: [name, key, value].
    // PURE PASSTHRU: assignsparam with `name[key]` form (C port of
    // `Src/params.c::assignsparam` subscript path at c:3210-3231)
    // already does the indexed-array vs assoc decision, PM_HASHED
    // auto-vivification, numeric-subscript bounds handling, and
    // PM_READONLY rejection.
    vm.register_builtin(BUILTIN_SET_ASSOC, |vm, _argc| {
        // `${~spec}` carrier: an assignment statement is a word-
        // pipeline boundary too — restore the user's GLOB_SUBST
        // before the NEXT word expands (`Z[d]=${~Z[d]}; print
        // ${options[globsubst]}` must read the user value).
        consume_tilde_globsubst_carrier();
        // argc 4 = compile flagged the subscript as DYNAMIC (`H[$k]`):
        // an EXPANDED-empty key is then a legal assoc key (C's
        // assignsparam isident gate sees the raw `$k` text and the
        // empty key stores at getindex time — zinit's
        // ZINIT_SICE[$1…$2] relies on it). argc 3 = source-literal
        // key; `H[]` stays the "not an identifier" error.
        let key_is_dynamic = if _argc == 4 {
            vm.pop().to_int() != 0
        } else {
            false
        };
        let value = vm.pop().to_str();
        let key = vm.pop().to_str();
        let name = vm.pop().to_str();
        // c:Src/params.c:3203-3207 — `if (!isident(s)) { zerr("not an
        // identifier: %s", s); errflag |= ERRFLAG_ERROR; return NULL; }`.
        // Every subscripted assignment passes through that gate, and isident
        // rejects an empty subscript at c:1334 `if (!(ss =
        // parse_subscript(++ss, 1, ']'))) return 0;` — the LHS text is
        // untokenized by then, so the `]` IS parse_subscript's literal endchar
        // and c:Src/lex.c:1748 returns NULL. `m[]=z` / `A[]=z` / `s[]=z` are
        // therefore all `not an identifier: NAME[]`, never a store.
        // Only the SOURCE-LITERAL empty subscript is affected: with a dynamic
        // key (`H[$k]=v`) C's isident sees the unexpanded `$k` text, passes,
        // and getindex stores the expanded — possibly empty — key.
        // The gate lives here because the PM_HASHED fast path and the numeric
        // pre-resolve below both reach the store without calling assignsparam
        // (the empty key resolved to "" for a hash and to math-0 for an
        // indexed array, so `A[]=z` reported "assignment to invalid subscript
        // range" instead). Route through assignsparam so the diagnostic and
        // the errflag are the canonical ones.
        if !key_is_dynamic && key.is_empty() {
            crate::ported::params::assignsparam(
                &format!("{}[]", name), // c:3203 — the LHS spelling zsh reports
                &value,
                crate::ported::zsh_h::ASSPM_WARN,
            );
            return Value::Status(1);
        }
        // Bash sparse-array tracking for `a[i]=v` (scalar single-index). A set
        // that pads the dense Vec past its old end leaves old_len..i as holes;
        // on an undefined array, `a[5]=q` leaves only index 5 (count 1). Only
        // for INDEXED arrays (assoc keys are strings), bash mode, numeric key.
        // Captured before the assign; applied after (on the array path).
        let sparse_track: Option<(String, usize, usize)> =
            if crate::dash_mode::bash_mode() && !key.contains(',') {
                key.trim().parse::<usize>().ok().and_then(|i| {
                    with_executor(|exec| {
                        if !exec.has_assoc(&name) {
                            let old_len = exec.array(&name).map(|a| a.len()).unwrap_or(0);
                            Some((name.clone(), old_len, i))
                        } else {
                            None
                        }
                    })
                })
            } else {
                None
            };
        if key_is_dynamic && key.is_empty() {
            with_executor(|exec| {
                let _ = exec;
            });
            // Mirror assignsparam's PM_HASHED tail directly (the
            // textual `name[]` reconstruction can't pass isident).
            if let Ok(mut store) = crate::ported::params::paramtab_hashed_storage().lock() {
                let entry = store.entry(name.clone()).or_default();
                let newval = if let Some(old) = entry.get("") {
                    // `+=` arrives pre-concatenated by the compile
                    // read-modify-write; plain `=` overwrites.
                    let _ = old;
                    value.clone()
                } else {
                    value.clone()
                };
                entry.insert(String::new(), newval);
            }
            return Value::Status(0);
        }
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
            // Existence probes only — use the non-cloning `has_*`
            // checks. `exec.assoc()` / `exec.array()` return owned
            // clones of the whole map/vector, so probing `.is_some()`
            // here copied the entire associative array on every
            // `h[k]=v` (O(n) per store → O(n²) for a fill loop). The
            // profiler flagged `ShellExecutor::assoc → IndexMap::clone`
            // as the dominant cost.
            let is_indexed = exec.has_array(&name);
            let is_assoc = exec.has_assoc(&name);
            let is_scalar = !is_indexed && !is_assoc && exec.has_scalar(&name);
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
                                            &{
                                                let mut __pat_tok = (pat).to_string();
                                                crate::ported::glob::tokenize(&mut __pat_tok);
                                                __pat_tok
                                            },
                                            crate::ported::zsh_h::PAT_HEAPDUP as i32,
                                            None,
                                        )
                                        .map_or(false, |p| crate::ported::pattern::pattry(&p, elem))
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
                                                &{
                                                    let mut __pat_tok = (pat).to_string();
                                                    crate::ported::glob::tokenize(&mut __pat_tok);
                                                    __pat_tok
                                                },
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
                                                    &{
                                                        let mut __pat_tok = (pat).to_string();
                                                        crate::ported::glob::tokenize(
                                                            &mut __pat_tok,
                                                        );
                                                        __pat_tok
                                                    },
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
                                        if last_found.is_some() && last_found.unwrap() >= start {
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
        // c:Src/params.c getindex — C parses the subscript from the
        // TOKENIZED source word, so a `]`/`}` that arrived via `$key`
        // expansion is plain data and can never terminate the
        // subscript. The textual `name[key]` rebuild below re-parses
        // the FLAT string, where an expanded `]` splits the key at the
        // first bracket (`c[$k]=5` with k='x]y' stored key "x" and
        // spilled junk — zpwr expandstats died on the spill in a later
        // math expr). For a PM_HASHED target the compile-time split
        // already isolated the exact key: store it directly via the
        // canonical hashed storage (same mechanism as the
        // dynamic-empty-key arm above / assignsparam's PM_HASHED
        // tail), with the readonly guard assignsparam would apply.
        let target_flags = with_executor(|exec| exec.param_flags(&name));
        // PM_SPECIAL exclusion: the zsh/parameter magic assocs
        // (functions / aliases / galiases / saliases / options / …)
        // have per-key setfns with SIDE EFFECTS — `functions[x]=body`
        // must parse the body into shfunctab (Src/Modules/
        // parameter.c:296 setfunction), `aliases[x]=v` must write
        // aliastab. The direct hashed-storage store below silently
        // swallowed those: zinit's tmp-subst wrappers
        // (`functions[autoload]=':zinit-tmp-subst-autoload "$@";'`)
        // never became real functions, so every
        // `.zinit-tmp-subst-off` spammed `unfunction: no such hash
        // table element: autoload/compdef/bindkey/…`. Route specials
        // through assignsparam's canonical per-name arms instead.
        if (target_flags as u32 & crate::ported::zsh_h::PM_HASHED) != 0
            && (target_flags as u32 & crate::ported::zsh_h::PM_SPECIAL) == 0
        {
            if (target_flags as u32 & crate::ported::zsh_h::PM_READONLY) != 0 {
                crate::ported::utils::zerr(&format!("read-only variable: {}", name));
                return Value::Status(1);
            }
            if let Ok(mut store) = crate::ported::params::paramtab_hashed_storage().lock() {
                store
                    .entry(name.clone())
                    .or_default()
                    .insert(resolved_key.clone(), value.clone());
            }
            return Value::Status(0);
        }
        let subscripted = format!("{}[{}]", name, resolved_key);
        crate::ported::params::assignsparam(&subscripted, &value, crate::ported::zsh_h::ASSPM_WARN);
        if let Some((nm, old_len, i)) = sparse_track {
            crate::bash_arrays::note_subscript_set(&nm, old_len, i);
        }
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
                    .iter()
                    .filter(|x| !x.to_str().is_empty())
                    .cloned()
                    .collect();
                Value::array(filtered)
            }
            Value::Str(s) if s.is_empty() => Value::array(Vec::new()),
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
                '\\' | '<' | '>' | '(' | '|' | ')' | '^' | '#' | '~' | '[' | ']' | '*' | '?'
                | '$' | ' ' => {
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
        // The Array is a carrier for "no argv word", not an array-SHAPED
        // result: a single empty field came from an empty SCALAR (c:3922-3923
        // `if (!aval || !aval[0]) val = dupstring("");`). Record that, or
        // under RC_EXPAND_PARAM `concat_plan9` reads a stale bit and applies
        // c:4362's `uremnode` to a word zsh keeps — `x$(true)y` and
        // `v=""; x${=v}y` are each the single word `xy`, not zero words.
        if parts.len() == 1 && parts[0].is_empty() {
            note_empty_is_scalar(true);
            return Value::array(Vec::new());
        }
        // Zero parts is the same story reached by a different route: an empty
        // command substitution splits to nothing at all rather than to one
        // empty field. `to_str()` above means this builtin's input is always
        // a scalar, so an empty result here is never array-shaped.
        restore_empty_shape(nodes_to_value(parts), true)
    });

    // BUILTIN_FORCE_SPLIT — `${=name}` / SH_WORD_SPLIT forced split.
    // c:Src/subst.c:3920-3928 —
    //     if (force_split && !isarr) {
    //         aval = sepsplit(val, spsep, 0, 1);
    //         if (!aval || !aval[0])   val = dupstring("");
    //         else if (!aval[1])       val = aval[0];
    //         else                     isarr = nojoin ? 1 : 2;
    //     }
    // with spsep == NULL for the `=` flag, so sepsplit falls through to
    // Src/utils.c:3711 spacesplit(s, allownull=0). See BUILTIN_FORCE_SPLIT's
    // doc comment for the empty-field rule and the argc contract.
    vm.register_builtin(BUILTIN_FORCE_SPLIT, |vm, argc| {
        let s = vm.pop().to_str();
        let keep_empties = argc == 1;
        // c:3921 — `sepsplit(val, spsep, 0, 1)`; spsep NULL → spacesplit.
        let raw = crate::ported::utils::sepsplit(&s, None, false);
        // c:Src/subst.c:36 `char nulstring[] = {Nularg, '\0'};` — spacesplit
        // emits this for an empty field delimited by IFS-NON-whitespace
        // (c:Src/utils.c:3732 / :3752); it survives prefork's empty-node
        // delete and remnulargs (c:Src/glob.c:3649) turns it back into "".
        // A plain "" field (c:3734 / :3757) is what a skipped run of
        // IFS-WHITESPACE leaves behind, and prefork DOES delete that one.
        let nulstring = crate::ported::zsh_h::Nularg.to_string();
        let mut out: Vec<String> = Vec::with_capacity(raw.len());
        for w in raw {
            if w == nulstring {
                out.push(String::new());
            } else if w.is_empty() {
                if keep_empties {
                    out.push(String::new());
                }
            } else {
                out.push(w);
            }
        }
        if out.is_empty() {
            // c:3922-3923 — `if (!aval || !aval[0]) val = dupstring("");`:
            // the split produced nothing, so the value is the empty SCALAR.
            // Quoted, that is one empty word (c:4465 `if (qt && !*y) y =
            // dupstring(nulstring);` → `a=( "${=v}" )` has one element);
            // unquoted, prefork deletes it and the word vanishes.
            if keep_empties {
                return Value::str(String::new());
            }
            note_empty_is_scalar(true);
            return Value::array(Vec::new());
        }
        if out.len() == 1 {
            // c:3924 — `else if (!aval[1]) val = aval[0];` — a one-field
            // split stays a SCALAR (this is why `${#${(f)v}}` counts
            // characters when the split yields a single line).
            return Value::str(out.into_iter().next().unwrap());
        }
        // c:3927 — `isarr = nojoin ? 1 : 2;`
        Value::array(out.into_iter().map(Value::str).collect())
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
        // Brace expansion runs BETWEEN the expansion builtin that produced
        // this word and the concat that consumes it (`x${${P}}y` compiles to
        // EXPAND_TEXT, BRACE_EXPAND, CONCAT_DISTRIBUTE). It cannot change
        // whether an empty result was scalar- or array-SHAPED — c:Src/glob.c
        // xpandbraces only ever rewrites the text of existing words. But both
        // exits below funnel through `nodes_to_value`, which records
        // `note_empty_is_scalar(false)` for an empty result, so the shape bit
        // its producer set was being overwritten with "array" before
        // `concat_plan9` could read it. Under RC_EXPAND_PARAM that deleted
        // words zsh keeps: `unset P; x${${P}}y` and `x$(true)y` are each the
        // single word `xy` (c:4438-4467 scalar arm), not zero words.
        // Carry the incoming bit across an empty→empty pass-through.
        let incoming_empty_is_scalar = empty_is_scalar();
        // c:Src/options.c — `no_brace_expand` (negated braceexpand)
        // disables brace expansion entirely. When set, `{a,b}` stays
        // literal. Mirror by short-circuiting xpandbraces; pass the
        // input through unchanged.
        let brace_expand = opt_state_get("braceexpand").unwrap_or(true);
        let brace_ccl = opt_state_get("braceccl").unwrap_or(false);
        let inputs: Vec<String> = match raw {
            Value::Array(items) => items.iter().map(|v| v.to_str()).collect(),
            other => vec![other.to_str()],
        };
        if !brace_expand {
            return restore_empty_shape(nodes_to_value(inputs), incoming_empty_is_scalar);
        }
        let mut out: Vec<String> = Vec::with_capacity(inputs.len());
        for s in inputs {
            for w in crate::ported::glob::xpandbraces(&s, brace_ccl) {
                out.push(w);
            }
        }
        restore_empty_shape(nodes_to_value(out), incoming_empty_is_scalar)
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
        // c:Src/glob.c:1872 — honour `setopt noglob` / `noglob CMD`
        // precommand. When the option is on, the word stays literal
        // (zsh skips the glob expansion entirely). Without this, the
        // segment-fast-path BUILTIN_GLOB_EXPAND fired even after
        // `noglob` set the option, so `noglob echo *.xyz` saw the
        // NOMATCH error instead of the literal pass-through.
        let raw = vm.pop();
        let noglob =
            opt_state_get("noglob").unwrap_or(false) || !opt_state_get("glob").unwrap_or(true);
        glob_expand_word_value(raw, noglob)
    });
    // Redirect-target variant of BUILTIN_GLOB_EXPAND. c:Src/glob.c:
    // 2161-2167 xpandredir — `prefork(&fake, isset(MULTIOS) ? 0 :
    // PREFORK_SINGLE, NULL)` then "Globbing is only done for
    // multios.": a redirect target word is only globbed when the
    // MULTIOS option is set. With it unset, `echo hi > *.txt`
    // creates the literal file `*.txt`, and `wc -c < *.txt` errors
    // "no such file or directory: *.txt". Bug #36 follow-up in
    // docs/BUGS.md.
    vm.register_builtin(BUILTIN_REDIR_GLOB_EXPAND, |vm, _argc| {
        let raw = vm.pop();
        let noglob =
            opt_state_get("noglob").unwrap_or(false) || !opt_state_get("glob").unwrap_or(true);
        let multios = opt_state_get("multios").unwrap_or(true);
        glob_expand_word_value(raw, noglob || !multios)
    });
    // Clear the default-word glob-pending carrier before the word's
    // expansion runs, so a flag set by a prior word never leaks in.
    vm.register_builtin(BUILTIN_DEFAULT_WORD_GLOB_RESET, |_vm, _argc| {
        crate::ported::subst::DEFAULT_WORD_GLOB_PENDING.with(|c| c.set(false));
        Value::Status(0)
    });
    // After the word is assembled, run filename generation ONLY if the
    // default/alternate paramsubst arm flagged a source-glob default
    // (DEFAULT_WORD_GLOB_PENDING). Otherwise pass the word through
    // literally — a parameter VALUE must not glob. c:Src/subst.c globlist.
    vm.register_builtin(BUILTIN_DEFAULT_WORD_GLOB, |vm, _argc| {
        let raw = vm.pop();
        let pending = crate::ported::subst::DEFAULT_WORD_GLOB_PENDING.with(|c| {
            let v = c.get();
            c.set(false); // read + clear
            v
        });
        if !pending {
            return raw;
        }
        let noglob =
            opt_state_get("noglob").unwrap_or(false) || !opt_state_get("glob").unwrap_or(true);
        glob_expand_word_value(raw, noglob)
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

    // `break N`/`continue N` with a RUNTIME level count. Pops [count,
    // name]; math-evaluates count (c:builtin.c:5811 `mathevali`); on
    // count <= 0 emits `argument is not positive: N` via zerrnam (sets
    // errflag → abort, c:5813) and pushes Int(0) (matches no jump-table
    // entry → control falls through to the errflag abort). Otherwise
    // pushes Int(count) for the compiled jump table to dispatch on.
    vm.register_builtin(BUILTIN_BREAK_COUNT_VALIDATE, |vm, _argc| {
        let name = vm.pop().to_str();
        let count_s = vm.pop().to_str();
        let count = crate::ported::math::mathevali(&count_s).unwrap_or(0);
        if count <= 0 {
            crate::ported::utils::zerrnam(&name, &format!("argument is not positive: {count}"));
            return Value::Int(0);
        }
        Value::Int(count)
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
        let glob_subst = crate::ported::zsh_h::isset(crate::ported::zsh_h::GLOBSUBST);
        if !glob_subst {
            return raw;
        }
        // Collect input strings (Str → vec![s]; Array → multiple).
        let inputs: Vec<String> = match raw {
            Value::Array(items) => items.iter().map(|v| v.to_str()).collect(),
            other => vec![other.to_str()],
        };
        // Run expand_glob on each. Empty matches collapse to a
        // single literal pass-through to mirror nullglob-off default.
        let mut out: Vec<String> = Vec::with_capacity(inputs.len());
        for pattern in inputs {
            // c:Src/subst.c — GLOB_SUBST subjects the value to the FULL
            // filename-generation pipeline: `filesub` (tilde/`=` expansion)
            // BEFORE globbing (prefork runs filesub then globlist). zshrs
            // globbed but skipped filesub, so `${~x}` / `setopt globsubst`
            // left `~/foo` un-expanded. filesubstr matches the Tilde TOKEN,
            // so shtokenize the value first (`~`→Tilde, glob metas active),
            // run filesub, then untokenize the surviving glob metas back to
            // raw for expand_glob (which re-tokenizes internally).
            let pattern = if pattern.contains('~') || pattern.contains('=') {
                let mut tok = pattern.clone();
                crate::ported::glob::shtokenize(&mut tok);
                let fs = crate::ported::subst::filesub(&tok, 0);
                crate::ported::lex::untokenize(&fs)
            } else {
                pattern
            };
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
            Value::array(out.into_iter().map(Value::str).collect())
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
        let glob_subst = crate::ported::zsh_h::isset(crate::ported::zsh_h::GLOBSUBST);
        if glob_subst {
            return Value::str(p);
        }
        let mut out = String::with_capacity(p.len() * 2);
        for c in p.chars() {
            match c {
                '*' | '?' | '[' | ']' | '(' | ')' | '|' | '<' | '>' | '#' | '^' | '~' | '\\' => {
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
            let ifs_full = exec.scalar("IFS").unwrap_or_else(|| " \t\n".to_string());
            let sep = ifs_full
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_default();
            let in_dq = exec.in_dq_context > 0;
            let joined = if let Some(v) = crate::dash_mode::bash_special_array(&name) {
                // bash `"${PIPESTATUS[*]}"` / `"${FUNCNAME[*]}"` join.
                v.join(&sep)
            } else if name == "@" || name == "*" || name == "argv" {
                exec.pparams().join(&sep)
            } else if let Some(assoc_map) = exec.assoc(&name) {
                // c:Src/params.c — assoc-splat values for
                // `"${h[@]}"` / `"${h[*]}"`. Bug #109 in
                // docs/BUGS.md.
                assoc_map.values().cloned().collect::<Vec<_>>().join(&sep)
            } else if let Some(arr) = exec.array(&name) {
                // bash sparse arrays: `"${a[*]}"` joins only LIVE elements,
                // dropping hole slots. No-op in --zsh (no holes tracked).
                crate::bash_arrays::compact(&name, arr).join(&sep)
            } else if let Some(arr) = crate::ported::subst::arrays_get(&name) {
                // c:Src/Modules/parameter.c:2239-2291 partab[] — the PM_ARRAY
                // magic specials (reswords/patchars/dis_*/…) are getfn-backed,
                // so `getaparam`'s `pm->u.arr` read (behind `exec.array`) comes
                // back NULL and the join fell through to the scalar fallback,
                // which is empty. Same omission the `[@]` splat had.
                arr.join(&sep)
            } else if let Some(map) = crate::ported::subst::assoc_get(&name) {
                // c:Src/Modules/parameter.c:2235-2298 partab[] — the PM_HASHED
                // magic assocs (aliases/functions/options/…) live behind a
                // scanfn+getfn pair, not in the executor's assoc storage, so
                // `exec.assoc` above misses them and `${aliases[*]}` joined an
                // empty scalar where zsh joins the alias VALUES.
                map.values().cloned().collect::<Vec<_>>().join(&sep)
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
            return Value::array(Vec::new());
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
            Value::array(Vec::new())
        } else if parts.len() == 1 {
            Value::str(parts.into_iter().next().unwrap())
        } else {
            Value::array(parts.into_iter().map(Value::str).collect())
        }
    });

    vm.register_builtin(BUILTIN_ARRAY_ALL, |vm, _argc| {
        let name = vm.pop().to_str();
        // c:Src/params.c:2027-2029 — a `[@]`/`[*]` subscript sets
        // SCANPM_ISVAR_AT, i.e. `isarr != 0` (c:2915). An empty result is
        // therefore an empty ARRAY, and plan9 deletes the whole word
        // (c:4362) rather than keeping the surrounding text.
        //
        // The note describes the EMPTY value this expansion produces, so it
        // must not survive a NON-empty one: the bit is read by concat_plan9
        // when the word folds left-associatively, and a later non-empty
        // segment overwriting it made `setopt rcexpandparam; e=''; a=(x y);
        // print -rl -- ${e}${a}` delete the whole word instead of printing
        // `x` and `y` (c:4437 keeps the surrounding text for a scalar
        // empty; only c:4362's empty ARRAY deletes it). Restore the
        // incoming bit whenever the result is not an empty array.
        let saved_empty_is_scalar = empty_is_scalar();
        note_empty_is_scalar(false);
        let array_all = |vm: &mut fusevm::VM| -> Value {
            let _ = &vm;
            // bash `"${PIPESTATUS[@]}"` / `"${FUNCNAME[@]}"` / `"${BASH_VERSINFO[@]}"`
            // splat — alias the zsh-native special. No-op in --zsh.
            if let Some(v) = crate::dash_mode::bash_special_array(&name) {
                return Value::array(v.into_iter().map(Value::str).collect());
            }
            with_executor(|exec| {
                // Special positional names — splice the positional list.
                if name == "@" || name == "*" || name == "argv" {
                    return Value::array(exec.pparams().iter().map(Value::str).collect());
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
                    // c:Src/params.c:589-594 getparamnode → c:563-585 loadparamnode —
                    // the `[@]` splat resolves the NAME, clearing PM_AUTOLOAD so
                    // paramtypestr (c:Src/Modules/parameter.c:48-50) reports the real
                    // type. Mirrors the arrays_get arm in src/ported/subst.rs.
                    if !crate::vm_helper::magic_special_shadowed(&name) {
                        crate::vm_helper::mark_module_param_used(&name);
                    }
                    let arr = crate::ported::modules::datetime::getcurrenttime();
                    return Value::array(arr.into_iter().map(Value::str).collect());
                }
                if matches!(
                    name.as_str(),
                    "funcstack" | "funcfiletrace" | "funcsourcetrace" | "functrace"
                ) {
                    // c:Src/params.c:589-594 — see the epochtime arm above.
                    if !crate::vm_helper::magic_special_shadowed(&name) {
                        crate::vm_helper::mark_module_param_used(&name);
                    }
                    // Route the three trace arrays through the canonical
                    // ported getfns (Src/Modules/parameter.c:648/:679/:711)
                    // — the previous inline copy emitted wrong shapes
                    // (bare filename for funcfiletrace, `name:lineno` for
                    // functrace instead of `caller:lineno`); same dedup as
                    // the parallel arrays_get handler in subst.rs.
                    let vals: Vec<String> = match name.as_str() {
                        "funcstack" => crate::ported::modules::parameter::FUNCSTACK
                            .lock()
                            .map(|f| f.iter().rev().map(|fs| fs.name.clone()).collect())
                            .unwrap_or_default(),
                        "funcfiletrace" => crate::ported::modules::parameter::funcfiletracegetfn(
                            std::ptr::null_mut(),
                        ),
                        "funcsourcetrace" => {
                            crate::ported::modules::parameter::funcsourcetracegetfn(
                                std::ptr::null_mut(),
                            )
                        }
                        _ => {
                            crate::ported::modules::parameter::functracegetfn(std::ptr::null_mut())
                        }
                    };
                    return Value::array(vals.into_iter().map(Value::str).collect());
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
                    return Value::array(assoc_map.values().cloned().map(Value::str).collect());
                }
                match exec.array(&name) {
                    Some(v) => {
                        // bash sparse arrays: `"${a[@]}"` splats only LIVE
                        // elements, dropping hole slots (`a[5]=q` padding,
                        // `unset a[i]`). No-op in --zsh (no holes tracked).
                        let v = crate::bash_arrays::compact(&name, v);
                        Value::array(v.iter().map(Value::str).collect())
                    }
                    None => {
                        // c:Src/Modules/parameter.c:2235-2298 partab[] — the
                        // PM_HASHED magic assocs (aliases/functions/parameters/
                        // options/commands/builtins/modules/widgets/nameddirs/…)
                        // are real hash params in C, so `${aliases[@]}` takes the
                        // ordinary getvaluearr path and enumerates their VALUES.
                        // zshrs keeps them OUT of the executor's assoc storage
                        // (they are synthesized on demand by `subst::assoc_get`),
                        // so the `exec.assoc` probe above missed and this arm fell
                        // through to the scalar fallback, which returned an EMPTY
                        // array: `alias foo=bar; print -r -- "${aliases[@]}"` gave
                        // nothing where zsh gives `bar man whence`. Every other
                        // form already routed through paramsubst's own magic-assoc
                        // arms; only the flagless `[@]`/`[*]` splat compiles to
                        // BUILTIN_ARRAY_ALL and reached here.
                        //
                        // Placed in the `None` arm so a real indexed array or a
                        // user-defined assoc of the same name still wins, and so
                        // no ordinary array read pays for the PARTAB scan.
                        //
                        // Same gap on the PM_ARRAY side (c:2239-2291 partab[]
                        // rows: reswords/dis_reswords/patchars/dis_patchars/…):
                        // `getaparam` reads `pm->u.arr`, which is NULL on the
                        // placeholder node zshrs installs for a getfn-backed
                        // special, so `${reswords[@]}` splatted nothing. Route
                        // through the canonical `arrays_get` getfn dispatch.
                        if let Some(arr) = crate::ported::subst::arrays_get(&name) {
                            return Value::array(arr.into_iter().map(Value::str).collect());
                        }
                        if let Some(map) = crate::ported::subst::assoc_get(&name) {
                            return Value::array(map.values().cloned().map(Value::str).collect());
                        }
                        // Fall back to scalar lookup. zsh (unlike bash)
                        // does NOT IFS-split a scalar variable in a for
                        // list — `for w in $scalar` iterates ONCE with the
                        // scalar value. Word-splitting requires either
                        // sh_word_split option or explicit `${(s.,.)scalar}`.
                        let val = exec.get_variable(&name);
                        if val.is_empty() && !exec.has_scalar(&name) && env::var(&name).is_err() {
                            // c:Src/subst.c:3480-3485 — `${arr[@]}` on a genuinely
                            // UNSET parameter under NO_UNSET is a "parameter not set"
                            // error (vunset > 0 && unset(UNSET)), exactly like the
                            // scalar `$arr`, the `${arr[*]}` splat, and `${arr[1]}`
                            // — all of which already fire it via GET_VAR. The `[@]`
                            // splat path returned an empty array silently, so
                            // `setopt NO_UNSET; print "${arr[@]}"` exited 0 where zsh
                            // exits 1. A DECLARED-but-empty array (`arr=()`) resolves
                            // to `Some(vec![])` above and never reaches here, so it
                            // still splats to nothing without erroring — matching zsh.
                            if opt_state_get("nounset").unwrap_or(false) {
                                crate::ported::utils::zerr(&format!("{}: parameter not set", name));
                                crate::ported::utils::errflag.fetch_or(
                                    crate::ported::zsh_h::ERRFLAG_ERROR,
                                    std::sync::atomic::Ordering::Relaxed,
                                );
                                exec.set_last_status(1);
                            }
                            // c:Src/subst.c:3480-3485 — an UNSET parameter takes the
                            // `vunset` arm: `val = dupstring("")` with isarr left at
                            // 0. That is a SCALAR empty, so plan9 keeps the
                            // surrounding text (`setopt rcexpandparam;
                            // print -r -- "[${unset[@]}]"` → `[]`), unlike a
                            // DECLARED-but-empty array (`arr=()`, matched by the
                            // `Some(vec![])` arm above), which sets isarr and gets
                            // the word deleted at c:4362.
                            note_empty_is_scalar(true);
                            Value::array(vec![])
                        } else if opt_state_get("shwordsplit").unwrap_or(false) {
                            // c:3921 `aval = sepsplit(val, spsep, 0, 1)` — same
                            // splitter as `${=name}` (Src/utils.c:3711 spacesplit),
                            // not a naive `split().filter(non-empty)`: only the
                            // IFS-WHITESPACE-derived empty fields are elided; the
                            // `nulstring` ones an IFS-NON-whitespace separator makes
                            // survive (c:Src/subst.c:36).
                            let nulstring = crate::ported::zsh_h::Nularg.to_string();
                            let parts: Vec<Value> =
                                crate::ported::utils::sepsplit(&val, None, false)
                                    .into_iter()
                                    .filter_map(|w| {
                                        if w == nulstring {
                                            Some(Value::str(String::new()))
                                        } else if w.is_empty() {
                                            None // c:184-187 prefork uremnode
                                        } else {
                                            Some(Value::str(w))
                                        }
                                    })
                                    .collect();
                            Value::array(parts)
                        } else {
                            Value::array(vec![Value::str(val)])
                        }
                    }
                }
            })
        };
        let result = array_all(vm);
        if !matches!(&result, Value::Array(a) if a.is_empty()) {
            note_empty_is_scalar(saved_empty_is_scalar);
        }
        result
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
        let job_text = vm.pop().to_str();
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

        // c:Src/exec.c:1710-1712 — starting a new coproc closes the
        // previous one's fds FIRST:
        //     if (coprocin >= 0) { zclose(coprocin); zclose(coprocout); }
        // The old coproc child then sees EOF on its stdin and exits on
        // its own schedule (its job-table entry stays until it's
        // reaped) — zsh does NOT deletejob it here. This is also what
        // makes the `exec 4<&p; coproc exit; read -u4` EOF idiom work:
        // the replacement coproc closes the shell's write end to the
        // old one.
        {
            use std::sync::atomic::Ordering;
            let old_in = crate::ported::modules::clone::coprocin.load(Ordering::Relaxed);
            if old_in >= 0 {
                let old_out = crate::ported::modules::clone::coprocout.load(Ordering::Relaxed);
                unsafe {
                    libc::close(old_in);
                    if old_out >= 0 {
                        libc::close(old_out);
                    }
                }
                crate::ported::modules::clone::coprocin.store(-1, Ordering::Relaxed);
                crate::ported::modules::clone::coprocout.store(-1, Ordering::Relaxed);
            }
        }

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
        // c:Src/exec.c:5160 mpipe — both pipes' fds are moved above
        // the user-visible range (movefd → F_DUPFD ≥ 10) so the
        // coproc fds never collide with explicit user fds like
        // `exec 3>&p`.
        for fd in p2c.iter_mut().chain(c2p.iter_mut()) {
            *fd = crate::ported::utils::movefd(*fd);
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
            pid => {
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
                // c:Src/exec.c:1725 — `fdtable[coprocin] =
                // fdtable[coprocout] = FDT_UNUSED;`: the two kept ends
                // are user-reachable (via `>&p` / `<&p`), so they drop
                // the FDT_INTERNAL mark movefd gave them.
                crate::ported::utils::fdtable_set(read_fd, crate::ported::zsh_h::FDT_UNUSED);
                crate::ported::utils::fdtable_set(write_fd, crate::ported::zsh_h::FDT_UNUSED);
                // c:Src/exec.c:2837 — `lastpid = (zlong) pid;`. zsh
                // sets the `$!` global to the coproc child's PID so
                // subsequent `$!` reads return it. The Rust port at
                // exec.rs:6773 mirrors this for regular background
                // jobs but the coproc launch path was missing the
                // assignment, leaving `$!` at 0 after `coproc cmd`.
                crate::ported::modules::clone::lastpid
                    .store(pid, std::sync::atomic::Ordering::Relaxed);
                // c:Src/exec.c:1700-1758 — the coproc rides the SAME
                // Z_ASYNC job-table path as `cmd &`: `thisjob = newjob
                // = initjob()` (c:1700), addproc hangs the pid+text
                // proc entry off the job, `jobtab[thisjob].stat |=
                // STAT_NOSTTY` (c:1746), `clearoldjobtab()` (c:1744)
                // and `spawnjob()` (c:1758) promote it to curjob. This
                // is what makes `jobs` list the coproc as
                // `[1]  + running    cat` and `kill %1` resolve it.
                // Mirrors the BUILTIN_RUN_BG parent arm exactly.
                {
                    use crate::ported::jobs;
                    use std::sync::Mutex;
                    let table = jobs::JOBTAB.get_or_init(|| Mutex::new(Vec::new()));
                    let idx = {
                        let mut tab = table.lock().unwrap_or_else(|e| e.into_inner());
                        let idx = jobs::initjob(&mut tab); // c:exec.c:1700
                        jobs::addproc(
                            &mut tab[idx],
                            pid,
                            &job_text,
                            false,
                            Some(std::time::Instant::now()),
                            -1,
                            -1,
                        );
                        tab[idx].stat |= crate::ported::zsh_h::STAT_NOSTTY; // c:exec.c:1746
                        idx
                    };
                    jobs::clearoldjobtab(); // c:exec.c:1744
                    if let Ok(mut tj) = jobs::THISJOB.get_or_init(|| Mutex::new(-1)).lock() {
                        *tj = idx as i32;
                    }
                    jobs::spawnjob(); // c:exec.c:1758
                }
                with_executor(|exec| {
                    exec.jobs
                        .add_pid_job(pid, job_text.clone(), JobState::Running);
                });
                Value::Status(0)
            }
        }
    });

    vm.register_builtin(BUILTIN_ARRAY_FLATTEN, |vm, argc| {
        // `${~spec}` carrier: a `for`/`select` WORD LIST is a word-
        // pipeline boundary too. In C the `globsubst` flag is a
        // paramsubst-LOCAL int (Src/subst.c:1671 `int globsubst =
        // isset(GLOBSUBST);`, forced to 2 by `${~…}` at
        // Src/subst.c:2603) whose only lasting effect is the
        // `shtokenize` of that substitution's own result — it never
        // reaches the option table, so `execfor`'s list prefork
        // (Src/loop.c:196-235) cannot leak it into the loop BODY.
        // zshrs carries the flag through the global option table so
        // the compile-emitted glob ops of the SAME word can see it
        // (documented deviation at subst.rs:3190), and restores it at
        // command-dispatch boundaries — but a `for` list has no
        // trailing dispatch of its own, so `for i in "${a:#${~p}*}"`
        // left GLOB_SUBST ON and filename-generated the FIRST body
        // command's words (`_parameters:34` → `ary+=($i:"$v")` glob-
        // erroring "bad pattern: HISTCHARS:!^#", which aborted the
        // whole `pr<TAB>` completion). This builtin ends EVERY for/
        // select list expansion and runs AFTER each word's
        // GLOB_SUBST_EXPAND op, so the carrier has been read by then.
        consume_tilde_globsubst_carrier();
        let n = argc as usize;
        let start = vm.stack.len().saturating_sub(n);
        let raw: Vec<Value> = vm.stack.drain(start..).collect();
        let mut flat: Vec<Value> = Vec::with_capacity(raw.len());
        for v in raw {
            match v {
                Value::Array(items) => flat.extend(items.iter().cloned()),
                other => flat.push(other),
            }
        }
        let len = flat.len() as i64;
        // Push the array first; the Int(len) becomes the builtin's return
        // value (which CallBuiltin already pushes). Caller consumes in
        // reverse: SetSlot(len_slot) pops Int, SetSlot(arr_slot) pops Array.
        vm.push(Value::array(flat));
        Value::Int(len)
    });

    // Shell variable get/set — routes through executor.variables so nested
    // VMs (function calls) and tree-walker callers see the same storage.
    // GET_VAR / GET_VAR_DQ share one body via `get_var_impl`; the only
    // difference is `force_dq`, which the compiler sets for QUOTED simple
    // reads (`"$name"`) so an array's empty elements are preserved (the
    // `in_dq_context` runtime flag is 0 for these compiler-direct reads).
    fn get_var_impl(vm: &mut fusevm::VM, argc: u8, force_dq: bool) -> Value {
        let args = pop_args(vm, argc);
        let name = args.into_iter().next().unwrap_or_default();
        let live_status = vm.last_status;
        // `$@` and `$*` need splice semantics — return Value::Array of
        // positional params so for-loop's BUILTIN_ARRAY_FLATTEN spreads them
        // and pop_args splits them into argv slots. zsh's `"$@"` bslashquote-each-
        // word semantics matches: each pos-param becomes its own arg.
        // Same for arrays accessed by name (e.g. `$arr` in some contexts).
        //
        // vm.last_status is authoritative: `subshell_end` now returns
        // Some(status) and fusevm's `Op::SubshellEnd` writes it into
        // vm.last_status, so a deferred subshell `exit N` is visible
        // here. Suppressing this sync (as an older revision did, back
        // when the host hook returned nothing) made LASTVAL win over
        // any status the VM set AFTER SubshellEnd — which dropped the
        // `!` negation of `Src/exec.c:1979-1980`
        //   if ((slflags & WC_SUBLIST_NOT) && !errflag && !retflag)
        //       lastval = !lastval;
        // for `! (exit 7)` (emit_negate_status' SetStatus updated
        // vm.last_status, then `$?` read the stale LASTVAL=7).
        let sync_status = |exec: &mut ShellExecutor| {
            exec.set_last_status(live_status);
        };
        if name == "@" || name == "*" {
            // Quoting decides empty-word retention (c:Src/subst.c:
            // 184-187): the COMPILE site knows it and emits
            // BUILTIN_ARRAY_DROP_EMPTY after this read for the
            // unquoted form only — in_dq_context is NOT a valid
            // discriminator here (the quoted "$@" fast path emits
            // GET_VAR directly without an EXPAND_TEXT wrapper).
            return with_executor(|exec| {
                sync_status(exec);
                Value::array(exec.pparams().iter().map(Value::str).collect())
            });
        }
        // RC_EXPAND_PARAM: when the option is set and `name` refers to
        // an array, return Value::Array so the enclosing word's
        // BUILTIN_CONCAT_DISTRIBUTE distributes element-wise. Without
        // the option, arrays still join to a space-separated scalar
        // (zsh's default unquoted-array-as-scalar semantics).
        let rc_expand = with_executor(|exec| opt_state_get("rcexpandparam").unwrap_or(false));
        // c:Src/subst.c — under KSHARRAYS a bare `$name` (no [@]/[*] subscript;
        // this GET_VAR path only handles the bare form) is element 1 ONLY — a
        // scalar. RC_EXPAND_PARAM then has a single value to distribute, so
        // `$acc` → "p1", NOT the whole array. Skip the whole-array rc_expand
        // shortcut when KSHARRAYS is set and fall through to the normal path,
        // which applies the element-1 collapse. Without this gate,
        // `setopt KSH_ARRAYS rc_expand_param; print -r -- $acc` splatted every
        // element while zsh prints just "p1".
        let ksh_arrays = crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS);
        // c:Src/subst.c:4245 `if (isarr)` gates the whole plan9 block
        // (c:4316), and TWO earlier arms have already zeroed `isarr` for a
        // bare array read that is quoted or scalar-substituted:
        //   c:3032 `if (qt && !getlen && isarr > 0) { val = sepjoin(aval,
        //           sep, 1); isarr = 0; }`                       — DQ context
        //   c:3905 `if (nojoin == 0 || sep) { val = sepjoin(aval, sep, 1);
        //           isarr = 0; }` under `if (ssub || …)` at c:3901
        //                                          — PREFORK_SINGLE (scalar
        //                                            assignment RHS)
        // So RC_EXPAND_PARAM never cross-products a plain `"$a"` / `b=$a`;
        // the array collapses to one IFS-joined scalar first. `force_dq` is
        // exactly the compiler's flag for those two contexts (it is set for
        // `in_dq || scalar_assign_depth > 0 || assign_builtin_arg_depth > 0`),
        // Without this gate `setopt rcexpandparam; a=(x y); print -rl -- "$a"Z`
        // emitted `xZ` / `yZ` instead of zsh's single `x yZ` — which is how
        // `_sqlite`'s `"($exclusive)"$^dashes'-header[…]'` reached
        // `comparguments` as five words starting `(-noheader`.
        //
        // Only the compile-time flag is consulted. The runtime
        // `in_dq_context` counter stays set while a `$(…)` INSIDE double
        // quotes runs its body, so reading it here would join an unquoted
        // `$a` in `"$(print -l -- $a)"`.
        if rc_expand && !ksh_arrays && !force_dq {
            let arr_val = with_executor(|exec| {
                sync_status(exec);
                exec.array(&name)
            });
            if let Some(arr) = arr_val {
                // c:4245 — a real array reference (`isarr != 0`). An empty one
                // takes plan9's word-removal path (c:4362), so clear the
                // scalar bit; a preceding empty-SCALAR expansion in the same
                // word would otherwise leave it set and keep the word alive
                // (`empty=''; e=(); a=("$empty"); print -rl -- x$e y`).
                // Only an EMPTY array may clear it: a non-empty one carries
                // no emptiness of its own, and clearing on it wiped the bit a
                // preceding empty SCALAR had set — `setopt rcexpandparam;
                // e=''; a=(x y); print -rl -- $e$a` lost the whole word.
                if arr.is_empty() {
                    note_empty_is_scalar(false);
                }
                return Value::array(arr.into_iter().map(Value::str).collect());
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
            // c:Src/params.c:2293-2296 — KSHARRAYS bare reference
            // collapses to the FIRST element in scan order
            // (`v->end = 1, v->isarr = 0`). For `options` the scan
            // order is optiontab bucket order (OPTIONTAB), so zsh 5.9
            // prints `off` (posixargzero) for
            // `setopt ksharrays; print $options`.
            if opt_state_get("ksharrays").unwrap_or(false) {
                return Value::str(vals.into_iter().next().unwrap_or_default());
            }
            return Value::array(vals.into_iter().map(Value::str).collect());
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
            let in_dq = force_dq || exec.in_dq_context > 0;
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
            if exec.assoc(&name).is_some() {
                // c:Src/params.c:2351-2358 — under KSH EMULATION a bare
                // `$assoc` is `${assoc[0]}` (a KEY-"0" lookup), so it is
                // EMPTY unless the hash actually has a key "0". This is
                // EMULATION-gated, not KSHARRAYS-option-gated: `emulate -L
                // ksh; typeset -A h=(a 1 b 2); print $h` is empty, whereas
                // `setopt ksharrays; …; print $h` collapses to the bucket-
                // first value below.
                if crate::ported::zsh_h::EMULATION(crate::ported::zsh_h::EMULATE_KSH) {
                    let v = crate::ported::subst::assoc_get(&name)
                        .and_then(|m| m.get("0").cloned())
                        .unwrap_or_default();
                    return Some((vec![v], in_dq));
                }
                // c:Src/hashtable.c scanhashtable — a bare `$assoc` joins its
                // VALUES in zsh hash-BUCKET order (the same order `(k)`/`(v)`
                // enumerate), NOT sorted or insertion order. `assoc_get`
                // rebuilds zsh's bucket layout; use it so `$as` matches
                // `${(v)as}` (`as=(zebra 9 apple 1)` → `9 1`, not the
                // alphabetical `1 9`). Under KSHARRAYS the bare form collapses
                // to the bucket-FIRST value (`9`), matching zsh.
                let values: Vec<String> = crate::ported::subst::assoc_get(&name)
                    .map(|m| m.values().cloned().collect())
                    .unwrap_or_default();
                if ksh_arrays {
                    return Some((vec![values.into_iter().next().unwrap_or_default()], in_dq));
                }
                return Some((values, in_dq));
            }
            None
        });
        if let Some((items, in_dq)) = arr_assoc_data {
            // c:Src/subst.c:184-187 — prefork's `else if (!keep)
            // uremnode(list, node)`: UNQUOTED expansion drops empty
            // list nodes before they reach argv, so `a=(y '' x);
            // print -- $a` passes TWO args in zsh (`y x`), while the
            // quoted "${a[@]}" splat keeps the empty slot. The
            // paramsubst splat path already does this (Bug #578
            // retain); this GET_VAR fast path bypassed it and leaked
            // empty argv slots (visible double-space, wrong arg
            // counts in `for`/`print -l`).
            let items: Vec<String> = if in_dq {
                items
            } else {
                items.into_iter().filter(|s| !s.is_empty()).collect()
            };
            if in_dq {
                // c:Src/utils.c:3936-3945 sepjoin default-sep rule:
                // set-but-empty IFS joins with "" (`IFS=""; echo
                // "$arr"` concatenates); only unset / space-leading
                // IFS yields " ". The previous get_variable read
                // couldn't distinguish unset from set-empty.
                return Value::str(crate::ported::utils::sepjoin(&items, None));
            }
            // c:4245 — a real array reference: `isarr != 0`, so an empty
            // one takes plan9's word-removal path, not the scalar path.
            // Only note it when the array IS empty — see the rc_expand arm
            // above for why a non-empty read must not touch the bit.
            if items.is_empty() {
                note_empty_is_scalar(false);
            }
            return Value::array(items.into_iter().map(Value::str).collect());
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
            (v, force_dq || exec.in_dq_context > 0, known)
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
            // c:1650-1656 / c:4437 — a SCALAR parameter has `isarr == 0`,
            // so it never reaches plan9's word-removal at c:4362. Flag the
            // empty Array below as a scalar so `setopt rcexpandparam;
            // v=; print -rl -- x$v y` still emits `x` (only an empty
            // ARRAY deletes the word).
            note_empty_is_scalar(true);
            return Value::array(Vec::new());
        }
        // c:Src/subst.c:1759 SH_WORD_SPLIT — when shwordsplit is set and
        // we're in unquoted command-arg position (not DQ), split scalar
        // value on IFS into multiple words. Matches BUILTIN_ARRAY_ALL's
        // shwordsplit arm (fusevm_bridge.rs:2200). Without this, bare
        // `$s` in `print $s` stayed a single arg even with the option
        // set, breaking POSIX-style scalar word-splitting.
        if !in_dq && opt_state_get("shwordsplit").unwrap_or(false) {
            // c:1705 — `spbreak = (pf_flags & PREFORK_SHWORDSPLIT) && !qt`,
            // then c:3902 `force_split = !ssub && (spbreak || spsep)` and
            // c:3921 `aval = sepsplit(val, spsep, 0, 1)`. SH_WORD_SPLIT runs
            // the SAME splitter as `${=name}`, so route it through the same
            // port. The previous `split(|c| ifs.contains(c)).filter(non-empty)`
            // dropped every empty field, but spacesplit (Src/utils.c:3711)
            // only elides the ones a run of IFS-WHITESPACE produces — an
            // IFS-NON-whitespace separator preserves them as `nulstring`.
            // `IFS=x; v=xaxbx; setopt shwordsplit; print -rl -- $v` is four
            // words in zsh (``, a, b, ``), not two.
            let raw = crate::ported::utils::sepsplit(&val, None, false); // c:3921
            let nulstring = crate::ported::zsh_h::Nularg.to_string(); // c:36
            let parts: Vec<Value> = raw
                .into_iter()
                .filter_map(|w| {
                    if w == nulstring {
                        Some(Value::str(String::new()))
                    } else if w.is_empty() {
                        // c:184-187 — prefork deletes the truly-empty node.
                        None
                    } else {
                        Some(Value::str(w))
                    }
                })
                .collect();
            if parts.is_empty() {
                // c:3922 — `val = dupstring("")`: an empty SCALAR, not an
                // empty array (see EMPTY_EXPANSION_IS_SCALAR).
                note_empty_is_scalar(true);
                return Value::array(Vec::new());
            } else if parts.len() == 1 {
                // c:3924 — `else if (!aval[1]) val = aval[0];`
                return parts.into_iter().next().unwrap();
            } else {
                return Value::array(parts); // c:3927
            }
        }
        Value::str(val)
    }
    vm.register_builtin(BUILTIN_GET_VAR, |vm, argc| get_var_impl(vm, argc, false));
    vm.register_builtin(BUILTIN_GET_VAR_DQ, |vm, argc| get_var_impl(vm, argc, true));

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
            // Existence probe uses the non-cloning `has_array` — the
            // owning `exec.array()` clone here made `arr+=x` in a loop
            // O(n²) (see the assoc-store fix above).
            if exec.has_array(&name) {
                // c:Src/params.c — under KSHARRAYS a bare array name
                // addresses element 0 (ksh), so `a+=X` (scalar augment)
                // CONCATENATES onto the first element ("firstlast second"),
                // it does NOT push a new element. C routes scalar `+=`
                // through assignsparam (which targets the elem-0 value);
                // zshrs's APPEND_SCALAR_OR_PUSH would otherwise push.
                if crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS) {
                    let mut arr = exec.array(&name).unwrap_or_default();
                    if arr.is_empty() {
                        arr.push(value.clone());
                    } else {
                        arr[0] = format!("{}{}", arr[0], value);
                    }
                    exec.set_array(name.clone(), arr);
                    #[cfg(feature = "recorder")]
                    if crate::recorder::is_enabled() && exec.local_scope_depth == 0 {
                        let ctx = exec.recorder_ctx();
                        let attrs = exec.recorder_attrs_for(&name);
                        emit_path_or_assign(&name, std::slice::from_ref(&value), attrs, true, &ctx);
                    }
                    return;
                }
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
            if exec.has_assoc(&name) {
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
    // Sets the GLOB_ASSIGN-eligibility flag consumed by the NEXT BUILTIN_SET_VAR.
    // Emitted only when a scalar-assign RHS had an unquoted glob token. Takes no
    // args; its pushed return is discarded by a following Op::Pop.
    vm.register_builtin(BUILTIN_MARK_GLOB_ELIGIBLE, |_vm, _argc| {
        SET_VAR_GLOB_ELIGIBLE.with(|c| c.set(true));
        fusevm::Value::Int(0)
    });
    vm.register_builtin(BUILTIN_SET_VAR, |vm, argc| {
        // `${~spec}` carrier: an assignment statement is a word-
        // pipeline boundary too — restore the user's GLOB_SUBST
        // before the NEXT word expands (`Z[d]=${~Z[d]}; print
        // ${options[globsubst]}` must read the user value).
        consume_tilde_globsubst_carrier();
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
        let mut assign_failed = false;
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
            // command return). Only the PREFIX assignments belong in
            // the frame: c:Src/exec.c:4410 save_params snapshots the
            // parsed WC_ASSIGN chain and nothing else. The frame stays
            // on the stack while the command runs, so gate on
            // `recording` (cleared by SEAL_INLINE_ENV once the prefix
            // assignments have committed) — otherwise every assignment
            // the command itself makes gets recorded and then reverted
            // (`X=y . file` wiped every global the file defined).
            if exec
                .inline_env_stack
                .last()
                .is_some_and(|frame| frame.recording)
            {
                let prev_var = crate::ported::params::getsparam(&name);
                let prev_env = env::var(&name).ok();
                exec.inline_env_stack.last_mut().unwrap().saved.push((
                    name.clone(),
                    prev_var,
                    prev_env,
                ));
                let _ = crate::ported::params::zputenv(&format!("{}={}", &name, &value));
                // c:Src/params.c:5354
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
                    assign_failed = crate::ported::params::setsparam(&name, &value).is_none();
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
                    assign_failed = crate::ported::params::setsparam(&name, &value).is_none();
                }
            } else {
                // c:Src/exec.c:2554-2567 — GLOB_ASSIGN. When the
                // `globassign` option is on and the scalar RHS is a glob
                // pattern, glob it and recreate the parameter as a scalar
                // (≤1 match) or array (>1) — csh-style assignment. The
                // bridge hands `value` UNTOKENIZED, so re-tokenize
                // (shtokenize) before haswilds/globlist; zsh's wordcode
                // value arrives pre-tokenized via `htok`. The
                // `isset(GLOBASSIGN)` gate is first and cheap (option off
                // by default), so the common path is unchanged.
                let mut globbed = false;
                // Only glob the RHS when the compiler flagged an UNQUOTED glob
                // token in the literal wordcode (SET_VAR_GLOB_ELIGIBLE). zsh's
                // GLOB_ASSIGN (Src/exec.c:2554) globs literal patterns only —
                // `x="/tmp/*"`, `x='/tmp/*'`, `x=$param`, `x=$(cmd)` all assign
                // verbatim. The value arrives here untokenized (DQ-wrapped by
                // the compiler), so this compile-time flag is the only surviving
                // signal of whether the pattern was quote-protected.
                let glob_eligible = SET_VAR_GLOB_ELIGIBLE.with(|c| c.replace(false));
                if glob_eligible && crate::ported::zsh_h::isset(crate::ported::zsh_h::GLOBASSIGN) {
                    let mut tv = value.clone();
                    crate::ported::glob::shtokenize(&mut tv);
                    if crate::ported::pattern::haswilds(&tv) {
                        // Committed to the glob path: never fall back to
                        // assigning the literal pattern (zsh errors on
                        // no-match instead).
                        globbed = true;
                        // globlist tokenizes its input internally (for
                        // haswilds + glob_path) and prints the ORIGINAL
                        // string verbatim in its "no matches found" error,
                        // so feed it the UNtokenized value — passing the
                        // tokenized form would leak the Star/Quest token
                        // bytes into the error message.
                        let mut ll: crate::ported::linklist::LinkList<String> = Default::default();
                        ll.push_back(value.clone());
                        crate::ported::subst::globlist(&mut ll, 0); // c:2556
                        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed)
                            == 0
                        {
                            let matches: Vec<String> = ll
                                .nodes
                                .iter()
                                .map(|s| crate::ported::lex::untokenize(s).to_string())
                                .collect();
                            crate::ported::params::unsetparam(&name); // c:2562
                            if matches.len() <= 1 {
                                let v = matches.into_iter().next().unwrap_or_default();
                                assign_failed =
                                    crate::ported::params::setsparam(&name, &v).is_none();
                            } else {
                                crate::ported::params::setaparam(&name, matches);
                            }
                        }
                        // errflag set → globlist already reported
                        // "no matches found"; leave the param unassigned
                        // to match zsh's abort.
                    }
                }
                // c:Src/exec.c addvars — a NULL return from
                // assignsparam (e.g. nameref resolving out of scope,
                // createparam refusal at c:1108-1118) fails the
                // assignment with status 1.
                if !globbed {
                    assign_failed = crate::ported::params::setsparam(&name, &value).is_none();
                }
            }
            // PM_EXPORTED / allexport env mirror — read AFTER setsparam
            // so the flag bit reflects any GSU setfn side-effects.
            let allexport = opt_state_get("allexport").unwrap_or(false);
            let already_exported =
                (exec.param_flags(&name) as u32 & crate::ported::zsh_h::PM_EXPORTED) != 0;
            if allexport || already_exported {
                // c:Src/params.c:3024 — the env mirror is `addenv(pm, value)`,
                // and addenv builds its string with `mkenvstr(nam, value,
                // pm->flags)` (c:5463) so `copyenvstr` (c:5434) can apply the
                // PM_LOWER / PM_UPPER fold. Formatting `name=value` by hand
                // skipped that: `typeset -lx v; v=HeLLo` exported `HeLLo`
                // where zsh exports `hello`. The fold has to happen HERE
                // because the paramtab now stores the value verbatim.
                let envstr = crate::ported::params::mkenvstr(
                    &name,
                    &value,
                    exec.param_flags(&name), // c:5463 pm->flags
                );
                let _ = crate::ported::params::zputenv(&envstr); // c:Src/params.c:5354
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
            // c:Src/exec.c:1367-1370 — `if (code == WC_ASSIGN) { cmdoutval = 0;
            // addvars(state, state->pc - 1, 0); setunderscore(""); … }`. A
            // simple command consisting ONLY of scalar assignments clears `$_`;
            // it never goes through execcmd's c:3545-3547
            // `setunderscore(lastnode(args))`. src/ported/exec.rs:6946-6950
            // already ports that arm, but the WC_ASSIGN wordcode never
            // executes under fusevm — a bare `x=1` arrives here as
            // BUILTIN_SET_VAR, so `$_` kept the PREVIOUS command's last
            // argument. Symptom: in the `unset <TAB>` listing (the user's
            // `_parameters` override runs `maxLen=50` right before the
            // `$parameters` walk) zsh shows `_` empty while zshrs showed the
            // completion-internal `^a*` — `_parameters -g '^a*'`'s last arg.
            //
            // Two exclusions, both verified against zsh 5.9.2
            // (`true aa; <form>; print -r -- "[$_]"`):
            //   * PREFIX assignments (`x=1 true dd` → `dd`) are part of a
            //     command, so c:3545-3547 owns `$_`. They are exactly the
            //     assignments recorded into an open inline-env frame above.
            //   * `(( q = 1 ))` (→ `aa`, unchanged) is WC_ARITH, not
            //     WC_ASSIGN; the arith paths are the only ones that hand this
            //     builtin an Int/Float `Value` (see the `int_assign` note).
            if !int_assign
                && !float_assign
                && !exec
                    .inline_env_stack
                    .last()
                    .is_some_and(|frame| frame.recording)
            {
                // c:1369 — assignment-only command clears `$_`. The
                // former DUAL-STATE note here is obsolete: `set_zunderscore`
                // and the ported `setunderscore` now write the SAME
                // `init::zunderscore` global (params.rs `zunderscore_lock`
                // points at it), matching C's single store, so this one call
                // is the whole effect.
                crate::ported::exec::setunderscore(""); // c:1369
            }
        });
        Value::Status(vm.last_status)
    });

    // c:Src/exec.c execfor → Src/params.c:6362 setloopvar — the
    // for-loop variable bind. Distinct from BUILTIN_SET_VAR because a
    // PM_NAMEREF loop variable REBINDS (new refname) instead of
    // assigning through the resolved chain.
    vm.register_builtin(BUILTIN_SET_LOOP_VAR, |vm, argc| {
        let args = pop_args(vm, argc);
        let name = args.first().cloned().unwrap_or_default();
        let value = args.get(1).cloned().unwrap_or_default();
        if crate::vm_helper::is_nameref(&name) {
            let ef_before =
                crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed);
            crate::ported::params::setloopvar(&name, &value); // c:6362
            let ef_after = crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed);
            if (ef_after & crate::ported::utils::ERRFLAG_ERROR) != 0 && ef_after != ef_before {
                // zerr fired (read-only reference / invalid self
                // reference) — abort the loop, status 1 (C errflag).
                vm.last_status = 1;
                return Value::Bool(false);
            }
            return Value::Bool(true);
        }
        // Plain loop var — canonical scalar path (same shape as
        // BUILTIN_SET_VAR's setsparam arm).
        with_executor(|exec| {
            if exec.is_readonly_param(&name) {
                crate::ported::utils::zerr(&format!("read-only variable: {}", name));
                return;
            }
            crate::ported::params::setsparam(&name, &value);
            let allexport = opt_state_get("allexport").unwrap_or(false);
            let already_exported =
                (exec.param_flags(&name) as u32 & crate::ported::zsh_h::PM_EXPORTED) != 0;
            if allexport || already_exported {
                // c:Src/params.c:3024 — the env mirror is `addenv(pm, value)`,
                // and addenv builds its string with `mkenvstr(nam, value,
                // pm->flags)` (c:5463) so `copyenvstr` (c:5434) can apply the
                // PM_LOWER / PM_UPPER fold. Formatting `name=value` by hand
                // skipped that: `typeset -lx v; v=HeLLo` exported `HeLLo`
                // where zsh exports `hello`. The fold has to happen HERE
                // because the paramtab now stores the value verbatim.
                let envstr = crate::ported::params::mkenvstr(
                    &name,
                    &value,
                    exec.param_flags(&name), // c:5463 pm->flags
                );
                let _ = crate::ported::params::zputenv(&envstr); // c:Src/params.c:5354
            }
        });
        Value::Bool(true)
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
        // c:Src/cond.c:361 `case 'v': return !issetvar(left)`. `-v` is
        // NOT `${+name}` — issetvar (params.c:751) additionally rejects
        // trailing chars after the parsed name/subscript (`arr[3]extra`,
        // nested `arr[2][1]`) and validates array-slice bounds (an
        // out-of-range `(i)`-not-found index is "unset"). `${+}` is
        // lenient and reported those as set.
        Value::Bool(crate::ported::params::issetvar(&name) != 0)
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
        // c:Src/jobs.c — zsh's `time` reports only for JOBS (forked
        // work). Builtins/brace-groups/functions run in the shell
        // process with no job, so `zsh -fc 'time true'` emits NOTHING.
        // Snapshot the fork-event counter; report only if the timed
        // body forked (external command or subshell).
        let forks_before = crate::vm_helper::FORK_EVENTS.load(std::sync::atomic::Ordering::Relaxed);
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
        // c:Src/jobs.c — no job, no report (see forks_before above).
        let forked = crate::vm_helper::FORK_EVENTS.load(std::sync::atomic::Ordering::Relaxed)
            != forks_before;
        if forked {
            let line = crate::ported::jobs::printtime(elapsed.as_secs_f64(), &ti, &fmt, &desc);
            eprintln!("{}", line);
        }
        Value::Status(status)
    });

    // `{name}>file` / `{name}<file` / `{name}>>file` — named-fd allocator.
    // Stack: [path, varid, op_byte]. Opens path with the appropriate mode
    // and stores the resulting fd number in $varid as a string. We use
    // a high starting fd (10+) by allocating then dup'ing — matches zsh's
    // "fresh fd >= 10" promise so subsequent commands don't collide on
    // stdin/out/err.
    vm.register_builtin(BUILTIN_OPEN_NAMED_FD, |vm, _argc| {
        use std::sync::atomic::Ordering;
        let op_byte = vm.pop().to_int() as u8;
        let varid = vm.pop().to_str();
        let path = vm.pop().to_str();
        // Param introspection used by both the open and close forms.
        let param_flags = crate::ported::params::paramtab()
            .read()
            .ok()
            .and_then(|t| t.get(&varid).map(|p| p.node.flags));
        let param_readonly = param_flags
            .map(|f| (f & crate::ported::zsh_h::PM_READONLY as i32) != 0)
            .unwrap_or(false);
        // `{varid}>&-` / `{varid}<&-` — REDIR_CLOSE with varid.
        // Direct port of Src/exec.c:3805-3850.
        if matches!(
            op_byte,
            b if b == fusevm::op::redirect_op::DUP_WRITE
                || b == fusevm::op::redirect_op::DUP_READ
        ) {
            let n = path.trim_start_matches('&');
            if n == "-" {
                let val = with_executor(|exec| exec.scalar(&varid)).unwrap_or_default();
                let fd1 = val.parse::<i32>();
                // c:3811-3816 — bad=1: parameter doesn't contain an fd.
                let Ok(fd1) = fd1 else {
                    crate::ported::utils::zwarn(&format!(
                        "parameter {} does not contain a file descriptor",
                        varid
                    ));
                    with_executor(|exec| exec.redirect_failed = true);
                    return Value::Status(1);
                };
                // c:3813-3814 — bad=2: readonly parameter.
                if param_readonly {
                    crate::ported::utils::zwarn(&format!(
                        "can't close file descriptor from readonly parameter {}",
                        varid
                    ));
                    with_executor(|exec| exec.redirect_failed = true);
                    return Value::Status(1);
                }
                // c:3830-3835 — bad=3: fd >= 10 marked FDT_INTERNAL.
                if fd1 >= 10
                    && fd1 <= crate::ported::utils::MAX_ZSH_FD.load(Ordering::Relaxed)
                    && crate::ported::utils::fdtable_get(fd1) == crate::ported::zsh_h::FDT_INTERNAL
                {
                    crate::ported::utils::zwarn(&format!(
                        "file descriptor {} used by shell, not closed",
                        fd1
                    ));
                    with_executor(|exec| exec.redirect_failed = true);
                    return Value::Status(1);
                }
                // c:3870-3873 — close; report failure (varid form
                // always reports, unlike bare `N>&-`).
                if crate::ported::utils::zclose(fd1) < 0 {
                    crate::ported::utils::zwarn(&format!(
                        "failed to close file descriptor {}: {}",
                        fd1,
                        std::io::Error::last_os_error()
                    ));
                    return Value::Status(1);
                }
                return Value::Status(0);
            }
            // `{varid}>&N` — dup N to a fresh fd >= 10, store in varid.
            if let Ok(src) = n.parse::<i32>() {
                if param_readonly {
                    crate::ported::utils::zwarn(&format!(
                        "can't allocate file descriptor to readonly parameter {}",
                        varid
                    ));
                    with_executor(|exec| exec.redirect_failed = true);
                    return Value::Status(1);
                }
                let dup = unsafe { libc::fcntl(src, libc::F_DUPFD, 10) };
                if dup < 0 {
                    crate::ported::utils::zwarn(&format!("{}: bad file descriptor", src));
                    with_executor(|exec| exec.redirect_failed = true);
                    return Value::Status(1);
                }
                // c:2404-2412 addfd varid arm — movefd + FDT_EXTERNAL.
                let final_fd = crate::ported::utils::movefd(dup);
                crate::ported::utils::fdtable_set(final_fd, crate::ported::zsh_h::FDT_EXTERNAL);
                with_executor(|exec| {
                    exec.set_scalar(varid, final_fd.to_string());
                });
                return Value::Status(0);
            }
            return Value::Status(1);
        }
        // `{varid}<<HERE` / `{varid}<<<str` — op byte 255 (zshrs-side
        // contract with compile_redir; fusevm's redirect_op stops at
        // 8). C path: gethere/getherestr write the body to a temp
        // file (Src/exec.c:4660-4682), then addfd's varid arm moves
        // the read fd >= 10, marks FDT_EXTERNAL and sets the param
        // (c:2402-2412). `path` carries the BODY text here.
        if op_byte == 255 {
            if param_readonly {
                crate::ported::utils::zwarn(&format!(
                    "can't allocate file descriptor to readonly parameter {}",
                    varid
                ));
                with_executor(|exec| exec.redirect_failed = true);
                return Value::Status(1);
            }
            let body = format!("{}\n", path.trim_end_matches('\n'));
            let mut tmpl: Vec<u8> = b"/tmp/zshrs_hd_XXXXXX\0".to_vec();
            let write_fd = unsafe { libc::mkstemp(tmpl.as_mut_ptr() as *mut libc::c_char) };
            if write_fd < 0 {
                crate::ported::utils::zwarn(&format!(
                    "can't create temp file for here document: {}",
                    std::io::Error::last_os_error()
                ));
                return Value::Status(1);
            }
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
            unsafe { libc::close(write_fd) };
            let read_fd =
                unsafe { libc::open(tmpl.as_ptr() as *const libc::c_char, libc::O_RDONLY) };
            unsafe { libc::unlink(tmpl.as_ptr() as *const libc::c_char) };
            if read_fd < 0 {
                return Value::Status(1);
            }
            let final_fd = crate::ported::utils::movefd(read_fd);
            if final_fd < 0 {
                return Value::Status(1);
            }
            crate::ported::utils::fdtable_set(final_fd, crate::ported::zsh_h::FDT_EXTERNAL);
            with_executor(|exec| {
                exec.set_scalar(varid, final_fd.to_string());
            });
            return Value::Status(0);
        }
        // Open form: `{varid}>file` etc.
        // c:Src/exec.c:2177-2215 checkclobberparam — gate BEFORE open.
        if param_readonly {
            // c:2191-2197
            crate::ported::utils::zwarn(&format!(
                "can't allocate file descriptor to readonly parameter {}",
                varid
            ));
            with_executor(|exec| exec.redirect_failed = true);
            return Value::Status(1);
        }
        // c:2199-2213 — NO_CLOBBER refuses to overwrite a parameter
        // already holding an OPEN fd (decimal value, fdtable says
        // FDT_EXTERNAL).
        if !isset(crate::ported::zsh_h::CLOBBER) && op_byte != fusevm::op::redirect_op::CLOBBER {
            if let Some(val) = with_executor(|exec| exec.scalar(&varid)) {
                if let Ok(fd) = val.parse::<i32>() {
                    if fd >= 0
                        && fd <= crate::ported::utils::MAX_ZSH_FD.load(Ordering::Relaxed)
                        && crate::ported::utils::fdtable_get(fd)
                            == crate::ported::zsh_h::FDT_EXTERNAL
                    {
                        crate::ported::utils::zwarn(&format!(
                            "can't clobber parameter {} containing file descriptor {}",
                            varid, fd
                        ));
                        with_executor(|exec| exec.redirect_failed = true);
                        return Value::Status(1);
                    }
                }
            }
        }
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
        let fd = unsafe { libc::open(path_c.as_ptr(), flags, 0o666) };
        if fd < 0 {
            // c:Src/exec.c:3790-3795 — report the open failure and mark the
            // redirect failed so the command is SKIPPED, matching the numeric-fd
            // (`3< file`) and non-varid (`< file`) paths. Previously this
            // silently returned Status(1) with no diagnostic and no
            // redirect_failed flag, so `{fd}< /nonexistent` ran the command
            // anyway with exit 0 (zsh errors "no such file or directory" +
            // skips the command). `%e: %s` = strerror(errno) : filename.
            let e = std::io::Error::last_os_error();
            let msg = redir_errno_msg(&e);
            crate::ported::utils::zwarn(&format!("{}: {}", msg, path));
            with_executor(|exec| exec.redirect_failed = true);
            return Value::Status(1);
        }
        // c:2404-2412 addfd varid arm — `fd1 = movefd(fd2);
        // fdtable[fd1] = FDT_EXTERNAL; setiparam(varid, fd1);`.
        // FDT_EXTERNAL (not INTERNAL): the user owns this fd — the
        // NO_CLOBBER gate above and `{fd}>&-` close both key off it.
        let final_fd = crate::ported::utils::movefd(fd);
        if final_fd < 0 {
            crate::ported::utils::zerr(&format!(
                "cannot move fd {}: {}",
                fd,
                std::io::Error::last_os_error()
            ));
            return Value::Status(1);
        }
        crate::ported::utils::fdtable_set(final_fd, crate::ported::zsh_h::FDT_EXTERNAL);
        let _ = Ordering::Relaxed;
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
        // c:Src/exec.c WC_TRYBLOCK — the always-arm runs with a
        // clean escape state. Snapshot RETFLAG / BREAKS / CONTFLAG /
        // EXIT_PENDING here and clear them; RESTORE_TRY_BLOCK_STATUS
        // re-applies them at always-arm exit so the propagation jump
        // emitted by compile_zsh fires correctly.
        let ret_save = crate::ported::builtin::RETFLAG.swap(0, Ordering::Relaxed); // c:769-770
        let brk_save = crate::ported::builtin::BREAKS.swap(0, Ordering::Relaxed); // c:771-772
        let cont_save = crate::ported::builtin::CONTFLAG.swap(0, Ordering::Relaxed); // c:773-774
        let exit_save = crate::ported::builtin::EXIT_PENDING.swap(0, Ordering::Relaxed);
        // c:Src/loop.c:762-763 — `save_try_errflag = try_errflag;
        // save_try_interrupt = try_interrupt;`. Restored at c:778-779
        // by RESTORE_TRY_BLOCK_STATUS so a nested try block doesn't
        // clobber the enclosing one's `$TRY_BLOCK_ERROR`.
        let try_err_save = crate::ported::r#loop::try_errflag.load(Ordering::Relaxed); // c:762
        let try_int_save = crate::ported::r#loop::try_interrupt.load(Ordering::Relaxed); // c:763
        TRY_ESCAPE_SAVE.with(|s| {
            s.borrow_mut().push((
                ret_save,
                brk_save,
                cont_save,
                exit_save,
                try_err_save,
                try_int_save,
            ));
        });
        // c:Src/loop.c:764-766 — `try_errflag = (zlong)(errflag &
        // ERRFLAG_ERROR); try_interrupt = (zlong)((errflag &
        // ERRFLAG_INT) ? 1 : 0);`. Both are the RAW FLAG BITS, not the
        // try-list's exit status: ERRFLAG_ERROR is 1 (zsh.h:2972), so
        // `$TRY_BLOCK_ERROR` is 1-or-0 in zsh regardless of what the
        // failing command's `$?` was. The try-list's status is carried
        // separately in `__zshrs_try_block_saved_status`.
        let live_errflag = crate::ported::utils::errflag.load(Ordering::Relaxed);
        let try_err = (live_errflag & crate::ported::zsh_h::ERRFLAG_ERROR) as i64; // c:765
        let try_int = if (live_errflag & crate::ported::zsh_h::ERRFLAG_INT) != 0 {
            1i64
        } else {
            0i64
        }; // c:766
        crate::ported::r#loop::try_errflag.store(try_err, Ordering::Relaxed); // c:765
        crate::ported::r#loop::try_interrupt.store(try_int, Ordering::Relaxed); // c:766
                                                                                // c:Src/loop.c:755 — `endval = lastval ? lastval : errflag;`.
                                                                                // The status of the WHOLE `{…} always {…}` construct, captured
                                                                                // BEFORE the always-list runs (exectry returns it at c:801) and
                                                                                // deliberately including the errflag fallback: a try-list that
                                                                                // failed with `lastval == 0` but raised errflag still reports 1.
        let endval = if vm_status != 0 {
            vm_status
        } else {
            live_errflag
        }; // c:755
        with_executor(|exec| {
            // flags=0 (not setsparam's ASSPM_WARN): VM-internal scratch —
            // must never surface as a WARN_CREATE_GLOBAL diagnostic inside
            // a user function running `{...} always {...}` (f-sy-h's
            // `_zsh_highlight` does exactly that under warncreateglobal).
            crate::ported::params::assignsparam(
                "__zshrs_try_block_saved_status",
                &endval.to_string(),
                0,
            );
            let _ = exec;
            // Mirror into paramtab so `${parameters[TRY_BLOCK_ERROR]}`
            // and the PM_INTEGER `u_val` shadow agree with the atomic
            // the special-var getter reads. (setsparam → intsetfn's
            // TRY_BLOCK_ERROR arm re-stores the same value.)
            exec.set_scalar("TRY_BLOCK_ERROR".to_string(), try_err.to_string());
            exec.set_scalar("TRY_BLOCK_INTERRUPT".to_string(), try_int.to_string());
        });
        // c:Src/loop.c:768 — `errflag = 0;` ("We need to reset all
        // errors to allow the block to execute"). C clears the WHOLE
        // word, not just ERRFLAG_ERROR.
        crate::ported::utils::errflag.store(0, Ordering::Relaxed); // c:768
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
            exec.inline_env_stack
                .push(crate::vm_helper::InlineEnvFrame::new());
        });
        Value::Status(0)
    });
    // Closes the frame's save list — see BUILTIN_SEAL_INLINE_ENV.
    vm.register_builtin(BUILTIN_SEAL_INLINE_ENV, |_vm, _argc| {
        with_executor(|exec| {
            if let Some(frame) = exec.inline_env_stack.last_mut() {
                frame.recording = false;
            }
        });
        Value::Status(0)
    });
    vm.register_builtin(BUILTIN_END_INLINE_ENV, |_vm, _argc| {
        with_executor(|exec| {
            if let Some(frame) = exec.inline_env_stack.pop() {
                for (name, prev_var, prev_env) in frame.saved.into_iter().rev() {
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
    // c:Src/exec.c:3969-3976 — bare-exec assignment epilogue: see the
    // const's doc block. POSIX_BUILTINS → assignments persist (pop the
    // frame, discard the saved state); otherwise → restore_params
    // (same walk as END_INLINE_ENV).
    vm.register_builtin(BUILTIN_EXEC_INLINE_ENV_DONE, |_vm, _argc| {
        let persist = isset(crate::ported::zsh_h::POSIXBUILTINS);
        with_executor(|exec| {
            if let Some(frame) = exec.inline_env_stack.pop() {
                if persist {
                    return; // c:3971 — no save/restore under POSIX_BUILTINS
                }
                for (name, prev_var, prev_env) in frame.saved.into_iter().rev() {
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
        // c:Src/loop.c:801 — `return endval;`. The construct's exit
        // status is the try-list's (captured at c:755 by
        // SET_TRY_BLOCK_ERROR), never the always-list's.
        let saved = with_executor(|exec| {
            exec.scalar("__zshrs_try_block_saved_status")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0)
        });
        // c:Src/exec.c:1375 — `lastval = lv;` on the exectry return.
        // The always-list's own commands left their status in LASTVAL
        // (`always { : }` → 0); without this store the errflag re-raise
        // below aborts the shell with the always-list's 0 instead of
        // the try-list's failure status.
        crate::ported::builtin::LASTVAL.store(saved, Ordering::Relaxed); // c:1375
                                                                         // c:Src/loop.c:774-777 — the error RE-RAISE. This is the
                                                                         // whole point of TRY_BLOCK_ERROR being writable:
                                                                         //
                                                                         //     if (try_errflag)  errflag |= ERRFLAG_ERROR;
                                                                         //     else              errflag &= ~ERRFLAG_ERROR;
                                                                         //     if (try_interrupt) errflag |= ERRFLAG_INT;
                                                                         //     else               errflag &= ~ERRFLAG_INT;
                                                                         //
                                                                         // SET_TRY_BLOCK_ERROR cleared errflag (c:768) so the always-arm
                                                                         // could run; the try-block's error is PARKED in `try_errflag`
                                                                         // and re-raised HERE unless the always-arm zeroed it
                                                                         // (`TRY_BLOCK_ERROR=0`, the documented swallow idiom — routed
                                                                         // to the atomic by intsetfn's IPDEF6 arm, params.rs).
                                                                         //
                                                                         // zshrs used to just drop the parked error, so
                                                                         // `f() { { typeset -r ro=1; ro=2 } always { … }; print reached }`
                                                                         // kept running and exited 0, where zsh aborts f with status 1.
        let te = crate::ported::r#loop::try_errflag.load(Ordering::Relaxed); // c:774
        let ti = crate::ported::r#loop::try_interrupt.load(Ordering::Relaxed); // c:776
        if te != 0 {
            crate::ported::utils::errflag
                .fetch_or(crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
        // c:775
        } else {
            crate::ported::utils::errflag
                .fetch_and(!crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
            // c:777
        }
        if ti != 0 {
            crate::ported::utils::errflag
                .fetch_or(crate::ported::zsh_h::ERRFLAG_INT, Ordering::Relaxed);
        // c:779
        } else {
            crate::ported::utils::errflag
                .fetch_and(!crate::ported::zsh_h::ERRFLAG_INT, Ordering::Relaxed);
            // c:781
        }
        // Re-apply the escape flags captured by SET_TRY_BLOCK_ERROR.
        // If the always-arm itself fired return/break/continue/exit,
        // its handler already overwrote the canonical atomics; let
        // those win — the always-arm's own escape always takes
        // priority over the try-block's deferred one.
        if let Some((ret, brk, cont, exit_p, try_err_save, try_int_save)) =
            TRY_ESCAPE_SAVE.with(|s| s.borrow_mut().pop())
        {
            // c:Src/loop.c:782-783 — `try_errflag = save_try_errflag;
            // try_interrupt = save_try_interrupt;`
            crate::ported::r#loop::try_errflag.store(try_err_save, Ordering::Relaxed); // c:782
            crate::ported::r#loop::try_interrupt.store(try_int_save, Ordering::Relaxed);
            // c:783
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

    // `[[ -r/-w/-x file ]]` — the cond path must use access(2) (the
    // C-faithful doaccess), NOT fusevm's generic Op::TestFile which only
    // checks existence for -r/-w (so a `chmod 000` file read as readable;
    // C02cond.ztst:13). Stack: [path, mode]; mode is the access(2) bit
    // (R_OK=4, W_OK=2, X_OK=1). Mirrors cond.rs:232/238/267 `doaccess`.
    vm.register_builtin(BUILTIN_COND_ACCESS, |vm, _argc| {
        let mode = vm.pop().to_int() as i32;
        let path = vm.pop().to_str();
        Value::Bool(crate::ported::cond::doaccess(&path, mode) != 0)
    });

    // `[[ -prefix PAT ]]` / `-suffix` / `-after` / `-between` module condition.
    // Stack (pushed by the ModCond compile arm): arg0 … argN-1, then the
    // operator word last. argc = N+1.
    // c:Src/subst.c:4419-4420 `if (globsubst) shtokenize(y)` — see the
    // BUILTIN_COND_SHTOKENIZE doc for why a module condition needs it.
    vm.register_builtin(BUILTIN_COND_SHTOKENIZE, |vm, _argc| {
        let mut s = vm.pop().to_str();
        crate::ported::glob::shtokenize(&mut s);
        Value::str(s)
    });

    vm.register_builtin(BUILTIN_COND_MOD, |vm, argc| {
        use crate::ported::zle::complete::{cond_psfix, cond_range, CVT_PREPAT, CVT_SUFPAT};
        let op = vm.pop().to_str(); // operator word (pushed last → popped first)
        let n = (argc as usize).saturating_sub(1);
        let mut args: Vec<String> = Vec::with_capacity(n);
        for _ in 0..n {
            args.push(vm.pop().to_str());
        }
        args.reverse(); // restore arg0 … argN-1 order
                        // Dispatch the module/completion condition (C evalcond COND_MOD path:
                        // condtab lookup + arity check, cond.c:149-185, over the four cotab[]
                        // entries at complete.c:1697-1702). Handlers return 1=match/true.
        let name: String = op
            .trim_start_matches(|c: char| c == '-' || c == '\u{9b}')
            .to_string();
        // c:Src/cond.c:149-150 — `cd = getconddef((ctype == COND_MODI),
        // name + 1, 1)`. The `autol = 1` argument is what makes an
        // autoloadable condition LOAD its module: `getconddef`
        // (Src/module.c:647) sees the `c:`-stub's `p->module` and calls
        // `ensurefeature(p->module, "c:", name)`, then re-looks-up the
        // now-real definition that `zsh/complete`'s cotab installed.
        // Without this call zshrs answered `[[ -prefix … ]]` straight
        // from the compiled-in handler table and left `zsh/complete`
        // unloaded, where `zsh -f` reports it loaded after the first use.
        // `try_lock`: every other MODULESTAB caller holds the same mutex
        // for a moment and the load chain re-enters it; falling through
        // to the compiled-in table is the safe outcome, not a deadlock.
        let cd = match crate::ported::module::MODULESTAB.try_lock() {
            Ok(mut tab) => crate::ported::module::getconddef(0, &name, 1, &mut tab), // c:150
            Err(_) => None,
        };
        // c:151-155 — arity check against the conddef's own min/max
        // (`if (l < cd->min || (cd->max >= 0 && l > cd->max))`). The
        // fallback pins the same numbers the cotab rows carry
        // (complete.c:1698-1701) for the window before zsh/complete is
        // loaded, when `getconddef` has only the module-less stub.
        let (min, max): (usize, usize) = match cd.as_ref() {
            Some(c) if c.max >= 0 => (c.min.max(0) as usize, c.max as usize), // c:152
            _ => match name.as_str() {
                "prefix" | "suffix" => (1, 2), // c:1700-1701
                "after" => (1, 1),             // c:1698
                "between" => (2, 2),           // c:1699
                _ => {
                    crate::ported::utils::zerr(&format!(
                        "unknown condition: {}",
                        op.replace('\u{9b}', "-")
                    ));
                    return Value::Bool(false);
                }
            },
        };
        if args.len() < min || args.len() > max {
            crate::ported::utils::zerr(&format!(
                "unknown condition: {}",
                op.replace('\u{9b}', "-")
            ));
            return Value::Bool(false);
        }
        // c:158 — `return !cd->handler(strs, cd->condid);`
        let r = match cd.as_ref().and_then(|c| c.handler.map(|h| (h, c.condid))) {
            Some((handler, condid)) => handler(&args, condid),
            // Pre-load window: zsh/complete's cotab is not installed yet,
            // so dispatch through the compiled-in handlers directly.
            None => match name.as_str() {
                "prefix" => cond_psfix(&args, CVT_PREPAT),
                "suffix" => cond_psfix(&args, CVT_SUFPAT),
                "after" => cond_range(&args, 0),
                "between" => cond_range(&args, 1),
                _ => 0,
            },
        };
        Value::Bool(r == 1)
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

    // c:Src/exec.c:4918/5040/5069 — a process substitution used inside a
    // `[[ … ]]` cond operand errors "process substitution %s cannot be
    // used here" (getoutputfile/getproc run with thisjob == -1). Emitted
    // by the compiler in place of ProcessSubIn/Out when in_cond_operand.
    vm.register_builtin(BUILTIN_PROCSUB_COND_ERROR, |_vm, _argc| {
        let cmd = _vm.pop().to_str();
        crate::ported::utils::zerr(&format!("process substitution {} cannot be used here", cmd));
        // c:getoutputfile returns NULL with errflag set → the enclosing
        // statement aborts (empty stdout, exit 1), rather than the cond
        // merely evaluating false.
        crate::ported::utils::errflag.fetch_or(
            crate::ported::zsh_h::ERRFLAG_ERROR,
            std::sync::atomic::Ordering::Relaxed,
        );
        with_executor(|exec| exec.set_last_status(1));
        _vm.last_status = 1;
        Value::str("")
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
        // Also drive lex::LEX_LINENO — zerrmsg (utils.rs:376) reads
        // THAT counter for the `name:N:` prefix. C zsh interleaves
        // parse and execute per top-level list, so its single
        // `lineno` global serves both; zshrs compiles the whole
        // script before running, leaving LEX_LINENO parked at EOF.
        // Without this write, every runtime zwarn/zerr reported the
        // script's LAST line instead of the failing statement's.
        crate::ported::lex::set_lineno(n as u64);
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
        let r = crate::ported::cond::optison(None, &name); // c:cond.c:502 (fromtest=NULL for [[ -o ]])
        match r {
            0 => Value::Bool(true),  // c:cond.c:520 set
            1 => Value::Bool(false), // c:cond.c:518/520 unset
            _ => {
                // c:cond.c:514 — unknown option. optison already emitted the
                // diagnostic (via zwarn, now that fromtest=NULL for
                // `[[ -o ]]`); re-printing here double-emitted for
                // `[[ ! -o bad ]]` / `[[ -o a || -o b ]]`.
                Value::Bool(false)
            }
        }
    });
    // Tri-state `-o` for compile_cond's direct status path. Returns
    // 0 / 1 / 3 as a Value::Int that compile_cond consumes via
    // Op::SetStatus. Mirrors zsh's `[[ -o invalid ]]` returning $?=3.
    vm.register_builtin(BUILTIN_OPTION_CHECK_TRISTATE, |vm, _argc| {
        let name = vm.pop().to_str();
        let r = crate::ported::cond::optison(None, &name); // c:cond.c:502 (fromtest=NULL for [[ -o ]])
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
        if popped.len() < 3 {
            return Value::Status(1);
        }
        // c:Src/params.c:3511-3526 — trailing append flag. The ARRAY
        // subscript path (`a[N]+=(v)` / `a[lo,hi]+=(v)`) sets it so the
        // AUGMENT transform below collapses the range to an empty range
        // after the slice end and inserts ONLY the new value. The scalar
        // path pre-concats the old slice (ARRAY_INDEX+Concat) and passes
        // 0, so it keeps plain-replace semantics.
        let append = popped.pop().map_or(false, |v| v.to_str() == "1");
        let key = popped.pop().unwrap().to_str();
        let name = popped.pop().unwrap().to_str();
        let mut values: Vec<String> = Vec::new();
        for v in popped {
            match v {
                Value::Array(items) => {
                    for it in items.iter() {
                        values.push(it.to_str());
                    }
                }
                other => values.push(other.to_str()),
            }
        }
        // Bash sparse-array tracking: a single-index `a[i]=v` that pads the
        // dense Vec past its old end leaves indices old_len..i as HOLES (not
        // real elements), so `${#a[@]}`/`${!a[@]}` skip them like bash. Only
        // in bash mode, only for a plain non-append single index (0-based
        // under ksharrays). Captured before the assign; applied after.
        let sparse_track: Option<(String, usize, usize)> =
            if crate::dash_mode::bash_mode() && !append && !key.contains(',') {
                key.trim().parse::<usize>().ok().map(|i| {
                    let old_len =
                        with_executor(|exec| exec.array(&name).map(|a| a.len()).unwrap_or(0));
                    (name.clone(), old_len, i)
                })
            } else {
                None
            };
        // c:Src/params.c:3383-3389 — a subscripted ARRAY assignment to an
        // associative array is an error, whatever the subscript looks like:
        //     if (v && PM_TYPE(v->pm->node.flags) == PM_HASHED) {
        //         unqueue_signals();
        //         zerr("%s: attempt to set slice of associative array",
        //              v->pm->node.nam);
        //         freearray(val);
        //         errflag |= ERRFLAG_ERROR;
        //         return NULL;
        //     }
        // assignaparam (params.rs) ports this, but a single-key `h[k]=(1 2)`
        // never reaches it: the VM lowers subscripted assignment to this
        // builtin instead. The comma form `h[a,b]=(1 2)` DID error — it takes a
        // different route — so the gap looked like a subscript-parsing quirk
        // when it was really "the check lives on a path this form doesn't
        // take". Untreated, the assignment was SILENTLY DISCARDED: rc=0 and
        // `${h[k]}` still read its old value.
        //
        // Only array-valued assignment is rejected; `h[k]=x` is a scalar
        // element store and stays legal.
        {
            let is_hashed = crate::ported::params::paramtab()
                .read()
                .ok()
                .and_then(|t| {
                    t.get(&name).map(|pm| {
                        crate::ported::zsh_h::PM_TYPE(pm.node.flags as u32)
                            == crate::ported::zsh_h::PM_HASHED
                    })
                })
                .unwrap_or(false);
            if is_hashed {
                crate::ported::utils::zerr(&format!(
                    "{name}: attempt to set slice of associative array" // c:3385
                ));
                crate::ported::utils::errflag.fetch_or(
                    crate::ported::zsh_h::ERRFLAG_ERROR,
                    std::sync::atomic::Ordering::Relaxed,
                ); // c:3387
                return Value::Status(1); // c:3388
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
                    crate::ported::params::paramtab().read().ok().and_then(|t| {
                        t.get(&name).and_then(|pm| {
                            if crate::ported::zsh_h::PM_TYPE(pm.node.flags as u32)
                                == crate::ported::zsh_h::PM_SCALAR
                            {
                                pm.u_str.as_ref().map(|s| s.chars().count() as i64)
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
                if ksh_arrays && raw >= 0 {
                    raw + 1
                } else {
                    raw
                }
            };
            // c:Src/params.c getindex — subscript bounds are MATH
            // expressions, not bare integers: `a[(( ${#a}+1 ))]=(x)`,
            // `a[n+1]=(x)`. Plain `parse::<i64>()` returned 0 on any
            // arithmetic subscript, which the `i == 0` guard turned into
            // a silent no-op (computed-index append never landed). Parse
            // the literal fast-path first, then fall back to mathevali
            // (which handles `(( ))` grouping, var refs, and operators).
            let eval_bound = |s: &str| -> i64 {
                let t = s.trim();
                t.parse::<i64>()
                    .ok()
                    .or_else(|| crate::ported::math::mathevali(t).ok())
                    .unwrap_or(0)
            };
            let (start, end) = if let Some((s_str, e_str)) = key.split_once(',') {
                let s = ksh_shift(eval_bound(s_str));
                let e = ksh_shift(eval_bound(e_str));
                (start_translate(s), end_translate(e))
            } else {
                let i = ksh_shift(eval_bound(&key));
                if i == 0 {
                    return;
                }
                let n = start_translate(i);
                (n, n)
            };
            // c:Src/params.c:3518-3520 (assignaparam ASSPM_AUGMENT) — a
            // subscripted `+=` to an array does NOT prepend the old slice;
            // it collapses the range to an EMPTY range positioned right
            // AFTER the slice end (`v->start = v->end--`) and splices in
            // ONLY the new value: `a[2]+=(d)` on (a b c) → (a b d c);
            // `a[2,3]+=(x)` on (1 2 3 4) → (1 2 3 x 4). In setarrvalue's
            // 1-based convention here that means start = end+1 (so
            // start_idx == end_idx == end → splice arr[end..end]).
            let (start, end) = if append && end > 0 {
                (end + 1, end)
            } else {
                (start, end)
            };
            // c:Src/params.c:392-430 IPDEF9("argv"/"@"/"*", &pparams) —
            // the positional parameters live in the `pparams` vector, NOT
            // paramtab, so a subscript splice (`argv[2]=(X Y Z)`,
            // `2=(X Y Z)`) must read/write pparams. Splice a synthetic
            // array param holding the current positionals via the
            // canonical setarrvalue, then store the result back to
            // pparams — mirroring assignaparam's argv/@/* special-case
            // (params.rs:6937) for the whole-array form.
            if name == "argv" || name == "@" || name == "*" {
                let mut pm = {
                    crate::ported::params::createparam(
                        &name,
                        crate::ported::zsh_h::PM_ARRAY as i32,
                    );
                    crate::ported::params::paramtab()
                        .write()
                        .ok()
                        .and_then(|mut t| t.remove(&name))
                };
                if let Some(ref mut p) = pm {
                    p.u_arr = Some(exec.pparams());
                }
                let mut v = crate::ported::zsh_h::value {
                    pm,
                    arr: Vec::new(),
                    scanflags: 0,
                    valflags: 0,
                    start,
                    end,
                };
                crate::ported::params::setarrvalue(&mut v, values);
                let result = v.pm.and_then(|p| p.u_arr).unwrap_or_default();
                exec.set_pparams(result);
                return;
            }
            // Route through canonical setarrvalue (Src/params.c:2895).
            // It handles PM_READONLY rejection, PM_HASHED slice-error,
            // PM_ARRAY splice + bounds clamp + padding (c:2980+).
            let taken = match crate::ported::params::paramtab().write() {
                Ok(mut tab) => tab.remove(&name),
                Err(_) => None,
            };
            // c:Src/exec.c:2640 / getvalue(…, 1) — a subscript assignment to
            // a NONEXISTENT parameter auto-creates it. getindex/fetchvalue
            // with the create flag calls createparam(name, PM_ARRAY) so the
            // splice has an array to write into; `unset u; u[1,2]=(a z)`
            // then yields the array (a z). Without this, setarrvalue saw
            // v.pm == None and silently stored nothing. The single-index
            // scalar-value path (SET_ASSOC/SET_ARRAY_AT) already vivifies;
            // this brings the range/array-value path to parity.
            let taken = taken.or_else(|| {
                crate::ported::params::createparam(&name, crate::ported::zsh_h::PM_ARRAY as i32);
                crate::ported::params::paramtab()
                    .write()
                    .ok()
                    .and_then(|mut t| t.remove(&name))
            });
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
        if let Some((nm, old_len, i)) = sparse_track {
            crate::bash_arrays::note_subscript_set(&nm, old_len, i);
        }
        Value::Status(0)
    });

    // BUILTIN_CONCAT_SPLICE — word-segment concat for an expansion whose
    // ARRAY shape survives into the word (`${arr[@]}`, `$@`, `${(@)a}`,
    // `${=v}`, slices). c:Src/subst.c:4245 `if (isarr)` gates the two
    // emit shapes and c:1663 `int plan9 = isset(RCEXPANDPARAM);` picks
    // between them at RUNTIME — so the option, not the compile-time
    // segment shape, decides splice-vs-cross-product here.
    vm.register_builtin(BUILTIN_CONCAT_SPLICE, |vm, _argc| {
        let rhs = vm.pop();
        let lhs = vm.pop();
        if plan9_active() {
            return concat_plan9(lhs, rhs);
        }
        concat_splice(lhs, rhs)
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
    // `${^arr}` — RC_EXPAND_PARAM forced on by the flag. concat_plan9 carries
    // both halves of C's plan9 block: the c:4316-4350 cartesian emit AND the
    // c:4362 `uremnode` word deletion for an empty array. The DISTRIBUTE_FORCED
    // handler below cannot be reused: it is shared with `${(@)a}` / `${(f)v}` /
    // `${a[@]}`, which KEEP the word on empty (`x${(@)a}y` → `xy`).
    vm.register_builtin(BUILTIN_CONCAT_PLAN9, |vm, _argc| {
        let rhs = vm.pop();
        let lhs = vm.pop();
        concat_plan9(lhs, rhs)
    });

    // `${^^arr}` — RC_EXPAND_PARAM forced OFF (c:2553-2555 `plan9 = 0`). Every
    // other concat builtin re-checks plan9_active(), which is the OPTION, so
    // under `setopt rcexpandparam` they cross-product regardless of the flag.
    // Go straight to concat_splice — C's non-plan9 path (c:4366-4437).
    vm.register_builtin(BUILTIN_CONCAT_SPLICE_NOPLAN9, |vm, _argc| {
        let rhs = vm.pop();
        let lhs = vm.pop();
        concat_splice(lhs, rhs)
    });

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
                for a in la.iter() {
                    let a_s = a.as_str_cow();
                    for b in ra.iter() {
                        let b_s = b.as_str_cow();
                        let mut s = String::with_capacity(a_s.len() + b_s.len());
                        s.push_str(&a_s);
                        s.push_str(&b_s);
                        out.push(Value::str(s));
                    }
                }
                Value::array(out)
            }
            (Value::Array(la), rhs_scalar) => {
                // An EMPTY array contributes nothing to a concatenated
                // word — the surrounding scalar text survives. zsh:
                // `x${^a}y` (a=()) / `x${(P)scalar-empty}y` → "xy", NOT
                // a dropped word. Without this, a `(P)` indirect to an
                // unset/empty scalar (which nodes_to_value collapses to
                // Value::Array([]) for standalone-removal semantics)
                // cartesian-dropped the whole word — p10k's
                // `typeset -g _$2=${(P)2}` then arrived as a bare
                // `typeset` and dumped every parameter (~217× → 19 MB
                // terminal flood → startup hang).
                if la.is_empty() {
                    return rhs_scalar;
                }
                let r = rhs_scalar.as_str_cow();
                let out: Vec<Value> = la
                    .iter()
                    .map(|a| {
                        let a_s = a.as_str_cow();
                        let mut s = String::with_capacity(a_s.len() + r.len());
                        s.push_str(&a_s);
                        s.push_str(&r);
                        Value::str(s)
                    })
                    .collect();
                Value::array(out)
            }
            (lhs_scalar, Value::Array(ra)) => {
                // Symmetric empty-array-contributes-nothing rule; see
                // the (Array, scalar) arm above.
                if ra.is_empty() {
                    return lhs_scalar;
                }
                let l = lhs_scalar.as_str_cow();
                let out: Vec<Value> = ra
                    .iter()
                    .map(|b| {
                        let b_s = b.as_str_cow();
                        let mut s = String::with_capacity(l.len() + b_s.len());
                        s.push_str(&l);
                        s.push_str(&b_s);
                        Value::str(s)
                    })
                    .collect();
                Value::array(out)
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
        // c:Src/subst.c:4245 `if (isarr)` — an unquoted array embedded
        // in a word ALWAYS emits one word per element, never a scalar
        // join. The shape (splice vs plan9 cross-product) is chosen at
        // RUNTIME by c:1663 `int plan9 = isset(RCEXPANDPARAM);`, exactly
        // as BUILTIN_CONCAT_SPLICE does. The only extra case DISTRIBUTE
        // handles is the DQ context: the compiler emits
        // CallBuiltin(BUILTIN_CONCAT_DISTRIBUTE, 1) when the parent word
        // is DQ-wrapped (compile_zsh.rs parent_is_dq), and inside DQ
        // `"pre${arr}post"` joins via $IFS[0] to a single scalar
        // regardless of the option (c:Src/subst.c:1650-1656 isarr
        // comment). The default UNQUOTED path emits argc=2 (lhs + rhs).
        // Bug #246 in docs/BUGS.md.
        if argc == 1 {
            // DQ context: join any Array side to scalar via sepjoin's
            // IFS default. c:Src/utils.c:3936-3945 — set-but-empty IFS
            // joins with "" (`IFS=""; echo "x$*y"` → `xabcy`); only
            // unset / space-leading IFS yields " ".
            let join_arr = |arr: &[Value]| -> String {
                let strs: Vec<String> = arr.iter().map(|v| v.as_str_cow().into_owned()).collect();
                crate::ported::utils::sepjoin(&strs, None)
            };
            let l = match lhs {
                Value::Array(a) => join_arr(&a),
                other => other.as_str_cow().into_owned(),
            };
            let r = match rhs {
                Value::Array(a) => join_arr(&a),
                other => other.as_str_cow().into_owned(),
            };
            let mut s = String::with_capacity(l.len() + r.len());
            s.push_str(&l);
            s.push_str(&r);
            return Value::str(s);
        }
        // Unquoted plain `${arr}`: same runtime dispatch as
        // BUILTIN_CONCAT_SPLICE — c:4245 `if (isarr)` always distributes
        // one word per element; c:1663 picks splice (default, first/last
        // sticking, c:4366-4437) vs plan9 cross-product (c:4316-4365).
        // concat_splice / concat_plan9 both honor EMPTY_EXPANSION_IS_SCALAR
        // so the p10k `${(P)2}` empty-array word-removal semantics survive.
        if plan9_active() {
            return concat_plan9(lhs, rhs);
        }
        concat_splice(lhs, rhs)
    });

    // See BUILTIN_WORD_ASSEMBLE_PLAN9's doc comment for the stack contract.
    vm.register_builtin(BUILTIN_WORD_ASSEMBLE_PLAN9, |vm, argc| {
        // Pop argc values: descriptor was pushed FIRST (bottom), segments
        // after it, so popping top-first then reversing yields
        // [descriptor, seg0, …, seg(n-1)].
        let mut popped: Vec<Value> = Vec::with_capacity(argc as usize);
        for _ in 0..argc {
            popped.push(vm.pop());
        }
        popped.reverse();
        let mut it = popped.into_iter();
        let descriptor = it.next().map(|v| v.to_str()).unwrap_or_default();
        let plan9_flags: Vec<bool> = descriptor.chars().map(|c| c == '1').collect();
        let segments: Vec<Value> = it.collect();
        word_assemble_plan9(&segments, &plan9_flags)
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
        let on = crate::ported::zsh_h::isset(crate::ported::zsh_h::XTRACE);
        Value::Int(if on { 1 } else { 0 })
    });

    vm.register_builtin(BUILTIN_XTRACE_LINE, |vm, _argc| {
        // Keep the Value; `to_str()` allocates a String, and this handler
        // runs on EVERY `(( … ))` / `[[ … ]]` / loop-head evaluation, where
        // xtrace is off essentially always. Defer the allocation to the
        // `on` branch below.
        let cmd_val = vm.pop();
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
        let on = crate::ported::zsh_h::isset(crate::ported::zsh_h::XTRACE);
        if on {
            let already = XTRACE_DONE_PS4.with(|f| f.get());
            if !already {
                printprompt4();
            }
            // c:exec.c:5240/5286 — `fprintf(xtrerr, "%s\n", expr)`. Buffer
            // the line + newline, flush once (single write).
            xtrerr_fputs(&cmd_val.to_str());
            xtrerr_fputs("\n");
            xtrerr_flush();
            XTRACE_DONE_PS4.with(|f| f.set(false));
        }
        Value::Status(0)
    });

    // BUILTIN_XTRACE_ARRAY_LINE — xtrace line for an `arr=(...)` / `arr+=(...)`
    // assignment. Stack on entry: [array, prefix] (argc = 2); pops prefix
    // ("name=( " / "name+=( "), then the whole assembled Value::Array. Direct
    // port of c:Src/exec.c::addvars:2624-2632, guarded on the live xtrace
    // state like C's `if (xtr)`: prints `prefix qz(e0) qz(e1) … ) ` with each
    // element shell-quoted (quotedzputs). Replaces the former one-VM-slot-per-
    // element trace, which overflowed next_slot (u16) on large literals.
    vm.register_builtin(BUILTIN_XTRACE_ARRAY_LINE, |vm, _argc| {
        let prefix = vm.pop().to_str();
        let arr = vm.pop();
        let live = vm.last_status;
        with_executor(|exec| {
            exec.set_last_status(live);
        });
        let on = crate::ported::zsh_h::isset(crate::ported::zsh_h::XTRACE);
        if on {
            let already = XTRACE_DONE_PS4.with(|f| f.get());
            if !already {
                printprompt4();
            }
            let mut line = String::with_capacity(prefix.len() + 16);
            line.push_str(&prefix);
            if let Value::Array(items) = arr {
                for it in items.iter() {
                    line.push_str(&crate::ported::utils::quotedzputs(&it.to_str()));
                    line.push(' ');
                }
            }
            line.push_str(") ");
            line.push('\n');
            xtrerr_fputs(&line);
            xtrerr_flush();
            XTRACE_DONE_PS4.with(|f| f.set(false));
        }
        Value::Status(0)
    });

    // BUILTIN_MAKE_ARRAY_COUNTED — pop a count Int (top), then pop that many
    // values below it, and push them as one Value::Array (bottom-of-group
    // first). Same result as Op::MakeArray(N) but N comes from the stack as an
    // i64, dodging MakeArray's u16 operand cap. The compiler emits this only
    // when a literal `arr=(...)` has more than u16::MAX elements.
    vm.register_builtin(BUILTIN_MAKE_ARRAY_COUNTED, |vm, _argc| {
        let count = vm.pop().to_int().max(0) as usize;
        let mut items: Vec<Value> = Vec::with_capacity(count);
        for _ in 0..count {
            items.push(vm.pop());
        }
        items.reverse();
        Value::array(items)
    });

    // BUILTIN_ARGV_RFLATTEN — recursively flatten a MakeArray-packed argv
    // bundle so a >255-arg Call/CallFunction/CallBuiltin (dispatched with
    // argc=1 over the single packed Array) recovers every positional arg. The
    // call ops flatten only one level; a brace/glob/`$arr` word contributes a
    // nested Array that would otherwise stringify. See the const doc.
    vm.register_builtin(BUILTIN_ARGV_RFLATTEN, |vm, _argc| {
        let v = vm.pop();
        let mut out: Vec<String> = Vec::new();
        flatten_array_value(v, &mut out);
        Value::array(out.into_iter().map(Value::str).collect())
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
        let on = crate::ported::zsh_h::isset(crate::ported::zsh_h::XTRACE);
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
                            for item in items.iter() {
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
                // printprompt4 + the args + `\n` all land in the xtrerr
                // buffer; the single fflush below writes the whole line in
                // one syscall so concurrent pipeline stages never
                // interleave (c:makecline:2122-2123).
                let already_ps4 = XTRACE_DONE_PS4.with(|f| f.get());
                if !already_ps4 {
                    printprompt4();
                }
                xtrerr_fputs(&line);
                xtrerr_fputs("\n"); // c:2122 fputc('\n', xtrerr)
                xtrerr_flush(); // c:2123 fflush(xtrerr)
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
        let on = crate::ported::zsh_h::isset(crate::ported::zsh_h::XTRACE);
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
                // (val); fputc(' ', xtrerr);`. Append to the xtrerr buffer
                // (no newline / no flush — the line continues with the
                // command via XTRACE_ARGS, or ends at XTRACE_NEWLINE).
                xtrerr_fputs(&format!("{}={} ", name, quotedzputs(&value)));
            }
        }
        Value::Status(0)
    });

    // BUILTIN_XTRACE_NEWLINE — emit trailing `\n` + flush iff a
    // prior XTRACE_ASSIGN this line already emitted PS4. Mirrors
    // C's `fputc('\n', xtrerr); fflush(xtrerr);` at exec.c:3398
    // (the assignment-only path through execcmd_exec).
    vm.register_builtin(BUILTIN_XTRACE_NEWLINE, |_vm, _argc| {
        let on = crate::ported::zsh_h::isset(crate::ported::zsh_h::XTRACE);
        if on {
            let already_ps4 = XTRACE_DONE_PS4.with(|f| f.get());
            if already_ps4 {
                xtrerr_fputs("\n"); // c:3398 fputc('\n', xtrerr)
                xtrerr_flush(); // c:3398 fflush(xtrerr)
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
    // c:Src/loop.c — `loops++` / `loops--` bracket every iterative
    // construct: execfor c:114/188, execwhile c:427/491, execrepeat
    // c:523/546. `loops` is a GLOBAL, not a per-frame counter, and
    // `bin_break` reads it (`if (!loops)`) to decide whether `break` /
    // `continue` is legal. Because doshfunc does NOT reset it (only
    // restores it under LOCAL_LOOPS, c:6104-6112), a function called
    // from inside a loop sees the CALLER's count and its `break` ends
    // the caller's loop. zshrs's compiled for/while/until/repeat lower
    // to raw jumps, so without these two ops the counter stayed 0 and
    // every such `break` errored out instead.
    vm.register_builtin(BUILTIN_LOOP_ENTER, |_vm, _argc| {
        crate::ported::builtin::LOOPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // c:114
        Value::Int(0)
    });
    vm.register_builtin(BUILTIN_LOOP_EXIT, |_vm, _argc| {
        use std::sync::atomic::Ordering::SeqCst;
        // Saturating: a chunk aborted mid-loop (errflag, `return`)
        // unwinds through `run_chunk`'s restore rather than this op, so
        // never let a stray decrement drive the count negative.
        let _ = crate::ported::builtin::LOOPS.fetch_update(SeqCst, SeqCst, |n| {
            Some(if n > 0 { n - 1 } else { 0 })
        }); // c:188
        Value::Int(0)
    });

    // c:Src/loop.c:529-534 (execwhile), :180-185 (execfor), :540-545
    // (execrepeat) — the identical post-body drain every loop runs:
    //     if (breaks) {
    //         breaks--;
    //         if (breaks || !contflag) break;
    //         contflag = 0;
    //     }
    // Returns Int(1) when this loop must terminate, Int(0) when it
    // should proceed to the next iteration. Only a `break`/`continue`
    // executed in a DIFFERENT chunk (a called function, `eval`, a
    // sourced file) reaches here — an in-chunk `break` compiles to a
    // direct jump and never touches the counter.
    vm.register_builtin(BUILTIN_LOOP_BREAK_DRAIN, |_vm, _argc| {
        use std::sync::atomic::Ordering::SeqCst;
        let breaks = crate::ported::builtin::BREAKS.load(SeqCst);
        if breaks == 0 {
            return Value::Int(0);
        }
        let remaining = breaks - 1;
        crate::ported::builtin::BREAKS.store(remaining, SeqCst); // c:530
        let contflag = crate::ported::builtin::CONTFLAG.load(SeqCst);
        if remaining != 0 || contflag == 0 {
            return Value::Int(1); // c:532 — `break`
        }
        crate::ported::builtin::CONTFLAG.store(0, SeqCst); // c:533
        Value::Int(0)
    });

    // c:Src/exec.c:1370 execlist — `while (wc_code(code) == WC_LIST &&
    // !breaks && !retflag && !errflag)`. A pending `breaks` stops the
    // CURRENT list at the next statement boundary WITHOUT consuming it,
    // so the flag keeps travelling outward until a loop's drain eats it.
    // Non-consuming by design: the drain above is the only consumer.
    vm.register_builtin(BUILTIN_BREAKS_PENDING, |_vm, _argc| {
        let b = crate::ported::builtin::BREAKS.load(std::sync::atomic::Ordering::SeqCst);
        Value::Int(if b != 0 { 1 } else { 0 })
    });

    vm.register_builtin(BUILTIN_NOEXEC_CHECK, |_vm, _argc| {
        // c:Src/exec.c:1390 — `set -n` / `noexec` option: parse but
        // don't execute. Returns Int(1) when noexec is set so the
        // emit-side JumpIfTrue skips the statement body.
        if opt_state_get("noexec").unwrap_or(false) {
            return Value::Int(1);
        }
        // c:Src/exec.c:1390 — execlist's list-loop gate:
        //   `while (wc_code(code) == WC_LIST && !breaks && !retflag
        //          && !errflag)`
        // — once errflag is set, the NEXT sublist never starts, so
        // lastval survives untouched to the shell exit. Without this
        // prologue gate the follow-up statement RAN, its dispatch
        // saw errflag, returned 1, and SetStatus clobbered lastval —
        // `[[ x == [a- ]]; print rc=$?` exited 1 instead of zsh's 2
        // (the cond syntax error set lastval=2 per exec.c:5216-5221).
        if (crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed)
            & crate::ported::zsh_h::ERRFLAG_ERROR)
            != 0
        {
            return Value::Int(1);
        }
        Value::Int(0)
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
        // `${~spec}` carrier: C's `globsubst` is a paramsubst-LOCAL
        // int (c:Src/subst.c:1671 `int globsubst = isset(GLOBSUBST);`,
        // set to 2 by `${~}` at c:2597-2603) whose only effect is the
        // `shtokenize()` of THAT substitution's own result
        // (c:4419-4420). It can therefore never be observed by a later
        // statement. zshrs carries the flag on the global option table
        // (subst.rs:5125-5136) so the compile-emitted glob ops in the
        // same word pipeline can see it, and restores it at
        // command-dispatch boundaries — but a `${~}` sitting in a word
        // that dispatches NO command (a `for`/`select` word list, a
        // loop/`case` header) had no such boundary before the NEXT
        // statement's words were expanded, so GLOB_SUBST leaked into
        // them. This op is emitted exactly once per sublist, in
        // compile_list's prologue (compile_zsh.rs:557) — i.e. BEFORE
        // the sublist's words expand — which is the same "state is
        // gone by the next statement" guarantee C gets for free.
        // Without it, `_parameters`' `for i in ${…:#${~pfilt}*}` loop
        // globbed its `ary+=($i:"$val")` body word and died with
        // "bad pattern: HISTCHARS:!^#", killing `-<TAB>` completion.
        consume_tilde_globsubst_carrier();
        Value::Status(0)
    });

    vm.register_builtin(BUILTIN_SUBLIST_FINISH, |vm, _argc| {
        // c:Src/jobs.c:1754 — `pipestats[0] = lastval;`. C has ONE
        // `lastval` global (c:Src/exec.c:120), so `waitonejob` reads
        // exactly the status the finished sublist just produced.
        //
        // zshrs splits that global in two: the fusevm status cell that
        // `Op::SetStatus`/`Op::GetStatus` and therefore `$?` use, and
        // the `builtin::LASTVAL` mirror that the ported `waitonejob`
        // reads. The compound-command compilers (compile_if,
        // compile_while, compile_for, compile_case, …) settle their
        // result with `Op::SetStatus` alone, so LASTVAL still holds
        // whatever the last dispatched BUILTIN returned — the loop
        // condition, typically. `if [[ -z x ]]; then :; fi` and
        // `while false; do :; done` both end with `$? == 0` and a
        // stale LASTVAL of 1.
        //
        // compile_sublist pushes `Op::GetStatus` ahead of this call so
        // the authoritative status arrives as an argument; republish it
        // through LASTVAL to reunify the two before the ported
        // waitonejob reads it, exactly as the single-command dispatch
        // sites at c:Src/exec.c:4367 do.
        let status = vm.pop().to_int() as i32;
        crate::ported::builtin::LASTVAL.store(status, std::sync::atomic::Ordering::Relaxed);
        // c:Src/jobs.c:1750-1756 — `compile_sublist` only emits this
        // marker for a cmplx sublist element that is not a multi-stage
        // pipeline, i.e. exactly the case where C's job carries no
        // procs, so drive the canonical port with a procs-less job the
        // same way the single-command sites do.
        let mut synth = crate::ported::zsh_h::job::default();
        crate::ported::jobs::waitonejob(&mut synth);
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
            Value::Array(arr) => arr.iter().map(|x| x.to_str()).collect(),
            Value::Str(s) => vec![s.to_string()],
            other => vec![other.to_str()],
        };
        let mut args: Vec<&str> = vec![op];
        if words.is_empty() {
            args.push("");
        } else {
            args.extend(words.iter().map(|s| s.as_str()));
        }
        let opts: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        let vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();
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
        // Then pop (op, target) pairs in reverse compile order. Keep
        // targets as Values — a glob-bearing target arrives as a
        // Value::Array of matches.
        let n_targets = ((argc - 1) / 2) as usize;
        let mut pairs: Vec<(u8, Value)> = Vec::with_capacity(n_targets);
        for _ in 0..n_targets {
            let op_byte = vm.pop().to_int() as u8;
            let target = vm.pop();
            pairs.push((op_byte, target));
        }
        // Restore compile order (target_1 first).
        pairs.reverse();

        // c:Src/glob.c:2195-2203 xpandredir — "Loop over matches,
        // duplicating the redirection for each file found": a glob
        // target with N matches becomes N members of the same multio
        // (`echo hi > *.txt` with two matches writes both files).
        let mut entries: Vec<(u8, String)> = Vec::with_capacity(pairs.len());
        for (op_byte, target) in pairs {
            match target {
                Value::Array(items) => {
                    for item in items.iter() {
                        entries.push((op_byte, item.to_str()));
                    }
                }
                other => entries.push((op_byte, other.to_str())),
            }
        }
        if entries.is_empty() {
            return Value::Status(1);
        }

        // c:Src/exec.c:2418 — `else if (!mfds[fd1] || unset(MULTIOS))`:
        // with MULTIOS unset every redirect takes the REPLACE path in
        // script order — each target is still opened (created /
        // truncated) and dup2'd over the fd, so the LAST one wins and
        // earlier files end up empty (`unsetopt multios; print x > a
        // > b` leaves `a` empty, `x` in `b`). host_apply_redirect is
        // exactly one replace step, noclobber gate included.
        let multios_on = opt_state_get("multios").unwrap_or(true);
        if !multios_on {
            with_executor(|exec| {
                for (op_byte, target) in &entries {
                    exec.host_apply_redirect(fd as u8, *op_byte, target);
                    if exec.redirect_failed {
                        // c:Src/exec.c execerr — abort the remaining
                        // redirect list on failure.
                        break;
                    }
                }
            });
            return Value::Status(0);
        }

        if entries.len() == 1 {
            // Single member after splicing — a plain replace
            // (c:2418 new-multio arm). Route through
            // host_apply_redirect so the noclobber gate, the
            // pipeline-output split partial, and error handling all
            // apply exactly as for an un-bagged redirect.
            let (op_byte, target) = &entries[0];
            with_executor(|exec| {
                exec.host_apply_redirect(fd as u8, *op_byte, target);
            });
            return Value::Status(0);
        }

        // c:Src/exec.c:3722-3724 — when this command's stdout IS the
        // pipeline output, C seeds mfds[1] with the pipe BEFORE
        // walking the redirect list, so the pipe is the multio's
        // first member (`print x >&1 > f | cat` sends `x` down the
        // pipe TWICE: once for the seed, once for the `>&1` dup).
        let pipe_seed = fd == 1
            && with_executor(|exec| {
                exec.pipe_output_scope
                    .is_some_and(|d| d + 1 == exec.redirect_scope_stack.len())
            });

        // Save current fd state for scope-end restoration — BEFORE
        // the first member's replace dup2 below.
        // c:Src/exec.c:2425 — `int fdN = movefd(fd1); save[fd1] = fdN;`. A SAVED
        // descriptor is shell state and must live above the script's fd range:
        // plain dup() returns the LOWEST free fd, which parked the saved stdout
        // on fd 3, so `print -u 3 -r -- X 2>/dev/null` wrote into the shell's own
        // saved descriptor and reported success where zsh says `bad file number`.
        // F_DUPFD with a floor of 10 is exactly what movefd does.
        let saved = unsafe { libc::fcntl(fd, libc::F_DUPFD, 10) };
        if saved >= 0 {
            with_executor(|exec| {
                if let Some(top) = exec.redirect_scope_stack.last_mut() {
                    top.push((fd, saved));
                } else {
                    unsafe { libc::close(saved) };
                }
            });
        }

        // Accumulate member fds in redirect order. c:Src/exec.c:
        // 2447-2480 addfd — the FIRST member REPLACES the fd
        // (c:2448-2450 `mfds[fd1]->ct=1; mfds[fd1]->fds[0]=fd1;`), so
        // a later numeric `>&N` self-dup resolves against the fd's
        // value at that point in the sequence: `print x > f >&1`
        // writes f TWICE; `print x >&1 > f` writes the ORIGINAL
        // stdout + f.
        let mut target_fds: Vec<i32> = Vec::with_capacity(entries.len() + 1);
        if pipe_seed {
            let p = unsafe { libc::fcntl(fd, libc::F_DUPFD, 10) };
            if p >= 0 {
                target_fds.push(p);
            }
        }
        let noclobber = opt_state_get("noclobber").unwrap_or(false)
            || !opt_state_get("clobber").unwrap_or(true);
        for (i, (op_byte, target)) in entries.iter().enumerate() {
            let open_result: std::io::Result<i32> = match *op_byte {
                r::DUP_WRITE | r::DUP_READ => {
                    // Numeric `>&N` — dup the LIVE fd N (after any
                    // earlier member's replace).
                    match target.trim_start_matches('&').parse::<i32>() {
                        Ok(src) => {
                            let d = unsafe { libc::fcntl(src, libc::F_DUPFD, 10) };
                            if d >= 0 {
                                Ok(d)
                            } else {
                                Err(std::io::Error::last_os_error())
                            }
                        }
                        Err(_) => Err(std::io::Error::from_raw_os_error(libc::EBADF)),
                    }
                }
                r::WRITE => {
                    // c:Src/exec.c clobber_open — noclobber applies
                    // to multio file targets too; failure aborts the
                    // remaining redirect list (execerr), so `setopt
                    // noclobber; touch a; print x > a > b` errors on
                    // `a` and never creates `b`.
                    let target_meta = std::fs::metadata(target).ok();
                    let target_is_regular_file = target_meta
                        .as_ref()
                        .map(|m| m.file_type().is_file())
                        .unwrap_or(false);
                    // c:Src/exec.c:2313 clobber_open — CLOBBER_EMPTY re-uses
                    // an empty regular file under noclobber (same allowance
                    // as the single-redirect path).
                    let clobber_empty_ok = opt_state_get("clobberempty").unwrap_or(false)
                        && target_meta.as_ref().map(|m| m.len() == 0).unwrap_or(false);
                    if noclobber && target_is_regular_file && !clobber_empty_ok {
                        eprintln!(
                            "{}:{}: file exists: {}",
                            shname(),
                            crate::ported::lex::lineno(),
                            target
                        );
                        for prev in &target_fds {
                            unsafe {
                                libc::close(*prev);
                            }
                        }
                        with_executor(|exec| {
                            exec.redirect_failed = true;
                        });
                        // Sink the upcoming command's output (mirrors
                        // the single-redirect noclobber arm in
                        // host_apply_redirect).
                        if let Ok(file) = fs::OpenOptions::new().write(true).open("/dev/null") {
                            let new_fd = file.into_raw_fd();
                            unsafe {
                                libc::dup2(new_fd, fd);
                                libc::close(new_fd);
                            }
                        }
                        return Value::Status(1);
                    }
                    fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(target)
                        .map(|f| f.into_raw_fd())
                }
                r::APPEND => fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .append(true)
                    .open(target)
                    .map(|f| f.into_raw_fd()),
                _ => fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(target)
                    .map(|f| f.into_raw_fd()),
            };
            match open_result {
                Ok(tfd) => {
                    if i == 0 && !pipe_seed {
                        // c:2448-2450 — first member replaces the fd.
                        unsafe {
                            libc::dup2(tfd, fd);
                        }
                    }
                    target_fds.push(tfd);
                }
                Err(e) => {
                    // c:Src/exec.c:3741 — `zwarn("%e: %s", errno, fname)`:
                    // zwarning supplies the `name:LINE:` prefix with the
                    // REAL current lineno; redir_errno_msg builds the `%e`
                    // errno message (was a hardcoded ErrorKind match that
                    // showed generic "redirect failed" for EROFS/etc.).
                    let msg = redir_errno_msg(&e);
                    crate::ported::utils::zwarn(&format!("{}: {}", msg, target));
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
        let write_dup = unsafe { libc::fcntl(pipe_write_raw, libc::F_DUPFD, 10) };
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
        // Shell-internal bookkeeping fd — above the script's range (movefd).
        let close_on_end = unsafe { libc::fcntl(fd, libc::F_DUPFD, 10) };
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
    // layout pushed by compile_zsh (mirrors the write side):
    //   [source_1, op_1, source_2, op_2, …, source_N, op_N, fd]
    // argc = 2N + 1; op distinguishes file opens (READ) from numeric
    // `<&N` dups (DUP_READ); a glob source arrives as Value::Array
    // and splices into one member per match (c:Src/glob.c:2195-2203).
    // Opens every source, sets up a pipe + producer thread that
    // reads each source in order and writes to the pipe write-end,
    // then closes its write-end so the consumer gets EOF. dup2 the
    // pipe read-end onto fd. Bug #36 input side in docs/BUGS.md.
    vm.register_builtin(BUILTIN_MULTIOS_READ, |vm, argc| {
        if argc < 3 || argc % 2 == 0 {
            return Value::Status(1);
        }
        let fd = vm.pop().to_int() as i32;
        let n_sources = ((argc - 1) / 2) as usize;
        let mut pairs: Vec<(u8, Value)> = Vec::with_capacity(n_sources);
        for _ in 0..n_sources {
            let op_byte = vm.pop().to_int() as u8;
            let source = vm.pop();
            pairs.push((op_byte, source));
        }
        pairs.reverse();

        // Splice glob match arrays (c:Src/glob.c:2195-2203).
        let mut entries: Vec<(u8, String)> = Vec::with_capacity(pairs.len());
        for (op_byte, source) in pairs {
            match source {
                Value::Array(items) => {
                    for item in items.iter() {
                        entries.push((op_byte, item.to_str()));
                    }
                }
                other => entries.push((op_byte, other.to_str())),
            }
        }
        if entries.is_empty() {
            return Value::Status(1);
        }

        // c:Src/exec.c:2418 — `unset(MULTIOS)`: sequential replace,
        // last source wins (`unsetopt multios; cat < a < b` reads
        // only b; a is still opened — and errors still surface).
        let multios_on = opt_state_get("multios").unwrap_or(true);
        if !multios_on {
            with_executor(|exec| {
                for (op_byte, source) in &entries {
                    exec.host_apply_redirect(fd as u8, *op_byte, source);
                    if exec.redirect_failed {
                        break;
                    }
                }
            });
            return Value::Status(0);
        }

        if entries.len() == 1 {
            // Single member after splicing — plain replace.
            let (op_byte, source) = &entries[0];
            with_executor(|exec| {
                exec.host_apply_redirect(fd as u8, *op_byte, source);
            });
            return Value::Status(0);
        }

        // Save current fd state for scope-end restoration — BEFORE
        // the first member's replace dup2 below.
        // c:Src/exec.c:2425 — `int fdN = movefd(fd1); save[fd1] = fdN;`. A SAVED
        // descriptor is shell state and must live above the script's fd range:
        // plain dup() returns the LOWEST free fd, which parked the saved stdout
        // on fd 3, so `print -u 3 -r -- X 2>/dev/null` wrote into the shell's own
        // saved descriptor and reported success where zsh says `bad file number`.
        // F_DUPFD with a floor of 10 is exactly what movefd does.
        let saved = unsafe { libc::fcntl(fd, libc::F_DUPFD, 10) };
        if saved >= 0 {
            with_executor(|exec| {
                if let Some(top) = exec.redirect_scope_stack.last_mut() {
                    top.push((fd, saved));
                } else {
                    unsafe { libc::close(saved) };
                }
            });
        }

        // Open every source in redirect order; numeric `<&N` dups
        // resolve against the LIVE fd table. First member replaces
        // the fd (c:2448-2450) so later self-dups see it.
        let mut source_fds: Vec<i32> = Vec::with_capacity(entries.len());
        for (i, (op_byte, source)) in entries.iter().enumerate() {
            let open_result: std::io::Result<i32> = match *op_byte {
                r::DUP_READ | r::DUP_WRITE => match source.trim_start_matches('&').parse::<i32>() {
                    Ok(src) => {
                        let d = unsafe { libc::fcntl(src, libc::F_DUPFD, 10) };
                        if d >= 0 {
                            Ok(d)
                        } else {
                            Err(std::io::Error::last_os_error())
                        }
                    }
                    Err(_) => Err(std::io::Error::from_raw_os_error(libc::EBADF)),
                },
                _ => fs::File::open(source).map(|f| f.into_raw_fd()),
            };
            match open_result {
                Ok(tfd) => {
                    if i == 0 {
                        unsafe {
                            libc::dup2(tfd, fd);
                        }
                    }
                    source_fds.push(tfd);
                }
                Err(e) => {
                    let msg = match e.kind() {
                        std::io::ErrorKind::PermissionDenied => "permission denied",
                        std::io::ErrorKind::NotFound => "no such file or directory",
                        _ => "open failed",
                    };
                    // c:Src/exec.c:3741 — zwarn with real lineno prefix.
                    crate::ported::utils::zwarn(&format!("{}: {}", msg, source));
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
                        libc::read(sfd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
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
    // c:Src/exec.c:3978-3986 — nullexec==1 marker. See the const's
    // doc block. Arg: 1 = entering a bare-exec redirect, 0 = leaving.
    vm.register_builtin(BUILTIN_EXEC_PERM_REDIRS, |vm, _argc| {
        let on = vm.pop().to_int() != 0;
        with_executor(|exec| exec.exec_redirs_permanent = on);
        Value::Status(0)
    });
    // Bare-exec redirect epilogue — see the const's doc block.
    // c:Src/exec.c:252-259 (execerr) + c:4367-4386 (done: POSIX gate).
    vm.register_builtin(BUILTIN_EXEC_REDIR_DONE, |vm, _argc| {
        use std::sync::atomic::Ordering;
        let failed = with_executor(|exec| {
            let f = exec.redirect_failed;
            exec.redirect_failed = false;
            f
        });
        if !failed {
            return Value::Status(0);
        }
        // c:255 — `redir_err = lastval = 1`.
        vm.last_status = 1;
        if isset(crate::ported::zsh_h::POSIXBUILTINS) && !isset(crate::ported::zsh_h::INTERACTIVE) {
            // c:4379-4383 — non-interactive POSIX fatal: exit(1).
            // In-process equivalent: arm EXIT_PENDING/EXIT_VAL so the
            // next BUILTIN_ERREXIT_CHECK (trigger 2) unwinds the
            // script with status 1 — same deferred-exit shape the
            // `exit` builtin uses inside subshell contexts.
            crate::ported::builtin::EXIT_VAL.store(1, Ordering::Relaxed);
            crate::ported::builtin::EXIT_PENDING.store(1, Ordering::Relaxed);
        }
        Value::Status(1)
    });
    // c:Src/exec.c:3722-3724 — see the const's doc block. No args.
    vm.register_builtin(BUILTIN_PIPE_OUTPUT_MARK, |_vm, _argc| {
        with_executor(|exec| exec.pipe_output_pending = true);
        Value::Status(0)
    });
    // c:Src/exec.c:3710-3724 — install this pipeline stage's fds.
    //     /* Make a copy of stderr for xtrace output before redirecting */
    //     fflush(xtrerr);
    //     ...
    //     /* Add pipeline input/output to mnodes */
    //     if (input)  addfd(forked, save, mfds, 0, input, 0, NULL);
    //     if (output) addfd(forked, save, mfds, 1, output, 1, NULL);
    // Emitted into the stage chunk by compile_zsh.rs (after the arg
    // words' expansion ops, before the redirect scope), and fed by
    // BUILTIN_RUN_PIPELINE via `stage_fds_park`. Doing the dup2 HERE
    // rather than before the chunk runs is what makes a `$(...)` in a
    // stage's arguments see the shell's fd 0 instead of the pipe.
    vm.register_builtin(BUILTIN_PIPE_FDS_INSTALL, |vm, argc| {
        // Arg: `|&` merge-stderr flag (compile_zsh always passes it).
        let merge_stderr = pop_args(vm, argc)
            .first()
            .map(|s| s != "0" && !s.is_empty())
            .unwrap_or(false);
        let (in_fd, out_fd) = stage_fds_take();
        if in_fd < 0 && out_fd < 0 {
            return Value::Status(0);
        }
        // c:3711 `fflush(xtrerr)` — flush before the fds move, so
        // anything buffered from the expansion phase lands on the
        // ORIGINAL fd, not on the pipe.
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        unsafe {
            if in_fd >= 0 {
                libc::dup2(in_fd, libc::STDIN_FILENO);
                if in_fd != libc::STDIN_FILENO {
                    libc::close(in_fd);
                }
            }
            if out_fd >= 0 {
                libc::dup2(out_fd, libc::STDOUT_FILENO);
                if out_fd != libc::STDOUT_FILENO {
                    libc::close(out_fd);
                }
                // `cmd |& next`: the `2>&1` C appends to cmd's redirect
                // list (walked at c:3730+, i.e. after this addfd), so
                // stderr follows the pipe, not the shell's stdout.
                if merge_stderr {
                    libc::dup2(libc::STDOUT_FILENO, libc::STDERR_FILENO);
                }
            }
        }
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
    // c:Src/cond.c:308-316 — `if (!(pprog = patcompile(right, ...)))
    //   { zwarnnam(fromtest, "bad pattern: %s", right); return 2; }`.
    // The cond path must NOT use str_match/glob_match_static: the
    // case-statement consumer of those follows Src/loop.c:667 zerr
    // semantics (errflag abort), while cond is a zwarn + status-2
    // soft failure. COND_BAD_PATTERN carries the 2 across the
    // Bool-shaped stack contract (so `!=`'s LogNot can't lose it).
    thread_local! {
        static COND_BAD_PATTERN: std::cell::Cell<bool> =
            const { std::cell::Cell::new(false) };
    }
    vm.register_builtin(BUILTIN_COND_STRMATCH, |vm, _argc| {
        let pat = vm.pop().to_str();
        let s = vm.pop().to_str();
        // bash `shopt -s nocasematch` → case-insensitive `[[ == ]]` / `[[ != ]]`.
        // Lowercase BOTH sides for the match decision (glob metacharacters are
        // not letters, so the pattern's `*`/`?`/`[…]` structure is preserved).
        // No-op unless the bash shopt is active. --zsh unaffected.
        let (s, pat) = if crate::dash_mode::nocasematch() {
            (s.to_lowercase(), pat.to_lowercase())
        } else {
            (s, pat)
        };
        let mut pat_tok = pat.clone();
        crate::ported::glob::tokenize(&mut pat_tok);
        if crate::ported::pattern::patcompile(
            &pat_tok,
            crate::ported::zsh_h::PAT_STATIC as i32,
            None,
        )
        .is_none()
        {
            // c:314 — zwarnnam(fromtest, "bad pattern: %s", right).
            crate::ported::utils::zwarn(&format!("bad pattern: {}", pat));
            COND_BAD_PATTERN.with(|c| c.set(true));
            return Value::Bool(false);
        }
        // Match via the shared engine so `(#b)`/`(#m)` backref and
        // MATCH-variable population stays in one place.
        Value::Bool(crate::vm_helper::glob_match_static(&s, &pat))
    });
    vm.register_builtin(BUILTIN_COND_UNKNOWN, |vm, _argc| {
        // c:Src/cond.c:150-188 — `zwarnnam(fromtest, "unknown condition: %s",
        // name)` for a `-X` op with no matching cond module. Like a cond
        // syntax error it yields status 2 and aborts: arm COND_BAD_PATTERN so
        // the downstream BUILTIN_COND_STATUS_FROM_BOOL carries the 2 across the
        // Bool-shaped stack and runs the shared errflag+set_last_status(2)+abort
        // path (c:Src/exec.c:5216-5221). Returns Bool(false) as the operand.
        let op = vm.pop().to_str();
        crate::ported::utils::zerr(&format!("unknown condition: {}", op));
        COND_BAD_PATTERN.with(|c| c.set(true));
        Value::Bool(false)
    });
    vm.register_builtin(BUILTIN_COND_STATUS_FROM_BOOL, |vm, _argc| {
        // `${~pat}` / `${(P)~pat}` inside a `[[ … ]]` operand flips
        // GLOB_SUBST on via the tilde carrier so the pattern match sees
        // active metacharacters. In C that flag is prefork-scoped and
        // gone once the operand is consumed; zshrs restores it at the
        // next command-dispatch boundary, but a bare `[[ … ]]` has no
        // trailing assignment to trigger that — so globsubst leaked ON
        // into the NEXT command's word expansion, filename-generating a
        // scalar value it should not (p10k `_p9k_set_prompt`: line 45
        // `[[ … != ${(P)~disabled} ]]` leaked into line 46's
        // `local val=$arr[idx]`, whose glob-char-laden value then hit
        // "no matches found" and aborted the whole prompt build →
        // garbled 25-line prompt / interactive hang). Consume the
        // carrier here: this builtin ends EVERY `[[ … ]]`, and runs
        // after the operands (and their pattern match) are done.
        consume_tilde_globsubst_carrier();
        let ok = vm.pop().to_int() != 0;
        let bad = COND_BAD_PATTERN.with(|c| {
            let b = c.get();
            c.set(false);
            b
        });
        if bad {
            // c:Src/exec.c:5216-5221 — `stat = evalcond(...);
            //   /* 2 indicates a syntax error. For compatibility,
            //      turn this into a shell error. */
            //   if (stat == 2) errflag |= ERRFLAG_ERROR;`
            // The errflag abort exits the script with lastval (2),
            // matching `zsh -fc '[[ x == [a- ]]; print rc=$?'`
            // printing nothing after the diagnostic and exiting 2.
            crate::ported::utils::errflag.fetch_or(
                crate::ported::zsh_h::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            with_executor(|exec| exec.set_last_status(2));
            return Value::Int(2); // c:Src/cond.c:316 `return 2;`
        }
        let status: i32 = if ok { 0 } else { 1 };
        // c:Src/exec.c:5216 — `lastval = evalcond(...)`: the conditional's
        // result IS the command's lastval, and c:Src/cond.c's evalcond
        // never inspects errflag while evaluating. So when a `[[ … ]]`
        // operand raised errflag (e.g. a nounset "parameter not set" zerr
        // on `${arr[99]}` under NO_UNSET), zsh STILL completes the test and
        // exits with the cond result; the errflag only aborts the FOLLOWING
        // commands. Sync the result to the executor's live lastval HERE —
        // the nounset site left it at a transient 1, and the next
        // BUILTIN_ERREXIT_CHECK reads the executor (not vm.last_status), so
        // without this sync `setopt NO_UNSET; [[ -z ${arr[99]} ]]` exited 1
        // instead of 0. The Op::SetStatus that follows sets vm.last_status;
        // this keeps the executor coherent with it before the abort check.
        with_executor(|exec| exec.set_last_status(status));
        Value::Int(status as i64)
    });
    vm.register_builtin(BUILTIN_USE_CMDOUTVAL_RESET, |_vm, _argc| {
        crate::ported::exec::use_cmdoutval.store(0, std::sync::atomic::Ordering::Relaxed);
        Value::Status(0)
    });

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
            // c:Src/exec.c:3442 — a command whose words expand to ZERO
            // words is a NULL command: `cmdoutval = use_cmdoutval ?
            // lastval : 0`. `use_cmdoutval` is set (below, in
            // BUILTIN_CMD_SUBST_TEXT) only when a command substitution
            // ran during this command's word expansion, so:
            //   `false; $(exit 5)`  → keep the subst status (5)
            //   `false; $nonexistent` → reset to 0 (null command).
            // The previous port unconditionally kept `$?`, so
            // `false; $unset` wrongly stayed 1 (A01grammar.ztst:5).
            let keep =
                crate::ported::exec::use_cmdoutval.load(std::sync::atomic::Ordering::Relaxed) != 0;
            let status = if keep { vm.last_status } else { 0 };
            crate::ported::exec::use_cmdoutval.store(0, std::sync::atomic::Ordering::Relaxed);
            return Value::Status(status);
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
        // AOP intercepts (zshrs extension, no C counterpart) — same
        // gate as host_exec_external (the static-head path): dynamic
        // command names (`cmd=/bin/echo; $cmd payload`) must consult
        // registered intercepts before dispatch, else `intercept
        // before /bin/echo ...` fires for the literal spelling but
        // not the variable one. run_intercepts runs before-advice
        // in-place and returns None to continue; Some(status) means
        // an around/after advice fully handled the command.
        let intercepted = with_executor(|exec| {
            if exec.intercepts.is_empty() {
                return None;
            }
            let full_cmd = if args.len() == 1 {
                args[0].clone()
            } else {
                args.join(" ")
            };
            let rest: Vec<String> = args[1..].to_vec();
            exec.run_intercepts(&args[0], &full_cmd, &rest)
        });
        if let Some(result) = intercepted {
            return Value::Status(result.unwrap_or(127));
        }
        // zshrs-original opcode builtins (async, doctor, peach, …) reached
        // via a run-time-resolved head (`$var`): they are absent from the
        // static BUILTINS port table / builtintab, so execcmd_exec below would
        // treat the head as external and report "command not found" — even
        // though `whence` calls it a builtin and a literal head runs it via
        // CallBuiltin. Dispatch by name here, but ONLY when the head is neither
        // a user function nor a ported builtin, so the shell's
        // function -> builtin -> external order is preserved.
        if let Some(head) = args.first() {
            let is_fn = with_executor(|e| e.function_exists(head));
            let is_ported =
                crate::ported::builtin::createbuiltintable().contains_key(head.as_str());
            if !is_fn && !is_ported {
                if let Some(status) = try_run_registered_builtin(head, &args[1..]) {
                    crate::ported::builtin::LASTVAL
                        .store(status, std::sync::atomic::Ordering::Relaxed);
                    return Value::Status(status);
                }
            }
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
        // directly); `output != 0` at c:2988 forks immediately. last1=2
        // (c:Src/exec.c:2014 `last1 ? 1 : 2`): terminal pipe stage but
        // the shell IS needed afterward — the VM keeps executing
        // bytecode after this op. last1=1 would arm the fake-exec
        // optimization (c:3646-3651, gate at c:3662 `last1 != 1`),
        // making `execute()` execve THIS process for external heads:
        // `p=/bin/echo; $p hi; echo after` replaced the shell and
        // `after` never ran (D04parameter chunk 11 shell-killer).
        // c:Src/exec.c:1690-1700 — execpline's job frame: save thisjob
        // (`pj = thisjob`) and allocate the jobtab slot that
        // execcmd_fork's addproc (c:2853) hangs the child pid off.
        // Without a live thisjob, the fork at c:3662 (last1 != 1 →
        // external must fork) registers no proc, nothing waits, and
        // the child races the rest of the script.
        let pj = {
            use crate::ported::jobs;
            *jobs::THISJOB
                .get_or_init(|| std::sync::Mutex::new(-1))
                .lock()
                .unwrap_or_else(|e| e.into_inner())
        };
        let newjob = {
            use crate::ported::jobs;
            let table = jobs::JOBTAB.get_or_init(|| std::sync::Mutex::new(Vec::new()));
            let mut tab = table.lock().unwrap_or_else(|e| e.into_inner());
            jobs::initjob(&mut tab) // c:1700 `thisjob = newjob = initjob()`
        };
        {
            use crate::ported::jobs;
            *jobs::THISJOB
                .get_or_init(|| std::sync::Mutex::new(-1))
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = newjob as i32;
        }
        crate::ported::exec::execcmd_exec(
            &mut state,
            &mut eparams,
            0,                                   // input  (c:2989)
            0,                                   // output (c:2988)
            crate::ported::zsh_h::Z_SYNC as i32, // how
            2,                                   // last1=2 — shell continues (c:2014)
            -1,                                  // close_if_forked
        );
        // c:Src/exec.c:1828-1835 — execpline's Z_SYNC tail: waitjobs()
        // reaps the forked external. c:Src/jobs.c:487-495 + 551-552 —
        // the job's LAST proc sets lastval (0200|sig when signalled,
        // else WEXITSTATUS). Builtin/shfunc heads never forked (job
        // has no procs) — LASTVAL was already set by execbuiltin /
        // doshfunc inside execcmd_exec; skip the wait.
        {
            use crate::ported::jobs;
            let table = jobs::JOBTAB.get_or_init(|| std::sync::Mutex::new(Vec::new()));
            let mut tab = table.lock().unwrap_or_else(|e| e.into_inner());
            if jobs::hasprocs(&tab, newjob) {
                jobs::waitjobs(&mut tab, newjob); // c:1835
                if let Some(p) = tab[newjob].procs.last() {
                    let val = if p.is_signaled() {
                        0o200 | p.term_sig() // c:Src/jobs.c:489-490
                    } else {
                        p.exit_status() // c:Src/jobs.c:494
                    };
                    crate::ported::builtin::LASTVAL
                        .store(val, std::sync::atomic::Ordering::Relaxed);
                }
            }
            // c:1977-1979 — `deletejob(jn, 0)` once done; c:1981
            // `thisjob = pj` restores the caller's job.
            if newjob < tab.len() {
                jobs::deletejob(&mut tab[newjob], false);
            }
            *jobs::THISJOB
                .get_or_init(|| std::sync::Mutex::new(-1))
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = pj;
        }
        let status = crate::ported::builtin::LASTVAL.load(std::sync::atomic::Ordering::Relaxed);
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

    // Fatal-only abort check emitted between the pipes of an `&&` / `||`
    // chain, where the full errexit check is suppressed. Mirrors ONLY the
    // errflag arm of BUILTIN_ERREXIT_CHECK below: an errflag abandons the
    // list in zsh, and no connector can consume it.
    vm.register_builtin(BUILTIN_FATAL_ABORT_CHECK, |vm, _argc| {
        use std::sync::atomic::Ordering;
        let errflag_set = (crate::ported::utils::errflag.load(Ordering::Relaxed)
            & crate::ported::zsh_h::ERRFLAG_ERROR)
            != 0;
        if !errflag_set || isset(crate::ported::zsh_h::INTERACTIVE) {
            return Value::Int(0);
        }
        // CONTINUE_ON_ERROR: clear and keep going, as the full check does.
        if isset(crate::ported::zsh_h::CONTINUEONERROR) {
            crate::ported::utils::errflag
                .fetch_and(!crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
            return Value::Int(0);
        }
        // Abort the chain with the failing command's own status intact —
        // a cond syntax error left lastval=2 (c:Src/exec.c:5216-5221), and
        // that 2 is what zsh exits with. Reading the executor's live
        // lastval (not forcing 1) is the same rule the full check uses.
        vm.last_status = with_executor(|exec| exec.last_status());
        Value::Int(1)
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
        // c:Src/exec.c:1571-1603 — `sublist_done:` runs the ZERR trap for
        // the sublist that just failed. It is NOT gated on retflag: C only
        // consults retflag at the TOP of the list loop (c:1370 `while
        // (wc_code(code) == WC_LIST && !breaks && !retflag && !errflag)`),
        // which stops the NEXT sublist — the current one still completes
        // its sublist_done. So `return 5` fires the ERR trap on its way out.
        //
        // zshrs's escape short-circuit below returns before ever reaching
        // the ZERR fire, so a `return N` inside a try-list skipped the trap:
        //   f() { { return 5 } always { print fin } }; f
        // printed `fin / err=5` where zsh prints `err=5 / fin / err=5`.
        // (Plain `f() { return 5 }` matched by luck — the inner fire was
        // missing but the OUTER sublist fired instead, since doshfunc had
        // cleared retflag by then and DONETRAP was still 0.)
        //
        // `exit` is deliberately excluded: C's `exit` goes zexit() →
        // realexit(), leaving the process without ever reaching
        // sublist_done. Verified: `zsh -fc 'trap "print err" ERR; f(){ exit
        // 5 }; f'` prints nothing.
        if retflag != 0 && exit_pending == 0 {
            let last = vm.last_status;
            // c:1598-1603 — same DONETRAP gate as the non-escape path below.
            if last != 0 && crate::ported::exec::DONETRAP.load(Ordering::Relaxed) == 0 {
                // c:Src/signals.c:1085-1087 — `int obreaks = breaks; int
                // oretflag = retflag; int olastval = lastval;` and c:1220-1222
                // — `breaks += obreaks; retflag = oretflag;`. dotrapargs
                // brackets EVERY trap dispatch with this save/restore because
                // the trap body runs as a normal list and would otherwise
                // consume the caller's control-flow flags. That matters
                // exactly here: we are firing ZERR while retflag is SET, and a
                // FUNCTION-form trap (`TRAPZERR() { … }`) goes through
                // doshfunc, whose epilogue eats retflag outright
                // (c:Src/exec.c:6047-6052 `if (retflag) { retflag = 0; breaks
                // = funcsave->breaks; }`). Without the bracket the pending
                // `return 5` was swallowed by its own ERR trap and the
                // function ran on:
                //   TRAPZERR() { print z }; f() { { return 2 } always { : }
                //                             print after }; f
                // printed `after`, where zsh returns from f.
                //
                // zshrs's `dotrap` inlines the dispatch and does not carry
                // dotrapargs' save/restore, so the bracket lives at this call
                // site. lastval is restored too (c:1087 / c:1213 `lastval =
                // olastval`) — the trap body's own commands must not become
                // the caller's `$?`.
                let obreaks = crate::ported::builtin::BREAKS.load(Ordering::Relaxed); // c:1085
                let oretflag = crate::ported::builtin::RETFLAG.load(Ordering::Relaxed); // c:1086
                let olastval = crate::ported::builtin::LASTVAL.load(Ordering::Relaxed); // c:1087
                let _ = crate::ported::signals::dotrap(crate::ported::signals_h::SIGZERR); // c:1601
                crate::ported::exec::DONETRAP.store(1, Ordering::Relaxed); // c:1602
                crate::ported::builtin::BREAKS.store(obreaks, Ordering::Relaxed); // c:1220
                crate::ported::builtin::RETFLAG.store(oretflag, Ordering::Relaxed); // c:1222
                crate::ported::builtin::LASTVAL.store(olastval, Ordering::Relaxed);
                // c:1213
            }
        }
        if retflag != 0 || exit_pending != 0 {
            if exit_pending != 0 {
                // c:Src/builtin.c zexit — the deferred exit carries its
                // status in EXIT_VAL; sync it into the VM counter so
                // the top-level unwind reports it as the script's exit
                // (run_chunk returns vm.last_status). Without this, a
                // POSIX-fatal `.` failure exited 127 (bin_dot's return)
                // instead of C's exit(1) at Src/exec.c:4383.
                vm.last_status = crate::ported::builtin::EXIT_VAL.load(Ordering::Relaxed) & 0xFF;
            }
            return Value::Int(1);
        }
        let errflag_set = (crate::ported::utils::errflag.load(Ordering::Relaxed)
            & crate::ported::zsh_h::ERRFLAG_ERROR)
            != 0;
        // c:Src/init.c:1931 — `if (errflag && !interact &&
        // !isset(CONTINUEONERROR)) { errexit = 1; break; }` — with
        // CONTINUE_ON_ERROR set, the top-level do-while re-enters
        // loop() and the NEXT list runs instead of the shell exiting.
        // Clear the flag so the next statement starts clean (the
        // failed statement's lastval is already in place).
        if errflag_set
            && !crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE)
            && crate::ported::zsh_h::isset(crate::ported::zsh_h::CONTINUEONERROR)
        {
            crate::ported::utils::errflag
                .fetch_and(!crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
            return Value::Int(0);
        }
        if errflag_set && !crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE) {
            // c:Src/exec.c execlist — every enclosing list loop runs
            // `while (... && !errflag)`, so a set errflag breaks the
            // CURRENT scope and the check in the enclosing scope
            // breaks THAT one, all the way out. Leave errflag SET —
            // do NOT convert it to EXIT_PENDING: a process-exit
            // signal tunnels through the containment boundaries C
            // has, namely eval (Src/builtin.c:6221 `errflag &=
            // ~ERRFLAG_ERROR`), source (Src/init.c:1663 same), fork
            // boundaries (subshell/cmdsubst — child's errflag dies
            // with the child), and the interactive toplevel
            // (Src/init.c:139). Those boundaries clear errflag
            // themselves and execution continues past them; with
            // EXIT_PENDING armed here, `eval 'assoc=(odd)'; echo
            // after` aborted the whole script where zsh 5.9 prints
            // `after` (eval status 1). Bug #74's function case
            // (`f() { local -r x=5; x=10; }; f; echo after`) still
            // aborts: the function scope unwinds on THIS check, and
            // the caller's next ERREXIT_CHECK sees the still-set
            // errflag and unwinds too — exactly C's propagation.
            //
            // c:Src/init.c:234 — loop() BREAKS on errflag and
            // zsh_main exits with the UNTOUCHED lastval, NOT a
            // forced 1: `typeset -i x=3#8` (math error during the
            // assignment, before typeset sets a status) exits 0 in
            // zsh; a cond syntax error set lastval=2 (exec.c:5216-
            // 5221) and zsh exits 2; the readonly-reassign case
            // exits 1 because ITS lastval is 1. Sync the VM counter
            // from the executor's live lastval instead of
            // overwriting.
            vm.last_status = with_executor(|exec| exec.last_status());
            // c:Src/exec.c:1598-1603 — `sublist_done:` runs the ZERR trap
            // for the failed sublist BEFORE the enclosing list loop breaks
            // on errflag (`while (... && !errflag)` at c:1370). So an
            // errflag-setting command (readonly reassign, bad redirect)
            // must fire ZERR on its way out, exactly like the retflag
            // escape above and the non-escape fall-through below. Without
            // this the errflag early-return pre-empted the ZERR block
            // further down, so `TRAPZERR() { … }; typeset -r ro=1; ro=2`
            // aborted the script (correct) but never fired the trap. Same
            // DONETRAP gate + dotrapargs save/restore bracket
            // (c:signals.c:1085-1087 / 1213-1222) as the retflag branch:
            // a function-form TRAPZERR runs through doshfunc and would
            // otherwise consume the caller's breaks/retflag/lastval.
            let last = with_executor(|exec| exec.last_status());
            if last != 0 && crate::ported::exec::DONETRAP.load(Ordering::Relaxed) == 0 {
                let obreaks = crate::ported::builtin::BREAKS.load(Ordering::Relaxed); // c:1085
                let oretflag = crate::ported::builtin::RETFLAG.load(Ordering::Relaxed); // c:1086
                let olastval = crate::ported::builtin::LASTVAL.load(Ordering::Relaxed); // c:1087
                                                                                        // c:Src/signals.c:1101 — dotrapargs returns early if errflag
                                                                                        // is set, and c:1174/1205-1218 brackets the dispatch with
                                                                                        // `traperr = errflag` … restore. The failing assignment left
                                                                                        // errflag SET, so the trap body (`print zerr`) would itself
                                                                                        // bail on the first op. Clear errflag across the dispatch so
                                                                                        // the body runs, then restore it so the script still aborts.
                let oerrflag = crate::ported::utils::errflag.load(Ordering::Relaxed); // c:1174
                crate::ported::utils::errflag.store(0, Ordering::Relaxed);
                let _ = crate::ported::signals::dotrap(crate::ported::signals_h::SIGZERR); // c:1601
                crate::ported::utils::errflag.store(oerrflag, Ordering::Relaxed); // c:1216
                crate::ported::exec::DONETRAP.store(1, Ordering::Relaxed); // c:1602
                crate::ported::builtin::BREAKS.store(obreaks, Ordering::Relaxed); // c:1220
                crate::ported::builtin::RETFLAG.store(oretflag, Ordering::Relaxed); // c:1222
                crate::ported::builtin::LASTVAL.store(olastval, Ordering::Relaxed);
                // c:1213
            }
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
        let already_done = crate::ported::exec::DONETRAP.load(Ordering::Relaxed) != 0;
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
        let in_unwindable_scope =
            isset(crate::ported::zsh_h::INTERACTIVE) || locallvl != 0 || sourcelvl != 0;
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
            let on_canonical = isset(ERREXIT) || (errreturn_opt && !errreturn); // c:1608-1609
            let on_legacy = opt_state_get("errexit").unwrap_or(false);
            (
                (on_canonical || on_legacy) && (no_err & crate::ported::zsh_h::NOERREXIT_EXIT) == 0,
                !exec.subshell_snapshots.is_empty(),
            )
        });
        if !errexit_on {
            return Value::Int(0);
        }
        // c:Src/exec.c:1611-1618 — under ERR_EXIT a failing command exits the
        // whole shell via realexit() FROM THE POINT OF FAILURE, before any
        // enclosing `always` arm can run. zsh 5.9.2 (the reference) has no
        // `this_noerrexit` deferral, so at top-level / function scope the
        // faithful behavior is to process-exit here (zexit fires the SIGEXIT
        // trap and exits). This bypasses the always arm, fixing
        // `setopt errexit; { false } always { print A }` which wrongly ran the
        // always body: the deferred EXIT_PENDING routed the unwind through
        // always_entry (compile_zsh.rs re-points it there) and
        // SET_TRY_BLOCK_ERROR then cleared the pending exit so the body ran.
        if crate::ported::builtin::SUBSHELL_DEPTH.load(Ordering::Relaxed) == 0 {
            crate::ported::builtin::zexit(last, crate::ported::zsh_h::ZEXIT_NORMAL);
            // c:1618 realexit
        }
        // Subshell: zshrs runs subshells in-process, so it cannot process-exit
        // the whole shell here — defer to the subshell-end unwind.
        crate::ported::builtin::EXIT_VAL.store(last, Ordering::Relaxed);
        crate::ported::builtin::EXIT_PENDING.store(1, Ordering::Relaxed);
        let _ = in_subshell;
        Value::Int(1)
    });

    // BUILTIN_ASSIGN_ONLY_STATUS — status of an assignment-only
    // simple command. c:Src/exec.c:3393-3396 (execcmd_exec, no
    // command word + varspc): `if (errflag) lastval = 1; else
    // lastval = cmdoutval;`; same shape at c:1322 (execsimple
    // WC_ASSIGN: `lv = (errflag ? errflag : cmdoutval)`) and
    // c:3977 (nullexec=2 redir variant). cmdoutval is the exit of
    // a `$()` that ran in an RHS (already in vm.last_status via
    // compile_assign's per-assign SetStatus), 0 otherwise. The
    // store goes to the canonical LASTVAL too — that IS C's single
    // `lastval` global; without it the errflag-abort path
    // (BUILTIN_ERREXIT_CHECK trigger 4) syncs vm.last_status from
    // a stale LASTVAL and `readonly r=1; r=2` exited 0, not 1.
    vm.register_builtin(BUILTIN_ASSIGN_ONLY_STATUS, |vm, _argc| {
        use std::sync::atomic::Ordering;
        let had_cmd_subst = vm.pop().to_int() != 0;
        let errflag_set = (crate::ported::utils::errflag.load(Ordering::Relaxed)
            & crate::ported::zsh_h::ERRFLAG_ERROR)
            != 0;
        // c:Src/exec.c addvars — `if (!pm) { lastval = 1; if
        // (!cmdoutval) cmdoutval = 1; }` (assignment-failed cheat).
        let assign_failed = ASSIGN_FAILED_FLAG.swap(false, std::sync::atomic::Ordering::Relaxed);
        let status = if errflag_set || assign_failed {
            1 // c:Src/exec.c:3394 `lastval = 1` / addvars cmdoutval=1
        } else if had_cmd_subst {
            vm.last_status // c:3396 `lastval = cmdoutval` (subst exit)
        } else {
            0 // c:3396 `lastval = cmdoutval` (cmdoutval = 0)
        };
        with_executor(|exec| exec.set_last_status(status));
        // c:Src/jobs.c deletefilelist — a `=(cmd)` temp file is bound to the
        // JOB of the command that created it and unlinked when that command
        // completes (Src/exec.c:5588 for the shfunc case; the simple-command
        // job's filelist likewise). An assignment-only command like
        // `f==(cmd)` has no consuming builtin/exec, so the PsubFdGuard that
        // cleans consuming commands never fires — the temp leaked and a later
        // `$(<$f)` / `[[ -f $f ]]` still saw it, where zsh deletes it at the
        // end of the assignment (verified: even `f==(x) && cat $f` fails).
        // Clean here so the assignment command is the temp's job boundary.
        close_pending_psub_fds();
        Value::Status(status)
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
        // !!! DASH-STRICT GATE !!! dash/ash have no `${var:offset:length}`
        // substring expansion (it is a "Bad substitution"); bash/ksh/sh do.
        if crate::dash_mode::dash_strict() {
            crate::ported::utils::zerr("bad substitution");
            crate::ported::utils::errflag.fetch_or(
                crate::ported::zsh_h::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            with_executor(|exec| exec.set_last_status(1));
            return Value::str("");
        }
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
        // c:Src/exec.c — a command substitution running during a
        // command's word expansion makes its exit the status of an
        // otherwise-empty command (`$(exit 5)` → 5). Flag it so
        // BUILTIN_EXEC_DYNAMIC's null-command branch keeps `$?` instead
        // of resetting to 0.
        crate::ported::exec::use_cmdoutval.store(1, std::sync::atomic::Ordering::Relaxed);
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
    //   7 = RedirTarget — same as Default but glob gated on MULTIOS
    //         (c:Src/glob.c:2161-2167 xpandredir)
    //   8 = unquoted assignment VALUE — same as 6 plus PREFORK_SINGLE
    //         (c:Src/exec.c:2603 / :4239-4241)
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
                // c:Src/lex.c untokenize — the final argv pass C runs
                // on every expanded word (glob.c:1862 / exec.c) drops
                // the Nularg empty-word sentinel remnulargs left in
                // place and folds any remaining token chars. Without
                // it, quoted splits with empty pieces
                // ("${(s:|:)x}" on "|a|b|") leak U+00A1 into argv.
                let result_value = if mode == 5 {
                    let out = crate::ported::subst::singsub(&prepped);
                    Value::str(crate::ported::lex::untokenize(&out))
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
                        Value::array(Vec::new())
                    } else if nodes.len() == 1 {
                        Value::str(crate::ported::lex::untokenize(
                            &nodes.into_iter().next().unwrap(),
                        ))
                    } else {
                        Value::array(
                            nodes
                                .into_iter()
                                .map(|n| Value::str(crate::ported::lex::untokenize(&n)))
                                .collect(),
                        )
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
                        Value::array(parts)
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
                // Mode 8 = the unquoted VALUE of a `NAME=VALUE` assignment
                // (bare statement or typeset-family argument). C preforks
                // exactly that with `PREFORK_SINGLE|PREFORK_ASSIGN`
                // (c:Src/exec.c:2603 and c:Src/exec.c:4239-4241); the
                // PREFORK_SINGLE half is paramsubst's `ssub`
                // (c:Src/subst.c:1761), which gates off the forced split at
                // c:Src/subst.c:3913.
                let pf_flags = if mode == 8 {
                    crate::ported::zsh_h::PREFORK_SINGLE | crate::ported::zsh_h::PREFORK_ASSIGN
                } else if mode == 6 {
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
                // c:Src/subst.c:3929-3932 — `if (isarr) l->list.flags |=
                // LF_ARRAY; else l->list.flags &= ~LF_ARRAY;`. C's paramsubst
                // holds the LinkList and stamps its `isarr` on it directly;
                // the Rust port hands the same bit back through the
                // `PARAMSUBST_LF_ARRAY` thread-local (subst.rs:20511) — set at
                // subst.rs:17796 (`isarr != 0 && !forced_split_to_one`) and
                // reset to false at the top of EVERY paramsubst
                // (subst.rs:3891 / subst.rs:17445).
                //
                // Clear it BEFORE multsub so a segment that runs no paramsubst
                // at all reads false instead of some earlier expansion's
                // value. `multsub`'s own `isarr` return cannot be used for
                // this: an unquoted `$(cmd)` / `` `cmd` `` sets LF_ARRAY
                // unconditionally (subst.rs:897 / :1159, c:Src/subst.c:285-286
                // `if (!qt) list->list.flags |= LF_ARRAY;` and c:331), so
                // `x$(true)y` would look array-shaped when zsh keeps that
                // word (verified: it is the single word `xy`).
                //
                // Every paramsubst re-initialises the cell on entry, so
                // clearing it here cannot disturb subst.rs's own readers
                // (stringsubst reads it immediately after each paramsubst
                // call, subst.rs:1094).
                crate::ported::subst::PARAMSUBST_LF_ARRAY.with(|c| c.set(false));
                let (_first, nodes, _ms_ws, _ret) =
                    crate::ported::subst::multsub(&prepped, pf_flags);
                // Read immediately: brace expansion / filesub / glob below can
                // re-enter paramsubst and overwrite the cell. `seg_is_array` is
                // the array-ness of the OUTERMOST paramsubst in this segment —
                // C's c:4245 `if (isarr)` for the same expansion. Paired with
                // `seg_zero_words` it separates the two empty shapes for the
                // empty-result arm further down (c:4362 vs c:4464).
                let seg_is_array = crate::ported::subst::PARAMSUBST_LF_ARRAY.with(|c| c.get());
                let seg_zero_words = nodes.is_empty();
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
                // c:Src/subst.c:170 — `if (unset(IGNOREBRACES) && !(flags &
                // PREFORK_SINGLE))` guards the `xpandbraces` loop, so a word
                // preforked as a scalar (assignment VALUE, mode 8) is NEVER
                // brace-expanded: `local x={a,b}` stores the five literal
                // characters. This pass stands in for prefork's loop, so it
                // owes the same guard.
                let brace_expand = opt_state_get("braceexpand").unwrap_or(true)
                    && (pf_flags & crate::ported::zsh_h::PREFORK_SINGLE) == 0; // c:170
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
                // Mode 7 = redirect-target word: glob only under
                // MULTIOS (c:Src/glob.c:2161-2167 xpandredir,
                // "Globbing is only done for multios.").
                let noglob = opt_state_get("noglob").unwrap_or(false)
                    || opt_state_get("GLOB").map(|v| !v).unwrap_or(false)
                    || !opt_state_get("glob").unwrap_or(true)
                    || (mode == 7 && !opt_state_get("multios").unwrap_or(true));
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
                        // c:Src/pattern.c:4306 haswilds on the still-
                        // TOKENIZED word (pre-untokenize), matching C's
                        // zglob entry gate (Src/glob.c:1230) which runs
                        // on the lexer-tokenized string. haswilds
                        // matches ONLY token codes: source-level
                        // `*.toml` carries Star and fires; bare literal
                        // `[`/`*`/`?` from `$'...'` decode, `:-`
                        // default values, or nested-substitution
                        // results were never shtokenize'd (C
                        // subst.c:3231 sets globsubst=0 in the `:-`
                        // arm) and stay literal — bug #625. Plain
                        // multibyte text (`↔`) never matches a token
                        // codepoint — bug #627.
                        let is_glob_pre = !noglob && crate::ported::pattern::haswilds(&s);
                        // c:Src/glob.c:1230 — zglob receives the word in
                        // LEXER-TOKENIZED form and only untokenizes it when
                        // it declines to glob (c:1232) or falls back to the
                        // literal (c:1884). The token form is what makes a
                        // QUOTED metachar distinguishable from an active one:
                        // in `*(.e['[[ $REPLY == a* ]]'])` the body's `]`
                        // bytes are raw ASCII while the qualifier's real
                        // closer is `Outbrack`, which is exactly how
                        // checkglobqual (c:1163/1170, testing Outpar/Inpar)
                        // and get_strarg's tokenized delimiter half
                        // (Src/subst.c:1379-1390) find the true end. zshrs
                        // untokenized here, one line before the glob layer,
                        // so every quoted metachar became indistinguishable
                        // from an active one and the qualifier parser closed
                        // on the first quoted `]`. Keep the tokenized word
                        // for the glob call; the untokenized form still
                        // drives the non-glob arms below.
                        let s_tok = s.clone();
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
                            exec.expand_glob(&s_tok)
                        } else if is_assignment_shape
                            && crate::ported::zsh_h::isset(crate::ported::zsh_h::MAGICEQUALSUBST)
                        {
                            // c:Src/exec.c:3353 — when MAGIC_EQUAL_SUBST is set
                            // on a non-typeset command, esprefork = PREFORK_TYPESET,
                            // so every NAME=value arg runs through
                            // filesub(PREFORK_TYPESET): the `~`/`=` after the
                            // first `=` (and after each `:`) undergo filename
                            // expansion. `print foo=~/bar` → `foo=$HOME/bar`.
                            // filesubstr (subst.c:741) keys on the Tilde TOKEN,
                            // not literal `~`; this `s` was already untokenized
                            // above, so re-tokenize (as BUILTIN_MAGIC_EQUALS_PREFORK
                            // does) before filesub, then untokenize the result.
                            let mut tokd = s.clone();
                            crate::ported::glob::shtokenize(&mut tokd);
                            let exp = crate::ported::subst::filesub(
                                &tokd,
                                crate::ported::zsh_h::PREFORK_TYPESET,
                            );
                            vec![crate::lex::untokenize(&exp).to_string()]
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
                    //
                    // c:Src/subst.c:4437 + 1650-1656 — a word CONTAINING a
                    // quoted span never drops: `x"${v[-1]}"y` (v empty)
                    // is the scalar "xy", and a standalone `"${v[-1]}"`
                    // is ONE empty arg. The lexer marks DQ/SQ spans with
                    // Dnull(\u{9e})/Snull(\u{9d})/Qstring(\u{8c})/
                    // Bnull(\u{9f}); their presence in the SOURCE word
                    // means qt semantics apply. Without this gate, zpwr's
                    // global `setopt rc_expand_param` turned autopair's
                    // `local lchar="${LBUFFER[-1]}"` (empty prompt +
                    // backspace) into an ARGLESS `local` — the full
                    // parameter-table dump the user saw per keystroke.
                    if only.is_empty() {
                        // A quote span that WRAPS the expansion keeps the
                        // empty arg (`"${v[-1]}"` → one empty arg). But a
                        // quote INSIDE the `${…}` braces — e.g. the alternate
                        // of `${x:+'q'}` or `${x:-'d'}` — does NOT: when that
                        // branch isn't taken the result is a plain unquoted
                        // empty and must ELIDE, matching zsh (`a=(A ${x:+'q'}
                        // C)` → 2 elements, not 3). So only count quote
                        // markers at brace-depth 0 (outside `${…}`). Inbrace
                        // = \u{8f}, Outbrace = \u{90}.
                        let mut depth = 0i32;
                        let mut word_has_quoted_span = false;
                        for c in text.chars() {
                            match c {
                                '\u{8f}' => depth += 1,
                                '\u{90}' => depth -= 1,
                                '\u{9e}' | '\u{9d}' | '\u{8c}' | '\u{9f}' | '"' | '\''
                                    if depth <= 0 =>
                                {
                                    word_has_quoted_span = true;
                                    break;
                                }
                                _ => {}
                            }
                        }
                        if word_has_quoted_span {
                            // Returns a SCALAR Value, so the empty-Array
                            // shape bit does not describe it — leave the
                            // cell alone. Overwriting it here would let a
                            // trailing quoted-empty segment resurrect a word
                            // an EARLIER empty array already deleted:
                            // `setopt rcexpandparam; a=(); x${a}"${P}"y` is
                            // ZERO words in zsh, and concat_plan9's
                            // `(Array(empty), scalar)` arm returns the scalar
                            // when the bit says "scalar".
                            Value::str(String::new())
                        } else {
                            // The empty `Value::Array` below stands for TWO
                            // different C shapes, and under RC_EXPAND_PARAM
                            // they behave OPPOSITELY:
                            //
                            //   c:Src/subst.c:4362-4365 (plan9, empty ARRAY)
                            //     if (plan9) { uremnode(l, n); return n; }
                            //   → the whole word is deleted:
                            //     `a=(); x${a}y` and `x${${a}}y` are 0 words.
                            //
                            //   c:Src/subst.c:4438-4467 (scalar arm, empty
                            //   SCALAR) — c:4464
                            //     *str = strcatsub(&y, ostr, aptr, x, xlen,
                            //                      fstr, globsubst, copied);
                            //   then c:4467 `setdata(n, (void *) y);`
                            //   → the surrounding text survives:
                            //     `unset P; x${P}y` and `x${${P}}y` are the
                            //     single word `xy`.
                            //
                            // Nesting is NOT the discriminator — shape is.
                            // The word is deleted only when the expansion was
                            // ARRAY-shaped (c:4245 `if (isarr)`) AND produced
                            // zero words, i.e. C's `while ((x = *aval++))`
                            // loop at c:4327 never ran and left `plan9`
                            // non-zero at c:4362. Anything else empty — an
                            // unset/empty scalar, a nested subexp that
                            // resolved to a scalar, a command substitution
                            // with no output — takes c:4438's scalar arm and
                            // keeps the word.
                            note_empty_is_scalar(!(seg_zero_words && seg_is_array));
                            Value::array(Vec::new())
                        }
                    } else {
                        Value::str(only)
                    }
                } else {
                    Value::array(parts.into_iter().map(Value::str).collect())
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
        let dq_flag = vm.pop().to_int() != 0;
        let op = vm.pop().to_int() as u8;
        let repl = vm.pop().to_str();
        let pattern = vm.pop().to_str();
        let name = vm.pop().to_str();
        // !!! DASH-STRICT GATE !!! dash / ash have no `${var/pat/repl}`
        // pattern-replacement expansion — it is a "Bad substitution" error
        // (bash/ksh/POSIX-sh support it, so those modes fall through). Raise
        // the canonical zsh diagnostic + exit 1 to match /bin/dash's failure.
        if crate::dash_mode::dash_strict() {
            crate::ported::utils::zerr("bad substitution");
            crate::ported::utils::errflag.fetch_or(
                crate::ported::zsh_h::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            with_executor(|exec| exec.set_last_status(1));
            return Value::str("");
        }
        // DQ context: C's lexer marks every `$` inside double quotes
        // as the Qstring token (Src/lex.c dquote_parse) and keeps `'`
        // a plain char — so a DQ replacement's `$'…'` is LITERAL in
        // C (Src/subst.c:301 decodes only the tokenized Snull form;
        // `"${a/X/$'\0'}"` keeps the five chars `$'\0'`). The body
        // rebuilt below re-enters stringsubst as raw text, which
        // would mis-decode `$'…'` as ANSI-C; stamp the Qstring
        // marker on the repl's `$` so stringsubst sees the same DQ
        // signal C's tokens carry. The PATTERN side keeps decoding
        // (matches observed zsh: the pattern's `$'\0'` matches a
        // real NUL while the repl's stays literal).
        let repl = if dq_flag {
            repl.replace('$', "\u{8c}")
        } else {
            repl
        };
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
        // c:Src/subst.c:1625 — paramsubst's qt flag. The compiler
        // threads the word's DQ context onto the stack; dropping it
        // (the old `let _dq_flag`) ran the rebuilt body with qt=false
        // whenever the opcode fired outside an EXPAND_TEXT scope, so
        // DQ-only semantics inside the replacement (e.g. `$'` staying
        // literal per Src/subst.c:301 — `"${a/x/$'\t'q}"`) were lost.
        // Bump in_dq_context exactly like EXPAND_TEXT mode 1 so
        // paramsubst_to_value's qt probe sees the right context.
        if dq_flag {
            with_executor(|exec| exec.in_dq_context += 1);
        }
        let ret = paramsubst_to_value(&body);
        if dq_flag {
            with_executor(|exec| exec.in_dq_context -= 1);
        }
        ret
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

/// Shared `${name[idx]}` subscript dispatch for BUILTIN_ARRAY_INDEX
/// and the KSHARRAYS-unset arm of BUILTIN_ARRAY_INDEX_UNBRACED.
///
/// c:Src/subst.c subscript parsing — when paramsubst re-parses the
/// synthesized `${name[idx]}` body, characters like `'` `"` `\` `$`
/// etc. are LEXER-active inside the `[…]` and get reinterpreted
/// (quote-strip, paramsubst recursion, …). For PRE-EVALUATED key
/// strings (the dynamic-key fast path at compile_zsh.rs:3234 already
/// expanded `$k` via EXPAND_TEXT), the idx is a literal string that
/// must match the stored key byte-for-byte — no further
/// reinterpretation. Direct assoc lookup bypasses the lexer for this
/// case, avoiding the quote-strip bug where `h[a'b]` failed to
/// resolve because paramsubst's subscript lexer treated the `'` as a
/// quote. Bug #338. Only fires for simple assoc-name + non-flag idx
/// (no outer-flag sentinels, no `(…)` flag prefix on idx, no splat
/// operator). Other paths (slice, splat, flag-based search,
/// magic-assoc) still flow through paramsubst.
fn array_index_lookup(name: &str, idx: &str) -> Value {
    let idx_is_simple = !idx.starts_with('(') && idx != "@" && idx != "*" && !idx.contains(',');
    if idx_is_simple {
        // assoc_key_hit: single-lock O(1) probe — exec.assoc() clones
        // the WHOLE map per lookup (O(n), quadratic in shell loops).
        // When `name` IS an assoc, exact-key semantics apply to EVERY
        // plain key: hit → value, miss → empty (C `${assoc[missing]}`).
        // Never fall through to the textual `${name[key]}` rebuild —
        // keys carrying `{`/`}`/`[`/`]` (zsh-autopair probes
        // `${AUTOPAIR_LBOUNDS[$pair]}` with pair='{') re-parse as
        // broken syntax there ("failed to compile regex: repetition
        // quantifier…" + a `}` appended per keystroke).
        if let Some((_, v)) = crate::vm_helper::assoc_key_hit(name, idx) {
            return Value::str(v.unwrap_or_default());
        }
    }
    // c:Src/params.c:1449-1450 getindex — a leading `(e)`/`(E)` flag
    // group makes the subscript LITERAL (group consumed, exact key).
    // The textual rebuild below re-parses a FLAT `${name[(e)KEY]}`
    // string, so a `]` / `}` that arrived via `$key` expansion
    // terminates the subscript / brace early — "bad substitution" or
    // spilled-junk values (zpwr expandstats iterates alias keys
    // containing brackets). C never re-parses: getarg scans the
    // TOKENIZED source where expanded data brackets are inert. Do the
    // exact-match lookup directly against the assoc (plain or magic
    // alias tables); search groups ((r)/(i)/(k)/…) and other targets
    // keep the textual path.
    if let Some(rest) = idx.strip_prefix('(') {
        if let Some(close) = rest.find(')') {
            let grp = &rest[..close];
            if !grp.is_empty() && grp.chars().all(|ch| ch == 'e' || ch == 'E') {
                let key = &rest[close + 1..];
                if let Some(hit) = direct_assoc_key_get(name, key) {
                    return Value::str(hit.unwrap_or_default());
                }
            }
        }
    }
    // Plain assoc key that the flat rebuild would mangle (`]` closes
    // the subscript, `}` closes the brace): direct lookup. On a miss
    // return empty — the textual fallback cannot represent the key.
    if (idx.contains(']') || idx.contains('}')) && !idx.starts_with('(') {
        if let Some(hit) = direct_assoc_key_get(name, idx) {
            return Value::str(hit.unwrap_or_default());
        }
    }
    let body = format!("${{{}[{}]}}", name, idx);
    paramsubst_to_value(&body)
}

/// Exact-key read against an assoc-like target WITHOUT the textual
/// `${name[key]}` reparse (see array_index_lookup — expanded `]`/`}`
/// in keys break the flat form). `Some(hit)` when `name` is a target
/// this helper understands (plain assoc, or the alias magic assocs of
/// zsh/parameter — Src/Modules/parameter.c getpmalias family);
/// `None` = not direct-capable, caller keeps the textual path.
fn direct_assoc_key_get(name: &str, key: &str) -> Option<Option<String>> {
    use crate::ported::zsh_h::{ALIAS_GLOBAL, DISABLED};
    // c:Src/Modules/parameter.c:1247+ getpmalias / getpmgalias /
    // getpmsalias — each view filters its table by flags.
    let alias_view = |global: bool, suffix: bool, disabled: bool| -> Option<String> {
        let tab = if suffix {
            crate::ported::hashtable::sufaliastab_lock()
        } else {
            crate::ported::hashtable::aliastab_lock()
        };
        tab.read().ok().and_then(|t| {
            t.iter().find_map(|(k, a)| {
                let f = a.node.flags as u32;
                if k == key
                    && ((f & ALIAS_GLOBAL as u32 != 0) == global || suffix)
                    && ((f & DISABLED as u32 != 0) == disabled)
                {
                    Some(a.text.clone())
                } else {
                    None
                }
            })
        })
    };
    match name {
        "aliases" => Some(alias_view(false, false, false)),
        "galiases" => Some(alias_view(true, false, false)),
        "saliases" => Some(alias_view(false, true, false)),
        "dis_aliases" => Some(alias_view(false, false, true)),
        "dis_galiases" => Some(alias_view(true, false, true)),
        "dis_saliases" => Some(alias_view(false, true, true)),
        _ => {
            // Single-lock O(1) probe (see assoc_key_hit) — the previous
            // double exec.assoc() cloned the whole map twice per lookup.
            crate::vm_helper::assoc_key_hit(name, key).map(|(_, v)| v)
        }
    }
}

/// KSHARRAYS bare-`$name` expansion words for the unbraced
/// no-subscript form (BUILTIN_ARRAY_INDEX_UNBRACED's KSHARRAYS arm).
///
/// - `@` / `*` stay the full positional list (the c:Src/params.c:
///   2293-2296 first-element collapse is gated on
///   `itype_end(t, IIDENT, 1) != t` — an identifier-shaped name —
///   which `@`/`*` are not). The literal `[idx]` then joins the LAST
///   word, matching zsh 5.9: `setopt ksharrays; set -- p q;
///   print -- $@[0]` → `zsh:1: no matches found: q[0]`.
/// - Identifier-named arrays collapse to the FIRST element
///   (c:Src/params.c:2293-2296 `v->end = 1, v->isarr = 0`).
/// - Assocs collapse to the first value in scan order; the `options`
///   magic assoc's scan order is `OPTIONTAB` bucket order (first key
///   `posixargzero`), matching zsh 5.9: `emulate sh -L;
///   print $options[posixargzero]` → `off[posixargzero]`.
/// - Scalars / unset names expand to their value / empty (zsh 5.9:
///   `setopt ksharrays; print -- $unsetvar[0]` →
///   `zsh:1: no matches found: [0]`).
fn ksharrays_bare_words(name: &str) -> Vec<String> {
    if name == "@" || name == "*" {
        return with_executor(|exec| exec.pparams());
    }
    // Magic special-parameter lookups first — mirrors the
    // BUILTIN_GET_VAR precedence (partab before executor tables).
    if let Some(vals) = crate::vm_helper::partab_array_get(name) {
        return vec![vals.into_iter().next().unwrap_or_default()];
    }
    if let Some(keys) = crate::vm_helper::partab_scan_keys(name) {
        let v = keys
            .first()
            .and_then(|k| crate::vm_helper::partab_get(name, k))
            .unwrap_or_default();
        return vec![v];
    }
    let arr_or_assoc = with_executor(|exec| {
        if let Some(arr) = exec.array(name) {
            // c:Src/params.c:2293-2296 — first element only.
            return Some(arr.first().cloned().unwrap_or_default());
        }
        if let Some(map) = exec.assoc(name) {
            // c:Src/params.c:2351-2358 — under KSH EMULATION a bare
            // `$assoc` is `${assoc[0]}` (KEY-"0" lookup), EMPTY unless the
            // hash has a key "0": `emulate -L ksh; typeset -A h=(a 1 b 2);
            // print $h` is empty. Every other mode (`setopt ksharrays`,
            // `emulate sh`) falls through to the first bucket value below.
            if crate::ported::zsh_h::EMULATION(crate::ported::zsh_h::EMULATE_KSH) {
                return Some(map.get("0").cloned().unwrap_or_default());
            }
            // Mirrors BUILTIN_GET_VAR's bare-assoc ordering
            // (sorted keys, first value).
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            return Some(
                keys.first()
                    .and_then(|k| map.get(*k).cloned())
                    .unwrap_or_default(),
            );
        }
        None
    });
    if let Some(v) = arr_or_assoc {
        return vec![v];
    }
    vec![with_executor(|exec| exec.get_variable(name))]
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
    paramsubst_to_value_pf(body, 0)
}

/// `paramsubst_to_value` with an explicit `pf_flags` (`PREFORK_*`) set.
///
/// c:Src/subst.c:1627 — `paramsubst(l, n, str, qt, pf_flags, ret_flags)`.
/// The only caller that needs a non-zero set today is the `${(flags)NAME}`
/// fast path when the word is a scalar-assignment VALUE: C preforks that with
/// `PREFORK_SINGLE|PREFORK_ASSIGN` (c:Src/exec.c:2603 for `x=…`,
/// c:Src/exec.c:4239-4241 for the typeset-family `NAME=…` argument), and
/// `PREFORK_SINGLE` is the `ssub` that turns off c:3913's forced split.
fn paramsubst_to_value_pf(body: &str, pf_flags: i32) -> Value {
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
    let (_full, _pos, nodes) =
        crate::ported::subst::paramsubst(body, 0, qt, pf_flags, &mut ret_flags);
    if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
        with_executor(|exec| exec.set_last_status(1));
    }
    // c:Src/lex.c untokenize — the final argv pass C runs on every
    // expanded word (glob.c:1862 / exec.c) DROPS the Nularg
    // empty-word sentinel (c:2089 `if (c != Nularg)`) that
    // remnulargs faithfully leaves in place (glob.c:3673 re-adds it
    // for all-empty results). These fast-path bridges are terminal —
    // their output lands directly in argv slots — so apply it here.
    // Without it, quoted splits with empty pieces ("${(s:|:)x}" on
    // "|a|b|") leak U+00A1 into argv.
    let nodes: Vec<String> = nodes
        .into_iter()
        .map(|n| crate::ported::lex::untokenize(&n))
        .collect();
    nodes_to_value(nodes)
}

/// Wrap a `Vec<String>` (e.g. paramsubst nodes, multsub parts,
/// xpandbraces output) into a fusevm `Value`: 0 → empty Array, 1 →
/// Str, >1 → Array. Same unwrap idiom every handler that calls a
/// canonical Vec-returning fn does.
/// c:Src/subst.c:1663 — `int plan9 = isset(RCEXPANDPARAM);`
///
/// zsh calls the RC_EXPAND_PARAM word shape "plan9" after the rc(1)
/// shell it comes from. The option is read fresh on every expansion
/// (`setopt` mid-script changes the very next word), so the concat
/// builtins must consult it at RUNTIME, not bake it in at compile time.
fn plan9_active() -> bool {
    with_executor(|_exec| opt_state_get("rcexpandparam").unwrap_or(false))
}

thread_local! {
    /// The `isarr` bit that `Value` cannot carry.
    ///
    /// c:Src/subst.c:4245 `if (isarr)` gates the whole array emit block,
    /// so plan9's word-removal rule (c:4362 `uremnode`) only ever applies
    /// to an ARRAY-valued expansion. An empty SCALAR has `isarr == 0`,
    /// takes the c:4437 scalar branch, and leaves the surrounding text
    /// intact — `setopt rcexpandparam; v=; print -rl -- x$v y` prints `x`
    /// and `y`, while the same line with an empty ARRAY prints only `y`.
    ///
    /// zshrs collapses BOTH shapes to `Value::Array(vec![])`: a real empty
    /// array, and an unquoted empty scalar (collapsed so a standalone `$v`
    /// contributes zero argv words, mirroring prefork's `uremnode` at
    /// c:Src/subst.c:184-187). The two are indistinguishable by the time
    /// they reach a concat builtin, so the expansion builtins record here
    /// whether the empty Array they just produced came from a scalar.
    ///
    /// Sticky until the next expansion overwrites it — a word folds its
    /// segments left-associatively (`concat(concat(x, $e), y)`), so the
    /// propagating second concat must still see the first one's bit.
    /// Consequence: only a builtin that RETURNS an empty `Value::Array` may
    /// write the cell. A builtin returning an empty SCALAR `Value` must leave
    /// it alone, or it resurrects a word an earlier empty array deleted
    /// (`a=(); x${a}"${P}"y` is zero words in zsh) — see the
    /// `word_has_quoted_span` arm of `BUILTIN_EXPAND_TEXT`.
    static EMPTY_EXPANSION_IS_SCALAR: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

/// Record whether the empty expansion just produced was a scalar (`true`)
/// or a genuine array (`false`). See `EMPTY_EXPANSION_IS_SCALAR`.
fn note_empty_is_scalar(is_scalar: bool) {
    EMPTY_EXPANSION_IS_SCALAR.with(|c| c.set(is_scalar));
}

/// True when the empty `Value::Array` about to be concatenated stands for
/// an empty SCALAR (c:4438-4467), not an empty array (c:4362).
fn empty_is_scalar() -> bool {
    EMPTY_EXPANSION_IS_SCALAR.with(|c| c.get())
}

/// Re-assert `was_scalar` when `v` is an empty result.
///
/// For a pass-through stage — one that rewrites word TEXT but cannot turn a
/// scalar into an array or vice versa — the shape bit belongs to whichever
/// expansion produced the value, not to the stage. Such stages usually end in
/// `nodes_to_value`, which records "array" for an empty result, so without
/// this the producer's bit is lost before `concat_plan9` reads it.
/// Non-empty results carry their shape in the `Value` itself and are
/// returned untouched.
fn restore_empty_shape(v: Value, was_scalar: bool) -> Value {
    if matches!(&v, Value::Array(a) if a.is_empty()) {
        note_empty_is_scalar(was_scalar);
    }
    v
}

/// c:Src/subst.c:4366-4437 — the NON-plan9 arm of paramsubst's array
/// emit block: "simply join the first and last values."
///
/// The word prefix (`ostr..aptr`) is concatenated onto element 0
/// (c:4386 `strcatsub(&y, ostr, aptr, x, xlen, NULL, …)`), the interior
/// elements are emitted bare (c:4393-4412), and the word suffix (`fstr`)
/// is concatenated onto the final element (c:4414-4429). Applied left-
/// associatively across a word's segments this reproduces zsh's
/// `pre${arr}post` → `prep` / `q` / `rpost`.
///
/// An EMPTY array never reaches this arm in C: c:4261
/// `if ((!aval[0] || !aval[1]) && !plan9)` collapses it to the scalar ""
/// first, so the surrounding text survives as one word (`x$e y` → `x`).
/// That is what the empty-Array arms below reproduce.
fn concat_splice(lhs: Value, rhs: Value) -> Value {
    match (lhs, rhs) {
        (Value::Array(la), Value::Array(ra)) => {
            if la.is_empty() {
                return Value::Array(ra);
            }
            if ra.is_empty() {
                return Value::Array(la);
            }
            // Last of la merges with first of ra; rest unchanged.
            let mut la = la.to_vec();
            let last_l = la.pop().unwrap();
            let mut ra_iter = ra.iter().cloned();
            let first_r = ra_iter.next().unwrap();
            let l_s = last_l.as_str_cow();
            let r_s = first_r.as_str_cow();
            let mut merged = String::with_capacity(l_s.len() + r_s.len());
            merged.push_str(&l_s);
            merged.push_str(&r_s);
            la.push(Value::str(merged));
            la.extend(ra_iter);
            Value::array(la)
        }
        (Value::Array(la), rhs_scalar) => {
            // c:4261 — empty array + empty surrounding text is zero
            // words, not one empty word. Bug #120 in docs/BUGS.md:
            // `b=("${a[@]:0:-1}")` gave len=1 instead of zsh's len=0.
            let rhs_s = rhs_scalar.as_str_cow();
            if la.is_empty() {
                if rhs_s.is_empty() {
                    return Value::array(Vec::new());
                }
                return Value::str(rhs_s.to_string());
            }
            let mut la = la.to_vec();
            let last = la.pop().unwrap();
            let l_s = last.as_str_cow();
            let mut s = String::with_capacity(l_s.len() + rhs_s.len());
            s.push_str(&l_s);
            s.push_str(&rhs_s);
            la.push(Value::str(s));
            Value::array(la)
        }
        (lhs_scalar, Value::Array(ra)) => {
            let lhs_s = lhs_scalar.as_str_cow();
            if ra.is_empty() {
                // Symmetric c:4261 empty-array rule; see the arm above.
                if lhs_s.is_empty() {
                    return Value::array(Vec::new());
                }
                return Value::str(lhs_s.to_string());
            }
            let mut ra = ra.to_vec();
            let first = ra.remove(0);
            let r_s = first.as_str_cow();
            let mut s = String::with_capacity(lhs_s.len() + r_s.len());
            s.push_str(&lhs_s);
            s.push_str(&r_s);
            let mut out = Vec::with_capacity(ra.len() + 1);
            out.push(Value::str(s));
            out.extend(ra);
            Value::array(out)
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
}

/// c:Src/subst.c:4316-4365 — the plan9 (RC_EXPAND_PARAM) arm of
/// paramsubst's array emit block.
///
/// Every element gets the FULL word prefix and suffix
/// (c:4341 `strcatsub(&y, ostr, aptr, x, xlen, y + 1, …)` inside the
/// per-element loop), giving the cross product with the surrounding
/// text: `pre${arr}post` → `preppost` / `preqpost` / `prerpost`.
///
/// An EMPTY array removes the WHOLE word: the c:4327
/// `while ((x = *aval++))` loop body never runs, so `plan9` is still
/// non-zero at c:4362 and the node is deleted —
/// `if (plan9) { uremnode(l, n); return n; }` (c:4362-4365). `e=();
/// setopt RC_EXPAND_PARAM; print -rl -- x$e y` prints only `y`. This is
/// the opposite of the non-plan9 rule at c:4261, which keeps `x`.
fn concat_plan9(lhs: Value, rhs: Value) -> Value {
    // c:4245 `if (isarr)` — an empty SCALAR never enters the array emit
    // block, so it contributes "" and the word survives (c:4437). Only a
    // real empty ARRAY reaches c:4362's `uremnode`. `Value` cannot tell
    // the two apart; EMPTY_EXPANSION_IS_SCALAR carries the missing bit.
    let scalar_empty = empty_is_scalar();
    match (lhs, rhs) {
        // c:4362-4365 — an empty array on either side deletes the word.
        // Propagated as an empty Array so a later concat in the same word
        // (`x${e[@]}y` folds twice) keeps the word deleted; pop_args
        // splats an empty Array into zero argv words.
        (Value::Array(la), rhs_v) if la.is_empty() => {
            if scalar_empty {
                // Empty scalar prefix: "" + rhs (c:4437 strcatsub).
                return match rhs_v {
                    Value::Array(ra) if ra.is_empty() => Value::array(Vec::new()),
                    other => other,
                };
            }
            Value::array(Vec::new())
        }
        (lhs_v, Value::Array(ra)) if ra.is_empty() => {
            if scalar_empty {
                // Empty scalar suffix: lhs + "" (c:4437 strcatsub).
                return lhs_v;
            }
            Value::array(Vec::new())
        }
        (Value::Array(la), Value::Array(ra)) => {
            let mut out = Vec::with_capacity(la.len() * ra.len());
            for a in la.iter() {
                let a_s = a.as_str_cow();
                for b in ra.iter() {
                    let b_s = b.as_str_cow();
                    let mut s = String::with_capacity(a_s.len() + b_s.len());
                    s.push_str(&a_s);
                    s.push_str(&b_s);
                    out.push(Value::str(s));
                }
            }
            Value::array(out)
        }
        (Value::Array(la), rhs_scalar) => {
            let r = rhs_scalar.as_str_cow();
            let out: Vec<Value> = la
                .iter()
                .map(|a| {
                    let a_s = a.as_str_cow();
                    let mut s = String::with_capacity(a_s.len() + r.len());
                    s.push_str(&a_s);
                    s.push_str(&r);
                    Value::str(s)
                })
                .collect();
            Value::array(out)
        }
        (lhs_scalar, Value::Array(ra)) => {
            let l = lhs_scalar.as_str_cow();
            let out: Vec<Value> = ra
                .iter()
                .map(|b| {
                    let b_s = b.as_str_cow();
                    let mut s = String::with_capacity(l.len() + b_s.len());
                    s.push_str(&l);
                    s.push_str(&b_s);
                    Value::str(s)
                })
                .collect();
            Value::array(out)
        }
        (lhs_s, rhs_s) => {
            // Both scalar: nothing to distribute (c:4444 scalar branch).
            let l = lhs_s.as_str_cow();
            let r = rhs_s.as_str_cow();
            let mut s = String::with_capacity(l.len() + r.len());
            s.push_str(&l);
            s.push_str(&r);
            Value::str(s)
        }
    }
}

/// Flatten one word-segment `Value` into its element strings: an Array splats
/// to its items, a scalar is a single element.
fn word_seg_elems(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => items.iter().map(|i| i.as_str_cow().into_owned()).collect(),
        other => vec![other.as_str_cow().into_owned()],
    }
}

/// Assemble a DQ word from its segments, mixing plan9 (`^`, cross-product) and
/// non-plan9 (splice) segments in one pass — see BUILTIN_WORD_ASSEMBLE_PLAN9.
///
/// c:Src/subst.c:4316-4437 — zsh threads a "growing edge" (`aptr`/`fstr`)
/// through the whole word: an element stays active until a splice freezes all
/// but the last. `active_lo` is the index where that active tail begins.
///   * plan9 segment  → every active element crosses with EVERY new element;
///     all results stay active (c:4316-4350 cartesian). An empty plan9 array
///     deletes the word (c:4362 `uremnode`).
///   * splice segment → every active element takes the FIRST new element, the
///     remaining new elements append as fresh words; the last becomes the new
///     growing edge (c:4366-4437 first/last join). A single-element splice keeps
///     the whole active tail active (nothing frozen). An empty splice array
///     contributes nothing and leaves the word intact.
fn word_assemble_plan9(segments: &[Value], plan9_flags: &[bool]) -> Value {
    let mut words: Vec<String> = Vec::new();
    let mut active_lo: usize = 0;
    let mut started = false;
    for (i, seg) in segments.iter().enumerate() {
        let plan9 = plan9_flags.get(i).copied().unwrap_or(false);
        let elems = word_seg_elems(seg);
        if plan9 && elems.is_empty() {
            // c:4362-4365 — plan9 empty array deletes the whole word.
            return Value::array(Vec::new());
        }
        if !started {
            started = true;
            // c:Src/subst.c:4261 — `if ((!aval[0] || !aval[1]) && !plan9)`.
            // A NON-plan9 EMPTY expansion (empty array, or an empty scalar
            // that zshrs collapsed to the same empty `Value::Array`) is
            // folded into the word text as the empty string and the node
            // SURVIVES (c:4268-4274 `strcatsub` of prefix + "" + suffix).
            // Only the plan9 arm deletes the word, and that is the
            // `uremnode` case already returned above (c:4362-4365).
            //
            // Seeding `words` with that single empty element is what keeps a
            // growing edge alive for the segments that follow. Leaving
            // `words` empty instead made `words[active_lo..]` an empty slice
            // forever, so every later segment cross-multiplied against
            // nothing and the whole word vanished: `n=""; a=(x y z);
            // print -rl -- $n${^a}` printed nothing where zsh prints
            // `x`/`y`/`z`, and `$n$a${^a}` dropped the leading `x`. Only a
            // word whose FIRST segment was the empty one was affected —
            // `pre$n${^a}` already started from the literal and hit the
            // "contributes nothing" `continue` below.
            words = if elems.is_empty() {
                vec![String::new()]
            } else {
                elems
            };
            // plan9 → the whole first array is the growing edge; splice/scalar
            // → only its last element grows, earlier ones are finalized words.
            active_lo = if plan9 {
                0
            } else {
                words.len().saturating_sub(1)
            };
            continue;
        }
        if plan9 {
            let mut new_active = Vec::with_capacity(words[active_lo..].len() * elems.len());
            for a in &words[active_lo..] {
                for r in &elems {
                    new_active.push(format!("{a}{r}"));
                }
            }
            let frozen_len = active_lo;
            words.truncate(frozen_len);
            words.extend(new_active);
            active_lo = frozen_len; // all cross-products stay active
        } else {
            if elems.is_empty() {
                // Non-plan9 empty array contributes nothing; word survives.
                continue;
            }
            let frozen_len = active_lo;
            let r0 = &elems[0];
            let head: Vec<String> = words[active_lo..]
                .iter()
                .map(|a| format!("{a}{r0}"))
                .collect();
            words.truncate(frozen_len);
            words.extend(head);
            words.extend(elems[1..].iter().cloned());
            active_lo = if elems.len() == 1 {
                frozen_len // single-element splice: head stays the growing edge
            } else {
                words.len() - 1 // multi: only the last appended word grows
            };
        }
    }
    match words.len() {
        0 => Value::array(Vec::new()),
        1 => Value::str(words.pop().unwrap()),
        _ => Value::array(words.into_iter().map(Value::str).collect()),
    }
}

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
        // Zero nodes = an ARRAY-shaped expansion that produced no words
        // (empty array splat, empty slice). c:4245 `if (isarr)` holds, so
        // plan9 deletes the surrounding word (c:4362).
        note_empty_is_scalar(false);
        Value::array(Vec::new())
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
                // One empty node = a SCALAR-shaped empty result (c:4437),
                // not an empty array — see EMPTY_EXPANSION_IS_SCALAR.
                note_empty_is_scalar(true);
                return Value::array(Vec::new());
            }
        }
        Value::str(only)
    } else {
        Value::array(stripped.into_iter().map(Value::str).collect())
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
                for item in items.iter() {
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
    // c:Src/exec.c:2709 setunderscore / c:Src/params.c:252 underscore_gsu
    // — `$_` has exactly ONE store in zsh, the `zunderscore` global
    // (`Src/init.c:49`), read back through `underscoregetfn`. It is
    // never written into the parameter table: the `_` Param created by
    // `IPDEF2("_", underscore_gsu, PM_DONTIMPORT)` (c:Src/params.c:326)
    // carries `nullstrsetfn` as its setfn, so even `_=x` stores nothing.
    //
    // The deferred `pending_underscore` → `set_scalar("_")` promotion
    // that used to live here was a second, contradictory store: it wrote
    // the paramtab node, which (a) CLEARED the PM_UNSET that `unset _`
    // had just set — resurrecting the parameter, so `${+_}` flipped back
    // to 1 and `$_` kept reading a value where zsh reports empty — and
    // (b) shadowed the canonical zunderscore value. Every dispatch path
    // now calls `set_zunderscore` (the `setunderscore` equivalent) just
    // before running its command, which is where C sets it
    // (execcmd_exec, c:3545-3547), so the deferral is unnecessary as
    // well: argument expansion has already happened by then, exactly as
    // in C.
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
/// RAII guard that queues signals for the lifetime of a synchronous
/// foreground `waitpid` (via `std::process::Command::status`/`wait`).
///
/// zshrs installs a process-wide SIGCHLD handler (`zhandler` →
/// `wait_for_processes` → `waitpid(-1, WNOHANG)`) that reaps EVERY
/// exited child to drive the job table. `std::process::Command` does
/// its own targeted `waitpid(pid)`; when the reaper fires on any
/// thread between the fork and that wait, it reaps the child first and
/// `Command::status()` fails with ECHILD ("No child processes (os
/// error 10)"). This surfaced as `zshrs: hostname: No child processes
/// (os error 10)` when a coreutils shadow (`coreutils_shadows = off`
/// default) fork-execs `/usr/bin/hostname` while a background prewarm
/// child exits at the same instant.
///
/// Holding this guard bumps `queueing_enabled` (a global SeqCst atomic
/// that `zhandler` honors on every thread), so a SIGCHLD arriving
/// during the wait is pushed onto the deferred queue instead of being
/// reaped — `Command::status()` reaps its own child and reads the real
/// status. On drop, `unqueue_signals()` drains the queue, so any
/// genuine background children that exited meanwhile still get reaped
/// and routed to the job table. This is the same queue_signals /
/// unqueue_signals fencing zsh uses around its own foreground waits
/// (Src/exec.c). Panic-safe via `Drop`.
pub(crate) struct ForegroundWaitGuard;

impl ForegroundWaitGuard {
    #[inline]
    pub(crate) fn enter() -> Self {
        crate::ported::signals_h::queue_signals();
        ForegroundWaitGuard
    }
}

impl Drop for ForegroundWaitGuard {
    #[inline]
    fn drop(&mut self) {
        crate::ported::signals_h::unqueue_signals();
    }
}

fn exec_system_command(name: &str, args: &[String]) -> i32 {
    // c:Src/jobs.c — count the fork so `time` reports for an
    // overridable coreutils shadow run as an external (`time sleep 0`,
    // `time cat …`). This is a distinct spawn path from
    // execute_external_bg; without the bump BUILTIN_TIME_SUBLIST saw no
    // job and stayed silent. (Builtins that don't reach a spawn never
    // hit this fn.)
    crate::vm_helper::FORK_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Queue signals across the wait so the SIGCHLD reaper can't steal
    // this child out from under Command::status — see ForegroundWaitGuard.
    let status = {
        let _wait_guard = ForegroundWaitGuard::enter();
        std::process::Command::new(name)
            .args(args)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
    };
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

/// Like `BUILTIN_GET_VAR` but forces double-quoted (DQ) semantics on
/// the read regardless of the runtime `in_dq_context`. The compiler
/// emits this for a QUOTED simple-var read (`"$name"`) — those compile
/// to a direct GET_VAR with no EXPAND_TEXT wrapper, so `in_dq_context`
/// is 0 and the plain GET_VAR would wrongly word-elide an array's empty
/// elements (`a=(1 "" 3); "$a"` must keep the empty → `1  3`, not
/// `1 3`). With force_dq the array joins via sepjoin keeping empties and
/// a scalar is returned verbatim (no empty-drop, no SH_WORD_SPLIT).
pub const BUILTIN_GET_VAR_DQ: u16 = 639;

/// Builtin ID for `name=value` assignments — pops [name, value] and
/// routes through canonical `setsparam` (Src/params.c:3350).
pub const BUILTIN_SET_VAR: u16 = 284;

/// Builtin ID that sets the thread-local [`SET_VAR_GLOB_ELIGIBLE`] flag true.
/// Emitted by the compiler immediately before a `BUILTIN_SET_VAR` whose scalar
/// RHS carried an UNQUOTED glob token, so the runtime knows the RHS is a literal
/// glob pattern eligible for GLOB_ASSIGN. Takes no stack args, pushes nothing.
pub const BUILTIN_MARK_GLOB_ELIGIBLE: u16 = 640;

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
/// so this lands at 290. Pops the sub-chunk index then the job text; forks;
/// child detaches (`setsid`), runs the sub-chunk on a fresh VM, exits with
/// last_status; parent registers the job in the canonical JOBTAB
/// (initjob/addproc/spawnjob per c:Src/exec.c:1700-1758) so `jobs` / `wait
/// %N` / `kill %N` / `disown` and the zsh/parameter assocs all see it, then
/// returns Status(0) immediately.
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

/// `name[@]=(...)` / `name[*]=(...)` whole-array SET. Identical to
/// BUILTIN_SET_ARRAY for an indexed array / scalar (whole replace), but
/// rejects an associative target with "attempt to set slice of
/// associative array" (c:Src/params.c:3324-3327).
pub const BUILTIN_SET_ARRAY_AT: u16 = 633;

/// `name[@]+=(...)` / `name[*]+=(...)` whole-array APPEND. Indexed
/// append (push), assoc target → same slice-of-assoc error as 633.
pub const BUILTIN_APPEND_ARRAY_AT: u16 = 634;

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

/// `${=name}` / SH_WORD_SPLIT forced IFS split — c:Src/subst.c:3920-3928
/// `aval = sepsplit(val, spsep, 0, 1);`.
///
/// Unlike BUILTIN_WORD_SPLIT (which routes through `multsub`'s
/// PREFORK_SPLIT walker — the c:553-620 loop that COLLAPSES runs of
/// separators and never emits an empty field), this is the *other* zsh
/// splitter: `sepsplit` → `Src/utils.c:3711 spacesplit(s, allownull=0)`,
/// which distinguishes the two IFS classes and preserves empty fields.
///
/// Stack: \[value\]. argc selects the empty-field rule:
///   * argc == 0 — the expansion is a bare unquoted word (`${=v}`): the
///     leading/trailing `""` fields spacesplit emits for skipped
///     IFS-WHITESPACE are empty argv words and prefork deletes them
///     (c:Src/subst.c:184-187 `uremnode`).
///   * argc == 1 — the expansion is quoted (`"${=v}"`) or has adjacent
///     word segments (`x${=v}y`): those fields survive, because in C the
///     word's `Dnull` quote markers / literal prefix+suffix attach to the
///     first and last elements (c:4386 / c:4429 strcatsub) and the node is
///     no longer empty. `v=$' a:b '` → `""`, `a:b`, `""` quoted; `x`,
///     `a:b`, `y` with surrounding text.
///
/// Empty fields that come from an IFS-NON-whitespace separator are the
/// `nulstring` (`Nularg`, c:Src/subst.c:36) and survive in BOTH cases —
/// `IFS=x; v=xaxbx` splits to `""`, `a`, `b`, `""` quoted or not.
pub const BUILTIN_FORCE_SPLIT: u16 = 643;

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
/// Fatal-error-only abort check, for use INSIDE an `&&` / `||` chain.
///
/// A chain suppresses the errexit check (a non-zero status is "consumed"
/// by the connector — `false && x` must not fire ERREXIT or the ZERR
/// trap). But an errflag — a *fatal* error such as a `[[ ]]` bad pattern
/// — is not a status the connector can consume: zsh abandons the whole
/// list. Without this, zshrs ran the `||` right-hand side after the
/// error (`[[ x = [a- ]] || touch f` created `f`; zsh does not) and the
/// aborted builtin then overwrote the cond's status 2 with 1.
///
/// Same errflag arm as `BUILTIN_ERREXIT_CHECK`, with the errexit/ZERR
/// half omitted.
pub const BUILTIN_FATAL_ABORT_CHECK: u16 = 641;
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
/// `loops++` on entry to a compiled for/while/until/repeat
/// (c:Src/loop.c:114/427/523).
pub const BUILTIN_LOOP_ENTER: u16 = 656;
/// `loops--` on exit from a compiled for/while/until/repeat
/// (c:Src/loop.c:188/491/546).
pub const BUILTIN_LOOP_EXIT: u16 = 657;
/// Post-body `if (breaks) { breaks--; … }` drain (c:Src/loop.c:529-534).
/// Int(1) = terminate this loop, Int(0) = next iteration.
pub const BUILTIN_LOOP_BREAK_DRAIN: u16 = 658;
/// Non-consuming `breaks != 0` probe for execlist's per-statement gate
/// (c:Src/exec.c:1370).
pub const BUILTIN_BREAKS_PENDING: u16 = 659;
/// `shtokenize` the top-of-stack string in place — c:Src/subst.c:4419-4420
/// `if (globsubst) shtokenize(y)`, the step that makes a `${~spec}` /
/// `$~spec` value's metachars PATTERN-ACTIVE.
///
/// zshrs expands a `[[ ]]` operand at the VM level and hands `cond_str`
/// (c:Src/cond.c:525) a finished string, so the token state C carries in
/// the word itself has to be re-applied at the point of use. Without it a
/// module condition compiles the value as a literal: `[[ -prefix $~pat ]]`
/// (Completion/Base/Utility/_numbers sh:65) is the one shipped completer
/// that depends on it.
pub const BUILTIN_COND_SHTOKENIZE: u16 = 660;
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
/// Reset `use_cmdoutval` to 0 at the START of a dynamic command (before
/// its words expand), so a command substitution from a PREVIOUS command
/// can't leak into this command's null-command status decision
/// (c:Src/exec.c:3009 `use_cmdoutval = !args`). See BUILTIN_EXEC_DYNAMIC.
pub const BUILTIN_USE_CMDOUTVAL_RESET: u16 = 637;
/// `[[ lhs == pat ]]` / `!=` glob compare — cond-specific so the
/// bad-pattern diagnostic follows Src/cond.c:308-316: zwarnnam
/// "bad pattern: %s" WITHOUT errflag (the script continues) and the
/// cond statement exits 2. Stack: [lhs, pat] → Bool. On compile
/// failure pushes Bool(false) and arms COND_BAD_PATTERN so
/// BUILTIN_COND_STATUS_FROM_BOOL reports 2.
pub const BUILTIN_COND_STRMATCH: u16 = 624;
/// Pops the cond result Bool → Int status per Src/cond.c: true→0,
/// false→1, but 2 when COND_BAD_PATTERN was armed during this cond
/// (covers `!=` where LogNot flips the Bool before status time).
pub const BUILTIN_COND_STATUS_FROM_BOOL: u16 = 625;
/// `[[ ]]` unknown condition. Pops \[op_name\], emits `zerr("unknown
/// condition: %s")` and sets ERRFLAG_ERROR so the next BUILTIN_ERREXIT_CHECK
/// (trigger 4) aborts the input — matching zsh's COND_MODI "unknown condition"
/// path (Src/cond.c:150-188) for a `-X` op with no matching loadable module.
/// Returns Bool(false). Replaces a compile-time `eprintln!` hack that printed
/// the message but never set errflag (so the line didn't abort).
pub const BUILTIN_COND_UNKNOWN: u16 = 632;
/// Bare-`exec` redirect epilogue. Consumes `exec.redirect_failed` and
/// applies the C `done:` tail of execcmd_exec:
///   - c:Src/exec.c:252-259 execerr — `redir_err = lastval = 1` (the
///     failed redirect makes the exec statement exit 1, NOT fatal by
///     itself);
///   - c:Src/exec.c:4367-4386 — `if (isset(POSIXBUILTINS) && (cflags
///     & (BINF_PSPECIAL|BINF_EXEC)) ...) { if (redir_err || errflag)
///     { if (!isset(INTERACTIVE)) exit(1); } }` — POSIX_BUILTINS makes
///     a failed exec redirect fatal in a non-interactive shell.
/// Returns Value::Status(0|1) for the trailing SetStatus.
pub const BUILTIN_EXEC_REDIR_DONE: u16 = 626;
/// Assignment-prefix epilogue for bare `exec` redirects
/// (`x=$(cmd) exec >file`). c:Src/exec.c:3969-3976 — nullexec==1
/// runs addvars THEN, without POSIX_BUILTINS, restores the params
/// (`save_params` / `restore_params`): the RHS side effects fire but
/// the values don't persist. With POSIX_BUILTINS the assignments
/// stick. Pops the BEGIN_INLINE_ENV frame either way.
pub const BUILTIN_EXEC_INLINE_ENV_DONE: u16 = 627;

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
/// `BUILTIN_XTRACE_ARRAY_LINE` — xtrace an `arr=(...)` assignment from the
/// whole assembled `Value::Array` (see compile_zsh array-literal codegen).
pub const BUILTIN_XTRACE_ARRAY_LINE: u16 = 649;
/// `BUILTIN_MAKE_ARRAY_COUNTED` — like `Op::MakeArray(u16)` but the element
/// count is a runtime `Int` on the stack, so it is not capped at 65535. Used
/// by the array-literal codegen only when the literal has > u16::MAX elements
/// (e.g. a .zcompdump's ~103k-element `_comps=(...)`).
pub const BUILTIN_MAKE_ARRAY_COUNTED: u16 = 650;
/// `BUILTIN_ARGV_RFLATTEN` — pop one `Op::MakeArray`-packed argv bundle and
/// push it back as ONE recursively-flattened `Value::Array` of scalars. Emitted
/// by the simple-command codegen ONLY on the >255-arg overflow path: the
/// `Call`/`CallFunction`/`CallBuiltin` opcodes carry argc as a u8, so a command
/// with more than 255 args is packed into a single Array (dispatched with
/// argc=1) instead. But those call ops flatten their argv only ONE level, which
/// would stringify a nested Array (a brace/glob/`$arr` word contributes a
/// `Value::Array`). Pre-flattening here — same descent as
/// [`flatten_array_value`], the array-assignment path — makes the bundle flat
/// so the call op's single-level splat restores every positional arg. Bit
/// compsys: a completer's `_arguments <specs…>` with a large brace-form option
/// set (curl ships 59 `{-x,--long}` specs) dropped the long forms.
pub const BUILTIN_ARGV_RFLATTEN: u16 = 653;
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
/// Closes the current inline-env frame's save list. Emitted right
/// after the prefix assignments of `X=foo cmd` have committed and
/// before `cmd` dispatches, so assignments performed BY the command
/// are not recorded into (and therefore not reverted with) the frame.
/// c:Src/exec.c:4410 `save_params` snapshots only the parsed
/// WC_ASSIGN chain; the list is closed before the builtin/shell
/// function runs. Without the seal, `X=y . file` reverted every
/// global the sourced file assigned — which emptied git's
/// `git-completion.bash` option tables (`__git_log_common_options`
/// et al.) that `_git` sources via `GIT_SOURCING_ZSH_COMPLETION=y . …`.
pub const BUILTIN_SEAL_INLINE_ENV: u16 = 654;

/// End-of-sublist `waitonejob` for a sublist that ran wholly in the
/// current shell. Emitted by `compile_zsh::compile_sublist` after each
/// element of the `&&`/`||` chain whose parse-time `cmplx` flag is set
/// (see `compile_zsh::sublist_elem_is_cmplx`) and whose top level is
/// NOT a multi-stage pipeline.
///
/// c:Src/exec.c:1489-1492 — `execlist` routes each sublist element on
/// the parse-time flag: `if (WC_SUBLIST_FLAGS(code) & WC_SUBLIST_SIMPLE)
/// execsimple(state); else execpline(state, code, ltype, ...);`. Only
/// the `execpline` arm builds a job, and `execpline` ends by calling
/// `waitonejob` on it.
///
/// c:Src/jobs.c:1748-1757 — `waitonejob(Job jn)`:
/// ```c
/// if (jn->procs || jn->auxprocs) zwaitjob(jn - jobtab, 0);
/// else { deletejob(jn, 0); pipestats[0] = lastval; numpipestats = 1; }
/// ```
/// A sublist that forked (a real multi-stage pipeline) takes the
/// `zwaitjob` arm, whose `storepipestats` (c:Src/jobs.c:420) publishes
/// the per-stage array — in zshrs that is `BUILTIN_RUN_PIPELINE`'s own
/// `set_array("pipestatus", ...)`. Every other cmplx sublist runs with
/// an empty proc list and takes the `else` arm, which is what this
/// builtin performs. Resolving which arm applies is a compile-time
/// decision in C (the parse-time flag) and is a compile-time decision
/// here too, so no marker is emitted for the pipeline case at all.
///
/// This is what makes a compound command publish `$pipestatus`:
/// `if ...; fi`, `for ...; done`, `case ... esac`, `while ...; done`,
/// `{ ... }`, `( ... )` and a bare command all reach `execpline` when
/// their body is cmplx, so zsh leaves `numpipestats == 1`. It also
/// makes the OUTER sublist win over an inner pipeline's array —
/// `if true; then true|false; fi` is `n=1 p=(1)`, not `(0 1)` — because
/// the outer procs-less job overwrites what the inner job stored.
///
/// Fires BEFORE the `!` negation (`emit_negate_status`): C applies
/// `WC_SUBLIST_NOT` inside `execpline` after the wait, so
/// `! [[ -z x ]]` records the PRE-negation status — `p=(1)` with `$?`
/// of 0.
pub const BUILTIN_SUBLIST_FINISH: u16 = 655;

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
///
/// ID 644 (unique, next free above the previous max of 643). This was
/// 325, which COLLIDED with BUILTIN_FILE_OLDER (also 325). The VM's
/// builtin table is last-registration-wins, and FILE_OLDER registered
/// after IS_TTY, so every `[[ -t fd ]]` silently dispatched to the
/// file-`-ot` handler: it compared the mtime of a file NAMED by the fd
/// string ("0", "1", …) — which never exists — so `[[ -t 0 ]]` was
/// always false. That broke interactive detection (`[[ -t 0 && -t 1 ]]`)
/// and any config gated on it. c:Src/cond.c:390 `return !isatty(...)`.
pub const BUILTIN_IS_TTY: u16 = 644;
/// Runtime rejection of a process substitution used inside a `[[ … ]]`
/// cond operand. c:Src/exec.c:4918/5040/5069 — `getoutputfile`/`getproc`
/// error `"process substitution %s cannot be used here"` when `thisjob ==
/// -1`, which is the case during cond evaluation. zshrs's THISJOB never
/// distinguishes that context at runtime, so the compiler emits this
/// builtin (gated on `in_cond_operand`) instead of the ProcessSubIn/Out
/// opcode. Pops the substitution text, zerrs, sets errflag (aborting the
/// statement → empty stdout, exit 1, matching zsh), returns empty.
pub const BUILTIN_PROCSUB_COND_ERROR: u16 = 645;
/// `${^arr}` cross-product concat — RC_EXPAND_PARAM forced ON by the `^` flag.
///
/// Distinct from BUILTIN_CONCAT_DISTRIBUTE_FORCED, which the other distribute
/// shapes (`${(@)a}`, `${(f)v}`, `${a[@]}`) share: those keep the word when the
/// array is EMPTY (`x${(@)a}y` → `xy`), but plan9 DELETES it
/// (c:Src/subst.c:4362-4365 `if (plan9) { uremnode(l, n); return n; }`), so
/// `x${^a}y` with `a=()` produces no word at all. One builtin cannot serve both
/// — the plan9-ness is known only at compile time, from the `^` flag itself.
/// Routes to `concat_plan9`, which already ports both c:4362's removal and the
/// c:4316-4350 cartesian emit, and is what the OPTION path
/// (`setopt rcexpandparam`) has always used.
pub const BUILTIN_CONCAT_PLAN9: u16 = 646;
/// `${^^arr}` concat — RC_EXPAND_PARAM forced OFF by the doubled flag
/// (c:Src/subst.c:2553-2555 `plan9 = 0`).
///
/// The mirror of BUILTIN_CONCAT_PLAN9. Needed because every other concat
/// builtin consults `plan9_active()` (the runtime OPTION) and so cross-products
/// anyway under `setopt rcexpandparam`, while `^^` must override the option:
///     setopt rcexpandparam; a=(a b c); print -rl -- ${^^a}.x
///     # zsh: `a`, `b`, `c.x`  — spliced, NOT `a.x b.x c.x`
/// The override is computed in paramsubst but the distribution call is made
/// here, so — like the `^` flag — the only place that knows is the compiler.
/// Routes straight to `concat_splice`, C's non-plan9 join-first-and-last path
/// (c:4366-4437).
pub const BUILTIN_CONCAT_SPLICE_NOPLAN9: u16 = 647;
/// Atomic word assembler for a DQ word that MIXES a plan9 (`^`) segment with a
/// non-plan9 (splice/scalar) segment — e.g. `"${(@)^a}${(@)b}"`.
///
/// The per-pair concat fold (CONCAT_PLAN9 / CONCAT_SPLICE picked ONCE for the
/// whole word) cannot express a word where segment A distributes but segment B
/// splices: a single operator does full-cross OR first/last-splice, never both,
/// and it loses track of which trailing elements are still the "growing edge".
/// zsh (Src/subst.c:4316-4437) instead threads a growing edge through the whole
/// word — an element is "active" until a splice freezes all but the last.
///
/// This builtin ports that edge-tracking directly. Stack (bottom→top):
///   descriptor, seg0, seg1, …, seg(n-1)     with argc = n + 1
/// where `descriptor` is an n-char string, one char per segment: `'1'` = plan9
/// (`^`), `'0'` = splice/scalar/literal. Each segment value is an Array (splat)
/// or scalar (1 element). Result is the assembled Array (or scalar / deleted).
pub const BUILTIN_WORD_ASSEMBLE_PLAN9: u16 = 652;
/// `break N`/`continue N` runtime-count validator (see registration).
pub const BUILTIN_BREAK_COUNT_VALIDATE: u16 = 648;
/// `[[ -r/-w/-x file ]]` via access(2) (doaccess) — see handler.
pub const BUILTIN_COND_ACCESS: u16 = 638;

/// Evaluate a `[[ ]]` module/completion condition (`-prefix`/`-suffix`/
/// `-after`/`-between`). Stack (top-first): argc operand words, then the
/// operator word. Dispatches to `complete::eval_mod_cond`. Result pushed as
/// Bool (true = condition matched). Used by the `ZshCond::ModCond` compile arm.
pub const BUILTIN_COND_MOD: u16 = 651;

/// Update `$LINENO` to track the source line of the next statement.
/// Stack: \[n\] (the line number from `ZshPipe.lineno`). Direct port
/// of zsh's `lineno` global tracking (Src/input.c:330) — the
/// compiler emits one of these per top-level pipe so `$LINENO`
/// reflects the source position at runtime. ID 342 picked because
/// the previous `326` collided with `BUILTIN_HAS_STICKY` (the 325
/// collision between IS_TTY and FILE_OLDER has since been fixed by
/// moving IS_TTY to 644).
pub const BUILTIN_SET_LINENO: u16 = 342;

/// Pop a scalar from the VM stack, run expand_glob on it, push the
/// result as Value::Array. Used by the segment-concat compile path
/// when var refs concatenate with glob meta literals (`$D/*`,
/// `${prefix}*`, etc.) — those skip the bridge's pathname-expansion
/// pass and would otherwise leak the glob meta to argv as a literal.
pub const BUILTIN_GLOB_EXPAND: u16 = 343;

/// MULTIOS-gated glob expansion for redirect-target words
/// (c:Src/glob.c:2161-2167 xpandredir: "Globbing is only done for
/// multios."). Same stack shape as BUILTIN_GLOB_EXPAND; additionally
/// passes the word through literally when `unsetopt multios`.
// 624 is BUILTIN_COND_STRMATCH — the VM's builtin table is
// last-registration-wins, so a duplicate id silently shadows the
// earlier handler.
pub const BUILTIN_REDIR_GLOB_EXPAND: u16 = 628;

/// Reset the default-word glob-pending carrier at the START of a word
/// whose source contains a glob metachar (so the flag never leaks from a
/// prior word/statement). Paired with BUILTIN_DEFAULT_WORD_GLOB.
pub const BUILTIN_DEFAULT_WORD_GLOB_RESET: u16 = 635;

/// Filename-generate the ASSEMBLED word ONLY when the default/alternate
/// paramsubst arm took a SOURCE word carrying glob metachars
/// (subst::DEFAULT_WORD_GLOB_PENDING). A parameter VALUE never sets the
/// flag, so `x='*file'; ${x:-d}` stays literal while `${x:-*file}` /
/// `${x:-a*}bar` glob. Reads+clears the flag. c:Src/subst.c → globlist.
pub const BUILTIN_DEFAULT_WORD_GLOB: u16 = 636;
/// `BUILTIN_SET_LOOP_VAR` constant — for-loop variable binding via
/// `setloopvar` (Src/params.c:6362): a PM_NAMEREF loop var REBINDS
/// to each word (SETREFNAME + setscope) instead of assigning
/// through the chain. Returns Bool(false) when zerr fired
/// (read-only reference / invalid self reference) so the loop
/// driver aborts, mirroring C execfor's errflag check.
pub const BUILTIN_SET_LOOP_VAR: u16 = 629;

/// EXTEND step of typeset paren-init packing. Pops `argc` values:
/// [base, e1, …, eN] — base is either the opener (`name=(` /
/// `name+=(`) or a previous EXTEND result. Pushes base with
/// `\u{1f}` + element appended per element. Array values SPLICE
/// their items as separate elements (`typeset b=( x $arr )` splat);
/// an empty Array contributes nothing (unquoted-empty elision).
/// CallBuiltin's argc is u8, so the compiler emits one EXTEND per
/// ≤200-element chunk — p10k's 408-element `__p9k_colors=( … )`
/// overflowed a single-shot pack (argc wrapped mod 256 and the
/// stack spilled into the arg list: "not an identifier: 173…").
/// BUILTIN_TYPESET_PAREN_CLOSE appends the final `\u{1f})`,
/// yielding the exact REJOIN_SEP-delimited one-arg form
/// bin_typeset's single-arg splitter consumes (builtin.rs ~4891,
/// empties preserved, leading/trailing sentinel-empties trimmed
/// once). One arg in → one arg out: bin_typeset's multi-arg rejoin
/// (paren-depth scan, unsafe on EXPANDED paren-literal elements
/// like p10k's `')' ''`) never runs.
pub const BUILTIN_TYPESET_PAREN_PACK: u16 = 630;

/// CLOSE step — pops the EXTEND chain's result, pushes it with
/// `\u{1f})` appended. See BUILTIN_TYPESET_PAREN_PACK.
pub const BUILTIN_TYPESET_PAREN_CLOSE: u16 = 631;

/// Shared body of BUILTIN_GLOB_EXPAND / BUILTIN_REDIR_GLOB_EXPAND.
/// c:Src/glob.c:1872 — `zglob` runs per-word in the argv pipeline.
/// When the upstream EXPAND_TEXT returned an array (e.g. `${a:e}`
/// splat → ["txt","md"]), glob each element separately, not a
/// sepjoin'd scalar. `skip_glob` short-circuits to a literal
/// pass-through (noglob, or a redirect target under
/// `unsetopt multios`).
fn glob_expand_word_value(raw: Value, skip_glob: bool) -> Value {
    let patterns: Vec<String> = match raw {
        Value::Array(items) => items.iter().map(|v| v.to_str()).collect(),
        other => vec![other.to_str()],
    };
    if skip_glob {
        return if patterns.is_empty() {
            Value::array(Vec::new())
        } else if patterns.len() == 1 {
            Value::str(patterns.into_iter().next().unwrap())
        } else {
            Value::array(patterns.into_iter().map(Value::str).collect())
        };
    }
    let mut out: Vec<String> = Vec::with_capacity(patterns.len());
    for pattern in &patterns {
        // c:Src/subst.c — filename generation runs `filesub` (tilde/`=`
        // expansion) BEFORE globbing. A `~`/`=` reaching this word-glob op
        // comes from `${~spec}` / GLOB_SUBST marking a substituted VALUE:
        // literal and quoted `~` words are filesub'd (or skip glob) upstream
        // and never arrive here. filesubstr matches the Tilde TOKEN, so
        // shtokenize first (raw `~`->Tilde; already-Tilde `${~a[@]}` results
        // pass through), run filesub, then untokenize surviving glob metas
        // for expand_glob. Gated on `~`/`=` (raw or token) so ordinary
        // substituted words skip the roundtrip. Fixes `${~x}` x="~/foo".
        let filesubbed = if pattern.contains('~')
            || pattern.contains('=')
            || pattern.contains(crate::ported::zsh_h::Tilde)
            || pattern.contains(crate::ported::zsh_h::Equals)
        {
            let mut tok = pattern.clone();
            crate::ported::glob::shtokenize(&mut tok);
            crate::ported::lex::untokenize(&crate::ported::subst::filesub(&tok, 0))
        } else {
            pattern.clone()
        };
        let matches = with_executor(|exec| exec.expand_glob(&filesubbed));
        if matches.is_empty() {
            // c:1872 nullglob — drop this word, don't emit a hole
            continue;
        }
        for m in matches {
            out.push(m);
        }
    }
    if out.is_empty() {
        return Value::array(Vec::new());
    }
    if patterns.len() == 1 && out.len() == 1 && out[0] == patterns[0] {
        // No real matches; expand_glob returned the literal. Pass
        // back as scalar so downstream ops don't re-flatten.
        return Value::str(out.into_iter().next().unwrap());
    }
    Value::array(out.into_iter().map(Value::str).collect())
}

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
/// op_byte_N, fd]`. Pops 2N+1 elements; `argc = 2*N + 1`. A target
/// may be a Value::Array of glob matches (spliced into one member
/// per match, c:Src/glob.c:2195-2203); an op may be DUP_WRITE for a
/// numeric `>&N` member (c:Src/exec.c:3895-3917).
///
/// Runtime (MULTIOS set):
///   1. Seed the member list with `dup(1)` when this command's
///      stdout is the pipeline output (c:Src/exec.c:3722-3724).
///   2. Open/dup all targets per their op_byte in redirect order
///      (WRITE truncate + noclobber gate / APPEND / DUP_WRITE live
///      dup); the first member replaces the fd (c:2448-2450).
///   3. Save `dup(fd)` onto the active redirect_scope_stack so
///      `host_redirect_scope_end` restores the original fd.
///   4. Create a pipe; spawn a thread that reads from the pipe
///      read-end and writes every chunk to every opened target.
///   5. dup2 the pipe write-end onto `fd` so the command's writes
///      go through the splitter.
///   6. Track `(pipe_write_fd, JoinHandle)` so scope-end can close
///      the pipe (draining the thread) and join before restoring.
///
/// MULTIOS unset (c:2418 `unset(MULTIOS)` replace arm): each entry
/// is applied as a plain sequential replace via host_apply_redirect
/// — every file still opened/truncated, last one wins.
pub const BUILTIN_MULTIOS_REDIRECT: u16 = 617;

/// MULTIOS input-side concatenation for `cmd < a < b` shapes
/// (Bug #36 input arm). C zsh's `Src/exec.c:2418` mfds dispatch
/// also covers the read direction — when multiple `<` redirects
/// target the same fd, mfds[fd] grows and addfd splices a
/// concatenating cat into the pipe.
///
/// Stack layout (mirrors the write side): `[source_1, op_1,
/// source_2, op_2, …, source_N, op_N, fd]`. Pops 2N + 1 elements
/// (argc = 2N + 1). op is READ for file sources, DUP_READ for
/// numeric `<&N` members; a source may be a Value::Array of glob
/// matches (spliced, c:Src/glob.c:2195-2203).
///
/// Runtime (MULTIOS set):
///   1. Open/dup every source in redirect order; first member
///      replaces the fd (c:Src/exec.c:2448-2450).
///   2. Save `dup(fd)` onto the redirect_scope_stack.
///   3. Create a pipe; spawn a thread that reads each source in
///      order and writes every chunk to the pipe write-end. Close
///      write-end when done so the consumer sees EOF.
///   4. dup2 the pipe read-end onto `fd`.
///   5. Track the JoinHandle so scope-end joins (no fd-close needed
///      here — the producer thread closes its own pipe write-end
///      on exit).
///
/// MULTIOS unset: sequential replace via host_apply_redirect — last
/// source wins (c:2418).
pub const BUILTIN_MULTIOS_READ: u16 = 618;

/// Toggle `ShellExecutor::exec_redirs_permanent`. Emitted by
/// compile_zsh's bare-`exec`-with-redirects arm tightly around each
/// `Op::Redirect`: `LoadInt(1); CallBuiltin; …Redirect…; LoadInt(0);
/// CallBuiltin`. While set, `host_apply_redirect` skips pushing the
/// saved fd into the enclosing redirect scope, making the fd change
/// permanent.
///
/// c:Src/exec.c:3978-3986 — nullexec==1 (`exec` carrying only
/// redirections): "If nullexec is 1 we specifically *don't* restore
/// the original fd's before returning" — the per-execcmd `save[]`
/// dups are closed unrestored. An ENCLOSING group's own saves are a
/// different execcmd's `save[]` and still restore (verified:
/// `{ exec 2>/dev/null; } 2>&1; ls /nope` prints the ls error in zsh).
pub const BUILTIN_EXEC_PERM_REDIRS: u16 = 619;

/// Set `ShellExecutor::pipe_output_pending`. Emitted by compile_pipe
/// at the head of a NON-LAST pipeline-stage sub-chunk when that
/// stage's top-level command carries redirects (`Simple` with redirs
/// or `Redirected` compound). The forked stage child runs the chunk
/// with stdout already dup2'd onto the pipe; the first
/// `host_redirect_scope_begin` (the stage command's own redirect
/// list) consumes the flag into `pipe_output_scope`, enabling the
/// MULTIOS stream-split for fd-1 write redirects in that list.
///
/// c:Src/exec.c:3722-3724 — `addfd(forked, save, mfds, 1, output, 1,
/// NULL)` registers the pipe in mfds[1] in the SAME execcmd that
/// walks the stage command's redirect list; mfds is per-execcmd, so
/// nested body commands (`{ echo a > f; } | cat`) never see it.
pub const BUILTIN_PIPE_OUTPUT_MARK: u16 = 620;

/// Install the pipeline stage's parked fds onto 0/1.
///
/// c:Src/exec.c:3720-3724 — `addfd(forked, save, mfds, 0, input, 0,
/// NULL)` / `addfd(..., 1, output, 1, NULL)`. Runs after prefork
/// (c:3304) and globlist (c:3702) have expanded the stage's argument
/// words, which is why a `$(...)` in those words reads the shell's
/// original stdin rather than the pipe. Emitted by
/// `compile_zsh.rs::emit_stage_fds_install`; the fds themselves are
/// parked by `BUILTIN_RUN_PIPELINE`.
pub const BUILTIN_PIPE_FDS_INSTALL: u16 = 642;

/// Magic-equals prefork for a single arg word of a
/// `BINF_MAGICEQUALS` builtin head (`alias`). Direct port of
/// c:Src/exec.c:3298-3304 — `esprefork = PREFORK_TYPESET;
/// prefork(args, esprefork, NULL)` runs on the argv BEFORE the addfd
/// redirect loop at c:3720, so an expansion zerr (`alias bad===` →
/// equalsubstr "= not found" at Src/subst.c:726) prints to the
/// command's UN-redirected stderr. argc=1: pops the just-pushed
/// word value, runs shtokenize → prefork(PREFORK_TYPESET) →
/// untokenize on it (each element for Array splices), pushes the
/// result back. Emitted by compile_simple per arg word when the
/// dispatch head is `alias`; BUILTIN_ALIAS itself no longer
/// preforks (it would double-fire the diagnostic).
pub const BUILTIN_MAGIC_EQUALS_PREFORK: u16 = 621;

/// Bare (unbraced) `$name[idx]` subscript — same dispatch as
/// `BUILTIN_ARRAY_INDEX` while KSHARRAYS is unset, but under
/// KSHARRAYS the unbraced form does NOT subscript (c:Src/subst.c:
/// 2800-2802 + 2867): `$name` expands bare and `[idx]` stays literal
/// trailing text that undergoes filename generation. Operands:
/// [name, idx, suffix, quoted].
pub const BUILTIN_ARRAY_INDEX_UNBRACED: u16 = 622;

/// Assignment-only simple-command exit status. Direct port of
/// `lv = (errflag ? errflag : cmdoutval)` (c:Src/exec.c:1322,
/// execsimple's WC_ASSIGN arm) / `if (errflag) lastval = 1; else
/// lastval = cmdoutval;` (c:Src/exec.c:3393-3396, execcmd_exec's
/// no-command-word varspc path; redir variant at c:3977). Pops
/// [had_cmd_subst]; cmdoutval is the live vm.last_status when a
/// `$()` ran in any RHS of the chain, 0 otherwise. Writes the
/// canonical LASTVAL (C's single `lastval` global) so the
/// non-interactive errflag abort exits with this value per
/// Src/init.c:234. Caller pairs with SetStatus.
pub const BUILTIN_ASSIGN_ONLY_STATUS: u16 = 623;

/// c:Src/exec.c addvars — `if (!pm) { lastval = 1; if (!cmdoutval)
/// cmdoutval = 1; }`. Set by BUILTIN_SET_VAR on assignsparam
/// failure, consumed by BUILTIN_ASSIGN_ONLY_STATUS so the
/// assignment-only command reports status 1. Process-global like
/// C's `cmdoutval` (function bodies may run on a different thread
/// than the opcode that reads the status back).
pub static ASSIGN_FAILED_FLAG: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

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
        // Shell glob match — `*`, `?`, `[...]`, alternation. After the
        // cond path moved to BUILTIN_COND_STRMATCH, the consumer here
        // is the `case` arm dispatch, whose bad-pattern semantics are
        // Src/loop.c:663-667: `if (!(pprog = patcompile(pat, ...)))
        // zerr("bad pattern: %s", pat);` — errflag set, the arm
        // doesn't match, and the script aborts at the next command
        // boundary (matching `zsh -fc 'case x in [a-) ...'` printing
        // the diagnostic with exit 0 = untouched lastval).
        let mut pat_tok = pattern.to_string();
        crate::ported::glob::tokenize(&mut pat_tok);
        if crate::ported::pattern::patcompile(
            &pat_tok,
            crate::ported::zsh_h::PAT_STATIC as i32,
            None,
        )
        .is_none()
        {
            crate::ported::utils::zerr(&format!("bad pattern: {}", pattern)); // c:667
            return false;
        }
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
        // c:Src/exec.c:4906 getoutputfile — `=(cmd)` (marked "equalsubst" by the
        // compiler) is the TEMP-FILE flavor: create a real regular file, fork a
        // writer whose stdout is the file, WAIT for it (so the file is complete
        // and seekable before the consumer runs), and return the file path. It
        // is unlinked at job end. This differs from `<(cmd)` below, which is a
        // /dev/fd pipe that is never waited on.
        if sub.source == "equalsubst" {
            let nam = crate::ported::utils::gettempname(None, true)
                .unwrap_or_else(|| format!("/tmp/zshrs_eqsub_{}", std::process::id()));
            let cpath = match std::ffi::CString::new(nam.as_str()) {
                Ok(c) => c,
                Err(_) => return String::from("/dev/null"),
            };
            // c:4945 — O_WRONLY|O_CREAT|O_EXCL|O_NOCTTY, 0600.
            let fd = unsafe {
                libc::open(
                    cpath.as_ptr(),
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOCTTY,
                    0o600,
                )
            };
            if fd < 0 {
                return String::from("/dev/null");
            }
            let sub_for_child = sub.clone();
            match unsafe { libc::fork() } {
                -1 => {
                    unsafe { libc::close(fd) };
                    let _ = fs::remove_file(&nam);
                    return String::from("/dev/null");
                }
                0 => {
                    // c:4985 — child: stdout → the temp file, run the body, exit.
                    // Clear the inherited pending-file list so this child never
                    // unlinks the PARENT's =() temp files when its own commands
                    // dispatch (fork copies the list; unlink hits the shared fs).
                    PSUB_PENDING_FILES.with(|v| v.borrow_mut().clear());
                    unsafe {
                        libc::dup2(fd, libc::STDOUT_FILENO);
                        libc::close(fd);
                    }
                    let mut vm = fusevm::VM::new(sub_for_child);
                    register_builtins(&mut vm);
                    vm.set_shell_host(Box::new(ZshrsHost));
                    let _ = vm.run();
                    let _ = std::io::stdout().flush();
                    unsafe { libc::_exit(0) };
                }
                child_pid => {
                    // c:4976-4980 — parent: close the write fd and WAIT so the
                    // file is fully written before the consumer opens it.
                    unsafe {
                        libc::close(fd);
                        let mut status: libc::c_int = 0;
                        libc::waitpid(child_pid, &mut status, 0);
                    }
                    let depth = PSUB_SCOPE_DEPTH.with(|d| d.get());
                    PSUB_PENDING_FILES.with(|v| v.borrow_mut().push((depth, nam.clone())));
                    return nam;
                }
            }
        }
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
                PSUB_PENDING_FILES.with(|v| v.borrow_mut().clear());
                unsafe {
                    libc::close(read_end);
                    libc::dup2(write_end, libc::STDOUT_FILENO);
                    libc::close(write_end);
                }
                // c:Src/exec.c:5101/5150 — `execode(prog, 0, 1, out ?
                // "outsubst" : "insubst");`. execode (c:1245-1266) APPENDS its
                // context for the duration of the body, so `<(cmd)` — whose child WRITES (out=1) — runs as `…:outsubst`.
                // zshrs's ported getproc carries these citations but is NOT the
                // live path (established in #1062) — the VM forks here instead,
                // so the push belongs in this child. No pop needed: this runs
                // INSIDE the forked child and dies with it. Bug #1069 (procsub
                // legs).
                if let Ok(mut ctx) = crate::ported::exec::zsh_eval_context.lock() {
                    ctx.push("outsubst".to_string());
                    let joined = ctx.join(":");
                    if let Ok(mut tab) = crate::ported::params::paramtab().write() {
                        if let Some(pm) = tab.get_mut("zsh_eval_context") {
                            pm.u_arr = Some(ctx.clone());
                            pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                        }
                        if let Some(pm) = tab.get_mut("ZSH_EVAL_CONTEXT") {
                            pm.u_str = Some(joined);
                            pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                        }
                    }
                }
                crate::fusevm_disasm::maybe_print_stdout("process_subst_in", &sub_for_child);
                let mut vm = fusevm::VM::new(sub_for_child);
                register_builtins(&mut vm);
                vm.set_shell_host(Box::new(ZshrsHost));
                let _ = vm.run();
                let _ = std::io::stdout().flush();
                unsafe { libc::_exit(0) };
            }
            child_pid => {
                // c:Src/exec.c:5092 `procsubstpid = pid;` — record the
                // forked child's PID so `${sysparams[procsubstpid]}`
                // returns it (was reading the never-updated atomic, so it
                // always came back 0). p10k's gitstatus daemon reads
                // `sysparams[procsubstpid]` right after `sysopen <(cmd)`
                // to track its worker PID; with 0 the daemon's self-check
                // failed and gitstatus fell back to re-downloading
                // gitstatusd — surfacing as "no prebuilt gitstatusd".
                crate::ported::exec::procsubstpid
                    .store(child_pid, std::sync::atomic::Ordering::Relaxed);
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
                // Park read_end for close-after-consuming-command,
                // exactly like process_sub_out does for its write_end
                // (c:Src/exec.c addfilelist(NULL, fd) → deletefilelist).
                // WITHOUT this the parent's read_end stayed open for
                // the whole shell lifetime: p10k's async worker /
                // realtime clock do `exec {fd}< <(cmd)` on every prompt,
                // so each keystroke/redraw leaked a pipe fd until the
                // ~256-fd limit was hit and the shell locked up
                // (107 leaked pipes + 107 unreaped children observed).
                let depth = PSUB_SCOPE_DEPTH.with(|d| d.get());
                PSUB_PENDING_FDS.with(|v| v.borrow_mut().push((depth, read_end)));
                // Reap the forked child so it doesn't linger as a
                // zombie. `<(cmd)` children are fire-and-forget (their
                // output flows through the pipe); C reaps them via the
                // job machinery. A non-blocking reap here is scheduled;
                // do a best-effort WNOHANG now and the rest drain on
                // subsequent proc-subs / prompt cycles.
                crate::fusevm_bridge::note_psub_child(child_pid);
            }
        }
        format!("/dev/fd/{}", read_end)
    }

    fn process_sub_out(&mut self, sub: &fusevm::Chunk) -> String {
        // c:Src/exec.c:5025 getproc, PATH_DEV_FD branch — `>(cmd)`
        // (out == 0): `mpipe(pipes)`, fork; the CHILD `redup(pipes[0],
        // 0)` (pipe read end onto stdin) and `closem` drops the write
        // end; the PARENT closes pipes[0] and hands the consumer
        // `/dev/fd/<pipes[1]>` (the write end). The previous Rust port
        // used mkfifo + a child that BLOCKED in open(FIFO, O_RDONLY)
        // before running cmd — with no writer the child never started,
        // never exited, and kept its inherited stdout (e.g. a `$()`
        // capture pipe) open forever: `a=$(print -r -- >(true))` hung.
        // With the pipe shape the child runs immediately and exits,
        // releasing inherited fds exactly like zsh (verified: zsh
        // blocks ~2s on `a=$(print -r -- >(sleep 2))`, then EOFs).
        let mut fds: [libc::c_int; 2] = [-1, -1];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            // Pipe creation failed — fall back to a plain temp file so
            // the consumer at least has a writable path.
            let fallback = format!(
                "/tmp/zshrs_psub_out_{}_{}",
                std::process::id(),
                with_executor(|e| {
                    let n = e.process_sub_counter;
                    e.process_sub_counter += 1;
                    n
                })
            );
            let _ = fs::write(&fallback, "");
            return fallback;
        }
        let (read_end, write_end) = (fds[0], fds[1]);
        let sub_for_child = sub.clone();
        match unsafe { libc::fork() } {
            -1 => {
                unsafe {
                    libc::close(read_end);
                    libc::close(write_end);
                }
                String::from("/dev/null")
            }
            0 => {
                // Child: close the write end (c: closem after redup),
                // dup the read end onto stdin (c: redup(pipes[0], 0)),
                // run the sub-chunk, exit. Other std fds stay
                // inherited — zsh's child keeps the surrounding
                // command's stdout/stderr.
                unsafe {
                    libc::close(write_end);
                    libc::dup2(read_end, libc::STDIN_FILENO);
                    libc::close(read_end);
                }
                // c:Src/exec.c:5101/5150 — `execode(prog, 0, 1, out ?
                // "outsubst" : "insubst");`. execode (c:1245-1266) APPENDS its
                // context for the duration of the body, so `>(cmd)` — whose child READS (out=0) — runs as `…:insubst`.
                // zshrs's ported getproc carries these citations but is NOT the
                // live path (established in #1062) — the VM forks here instead,
                // so the push belongs in this child. No pop needed: this runs
                // INSIDE the forked child and dies with it. Bug #1069 (procsub
                // legs).
                if let Ok(mut ctx) = crate::ported::exec::zsh_eval_context.lock() {
                    ctx.push("insubst".to_string());
                    let joined = ctx.join(":");
                    if let Ok(mut tab) = crate::ported::params::paramtab().write() {
                        if let Some(pm) = tab.get_mut("zsh_eval_context") {
                            pm.u_arr = Some(ctx.clone());
                            pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                        }
                        if let Some(pm) = tab.get_mut("ZSH_EVAL_CONTEXT") {
                            pm.u_str = Some(joined);
                            pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                        }
                    }
                }
                crate::fusevm_disasm::maybe_print_stdout("process_subst_out:child", &sub_for_child);
                let mut vm = fusevm::VM::new(sub_for_child);
                register_builtins(&mut vm);
                vm.set_shell_host(Box::new(ZshrsHost));
                let _ = vm.run();
                unsafe { libc::_exit(0) };
            }
            child_pid => {
                // c:Src/exec.c:5143 `procsubstpid = pid;` — same fix as
                // the `<(cmd)` in-path above: record the forked child's
                // PID for `${sysparams[procsubstpid]}` (was always 0).
                crate::ported::exec::procsubstpid
                    .store(child_pid, std::sync::atomic::Ordering::Relaxed);
                // Parent: close the read end, keep the write end open
                // under its fd value so `/dev/fd/N` resolves to the
                // pipe's write side. FD_CLOEXEC must STAY clear —
                // consumers (`tee >(cmd)`) discover the fd via exec
                // inheritance, matching process_sub_in above and C's
                // fdtable[fd] = FDT_PROC_SUBST bookkeeping. Park the
                // fd for close-after-consuming-command (c: addfilelist
                // (NULL, fd) → deletefilelist) so the child's reader
                // EOFs — without this `tee >(wc -c) </dev/null` left
                // wc blocked until shell exit.
                unsafe {
                    libc::close(read_end);
                }
                let depth = PSUB_SCOPE_DEPTH.with(|d| d.get());
                PSUB_PENDING_FDS.with(|v| v.borrow_mut().push((depth, write_end)));
                format!("/dev/fd/{}", write_end)
            }
        }
    }

    fn subshell_begin(&mut self) {
        with_executor(|exec| {
            // Special parameters whose value lives in a process GLOBAL behind a
            // GSU (`Src/params.c`'s `ifs`, `wordchars`, `home`, `histsiz`, …)
            // rather than in the param table. Mirrors the getfn dispatch list at
            // params.rs:12548. C isolates these for free by forking `(...)`;
            // zshrs's in-process subshell has to snapshot them by hand, or a
            // subshell-local `IFS=,` rewrites the parent's word-splitting.
            const SUBSHELL_SPECIAL_GLOBALS: &[&str] = &[
                "IFS",
                "HOME",
                "TERM",
                "USERNAME",
                "WORDCHARS",
                "TERMINFO",
                "TERMINFO_DIRS",
                "KEYBOARD_HACK",
                "histchars",
                "HISTSIZE",
                "SAVEHIST",
            ];
            // An UNSET special yields None and is skipped: the paramtab snapshot
            // restores its PM_UNSET flag, and the getfn dispatch refuses to read
            // the (stale) global while PM_UNSET is set (params.rs:12552).
            let special_globals_snap: Vec<(String, String)> = SUBSHELL_SPECIAL_GLOBALS
                .iter()
                .filter_map(|n| crate::ported::params::getsparam(n).map(|v| ((*n).to_string(), v)))
                .collect();
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
                // c:Src/params.c:854 — a fresh table is 151 buckets, not
                // the 17-bucket `Default`.
                .unwrap_or_else(|| crate::ported::hashtable::hashtable_nodes::newhashtable(151));
            let paramtab_hashed_snap = crate::ported::params::paramtab_hashed_storage()
                .lock()
                .ok()
                .map(|m| m.clone())
                .unwrap_or_default();
            let loop_flags_snap = {
                use std::sync::atomic::Ordering::SeqCst;
                (
                    crate::ported::builtin::LOOPS.load(SeqCst),
                    crate::ported::builtin::BREAKS.load(SeqCst),
                    crate::ported::builtin::CONTFLAG.load(SeqCst),
                )
            };
            exec.subshell_snapshots.push(SubshellSnapshot {
                loop_flags: loop_flags_snap,
                paramtab: paramtab_snap,
                paramtab_hashed_storage: paramtab_hashed_snap,
                special_globals: special_globals_snap,
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
                            .map(|(k, v)| (k.clone(), v.text.clone(), v.node.flags))
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
                // c:Src/exec.c::entersubsh fork semantics — `$!`
                // (clone::lastpid) set by a `&` INSIDE the subshell
                // dies with the child: `( : & ); echo $!` -> 0.
                lastpid: crate::ported::modules::clone::lastpid
                    .load(std::sync::atomic::Ordering::Relaxed),
                // c:Src/exec.c::entersubsh fork semantics — the
                // subshell gets a COPY of the job table; its disown/
                // wait/`&` mutations die with it. Bug #462.
                jobtab: crate::ported::jobs::JOBTAB
                    .get_or_init(|| std::sync::Mutex::new(Vec::new()))
                    .lock()
                    .map(|t| t.clone())
                    .unwrap_or_default(),
                curjob: *crate::ported::jobs::CURJOB
                    .get_or_init(|| std::sync::Mutex::new(-1))
                    .lock()
                    .unwrap(),
                prevjob: *crate::ported::jobs::PREVJOB
                    .get_or_init(|| std::sync::Mutex::new(-1))
                    .lock()
                    .unwrap(),
                maxjob: *crate::ported::jobs::MAXJOB
                    .get_or_init(|| std::sync::Mutex::new(0))
                    .lock()
                    .unwrap(),
                thisjob: *crate::ported::jobs::THISJOB
                    .get_or_init(|| std::sync::Mutex::new(-1))
                    .lock()
                    .unwrap(),
                // c:Src/exec.c entersubsh — fork copies the fd table;
                // the child's `exec >file` / `exec N<&-` mutations die
                // with it. Dup each user-range fd to >= 10 so
                // subshell_end can restore the parent's exact table.
                saved_fds: (0..10)
                    .map(|fd| {
                        let dup = unsafe { libc::fcntl(fd, libc::F_DUPFD, 10) };
                        (fd, dup)
                    })
                    .collect(),
            });
            // C forks for `(...)` — count the fork-equivalent so
            // `time (builtin)` reports like zsh (see FORK_EVENTS).
            crate::vm_helper::FORK_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // c:Src/exec.c:1088-1092 — entersubsh resets traps in the child:
            //     if (!(flags & ESUB_KEEPTRAP))
            //         for (sig = 0; sig <= SIGCOUNT; sig++)
            //             if (!(sigtrapped[sig] & ZSIG_FUNC) &&
            //                 !(isset(POSIXTRAPS) && (sigtrapped[sig] & ZSIG_IGNORED)))
            //                 unsettrap(sig);
            //
            // A subshell does NOT inherit string-form traps. Two exemptions:
            // FUNCTION-form traps (`TRAPUSR1() { … }`, ZSIG_FUNC) survive, and
            // under POSIX_TRAPS an IGNORED trap (`trap '' SIG`) survives.
            //
            // The ZSIG_FUNC exemption is structural here rather than a flag
            // test: zshrs keeps string-form bodies in traps_table and
            // function-form ones in shfunctab as TRAPxxx, so filtering only
            // traps_table leaves the function form untouched by construction.
            // ZSIG_IGNORED is `trap '' SIG`, which stores an empty body.
            //
            // The LOOP BOUND is also part of the spec, not an implementation
            // detail: `sig <= SIGCOUNT` never reaches the PSEUDO-signals,
            // which zsh numbers above the real ones —
            //     #define SIGZERR   (SIGCOUNT+1)
            //     #define SIGDEBUG  (SIGCOUNT+2)      (c:Src/signals.h:34-35)
            // so ERR/ZERR and DEBUG traps SURVIVE a subshell, while SIGEXIT
            // (sig 0) is inside the loop and is cleared. Verified against the
            // oracle: `trap 'print e' ERR; (trap)` lists the ERR trap;
            // `trap 'print u' USR1; (trap)` lists nothing.
            //
            // Without this a subshell kept the parent's traps: `(trap)` listed
            // them where zsh lists nothing, and — the part that isn't
            // cosmetic — an inherited trap FIRED inside the child, so
            // `trap 'print p' USR1; (kill -USR1 $$; print after)` printed
            // p before after instead of after…p (the signal is meant to reach
            // the parent, whose trap runs there).
            //
            // The snapshot pushed above restores the parent's table at
            // subshell_end, which is what makes clearing safe for zshrs's
            // in-process subshell.
            {
                let posixtraps = crate::ported::zsh_h::isset(crate::ported::zsh_h::POSIXTRAPS);
                if let Ok(mut t) = crate::ported::builtin::traps_table().lock() {
                    t.retain(|name, body| {
                        // Above SIGCOUNT — outside c:1088's loop entirely.
                        if name == "ERR" || name == "ZERR" || name == "DEBUG" {
                            return true;
                        }
                        // c:1090-1092 — otherwise keep ONLY (POSIXTRAPS && ignored).
                        posixtraps && body.is_empty()
                    });
                }
            }
            // c:Src/exec.c:2862 — subshell fork flags carry ESUB_PGRP,
            // so entersubsh runs `clearjobtab(monitor)` (c:1219): the
            // child gets an EMPTY job table plus the procless control
            // job grabbed at Src/jobs.c:1828 (`thisjob = initjob()`).
            // That's why zsh's `(jobs)` prints nothing and `(kill %1)`
            // hits the empty control job instead of the parent's job 1.
            // The snapshot pushed above restores the parent's table at
            // subshell_end. Bug #462.
            let monitor = crate::ported::zsh_h::isset(crate::ported::zsh_h::MONITOR) as i32;
            crate::ported::jobs::clearjobtab(&mut exec.jobs, monitor);
            // clearjobtab left THISJOB on the control job (Src/jobs.c:
            // 1828). In C the very next pipeline's execpline reassigns
            // thisjob (Src/exec.c:1700 `thisjob = newjob = initjob()`),
            // so by the time any builtin runs, thisjob never aliases
            // the control job. zshrs has no per-pipeline job slot —
            // model the between-pipelines state (-1) so getjob's
            // `jobnum != thisjob` (c:jobs.c:2107) doesn't reject %1 and
            // setcurjob doesn't demote an inherited curjob that
            // collides with the control slot.
            *crate::ported::jobs::THISJOB
                .get_or_init(|| std::sync::Mutex::new(-1))
                .lock()
                .unwrap() = -1;
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
                // c:Src/exec.c::entersubsh fork semantics — `loops` /
                // `breaks` / `contflag` are process globals the child
                // owns a private copy of, so `(break)` inside a loop
                // cannot end the PARENT's loop. See
                // SubshellSnapshot::loop_flags.
                {
                    use std::sync::atomic::Ordering::SeqCst;
                    let (loops, breaks, contflag) = snap.loop_flags;
                    crate::ported::builtin::LOOPS.store(loops, SeqCst);
                    crate::ported::builtin::BREAKS.store(breaks, SeqCst);
                    crate::ported::builtin::CONTFLAG.store(contflag, SeqCst);
                }
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
                // Restore the global-backed specials (see
                // SubshellSnapshot::special_globals). MUST run after the
                // paramtab restore above: setsparam writes through the GSU setfn
                // to BOTH the process global and the param node, so the paramtab
                // overwrite would otherwise clobber the node half of it.
                for (name, val) in &snap.special_globals {
                    crate::ported::params::setsparam(name, val);
                }
                // c:Src/exec.c::entersubsh fork semantics — restore
                // the parent's `$!`; a background job inside `(...)`
                // dies with the child in C zsh.
                crate::ported::modules::clone::lastpid
                    .store(snap.lastpid, std::sync::atomic::Ordering::Relaxed);
                // c:Src/exec.c::entersubsh fork semantics — restore the
                // parent's job table + curjob/prevjob/maxjob/thisjob.
                // The subshell mutated only its own copy. Bug #462.
                if let Ok(mut t) = crate::ported::jobs::JOBTAB
                    .get_or_init(|| std::sync::Mutex::new(Vec::new()))
                    .lock()
                {
                    *t = snap.jobtab;
                }
                *crate::ported::jobs::CURJOB
                    .get_or_init(|| std::sync::Mutex::new(-1))
                    .lock()
                    .unwrap() = snap.curjob;
                *crate::ported::jobs::PREVJOB
                    .get_or_init(|| std::sync::Mutex::new(-1))
                    .lock()
                    .unwrap() = snap.prevjob;
                *crate::ported::jobs::MAXJOB
                    .get_or_init(|| std::sync::Mutex::new(0))
                    .lock()
                    .unwrap() = snap.maxjob;
                *crate::ported::jobs::THISJOB
                    .get_or_init(|| std::sync::Mutex::new(-1))
                    .lock()
                    .unwrap() = snap.thisjob;
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
                    for (name, text, flags) in snap.aliases {
                        tab.add(crate::ported::zsh_h::alias {
                            node: crate::ported::zsh_h::hashnode {
                                next: None,
                                nam: name,
                                // ALIAS_GLOBAL / DISABLED must survive the
                                // round-trip — flags:0 turned every global
                                // alias regular on ANY subshell exit.
                                flags,
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
                // c:Src/exec.c entersubsh fork semantics — restore the
                // parent's user-range fd table. A bare `exec >file` /
                // `exec N>&-` inside `(...)` died with the C child;
                // the in-process subshell must undo it here. Flush
                // Rust's stdout buffer FIRST so bytes the subshell
                // printed drain to the SUBSHELL's fd 1, not the
                // restored parent fd.
                {
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                }
                for (fd, saved) in snap.saved_fds {
                    unsafe {
                        if saved >= 0 {
                            libc::dup2(saved, fd);
                            libc::close(saved);
                        } else {
                            // fd was closed at entry; close whatever
                            // the subshell opened on that slot.
                            libc::close(fd);
                        }
                    }
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
        // c:Src/exec.c — a `( … )` subshell is a FORK in C: an errflag
        // abort inside the child ends the child with its lastval as
        // the exit status, and the flag dies with the child process.
        // The parent's $? picks up the status and the parent's lists
        // keep running. zsh 5.9: `(readonly r=1; r=2); echo "after
        // $?"` prints `after 1`. zshrs runs the subshell in-process,
        // so mirror the fork isolation by clearing ERRFLAG_ERROR at
        // the subshell boundary — exec.last_status() already carries
        // the child's lastval (synced by ERREXIT_CHECK trigger 4).
        //
        // ERRFLAG_HARD must die at this boundary too: `${u:?msg}` sets
        // errflag |= ERRFLAG_HARD (c:Src/subst.c:3344) and then, in a
        // C forked subshell, `_exit(1)` (c:3353) — the parent never
        // sees ANY errflag bit. A leaked HARD bit here made every
        // subsequent zerr() take the silent arm (c:Src/utils.c:175-177
        // `if (errflag || noerrs) { errflag |= ERRFLAG_ERROR; return; }`),
        // so the next eval/source's parse silently "failed" and the
        // D04 harness shell wedged after chunk 10's
        // `(print ${unset1:?exiting1})`.
        crate::ported::utils::errflag.fetch_and(
            !(crate::ported::zsh_h::ERRFLAG_ERROR | crate::ported::zsh_h::ERRFLAG_HARD),
            std::sync::atomic::Ordering::Relaxed,
        );
        // c:Src/builtin.c:5834 / Src/exec.c:1443 — `retflag` dies at the
        // fork boundary for the same reason `errflag` above does. In C a
        // `return` inside `( … )` sets retflag in the CHILD; the child's
        // execlist unwinds, the child _exit()s, and the PARENT's retflag
        // was never touched. zshrs runs subshells in-process, so the flag
        // survived and returned from the enclosing FUNCTION:
        //   f() { ( return 1 ); print IN }; f
        // printed nothing where zsh prints `IN`. Storing 0 is exactly a
        // restore-to-entry: a non-zero retflag unwinds its list
        // immediately (c:1443's `!retflag` gate), so the parent can never
        // be sitting at a subshell with the flag already set. Twin of the
        // save/restore `$( … )` already does in vm_helper.rs (the
        // `saved_retflag` pair around the cmd-subst sub-VM), and of the
        // loops/breaks/contflag restore in SubshellSnapshot::loop_flags.
        crate::ported::builtin::RETFLAG.store(0, std::sync::atomic::Ordering::Relaxed);
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
        // c:Src/cond.c:113-119 — WHICH engine `=~` uses is an option:
        //
        //   char *modname = isset(REMATCHPCRE) ? "zsh/pcre" : "zsh/regex";
        //
        // and the two speak different languages (POSIX ERE vs PCRE), so the
        // option decides whether `\d` is a digit class or a literal `d`, and
        // whether `(?<name>…)` compiles at all. This dispatch was missing:
        // `=~` always used the regex module, so `setopt rematchpcre` silently
        // did nothing.
        if crate::ported::zsh_h::isset(crate::ported::zsh_h::REMATCHPCRE) {
            // c:115 — "zsh/pcre" → the `-pcre-match` cond.
            crate::ported::modules::pcre::cond_pcre_match(
                &[s_clean, regex_clean],
                crate::ported::modules::pcre::CPCRE_PLAIN,
            ) != 0
        } else {
            // c:115 — "zsh/regex" → the `-regex-match` cond.
            crate::ported::modules::regex::zcond_regex_match(
                &[s_clean.as_str(), regex_clean.as_str()],
                crate::ported::modules::regex::ZREGEX_EXTENDED,
            ) != 0
        }
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
        // c:Src/exec.c getproc + Src/jobs.c deletefilelist — close
        // any `>(cmd)` write ends owned by this command once it
        // finishes (drops on every return path below).
        let _psub_fds = PsubFdGuard;
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
        consume_tilde_globsubst_carrier();
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
        // c:Src/subst.c:505-507 — CSH_NULL_GLOB external-path
        // boundary: command skipped with `no match` but the NEXT
        // sublist runs (zsh -fc 'setopt cshnullglob; ls *nope*;
        // print after' prints the error then `after` — verified
        // zsh 5.9.1), so clear ERRFLAG like the glob_failed arm.
        if consume_badcshglob() {
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
        // c:Src/exec.c:3545-3547 — `setunderscore(lastnode(args))` for the
        // command about to run. Write the canonical `zunderscore` global,
        // NOT the paramtab node: `_`'s setfn is `nullstrsetfn`
        // (c:Src/params.c:252-253), so a table write has no counterpart in
        // C and clobbers the PM_UNSET bit that `unset _` relies on.
        if let Some(last) = args.last() {
            crate::ported::params::set_zunderscore(std::slice::from_ref(last)); // c:3546
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
        let saved_stdout = unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_DUPFD, 10) };
        if saved_stdout < 0 {
            return String::new();
        }
        let saved_stderr = unsafe { libc::fcntl(libc::STDERR_FILENO, libc::F_DUPFD, 10) };
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

        // Nested scope for `>(cmd)` fd ownership — commands inside
        // the cmdsub must not drain the enclosing command's pending
        // psub fds (see PSUB_SCOPE_DEPTH).
        let _psub_scope = PsubScope::enter();

        // c:Src/exec.c:1161 — forked cmdsub child runs entersubsh()
        // which does `zsh_subshell++`; in-process equivalent.
        let _subshell_bump = CmdSubstSubshellBump::enter();

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
        // ACTUALLY A ZSH FUNCTION: zmv/zcp/zln/zcalc are zsh autoload
        // functions, NOT builtins. zshrs ships fast native impls, but they
        // must behave like the zsh functions — command-not-found until
        // `autoload -Uz <name>` creates a function entry. When autoloaded we
        // run the native impl here (short-circuiting the fpath source, which
        // can hang zshrs's parser on zsh-specific syntax); when NOT autoloaded
        // we fall through (return None → resolution ends in command-not-found),
        // matching `zsh -f; zmv` → "command not found: zmv".
        if matches!(name, "zmv" | "zcp" | "zln" | "zcalc")
            && !with_executor(|exec| exec.function_exists(name))
        {
            return None;
        }
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
            // znative — the plugin package manager (src/extensions/pkg/). Installs
            // + loads zsh script and native (Rust cdylib) plugins from a global
            // content-addressed store. `znative add owner/repo`, `znative load`, ...
            "znative" => {
                return Some(crate::extensions::pkg::builtin::znative(&args));
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
        let has_user_fn =
            !user_fn_disabled && with_executor(|exec| exec.functions_compiled.contains_key(name));
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
            let bn_in_tab =
                !disabled && crate::ported::builtin::createbuiltintable().contains_key(name);
            if bn_in_tab {
                return Some(dispatch_builtin_raw(name, args));
            }
            // zshrs-original opcode builtins (async, doctor, peach, …) are not
            // in builtintab, so a run-time-resolved name (`$var`) never reaches
            // them. Dispatch by name here — after ported builtins, before
            // external — matching the shell's function -> builtin -> external
            // order (`has_user_fn` was checked above, so functions still win).
            if let Some(status) = try_run_registered_builtin(name, &args) {
                return Some(status);
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
        {
            let dollar_underscore = args.last().cloned().unwrap_or_else(|| fn_name.clone());
            // c:3546 — zunderscore is the only store; the paramtab write
            // that used to accompany this cleared PM_UNSET (see pop_args).
            crate::ported::params::set_zunderscore(std::slice::from_ref(&dollar_underscore));
        }

        // Delegate the actual function dispatch to the canonical
        // `dispatch_function_call` (which itself wraps the canonical
        // `doshfunc` port from `Src/exec.c:5823`). Single doshfunc
        // call-site keeps scope-mgmt invariants in one place.
        let status = with_executor(|exec| exec.dispatch_function_call(&fn_name, &args));

        // Anonymous functions (`() { … } args`, compiled by
        // parse_anon_funcdef as `_zshrs_anon_N` / `_zshrs_anon_kw_N`)
        // execute exactly ONCE and must not persist. zsh runs the body and
        // frees the function, so `${functions}` / `typeset -f` never show
        // it. Remove every trace right after the single invocation —
        // AFTER `status` is captured, so the body's exit code is preserved
        // ($? — calling `unfunction` here would reset it to 0 instead).
        // Without this, real plugins that use `() { … }` (fzf-tab, zinit,
        // p10k, …) leaked dozens of `_zshrs_anon_N` into `$functions`,
        // diverging from zsh's function table on every such config.
        if fn_name.starts_with("_zshrs_anon_") {
            // `${functions}` / `typeset -f` enumerate the canonical
            // `shfunctab` (via scanpmfunctions); the bytecode call path
            // also keeps the body in the executor's compiled-fn maps. Clear
            // BOTH so no trace of the one-shot anon survives.
            crate::ported::hashtable::removeshfuncnode(&fn_name);
            with_executor(|exec| {
                exec.functions_compiled.remove(&fn_name);
                exec.function_source.remove(&fn_name);
                exec.function_line_base.remove(&fn_name);
                exec.function_def_file.remove(&fn_name);
            });
        }

        // c:Src/exec.c:6207-6265 — doshfunc saves `ou = zunderscore`
        // around the body and runs `setunderscore(ou)` (c:6257) on the way
        // out, so a function call leaves `$_` at the CALL's last argument
        // rather than at whatever the body's last command set. The value
        // saved there is the one execcmd_exec installed just before the
        // call (c:3546), i.e. exactly `args.last()`.
        {
            let last_call_arg = args.last().cloned().unwrap_or_else(|| fn_name.clone());
            crate::ported::params::set_zunderscore(std::slice::from_ref(&last_call_arg)); // c:6257
        }

        status
    }
}

// ───────────────────────────────────────────────────────────────────────────
/// Render a failed-redirect open error the way C's `zerrmsg` `%e` format
/// code does (Src/utils.c): `strerror(errno)` with the first character
/// lowercased, except `EIO` (kept capitalized) and `EINTR` (→ "interrupt").
/// C's redirect open failures call `zwarn("%e: %s", errno, fname)`
/// (Src/exec.c:3741); zshrs's `zwarning` takes a pre-built string, so the
/// `%e` part is built here. Replaces the prior hardcoded `ErrorKind` match
/// that fell back to a generic "redirect failed" for `EROFS`/`EACCES`/etc.
fn redir_errno_msg(err: &std::io::Error) -> String {
    let errno = match err.raw_os_error() {
        Some(n) if n != 0 => n,
        _ => return "redirect failed".to_string(),
    };
    if errno == libc::EINTR {
        return "interrupt".to_string(); // c:zerrmsg %e — EINTR special-case
    }
    let cptr = unsafe { libc::strerror(errno) };
    if cptr.is_null() {
        return "redirect failed".to_string();
    }
    let msg = unsafe { std::ffi::CStr::from_ptr(cptr) }.to_string_lossy();
    if errno == libc::EIO {
        return msg.into_owned(); // c:zerrmsg %e — EIO keeps capitalization
    }
    // c:zerrmsg %e — `fputc(tulower(errmsg[0])); fputs(errmsg + 1)`.
    let mut chars = msg.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => "redirect failed".to_string(),
    }
}

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
                    // When the target fd was already closed (e.g. `exec 0<&-;
                    // cmd < file`), open() returns the lowest free fd, which is
                    // `fd` itself. Then `dup2(fd, fd)` is a no-op: closing new_fd
                    // would CLOSE the fd we just opened, AND — since Rust's
                    // File::open sets O_CLOEXEC and a no-op dup2 does NOT clear
                    // it — an exec'd child would lose the descriptor. So in the
                    // reuse case, keep the fd and clear its close-on-exec flag;
                    // otherwise dup2 (which clears cloexec on the copy) + close.
                    if new_fd != fd {
                        libc::dup2(new_fd, fd);
                        libc::close(new_fd);
                    } else {
                        libc::fcntl(fd, libc::F_SETFD, 0);
                    }
                }
                true
            }
            Err(e) => {
                // c:Src/exec.c:3741 — zwarn("%e: %s", errno, fname) with the
                // real lineno prefix; redir_errno_msg builds the `%e` errno
                // message for all errnos (not just the few hardcoded before).
                let msg = redir_errno_msg(&e);
                crate::ported::utils::zwarn(&format!("{}: {}", msg, target));
                *redirect_failed = true;
                // The /dev/null sink keeps a failed scoped redirect
                // from leaking the aborted command's output to the
                // wrong fd until scope-end restores it. For a bare
                // `exec` redirect (permanent, no scope restore) C
                // leaves the fd UNTOUCHED — execerr() aborts the
                // statement and the original fd 1 keeps flowing
                // (A04redirect: `exec >./nonexistent/x` then `echo
                // output` still prints). c:Src/exec.c:3735-3742.
                let permanent = with_executor(|exec| exec.exec_redirs_permanent);
                if !permanent {
                    if let Ok(devnull) = fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open("/dev/null")
                    {
                        let new_fd = devnull.into_raw_fd();
                        unsafe {
                            if new_fd != fd {
                                libc::dup2(new_fd, fd);
                                libc::close(new_fd);
                            } else {
                                libc::fcntl(fd, libc::F_SETFD, 0);
                            }
                        }
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
                        // c:Src/exec.c — zwarn with real lineno prefix.
                        crate::ported::utils::zwarn(&format!("{}: bad file descriptor", src_fd));
                        self.set_last_status(1);
                        self.redirect_failed = true;
                        return;
                    }
                }
            }
        }
        // c:Src/exec.c:3978-3986 — bare `exec` redirects (nullexec==1)
        // skip the save entirely: "we specifically *don't* restore the
        // original fd's". C's save[] is per-execcmd, so exec's redirs
        // never enter an enclosing group's save list either; pushing
        // into `redirect_scope_stack.last_mut()` here (the enclosing
        // group's scope) made `{ exec 1>&-; … } 2>/dev/null` restore
        // stdout at group end — diverging from zsh, which keeps fd 1
        // closed for the rest of the script.
        if !self.exec_redirs_permanent {
            let saved = unsafe { libc::fcntl(fd, libc::F_DUPFD, 10) };
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
                let saved2 = unsafe { libc::fcntl(2, libc::F_DUPFD, 10) };
                if saved2 >= 0 {
                    if let Some(top) = self.redirect_scope_stack.last_mut() {
                        top.push((2, saved2));
                    } else {
                        unsafe { libc::close(saved2) };
                    }
                }
            }
        }
        // c:Src/exec.c:3722-3724 + 2447-2480 — MULTIOS split when this
        // command's stdout IS the pipeline output. C registers the pipe
        // in mfds[1] (`addfd(forked, save, mfds, 1, output, 1, NULL)`)
        // BEFORE walking the explicit redirect list, so a write-side
        // redirect of fd 1 finds mfds[1] occupied and, with MULTIOS
        // set, "split[s] the stream": fd 1 becomes the write end of an
        // internal pipe whose reader tees every chunk to BOTH the
        // pipeline pipe and the new target. That is why
        // `{ echo a; echo b >&2; } 3>&1 1>&2 2>&3 3>&- | cat` sends
        // `a` to the pipe (via the tee) AND to stderr — plain dup2
        // replacement loses the pipe stream. The scope-depth gate
        // mirrors mfds being per-execcmd: only the redirect list
        // attached to the stage's own command joins the pipe; nested
        // commands inside the body (`{ echo a > f; } | cat`) get a
        // fresh "mfds" and replace as usual.
        if fd == 1
            && self
                .pipe_output_scope
                .is_some_and(|d| d + 1 == self.redirect_scope_stack.len())
            && crate::ported::options::opt_state_get("multios").unwrap_or(true)
        {
            // Resolve the new write target exactly as the plain arms
            // below would, but as a raw fd for the tee.
            let new_target_fd: i32 = match op_byte {
                r::DUP_WRITE => {
                    // Numeric `>&N` only; `-` (close) and `p` (coproc)
                    // fall through to the plain arms.
                    target
                        .trim_start_matches('&')
                        .parse::<i32>()
                        .map(|src| unsafe { libc::fcntl(src, libc::F_DUPFD, 10) })
                        .unwrap_or(-1)
                }
                r::WRITE | r::CLOBBER => fs::File::create(target)
                    .map(|f| f.into_raw_fd())
                    .unwrap_or(-1),
                r::APPEND => fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(target)
                    .map(|f| f.into_raw_fd())
                    .unwrap_or(-1),
                _ => -1,
            };
            if new_target_fd >= 0 {
                let pipe_dup = unsafe { libc::fcntl(1, libc::F_DUPFD, 10) };
                match (pipe_dup >= 0).then(os_pipe::pipe) {
                    Some(Ok((read_end, write_end))) => {
                        // Splitter: same read-loop shape as
                        // BUILTIN_MULTIOS_REDIRECT, with one ordering
                        // refinement. C's tee is a forked process
                        // (closemn → teeproc) whose wakeup latency lets
                        // the stage's DIRECT pipe writes land first —
                        // observed zsh output for `{ echo a; echo b >&2; }
                        // 3>&1 1>&2 2>&3 3>&- | cat` is `b` then `a`,
                        // 15/15 runs. A Rust thread wakes faster than
                        // the debug-build VM dispatches the next echo,
                        // inverting the order. Emulate the C timing
                        // observably: stream to the NEW target (file /
                        // stderr dup) immediately, but defer the
                        // pipe-bound copy until EOF (or a 64KB cap so a
                        // long-running stream still flows instead of
                        // growing memory unboundedly).
                        let write_now = |tfd: i32, data: &[u8]| {
                            let mut off = 0;
                            while off < data.len() {
                                let w = unsafe {
                                    libc::write(
                                        tfd,
                                        data[off..].as_ptr() as *const libc::c_void,
                                        data.len() - off,
                                    )
                                };
                                if w <= 0 {
                                    break;
                                }
                                off += w as usize;
                            }
                        };
                        let handle = std::thread::spawn(move || {
                            let mut rd = read_end;
                            let mut buf = [0u8; 8192];
                            let mut pipe_pending: Vec<u8> = Vec::new();
                            loop {
                                match std::io::Read::read(&mut rd, &mut buf) {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => {
                                        write_now(new_target_fd, &buf[..n]);
                                        pipe_pending.extend_from_slice(&buf[..n]);
                                        if pipe_pending.len() >= 65536 {
                                            write_now(pipe_dup, &pipe_pending);
                                            pipe_pending.clear();
                                        }
                                    }
                                }
                            }
                            write_now(pipe_dup, &pipe_pending);
                            unsafe {
                                libc::close(pipe_dup);
                                libc::close(new_target_fd);
                            }
                        });
                        let write_raw = AsRawFd::as_raw_fd(&write_end);
                        unsafe { libc::dup2(write_raw, 1) };
                        drop(write_end);
                        // Scope-end closes this dup (the last writer once
                        // the saved fd 1 is restored) → EOF → join.
                        let close_on_end = unsafe { libc::fcntl(1, libc::F_DUPFD, 10) };
                        if let Some(top) = self.multios_scope_stack.last_mut() {
                            top.push((close_on_end, handle));
                        } else {
                            unsafe { libc::close(close_on_end) };
                            let _ = handle.join();
                        }
                        return;
                    }
                    _ => unsafe {
                        // pipe()/dup failure — fall through to plain replace.
                        if pipe_dup >= 0 {
                            libc::close(pipe_dup);
                        }
                        libc::close(new_target_fd);
                    },
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
                let target_meta = std::fs::metadata(target).ok();
                let target_is_regular_file = target_meta
                    .as_ref()
                    .map(|m| m.file_type().is_file())
                    .unwrap_or(false);
                // c:Src/exec.c:2313 clobber_open — CLOBBER_EMPTY permits
                // re-using an EMPTY regular file under noclobber: `setopt
                // noclobber clobberempty; : >f; echo hi >f` overwrites f.
                // The inline bridge check ignored this and errored.
                let clobber_empty_ok = opt_state_get("clobberempty").unwrap_or(false)
                    && target_meta.as_ref().map(|m| m.len() == 0).unwrap_or(false);
                if noclobber && target_is_regular_file && !clobber_empty_ok {
                    eprintln!(
                        "{}:{}: file exists: {}",
                        shname(),
                        crate::ported::lex::lineno(),
                        target
                    );
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
                if !Self::redir_open_or_fail(fd, open_result, target, &mut self.redirect_failed) {
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
                        // See redir_open_or_fail: when the opened fd IS the
                        // destination (target fd was closed), keep it and clear
                        // O_CLOEXEC; else dup2 + close.
                        if new_fd != fd {
                            libc::dup2(new_fd, fd);
                            libc::close(new_fd);
                        } else {
                            libc::fcntl(fd, libc::F_SETFD, 0);
                        }
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
                } else if op_byte == r::DUP_WRITE {
                    // c:Src/glob.c:2184-2187 xpandredir — a MERGEOUT
                    // word that expands to a non-number becomes
                    // REDIR_ERRWRITE: `cmd >& word` opens `word` and
                    // routes BOTH fd 1 and fd 2 there. Reached only
                    // for dynamic words (`>&$var`); static filenames
                    // were converted at compile time.
                    if let Ok(file) = fs::File::create(target) {
                        let new_fd = file.into_raw_fd();
                        unsafe {
                            libc::dup2(new_fd, 1);
                            libc::dup2(new_fd, 2);
                            libc::close(new_fd);
                        }
                    }
                } else {
                    // c:Src/glob.c:2185 — MERGEIN non-number:
                    // `zerr("file number expected")`.
                    crate::ported::utils::zerr("file number expected");
                    self.set_last_status(1);
                    self.redirect_failed = true;
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
        // c:Src/exec.c:3722-3724 — the pipeline child set
        // `pipe_output_pending` right after dup2'ing its stdout onto
        // the pipe; the FIRST redirect scope opened in that child is
        // the stage command's own redirect list (same execcmd as the
        // pipe's addfd into mfds[1]). Capture the depth so only THAT
        // list's fd-1 write redirects MULTIOS-join the pipe.
        if self.pipe_output_pending {
            self.pipe_output_pending = false;
            self.pipe_output_scope = Some(self.redirect_scope_stack.len());
        }
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
            // Close ALL tracked writer dups BEFORE joining any
            // thread. When one splitter holds a dup of another's
            // pipe write-end (two multios in one scope where a later
            // one duped fd 1 while an earlier splitter owned it),
            // joining in push order deadlocks: splitter A's EOF
            // waits on splitter B's writer dup, which only closes
            // after B's thread exits — blocked behind A's join.
            let mut handles = Vec::with_capacity(scope.len());
            for (write_fd, handle) in scope {
                if write_fd >= 0 {
                    unsafe {
                        libc::close(write_fd);
                    }
                }
                handles.push(handle);
            }
            for handle in handles {
                let _ = handle.join();
            }
        }
        // The scope that captured the pipeline-output marker is gone;
        // deeper-nested future scopes must not re-match its depth.
        if self.pipe_output_scope == Some(self.redirect_scope_stack.len()) {
            self.pipe_output_scope = None;
        }
    }

    /// Set up `content` as stdin (fd 0) for the next command.
    /// Used by `Op::HereDoc(idx)` and `Op::HereString`.
    ///
    /// c:Src/exec.c:4655 getherestr — C writes the body to a TEMP
    /// FILE (gettempfile → write_loop → close → reopen O_RDONLY →
    /// unlink), NOT a pipe. The previous pipe+writer-thread shape
    /// SIGPIPE'd the whole shell when the consumer never read the
    /// body (`: <<< ${(F)x/y}` — D04parameter chunk 211, flaky
    /// rc=141): the redirect-scope teardown closed the read end
    /// while the detached thread was still in write_all, and the
    /// shell's SIGPIPE disposition is SIG_DFL. A temp file has no
    /// reader/writer coupling — matching C exactly, including
    /// lseek-ability of fd 0, which pipes don't give.
    pub fn host_set_pending_stdin(&mut self, content: String) {
        // c:4673 — `gettempfile(NULL, 1, &s)`.
        let mut tmp = std::env::temp_dir();
        tmp.push(format!(
            "zshrs-herestr-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        // c:4675 — `write_loop(fd, t, len); close(fd);`
        if std::fs::write(&tmp, content.as_bytes()).is_err() {
            return; // c:4674 — tempfile failure → no redirect
        }
        // c:Src/utils.c gettempfile → mkstemp creates the temp file mode
        // 0600 IGNORING the umask, so the O_RDONLY reopen below always
        // succeeds. `std::fs::write` honors the umask, so under `umask
        // 0777` the file landed mode 0000 and the reopen failed with
        // EACCES — `cat <<<x` then read empty stdin. Force 0600 to match
        // mkstemp's umask-independent permissions.
        let _ = std::fs::set_permissions(
            &tmp,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        );
        // c:4677 — `fd = open(s, O_RDONLY | O_NOCTTY);`
        let file = match std::fs::File::open(&tmp) {
            Ok(f) => f,
            Err(_) => {
                let _ = std::fs::remove_file(&tmp);
                return;
            }
        };
        // c:4678 — `unlink(s);` — fd stays valid, name disappears.
        let _ = std::fs::remove_file(&tmp);
        let saved = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_DUPFD, 10) };
        if saved >= 0 {
            if let Some(top) = self.redirect_scope_stack.last_mut() {
                top.push((libc::STDIN_FILENO, saved));
            } else {
                unsafe { libc::close(saved) };
            }
        }
        // c:Src/utils.c:redup — `if (x != y) { dup2(x, y); zclose(x); }`.
        // When fd 0 was already CLOSED before this heredoc runs,
        // `File::open` returns the lowest free descriptor, which is 0
        // itself — so `read_fd == STDIN_FILENO`. C's redup skips both the
        // dup2 (a no-op for equal fds) AND the close in that case, leaving
        // the just-opened temp file installed at fd 0. Unconditionally
        // dropping the File here closed that fd back to nothing, so an
        // external NULLCMD (`cat`) inherited a closed fd 0 and failed with
        // EBADF (`cat <<EOF` inside `$(...)` when exec 0<&- closed stdin).
        let read_fd = AsRawFd::as_raw_fd(&file);
        if read_fd != libc::STDIN_FILENO {
            // dup2 installs a fresh fd 0 with FD_CLOEXEC clear (dup2 never
            // copies the flag), then we close the CLOEXEC-tagged source.
            unsafe { libc::dup2(read_fd, libc::STDIN_FILENO) };
            drop(file); // c:redup zclose(x)
        } else {
            // File::open reused fd 0. Rust opens with O_CLOEXEC, so fd 0
            // now carries FD_CLOEXEC and would be auto-closed when an
            // external NULLCMD (`cat`) exec's — the child then reads a
            // closed fd 0 and fails with EBADF. zsh opens the heredoc temp
            // via `open(s, O_RDONLY|O_NOCTTY)` (no CLOEXEC), so its child
            // inherits the fd. Clear the flag to match, then keep fd 0 open
            // (redup's x==y arm: no dup2, no close).
            unsafe {
                let flags = libc::fcntl(libc::STDIN_FILENO, libc::F_GETFD);
                if flags >= 0 {
                    libc::fcntl(libc::STDIN_FILENO, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                }
            }
            std::mem::forget(file);
        }
    }

    /// Spawn an external command using zshrs's full dispatch logic
    /// (intercepts, command_hash, redirect handling). Used by
    /// `ZshrsHost::exec` so the bytecode VM's `Op::Exec` and
    /// `Op::CallFunction` external fallback get the same semantics as
    /// the tree-walker's `execute_external` rather than a plain
    /// `Command::new` shortcut. Returns the exit status.
    pub fn host_exec_external(&mut self, args: &[String]) -> i32 {
        // Native p10k API: the `p10k(){ zshrs-p10k-api "$@" }` stub's
        // body lands here (the name is neither function nor builtin).
        // Route into the engine instead of a PATH miss.
        if let Some(name) = args.first() {
            if let Some(status) = crate::p10k::maybe_intercept_command(name, &args[1..]) {
                self.set_last_status(status);
                return status;
            }
        }
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
        consume_tilde_globsubst_carrier();
        if self.current_command_glob_failed.get() {
            self.current_command_glob_failed.set(false);
            crate::ported::utils::errflag.fetch_and(
                !crate::ported::zsh_h::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            self.set_last_status(1);
            return 1;
        }
        // c:Src/subst.c:505-507 — CSH_NULL_GLOB sibling of the
        // NOMATCH gate above, same external-path semantics (skip
        // command, `no match`, clear ERRFLAG so the next sublist
        // runs).
        if consume_badcshglob() {
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
        match cmd.as_str() {
            "sched" => return dispatch_builtin("sched", rest_vec.clone()),
            "echotc" => return dispatch_builtin("echotc", rest_vec.clone()),
            "echoti" => return dispatch_builtin("echoti", rest_vec.clone()),
            "zpty" => return dispatch_builtin("zpty", rest_vec.clone()),
            "ztcp" => return dispatch_builtin("ztcp", rest_vec.clone()),
            "zsocket" => {
                // c:Src/Modules/socket.c:276 BUILTIN spec — BUILTINS["zsocket"]
                // optstr "ad:ltv" parsed by execbuiltin.
                return dispatch_builtin("zsocket", rest_vec.clone());
            }
            "private" => {
                // c:Src/Modules/param_private.c:217 — bin_private via
                // BUILTINS["private"]. The autoload require_module
                // (exec.c:2700-2717) fires inside
                // dispatch_builtin_raw, the chokepoint for all routes.
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
            // ACTUALLY A ZSH FUNCTION: zmv/zcp/zln/zcalc are zsh autoload
            // functions — implemented natively in Rust so `autoload -Uz zmv`
            // works without shipping the function source (and without the
            // fpath source hanging the parser). The `function_exists` guard
            // keeps them command-not-found until autoloaded, exactly like zsh;
            // an un-guarded arm ran them for bare `zmv`, diverging from
            // `zsh -f; zmv` → "command not found: zmv".
            "zmv" if self.function_exists("zmv") => {
                return crate::extensions::ext_builtins::zmv(&rest_vec, "mv")
            }
            "zcp" if self.function_exists("zcp") => {
                return crate::extensions::ext_builtins::zmv(&rest_vec, "cp")
            }
            "zln" if self.function_exists("zln") => {
                return crate::extensions::ext_builtins::zmv(&rest_vec, "ln")
            }
            "zcalc" if self.function_exists("zcalc") => {
                return crate::extensions::ext_builtins::zcalc(&rest_vec)
            }
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
            | "zf_ln" | "zf_mv" | "zf_sync"
                // `--zsh` parity gate: zsh -fc has zsh/files UNLOADED
                // — bare `mkdir` is /bin/mkdir (so `command mkdir -p`
                // honors the system flag set; zconvey.plugin.zsh:44
                // got "File exists" from the in-process bin_mkdir
                // that this arm intercepted) and `zf_*` names are
                // command-not-found 127 until `zmodload zsh/files`.
                // Fall through to the external/exec path in --zsh
                // mode; default zshrs mode keeps the anti-fork
                // intercept.
                if !crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) =>
            {
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

#[cfg(test)]
mod word_assemble_tests {
    use super::{word_assemble_plan9, Value};

    fn arr(xs: &[&str]) -> Value {
        Value::array(xs.iter().map(|s| Value::str(*s)).collect())
    }
    fn out(v: Value) -> Vec<String> {
        match v {
            Value::Array(items) => items.iter().map(|i| i.to_str()).collect(),
            other => vec![other.to_str()],
        }
    }

    // The edge-tracking fold (c:Src/subst.c:4316-4437). A naive per-segment
    // operator gets s,p,p and p,s,p wrong because it forgets which trailing
    // elements are still the "growing edge". These pin the exact zsh output
    // (verified against zsh 5.9) for every plan9/splice permutation.
    #[test]
    fn plan9_then_splice() {
        // "${(@)^a}${(@)b}" a=(1 2) b=(A B) -> 1A 2A B
        let r = word_assemble_plan9(&[arr(&["1", "2"]), arr(&["A", "B"])], &[true, false]);
        assert_eq!(out(r), vec!["1A", "2A", "B"]);
    }
    #[test]
    fn splice_then_plan9() {
        // "${(@)a}${(@)^b}" -> 1 2A 2B
        let r = word_assemble_plan9(&[arr(&["1", "2"]), arr(&["A", "B"])], &[false, true]);
        assert_eq!(out(r), vec!["1", "2A", "2B"]);
    }
    #[test]
    fn plan9_then_plan9_is_full_cross() {
        let r = word_assemble_plan9(&[arr(&["1", "2"]), arr(&["A", "B"])], &[true, true]);
        assert_eq!(out(r), vec!["1A", "1B", "2A", "2B"]);
    }
    #[test]
    fn splice_then_splice() {
        let r = word_assemble_plan9(&[arr(&["1", "2"]), arr(&["A", "B"])], &[false, false]);
        assert_eq!(out(r), vec!["1", "2A", "B"]);
    }
    #[test]
    fn plan9_splice_plan9_growing_edge() {
        // "${(@)^a}${(@)b}${(@)^c}" -> 1A 2A Bp Bq  (only B, the edge, distributes)
        let r = word_assemble_plan9(
            &[arr(&["1", "2"]), arr(&["A", "B"]), arr(&["p", "q"])],
            &[true, false, true],
        );
        assert_eq!(out(r), vec!["1A", "2A", "Bp", "Bq"]);
    }
    #[test]
    fn splice_plan9_plan9_keeps_frozen_prefix() {
        // "${(@)a}${(@)^b}${(@)^c}" -> 1 2Ap 2Aq 2Bp 2Bq  (1 stays frozen)
        let r = word_assemble_plan9(
            &[arr(&["1", "2"]), arr(&["A", "B"]), arr(&["p", "q"])],
            &[false, true, true],
        );
        assert_eq!(out(r), vec!["1", "2Ap", "2Aq", "2Bp", "2Bq"]);
    }
    #[test]
    fn empty_plan9_array_deletes_word() {
        // "${(@)^a}${(@)b}" with a=() -> word deleted
        let r = word_assemble_plan9(&[Value::array(vec![]), arr(&["A", "B"])], &[true, false]);
        assert!(out(r).is_empty(), "plan9 empty array deletes the word");
    }
    #[test]
    fn leading_literal_then_mixed() {
        // "X${(@)^a}${(@)b}" -> X1A X2A B
        let r = word_assemble_plan9(
            &[Value::str("X"), arr(&["1", "2"]), arr(&["A", "B"])],
            &[false, true, false],
        );
        assert_eq!(out(r), vec!["X1A", "X2A", "B"]);
    }

    // c:Src/subst.c:4261 — a NON-plan9 empty expansion collapses to the empty
    // string and the word SURVIVES; only plan9 (c:4362 `uremnode`) deletes it.
    // A leading empty segment used to leave `words` empty, so every following
    // segment cross-multiplied against nothing and the word vanished.
    // Verified against zsh 5.9:
    //     n=""; a=(x y z); print -rl -- $n${^a}      -> x / y / z
    #[test]
    fn leading_empty_splice_keeps_word_and_crosses() {
        let r = word_assemble_plan9(
            &[Value::array(vec![]), arr(&["x", "y", "z"])],
            &[false, true],
        );
        assert_eq!(out(r), vec!["x", "y", "z"]);
    }

    //     n=""; a=(x y z); print -rl -- $n"pre"${^a} -> prex / prey / prez
    #[test]
    fn leading_empty_then_literal_then_plan9() {
        let r = word_assemble_plan9(
            &[Value::array(vec![]), Value::str("pre"), arr(&["x", "y", "z"])],
            &[false, false, true],
        );
        assert_eq!(out(r), vec!["prex", "prey", "prez"]);
    }

    //     n=""; a=(x y z); print -rl -- $n$a${^a} -> x / y / zx / zy / zz
    // The leading empty must not consume the splice's first element.
    #[test]
    fn leading_empty_does_not_eat_first_splice_element() {
        let r = word_assemble_plan9(
            &[
                Value::array(vec![]),
                arr(&["x", "y", "z"]),
                arr(&["x", "y", "z"]),
            ],
            &[false, false, true],
        );
        assert_eq!(out(r), vec!["x", "y", "zx", "zy", "zz"]);
    }

    //     n=""; e=(); print -rl -- $n${^e} -> nothing (plan9 empty still wins)
    #[test]
    fn leading_empty_then_empty_plan9_still_deletes_word() {
        let r = word_assemble_plan9(
            &[Value::array(vec![]), Value::array(vec![])],
            &[false, true],
        );
        assert!(out(r).is_empty(), "plan9 empty array still deletes the word");
    }
}
