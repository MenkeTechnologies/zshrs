//! Keep the shell's own file descriptors out of the user's fd space.
//!
//! zsh routes every internal open through `movefd()` (Src/utils.c:1975), which
//! relocates the descriptor to >= 10 with `fcntl(fd, F_DUPFD, 10)` and records
//! it as `FDT_INTERNAL`. The point is that fds 0-9 belong to the SCRIPT: `exec
//! 3>out`, `read -u 4`, `print -u 3` and friends address them by number, and a
//! shell that parks its own state there is squatting on the user's namespace.
//!
//! zshrs was squatting. A `-c` run held:
//!
//!     fd 3 -> ~/.zshrs/zshrs.log
//!     fd 4 -> ~/.zshrs/zshrs_history.db          (sqlite)
//!     fd 5,6,7 -> ~/.zshrs/compsys.db + -wal + -shm
//!
//! so `print -u 3 -r -- x` appended `x` to the shell's own LOG and reported
//! success (zsh: `bad file number: 3`, status 1), and `exec 3>myfile` would
//! dup2 over the log handle — `exec 4>myfile` over the live sqlite handle.
//!
//! `movefd` only works for a descriptor we own and can close. sqlite caches the
//! fd number inside the connection, so it cannot be relocated after the fact.
//! The fix therefore has to make the open LAND high: hold every low fd for the
//! duration of the open, so the kernel's "lowest free descriptor" rule hands out
//! >= 10, then release. That covers lazily-opened descriptors (sqlite's WAL and
//! SHM appear on first use) as long as the open happens inside the guard.
//!
//! # Concurrency and fork
//!
//! Unlike C zsh, zshrs runs the guard off the main thread: `compinit` ships its
//! cache rebuild to the worker pool, and that task calls
//! [`crate::compsys::cache::CompsysCache::open`] while the main thread is still
//! sourcing the user's rc files. So the guard MUST be safe against concurrent
//! opens in other threads.
//!
//! It is made safe without any lock. Every descriptor the guard owns is obtained
//! from `fcntl(F_DUPFD_CLOEXEC)`, which allocates the lowest free descriptor as a
//! single atomic kernel operation — two threads racing get two distinct fds, and
//! the guard never names, and therefore never clobbers, a descriptor somebody
//! else owns.
//!
//! Lock-freedom is not an optimization here, it is a correctness requirement:
//! `fork(2)` is called directly in `fusevm_bridge.rs` and `ported/exec.rs`, and
//! only the forking thread survives into the child. A mutex held by any other
//! thread at that moment is inherited permanently locked, so a guard that
//! serialized on one would deadlock the first `LowFdGuard::new()` in the child.
//! Raw fd syscalls have no such hazard: the child inherits the fd table and the
//! guard behaves identically there.
//!
//! `F_DUPFD_CLOEXEC` also means the reserved slots do not leak through an
//! `exec(2)` that races the guard — the old `dup2` path cleared `FD_CLOEXEC` on
//! its targets, so a child could start life with `/dev/null` parked on fds 3-9.

use std::os::unix::io::RawFd;

/// The first descriptor the shell may use for itself — everything below belongs
/// to the script. Matches C's `movefd` threshold (Src/utils.c:1980, `F_DUPFD, 10`).
const FIRST_INTERNAL_FD: RawFd = 10;

/// The first descriptor the script can name. 0/1/2 are stdin/stdout/stderr and
/// are never part of the reservation — `exec 3>out` starts at 3.
const FIRST_SCRIPT_FD: RawFd = 3;

/// How many slots the guard can hold at most: the whole script range.
const SCRIPT_FD_SLOTS: usize = (FIRST_INTERNAL_FD - FIRST_SCRIPT_FD) as usize;

/// Occupies every currently-free descriptor below [`FIRST_INTERNAL_FD`] so that
/// opens performed while it is alive are pushed above the user's fd range.
/// Releases them on drop.
///
/// Descriptors that are ALREADY open are left alone — they may be the user's
/// (`zshrs 3>&1 -c '…'` is legal), and stealing them would be the very bug this
/// exists to prevent.
///
/// # Thread safety
///
/// Claiming is done exclusively through `fcntl(F_DUPFD_CLOEXEC)`, which asks the
/// kernel for the *lowest free* descriptor and allocates it in one atomic step.
/// The guard therefore only ever owns descriptors the kernel just handed it, and
/// never names a target descriptor.
///
/// The previous implementation probed each slot with `fcntl(fd, F_GETFD)` and
/// then `dup2`'d onto it. Those are two syscalls with a gap in between, and
/// `dup2` *silently closes* whatever occupies its target. Under the interactive
/// boot — where `compinit` ships its cache rebuild to a `zshrs-worker-N` thread
/// (`ext_builtins.rs`, `worker_pool.submit`) while the main thread is still
/// sourcing the user's rc files — a concurrent `File::open` could land on a slot
/// this guard had already probed as free. The `dup2` then closed a live
/// `OwnedFd` out from under its owner, and `drop` on that owner tripped std's
/// `debug_assert_fd_is_open`, aborting with
/// `IO Safety violation: owned file descriptor already closed`.
pub struct LowFdGuard {
    held: Vec<RawFd>,
}

impl LowFdGuard {
    /// Reserve the free script descriptors. Cheap: one `open` of `/dev/null`
    /// plus at most [`SCRIPT_FD_SLOTS`] `fcntl` calls, and only for slots that
    /// are actually free.
    pub fn new() -> Self {
        let mut held = Vec::with_capacity(SCRIPT_FD_SLOTS);
        // SAFETY: plain fd syscalls; every descriptor here came from this
        // thread's own `open`/`fcntl` and is closed exactly once — either at the
        // end of this function or in `drop`, never both.
        unsafe {
            let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC);
            if devnull < 0 {
                return Self { held };
            }
            // `open` also allocates the lowest free descriptor, so the /dev/null
            // handle may itself land inside the range being reserved. When it
            // does it counts as one of the reserved slots and is released with
            // the rest; otherwise it is a plain temporary closed below.
            let devnull_is_held = (FIRST_SCRIPT_FD..FIRST_INTERNAL_FD).contains(&devnull);
            if devnull_is_held {
                held.push(devnull);
            }
            // Duplicate repeatedly. Each `F_DUPFD_CLOEXEC` returns the lowest
            // free descriptor at or above `FIRST_SCRIPT_FD`, so successive calls
            // walk up through the free slots; the first result at or above
            // `FIRST_INTERNAL_FD` proves the whole script range is now spoken for
            // — by this guard or by a pre-existing owner — and ends the loop.
            // `held.len()` bounds the iteration count even if another thread
            // frees a low slot mid-walk.
            //
            // This is `movefd` used as a claim rather than a move
            // (c:Src/utils.c:1980 `fcntl(fd, F_DUPFD, 10)`).
            while held.len() < SCRIPT_FD_SLOTS {
                let fd = libc::fcntl(devnull, libc::F_DUPFD_CLOEXEC, FIRST_SCRIPT_FD);
                if fd < 0 {
                    break;
                }
                if fd >= FIRST_INTERNAL_FD {
                    libc::close(fd);
                    break;
                }
                held.push(fd);
            }
            if !devnull_is_held {
                libc::close(devnull);
            }
        }
        // Off by default (`trace`), and the one call this module cares about —
        // "which thread reserved which slots" — is the only way to tell a
        // main-thread reservation from a worker-pool one after the fact.
        tracing::trace!(?held, "lowfd: reserved script descriptors");
        Self { held }
    }
}

impl Drop for LowFdGuard {
    fn drop(&mut self) {
        for &fd in &self.held {
            // SAFETY: only descriptors this guard installed are closed.
            unsafe {
                libc::close(fd);
            }
        }
    }
}

impl Default for LowFdGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Run `f` with the low descriptors reserved, so anything it opens lands at or
/// above [`FIRST_INTERNAL_FD`].
pub fn with_high_fds<T>(f: impl FnOnce() -> T) -> T {
    let _guard = LowFdGuard::new();
    f()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt as _;
    use std::os::unix::io::AsRawFd;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// The guard is process-global state. Two of these tests running on
    /// different harness threads would observe each other's reservations, so
    /// they take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// Which of the script descriptors are currently open.
    fn open_script_fds() -> Vec<RawFd> {
        (FIRST_SCRIPT_FD..FIRST_INTERNAL_FD)
            .filter(|&fd| unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1)
            .collect()
    }

    #[test]
    fn guard_pushes_opens_above_the_script_range() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let guard = LowFdGuard::new();
        let f = std::fs::File::open("/dev/null").expect("open /dev/null");
        assert!(
            f.as_raw_fd() >= FIRST_INTERNAL_FD,
            "open inside the guard landed on fd {}, inside the script range",
            f.as_raw_fd()
        );
        drop(f);
        drop(guard);
    }

    #[test]
    fn guard_restores_the_script_range_exactly() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let before = open_script_fds();
        {
            let _g = LowFdGuard::new();
        }
        assert_eq!(
            before,
            open_script_fds(),
            "guard leaked or freed descriptors it did not take"
        );
    }

    /// The bug this module was rewritten for: the old probe-then-`dup2` claim
    /// could land on a descriptor another thread had just been handed, and
    /// `dup2` closes its target silently. Detect it by identity, not by
    /// liveness — a clobbered descriptor is still *open*, it just points at
    /// `/dev/null` instead of the file its owner opened.
    ///
    /// Deliberately uses raw `libc` descriptors rather than `std::fs::File`:
    /// under the old code the theft also tripped std's IO-safety assert, which
    /// aborts the whole test binary instead of failing one test.
    #[test]
    fn concurrent_guards_never_steal_a_descriptor_from_another_thread() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());

        let path = std::env::temp_dir().join(format!("zshrs-lowfd-{}", std::process::id()));
        std::fs::write(&path, b"sentinel").expect("write probe file");
        let want_ino = std::fs::metadata(&path).expect("stat probe file").ino();
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let claimers: Vec<_> = (0..3)
            .map(|_| {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        drop(LowFdGuard::new());
                    }
                })
            })
            .collect();

        // Meanwhile: open the probe file over and over. Whenever the kernel
        // hands back a descriptor in the script range, that descriptor is ours
        // and must still refer to the probe file.
        let mut stolen = None;
        for _ in 0..200_000 {
            let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
            if fd < 0 {
                continue;
            }
            if fd < FIRST_INTERNAL_FD {
                std::thread::yield_now();
                let mut st: libc::stat = unsafe { std::mem::zeroed() };
                if unsafe { libc::fstat(fd, &mut st) } == 0 && st.st_ino != want_ino {
                    stolen = Some(fd);
                }
            }
            unsafe { libc::close(fd) };
            if stolen.is_some() {
                break;
            }
        }

        stop.store(true, Ordering::Relaxed);
        for t in claimers {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&path);

        assert!(
            stolen.is_none(),
            "a LowFdGuard on another thread took over live fd {:?}",
            stolen
        );
    }

    /// The guard must stay usable in a `fork(2)` child. zshrs forks raw in a
    /// dozen places (`fusevm_bridge.rs`, `ported/exec.rs`, `modules/zpty.rs`,
    /// `modules/clone.rs`, …) and only the forking thread survives, so any lock
    /// another thread happened to hold is inherited permanently locked. An
    /// earlier attempt to fix the descriptor race by serializing `new()` on a
    /// `Mutex` was reverted for exactly this reason; this test fails (by
    /// timeout) if anyone reintroduces one.
    #[test]
    fn guard_still_works_in_a_fork_child_while_other_threads_hold_it() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());

        let stop = Arc::new(AtomicBool::new(false));
        let hammers: Vec<_> = (0..4)
            .map(|_| {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        drop(LowFdGuard::new());
                    }
                })
            })
            .collect();

        let mut stuck = 0;
        let mut failed = 0;
        for _ in 0..20 {
            // SAFETY: the child does nothing but build a guard, open once, and
            // `_exit` — no unwinding, no atexit handlers, no stdio.
            let pid = unsafe { libc::fork() };
            if pid == 0 {
                let g = LowFdGuard::new();
                // The contract is "opens land high", not "we claimed
                // something": a child forked while another thread held the
                // whole range inherits those descriptors and correctly
                // reserves nothing.
                let fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
                let ok = fd >= FIRST_INTERNAL_FD;
                unsafe { libc::close(fd) };
                drop(g);
                unsafe { libc::_exit(i32::from(!ok)) };
            }
            assert!(pid > 0, "fork failed");

            // A guard that deadlocked in the child never reaches `_exit`.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut status = 0;
            loop {
                let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
                if r == pid {
                    if libc::WEXITSTATUS(status) != 0 {
                        failed += 1;
                    }
                    break;
                }
                if std::time::Instant::now() > deadline {
                    stuck += 1;
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                    unsafe { libc::waitpid(pid, &mut status, 0) };
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }

        stop.store(true, Ordering::Relaxed);
        for t in hammers {
            let _ = t.join();
        }
        assert_eq!(stuck, 0, "LowFdGuard::new() hung in a fork child");
        assert_eq!(
            failed, 0,
            "an open inside a fork child's guard landed in the script fd range"
        );
    }

    /// The reservation must be `FD_CLOEXEC`. The old `dup2` claim cleared it, so
    /// an `exec` racing the guard started the child with `/dev/null` parked on
    /// the script's descriptors.
    #[test]
    fn reserved_descriptors_are_close_on_exec() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let before = open_script_fds();
        let guard = LowFdGuard::new();
        for &fd in &guard.held {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            assert!(flags != -1, "reserved fd {fd} is not open");
            assert!(
                flags & libc::FD_CLOEXEC != 0,
                "reserved fd {fd} would leak through exec"
            );
        }
        // Only free slots may be claimed — never one that was already live.
        for fd in &before {
            assert!(
                !guard.held.contains(fd),
                "guard claimed fd {fd}, which was already open"
            );
        }
        drop(guard);
    }
}
