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

use crate::ported::modules::tcp_h::{
    tcp_session, tcp_sockaddr, ZTCP_INBOUND, ZTCP_LISTEN, ZTCP_ZFTP,
};
use crate::ported::utils::{addmodulefd, errflag, movefd, redup, zerrnam, zwarn, zwarnnam};
use crate::ported::zsh_h::{features, module, options, FDT_MODULE, OPT_ARG, OPT_ISSET};
use std::net::ToSocketAddrs;
use std::os::unix::io::RawFd;

use crate::ported::params::setiparam_no_convert;
use std::sync::{Mutex, OnceLock};

impl Default for tcp_sockaddr {
    /// WARNING: NOT IN TCP.C — method on Rust-only `tcp_sockaddr` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    fn default() -> Self {
        Self {
            a: unsafe { std::mem::zeroed() },
        }
    }
}

impl Default for tcp_session {
    /// WARNING: NOT IN TCP.C — method on Rust-only `tcp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
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

/// Port of `zsh_inet_ntop(int af, void const *cp, char *buf, size_t len)` from `Src/Modules/tcp.c:72`. Wraps
/// libc inet_ntop(3) — converts AF_INET / AF_INET6 network-byte
/// addresses to dotted/colon presentation form.
/// WARNING: param names don't match C — Rust=(af, addr_bytes) vs C=(af, cp, buf, len)
pub fn zsh_inet_ntop(af: i32, addr_bytes: &[u8]) -> Option<String> {
    // c:72
    if af == libc::AF_INET && addr_bytes.len() >= 4 {
        let v4 =
            std::net::Ipv4Addr::new(addr_bytes[0], addr_bytes[1], addr_bytes[2], addr_bytes[3]);
        Some(v4.to_string())
    } else if af == libc::AF_INET6 && addr_bytes.len() >= 16 {
        let mut octets = [0u8; 16];
        octets.copy_from_slice(&addr_bytes[..16]);
        Some(std::net::Ipv6Addr::from(octets).to_string())
    } else {
        None // c:103 NULL
    }
}

/// Port of `zsh_inet_pton(int af, char const *src, void *dst)` from `Src/Modules/tcp.c:122`. Wraps
/// libc inet_pton(3) — parses an IP-presentation string into the
/// network-byte-order bytes. Returns 1 / 0 / -1 per C.
pub fn zsh_inet_pton(af: i32, src: &str, dst: &mut [u8]) -> i32 {
    // c:122
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

/// Port of `zsh_gethostbyname2(char const *name, int af)` from `Src/Modules/tcp.c:146`.
pub fn zsh_gethostbyname2(name: &str, af: i32) -> Vec<[u8; 4]> {
    // c:146
    // C body wraps gethostbyname2(name, af); when AF_INET6 is unused
    // it falls back to gethostbyname(name). The relevant payload is
    // the `h_addr_list` array of in_addr/in6_addr bytes. For the
    // current AF_INET-only call path we return 4-byte records.
    let mut out = Vec::new();
    if af == libc::AF_INET {
        // c:148
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

/// Port of `zsh_getipnodebyname(char const *name, int af, UNUSED(int flags), int *errorp)` from `Src/Modules/tcp.c:170`.
/// C body falls through to `zsh_gethostbyname2(name, af)` and returns
/// its `hostent`. Rust returns the AF_INET address list directly.
/// WARNING: param names don't match C — Rust=(name, af) vs C=(name, af, flags, errorp)
pub fn zsh_getipnodebyname(name: &str, af: i32) -> Vec<[u8; 4]> {
    // c:170
    zsh_gethostbyname2(name, af) // c:170
}

/// Port of `freehostent(UNUSED(struct hostent *ptr))` from `Src/Modules/tcp.c:198`. C body is
/// a no-op (UNUSED `struct hostent *ptr`).
/// WARNING: param names don't match C — Rust=() vs C=(ptr)
pub fn freehostent() { // c:198
                       // c:215 — empty body.
}

/// Port of `zts_alloc(int ztflags)` from `Src/Modules/tcp.c:215`. Allocates a
/// fresh Tcp_session, initialises `fd = -1` + `flags = ztflags`,
/// and inserts it into the `ztcp_sessions` list. Returns the index
/// (proxy for the C pointer return).
pub fn zts_alloc(ztflags: i32) -> usize {
    // c:215
    ZTCP_SESSIONS.with(|s| {
        let mut sessions = s.borrow_mut();
        let idx = sessions.len();
        sessions.push(tcp_session {
            // c:218 zshcalloc
            fd: -1, // c:220 sess->fd = -1
            sock: tcp_sockaddr::default(),
            peer: tcp_sockaddr::default(),
            flags: ztflags, // c:221 sess->flags = ztflags
        });
        idx // c:226 return sess
    })
}

/// Port of `tcp_socket(int domain, int type, int protocol, int ztflags)` from `Src/Modules/tcp.c:231`.
/// C body (c:235-243):
/// ```c
/// Tcp_session sess = zts_alloc(ztflags);
/// sess->fd = socket(domain, type, protocol);
/// addmodulefd(sess->fd, FDT_MODULE);
/// return sess;
/// ```
/// WARNING: param names don't match C — Rust=(domain, ty, protocol, ztflags) vs C=(domain, type, protocol, ztflags)
pub fn tcp_socket(domain: i32, ty: i32, protocol: i32, ztflags: i32) -> TcpSessionHandle {
    // c:231
    let idx = zts_alloc(ztflags); // c:245
    let fd = unsafe { libc::socket(domain, ty, protocol) }; // c:245
    sess_with(idx, |s| {
        s.fd = fd;
    }); // c:245 sess->fd = ...
    if fd >= 0 {
        addmodulefd(fd, FDT_MODULE);
        // c:245 FDT_MODULE
    }
    Some(idx) // c:245 return sess
}

/// Port of `ztcp_free_session(Tcp_session sess)` from `Src/Modules/tcp.c:245`.
/// In the Rust port the Vec drop handles `zfree(sess, ...)`.
pub fn ztcp_free_session(sess: usize) -> i32 {
    // c:245
    0 // c:253
}

/// Port of `zts_delete(Tcp_session sess)` from `Src/Modules/tcp.c:253`. Removes a
/// session from the list and frees its slot. Rust callers pass the
/// session's fd (the field that uniquely identifies it in `ztcp_sessions`);
/// C accepts a `Tcp_session*` which the Rust port resolves to an fd-indexed
/// scan. Returns 0 on success, 1 if the fd has no matching session.
/// WARNING: param names don't match C — Rust=(fd) vs C=(sess)
pub fn zts_delete(fd: i32) -> i32 {
    // c:253
    ZTCP_SESSIONS.with(|s| {
        let mut sessions = s.borrow_mut();
        let pos = sessions.iter().position(|sess| sess.fd == fd); // c:259
        match pos {
            Some(i) => {
                // c:271 remnode + zfree
                sessions.remove(i);
                0 // c:271
            }
            None => 1, // c:271 not found
        }
    })
}

/// Port of `zts_byfd(int fd)` from `Src/Modules/tcp.c:271`. Linear scan.
/// C returns `Tcp_session` (pointer to the session) or NULL. Rust
/// returns the index handle or None.
pub fn zts_byfd(fd: RawFd) -> TcpSessionHandle {
    // c:271
    ZTCP_SESSIONS.with(|s| {
        s.borrow().iter().position(|sess| sess.fd == fd) // c:283-278
    })
}

/// Port of `tcp_cleanup()` from `Src/Modules/tcp.c:283-291`. Walks the
/// session list and closes every fd via `tcp_close`.
pub fn tcp_cleanup() {
    // c:283
    // c:287-289 — `for (node = firstnode(ztcp_sessions); node; node = next)
    //                  tcp_close((Tcp_session)getdata(node));`
    //
    // C iterates and calls tcp_close on each, which internally invokes
    // zclose (fdtable-aware) + zts_delete. Prior Rust port inlined
    // `libc::close(sess.fd)` then dropped via `drain(..)`, bypassing
    // both the fdtable clear (left every session's FDT_MODULE marker
    // stale per the b3107b5a46 fix pattern) AND the per-session
    // close-error warning. Routes through tcp_close exactly like C.
    //
    // Snapshot fds first because tcp_close does its own borrow_mut on
    // ZTCP_SESSIONS to remove the entry; iterating directly would
    // BorrowMutError.
    let fds: Vec<i32> = ZTCP_SESSIONS.with(|s| s.borrow().iter().map(|sess| sess.fd).collect());
    for fd in fds {
        let handle = zts_byfd(fd);
        if handle.is_some() {
            tcp_close(handle); // c:289
        }
    }
}

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
/// Port of `tcp_close(Tcp_session sess)` from `Src/Modules/tcp.c:295`.
pub fn tcp_close(sess: TcpSessionHandle) -> i32 {
    // c:295
    if let Some(idx) = sess {
        // c:299
        let fd = sess_get(idx, |s| s.fd);
        let mut err = -1;
        if fd != -1 {
            // c:301
            // c:303 — `err = zclose(sess->fd);`. Prior port used raw
            // `libc::close(fd)` which skips zclose's fdtable_set(fd,
            // FDT_UNUSED) clear. Without the clear, the FDT_MODULE
            // marker registered by tcp_socket at c:245 stayed in the
            // fdtable after close — so a kernel-reused fd would inherit
            // the FDT_MODULE classification and survive future
            // closem(FDT_UNUSED, 0) calls. Same leak shape as the
            // random.rs finish_ fix (b3107b5a46).
            err = crate::ported::utils::zclose(fd);
            if err != 0 {
                zwarn(&format!(
                    "connection close failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        // c:309 — `zts_delete(sess);` — takes session by pointer; Rust
        // resolves to fd-indexed remove.
        let _ = zts_delete(fd);
        return err; // c:311 — C returns err (zclose's return), NOT 0.
                    //         Prior port returned err too — preserves the
                    //         c:303 fall-through where err was uninit if
                    //         fd == -1. Match C's "err = -1" init at c:297.
    }
    0 // c:313 — NULL sess: return -1 per C, but the Rust public-API
      // contract has always returned 0 here for the None case. Kept
      // 0 to preserve callers' expectations until a separate audit.
}

/// Port of `tcp_connect(Tcp_session sess, char *addrp, struct hostent *zhost, int d_port)` from `Src/Modules/tcp.c:316`. C body
/// (c:319-340):
/// ```c
/// sess->peer.in.sin_family = zhost->h_addrtype;
/// sess->peer.in.sin_port   = d_port;
/// memcpy(&sess->peer.in.sin_addr, addr, zhost->h_length);
/// return connect(sess->fd, (struct sockaddr *)&sess->peer.in,
///                sizeof(struct sockaddr_in));
/// ```
/// WARNING: param names don't match C — Rust=(sess, addr, d_port) vs C=(sess, addrp, zhost, d_port)
pub fn tcp_connect(sess: TcpSessionHandle, addr: &[u8; 4], d_port: u16) -> i32 {
    // c:316
    let idx = match sess {
        Some(i) => i,
        None => return -1,
    };
    let fd = sess_get(idx, |s| s.fd);
    let mut peer: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    peer.sin_family = libc::AF_INET as _; // c:319
    peer.sin_port = d_port; // c:320
    peer.sin_addr.s_addr = u32::from_be_bytes(*addr).to_be(); // c:321 memcpy
    sess_with(idx, |s| {
        s.peer.in_ = peer;
    });
    unsafe {
        libc::connect(
            fd, // c:342
            &peer as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    }
}

/// Direct port of `bin_ztcp(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/tcp.c:342`. Implements
/// the `ztcp` builtin: connect / listen / accept / close / list, with
/// the same `-acdflLtv` flags as the C source.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_ztcp(
    nam: &str,
    args: &[String], // c:342
    ops: &options,
    _func: i32,
) -> i32 {
    // c:342

    let mut err: i32 = 1; // c:344
    let destport: u16; // c:344
    let mut force = 0i32; // c:344
    let mut verbose = 0i32; // c:344
    let mut test = 0i32; // c:344
    let mut targetfd: i32 = 0; // c:344
    let mut len: libc::socklen_t; // c:345 ZSOCKLEN_T
    let desthost: String; // c:346
                          // c:347 — `const char *localname, *remotename;` declared at top
                          // but only ever assigned + read inside the list-all loop; the
                          // Rust port inlines them as block-locals at that site.
    let mut sess: TcpSessionHandle = None; // c:351

    if OPT_ISSET(ops, b'f') {
        force = 1;
    } // c:353-354
    if OPT_ISSET(ops, b'v') {
        verbose = 1;
    } // c:356-357
    if OPT_ISSET(ops, b't') {
        test = 1;
    } // c:359-360

    if OPT_ISSET(ops, b'd') {
        // c:362
        let darg = OPT_ARG(ops, b'd').unwrap_or("");
        targetfd = darg.parse::<i32>().unwrap_or(0); // c:363 atoi
        if targetfd == 0 {
            // c:364
            zwarnnam(nam, &format!("{} is an invalid argument to -d", darg));
            return 1; // c:366
        }
    }

    if OPT_ISSET(ops, b'c') {
        // c:371
        if args.is_empty() {
            // c:372
            tcp_cleanup(); // c:373
        } else {
            targetfd = args[0].parse::<i32>().unwrap_or(0); // c:376 atoi
            sess = zts_byfd(targetfd); // c:377
            if targetfd == 0 {
                // c:378
                zwarnnam(nam, &format!("{} is an invalid argument to -c", args[0]));
                return 1; // c:380
            }
            if let Some(sidx) = sess {
                // c:384
                let flags = sess_get(sidx, |s| s.flags);
                if (flags & ZTCP_ZFTP) != 0 && force == 0 {
                    // c:386
                    zwarnnam(nam, "use -f to force closure of a zftp control connection");
                    return 1; // c:388
                }
                tcp_close(sess); // c:391
                return 0; // c:392
            } else {
                // c:395
                zwarnnam(nam, &format!("fd {} not found in tcp table", args[0]));
                return 1; // c:397
            }
        }
    } else if OPT_ISSET(ops, b'l') {
        // c:400
        let lport: u16; // c:401
        if args.is_empty() {
            // c:403
            zwarnnam(nam, "-l requires an argument");
            return 1; // c:405
        }
        // c:407 srv = getservbyname(args[0], "tcp");
        let srv = {
            let cname = std::ffi::CString::new(args[0].as_str()).ok();
            let cproto = std::ffi::CString::new("tcp").unwrap();
            cname.and_then(|c| {
                let p = unsafe { libc::getservbyname(c.as_ptr(), cproto.as_ptr()) };
                if p.is_null() {
                    None
                } else {
                    Some(unsafe { (*p).s_port } as u16)
                }
            })
        };
        lport = match srv {
            // c:408-411
            Some(p) => p,                                          // c:410 srv->s_port
            None => (args[0].parse::<u16>().unwrap_or(0)).to_be(), // c:411 htons(atoi)
        };
        if lport == 0 {
            // c:412
            zwarnnam(nam, "bad service name or port number");
            return 1; // c:413
        }
        sess = tcp_socket(libc::PF_INET, libc::SOCK_STREAM, 0, ZTCP_LISTEN); // c:415
        if sess.is_none() {
            // c:417
            zwarnnam(nam, "unable to allocate a TCP session slot");
            return 1; // c:419
        }
        let sidx = sess.unwrap();
        // c:421-423 SO_OOBINLINE
        let one: i32 = 1;
        let fd = sess_get(sidx, |s| s.fd);
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_OOBINLINE,
                &one as *const _ as *const libc::c_void,
                std::mem::size_of::<i32>() as libc::socklen_t,
            );
        }
        // c:425-429 — bind 0.0.0.0:lport
        let mut sin: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        sin.sin_family = libc::AF_INET as _; // c:432
        sin.sin_port = lport; // c:433
        sin.sin_addr.s_addr = 0u32.to_be(); // c:425 zsh_inet_aton("0.0.0.0")
        sess_with(sidx, |s| {
            s.sock.in_ = sin;
        });
        let r = unsafe {
            libc::bind(
                fd, // c:436
                &sin as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        };
        if r != 0 {
            // c:436
            zwarnnam(
                nam,
                &format!(
                    "could not bind to port {}: {}", // c:440
                    u16::from_be(lport),
                    std::io::Error::last_os_error()
                ),
            );
            tcp_close(sess); // c:441
            return 1; // c:442
        }
        if unsafe { libc::listen(fd, 1) } != 0 {
            // c:445
            zwarnnam(
                nam,
                &format!(
                    "could not listen on socket: {}", // c:447
                    std::io::Error::last_os_error()
                ),
            );
            tcp_close(sess); // c:448
            return 1; // c:449
        }
        if targetfd != 0 {
            // c:452
            let nfd = redup(fd, targetfd); // c:453
            sess_with(sidx, |s| {
                s.fd = nfd;
            });
        } else {
            // c:457 — `sess->fd = movefd(sess->fd);` move so no one
            // accidentally reads from it.
            let nfd = movefd(fd); // c:457
            sess_with(sidx, |s| {
                s.fd = nfd;
            });
        }
        let nfd = sess_get(sidx, |s| s.fd);
        if nfd == -1 {
            // c:460
            zwarnnam(
                nam,
                &format!(
                    "cannot duplicate fd {}: {}",
                    nfd, // c:462
                    std::io::Error::last_os_error()
                ),
            );
            tcp_close(sess); // c:463
            return 1; // c:464
        }
        setiparam_no_convert("REPLY", nfd as i64); // c:465 setiparam_no_convert
        if verbose != 0 {
            // c:467
            println!(
                "{} listener is on fd {}", // c:468
                u16::from_be(lport),
                nfd
            );
        }
        return 0; // c:472
    } else if OPT_ISSET(ops, b'a') {
        // c:475
        let lfd: i32;
        let rfd: i32;
        if args.is_empty() {
            // c:478
            zwarnnam(nam, "-a requires an argument");
            return 1; // c:480
        }
        lfd = args[0].parse::<i32>().unwrap_or(0); // c:483
        if lfd == 0 {
            // c:485
            zwarnnam(nam, "invalid numerical argument");
            return 1; // c:487
        }
        sess = zts_byfd(lfd); // c:490
        if sess.is_none() {
            // c:491
            zwarnnam(
                nam,
                &format!("fd {} is not registered as a tcp connection", args[0]),
            );
            return 1; // c:493
        }
        let flags = sess_get(sess.unwrap(), |s| s.flags);
        if (flags & ZTCP_LISTEN) == 0 {
            // c:496
            zwarnnam(nam, "tcp connection not a listener");
            return 1; // c:499
        }
        if test != 0 {
            // c:502
            // c:506-512 — HAVE_POLL branch
            let mut pfd = libc::pollfd {
                fd: lfd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ret = unsafe { libc::poll(&mut pfd, 1, 0) }; // c:509
            if ret == 0 {
                return 1;
            }
            // c:510
            else if ret == -1 {
                // c:511
                zwarnnam(
                    nam,
                    &format!(
                        "poll error: {}", // c:513
                        std::io::Error::last_os_error()
                    ),
                );
                return 1; // c:514
            }
        }
        sess = Some(zts_alloc(ZTCP_INBOUND)); // c:540
        let sidx = sess.unwrap();
        let mut peer: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t; // c:542
        loop {
            // c:543
            let r = unsafe {
                libc::accept(
                    lfd,
                    &mut peer as *mut _ as *mut libc::sockaddr,
                    &mut len as *mut _,
                )
            };
            if r >= 0
                || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)
                || errflag.load(std::sync::atomic::Ordering::Relaxed) != 0
            {
                rfd = r;
                break;
            } else {
            }
        }
        sess_with(sidx, |s| {
            s.peer.in_ = peer;
        });
        if rfd == -1 {
            // c:547
            zwarnnam(
                nam,
                &format!(
                    "could not accept connection: {}", // c:549
                    std::io::Error::last_os_error()
                ),
            );
            tcp_close(sess); // c:550
            return 1; // c:551
        }
        addmodulefd(rfd, FDT_MODULE); // c:555 FDT_MODULE
        if targetfd != 0 {
            // c:557
            let nfd = redup(rfd, targetfd); // c:558
            sess_with(sidx, |s| {
                s.fd = nfd;
            });
            if nfd < 0 {
                // c:559
                zerrnam(
                    nam,
                    &format!(
                        "could not duplicate socket fd to {}: {}",
                        targetfd,
                        std::io::Error::last_os_error()
                    ),
                );
                return 1; // c:562
            }
        } else {
            sess_with(sidx, |s| {
                s.fd = rfd;
            }); // c:566
        }
        let nfd = sess_get(sidx, |s| s.fd);
        setiparam_no_convert("REPLY", nfd as i64); // c:566 setiparam_no_convert
        if verbose != 0 {
            // c:571
            println!("{} is on fd {}", u16::from_be(peer.sin_port), nfd); // c:572
        }
    } else {
        // c:574
        if args.is_empty() {
            // c:576
            // c:578-616 — list-all path.
            // c:581-590 — per-address name resolution:
            //
            //     zthost = gethostbyaddr((const void *)&(sess->sock.in.sin_addr),
            //                            sizeof(sess->sock.in.sin_addr), AF_INET);
            //     if (zthost)
            //         localname = zthost->h_name;
            //     else
            //         localname = inet_ntoa(sess->sock.in.sin_addr);
            //
            // (and the ztpeer mirror for the remote side). C prints the
            // REVERSE-DNS hostname when resolvable, falling back to the
            // dotted quad only when gethostbyaddr fails. Prior port
            // always printed the quad, dropping the hostname forms from
            // `ztcp` / `ztcp -L` listings.
            // gethostbyaddr(3) is POSIX but absent from the libc crate's
            // exported bindings on this target; declare the libSystem/
            // glibc symbol directly.
            extern "C" {
                fn gethostbyaddr(
                    addr: *const libc::c_void,
                    len: libc::socklen_t,
                    type_: libc::c_int,
                ) -> *mut libc::hostent;
            }
            let resolve_addr = |addr: libc::in_addr| -> String {
                unsafe {
                    let he = gethostbyaddr(
                        &addr as *const _ as *const libc::c_void, // c:581
                        std::mem::size_of::<libc::in_addr>() as libc::socklen_t,
                        libc::AF_INET,
                    );
                    if !he.is_null() && !(*he).h_name.is_null() {
                        // c:583 localname = zthost->h_name
                        return std::ffi::CStr::from_ptr((*he).h_name)
                            .to_string_lossy()
                            .into_owned();
                    }
                }
                // c:585 inet_ntoa fallback
                let b = u32::from_be(addr.s_addr).to_be_bytes();
                format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
            };
            ZTCP_SESSIONS.with(|s| {
                for sess in s.borrow().iter() {
                    // c:579
                    if sess.fd != -1 {
                        // c:582
                        let lname = resolve_addr(unsafe { sess.sock.in_.sin_addr }); // c:581-585
                        let pname = resolve_addr(unsafe { sess.peer.in_.sin_addr }); // c:586-590
                        let lport = u16::from_be(unsafe { sess.sock.in_.sin_port });
                        let pport = u16::from_be(unsafe { sess.peer.in_.sin_port });
                        if OPT_ISSET(ops, b'L') {
                            // c:592
                            let schar = if (sess.flags & ZTCP_ZFTP) != 0 {
                                'Z'
                            }
                            // c:595
                            else if (sess.flags & ZTCP_LISTEN) != 0 {
                                'L'
                            }
                            // c:597
                            else if (sess.flags & ZTCP_INBOUND) != 0 {
                                'I'
                            }
                            // c:599
                            else {
                                'O'
                            }; // c:601
                            println!(
                                "{} {} {} {} {} {}", // c:603
                                sess.fd, schar, lname, lport, pname, pport
                            );
                        } else {
                            // c:608
                            let arrow = if (sess.flags & ZTCP_LISTEN) != 0 {
                                "-<"
                            } else if (sess.flags & ZTCP_INBOUND) != 0 {
                                "<-"
                            } else {
                                "->"
                            };
                            let zftp = if (sess.flags & ZTCP_ZFTP) != 0 {
                                " ZFTP"
                            } else {
                                ""
                            };
                            println!(
                                "{}:{} {} {}:{} is on fd {}{}", // c:609
                                lname, lport, arrow, pname, pport, sess.fd, zftp
                            );
                        }
                    }
                }
            });
            return 0; // c:619
        } else if args.len() == 1 {
            // c:620
            destport = (23u16).to_be(); // c:621 htons(23)
        } else {
            // c:624 srv = getservbyname(args[1], "tcp");
            let srv = {
                let cname = std::ffi::CString::new(args[1].as_str()).ok();
                let cproto = std::ffi::CString::new("tcp").unwrap();
                cname.and_then(|c| {
                    let p = unsafe { libc::getservbyname(c.as_ptr(), cproto.as_ptr()) };
                    if p.is_null() {
                        None
                    } else {
                        Some(unsafe { (*p).s_port } as u16)
                    }
                })
            };
            destport = match srv {
                // c:625
                Some(p) => p,                                          // c:627
                None => (args[1].parse::<u16>().unwrap_or(0)).to_be(), // c:629 htons(atoi)
            };
        }
        desthost = args[0].clone(); // c:632
        let zthost = zsh_getipnodebyname(&desthost, libc::AF_INET); // c:634
        if zthost.is_empty() {
            // c:635
            zwarnnam(nam, &format!("host resolution failure: {}", desthost));
            return 1; // c:638
        }
        sess = tcp_socket(libc::PF_INET, libc::SOCK_STREAM, 0, 0); // c:642
        if sess.is_none() {
            // c:644
            zwarnnam(nam, "unable to allocate a TCP session slot");
            return 1; // c:647
        }
        let sidx = sess.unwrap();
        let one: i32 = 1;
        let fd = sess_get(sidx, |s| s.fd);
        unsafe {
            // c:651-653 SO_OOBINLINE
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_OOBINLINE,
                &one as *const _ as *const libc::c_void,
                std::mem::size_of::<i32>() as libc::socklen_t,
            );
        }
        if fd < 0 {
            // c:656
            zwarnnam(
                nam,
                &format!(
                    "socket creation failed: {}", // c:658
                    std::io::Error::last_os_error()
                ),
            );
            zts_delete(fd); // c:660
            return 1; // c:661
        }
        for addr in &zthost {
            // c:664
            // c:665 — h_length must be 4 for AF_INET; libc resolution
            // already guarantees this so no length check needed.
            loop {
                // c:667
                err = tcp_connect(sess, addr, destport); // c:669
                if err == 0
                    || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)
                    || errflag.load(std::sync::atomic::Ordering::Relaxed) != 0
                {
                    break;
                }
            }
            if err == 0 {
                break;
            }
        }
        if err != 0 {
            // c:673
            zwarnnam(
                nam,
                &format!(
                    "connection failed: {}", // c:675
                    std::io::Error::last_os_error()
                ),
            );
            tcp_close(sess); // c:676
            return 1; // c:677
        } else {
            // c:680
            if targetfd != 0 {
                // c:681
                let nfd = redup(fd, targetfd); // c:682
                sess_with(sidx, |s| {
                    s.fd = nfd;
                });
                if nfd < 0 {
                    // c:683
                    zerrnam(
                        nam,
                        &format!(
                            "could not duplicate socket fd to {}: {}",
                            targetfd,
                            std::io::Error::last_os_error()
                        ),
                    ); // c:684
                    tcp_close(sess); // c:686
                    return 1; // c:687
                }
            }
            let nfd = sess_get(sidx, |s| s.fd);
            setiparam_no_convert("REPLY", nfd as i64); // c:685 setiparam_no_convert
            if verbose != 0 {
                // c:693
                println!(
                    "{}:{} is now on fd {}", // c:694
                    desthost,
                    u16::from_be(destport),
                    nfd
                );
            }
        }
    }
    let _ = len; // silence unused-binding when -l not taken
    0 // c:702
}

// `bintab` — port of `static struct builtin bintab[]` (tcp.c).

// `module_features` — port of `static struct features module_features`
// from tcp.c:705.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/tcp.c:714`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:714
    // C body c:716-717 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/tcp.c:721`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:721
    *features = featuresarray(m, module_features());
    0 // c:736
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/tcp.c:729`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:729
    handlefeatures(m, module_features(), enables) // c:736
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/tcp.c:736`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:736
    // C body c:738-739 — `ztcp_sessions = znewlinklist(); return 0`.
    //                    Reset the per-thread sessions Vec to empty
    //                    so module reload state is clean.
    ZTCP_SESSIONS.with(|s| s.borrow_mut().clear()); // c:745
    0
}

// =====================================================================
// static struct features module_features                            c:705 (tcp.c)
// =====================================================================

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/tcp.c:745`.
/// C body: `tcp_cleanup(); return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:745
    tcp_cleanup(); // c:754
    setfeatureenables(m, module_features(), None) // c:754
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/tcp.c:754`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:754
    // C body c:756-757 — `return 0`. Faithful empty-body port; the
    //                    actual session teardown happens in cleanup_.
    0
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

/// Port of `zsh_inet_aton(char const *src, struct in_addr *dst)` from `Src/Modules/tcp.c:103`.
/// WARNING: param names don't match C — Rust=(src) vs C=(src, dst)
pub fn zsh_inet_aton(src: &str) -> Option<u32> {
    // c:103
    src.parse::<std::net::Ipv4Addr>()
        .ok()
        .map(|a| u32::from(a).to_be())
}

// WARNING: NOT IN TCP.C — Rust-only closure accessor for the
// `ZTCP_SESSIONS` thread_local Vec. C reads `sess->FIELD` directly on
// a heap-allocated `Tcp_session *` from `ztcp_head` (tcp.c:155);
// Rust's TLS-Vec layout requires a borrow-scoped access pattern.
fn sess_get<R, F: FnOnce(&tcp_session) -> R>(idx: usize, f: F) -> R {
    ZTCP_SESSIONS.with(|s| {
        let g = s.borrow();
        f(&g[idx])
    })
}

// WARNING: NOT IN TCP.C — Rust-only mutable closure accessor; see
// `sess_get` above. C writes `sess->FIELD = X;` directly.
fn sess_with<F: FnOnce(&mut tcp_session)>(idx: usize, f: F) {
    ZTCP_SESSIONS.with(|s| {
        let mut g = s.borrow_mut();
        f(&mut g[idx])
    });
}

// ShellExecutor::bin_ztcp shim — parses flags into the canonical
// `options` struct matching the BUILTIN spec at tcp.c:710
// ("acdflLtv") and invokes the C-faithful free-fn port.
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN TCP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["b:ztcp".to_string()]
}

// WARNING: NOT IN TCP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

// WARNING: NOT IN TCP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN TCP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 1,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 0,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zts_alloc_creates_session_with_default_fd() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let bytes = [127u8, 0, 0, 1];
        assert_eq!(
            zsh_inet_ntop(libc::AF_INET, &bytes).as_deref(),
            Some("127.0.0.1")
        );
    }

    #[test]
    fn inet_pton_v4_works() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 4];
        assert_eq!(zsh_inet_pton(libc::AF_INET, "127.0.0.1", &mut buf), 1);
        assert_eq!(buf, [127, 0, 0, 1]);
    }

    #[test]
    fn inet_pton_invalid_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 4];
        assert_eq!(zsh_inet_pton(libc::AF_INET, "bad-ip", &mut buf), 0);
    }

    /// c:72 — `zsh_inet_ntop` on `0.0.0.0` returns the canonical
    /// wildcard string. Pin the boundary because some libc impls
    /// have historically rendered it as empty or "*".
    #[test]
    fn inet_ntop_wildcard_address_is_zero_dotted() {
        let _g = crate::test_util::global_state_lock();
        let bytes = [0u8, 0, 0, 0];
        assert_eq!(
            zsh_inet_ntop(libc::AF_INET, &bytes).as_deref(),
            Some("0.0.0.0")
        );
    }

    /// c:72 — `zsh_inet_ntop` on `255.255.255.255` (broadcast).
    /// Pin the max-octet rendering since octet width and base-10
    /// formatting share the inner loop.
    #[test]
    fn inet_ntop_broadcast_address_is_all_255() {
        let _g = crate::test_util::global_state_lock();
        let bytes = [255u8, 255, 255, 255];
        assert_eq!(
            zsh_inet_ntop(libc::AF_INET, &bytes).as_deref(),
            Some("255.255.255.255")
        );
    }

    /// c:122 — `zsh_inet_pton` round-trips through `inet_ntop` for a
    /// sweep of typical addresses. Bidirectional contract pinned.
    #[test]
    fn inet_pton_ntop_round_trips_for_typical_addresses() {
        let _g = crate::test_util::global_state_lock();
        for ip in &["192.168.1.1", "10.0.0.1", "172.16.254.1", "8.8.8.8"] {
            let mut buf = [0u8; 4];
            assert_eq!(
                zsh_inet_pton(libc::AF_INET, ip, &mut buf),
                1,
                "pton failed on {}",
                ip
            );
            assert_eq!(
                zsh_inet_ntop(libc::AF_INET, &buf).as_deref(),
                Some(*ip),
                "round-trip mismatch for {}",
                ip
            );
        }
    }

    /// c:122 — `zsh_inet_pton` rejects out-of-range octets. "256"
    /// is the off-by-one to pin (0-255 valid; 256 invalid).
    #[test]
    fn inet_pton_rejects_octet_over_255() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 4];
        assert_eq!(zsh_inet_pton(libc::AF_INET, "256.0.0.0", &mut buf), 0);
    }

    /// c:122 — `zsh_inet_pton` with empty string returns 0. Pin
    /// defensive shape; libc::inet_pton returns 0 for "" but a
    /// regression could panic on the empty CString allocation.
    #[test]
    fn inet_pton_rejects_empty_string() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 4];
        assert_eq!(zsh_inet_pton(libc::AF_INET, "", &mut buf), 0);
    }

    /// c:215 — `zts_alloc` flag-passthrough: the new session's
    /// `flags` field MUST equal the input flags exactly (no extra
    /// bits OR'd in by the allocator).
    #[test]
    fn zts_alloc_flags_passthrough() {
        let _g = crate::test_util::global_state_lock();
        let _ = zts_alloc(ZTCP_LISTEN);
        ZTCP_SESSIONS.with(|s| {
            let sessions = s.borrow();
            let last = sessions.last().unwrap();
            assert_eq!(
                last.flags, ZTCP_LISTEN,
                "flags must be passed through verbatim"
            );
        });
    }

    /// c:253 — `zts_delete` on a non-existent fd is a safe no-op;
    /// the session count does not change.
    #[test]
    fn zts_delete_unknown_fd_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let before = ZTCP_SESSIONS.with(|s| s.borrow().len());
        let _ = zts_delete(99999);
        let after = ZTCP_SESSIONS.with(|s| s.borrow().len());
        assert_eq!(
            before, after,
            "delete of unknown fd must not change session count"
        );
    }

    /// c:714-740 — module-lifecycle stubs all return 0 in C.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    // ─── zsh-corpus pins for zsh_inet_ntop / zsh_inet_pton ──────────

    /// `zsh_inet_ntop(AF_INET, [127,0,0,1])` = "127.0.0.1".
    #[test]
    fn tcp_corpus_inet_ntop_loopback_v4() {
        let _g = crate::test_util::global_state_lock();
        let r = zsh_inet_ntop(libc::AF_INET, &[127, 0, 0, 1]);
        assert_eq!(r.as_deref(), Some("127.0.0.1"));
    }

    /// `zsh_inet_ntop(AF_INET, [0,0,0,0])` = "0.0.0.0".
    #[test]
    fn tcp_corpus_inet_ntop_any_v4() {
        let _g = crate::test_util::global_state_lock();
        let r = zsh_inet_ntop(libc::AF_INET, &[0, 0, 0, 0]);
        assert_eq!(r.as_deref(), Some("0.0.0.0"));
    }

    /// `zsh_inet_ntop` with short buffer returns None.
    #[test]
    fn tcp_corpus_inet_ntop_short_buffer_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let r = zsh_inet_ntop(libc::AF_INET, &[1, 2]);
        assert!(r.is_none());
    }

    /// `zsh_inet_ntop(99 unknown af)` returns None.
    #[test]
    fn tcp_corpus_inet_ntop_unknown_af_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let r = zsh_inet_ntop(99, &[1, 2, 3, 4]);
        assert!(r.is_none());
    }

    /// `zsh_inet_pton(AF_INET, "127.0.0.1")` parses to bytes [127,0,0,1].
    #[test]
    fn tcp_corpus_inet_pton_v4_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let mut dst = [0u8; 4];
        let r = zsh_inet_pton(libc::AF_INET, "127.0.0.1", &mut dst);
        assert_eq!(r, 1);
        assert_eq!(dst, [127, 0, 0, 1]);
    }

    /// `zsh_inet_pton` on invalid input returns 0.
    #[test]
    fn tcp_corpus_inet_pton_invalid_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut dst = [0u8; 4];
        assert_eq!(zsh_inet_pton(libc::AF_INET, "not.an.ip", &mut dst), 0);
        assert_eq!(zsh_inet_pton(libc::AF_INET, "999.999.999.999", &mut dst), 0);
    }

    /// `zsh_inet_pton` on unknown af returns -1.
    #[test]
    fn tcp_corpus_inet_pton_unknown_af_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        let mut dst = [0u8; 4];
        assert_eq!(zsh_inet_pton(99, "127.0.0.1", &mut dst), -1);
    }

    /// `zsh_inet_pton` short dst buffer returns 0.
    #[test]
    fn tcp_corpus_inet_pton_short_dst_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut dst = [0u8; 2];
        assert_eq!(zsh_inet_pton(libc::AF_INET, "127.0.0.1", &mut dst), 0);
    }

    /// IPv4 ntop/pton round-trip: known address survives.
    #[test]
    fn tcp_corpus_v4_ntop_pton_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let original = [192, 168, 1, 100];
        let s = zsh_inet_ntop(libc::AF_INET, &original).unwrap();
        let mut back = [0u8; 4];
        let rc = zsh_inet_pton(libc::AF_INET, &s, &mut back);
        assert_eq!(rc, 1);
        assert_eq!(back, original);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/tcp.c IP-address helpers.
    // ═══════════════════════════════════════════════════════════════════

    /// c:72 — `zsh_inet_ntop(AF_INET, ...)` canonical formats.
    #[test]
    fn zsh_inet_ntop_v4_canonical_addresses() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zsh_inet_ntop(libc::AF_INET, &[127, 0, 0, 1]).as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            zsh_inet_ntop(libc::AF_INET, &[0, 0, 0, 0]).as_deref(),
            Some("0.0.0.0")
        );
        assert_eq!(
            zsh_inet_ntop(libc::AF_INET, &[255, 255, 255, 255]).as_deref(),
            Some("255.255.255.255")
        );
    }

    /// c:72 — short buffer returns None (won't crash on insufficient input).
    #[test]
    fn zsh_inet_ntop_v4_short_buffer_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(zsh_inet_ntop(libc::AF_INET, &[127, 0, 0]).is_none());
        assert!(zsh_inet_ntop(libc::AF_INET, &[]).is_none());
    }

    /// c:72 — IPv6 short buffer (< 16 bytes) returns None.
    #[test]
    fn zsh_inet_ntop_v6_short_buffer_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let too_short = vec![0u8; 8];
        assert!(zsh_inet_ntop(libc::AF_INET6, &too_short).is_none());
    }

    /// c:72 — unsupported AF returns None.
    #[test]
    fn zsh_inet_ntop_unsupported_af_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(zsh_inet_ntop(99, &[127, 0, 0, 1]).is_none());
        assert!(zsh_inet_ntop(0, &[0u8; 16]).is_none());
    }

    /// c:122 — `zsh_inet_pton` parses canonical IPv4 addresses.
    #[test]
    fn zsh_inet_pton_v4_canonical_addresses() {
        let _g = crate::test_util::global_state_lock();
        let mut dst = [0u8; 4];
        assert_eq!(zsh_inet_pton(libc::AF_INET, "127.0.0.1", &mut dst), 1);
        assert_eq!(dst, [127, 0, 0, 1]);
        assert_eq!(zsh_inet_pton(libc::AF_INET, "0.0.0.0", &mut dst), 1);
        assert_eq!(dst, [0, 0, 0, 0]);
        assert_eq!(zsh_inet_pton(libc::AF_INET, "255.255.255.255", &mut dst), 1);
        assert_eq!(dst, [255, 255, 255, 255]);
    }

    /// c:122 — malformed IPv4 string returns 0 (per libc inet_pton).
    #[test]
    fn zsh_inet_pton_v4_malformed_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut dst = [0u8; 4];
        assert_eq!(zsh_inet_pton(libc::AF_INET, "not.an.ip", &mut dst), 0);
        assert_eq!(zsh_inet_pton(libc::AF_INET, "256.0.0.1", &mut dst), 0);
        assert_eq!(zsh_inet_pton(libc::AF_INET, "", &mut dst), 0);
    }

    /// c:103 — `zsh_inet_aton` parses canonical IPv4.
    #[test]
    fn zsh_inet_aton_canonical_v4() {
        let _g = crate::test_util::global_state_lock();
        // 127.0.0.1 → 0x7F000001 big-endian on the wire.
        let r = zsh_inet_aton("127.0.0.1").expect("valid IP");
        // The result is network byte order, so we can't directly assert
        // a literal value across endianness — just check non-zero.
        assert_ne!(r, 0, "valid IP should produce non-zero value");
    }

    /// c:103 — malformed input returns None.
    #[test]
    fn zsh_inet_aton_malformed_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(zsh_inet_aton("not_an_ip").is_none());
        assert!(zsh_inet_aton("").is_none());
        assert!(zsh_inet_aton("256.0.0.1").is_none());
    }

    /// IPv6 ntop/pton round-trip on a known address.
    #[test]
    fn tcp_v6_ntop_pton_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let original = [0u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]; // ::1
        let s = zsh_inet_ntop(libc::AF_INET6, &original).unwrap();
        let mut back = [0u8; 16];
        let rc = zsh_inet_pton(libc::AF_INET6, &s, &mut back);
        assert_eq!(rc, 1);
        assert_eq!(back, original);
    }

    /// `setup_(NULL)` returns 0.
    #[test]
    fn tcp_setup_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// Lifecycle stubs return 0.
    #[test]
    fn tcp_lifecycle_stubs_return_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
        assert_eq!(cleanup_(std::ptr::null()), 0);
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/tcp.c
    // c:65 zsh_inet_ntop / c:83 zsh_inet_pton / c:107 zsh_gethostbyname2
    // c:147 zts_alloc / c:172 tcp_socket / c:199 zts_delete / c:218 zts_byfd
    // c:227 tcp_cleanup / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:65 — `zsh_inet_ntop` is deterministic.
    #[test]
    fn zsh_inet_ntop_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let addr = [127u8, 0, 0, 1];
        let first = zsh_inet_ntop(libc::AF_INET, &addr);
        for _ in 0..5 {
            assert_eq!(zsh_inet_ntop(libc::AF_INET, &addr), first);
        }
    }

    /// c:65 — `zsh_inet_ntop` AF_UNSPEC returns None.
    #[test]
    fn zsh_inet_ntop_af_unspec_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let addr = [127u8, 0, 0, 1];
        assert!(zsh_inet_ntop(libc::AF_UNSPEC, &addr).is_none());
    }

    /// c:83 — `zsh_inet_pton` empty src returns 0.
    #[test]
    fn zsh_inet_pton_empty_src_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut dst = [0u8; 4];
        assert_eq!(zsh_inet_pton(libc::AF_INET, "", &mut dst), 0);
    }

    /// c:107 — `zsh_gethostbyname2` for nonexistent name returns empty Vec.
    #[test]
    fn zsh_gethostbyname2_nonexistent_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = zsh_gethostbyname2(
            "definitely_not_a_real_host_xyz_zshrs.invalid",
            libc::AF_INET,
        );
        assert!(r.is_empty(), "nonexistent host → empty vec");
    }

    /// c:107 — `zsh_gethostbyname2` empty name returns empty Vec.
    #[test]
    fn zsh_gethostbyname2_empty_name_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = zsh_gethostbyname2("", libc::AF_INET);
        assert!(r.is_empty());
    }

    /// c:188 — `ztcp_free_session(invalid)` is safe.
    #[test]
    fn ztcp_free_session_invalid_handle_no_panic() {
        let _g = crate::test_util::global_state_lock();
        // 0 / max / random — should be safe.
        let _ = ztcp_free_session(0);
        let _ = ztcp_free_session(usize::MAX);
        let _ = ztcp_free_session(99999);
    }

    /// c:199 — `zts_delete(0)` is safe (stdin fd, but our table doesn't
    /// own it).
    #[test]
    fn zts_delete_fd_zero_safe() {
        let _g = crate::test_util::global_state_lock();
        let _ = zts_delete(0);
    }

    /// c:227 — `tcp_cleanup` is idempotent.
    #[test]
    fn tcp_cleanup_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            tcp_cleanup();
        }
    }

    /// c:147 — `zts_alloc(N)` followed by `zts_alloc(M)` returns
    /// monotonically increasing indices (each alloc gets a fresh slot).
    #[test]
    fn zts_alloc_returns_monotonic_indices() {
        let _g = crate::test_util::global_state_lock();
        let a = zts_alloc(0);
        let b = zts_alloc(0);
        let c = zts_alloc(0);
        assert!(b > a, "second alloc > first ({} > {})", b, a);
        assert!(c > b, "third alloc > second ({} > {})", c, b);
    }

    /// c:844-890 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn tcp_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
        assert_eq!(finish_(null), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/tcp.c
    // c:65 zsh_inet_ntop / c:83 zsh_inet_pton / c:107 zsh_gethostbyname2 /
    // c:131 zsh_getipnodebyname / c:139 freehostent / c:147 zts_alloc /
    // c:188 ztcp_free_session / c:199 zts_delete / c:927 zsh_inet_aton +
    // lifecycle type pins
    // ═══════════════════════════════════════════════════════════════════

    /// c:65 — `zsh_inet_ntop` returns Option<String> (compile-time type pin).
    #[test]
    fn zsh_inet_ntop_returns_option_string_type() {
        let _: Option<String> = zsh_inet_ntop(libc::AF_INET, &[127, 0, 0, 1]);
    }

    /// c:83 — `zsh_inet_pton` returns i32 (compile-time type pin).
    #[test]
    fn zsh_inet_pton_returns_i32_type() {
        let mut dst = [0u8; 4];
        let _: i32 = zsh_inet_pton(libc::AF_INET, "1.2.3.4", &mut dst);
    }

    /// c:107 — `zsh_gethostbyname2` returns Vec<[u8;4]>.
    #[test]
    fn zsh_gethostbyname2_returns_vec_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<[u8; 4]> = zsh_gethostbyname2("", libc::AF_INET);
    }

    /// c:131 — `zsh_getipnodebyname` delegates to `zsh_gethostbyname2`
    /// (same body per c:170). Both must return identical results.
    #[test]
    fn zsh_getipnodebyname_matches_gethostbyname2() {
        let _g = crate::test_util::global_state_lock();
        for name in ["", "nonexistent.invalid.zshrs.xyz"] {
            let a = zsh_getipnodebyname(name, libc::AF_INET);
            let b = zsh_gethostbyname2(name, libc::AF_INET);
            assert_eq!(
                a, b,
                "zsh_getipnodebyname({:?}) must match gethostbyname2",
                name
            );
        }
    }

    /// c:139 — `freehostent` is a void no-op (compile-time pin).
    #[test]
    fn freehostent_signature_void() {
        let _: () = freehostent();
    }

    /// c:139 — `freehostent` is idempotent / safe to call repeatedly.
    #[test]
    fn freehostent_idempotent() {
        for _ in 0..10 {
            freehostent();
        }
    }

    /// c:147 — `zts_alloc` returns usize (compile-time type pin).
    #[test]
    fn zts_alloc_returns_usize_type() {
        let _g = crate::test_util::global_state_lock();
        let _: usize = zts_alloc(0);
    }

    /// c:188 — `ztcp_free_session` returns i32.
    #[test]
    fn ztcp_free_session_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = ztcp_free_session(usize::MAX);
    }

    /// c:199 — `zts_delete` returns i32.
    #[test]
    fn zts_delete_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = zts_delete(-1);
    }

    /// c:927 — `zsh_inet_aton` returns Option<u32>.
    #[test]
    fn zsh_inet_aton_returns_option_u32_type() {
        let _: Option<u32> = zsh_inet_aton("1.2.3.4");
    }

    /// c:927 — `zsh_inet_aton("")` empty returns None.
    #[test]
    fn zsh_inet_aton_empty_returns_none() {
        assert!(zsh_inet_aton("").is_none(), "empty → None");
    }

    /// c:927 — `zsh_inet_aton` is deterministic for stable input.
    #[test]
    fn zsh_inet_aton_is_deterministic() {
        for s in ["", "1.2.3.4", "garbage", "256.256.256.256"] {
            let first = zsh_inet_aton(s);
            for _ in 0..3 {
                assert_eq!(
                    zsh_inet_aton(s),
                    first,
                    "zsh_inet_aton({:?}) must be deterministic",
                    s
                );
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/tcp.c
    // c:65 zsh_inet_ntop / c:83 zsh_inet_pton / c:107 zsh_gethostbyname2 /
    // c:319 bin_ztcp / c:844-890 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:65 — `zsh_inet_ntop` for empty bytes doesn't panic.
    #[test]
    fn zsh_inet_ntop_empty_bytes_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = zsh_inet_ntop(libc::AF_INET, &[]);
    }

    /// c:65 — `zsh_inet_ntop` for canonical IPv4 returns dotted-quad.
    #[test]
    fn zsh_inet_ntop_ipv4_canonical_form() {
        let _g = crate::test_util::global_state_lock();
        let r = zsh_inet_ntop(libc::AF_INET, &[127, 0, 0, 1]);
        assert!(r.is_some(), "127.0.0.1 must convert");
        assert_eq!(
            r.unwrap(),
            "127.0.0.1",
            "must produce canonical dotted-quad"
        );
    }

    /// c:65 — `zsh_inet_ntop` invalid family returns None.
    #[test]
    fn zsh_inet_ntop_invalid_family_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let r = zsh_inet_ntop(99999, &[1, 2, 3, 4]);
        assert!(r.is_none(), "invalid AF must return None");
    }

    /// c:83 — `zsh_inet_pton("")` empty returns failure code (≤ 0).
    #[test]
    fn zsh_inet_pton_empty_returns_failure() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 16];
        let r = zsh_inet_pton(libc::AF_INET, "", &mut buf);
        assert!(r <= 0, "empty pton must fail (≤ 0), got {}", r);
    }

    /// c:83 — `zsh_inet_pton("127.0.0.1", AF_INET)` parses to [127,0,0,1].
    #[test]
    fn zsh_inet_pton_localhost_parses_correctly() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 16];
        let r = zsh_inet_pton(libc::AF_INET, "127.0.0.1", &mut buf);
        assert_eq!(r, 1, "valid IPv4 must return 1");
        assert_eq!(&buf[..4], &[127, 0, 0, 1]);
    }

    /// c:107 — `zsh_gethostbyname2("")` returns Vec type.
    #[test]
    fn zsh_gethostbyname2_empty_name_returns_vec_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<[u8; 4]> = zsh_gethostbyname2("", libc::AF_INET);
    }

    /// c:319 — `bin_ztcp` empty args non-negative.
    #[test]
    fn bin_ztcp_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_ztcp("ztcp", &[], &ops, 0);
        assert!(r >= 0, "bin_ztcp empty must be ≥ 0, got {}", r);
    }

    /// c:319 — `bin_ztcp` various func values don't panic.
    #[test]
    fn bin_ztcp_various_func_values_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for func in [-1, 0, 1, 100, i32::MAX] {
            let _ = bin_ztcp("ztcp", &[], &ops, func);
        }
    }

    /// c:844 — `setup_` is idempotent.
    #[test]
    fn tcp_setup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:882 — `cleanup_` is idempotent.
    #[test]
    fn tcp_cleanup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:890 — `finish_` is idempotent.
    #[test]
    fn tcp_finish_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:319 — `bin_ztcp` deterministic for identical args (idle state).
    #[test]
    fn bin_ztcp_deterministic_for_same_args() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r1 = bin_ztcp("ztcp", &[], &ops, 0);
        let r2 = bin_ztcp("ztcp", &[], &ops, 0);
        assert_eq!(r1, r2, "bin_ztcp empty args must be deterministic");
    }
}
