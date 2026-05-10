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
use crate::ported::modules::tcp_h::tcp_sockaddr;
use crate::ported::modules::tcp_h::tcp_session;
use crate::ported::modules::tcp_h::ZTCP_LISTEN;
use crate::ported::modules::tcp_h::ZTCP_INBOUND;
use crate::ported::modules::tcp_h::ZTCP_ZFTP;

impl Default for tcp_sockaddr {
    fn default() -> Self {
        Self { a: unsafe { std::mem::zeroed() } }
    }
}

impl Default for tcp_session {
    fn default() -> Self {
        Self {
            fd: -1,
            sock: tcp_sockaddr::default(),
            peer: tcp_sockaddr::default(),
            flags: 0,
        }
    }
}

// File-static `ztcp_sessions` linked list — per PORT_PLAN Phase 2
// bucket-1, ported as a thread_local Vec.
thread_local! {
    /// Port of file-static `ztcp_sessions` from `Src/Modules/tcp.c`.
    static ZTCP_SESSIONS: std::cell::RefCell<Vec<tcp_session>> = const {
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
pub fn zsh_gethostbyname2(name: &str, af: i32) -> Vec<[u8; 4]> {         // c:146
    // C body wraps gethostbyname2(name, af); when AF_INET6 is unused
    // it falls back to gethostbyname(name). The relevant payload is
    // the `h_addr_list` array of in_addr/in6_addr bytes. For the
    // current AF_INET-only call path we return 4-byte records.
    use std::net::ToSocketAddrs;
    let mut out = Vec::new();
    if af == libc::AF_INET {                                             // c:148
        if let Ok(iter) = format!("{}:0", name).to_socket_addrs() {
            for sa in iter {
                if let std::net::SocketAddr::V4(v4) = sa {
                    out.push(v4.ip().octets());
                }
            }
        }
    }
    out
}

/// Port of `zsh_getipnodebyname()` from `Src/Modules/tcp.c:170`.
/// C body falls through to `zsh_gethostbyname2(name, af)` and returns
/// its `hostent`. Rust returns the AF_INET address list directly.
pub fn zsh_getipnodebyname(name: &str, af: i32) -> Vec<[u8; 4]> {        // c:170
    zsh_gethostbyname2(name, af)                                         // c:190
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
        sessions.push(tcp_session {                                      // c:218 zshcalloc
            fd: -1,                                                      // c:220 sess->fd = -1
            sock: tcp_sockaddr::default(),
            peer: tcp_sockaddr::default(),
            flags: ztflags,                                              // c:221 sess->flags = ztflags
        });
        idx                                                              // c:226 return sess
    })
}

// =====================================================================
// !!! WARNING: RUST-ONLY HELPERS — NO DIRECT C COUNTERPART !!!
// =====================================================================
//
// `sess_get` and `sess_with` DO NOT EXIST as functions in
// `Src/Modules/tcp.c`. The C source dereferences `Tcp_session sess`
// (a struct pointer) directly to read or write fields:
//
//     sess->fd = fd;
//     if (sess->flags & ZTCP_LISTEN) ...
//
// Rust's borrow checker won't let us hand out a long-lived reference
// to a slot inside the thread_local `ZTCP_SESSIONS` Vec without a
// borrow guard, so the Rust port wraps each field touch in a
// closure. Each call to `sess_with(idx, |s| { s.field = x })` maps
// 1:1 to a C `sess->field = x;` — they are NOT new policy, only an
// adapter for the storage shape (Vec<tcp_session> vs linked list).
//
// !!! Do NOT use these for any state that the C source doesn't
// already touch via `Tcp_session`. They are a borrow-checker
// adapter, not a new abstraction. !!!
// =====================================================================
//
// C `Tcp_session` is `struct tcp_session *` (a pointer into the
// `ztcp_sessions` linked list). Rust models the same: a handle that
// indexes into the thread-local `ZTCP_SESSIONS` Vec. NULL → None.
type TcpSessionHandle = Option<usize>;

/// !!! RUST-ONLY HELPER — see WARNING block above. Equivalent to
/// the C expression `sess->FIELD` (read).
fn sess_get<R, F: FnOnce(&tcp_session) -> R>(idx: usize, f: F) -> R {
    ZTCP_SESSIONS.with(|s| {
        let g = s.borrow();
        f(&g[idx])
    })
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Equivalent to
/// the C statement `sess->FIELD = X;` (write).
fn sess_with<F: FnOnce(&mut tcp_session)>(idx: usize, f: F) {
    ZTCP_SESSIONS.with(|s| {
        let mut g = s.borrow_mut();
        f(&mut g[idx])
    });
}

/// Port of `tcp_socket()` from `Src/Modules/tcp.c:231`.
/// C body (c:235-243):
/// ```c
/// Tcp_session sess = zts_alloc(ztflags);
/// sess->fd = socket(domain, type, protocol);
/// addmodulefd(sess->fd, FDT_MODULE);
/// return sess;
/// ```
pub fn tcp_socket(domain: i32, ty: i32, protocol: i32, ztflags: i32) -> TcpSessionHandle {  // c:231
    let idx = zts_alloc(ztflags);                                        // c:235
    let fd = unsafe { libc::socket(domain, ty, protocol) };              // c:238
    sess_with(idx, |s| { s.fd = fd; });                                  // c:238 sess->fd = ...
    if fd >= 0 {
        crate::ported::utils::addmodulefd(fd);                           // c:241
    }
    Some(idx)                                                            // c:243 return sess
}

/// Port of `ztcp_free_session()` from `Src/Modules/tcp.c:245`.
/// In the Rust port the Vec drop handles `zfree(sess, ...)`.
pub fn ztcp_free_session(_idx: usize) -> i32 {                           // c:245
    0                                                                    // c:250
}

/// Port of `zts_delete()` from `Src/Modules/tcp.c:253`. Removes a
/// session from the list and frees its slot. Returns 0 on success,
/// 1 if the fd has no matching session.
pub fn zts_delete(fd: RawFd) -> i32 {                                    // c:253
    ZTCP_SESSIONS.with(|s| {
        let mut sessions = s.borrow_mut();
        let pos = sessions.iter().position(|sess| sess.fd == fd);        // c:259
        match pos {
            Some(i) => {                                                 // c:266 remnode + zfree
                sessions.remove(i);
                0                                                        // c:268
            }
            None => 1,                                                   // c:262 not found
        }
    })
}

/// Port of `zts_byfd()` from `Src/Modules/tcp.c:271`. Linear scan.
/// C returns `Tcp_session` (pointer to the session) or NULL. Rust
/// returns the index handle or None.
pub fn zts_byfd(fd: RawFd) -> TcpSessionHandle {                         // c:271
    ZTCP_SESSIONS.with(|s| {
        s.borrow().iter().position(|sess| sess.fd == fd)                 // c:275-278
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

/// Port of `tcp_close()` from `Src/Modules/tcp.c:295`. Takes a session
/// pointer (Rust handle), closes its fd, and removes it from the list.
/// C body (c:298-313):
/// ```c
/// int err = -1;
/// if (sess) {
///     if (sess->fd != -1) {
///         err = zclose(sess->fd);
///         if (err) zwarn("connection close failed: %e", errno);
///     }
///     zts_delete(sess);
///     return err;
/// }
/// return 0;
/// ```
pub fn tcp_close(sess: TcpSessionHandle) -> i32 {                        // c:295
    if let Some(idx) = sess {                                            // c:298
        let fd = sess_get(idx, |s| s.fd);
        let mut err = -1;
        if fd != -1 {                                                    // c:301
            err = unsafe { libc::close(fd) };                            // c:303
            if err != 0 {
                crate::ported::utils::zwarn(&format!(
                    "connection close failed: {}",
                    std::io::Error::last_os_error()));
            }
        }
        // c:309 — zts_delete(sess); takes session by *pointer*. Rust
        // resolves it back to the fd-indexed remove call.
        let _ = zts_delete(fd);
        return err;                                                      // c:311
    }
    0                                                                    // c:313 — NULL sess: noop
}

/// Port of `tcp_connect()` from `Src/Modules/tcp.c:316`. C body
/// (c:319-340):
/// ```c
/// sess->peer.in.sin_family = zhost->h_addrtype;
/// sess->peer.in.sin_port   = d_port;
/// memcpy(&sess->peer.in.sin_addr, addr, zhost->h_length);
/// return connect(sess->fd, (struct sockaddr *)&sess->peer.in,
///                sizeof(struct sockaddr_in));
/// ```
pub fn tcp_connect(sess: TcpSessionHandle, addr: &[u8; 4], d_port: u16) -> i32 { // c:316
    let idx = match sess { Some(i) => i, None => return -1 };
    let fd = sess_get(idx, |s| s.fd);
    let mut peer: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    peer.sin_family = libc::AF_INET as _;                                // c:319
    peer.sin_port = d_port;                                              // c:320
    peer.sin_addr.s_addr = u32::from_be_bytes(*addr).to_be();            // c:321 memcpy
    sess_with(idx, |s| { s.peer.in_ = peer; });
    unsafe {
        libc::connect(fd,                                                // c:323
            &peer as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t)
    }
}


/// Direct port of `bin_ztcp()` from `Src/Modules/tcp.c:342`. Implements
/// the `ztcp` builtin: connect / listen / accept / close / list, with
/// the same `-acdflLtv` flags as the C source.
#[allow(non_snake_case)]
pub fn bin_ztcp(nam: &str, args: &[String],
                ops: &crate::ported::zsh_h::options, _func: i32) -> i32 { // c:342
    use crate::ported::zsh_h::{OPT_ISSET, OPT_ARG};
    use crate::ported::utils::{zwarnnam, zerrnam};

    let mut err: i32 = 1;                                                // c:344
    let destport: u16;                                                   // c:344
    let mut force = 0i32;                                                // c:344
    let mut verbose = 0i32;                                              // c:344
    let mut test = 0i32;                                                 // c:344
    let mut targetfd: i32 = 0;                                           // c:344
    let mut len: libc::socklen_t;                                        // c:345 ZSOCKLEN_T
    let desthost: String;                                                // c:346
    // c:347 — `const char *localname, *remotename;` declared at top
    // but only ever assigned + read inside the list-all loop; the
    // Rust port inlines them as block-locals at that site.
    let mut sess: TcpSessionHandle = None;                               // c:351

    if OPT_ISSET(ops, b'f') { force = 1; }                               // c:353-354
    if OPT_ISSET(ops, b'v') { verbose = 1; }                             // c:356-357
    if OPT_ISSET(ops, b't') { test = 1; }                                // c:359-360

    if OPT_ISSET(ops, b'd') {                                            // c:362
        let darg = OPT_ARG(ops, b'd').unwrap_or("");
        targetfd = darg.parse::<i32>().unwrap_or(0);                     // c:363 atoi
        if targetfd == 0 {                                               // c:364
            zwarnnam(nam, &format!("{} is an invalid argument to -d", darg));
            return 1;                                                    // c:366
        }
    }

    if OPT_ISSET(ops, b'c') {                                            // c:371
        if args.is_empty() {                                             // c:372
            tcp_cleanup();                                               // c:373
        } else {
            targetfd = args[0].parse::<i32>().unwrap_or(0);              // c:376 atoi
            sess = zts_byfd(targetfd);                                   // c:377
            if targetfd == 0 {                                           // c:378
                zwarnnam(nam, &format!("{} is an invalid argument to -c", args[0]));
                return 1;                                                // c:380
            }
            if let Some(sidx) = sess {                                   // c:384
                let flags = sess_get(sidx, |s| s.flags);
                if (flags & ZTCP_ZFTP) != 0 && force == 0 {              // c:386
                    zwarnnam(nam, "use -f to force closure of a zftp control connection");
                    return 1;                                            // c:388
                }
                tcp_close(sess);                                         // c:391
                return 0;                                                // c:392
            } else {                                                     // c:395
                zwarnnam(nam, &format!("fd {} not found in tcp table", args[0]));
                return 1;                                                // c:397
            }
        }
    } else if OPT_ISSET(ops, b'l') {                                     // c:400
        let lport: u16;                                                  // c:401
        if args.is_empty() {                                             // c:403
            zwarnnam(nam, "-l requires an argument");
            return 1;                                                    // c:405
        }
        // c:407 srv = getservbyname(args[0], "tcp");
        let srv = {
            let cname = std::ffi::CString::new(args[0].as_str()).ok();
            let cproto = std::ffi::CString::new("tcp").unwrap();
            cname.and_then(|c| {
                let p = unsafe { libc::getservbyname(c.as_ptr(), cproto.as_ptr()) };
                if p.is_null() { None } else { Some(unsafe { (*p).s_port } as u16) }
            })
        };
        lport = match srv {                                              // c:408-411
            Some(p) => p,                                                // c:410 srv->s_port
            None    => (args[0].parse::<u16>().unwrap_or(0)).to_be(),    // c:411 htons(atoi)
        };
        if lport == 0 {                                                  // c:412
            zwarnnam(nam, "bad service name or port number");
            return 1;                                                    // c:413
        }
        sess = tcp_socket(libc::PF_INET, libc::SOCK_STREAM, 0, ZTCP_LISTEN); // c:415
        if sess.is_none() {                                              // c:417
            zwarnnam(nam, "unable to allocate a TCP session slot");
            return 1;                                                    // c:419
        }
        let sidx = sess.unwrap();
        // c:421-423 SO_OOBINLINE
        let one: i32 = 1;
        let fd = sess_get(sidx, |s| s.fd);
        unsafe {
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_OOBINLINE,
                &one as *const _ as *const libc::c_void,
                std::mem::size_of::<i32>() as libc::socklen_t);
        }
        // c:425-429 — bind 0.0.0.0:lport
        let mut sin: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        sin.sin_family = libc::AF_INET as _;                             // c:432
        sin.sin_port = lport;                                            // c:433
        sin.sin_addr.s_addr = 0u32.to_be();                              // c:425 zsh_inet_aton("0.0.0.0")
        sess_with(sidx, |s| { s.sock.in_ = sin; });
        let r = unsafe {
            libc::bind(fd,                                               // c:436
                &sin as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t)
        };
        if r != 0 {                                                      // c:436
            zwarnnam(nam, &format!("could not bind to port {}: {}",      // c:440
                u16::from_be(lport), std::io::Error::last_os_error()));
            tcp_close(sess);                                             // c:441
            return 1;                                                    // c:442
        }
        if unsafe { libc::listen(fd, 1) } != 0 {                         // c:445
            zwarnnam(nam, &format!("could not listen on socket: {}",     // c:447
                std::io::Error::last_os_error()));
            tcp_close(sess);                                             // c:448
            return 1;                                                    // c:449
        }
        if targetfd != 0 {                                               // c:452
            let nfd = crate::ported::utils::redup(fd, targetfd);         // c:453
            sess_with(sidx, |s| { s.fd = nfd; });
        } else {
            // c:457 — `sess->fd = movefd(sess->fd);` move so no one
            // accidentally reads from it.
            let nfd = crate::ported::utils::movefd(fd);                  // c:457
            sess_with(sidx, |s| { s.fd = nfd; });
        }
        let nfd = sess_get(sidx, |s| s.fd);
        if nfd == -1 {                                                   // c:460
            zwarnnam(nam, &format!("cannot duplicate fd {}: {}", nfd,    // c:462
                std::io::Error::last_os_error()));
            tcp_close(sess);                                             // c:463
            return 1;                                                    // c:464
        }
        crate::ported::modules::ksh93::setiparam("REPLY", nfd as i64);   // c:467 setiparam_no_convert
        if verbose != 0 {                                                // c:469
            println!("{} listener is on fd {}",                          // c:470
                u16::from_be(lport), nfd);
        }
        return 0;                                                        // c:472
    } else if OPT_ISSET(ops, b'a') {                                     // c:475
        let lfd: i32;
        let rfd: i32;
        if args.is_empty() {                                             // c:478
            zwarnnam(nam, "-a requires an argument");
            return 1;                                                    // c:480
        }
        lfd = args[0].parse::<i32>().unwrap_or(0);                       // c:483
        if lfd == 0 {                                                    // c:485
            zwarnnam(nam, "invalid numerical argument");
            return 1;                                                    // c:487
        }
        sess = zts_byfd(lfd);                                            // c:490
        if sess.is_none() {                                              // c:491
            zwarnnam(nam, &format!("fd {} is not registered as a tcp connection",
                args[0]));
            return 1;                                                    // c:493
        }
        let flags = sess_get(sess.unwrap(), |s| s.flags);
        if (flags & ZTCP_LISTEN) == 0 {                                  // c:496
            zwarnnam(nam, "tcp connection not a listener");
            return 1;                                                    // c:499
        }
        if test != 0 {                                                   // c:502
            // c:506-512 — HAVE_POLL branch
            let mut pfd = libc::pollfd { fd: lfd, events: libc::POLLIN, revents: 0 };
            let ret = unsafe { libc::poll(&mut pfd, 1, 0) };             // c:509
            if ret == 0 { return 1; }                                    // c:510
            else if ret == -1 {                                          // c:511
                zwarnnam(nam, &format!("poll error: {}",                 // c:513
                    std::io::Error::last_os_error()));
                return 1;                                                // c:514
            }
        }
        sess = Some(zts_alloc(ZTCP_INBOUND));                            // c:540
        let sidx = sess.unwrap();
        let mut peer: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t; // c:542
        loop {                                                           // c:543
            let r = unsafe { libc::accept(lfd,
                &mut peer as *mut _ as *mut libc::sockaddr,
                &mut len as *mut _) };
            if r >= 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)
                || crate::ported::utils::errflag() != 0 {
                rfd = r;
                break;
            }
        }
        sess_with(sidx, |s| { s.peer.in_ = peer; });
        if rfd == -1 {                                                   // c:547
            zwarnnam(nam, &format!("could not accept connection: {}",    // c:549
                std::io::Error::last_os_error()));
            tcp_close(sess);                                             // c:550
            return 1;                                                    // c:551
        }
        crate::ported::utils::addmodulefd(rfd);                          // c:555
        if targetfd != 0 {                                               // c:557
            let nfd = crate::ported::utils::redup(rfd, targetfd);        // c:558
            sess_with(sidx, |s| { s.fd = nfd; });
            if nfd < 0 {                                                 // c:559
                zerrnam(nam, &format!("could not duplicate socket fd to {}: {}",
                    targetfd, std::io::Error::last_os_error()));
                return 1;                                                // c:562
            }
        } else {
            sess_with(sidx, |s| { s.fd = rfd; });                        // c:566
        }
        let nfd = sess_get(sidx, |s| s.fd);
        crate::ported::modules::ksh93::setiparam("REPLY", nfd as i64);   // c:569 setiparam_no_convert
        if verbose != 0 {                                                // c:571
            println!("{} is on fd {}", u16::from_be(peer.sin_port), nfd); // c:572
        }
    } else {                                                             // c:574
        if args.is_empty() {                                             // c:576
            // c:578-616 — list-all path.
            ZTCP_SESSIONS.with(|s| {
                for sess in s.borrow().iter() {                          // c:579
                    if sess.fd != -1 {                                   // c:582
                        // c:587 — `inet_ntoa(sess->sock.in.sin_addr)` (libc).
                        let lname = {
                            let b = u32::from_be(unsafe { sess.sock.in_.sin_addr.s_addr }).to_be_bytes();
                            format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
                        };
                        // c:590 — `inet_ntoa(sess->peer.in.sin_addr)` (libc).
                        let pname = {
                            let b = u32::from_be(unsafe { sess.peer.in_.sin_addr.s_addr }).to_be_bytes();
                            format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
                        };
                        let lport = u16::from_be(unsafe { sess.sock.in_.sin_port });
                        let pport = u16::from_be(unsafe { sess.peer.in_.sin_port });
                        if OPT_ISSET(ops, b'L') {                        // c:592
                            let schar = if (sess.flags & ZTCP_ZFTP)   != 0 { 'Z' }   // c:595
                                   else if (sess.flags & ZTCP_LISTEN) != 0 { 'L' }   // c:597
                                   else if (sess.flags & ZTCP_INBOUND)!= 0 { 'I' }   // c:599
                                   else                                    { 'O' };  // c:601
                            println!("{} {} {} {} {} {}",                // c:603
                                sess.fd, schar, lname, lport, pname, pport);
                        } else {                                         // c:608
                            let arrow = if (sess.flags & ZTCP_LISTEN)  != 0 { "-<" }
                                  else if (sess.flags & ZTCP_INBOUND) != 0 { "<-" }
                                  else                                    { "->" };
                            let zftp = if (sess.flags & ZTCP_ZFTP) != 0 { " ZFTP" } else { "" };
                            println!("{}:{} {} {}:{} is on fd {}{}",      // c:609
                                lname, lport, arrow, pname, pport, sess.fd, zftp);
                        }
                    }
                }
            });
            return 0;                                                    // c:619
        } else if args.len() == 1 {                                      // c:620
            destport = (23u16).to_be();                                  // c:621 htons(23)
        } else {
            // c:624 srv = getservbyname(args[1], "tcp");
            let srv = {
                let cname = std::ffi::CString::new(args[1].as_str()).ok();
                let cproto = std::ffi::CString::new("tcp").unwrap();
                cname.and_then(|c| {
                    let p = unsafe { libc::getservbyname(c.as_ptr(), cproto.as_ptr()) };
                    if p.is_null() { None } else { Some(unsafe { (*p).s_port } as u16) }
                })
            };
            destport = match srv {                                       // c:625
                Some(p) => p,                                            // c:627
                None    => (args[1].parse::<u16>().unwrap_or(0)).to_be(), // c:629 htons(atoi)
            };
        }
        desthost = args[0].clone();                                      // c:632
        let zthost = zsh_getipnodebyname(&desthost, libc::AF_INET);      // c:634
        if zthost.is_empty() {                                           // c:635
            zwarnnam(nam, &format!("host resolution failure: {}", desthost));
            return 1;                                                    // c:638
        }
        sess = tcp_socket(libc::PF_INET, libc::SOCK_STREAM, 0, 0);       // c:642
        if sess.is_none() {                                              // c:644
            zwarnnam(nam, "unable to allocate a TCP session slot");
            return 1;                                                    // c:647
        }
        let sidx = sess.unwrap();
        let one: i32 = 1;
        let fd = sess_get(sidx, |s| s.fd);
        unsafe {                                                         // c:651-653 SO_OOBINLINE
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_OOBINLINE,
                &one as *const _ as *const libc::c_void,
                std::mem::size_of::<i32>() as libc::socklen_t);
        }
        if fd < 0 {                                                      // c:656
            zwarnnam(nam, &format!("socket creation failed: {}",         // c:658
                std::io::Error::last_os_error()));
            zts_delete(fd);                                              // c:660
            return 1;                                                    // c:661
        }
        for addr in &zthost {                                            // c:664
            // c:665 — h_length must be 4 for AF_INET; libc resolution
            // already guarantees this so no length check needed.
            loop {                                                       // c:667
                err = tcp_connect(sess, addr, destport);                 // c:669
                if err == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)
                    || crate::ported::utils::errflag() != 0 { break; }
            }
            if err == 0 { break; }
        }
        if err != 0 {                                                    // c:673
            zwarnnam(nam, &format!("connection failed: {}",              // c:675
                std::io::Error::last_os_error()));
            tcp_close(sess);                                             // c:676
            return 1;                                                    // c:677
        } else {                                                         // c:680
            if targetfd != 0 {                                           // c:681
                let nfd = crate::ported::utils::redup(fd, targetfd);     // c:682
                sess_with(sidx, |s| { s.fd = nfd; });
                if nfd < 0 {                                             // c:683
                    zerrnam(nam, &format!("could not duplicate socket fd to {}: {}",
                        targetfd, std::io::Error::last_os_error()));     // c:684
                    tcp_close(sess);                                     // c:686
                    return 1;                                            // c:687
                }
            }
            let nfd = sess_get(sidx, |s| s.fd);
            crate::ported::modules::ksh93::setiparam("REPLY", nfd as i64); // c:691 setiparam_no_convert
            if verbose != 0 {                                            // c:693
                println!("{}:{} is now on fd {}",                        // c:694
                    desthost, u16::from_be(destport), nfd);
            }
        }
    }
    let _ = len;                                                         // silence unused-binding when -l not taken
    0                                                                    // c:702
}

// =====================================================================
// static struct features module_features                            c:705 (tcp.c)
// =====================================================================

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::{features as features_t, module};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None, bn_size: 1,                                       // bintab[1]: ztcp
        cd_list: None, cd_size: 0,
        mf_list: None, mf_size: 0,
        pd_list: None, pd_size: 0,
        n_abstract: 0,
    }))
}

/// Port of `setup_()` from `Src/Modules/tcp.c:714`.
pub fn setup_(_m: *const module) -> i32 {                                    // c:714
    // C body c:716-717 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_()` from `Src/Modules/tcp.c:721`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());                    // c:723
    0                                                                    // c:725
}

/// Port of `enables_()` from `Src/Modules/tcp.c:729`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)                       // c:731
}

/// Port of `boot_()` from `Src/Modules/tcp.c:736`.
pub fn boot_(_m: *const module) -> i32 {                                     // c:736
    // C body c:738-739 — `ztcp_sessions = znewlinklist(); return 0`.
    //                    Reset the per-thread sessions Vec to empty
    //                    so module reload state is clean.
    ZTCP_SESSIONS.with(|s| s.borrow_mut().clear());                          // c:738
    0
}

/// Port of `cleanup_()` from `Src/Modules/tcp.c:745`.
/// C body: `tcp_cleanup(); return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    tcp_cleanup();                                                       // c:748
    setfeatureenables(m, module_features(), None)                       // c:751
}

/// Port of `finish_()` from `Src/Modules/tcp.c:754`.
pub fn finish_(_m: *const module) -> i32 {                                   // c:754
    // C body c:756-757 — `return 0`. Faithful empty-body port; the
    //                    actual session teardown happens in cleanup_.
    0
}

fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:ztcp".to_string()]
}
fn handlefeatures(m: *const module, f: &Mutex<features_t>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(getfeatureenables(m, f));
    } else if let Some(e) = enables.as_ref() {
        return setfeatureenables(m, f, Some(e));
    }
    0
}
fn getfeatureenables(_m: *const module, f: &Mutex<features_t>) -> Vec<i32> {
    let g = f.lock().unwrap();
    vec![0; (g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract) as usize]
}
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 { 0 }

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

// ShellExecutor::bin_ztcp shim — parses flags into the canonical
// `options` struct matching the BUILTIN spec at tcp.c:710
// ("acdflLtv") and invokes the C-faithful free-fn port.
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

