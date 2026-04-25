//! Unix domain socket module - port of Modules/socket.c
//!
//! Provides zsocket builtin for Unix domain socket operations.

use std::io;
use std::os::unix::io::RawFd;

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
    pub fn new(fd: RawFd, path: &str, is_listener: bool) -> Self {
        Self {
            fd,
            path: path.to_string(),
            is_listener,
        }
    }
}

/// Create a listening Unix socket
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

/// Accept a connection on a listening Unix socket
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

/// Test if a socket has pending connections
#[cfg(unix)]
pub fn socket_test(fd: RawFd) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };

    let result = unsafe { libc::poll(&mut pfd, 1, 0) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(result > 0)
}

/// Connect to a Unix socket
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

/// Close a socket
#[cfg(unix)]
pub fn socket_close(fd: RawFd) -> io::Result<()> {
    let result = unsafe { libc::close(fd) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Execute zsocket builtin
pub fn builtin_zsocket(args: &[&str], options: &ZsocketOptions) -> (i32, String, Option<RawFd>) {
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
            match socket_test(listen_fd) {
                Ok(true) => {}
                Ok(false) => return (1, output, None),
                Err(e) => return (1, format!("zsocket: poll error: {}\n", e), None),
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
        let (status, output, _) = builtin_zsocket(&[], &options);
        assert_eq!(status, 1);
        assert!(output.contains("requires"));
    }

    #[test]
    fn test_builtin_zsocket_accept_no_arg() {
        let options = ZsocketOptions {
            accept: true,
            ..Default::default()
        };
        let (status, output, _) = builtin_zsocket(&[], &options);
        assert_eq!(status, 1);
        assert!(output.contains("requires"));
    }

    #[test]
    fn test_builtin_zsocket_connect_no_arg() {
        let options = ZsocketOptions::default();
        let (status, output, _) = builtin_zsocket(&[], &options);
        assert_eq!(status, 1);
        assert!(output.contains("requires"));
    }

    #[test]
    fn test_builtin_zsocket_accept_invalid_fd() {
        let options = ZsocketOptions {
            accept: true,
            ..Default::default()
        };
        let (status, output, _) = builtin_zsocket(&["abc"], &options);
        assert_eq!(status, 1);
        assert!(output.contains("invalid"));
    }
}
