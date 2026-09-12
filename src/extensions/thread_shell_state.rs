//! Per-thread storage for the shell state C keeps per THREAD OF EXECUTION.
//!
//! **zshrs-original infrastructure — no `Src/*.c` counterpart.** It is the
//! generalisation of [`crate::errflag_cell`], and it exists for the same
//! reason: zshrs replaced a `fork(2)` with a thread.
//!
//! C zsh runs on one thread. `scriptname` (`Src/init.c`, the diagnostic
//! prefix `zwarning` reads at `Src/utils.c:147`), `funcstack`
//! (`Src/exec.c:340`, behind `$funcstack` / `$functrace` /
//! `$funcfiletrace` / `$funcsourcetrace`), `zsh_eval_context`
//! (`zsh.export:355`, behind `$ZSH_EVAL_CONTEXT`) and `locallevel`
//! (`Src/params.c:54`) are all plain globals that describe WHERE THE ONE
//! THREAD CURRENTLY IS. Background work in C is a forked child, so the
//! child's pushes onto those globals are invisible to the parent.
//!
//! zshrs runs some of that work on the worker pool ([`crate::worker`])
//! instead — `async_precmd` (`crate::async_precmd`) hands hook FUNCTIONS to
//! a worker, and `compsys::in_editor` drives the completer tree on a
//! dedicated thread. A hook function moves every one of those globals while
//! it runs, and the shell thread reads the moved value:
//!
//! ```text
//! async_precmd_functions=(spin_hook)   # spin_hook is running on a worker
//! % done                               # typed at the prompt
//! spin_hook:4: parse error near `done'     <- zshrs, before this module
//! zsh: parse error near `done'             <- zsh
//! ```
//!
//! The user is pointed at a function that has nothing to do with what they
//! typed, which is worse than no information at all. `quiesce`
//! (`crate::async_precmd::quiesce`) closes the window where the shell
//! thread EXECUTES shell code during a batch, but the line above is
//! diagnosed by the LEXER, which runs before the `quiesce()` in
//! `init::loop_` — and `quiesce` is a serialisation, not a fix for the
//! sharing.
//!
//! Restoring the C model is the fix: the shell thread keeps the
//! process-wide value, every other thread gets its own. A thread that runs
//! shell code off the shell thread IS a forked child in the C design, so
//! where it is stays with it.
//!
//! Two shapes live here:
//!
//! * [`ThreadMutex`] for the stack-shaped state (`scriptname`,
//!   `scriptfilename`, `funcstack`, `zsh_eval_context`). It mirrors
//!   `Mutex`'s `lock()` / `clear_poison()` so the ~50 existing
//!   `FUNCSTACK.lock()` call sites read exactly as they did before.
//! * [`LocalLevelCell`] for `locallevel`, which cannot simply become
//!   per-thread — see its own docs.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{LockResult, Mutex, MutexGuard};
use std::thread::ThreadId;

thread_local! {
    /// Whether this thread is the shell thread, computed once per thread.
    /// Same test as [`crate::errflag_cell`] and
    /// `crate::async_precmd::quiesce`: the shell runs on the process's
    /// initial thread, which Rust names `"main"`; pool workers are spawned
    /// without a name or as `"zshrs-worker-N"`.
    static IS_SHELL: bool = std::thread::current().name() == Some("main");

    /// This thread's own `locallevel` — see [`LocalLevelCell`].
    static OWN_LOCALLEVEL: Cell<i32> = const { Cell::new(0) };
}

/// True on the shell thread, false on every worker.
///
/// !!! WARNING: RUST-ONLY !!! C has one thread and needs no such test.
///
/// Sharp edge, same as `errflag_cell`'s: a thread deliberately named
/// `"main"` by `thread::Builder` would be misclassified as the shell.
/// Nothing in this tree does that.
#[inline]
pub fn is_shell_thread() -> bool {
    IS_SHELL.with(|s| *s)
}

/// A `Mutex` whose contents are the shell thread's on the shell thread and
/// this thread's own everywhere else.
///
/// !!! WARNING: RUST-ONLY !!! No counterpart in `Src/*.c` — C gets this
/// split from `fork(2)`.
///
/// The API is deliberately `Mutex`'s, down to the `LockResult`, so a
/// global can be re-typed from `Mutex<T>` to `ThreadMutex<T>` without
/// touching its readers. The per-thread halves are leaked `Mutex<T>`s so
/// both branches can hand back the same `MutexGuard<'static, T>` type; the
/// leak is bounded by (threads that ever ran shell code) × (cells), which
/// is the small fixed worker pool.
///
/// A worker's half starts EMPTY rather than as a copy of the shell
/// thread's. That is the `fork()` a C zsh would do from `preprompt()`,
/// where the shell is at top level with nothing on any of these stacks;
/// it is not a copy of an arbitrary mid-function state, because no site
/// that runs shell code off the shell thread is reached from inside one.
pub struct ThreadMutex<T: 'static> {
    /// The shell thread's value — the process-wide one, as before.
    shell: Mutex<T>,
    /// Builds a fresh value for a thread's first use of this cell.
    seed: fn() -> T,
    /// One leaked `Mutex<T>` per non-shell thread that has touched this
    /// cell. `Option` so the map can be built in a `const fn`.
    others: Mutex<Option<HashMap<ThreadId, &'static Mutex<T>>>>,
}

impl<T: 'static> ThreadMutex<T> {
    /// `shell` is the shell thread's initial value; `seed` builds a
    /// non-shell thread's. `const` so the consumer stays a plain `static`.
    pub const fn new(shell: T, seed: fn() -> T) -> Self {
        ThreadMutex {
            shell: Mutex::new(shell),
            seed,
            others: Mutex::new(None),
        }
    }

    /// Mirrors `Mutex::lock`.
    #[inline]
    pub fn lock(&'static self) -> LockResult<MutexGuard<'static, T>> {
        if is_shell_thread() {
            self.shell.lock()
        } else {
            self.own().lock()
        }
    }

    /// Mirrors `Mutex::try_lock`.
    #[inline]
    pub fn try_lock(&'static self) -> std::sync::TryLockResult<MutexGuard<'static, T>> {
        if is_shell_thread() {
            self.shell.try_lock()
        } else {
            self.own().try_lock()
        }
    }

    /// Mirrors `Mutex::clear_poison`.
    pub fn clear_poison(&'static self) {
        if is_shell_thread() {
            self.shell.clear_poison();
        } else {
            self.own().clear_poison();
        }
    }

    /// This thread's half, created on first use.
    fn own(&'static self) -> &'static Mutex<T> {
        let id = std::thread::current().id();
        let mut guard = self.others.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        if let Some(slot) = map.get(&id) {
            return slot;
        }
        let slot: &'static Mutex<T> = Box::leak(Box::new(Mutex::new((self.seed)())));
        map.insert(id, slot);
        slot
    }
}

/// `locallevel` storage: shared for parameter scoping, per-thread for
/// diagnostics.
///
/// !!! WARNING: RUST-ONLY !!! C has one `int locallevel`
/// (`Src/params.c:54`) serving both jobs, because with one thread they are
/// the same number.
///
/// zshrs cannot simply make it per-thread. `locallevel` is the level a
/// parameter is STAMPED with (`Src/params.c:5837` `locallevel++` in
/// `startparamscope`) and the level `scanendscope` compares every entry
/// against on scope exit (`Src/params.c:5904-5907`, `if (pm->level >
/// locallevel)`) — and the param table those stamps live in is one shared
/// table that `async_precmd` hooks deliberately write into. Give each
/// thread its own counter and two threads both stamp level 1 in the same
/// table, so one thread's `endparamscope` deletes the other's locals: a
/// strictly worse version of the `$BUFFER` loss `quiesce` was added for.
/// So [`load`](Self::load) — every parameter-scoping reader — keeps
/// returning the shared value, unchanged.
///
/// What a worker must NOT contaminate is the other job: the
/// "am I inside a function" test the error formatters make
/// (`Src/utils.c:150` `if (unset(SHINSTDIN) || locallevel)`, `:301` in
/// `zerrmsg`). A hook running on a worker raised the shared counter, and
/// an error the user caused at top level came out formatted as if it had
/// happened inside a function — with a line number and a function-name
/// prefix zsh does not print. [`here`](Self::here) answers that question
/// for the CALLING thread, and the formatters read it.
///
/// Every mutation goes through this type, so the per-thread depth cannot
/// drift away from the sites that move the shared counter: the four
/// methods below are the entire mutation surface of `params::locallevel`.
pub struct LocalLevelCell {
    /// The stamp shared with the one param table. Read by every scoping
    /// site exactly as the `AtomicI32` it replaced was.
    shared: AtomicI32,
}

impl LocalLevelCell {
    /// A top-level counter. `const` so `locallevel` stays a plain `static`.
    pub const fn new() -> Self {
        LocalLevelCell {
            shared: AtomicI32::new(0),
        }
    }

    /// Mirrors `AtomicI32::load` — the SHARED counter, for param scoping.
    #[inline]
    pub fn load(&self, order: Ordering) -> i32 {
        self.shared.load(order)
    }

    /// Mirrors `AtomicI32::store`. Sites that save and restore the whole
    /// counter (`computil`'s `comparguments` bracket, `param_private`'s
    /// hoist) mean "this thread is now at level v", so the thread's own
    /// depth follows.
    #[inline]
    pub fn store(&self, val: i32, order: Ordering) {
        OWN_LOCALLEVEL.with(|d| d.set(val));
        self.shared.store(val, order);
    }

    /// Mirrors `AtomicI32::fetch_add`, returning the previous SHARED value.
    #[inline]
    pub fn fetch_add(&self, val: i32, order: Ordering) -> i32 {
        OWN_LOCALLEVEL.with(|d| d.set(d.get() + val));
        self.shared.fetch_add(val, order)
    }

    /// Mirrors `AtomicI32::fetch_sub`, returning the previous SHARED value.
    #[inline]
    pub fn fetch_sub(&self, val: i32, order: Ordering) -> i32 {
        OWN_LOCALLEVEL.with(|d| d.set(d.get() - val));
        self.shared.fetch_sub(val, order)
    }

    /// The calling thread's OWN function-scope depth — what C's
    /// `locallevel` would read if this thread were the only one, which is
    /// what the error formatters need. Clamped at zero: a thread whose
    /// first act is `endparamscope` (a scope opened before it existed)
    /// would otherwise report a negative depth.
    #[inline]
    pub fn here(&self) -> i32 {
        OWN_LOCALLEVEL.with(|d| d.get()).max(0)
    }
}

impl Default for LocalLevelCell {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression this module exists for, in miniature: a worker
    /// inside a hook function must not be able to move the shell thread's
    /// view of where the shell is.
    #[test]
    fn a_workers_frames_do_not_reach_the_shell_threads_view() {
        static STACK: ThreadMutex<Vec<String>> = ThreadMutex::new(Vec::new(), Vec::new);

        STACK.lock().unwrap().push("shell-frame".to_string());

        std::thread::spawn(|| {
            assert!(
                STACK.lock().unwrap().is_empty(),
                "a worker starts with its own stack, as a forked child would"
            );
            STACK.lock().unwrap().push("hook".to_string());
        })
        .join()
        .unwrap();

        let seen = STACK.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec!["shell-frame".to_string()],
            "the worker's frame leaked into the shell thread's stack"
        );
    }

    /// `load()` must stay shared — the param table's level stamps depend
    /// on it — while `here()` reports only the calling thread's depth.
    #[test]
    fn locallevel_shares_the_stamp_and_splits_the_depth() {
        static LL: LocalLevelCell = LocalLevelCell::new();
        LL.store(0, Ordering::Relaxed);

        std::thread::spawn(|| {
            LL.fetch_add(1, Ordering::Relaxed); // the hook's function scope
            assert_eq!(LL.here(), 1, "the worker is one scope deep");
        })
        .join()
        .unwrap();

        assert_eq!(
            LL.load(Ordering::Relaxed),
            1,
            "the shared stamp must still carry the worker's scope"
        );
        assert_eq!(
            LL.here(),
            0,
            "this thread is at top level and its diagnostics must say so"
        );
    }
}
