//! Clone module — direct port of `src/zsh/Src/Modules/clone.c`
//! `bin_clone` (lines 42-107).
//!
//! Provides the clone builtin: fork the current shell onto a new
//! terminal. The child inherits state but re-initializes session
//! and controlling-tty membership; the parent's `$!` becomes the
//! child pid and the child's `$!` is zero.

use std::io;
use crate::ported::utils::zwarnnam;

/// Port of `bin_clone()` from `Src/Modules/clone.c:44`.
///
/// The clone builtin: fork the shell onto a new terminal. The C source
/// performs all the work inline (open ctty, fork, child does setsid +
/// dup2 + TIOCSCTTY, parent closes ttyfd and returns). Mirrors that
/// inline structure here — no helper extraction.
///
/// Returns `(status, error_text, child_pid)`:
/// - status 1 + diagnostic for arg-missing or open/fork errors
/// - status 0 + Some(child_pid) in parent on success
/// - status 0 + Some(0) in child on success (per zsh's `$!` semantics)
pub fn bin_clone(args: &[&str]) -> (i32, String, Option<u32>) {
    if args.is_empty() {
        return (1, "clone: terminal required\n".to_string(), None);
    }
    let tty_path = args[0];

    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::io::RawFd;

        let tty_c = match CString::new(tty_path) {
            Ok(c) => c,
            Err(_) => return (1, format!("clone: {}: invalid tty path\n", tty_path), None),
        };

        // clone.c:49 — open with O_NOCTTY so opening doesn't steal ctty.
        let ttyfd: RawFd = unsafe { libc::open(tty_c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
        if ttyfd < 0 {
            return (1, format!("clone: {}: {}\n", tty_path, io::Error::last_os_error()), None);
        }

        // clone.c:54 — fork. Parent: close ttyfd, return child pid.
        let pid = unsafe { libc::fork() };
        match pid {
            -1 => {
                unsafe { libc::close(ttyfd) };
                (1, format!("clone: {}: {}\n", tty_path, io::Error::last_os_error()), None)
            }
            0 => {
                // CHILD path — clone.c:55-98.
                unsafe {
                    // clone.c:60 — setsid creates a new session and pgid.
                    // Failure is non-fatal; zsh just warns.
                    if libc::setsid() == -1 {
                        zwarnnam("clone", &format!("failed to create new session: {}", io::Error::last_os_error()));
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
                    // clone.c:85-91 — verify by opening /dev/tty.
                    let dev_tty = b"/dev/tty\0";
                    let verify = libc::open(dev_tty.as_ptr() as *const libc::c_char, libc::O_RDWR);
                    if verify >= 0 {
                        libc::close(verify);
                    }
                }
                (0, String::new(), Some(0))
            }
            child_pid => {
                // PARENT path — clone.c:99-100, 105-106.
                unsafe { libc::close(ttyfd) };
                (0, String::new(), Some(child_pid as u32))
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tty_path;
        (1, format!("clone: {}: clone not supported\n", tty_path), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_clone_no_args() {
        let (status, _, _) = bin_clone(&[]);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_clone_invalid_tty() {
        let (status, output, _) = bin_clone(&["/nonexistent/tty"]);
        assert_eq!(status, 1);
        assert!(output.contains("clone"));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// clone - create a subshell with forked state
    pub(crate) fn bin_clone(&mut self, args: &[String]) -> i32 {
        // Direct port of zsh/Src/Modules/clone.c:43-107: detach
        // current shell as session leader on the named tty. The
        // earlier Rust stub spawned `zshrs -c <args>` as a normal
        // child process, which is NOT what zsh's clone does — clone
        // takes over the named tty (ttyname arg), forks, sets up a
        // new session, makes the named tty the controlling tty for
        // the child, and continues running the SAME shell on that
        // tty. The parent exits.
        //
        // src/clone.rs provides clone_shell with the full ctty
        // acquisition sequence (TIOCNOTTY / setsid / TIOCSCTTY).
        // We delegate to it and surface stderr via the (status,
        // text, pid) tuple. Setting `$!` to the new pid mirrors
        // zsh's job-control side effect.
        let argv: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let (code, msg, pid) = crate::clone::bin_clone(&argv);
        if !msg.is_empty() {
            if code == 0 {
                print!("{}", msg);
            } else {
                eprint!("{}", msg);
            }
        }
        if let Some(p) = pid {
            self.variables.insert("!".to_string(), p.to_string());
        }
        code
    }
}
// END moved-from-exec-rs

/// Module loader entry — port of `setup_()` from Src/Modules/clone.c:123.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/clone.c:130.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/clone.c:138.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/clone.c:145.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/clone.c:152.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/clone.c:159.
pub fn finish_() -> i32 {
    0
}
