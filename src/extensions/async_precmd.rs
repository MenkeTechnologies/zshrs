//! `async_precmd` hook — run precmd-style functions on a POOL WORKER THREAD so
//! they never block prompt rendering.
//!
//! zsh's `precmd` hooks run synchronously before the prompt paints, so a slow
//! hook stalls the prompt. `async_precmd` is a new lifecycle hook (no zsh
//! equivalent): its functions run on the shared worker pool AFTER the prompt is
//! built/rendered, writing their results into the shared, `RwLock`-synchronized
//! global param table. The prompt reads whatever is currently there and never
//! waits — a slow segment simply updates a prompt or two later.
//!
//! Registration mirrors zsh's hook arrays:
//!   * a function literally named `async_precmd`, and/or
//!   * members of the `async_precmd_functions` array.
//!
//! Built on the Phase-1 [`crate::vm_helper::ShellExecutor::new_worker`]
//! lightweight worker executor. No isolation is needed here: `async_precmd`
//! WANTS its `typeset -g` writes to land in the shared table.
//!
//! ## The batch never overlaps shell code on the shell thread
//!
//! A hook is a shell FUNCTION, and running one moves state C keeps for a
//! single thread of execution: `locallevel` (`Src/params.c:5837`
//! `locallevel++` in `startparamscope`, `:5856` `locallevel--` in
//! `endparamscope`) and the level-stamped entries of the one `paramtab`,
//! which `scanendscope` (`Src/params.c:5904-5907`, `if (pm->level >
//! locallevel)`) restores or deletes on every scope exit. The worker's
//! pushes and pops land on the same counter and the same table as the
//! shell thread's.
//!
//! The batch is fired from `preprompt()`, and the first thing the shell
//! thread does after the prompt paints is usually run a widget: a
//! shell-function widget brackets its body with `startparamscope();
//! makezleparams(0); … endparamscope();` (`Src/Zle/zle_main.c:1533-1540`),
//! and `makezleparams` stamps `$BUFFER` and its family with the scope level
//! (`Src/Zle/zle_params.c:206`). A worker scope exit that lands inside that
//! window deletes the widget's `$BUFFER`, and the write-back in
//! `zle_param_sync::sync_from_paramtab` then copies the now-empty value
//! into the editor: every character typed so far on the line disappears.
//! Measured with a self-insert wrapper (`zle -N self-insert f; f() { zle
//! .self-insert }`, the shape zpwr's `zpwrSelfInsert` has) and a hook that
//! runs for half a second: most typed lines lost a prefix, and a trace
//! showed the worker's `endparamscope` removing `BUFFER` at the widget's
//! level immediately before the editor was overwritten with "".
//! A top-level `typeset X=1` typed during the batch was stamped with the
//! worker's level and vanished when the hook returned.
//!
//! So shell code on the shell thread waits for a batch that is already
//! running ([`quiesce`]), and withdraws one the pool has not started yet.
//! Plain typing is not held up: builtin widgets never open a scope. The
//! first scope usually opens after the prompt is on screen; a
//! `zle-line-init` widget is the exception, because this port's `zleread`
//! runs it before its first `zrefresh` (C paints first, c:1353, then calls
//! the hook, c:1357), so a running batch delays that paint by the hook's
//! runtime.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// The session's shared worker pool, published by `ShellExecutor::new()` at
/// startup. `preprompt()` runs BETWEEN commands where the thread_local
/// `CURRENT_EXECUTOR` is not set, so `try_with_executor` returns `None` there —
/// this global handle reaches the pool without an executor context.
static SESSION_POOL: OnceLock<Arc<crate::worker::WorkerPool>> = OnceLock::new();

/// Publish the session worker pool. Called once from `ShellExecutor::new()`.
pub fn set_session_pool(pool: Arc<crate::worker::WorkerPool>) {
    let _ = SESSION_POOL.set(pool);
}

/// Where the current batch is. Debounce: while it is not [`IDLE`], the next
/// prompt skips its round rather than pile up overlapping runs of the same
/// hooks.
static BATCH: AtomicU8 = AtomicU8::new(IDLE);
/// No batch in flight.
const IDLE: u8 = 0;
/// Submitted to the pool, not started. [`quiesce`] may withdraw it.
const QUEUED: u8 = 1;
/// A worker is running the hooks. [`quiesce`] waits for it.
const RUNNING: u8 = 2;

/// Paired with [`BATCH_DONE`]: the worker flips `BATCH` back to [`IDLE`]
/// while holding it, so a waiter that checked `BATCH` under the same lock
/// cannot miss the wake-up.
static BATCH_LOCK: Mutex<()> = Mutex::new(());
static BATCH_DONE: Condvar = Condvar::new();

/// Returns the batch to [`IDLE`] and wakes [`quiesce`] however the worker
/// leaves the hooks — including by unwinding out of a panicking one, which
/// would otherwise leave the shell thread waiting forever.
struct BatchFinished;

impl Drop for BatchFinished {
    fn drop(&mut self) {
        let _guard = BATCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        BATCH.store(IDLE, Ordering::Release);
        BATCH_DONE.notify_all();
    }
}

/// Make the shell thread safe to run shell code: wait for a batch a worker
/// is running, or withdraw one that is only queued (its hooks run on the
/// next prompt instead). Returns at once when no batch is in flight, and on
/// any thread but the shell's — a worker must never wait on its own batch.
///
/// Called where the shell thread opens a parameter scope
/// (`utils::inc_locallevel`, every function, widget, hook and trap call)
/// and before it executes an accepted line at top level (`init::loop_`).
/// After it returns no batch can start until the next `preprompt()`, since
/// that is the only place one is fired.
pub fn quiesce() {
    if BATCH.load(Ordering::Acquire) == IDLE {
        return;
    }
    // Same shell-thread test as `errflag_cell`: the shell runs on "main",
    // pool workers are named "zshrs-worker-N".
    if std::thread::current().name() != Some("main") {
        return;
    }
    if BATCH
        .compare_exchange(QUEUED, IDLE, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        tracing::debug!("async_precmd: batch withdrawn before it started");
        return;
    }
    let mut guard = BATCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    while BATCH.load(Ordering::Acquire) == RUNNING {
        guard = BATCH_DONE.wait(guard).unwrap_or_else(|e| e.into_inner());
    }
}

/// Collect the registered `async_precmd` hook function names: the function
/// literally named `async_precmd` (if defined) followed by every member of the
/// `async_precmd_functions` array, in order.
///
/// The hook functions must live in the shared global `shfunctab` for a worker
/// to run them — which is the case for functions defined by SOURCED config
/// (.zshrc / plugins), the normal way hooks are registered. (A function TYPED
/// at the interactive prompt currently takes a compile path that registers it
/// only in the per-executor table, so a worker can't see it — not the intended
/// registration route for a hook.)
fn collect_hook_functions() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if crate::ported::hashtable::shfunctab_lock()
        .read()
        .map(|t| t.get("async_precmd").is_some())
        .unwrap_or(false)
    {
        names.push("async_precmd".to_string());
    }
    if let Ok(t) = crate::ported::params::paramtab().read() {
        if let Some(p) = t.get("async_precmd_functions") {
            if let Some(arr) = p.u_arr.clone() {
                names.extend(arr);
            }
        }
    }
    names
}

/// Fire the `async_precmd` hooks on a worker thread. Called from `preprompt()`
/// AFTER precmd + prompt render, so the prompt is already on screen. Returns
/// immediately (non-blocking): it submits ONE closure to the shared worker pool
/// and lets it run in the background. Debounced via [`BATCH`].
pub fn fire_async_precmd() {
    let names = collect_hook_functions();
    if names.is_empty() {
        return;
    }
    // Debounce: only one batch in flight at a time.
    if BATCH
        .compare_exchange(IDLE, QUEUED, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    tracing::debug!(?names, "async_precmd: dispatching hooks to worker pool");
    // Reach the shared worker pool via the global session handle — the
    // executor context is not entered during preprompt.
    let Some(pool) = SESSION_POOL.get().map(Arc::clone) else {
        tracing::warn!("async_precmd: session pool not published yet — skipping");
        BATCH.store(IDLE, Ordering::Release);
        return;
    };
    let pool_for_worker = std::sync::Arc::clone(&pool);
    pool.submit(move || {
        // The shell thread withdrew the batch (see `quiesce`) before a
        // worker got to it: it is already running shell code.
        if BATCH
            .compare_exchange(QUEUED, RUNNING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let _finished = BatchFinished;
        // Lightweight worker executor — shares the global param/function tables.
        let mut wex = crate::vm_helper::ShellExecutor::new_worker(pool_for_worker);
        for name in &names {
            // A hook whose function is gone is SKIPPED, never executed as a
            // command word. c:Src/utils.c:1514-1518 -- `callhookfunc` looks
            // each `${hook}_functions` member up in `shfunctab` and only
            // calls it `if (shfunc)`; a stale name is silently ignored. The
            // ported loop in `ported::utils::callhookfunc` does the same.
            //
            // Running the name through the script pipeline instead made this
            // path diverge: with no function to find, the word fell through
            // to `execute_external` and printed
            //     zshrs: command not found: :hist:precmd
            // once per prompt, forever. Self-removing hooks reach that state
            // by design -- zsh-hist registers `:hist:precmd`, and on its
            // first run the body does `add-zsh-hook -d precmd $0` followed by
            // `unfunction $0`. The delete names the `precmd` hook, so a copy
            // registered on `async_precmd` keeps the now-dangling name.
            //
            // Looking the function up also skips a re-parse per hook per
            // prompt, and stops the name being subjected to alias expansion
            // and globbing on its way to being run.
            if !wex.function_exists(name) {
                tracing::debug!(hook = %name, "async_precmd: no such function — skipping");
                continue;
            }
            // Invoking the function by name runs its body on this worker; any
            // `typeset -g` lands in the shared global param table.
            let _ = wex.execute_script_zsh_pipeline(name);
        }
    });
}
