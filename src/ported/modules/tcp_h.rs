//! Direct port of `Src/Modules/tcp.h` — header file for the
//! `zsh/net/tcp` module (builtin FTP / generic TCP socket helpers).
//!
//! Original C copyright: Peter Stephenson 1998-2001.
//!
//! The C header pulls in the system network headers (`<sys/socket.h>`,
//! `<netdb.h>`, `<netinet/in.h>`, `<netinet/ip.h>`, `<arpa/inet.h>`),
//! decides at preprocessor time whether IPv6 is supported, and then
//! defines:
//!   * `union tcp_sockaddr` — a tagged union over `sockaddr_in` /
//!     `sockaddr_in6` / `sockaddr` (c:74-79)
//!   * `Tcp_session` typedef + `ZTCP_*` flag bits (c:81-85)
//!   * `struct tcp_session` — fd + local/remote sockaddrs + flags
//!     (c:87-92)
//!   * `INET_ADDRSTRLEN` / `INET6_ADDRSTRLEN` fallback values
//!     (c:96-102)
//!
//! The Rust port mirrors all of these. The system network headers
//! that `tcp.h` includes are available through the `libc` crate; the
//! relevant types/constants resolve under their POSIX names so the
//! rest of the port can write `libc::sockaddr_in` / `libc::AF_INET`
//! verbatim per `Src/Modules/tcp.c`.

// =====================================================================
// SUPPORT_IPV6 — from `Src/Modules/tcp.h:69-72`.
// =====================================================================
//
// C: defined when AF_INET6 + IN6ADDR_LOOPBACK_INIT + HAVE_INET_NTOP
// + HAVE_INET_PTON are all present. Since libc::AF_INET6 is
// unconditionally available on every platform zshrs supports
// (macOS, Linux, *BSD), we treat IPv6 support as always-on.
pub const SUPPORT_IPV6: bool = true;                                     // c:71

// =====================================================================
// `union tcp_sockaddr` — `Src/Modules/tcp.h:74-79`.
// =====================================================================
//
// C definition (c:74-79):
// ```c
// union tcp_sockaddr {
//     struct sockaddr a;
//     struct sockaddr_in in;
// #ifdef SUPPORT_IPV6
//     struct sockaddr_in6 in6;
// #endif
// };
// ```
//
// The C union is read-write polymorphic — same memory aliased as any
// of the three sockaddr shapes depending on the family bit. Rust port
// uses #[repr(C)] union with a sockaddr_storage backing field so the
// allocation is at least as wide as any sockaddr family.
#[allow(non_camel_case_types)]
#[repr(C)]
pub union tcp_sockaddr {                                                  // c:74
    pub a:   libc::sockaddr,                                              // c:75
    pub in_: libc::sockaddr_in,                                           // c:76 — `in` is a Rust keyword
    pub in6: libc::sockaddr_in6,                                          // c:78
    /// Storage placeholder so the union is at least as wide as
    /// `sockaddr_storage` on every platform. C relies on the
    /// preprocessor + struct sizes; Rust needs an explicit field.
    pub storage: libc::sockaddr_storage,
}

// =====================================================================
// `Tcp_session` typedef + `ZTCP_*` flags — `Src/Modules/tcp.h:81-85`.
// =====================================================================

/// Port of `typedef struct tcp_session *Tcp_session;` from
/// `Src/Modules/tcp.h:81`. The C source uses bare pointers; Rust
/// wraps in `Box` for ownership clarity at allocation sites and
/// keeps `*mut tcp_session` for FFI boundaries.
pub type Tcp_session = Box<tcp_session>;                                  // c:81

/// `ZTCP_LISTEN` from `Src/Modules/tcp.h:83`. Set on a session in
/// passive (listen) mode — the C source's `bin_ztcp -l` form.
pub const ZTCP_LISTEN:  i32 = 1;                                          // c:83

/// `ZTCP_INBOUND` from `Src/Modules/tcp.h:84`. Set on a session that
/// represents an incoming connection accepted on a listen-mode parent.
pub const ZTCP_INBOUND: i32 = 2;                                          // c:84

/// `ZTCP_ZFTP` from `Src/Modules/tcp.h:85`. Set on a session opened
/// by the `zsh/zftp` module (so `bin_ztcp -c` doesn't tear it down
/// from under zftp).
pub const ZTCP_ZFTP:    i32 = 16;                                         // c:85

// =====================================================================
// `struct tcp_session` — `Src/Modules/tcp.h:87-92`.
// =====================================================================
//
// C definition (c:87-92):
// ```c
// struct tcp_session {
//     int fd;                       /* file descriptor */
//     union tcp_sockaddr sock;      /* local address   */
//     union tcp_sockaddr peer;      /* remote address  */
//     int flags;
// };
// ```

#[allow(non_camel_case_types)]
#[repr(C)]
pub struct tcp_session {                                                  // c:87
    pub fd:    i32,                                                       // c:88 — file descriptor
    pub sock:  tcp_sockaddr,                                              // c:89 — local address
    pub peer:  tcp_sockaddr,                                              // c:90 — remote address
    pub flags: i32,                                                       // c:91 — ZTCP_* bitmask
}

// =====================================================================
// `INET_ADDRSTRLEN` / `INET6_ADDRSTRLEN` fallbacks — `tcp.h:96-102`.
// =====================================================================
//
// C provides these as fallback `#define`s when the system headers
// don't already supply them (older platforms). Modern libc always
// has them; the Rust port pulls from `libc` when available and
// falls back to the C-source numeric values otherwise.

/// Port of `INET_ADDRSTRLEN` fallback from `Src/Modules/tcp.h:97`.
/// "16" matches the C source's fallback, equal to `libc::INET_ADDRSTRLEN`
/// on every supported platform.
pub const INET_ADDRSTRLEN:  usize = 16;                                   // c:97

/// Port of `INET6_ADDRSTRLEN` fallback from `Src/Modules/tcp.h:101`.
/// "46" matches the C source's fallback, equal to `libc::INET6_ADDRSTRLEN`
/// on every supported platform.
pub const INET6_ADDRSTRLEN: usize = 46;                                   // c:101
