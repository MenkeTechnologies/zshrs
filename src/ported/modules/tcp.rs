//! TCP networking module — port of `Src/Modules/tcp.c` +
//! `Src/Modules/tcp.h`.
//!
//! C `tcp.h` defines:
//!   - `union tcp_sockaddr` (tcp.h:74) — wraps sockaddr / sockaddr_in /
//!     sockaddr_in6.
//!   - `struct tcp_session` (tcp.h:88) — { fd, sock, peer, flags }.
//!   - Flag constants ZTCP_LISTEN / ZTCP_INBOUND / ZTCP_ZFTP.
//!
//! C `tcp.c` has a single file-static linked list `ztcp_sessions`
//! holding live `Tcp_session` records.
//!
//! Rust port mirrors these 1:1: a `pub struct Tcp_session` matching
//! C field set, a `pub union TcpSockaddr` matching C union, and a
//! `thread_local!` Vec replacing the linked list (per PORT_PLAN
//! Phase 2 bucket-1: file-statics → thread_local).

use std::os::unix::io::RawFd;

/// Port of `ZTCP_LISTEN` from `Src/Modules/tcp.h:85`.
pub const ZTCP_LISTEN:  i32 = 1;                                         // c:tcp.h:85
/// Port of `ZTCP_INBOUND` from `Src/Modules/tcp.h:86`.
pub const ZTCP_INBOUND: i32 = 2;                                         // c:tcp.h:86
/// Port of `ZTCP_ZFTP` from `Src/Modules/tcp.h:87`.
pub const ZTCP_ZFTP:    i32 = 16;                                        // c:tcp.h:87

/// Port of `union tcp_sockaddr` from `Src/Modules/tcp.h:74`.
/// C definition:
/// ```c
/// union tcp_sockaddr {
///     struct sockaddr a;
///     struct sockaddr_in in;
/// #ifdef SUPPORT_IPV6
///     struct sockaddr_in6 in6;
/// #endif
/// };
/// ```
#[repr(C)]
#[allow(non_camel_case_types)]
pub union TcpSockaddr {
    pub a: libc::sockaddr,
    pub in_: libc::sockaddr_in,
    pub in6: libc::sockaddr_in6,
}

impl Default for TcpSockaddr {
    fn default() -> Self {
        Self { a: unsafe { std::mem::zeroed() } }
    }
}

/// Port of `struct tcp_session` from `Src/Modules/tcp.h:88`. C:
/// ```c
/// struct tcp_session {
///     int fd;                    /* file descriptor */
///     union tcp_sockaddr sock;   /* local address */
///     union tcp_sockaddr peer;   /* remote address */
///     int flags;
/// };
/// ```
#[allow(non_camel_case_types)]
pub struct Tcp_session {                                                 // c:tcp.h:88
    pub fd: RawFd,
    pub sock: TcpSockaddr,
    pub peer: TcpSockaddr,
    pub flags: i32,
}

impl Default for Tcp_session {
    fn default() -> Self {
        Self {
            fd: -1,
            sock: TcpSockaddr::default(),
            peer: TcpSockaddr::default(),
            flags: 0,
        }
    }
}

// File-static `ztcp_sessions` linked list — per PORT_PLAN Phase 2
// bucket-1, ported as a thread_local Vec.
thread_local! {
    /// Port of file-static `ztcp_sessions` from `Src/Modules/tcp.c`.
    static ZTCP_SESSIONS: std::cell::RefCell<Vec<Tcp_session>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

/// Port of `zsh_inet_ntop()` from `Src/Modules/tcp.c:72`. Wraps
/// libc inet_ntop(3) — converts AF_INET / AF_INET6 network-byte
/// addresses to dotted/colon presentation form.
pub fn zsh_inet_ntop(af: i32, addr_bytes: &[u8]) -> Option<String> {     // c:72
    if af == libc::AF_INET && addr_bytes.len() >= 4 {
        let v4 = std::net::Ipv4Addr::new(addr_bytes[0], addr_bytes[1], addr_bytes[2], addr_bytes[3]);
        Some(v4.to_string())
    } else if af == libc::AF_INET6 && addr_bytes.len() >= 16 {
        let mut octets = [0u8; 16];
        octets.copy_from_slice(&addr_bytes[..16]);
        Some(std::net::Ipv6Addr::from(octets).to_string())
    } else {
        None                                                              // c:88 NULL
    }
}

/// Port of `zsh_inet_aton()` from `Src/Modules/tcp.c:103`.
pub fn zsh_inet_aton(src: &str) -> Option<u32> {                         // c:103
    src.parse::<std::net::Ipv4Addr>().ok().map(|a| u32::from(a).to_be())
}

/// Port of `zsh_inet_pton()` from `Src/Modules/tcp.c:122`. Wraps
/// libc inet_pton(3) — parses an IP-presentation string into the
/// network-byte-order bytes. Returns 1 / 0 / -1 per C.
pub fn zsh_inet_pton(af: i32, src: &str, dst: &mut [u8]) -> i32 {        // c:122
    if af == libc::AF_INET {
        match src.parse::<std::net::Ipv4Addr>() {
            Ok(v4) if dst.len() >= 4 => {
                dst[..4].copy_from_slice(&v4.octets());
                1
            }
            _ => 0,
        }
    } else if af == libc::AF_INET6 {
        match src.parse::<std::net::Ipv6Addr>() {
            Ok(v6) if dst.len() >= 16 => {
                dst[..16].copy_from_slice(&v6.octets());
                1
            }
            _ => 0,
        }
    } else {
        -1
    }
}

/// Port of `zsh_gethostbyname2()` from `Src/Modules/tcp.c:146`.
pub fn zsh_gethostbyname2(name: &str, _af: i32) -> Vec<String> {         // c:146
    use std::net::ToSocketAddrs;
    format!("{}:0", name)
        .to_socket_addrs()
        .map(|iter| iter.map(|sa| sa.ip().to_string()).collect())
        .unwrap_or_default()
}

/// Port of `zsh_getipnodebyname()` from `Src/Modules/tcp.c:170`.
pub fn zsh_getipnodebyname(name: &str, af: i32) -> Vec<String> {         // c:170
    zsh_gethostbyname2(name, af)
}

/// Port of `freehostent()` from `Src/Modules/tcp.c:198`. C body is
/// a no-op (UNUSED `struct hostent *ptr`).
pub fn freehostent() {                                                   // c:198
    // c:200 — empty body.
}

/// Port of `zts_alloc()` from `Src/Modules/tcp.c:215`. Allocates a
/// fresh Tcp_session, initialises `fd = -1` + `flags = ztflags`,
/// and inserts it into the `ztcp_sessions` list. Returns the index
/// (proxy for the C pointer return).
pub fn zts_alloc(ztflags: i32) -> usize {                                // c:215
    ZTCP_SESSIONS.with(|s| {
        let mut sessions = s.borrow_mut();
        let idx = sessions.len();
        sessions.push(Tcp_session {                                      // c:218 zshcalloc
            fd: -1,                                                      // c:220 sess->fd = -1
            sock: TcpSockaddr::default(),
            peer: TcpSockaddr::default(),
            flags: ztflags,                                              // c:221 sess->flags = ztflags
        });
        idx                                                              // c:226 return sess
    })
}

/// Port of `tcp_socket()` from `Src/Modules/tcp.c:231`. Allocates a
/// session via zts_alloc, then opens a real socket via `socket(2)`
/// and registers the fd in the shell-wide fdtable as `FDT_MODULE`.
pub fn tcp_socket(domain: i32, ty: i32, protocol: i32, ztflags: i32) -> RawFd {  // c:231
    let idx = zts_alloc(ztflags);                                        // c:235
    let fd = unsafe { libc::socket(domain, ty, protocol) };              // c:238
    if fd >= 0 {
        ZTCP_SESSIONS.with(|s| {
            if let Some(sess) = s.borrow_mut().get_mut(idx) {
                sess.fd = fd;
            }
        });
        // c:241 — `addmodulefd(sess->fd, FDT_MODULE);`
        crate::ported::utils::addmodulefd(fd);
    }
    fd
}

/// Port of `ztcp_free_session()` from `Src/Modules/tcp.c:245`.
pub fn ztcp_free_session(_idx: usize) -> i32 {                           // c:245
    // c:248 — `zfree(sess, ...);` — Rust drop handles via zts_delete.
    0                                                                    // c:250
}

/// Port of `zts_delete()` from `Src/Modules/tcp.c:253`. Removes a
/// session from the linked list and frees it.
pub fn zts_delete(fd: RawFd) -> i32 {                                    // c:253
    ZTCP_SESSIONS.with(|s| {
        let mut sessions = s.borrow_mut();
        let pos = sessions.iter().position(|sess| sess.fd == fd);        // c:259
        match pos {
            Some(i) => {
                sessions.remove(i);                                      // c:266 remnode
                0                                                        // c:268
            }
            None => 1,                                                   // c:262 not found
        }
    })
}

/// Port of `zts_byfd()` from `Src/Modules/tcp.c:271`. Linear scan.
/// Returns the session's flags (or None if not found).
pub fn zts_byfd(fd: RawFd) -> Option<i32> {                              // c:271
    ZTCP_SESSIONS.with(|s| {
        s.borrow().iter().find(|sess| sess.fd == fd).map(|sess| sess.flags)  // c:275-278
    })
}

/// Port of `tcp_cleanup()` from `Src/Modules/tcp.c:283`. Walks the
/// session list and closes every fd.
pub fn tcp_cleanup() {                                                   // c:283
    ZTCP_SESSIONS.with(|s| {
        let mut sessions = s.borrow_mut();
        for sess in sessions.drain(..) {                                 // c:286-289
            if sess.fd >= 0 {
                unsafe { libc::close(sess.fd); }
            }
        }
    });
}

/// Port of `tcp_close()` from `Src/Modules/tcp.c:295`. Closes the
/// session's fd and removes it from the list.
pub fn tcp_close(fd: RawFd) -> i32 {                                     // c:295
    if fd < 0 { return -1; }
    let r = unsafe { libc::close(fd) };
    let _ = zts_delete(fd);
    if r < 0 { -1 } else { 0 }
}

/// Port of `tcp_connect()` from `Src/Modules/tcp.c:316`. Wraps
/// `socket(2)` + `connect(2)` for the connect path of `ztcp host
/// port`.
pub fn tcp_connect(host: &str, port: u16) -> std::io::Result<RawFd> {    // c:316
    use std::net::ToSocketAddrs;
    use std::os::unix::io::IntoRawFd;
    let addrs: Vec<_> = format!("{}:{}", host, port)
        .to_socket_addrs()?
        .collect();
    if addrs.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "host resolution failure",
        ));
    }
    for addr in addrs {
        let timeout = std::time::Duration::from_secs(10);
        if let Ok(stream) = std::net::TcpStream::connect_timeout(&addr, timeout) {
            let fd = stream.into_raw_fd();
            // c:241-equivalent — register in fdtable.
            crate::ported::utils::addmodulefd(fd);
            return Ok(fd);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "connection failed",
    ))
}

/// Port of `bin_ztcp()` from `Src/Modules/tcp.c:342`. The `ztcp`
/// builtin entry: parses `-l` (listen), `-a` (accept), `-c` (close),
/// `-d FD`, `-f` (force), `-L` (list), `-t` (test), `-v` (verbose)
/// flags + dispatch.
///
/// **Approximation:** the full bin_ztcp body is 350+ lines. Rust
/// port currently provides the dispatch skeleton — close-by-fd
/// path + list path. Full faithful port pending.
pub fn bin_ztcp(args: &[&str], ops: &[bool; 256]) -> i32 {               // c:342
    if ops[b'L' as usize] {                                              // c: -L list
        ZTCP_SESSIONS.with(|s| {
            for sess in s.borrow().iter() {
                println!("{}", sess.fd);
            }
        });
        return 0;
    }
    if ops[b'c' as usize] {                                              // c: -c close
        if args.is_empty() {
            tcp_cleanup();
            return 0;
        }
        // close specific fd
        let mut err = 0;
        for arg in args {
            if let Ok(fd) = arg.parse::<RawFd>() {
                if tcp_close(fd) != 0 { err = 1; }
            } else { err = 1; }
        }
        return err;
    }
    if ops[b'l' as usize] {                                              // c: -l listen
        // Listen path; full port pending.
        return 0;
    }
    if args.len() == 2 {                                                 // c: ztcp host port (connect)
        let host = args[0];
        if let Ok(port) = args[1].parse::<u16>() {
            return match tcp_connect(host, port) {
                Ok(_) => 0,
                Err(_) => 1,
            };
        }
    }
    0
}

/// Port of `setup_()` from `Src/Modules/tcp.c:714`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:714
    0                                                                    // c:717
}

/// Port of `features_()` from `Src/Modules/tcp.c:721`.
pub fn features_() -> i32 {                                              // c:721
    0                                                                    // c:725
}

/// Port of `enables_()` from `Src/Modules/tcp.c:729`.
pub fn enables_() -> i32 {                                               // c:729
    0                                                                    // c:732
}

/// Port of `boot_()` from `Src/Modules/tcp.c:736`. C body installs
/// the at-exit `tcp_cleanup` hook.
pub fn boot_() -> i32 {                                                  // c:736
    0                                                                    // c:740
}

/// Port of `cleanup_()` from `Src/Modules/tcp.c:745`. C body is
/// `tcp_cleanup(); return setfeatureenables(...);`.
pub fn cleanup_() -> i32 {                                               // c:745
    tcp_cleanup();                                                       // c:748
    0                                                                    // c:751
}

/// Port of `finish_()` from `Src/Modules/tcp.c:754`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:754
    0                                                                    // c:757
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zts_alloc_creates_session_with_default_fd() {
        let _ = zts_alloc(ZTCP_LISTEN);
        ZTCP_SESSIONS.with(|s| {
            let sessions = s.borrow();
            assert!(!sessions.is_empty());
            let last = sessions.last().unwrap();
            assert_eq!(last.fd, -1);
            assert_eq!(last.flags, ZTCP_LISTEN);
        });
    }

    #[test]
    fn inet_ntop_v4_works() {
        let bytes = [127u8, 0, 0, 1];
        assert_eq!(zsh_inet_ntop(libc::AF_INET, &bytes).as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn inet_pton_v4_works() {
        let mut buf = [0u8; 4];
        assert_eq!(zsh_inet_pton(libc::AF_INET, "127.0.0.1", &mut buf), 1);
        assert_eq!(buf, [127, 0, 0, 1]);
    }

    #[test]
    fn inet_pton_invalid_returns_zero() {
        let mut buf = [0u8; 4];
        assert_eq!(zsh_inet_pton(libc::AF_INET, "bad-ip", &mut buf), 0);
    }
}

// ShellExecutor::bin_ztcp shim — adapts `&[String]` argv + parses
// flags inline matching the BUILTIN spec at tcp.c:710 ("acdflLtvz").
impl crate::ported::exec::ShellExecutor {
    pub(crate) fn bin_ztcp(&mut self, args: &[String]) -> i32 {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut ops = [false; 256];
        let mut positional: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < argv.len() {
            let a = argv[i];
            if let Some(rest) = a.strip_prefix('-') {
                for ch in rest.chars() {
                    if ch.is_ascii_alphabetic() { ops[ch as u8 as usize] = true; }
                }
            } else {
                positional.push(a);
            }
            i += 1;
        }
        bin_ztcp(&positional, &ops)
    }
}
