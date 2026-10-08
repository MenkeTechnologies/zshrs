//! !!! WARNING: RUST-ONLY !!! Helper threads that never run the shell's
//! signal handlers.
//!
//! C zsh is single-threaded, so `child_block()` (c:Src/signals.h:52) around a
//! fork is enough to keep the SIGCHLD handler out of the fork window. zshrs
//! runs helper threads alongside the shell (the worker pool, multio pumps, the
//! `-c` long-command watchdog, p10k's async segments), and a process-directed
//! signal is delivered to ANY thread that has it unblocked. A SIGCHLD that
//! lands on a helper thread runs `zhandler` → `wait_for_processes` there,
//! which locks `JOBTAB` (and malloc's arena lock) while the main thread forks
//! with SIGCHLD blocked; the child inherits both locks held and deadlocks in
//! `entersubsh` → `clearjobtab`, and the parent waits on it forever. Measured
//! on Linux: `zshrs -f -c 'repeat 200 { (exit 1) & }; wait; print ok'` hung
//! with the child parked in `futex_wait` on `JOBTAB` and the SIGCHLD handler
//! running on the watchdog thread.
//!
//! A new thread inherits its creator's signal mask (pthread_create(3)), so
//! the mask is set to "everything blocked" around the spawn and restored
//! afterwards: the thread starts with no signal deliverable to it, and only
//! the main thread runs the shell's handlers, as in C. A signal that arrives
//! during the spawn stays pending and is delivered when the caller's mask is
//! restored.

use std::io;
use std::thread::{Builder, JoinHandle};

/// `std::thread::spawn` for a thread that must not receive signals.
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    spawn_with(Builder::new(), f).expect("failed to spawn thread")
}

/// `Builder::spawn` for a thread that must not receive signals.
pub fn spawn_with<F, T>(builder: Builder, f: F) -> io::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    unsafe {
        let mut all: libc::sigset_t = std::mem::zeroed();
        let mut old: libc::sigset_t = std::mem::zeroed();
        libc::sigfillset(&mut all);
        libc::pthread_sigmask(libc::SIG_SETMASK, &all, &mut old);
        let handle = builder.spawn(f);
        libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
        handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spawned thread starts with every catchable signal blocked, and the
    /// caller's own mask is unchanged afterwards.
    #[test]
    fn spawned_thread_blocks_every_signal_and_caller_mask_is_restored() {
        fn current_mask() -> libc::sigset_t {
            unsafe {
                let mut m: libc::sigset_t = std::mem::zeroed();
                libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), &mut m);
                m
            }
        }
        let is_member = |m: &libc::sigset_t, s| unsafe { libc::sigismember(m, s) } == 1;
        let before = current_mask();
        let child = spawn(current_mask).join().unwrap();
        for sig in [libc::SIGCHLD, libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGPIPE] {
            assert!(is_member(&child, sig), "signal {sig} deliverable to a helper thread");
        }
        let after = current_mask();
        for sig in 1..32 {
            assert_eq!(is_member(&before, sig), is_member(&after, sig), "caller mask changed for {sig}");
        }
    }
}
