//! Marks the stretch of a thread that is inside the allocator, so a signal
//! handler that lands there can defer instead of allocating.
//!
//! !!! WARNING: RUST-ONLY HELPER — NO C COUNTERPART !!!
//!
//! C brackets the allocator with the signal queue: `zalloc` and `zrealloc`
//! call `queue_signals()` before `malloc`/`realloc` and `unqueue_signals()`
//! after (c:Src/mem.c:965-970, 996-1004), so `zhandler` never runs while
//! the interrupted code holds the allocator's lock. Rust allocates through
//! the global allocator, which knows nothing of that queue. `zhandler`'s
//! SIGCHLD arm allocates (`wait_for_processes` builds a `Vec`), so a SIGCHLD
//! that interrupted `malloc` re-entered it and the thread waited on its own
//! lock for ever: glibc `__lll_lock_wait_private` under `malloc`, under
//! `zhandler`, under `malloc` (a pipeline whose `$(…)` allocates while the
//! forked first stage exits, ~1 run in 2000 on Linux).
//!
//! [`SignalGuardedAlloc`] counts, per thread, the calls in flight, and
//! [`alloc_busy`] reads that count from the handler, which runs on the
//! thread it interrupted. The handler then queues the signal, the way it
//! already does for a busy `paramtab` / `JOBTAB`. The count is a plain
//! `Cell` in a const-initialised thread local without a destructor, so
//! touching it neither allocates nor registers anything.
//!
//! Installed as `#[global_allocator]` by the `zshrs` binary, not by this
//! library, so a crate that links the library keeps its own allocator;
//! without it `alloc_busy` is always false and nothing changes.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static IN_ALLOC: Cell<u32> = const { Cell::new(0) };
}

/// True while this thread is inside the global allocator.
pub fn alloc_busy() -> bool {
    IN_ALLOC.try_with(|n| n.get() != 0).unwrap_or(false)
}

/// Runs `f` with this thread marked as inside the allocator.
#[inline]
fn guarded<T>(f: impl FnOnce() -> T) -> T {
    let _ = IN_ALLOC.try_with(|n| n.set(n.get() + 1));
    let out = f();
    let _ = IN_ALLOC.try_with(|n| n.set(n.get() - 1));
    out
}

/// The system allocator, with every call marked for [`alloc_busy`].
pub struct SignalGuardedAlloc;

unsafe impl GlobalAlloc for SignalGuardedAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        guarded(|| unsafe { System.alloc(layout) })
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        guarded(|| unsafe { System.alloc_zeroed(layout) })
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        guarded(|| unsafe { System.dealloc(ptr, layout) })
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        guarded(|| unsafe { System.realloc(ptr, layout, new_size) })
    }
}
