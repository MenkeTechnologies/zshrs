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

/// Run the signals `zhandler` deferred while the calling thread waited for a
/// key, then keep waiting in short slices until the queue is empty or `fd`
/// has input.
///
/// C never defers inside ZLE's blocking wait: the queueing baseline is dropped
/// across the read (`dont_queue_signals`, Src/Zle/zle_main.c:870), so its
/// handler always dispatches at once. `zhandler` here also defers when another
/// thread holds the parameter table or `JOBTAB`, and the queue drains only at
/// safe points. An idle shell reaches none of them, so a `SIGHUP` deferred
/// that way sat queued until the next keystroke and `kill -HUP $$` left the
/// shell alive. The `EINTR` the deferring handler leaves behind is the safe
/// point: this thread holds nothing there.
pub fn settle_deferred_signals(fd: i32) {
    use crate::ported::signals::{queue_front, queue_rear, queueing_enabled};
    use std::sync::atomic::Ordering::SeqCst;
    let pending = || queue_front.load(SeqCst) != queue_rear.load(SeqCst);
    while pending() && queueing_enabled.load(SeqCst) == 0 {
        crate::ported::signals_h::run_queued_signals();
        if !pending() {
            break;
        }
        // The other thread still holds the table; look again shortly, but
        // never sit on a key the user has already typed.
        let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut p, 1, 10) } > 0 {
            break;
        }
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
    /// The queueing branch of `zhandler` returns without the dispatch path's
    /// `signal_setmask`; the thread's mask must be what it was on entry, not the
    /// all-signals mask `zhandler` applies first. A mask left full is inherited by
    /// every external the shell then starts.
    #[test]
    fn a_queued_signal_leaves_the_threads_signal_mask_as_it_found_it() {
        use crate::ported::signals::zhandler;
        use crate::ported::signals_h::{queue_signals, unqueue_signals};

        let _g = crate::test_util::global_state_lock();
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            let mask = || unsafe {
                let mut m: libc::sigset_t = std::mem::zeroed();
                libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), &mut m);
                (1..32).filter(|&s| libc::sigismember(&m, s) == 1).collect::<Vec<_>>()
            };
            let before = mask();

            queue_signals();
            zhandler(libc::SIGCHLD);
            let after = mask();
            unqueue_signals();

            assert_eq!(after, before, "zhandler left its all-signals mask applied after queueing");
            let _ = done.send(());
        });
        assert!(finished.recv_timeout(Duration::from_secs(30)).is_ok(), "mask test hung");
    }
    /// A signal deferred because another thread holds the parameter table is
    /// run by `settle_deferred_signals` rather than left queued until the next
    /// keystroke (the idle-shell `kill -HUP $$` that did nothing).
    #[test]
    fn settle_runs_a_signal_deferred_behind_another_threads_table_lock() {
        use crate::ported::signals::{queue_front, queue_rear, zhandler};
        use std::time::Instant;

        let _g = crate::test_util::global_state_lock();
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            let pending = || queue_front.load(Ordering::SeqCst) != queue_rear.load(Ordering::SeqCst);
            assert!(!pending(), "the queue starts empty");

            let mut pipe = [0i32; 2];
            assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);

            let (locked, locked_rx) = mpsc::channel();
            let holder = std::thread::spawn(move || {
                let table = crate::ported::params::paramtab().write().expect("paramtab");
                locked.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(100));
                drop(table);
            });
            locked_rx.recv().unwrap();

            zhandler(libc::SIGCHLD);
            assert!(pending(), "the signal is deferred while another thread holds the table");

            let start = Instant::now();
            super::settle_deferred_signals(pipe[0]);
            assert!(!pending(), "the deferred signal ran once the table was free");
            assert!(start.elapsed() >= Duration::from_millis(50), "it waited for the holder");

            holder.join().unwrap();
            unsafe {
                libc::close(pipe[0]);
                libc::close(pipe[1]);
            }
            let _ = done.send(());
        });
        assert!(finished.recv_timeout(Duration::from_secs(30)).is_ok(), "settle_deferred_signals hung");
    }
    /// Replaying the queue while queueing is still on must return at once and
    /// leave the signal queued: `zhandler` re-queues it, so a drain that kept
    /// going spun for ever (a `SIGHUP` held behind a raised level never ran).
    #[test]
    fn run_queued_signals_does_not_spin_while_queueing_is_enabled() {
        use crate::ported::signals::{queue_front, queue_rear, zhandler};
        use crate::ported::signals_h::{queue_signals, run_queued_signals, unqueue_signals};

        let _g = crate::test_util::global_state_lock();
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            let pending = || queue_front.load(Ordering::SeqCst) != queue_rear.load(Ordering::SeqCst);
            assert!(!pending(), "the queue starts empty");

            queue_signals();
            zhandler(libc::SIGCHLD);
            assert!(pending(), "the signal is queued while queueing is enabled");

            run_queued_signals(); // before the fix: never returns
            assert!(pending(), "it stays queued until the level drops");

            unqueue_signals();
            assert!(!pending(), "dropping the level to zero dispatches it");
            let _ = done.send(());
        });
        assert!(
            finished.recv_timeout(Duration::from_secs(30)).is_ok(),
            "run_queued_signals spun while queueing_enabled was non-zero"
        );
    }
}
