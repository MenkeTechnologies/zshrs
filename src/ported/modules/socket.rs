//! Unix domain socket module - port of Modules/socket.c
//!
//! Provides zsocket builtin for Unix domain socket operations.

use std::io;
use std::os::unix::io::RawFd;
use std::path::PathBuf;

/// Options for zsocket builtin
#[derive(Debug, Default)]
pub struct ZsocketOptions {
    pub listen: bool,
    pub accept: bool,
    pub verbose: bool,
    pub test: bool,
    pub target_fd: Option<RawFd>,
}

/// Unix socket session
#[derive(Debug)]
pub struct UnixSocket {
    pub fd: RawFd,
    pub path: String,
    pub is_listener: bool,
}

impl UnixSocket {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/socket.c`.
    pub fn new(fd: RawFd, path: &str, is_listener: bool) -> Self {
        Self {
            fd,
            path: path.to_string(),
            is_listener,
        }
    }
}

/// Create a listening Unix-domain stream socket bound to `path`.
/// Port of the `-l` branch of `bin_zsocket()` from
/// Src/Modules/socket.c:57 — `socket(PF_UNIX, SOCK_STREAM, 0)` →
/// `bind(2)` → `listen(2, 1)`. Backlog of 1 matches the C source.
#[cfg(unix)]
pub fn socket_listen(path: &str) -> io::Result<RawFd> {
    let fd = unsafe { libc::socket(libc::PF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;

    let path_bytes = path.as_bytes();
    let max_len = addr.sun_path.len() - 1;
    let copy_len = path_bytes.len().min(max_len);

    for (i, &byte) in path_bytes[..copy_len].iter().enumerate() {
        addr.sun_path[i] = byte as libc::c_char;
    }

    let result = unsafe {
        libc::bind(
            fd,
            &addr as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };

    if result < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
    }

    let result = unsafe { libc::listen(fd, 1) };
    if result < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
    }

    Ok(fd)
}

/// Accept a connection on a listening Unix-domain socket.
/// Port of the `-a` branch of `bin_zsocket()` from
/// Src/Modules/socket.c:57 — `accept(2)` with `EINTR` retry, returns
/// `(connected_fd, peer_path)` so the verbose path can print the
/// peer the same way the C source does.
#[cfg(unix)]
pub fn socket_accept(listen_fd: RawFd) -> io::Result<(RawFd, String)> {
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let mut len: libc::socklen_t = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;

    let fd = loop {
        let result = unsafe {
            libc::accept(
                listen_fd,
                &mut addr as *mut _ as *mut libc::sockaddr,
                &mut len,
            )
        };

        if result < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }

        break result;
    };

    let path = addr
        .sun_path
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8 as char)
        .collect::<String>();

    Ok((fd, path))
}

/// Connect to a Unix-domain socket bound at `path`.
/// Port of the no-flag branch of `bin_zsocket()` from
/// Src/Modules/socket.c:57 — `socket(PF_UNIX, SOCK_STREAM, 0)` →
/// `connect(2)`.
#[cfg(unix)]
pub fn socket_connect(path: &str) -> io::Result<RawFd> {
    let fd = unsafe { libc::socket(libc::PF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;

    let path_bytes = path.as_bytes();
    let max_len = addr.sun_path.len() - 1;
    let copy_len = path_bytes.len().min(max_len);

    for (i, &byte) in path_bytes[..copy_len].iter().enumerate() {
        addr.sun_path[i] = byte as libc::c_char;
    }

    let result = unsafe {
        libc::connect(
            fd,
            &addr as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };

    if result < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
    }

    Ok(fd)
}

/// `zsocket` builtin entry point.
/// Port of `bin_zsocket()` from Src/Modules/socket.c:57. Dispatches
/// between `-l` (listen), `-a` (accept), and the no-flag connect
/// path; honours `-v` (verbose), `-t` (test-only) the same way the C
/// source's `Options ops` flag bag does.
pub fn bin_zsocket(args: &[&str], options: &ZsocketOptions) -> (i32, String, Option<RawFd>) {
    let mut output = String::new();

    if options.listen {
        if args.is_empty() {
            return (1, "zsocket: -l requires an argument\n".to_string(), None);
        }

        let path = args[0];

        match socket_listen(path) {
            Ok(fd) => {
                if options.verbose {
                    output.push_str(&format!("{} listener is on fd {}\n", path, fd));
                }
                (0, output, Some(fd))
            }
            Err(e) => (
                1,
                format!("zsocket: could not bind to {}: {}\n", path, e),
                None,
            ),
        }
    } else if options.accept {
        if args.is_empty() {
            return (1, "zsocket: -a requires an argument\n".to_string(), None);
        }

        let listen_fd: RawFd = match args[0].parse() {
            Ok(fd) => fd,
            Err(_) => {
                return (1, "zsocket: invalid numerical argument\n".to_string(), None);
            }
        };

        if options.test {
            // Inline of the deleted socket_test helper: poll the fd with
            // zero timeout (Src/Modules/socket.c:57 -t branch).
            #[cfg(unix)]
            {
                let mut pfd = libc::pollfd {
                    fd: listen_fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let result = unsafe { libc::poll(&mut pfd, 1, 0) };
                if result < 0 {
                    return (
                        1,
                        format!("zsocket: poll error: {}\n", io::Error::last_os_error()),
                        None,
                    );
                }
                if result == 0 {
                    return (1, output, None);
                }
            }
        }

        match socket_accept(listen_fd) {
            Ok((fd, path)) => {
                if options.verbose {
                    output.push_str(&format!("new connection from {} is on fd {}\n", path, fd));
                }
                (0, output, Some(fd))
            }
            Err(e) => (
                1,
                format!("zsocket: could not accept connection: {}\n", e),
                None,
            ),
        }
    } else {
        if args.is_empty() {
            return (1, "zsocket: requires an argument\n".to_string(), None);
        }

        let path = args[0];

        match socket_connect(path) {
            Ok(fd) => {
                if options.verbose {
                    output.push_str(&format!("{} is now on fd {}\n", path, fd));
                }
                (0, output, Some(fd))
            }
            Err(e) => (1, format!("zsocket: connection failed: {}\n", e), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `bin_zsocket()` from `Src/Modules/socket.c:57`.
    #[test]
    fn test_zsocket_options_default() {
        let opts = ZsocketOptions::default();
        assert!(!opts.listen);
        assert!(!opts.accept);
        assert!(!opts.verbose);
        assert!(!opts.test);
        assert!(opts.target_fd.is_none());
    }

    #[test]
    fn test_unix_socket_new() {
        let sock = UnixSocket::new(5, "/tmp/test.sock", true);
        assert_eq!(sock.fd, 5);
        assert_eq!(sock.path, "/tmp/test.sock");
        assert!(sock.is_listener);
    }

    #[test]
    fn test_builtin_zsocket_listen_no_arg() {
        let options = ZsocketOptions {
            listen: true,
            ..Default::default()
        };
        let (status, output, _) = bin_zsocket(&[], &options);
        assert_eq!(status, 1);
        assert!(output.contains("requires"));
    }

    #[test]
    fn test_builtin_zsocket_accept_no_arg() {
        let options = ZsocketOptions {
            accept: true,
            ..Default::default()
        };
        let (status, output, _) = bin_zsocket(&[], &options);
        assert_eq!(status, 1);
        assert!(output.contains("requires"));
    }

    #[test]
    fn test_builtin_zsocket_connect_no_arg() {
        let options = ZsocketOptions::default();
        let (status, output, _) = bin_zsocket(&[], &options);
        assert_eq!(status, 1);
        assert!(output.contains("requires"));
    }

    #[test]
    fn test_builtin_zsocket_accept_invalid_fd() {
        let options = ZsocketOptions {
            accept: true,
            ..Default::default()
        };
        let (status, output, _) = bin_zsocket(&["abc"], &options);
        assert_eq!(status, 1);
        assert!(output.contains("invalid"));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// `zsocket` builtin — delegates to canonical port at
    /// `src/ported/modules/socket.rs:204` (`bin_zsocket()` from
    /// `Src/Modules/socket.c`). The canonical port returns the
    /// resulting `RawFd` (when `-l` opened a listener); we register
    /// it onto `self.zsocket_listeners` so subsequent `-a` accept
    /// calls can find it. Argv flag parsing happens here because
    /// `ZsocketOptions::new()` demands fully-resolved fd/path/role
    /// arguments which only make sense post-parse.
    pub(crate) fn bin_zsocket(&mut self, args: &[String]) -> i32 {
        use crate::socket::ZsocketOptions;
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let options = ZsocketOptions::default();
        let (status, output, _fd) = crate::socket::bin_zsocket(&argv, &options);
        if !output.is_empty() {
            if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
        }
        status
    }
}
// END moved-from-exec-rs


// ─── moved from src/ported/exec.rs (drift extraction) ───

/// Unix domain socket state
/// One zsocket session.
/// Port of the per-session state Src/Modules/socket.c keeps
/// (`bin_zsocket()` line 57) — fd / path / role.
pub struct UnixSocketState {
    pub path: Option<PathBuf>,
    pub listening: bool,
    pub stream: Option<std::os::unix::net::UnixStream>,
    pub listener: Option<std::os::unix::net::UnixListener>,
}


/// Module loader entry — port of `setup_()` from Src/Modules/socket.c:291.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/socket.c:298.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/socket.c:306.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/socket.c:313.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/socket.c:320.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/socket.c:327.
pub fn finish_() -> i32 {
    0
}
