//! Clone module - port of Modules/clone.c
//!
//! Provides the clone builtin to start a forked instance of the shell on a new terminal.

use std::io;

/// Clone the current shell to a new terminal
#[cfg(unix)]
pub fn clone_shell(tty_path: &str) -> io::Result<u32> {
    use std::ffi::CString;
    use std::os::unix::io::RawFd;

    let tty_c = CString::new(tty_path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid tty path"))?;

    let ttyfd: RawFd = unsafe { libc::open(tty_c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };

    if ttyfd < 0 {
        return Err(io::Error::last_os_error());
    }

    let pid = unsafe { libc::fork() };

    match pid {
        -1 => {
            unsafe { libc::close(ttyfd) };
            Err(io::Error::last_os_error())
        }
        0 => {
            unsafe {
                if libc::setsid() == -1 {
                    eprintln!(
                        "clone: failed to create new session: {}",
                        io::Error::last_os_error()
                    );
                }

                libc::dup2(ttyfd, 0);
                libc::dup2(ttyfd, 1);
                libc::dup2(ttyfd, 2);

                if ttyfd > 2 {
                    libc::close(ttyfd);
                }

                let cttyfd = libc::open(tty_c.as_ptr(), libc::O_RDWR);
                if cttyfd >= 0 {
                    #[cfg(any(target_os = "linux", target_os = "macos"))]
                    {
                        libc::ioctl(cttyfd, libc::TIOCSCTTY as libc::c_ulong, 0);
                    }
                    libc::close(cttyfd);
                }
            }

            Ok(0)
        }
        child_pid => {
            unsafe { libc::close(ttyfd) };
            Ok(child_pid as u32)
        }
    }
}

#[cfg(not(unix))]
pub fn clone_shell(_tty_path: &str) -> io::Result<u32> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "clone not supported",
    ))
}

/// Execute clone builtin
pub fn builtin_clone(args: &[&str]) -> (i32, String, Option<u32>) {
    if args.is_empty() {
        return (1, "clone: terminal required\n".to_string(), None);
    }

    match clone_shell(args[0]) {
        Ok(pid) => (0, String::new(), Some(pid)),
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
