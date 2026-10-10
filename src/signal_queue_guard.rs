//! Rust-only utility (NOT a port — lives outside `src/ported/` by design).
//!
//! A scope guard for C's `queue_signals()` / `unqueue_signals()` pair
//! (`Src/signals.h:90-95`).
//!
//! C writes `queue_signals();` at the top of a routine and `unqueue_signals();`
//! before every `return`, which is how `assignaparam()` and friends keep an
//! asynchronous `SIGCHLD` from running job bookkeeping (which reads the
//! parameter table) in the middle of a parameter write. The Rust ports of those
//! routines have dozens of exits each; pairing every one by hand is how an exit
//! gets missed. Holding this guard for the rest of the function unqueues on every
//! path, early `return` and unwinding included, and the handler's deferral
//! (`zhandler`'s `queueing_enabled` check) does the rest.
//!
//! Why it matters: the handler runs on the SAME thread it interrupted. If that
//! thread holds the parameter-table lock and the handler wants it, the thread
//! waits on itself forever (`repeat 200 { (exit 1) & }; wait` hung that way, in
//! `assignaparam` → `SIGCHLD` → `update_job` → `getsparam`).

use crate::ported::signals_h::{queue_signals, unqueue_signals};

/// Signals are queued from [`QueuedSignals::enter`] until the guard drops.
#[must_use = "signals are unqueued when the guard is dropped; bind it to a variable"]
pub struct QueuedSignals(());

impl QueuedSignals {
    /// `queue_signals();`
    pub fn enter() -> Self {
        queue_signals();
        QueuedSignals(())
    }
}

impl Drop for QueuedSignals {
    /// `unqueue_signals();`
    fn drop(&mut self) {
        unqueue_signals();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn the_guard_pairs_queue_and_unqueue_on_every_exit() {
        let _g = crate::test_util::global_state_lock();
        let level = || crate::ported::signals::queueing_enabled.load(Ordering::SeqCst);
        let before = level();
        {
            let _q = QueuedSignals::enter();
            assert_eq!(level(), before + 1);
            let _nested = QueuedSignals::enter();
            assert_eq!(level(), before + 2);
        }
        assert_eq!(level(), before);

        fn early_return(flag: bool) -> bool {
            let _q = QueuedSignals::enter();
            if flag {
                return true;
            }
            false
        }
        assert!(early_return(true));
        assert!(!early_return(false));
        assert_eq!(level(), before);
    }
}

#[cfg(test)]
mod deferral_tests {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::time::Duration;

    /// The handler runs on the thread it interrupted. With the parameter table's write
    /// lock held on that thread, dispatching `SIGCHLD` would read the table (`getsparam`)
    /// and wait on itself for ever -- `repeat 200 { (exit 1) & }; wait` hung exactly so.
    /// `zhandler` must defer instead, and the queue must drain once the lock is gone.
    #[test]
    fn a_signal_arriving_while_the_parameter_table_is_locked_is_deferred_not_deadlocked() {
        let _g = crate::test_util::global_state_lock();
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            use crate::ported::signals::{queue_front, queue_rear, zhandler};
            let pending = || queue_front.load(Ordering::SeqCst) != queue_rear.load(Ordering::SeqCst);
            assert!(!pending(), "the queue starts empty");

            let held = crate::ported::params::paramtab().write().expect("paramtab");
            zhandler(libc::SIGCHLD); // before the fix: blocks here for ever
            assert!(pending(), "the signal is queued, not dispatched, while the table is held");
            drop(held);

            crate::ported::signals_h::run_queued_signals();
            assert!(!pending(), "the deferred signal is dispatched once the table is free");
            let _ = done.send(());
        });
        assert!(
            finished.recv_timeout(Duration::from_secs(30)).is_ok(),
            "zhandler deadlocked on the parameter table its own thread holds"
        );
    }
}
