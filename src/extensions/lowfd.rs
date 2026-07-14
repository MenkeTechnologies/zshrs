//! Keep the shell's own file descriptors out of the user's fd space.
//!
//! zsh routes every internal open through `movefd()` (Src/utils.c:1990), which
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

use std::os::unix::io::RawFd;

/// The first descriptor the shell may use for itself — everything below belongs
/// to the script. Matches C's `movefd` threshold (Src/utils.c:1994, `F_DUPFD, 10`).
const FIRST_INTERNAL_FD: RawFd = 10;

/// Occupies every currently-free descriptor below [`FIRST_INTERNAL_FD`] so that
/// opens performed while it is alive are pushed above the user's fd range.
/// Releases them on drop.
///
/// Descriptors that are ALREADY open are left alone — they may be the user's
/// (`zshrs 3>&1 -c '…'` is legal), and stealing them would be the very bug this
/// exists to prevent.
pub struct LowFdGuard {
    held: Vec<RawFd>,
}

impl LowFdGuard {
    /// Reserve the free low descriptors. Cheap: at most seven `dup2` calls onto
    /// `/dev/null`, and only for slots that are actually free.
    pub fn new() -> Self {
        let mut held = Vec::new();
        // SAFETY: plain fd syscalls; every descriptor opened here is recorded and
        // closed in `drop`.
        unsafe {
            let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC);
            if devnull < 0 {
                return Self { held };
            }
            // The /dev/null handle itself lands on the LOWEST free descriptor —
            // i.e. inside the very range being reserved. Left there, the loop
            // below would see that slot as "already open", skip it, and then free
            // it again on drop, so the next open would land right back on it.
            // Move the handle above the range first (this is `movefd` in miniature,
            // c:Src/utils.c:1994 `fcntl(fd, F_DUPFD, 10)`).
            let devnull = {
                let hi = libc::fcntl(devnull, libc::F_DUPFD, FIRST_INTERNAL_FD);
                if hi >= 0 {
                    libc::close(devnull);
                    hi
                } else {
                    devnull
                }
            };
            for fd in 3..FIRST_INTERNAL_FD {
                // Only claim descriptors that are closed. F_GETFD on a live fd
                // succeeds, and that one is not ours to take.
                if libc::fcntl(fd, libc::F_GETFD) != -1 {
                    continue;
                }
                if libc::dup2(devnull, fd) == fd {
                    held.push(fd);
                }
            }
            libc::close(devnull);
        }
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
