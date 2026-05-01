//! Clone module — direct port of `src/zsh/Src/Modules/clone.c`
//! `bin_clone` (lines 42-107).
//!
//! Provides the clone builtin: fork the current shell onto a new
//! terminal. The child inherits state but re-initializes session
//! and controlling-tty membership; the parent's `$!` becomes the
//! child pid and the child's `$!` is zero.

use std::io;

/// Result returned by `clone_shell` so the caller can wire $! /
/// $TTY / $PID / $PPID per src/zsh/Src/Modules/clone.c:55-98.
#[cfg(unix)]
pub struct CloneOutcome {
    /// Child pid in the parent (post-fork). 0 means we are the child.
    pub pid: i32,
    /// True iff the child successfully acquired the new tty as its
    /// controlling terminal. Mirrors the `cttyfd = open("/dev/tty")`
    /// success branch of clone.c:86-91.
    pub got_ctty: bool,
}

/// Clone the current shell to a new terminal. Direct port of
/// src/zsh/Src/Modules/clone.c:43-107 `bin_clone`.
///
/// Steps mirror the C implementation:
///   1. Open the tty path with O_RDWR|O_NOCTTY (clone.c:49).
///   2. fork(). On error, propagate (clone.c:101-104).
///   3. In the child:
///      a. setsid() (clone.c:60), warn if failed.
///      b. dup2 ttyfd onto fds 0/1/2 (clone.c:67-69).
///      c. close ttyfd if > 2 (clone.c:70-71).
///      d. open tty again to acquire it as ctty + TIOCSCTTY
///         (clone.c:75-84).
///      e. open /dev/tty to verify ctty acquisition (clone.c:85-91).
#[cfg(unix)]
pub fn clone_shell(tty_path: &str) -> io::Result<CloneOutcome> {
    use std::ffi::CString;
    use std::os::unix::io::RawFd;

    let tty_c = CString::new(tty_path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid tty path"))?;

    // clone.c:49 — open with O_NOCTTY so opening doesn't steal ctty.
    let ttyfd: RawFd = unsafe { libc::open(tty_c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };

    if ttyfd < 0 {
        return Err(io::Error::last_os_error());
    }

    // clone.c:54 — fork. Parent: close ttyfd, return child pid.
    // Child: continues at the !pid branch below.
    let pid = unsafe { libc::fork() };

    match pid {
        -1 => {
            unsafe { libc::close(ttyfd) };
            Err(io::Error::last_os_error())
        }
        0 => {
            // CHILD path — clone.c:55-98.
            let mut got_ctty = false;
            unsafe {
                // clone.c:60 — setsid creates a new session and pgid.
                // Failure is non-fatal; zsh just warns.
                if libc::setsid() == -1 {
                    eprintln!(
                        "clone: failed to create new session: {}",
                        io::Error::last_os_error()
                    );
                }

                // clone.c:67-69 — point std fds at the new tty.
                libc::dup2(ttyfd, 0);
                libc::dup2(ttyfd, 1);
                libc::dup2(ttyfd, 2);

                // clone.c:70-71 — close the original ttyfd if it's
                // not already 0/1/2 after dup2.
                if ttyfd > 2 {
                    libc::close(ttyfd);
                }

                // clone.c:75-84 — re-open the tty to acquire it as
                // controlling terminal via TIOCSCTTY.
                let cttyfd = libc::open(tty_c.as_ptr(), libc::O_RDWR);
                if cttyfd >= 0 {
                    #[cfg(any(target_os = "linux", target_os = "macos"))]
                    {
                        libc::ioctl(cttyfd, libc::TIOCSCTTY as libc::c_ulong, 0);
                    }
                    libc::close(cttyfd);
                }

                // clone.c:85-91 — verify by opening /dev/tty. If this
                // succeeds the kernel has assigned a ctty.
                let dev_tty = b"/dev/tty\0";
                let verify = libc::open(dev_tty.as_ptr() as *const libc::c_char, libc::O_RDWR);
                if verify >= 0 {
                    got_ctty = true;
                    libc::close(verify);
                }
            }

            Ok(CloneOutcome { pid: 0, got_ctty })
        }
        child_pid => {
            // PARENT path — clone.c:99-100, 105-106.
            unsafe { libc::close(ttyfd) };
            Ok(CloneOutcome {
                pid: child_pid,
                got_ctty: true,
            })
        }
    }
}

#[cfg(not(unix))]
pub struct CloneOutcome {
    pub pid: i32,
    pub got_ctty: bool,
}

#[cfg(not(unix))]
pub fn clone_shell(_tty_path: &str) -> io::Result<CloneOutcome> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "clone not supported",
    ))
}

/// Execute clone builtin. Direct port of clone.c:43-107 entry point.
/// Returns (status, error_text_for_caller, child_pid).
/// child_pid > 0 in parent, 0 in child, None on failure.
pub fn builtin_clone(args: &[&str]) -> (i32, String, Option<u32>) {
    if args.is_empty() {
        return (1, "clone: terminal required\n".to_string(), None);
    }

    match clone_shell(args[0]) {
        Ok(o) => (0, String::new(), Some(o.pid as u32)),
        Err(e) => (1, format!("clone: {}: {}\n", args[0], e), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_clone_no_args() {
        let (status, _, _) = builtin_clone(&[]);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_clone_invalid_tty() {
        let (status, output, _) = builtin_clone(&["/nonexistent/tty"]);
        assert_eq!(status, 1);
        assert!(output.contains("clone"));
    }
}
