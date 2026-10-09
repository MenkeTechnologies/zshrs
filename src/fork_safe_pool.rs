//! Rust-only utility (NOT a port — lives outside `src/ported/` by design).
//!
//! A rayon pool that survives `fork()`.
//!
//! zsh forks for `$(...)`, pipeline stages and background jobs. `fork()` copies
//! only the calling thread, so a rayon pool created by the parent has worker
//! handles in the child but no threads behind them: the first `par_iter` in
//! the child queues work nobody will ever take and blocks forever. A script
//! that globbed `**/*` in the shell and then again inside `$(...)` hung for
//! exactly that reason.
//!
//! [`pool`] hands out a pool owned by the *current process*: the first call in
//! a process, parent or forked child, builds one, and a child never touches the
//! pool its parent built. Entries are leaked on purpose — the parent's pool is
//! unusable in the child and must not be dropped there (dropping joins threads
//! that do not exist).
//!
//! The slot is a lock-free pointer rather than a `Mutex`: a mutex held by some
//! other thread at the moment of `fork()` stays locked forever in the child.

use std::sync::atomic::{AtomicPtr, Ordering};

struct Entry {
    pid: u32,
    pool: rayon::ThreadPool,
}

static CURRENT: AtomicPtr<Entry> = AtomicPtr::new(std::ptr::null_mut());

/// The calling process's own rayon pool, or `None` when one cannot be built
/// (the caller then runs serially).
pub fn pool() -> Option<&'static rayon::ThreadPool> {
    let pid = std::process::id();
    loop {
        let seen = CURRENT.load(Ordering::Acquire);
        // SAFETY: a non-null pointer was produced by `Box::into_raw` below and
        // is never freed, so it is valid for `'static`.
        if let Some(entry) = unsafe { seen.as_ref() } {
            if entry.pid == pid {
                return Some(&entry.pool);
            }
        }
        let pool = rayon::ThreadPoolBuilder::new().build().ok()?;
        let fresh = Box::into_raw(Box::new(Entry { pid, pool }));
        match CURRENT.compare_exchange(seen, fresh, Ordering::AcqRel, Ordering::Acquire) {
            // SAFETY: `fresh` was just leaked and is never freed.
            Ok(_) => return Some(unsafe { &(*fresh).pool }),
            // Another thread installed an entry first; use theirs on the next turn.
            // SAFETY: `fresh` was never published, so this thread still owns it.
            Err(_) => drop(unsafe { Box::from_raw(fresh) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rayon::prelude::*;

    #[test]
    fn same_process_reuses_one_pool() {
        let a = pool().expect("pool") as *const _;
        let b = pool().expect("pool") as *const _;
        assert_eq!(a, b);
    }

    /// The regression: a forked child using the parent's pool never returns.
    #[test]
    fn a_forked_child_runs_parallel_work_on_its_own_pool() {
        let parent = pool().expect("pool");
        assert_eq!(parent.install(|| (0..64).into_par_iter().sum::<i32>()), 2016);
        // SAFETY: the child only runs the closure and `_exit`s.
        match unsafe { libc::fork() } {
            -1 => panic!("fork: {}", std::io::Error::last_os_error()),
            0 => {
                let sum = pool().map(|p| p.install(|| (0..64).into_par_iter().sum::<i32>()));
                unsafe { libc::_exit(if sum == Some(2016) { 0 } else { 1 }) }
            }
            child => {
                let mut status = 0;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
                loop {
                    let done = unsafe { libc::waitpid(child, &mut status, libc::WNOHANG) };
                    if done == child {
                        break;
                    }
                    if std::time::Instant::now() > deadline {
                        unsafe { libc::kill(child, libc::SIGKILL) };
                        unsafe { libc::waitpid(child, &mut status, 0) };
                        panic!("forked child hung running parallel work");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0);
            }
        }
    }
}
