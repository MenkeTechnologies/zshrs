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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// The session's shared worker pool, published by `ShellExecutor::new()` at
/// startup. `preprompt()` runs BETWEEN commands where the thread_local
/// `CURRENT_EXECUTOR` is not set, so `try_with_executor` returns `None` there —
/// this global handle reaches the pool without an executor context.
static SESSION_POOL: OnceLock<Arc<crate::worker::WorkerPool>> = OnceLock::new();

/// Publish the session worker pool. Called once from `ShellExecutor::new()`.
pub fn set_session_pool(pool: Arc<crate::worker::WorkerPool>) {
    let _ = SESSION_POOL.set(pool);
}

/// True while an async_precmd batch is in flight. Debounce: if the previous
/// batch hasn't finished by the next prompt, skip this round rather than pile
/// up overlapping runs of the same hooks.
static RUNNING: AtomicBool = AtomicBool::new(false);

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
/// and lets it run in the background. Debounced via [`RUNNING`].
pub fn fire_async_precmd() {
    let names = collect_hook_functions();
    if names.is_empty() {
        return;
    }
    // Debounce: only one batch in flight at a time.
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    tracing::debug!(?names, "async_precmd: dispatching hooks to worker pool");
    // Reach the shared worker pool via the global session handle — the
    // executor context is not entered during preprompt.
    let Some(pool) = SESSION_POOL.get().map(Arc::clone) else {
        tracing::warn!("async_precmd: session pool not published yet — skipping");
        RUNNING.store(false, Ordering::Release);
        return;
    };
    let pool_for_worker = std::sync::Arc::clone(&pool);
    pool.submit(move || {
        // Lightweight worker executor — shares the global param/function tables.
        let mut wex = crate::vm_helper::ShellExecutor::new_worker(pool_for_worker);
        for name in &names {
            // Invoking the function by name runs its body on this worker; any
            // `typeset -g` lands in the shared global param table.
            let _ = wex.execute_script_zsh_pipeline(name);
        }
        RUNNING.store(false, Ordering::Release);
    });
}
