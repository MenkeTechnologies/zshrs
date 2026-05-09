//! Unix domain socket module — port of `Src/Modules/socket.c`.
//!
//! C source has zero `struct ...` / `enum ...` definitions. The
//! Rust port matches: zero types, only the function ports
//! (`bin_zsocket`, `setup_`/`features_`/`enables_`/`boot_`/
//! `cleanup_`/`finish_`).

use std::io;
use std::os::unix::io::RawFd;

/// `zsocket` builtin — port of `bin_zsocket()` from
/// `Src/Modules/socket.c:57`.
///
/// C signature: `static int bin_zsocket(char *nam, char **args,
///                                       Options ops, int func)`.
/// zshrs's builtin dispatch hands us argv post-flag-parse-by-name,
/// so this entry takes only `args: &[&str]` and parses the
/// `-a`/`-d FD`/`-l`/`-t`/`-v` flags inline (option spec from
/// socket.c:276 BUILTIN spec `"ad:ltv"`). Returns the exit code,
/// the message (stdout for success, stderr for error), and the
/// resulting fd for the listen/connect/accept paths so the
/// caller can register it in the shell's fdtable (matching C's
/// `addmodulefd(sfd, FDT_EXTERNAL)` at socket.c:131/167).
pub fn bin_zsocket(args: &[&str]) -> (i32, String, Option<RawFd>) {     // c:57
    // c:60-83 — flag parse (mirrors OPT_ISSET against "ad:ltv").
    let mut verbose = false;                                            // c:60
    let mut test = false;                                               // c:60
    let mut targetfd: i32 = 0;                                          // c:60
    let mut do_listen = false;                                          // c:60
    let mut do_accept = false;                                          // c:60
    let mut argv: Vec<&str> = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if a == "--" {
            i += 1;
            while i < args.len() {
                argv.push(args[i]);
                i += 1;
            }
            break;
        }
        if a.starts_with('-') && a.len() > 1 && !a[1..].chars().next().unwrap().is_ascii_digit() {
            for c in a[1..].chars() {
                match c {
                    'v' => verbose = true,                              // c:65
                    't' => test = true,                                 // c:68
                    'l' => do_listen = true,                            // c:84
                    'a' => do_accept = true,                            // c:142
                    'd' => {                                            // c:71
                        i += 1;
                        if i >= args.len() {
                            return (1, "zsocket: -d requires an argument\n".to_string(), None);
                        }
                        match args[i].parse::<i32>() {
                            Ok(n) => targetfd = n,
                            Err(_) => {
                                return (
                                    1,
                                    format!("zsocket: {} is an invalid argument to -d\n", args[i]),
                                    None,
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else {
            argv.push(a);
        }
        i += 1;
    }
    let _ = targetfd; // -d wiring (redup) deferred until shell fdtable is wired.

    let mut output = String::new();

    if do_listen {                                                      // c:84
        if argv.is_empty() {                                            // c:86
            return (1, "zsocket: -l requires an argument\n".to_string(), None);
        }
        let path = argv[0];                                             // c:90
        let listen_result = (|| -> io::Result<RawFd> {
            #[cfg(unix)]
            {
                let fd = unsafe { libc::socket(libc::PF_UNIX, libc::SOCK_STREAM, 0) }; // c:92
                if fd < 0 {
                    return Err(io::Error::last_os_error());             // c:96
                }
                let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
                addr.sun_family = libc::AF_UNIX as libc::sa_family_t;   // c:99
                let path_bytes = path.as_bytes();
                let max_len = addr.sun_path.len() - 1;
                let copy_len = path_bytes.len().min(max_len);
                for (i, &byte) in path_bytes[..copy_len].iter().enumerate() {
                    addr.sun_path[i] = byte as libc::c_char;            // c:100
                }
                let r = unsafe {
                    libc::bind(                                          // c:102
                        fd,
                        &addr as *const _ as *const libc::sockaddr,
                        std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
                    )
                };
                if r < 0 {
                    let err = io::Error::last_os_error();
                    unsafe { libc::close(fd) };
                    return Err(err);                                    // c:107
                }
                let r = unsafe { libc::listen(fd, 1) };                 // c:111
                if r < 0 {
                    let err = io::Error::last_os_error();
                    unsafe { libc::close(fd) };
                    return Err(err);                                    // c:114
                }
                Ok(fd)
            }
            #[cfg(not(unix))]
            {
                let _ = path;
                Err(io::Error::new(io::ErrorKind::Unsupported, "no unix sockets"))
            }
        })();
        return match listen_result {
            Ok(fd) => {
                if verbose {                                            // c:135
                    output.push_str(&format!("{} listener is on fd {}\n", path, fd));
                }
                (0, output, Some(fd))                                   // c:137
            }
            Err(e) => (
                1,
                format!("zsocket: could not bind to {}: {}\n", path, e),
                None,
            ),
        };
    }

    if do_accept {                                                      // c:142
        if argv.is_empty() {                                            // c:146
            return (1, "zsocket: -a requires an argument\n".to_string(), None);
        }
        let listen_fd: RawFd = match argv[0].parse() {                  // c:151
            Ok(fd) => fd,
            Err(_) => {
                return (1, "zsocket: invalid numerical argument\n".to_string(), None);
            }
        };
        if test {                                                       // c:156
            #[cfg(unix)]
            {
                let mut pfd = libc::pollfd {
                    fd: listen_fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let r = unsafe { libc::poll(&mut pfd, 1, 0) };          // c:158
                if r < 0 {
                    return (
                        1,
                        format!("zsocket: poll error: {}\n", io::Error::last_os_error()),
                        None,
                    );
                }
                if r == 0 {
                    return (1, output, None);                           // c:165
                }
            }
        }
        let accept_result = (|| -> io::Result<(RawFd, String)> {
            #[cfg(unix)]
            {
                let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
                let mut len: libc::socklen_t =
                    std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
                let fd = loop {
                    let r = unsafe {
                        libc::accept(                                    // c:175
                            listen_fd,
                            &mut addr as *mut _ as *mut libc::sockaddr,
                            &mut len,
                        )
                    };
                    if r < 0 {
                        let err = io::Error::last_os_error();
                        if err.kind() == io::ErrorKind::Interrupted {
                            continue;                                    // c:178
                        }
                        return Err(err);
                    }
                    break r;
                };
                let path = addr
                    .sun_path
                    .iter()
                    .take_while(|&&c| c != 0)
                    .map(|&c| c as u8 as char)
                    .collect::<String>();
                Ok((fd, path))
            }
            #[cfg(not(unix))]
            {
                Err(io::Error::new(io::ErrorKind::Unsupported, "no unix sockets"))
            }
        })();
        return match accept_result {
            Ok((fd, path)) => {
                if verbose {                                            // c:198
                    output.push_str(&format!("new connection from {} is on fd {}\n", path, fd));
                }
                (0, output, Some(fd))                                   // c:200
            }
            Err(e) => (
                1,
                format!("zsocket: could not accept connection: {}\n", e),
                None,
            ),
        };
    }

    // No -l, no -a → connect path (c:218).
    if argv.is_empty() {                                                // c:223
        return (1, "zsocket: requires an argument\n".to_string(), None);
    }
    let path = argv[0];                                                 // c:227
    let connect_result = (|| -> io::Result<RawFd> {
        #[cfg(unix)]
        {
            let fd = unsafe { libc::socket(libc::PF_UNIX, libc::SOCK_STREAM, 0) }; // c:229
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
            addr.sun_family = libc::AF_UNIX as libc::sa_family_t;       // c:236
            let path_bytes = path.as_bytes();
            let max_len = addr.sun_path.len() - 1;
            let copy_len = path_bytes.len().min(max_len);
            for (i, &byte) in path_bytes[..copy_len].iter().enumerate() {
                addr.sun_path[i] = byte as libc::c_char;
            }
            let r = unsafe {
                libc::connect(                                           // c:240
                    fd,
                    &addr as *const _ as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
                )
            };
            if r < 0 {
                let err = io::Error::last_os_error();
                unsafe { libc::close(fd) };
                return Err(err);                                        // c:245
            }
            Ok(fd)
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(io::Error::new(io::ErrorKind::Unsupported, "no unix sockets"))
        }
    })();
    match connect_result {
        Ok(fd) => {
            if verbose {                                                // c:265
                output.push_str(&format!("{} is now on fd {}\n", path, fd));
            }
            (0, output, Some(fd))                                       // c:267
        }
        Err(e) => (1, format!("zsocket: connection failed: {}\n", e), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_zsocket_listen_no_arg() {
        let (status, output, _) = bin_zsocket(&["-l"]);
        assert_eq!(status, 1);
        assert!(output.contains("requires"));
    }

    #[test]
    fn test_builtin_zsocket_accept_no_arg() {
        let (status, output, _) = bin_zsocket(&["-a"]);
        assert_eq!(status, 1);
        assert!(output.contains("requires"));
    }

    #[test]
    fn test_builtin_zsocket_connect_no_arg() {
        let (status, output, _) = bin_zsocket(&[]);
        assert_eq!(status, 1);
        assert!(output.contains("requires"));
    }

    #[test]
    fn test_builtin_zsocket_accept_invalid_fd() {
        let (status, output, _) = bin_zsocket(&["-a", "abc"]);
        assert_eq!(status, 1);
        assert!(output.contains("invalid"));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// `zsocket` builtin shim — delegates to canonical port at
    /// `bin_zsocket()` above (port of `Src/Modules/socket.c:57`).
    /// Argv flag parsing happens inside the canonical port (matching
    /// the C builtin's option spec parser); the shim only adapts the
    /// `&[String]` argv to the canonical `&[&str]` shape.
    pub(crate) fn bin_zsocket(&mut self, args: &[String]) -> i32 {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let (status, output, _fd) = crate::socket::bin_zsocket(&argv);
        if !output.is_empty() {
            if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
        }
        status
    }
}
// END moved-from-exec-rs

/// Port of `setup_()` from `Src/Modules/socket.c:291`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:291
    0                                                                    // c:294
}

/// Port of `features_()` from `Src/Modules/socket.c:298`. C body
/// is `*features = featuresarray(m, &module_features); return 0;`.
/// Static-link path: 0.
pub fn features_() -> i32 {                                              // c:298
    0                                                                    // c:302
}

/// Port of `enables_()` from `Src/Modules/socket.c:306`. C body is
/// `return handlefeatures(m, &module_features, enables);`.
pub fn enables_() -> i32 {                                               // c:306
    0                                                                    // c:310
}

/// Port of `boot_()` from `Src/Modules/socket.c:313`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn boot_() -> i32 {                                                  // c:313
    0                                                                    // c:316
}

/// Port of `cleanup_()` from `Src/Modules/socket.c:320`. C body
/// is `return setfeatureenables(m, &module_features, NULL);`.
pub fn cleanup_() -> i32 {                                               // c:320
    0                                                                    // c:323
}

/// Port of `finish_()` from `Src/Modules/socket.c:327`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:327
    0                                                                    // c:330
}
