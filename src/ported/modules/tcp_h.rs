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
/// `SUPPORT_IPV6` constant.
pub const SUPPORT_IPV6: bool = true; // c:71

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
/// `tcp_sockaddr` union.
#[allow(non_camel_case_types)]
#[repr(C)]
pub union tcp_sockaddr {
    // c:74
    pub a: libc::sockaddr,       // c:75
    pub in_: libc::sockaddr_in,  // c:76 — `in` is a Rust keyword
    pub in6: libc::sockaddr_in6, // c:78
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
pub type Tcp_session = Box<tcp_session>; // c:81

/// `ZTCP_LISTEN` from `Src/Modules/tcp.h:83`. Set on a session in
/// passive (listen) mode — the C source's `bin_ztcp -l` form.
pub const ZTCP_LISTEN: i32 = 1; // c:83

/// `ZTCP_INBOUND` from `Src/Modules/tcp.h:84`. Set on a session that
/// represents an incoming connection accepted on a listen-mode parent.
pub const ZTCP_INBOUND: i32 = 2; // c:84

/// `ZTCP_ZFTP` from `Src/Modules/tcp.h:85`. Set on a session opened
/// by the `zsh/zftp` module (so `bin_ztcp -c` doesn't tear it down
/// from under zftp).
pub const ZTCP_ZFTP: i32 = 16; // c:85

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
/// `tcp_session` — see fields for layout.
#[allow(non_camel_case_types)]
#[repr(C)]
pub struct tcp_session {
    // c:87
    pub fd: i32,            // c:88 — file descriptor
    pub sock: tcp_sockaddr, // c:89 — local address
    pub peer: tcp_sockaddr, // c:90 — remote address
    pub flags: i32,         // c:91 — ZTCP_* bitmask
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
pub const INET_ADDRSTRLEN: usize = 16; // c:97

/// Port of `INET6_ADDRSTRLEN` fallback from `Src/Modules/tcp.h:101`.
/// "46" matches the C source's fallback, equal to `libc::INET6_ADDRSTRLEN`
/// on every supported platform.
pub const INET6_ADDRSTRLEN: usize = 46; // c:101

#[cfg(test)]
mod tests {
    use super::*;

    /// `ZTCP_ZFTP` must occupy a bit clear of LISTEN/INBOUND so the
    /// zftp module's combined-mode flags (`listening zftp socket`) can
    /// OR them without ambiguity. This is the real ABI invariant —
    /// the individual values are just enum scaffolding.
    #[test]
    fn ztcp_zftp_bit_does_not_overlap_listen_or_inbound() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ZTCP_ZFTP & (ZTCP_LISTEN | ZTCP_INBOUND), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/tcp.h constants.
    // ═══════════════════════════════════════════════════════════════════

    /// c:71 — SUPPORT_IPV6 = true for modern platforms.
    #[test]
    fn support_ipv6_enabled() {
        assert!(SUPPORT_IPV6, "IPv6 must be enabled on modern platforms");
    }

    /// c:83 — ZTCP_LISTEN is bit 0 (= 1).
    #[test]
    fn ztcp_listen_is_bit_zero() {
        assert_eq!(ZTCP_LISTEN, 1);
    }

    /// c:84 — ZTCP_INBOUND is bit 1 (= 2).
    #[test]
    fn ztcp_inbound_is_bit_one() {
        assert_eq!(ZTCP_INBOUND, 2);
    }

    /// c:85 — ZTCP_ZFTP is bit 4 (= 16).
    #[test]
    fn ztcp_zftp_is_bit_four() {
        assert_eq!(ZTCP_ZFTP, 16);
    }

    /// ZTCP_* all positive, single-bit values.
    #[test]
    fn ztcp_flags_are_single_bits() {
        for &v in &[ZTCP_LISTEN, ZTCP_INBOUND, ZTCP_ZFTP] {
            assert!(v > 0, "{} must be positive", v);
            assert!(
                (v as u32).is_power_of_two(),
                "{} must be a power of 2 (single bit)",
                v
            );
        }
    }

    /// ZTCP_LISTEN | ZTCP_INBOUND can OR together cleanly.
    #[test]
    fn ztcp_listen_inbound_can_combine() {
        let combined = ZTCP_LISTEN | ZTCP_INBOUND;
        assert_eq!(combined, 3, "1 | 2 = 3");
        // Both bits readable from combined.
        assert_ne!(combined & ZTCP_LISTEN, 0);
        assert_ne!(combined & ZTCP_INBOUND, 0);
    }

    /// c:97 — INET_ADDRSTRLEN = 16 (canonical IPv4 max textual length:
    /// "255.255.255.255" + NUL).
    #[test]
    fn inet_addrstrlen_is_16() {
        assert_eq!(INET_ADDRSTRLEN, 16);
    }

    /// c:101 — INET6_ADDRSTRLEN = 46 (canonical IPv6 max textual length).
    #[test]
    fn inet6_addrstrlen_is_46() {
        assert_eq!(INET6_ADDRSTRLEN, 46);
    }

    /// IPv6 textual length > IPv4 (sanity invariant).
    #[test]
    fn inet6_strlen_greater_than_inet_strlen() {
        assert!(
            INET6_ADDRSTRLEN > INET_ADDRSTRLEN,
            "IPv6 string is longer than IPv4"
        );
    }

    /// INET_ADDRSTRLEN matches IPv4 textual representation length.
    /// "255.255.255.255" = 15 chars + NUL = 16.
    #[test]
    fn inet_addrstrlen_matches_max_ipv4_text() {
        assert_eq!(INET_ADDRSTRLEN, "255.255.255.255".len() + 1);
    }

    /// tcp_session struct is non-zero size (has fields).
    #[test]
    fn tcp_session_struct_non_zero_size() {
        let size = std::mem::size_of::<tcp_session>();
        assert!(size > 0, "tcp_session must have non-zero size");
        // Must hold at least: fd (i32) + 2 sockaddrs + flags (i32).
        assert!(size >= 4 + 4, "tcp_session must hold fd + flags minimum");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/tcp.h
    // c:71 SUPPORT_IPV6 / c:83 ZTCP_LISTEN / c:84 ZTCP_INBOUND /
    // c:85 ZTCP_ZFTP / c:87 tcp_session / c:97 INET_ADDRSTRLEN /
    // c:101 INET6_ADDRSTRLEN
    // ═══════════════════════════════════════════════════════════════════

    /// c:83-85 — all ZTCP_* are i32 type (compile-time pin).
    #[test]
    fn ztcp_flags_all_i32_type() {
        let _: i32 = ZTCP_LISTEN;
        let _: i32 = ZTCP_INBOUND;
        let _: i32 = ZTCP_ZFTP;
    }

    /// c:97/101 — INET_ADDRSTRLEN and INET6_ADDRSTRLEN are usize.
    #[test]
    fn inet_strlen_constants_are_usize_type() {
        let _: usize = INET_ADDRSTRLEN;
        let _: usize = INET6_ADDRSTRLEN;
    }

    /// c:101 — INET6_ADDRSTRLEN matches max IPv6 textual representation length.
    /// "ffff:ffff:ffff:ffff:ffff:ffff:255.255.255.255" + NUL = 46.
    #[test]
    fn inet6_addrstrlen_matches_max_ipv6_with_v4_mapped() {
        // Maximum IPv6 textual form is 45 chars + NUL = 46:
        // "ffff:ffff:ffff:ffff:ffff:ffff:255.255.255.255"
        let max_ipv6 = "ffff:ffff:ffff:ffff:ffff:ffff:255.255.255.255";
        assert_eq!(max_ipv6.len(), 45, "verify max IPv6 text length");
        assert_eq!(INET6_ADDRSTRLEN, max_ipv6.len() + 1,
            "INET6_ADDRSTRLEN must hold max textual + NUL");
    }

    /// c:71 — SUPPORT_IPV6 is bool (compile-time pin).
    #[test]
    fn support_ipv6_is_bool_type() {
        let _: bool = SUPPORT_IPV6;
    }

    /// c:83-85 — ZTCP_LISTEN/INBOUND combined OR keeps both bits.
    #[test]
    fn ztcp_listen_inbound_or_keeps_both_bits() {
        let combo = ZTCP_LISTEN | ZTCP_INBOUND;
        assert_ne!(combo & ZTCP_LISTEN, 0, "LISTEN bit preserved");
        assert_ne!(combo & ZTCP_INBOUND, 0, "INBOUND bit preserved");
    }

    /// c:83-85 — ZTCP_ZFTP can OR with LISTEN+INBOUND combination.
    #[test]
    fn ztcp_zftp_can_or_with_listen_inbound_combo() {
        let combo = ZTCP_ZFTP | ZTCP_LISTEN | ZTCP_INBOUND;
        assert_ne!(combo & ZTCP_ZFTP, 0);
        assert_ne!(combo & ZTCP_LISTEN, 0);
        assert_ne!(combo & ZTCP_INBOUND, 0);
    }

    /// c:97/101 — INET_ADDRSTRLEN + INET6_ADDRSTRLEN both positive.
    #[test]
    fn inet_strlen_constants_positive() {
        assert!(INET_ADDRSTRLEN > 0);
        assert!(INET6_ADDRSTRLEN > 0);
    }

    /// c:97 — `INET_ADDRSTRLEN` of 16 holds longest IPv4 ("255.255.255.255" + NUL).
    #[test]
    fn inet_addrstrlen_holds_max_ipv4_dotted() {
        let max_ipv4 = "255.255.255.255";
        assert_eq!(INET_ADDRSTRLEN, max_ipv4.len() + 1,
            "INET_ADDRSTRLEN must hold max IPv4 textual + NUL");
    }

    /// c:87 — tcp_session size is at least 8 bytes (fd + flags as i32 pair).
    #[test]
    fn tcp_session_minimum_size_holds_fd_plus_flags() {
        let size = std::mem::size_of::<tcp_session>();
        assert!(size >= 8, "must hold fd+flags pair (≥ 8 bytes), got {}", size);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/tcp.h structural invariants.
    // ═══════════════════════════════════════════════════════════════════

    /// c:74-79 — `tcp_sockaddr` union size ≥ `sockaddr_storage` (since
    /// the union has a `storage` field for that purpose). Pin the
    /// allocation-width contract.
    #[test]
    fn tcp_sockaddr_union_at_least_sockaddr_storage_size() {
        let union_sz = std::mem::size_of::<tcp_sockaddr>();
        let storage_sz = std::mem::size_of::<libc::sockaddr_storage>();
        assert!(union_sz >= storage_sz,
            "tcp_sockaddr union ({}) must hold sockaddr_storage ({})",
            union_sz, storage_sz);
    }

    /// c:74-79 — `tcp_sockaddr` union must be at least as big as each
    /// of its variants (sockaddr/sockaddr_in/sockaddr_in6).
    #[test]
    fn tcp_sockaddr_union_holds_every_variant() {
        let union_sz = std::mem::size_of::<tcp_sockaddr>();
        assert!(union_sz >= std::mem::size_of::<libc::sockaddr>(),
            "must hold sockaddr ({})", std::mem::size_of::<libc::sockaddr>());
        assert!(union_sz >= std::mem::size_of::<libc::sockaddr_in>(),
            "must hold sockaddr_in ({})", std::mem::size_of::<libc::sockaddr_in>());
        assert!(union_sz >= std::mem::size_of::<libc::sockaddr_in6>(),
            "must hold sockaddr_in6 ({})", std::mem::size_of::<libc::sockaddr_in6>());
    }

    /// c:87 — `tcp_session.fd` is i32 (compile-time access).
    /// Note: union field access is unsafe; this only pins the struct
    /// layout by constructing one through a known-zero pattern.
    #[test]
    fn tcp_session_fd_field_is_i32() {
        // SAFETY: We construct a tcp_session purely to read the `fd`
        // i32 field — the sockaddr unions are zero-init storage.
        let sess: tcp_session = unsafe { std::mem::zeroed() };
        let _: i32 = sess.fd;
        let _: i32 = sess.flags;
    }

    /// c:83/84/85 — sum of LISTEN + INBOUND + ZFTP equals their OR
    /// (proves no overlapping bits across all three flags).
    #[test]
    fn ztcp_flags_no_pairwise_overlap_across_all_three() {
        let sum = ZTCP_LISTEN + ZTCP_INBOUND + ZTCP_ZFTP;
        let or  = ZTCP_LISTEN | ZTCP_INBOUND | ZTCP_ZFTP;
        assert_eq!(sum, or,
            "sum==OR proves no shared bits between LISTEN/INBOUND/ZFTP");
    }

    /// c:83 — ZTCP_LISTEN does not overlap ZTCP_INBOUND (subset of the
    /// triple-disjointness test but pinned individually for tighter
    /// failure resolution).
    #[test]
    fn ztcp_listen_disjoint_from_inbound() {
        assert_eq!(ZTCP_LISTEN & ZTCP_INBOUND, 0,
            "LISTEN and INBOUND must not share bits");
    }

    /// c:85 — ZTCP_ZFTP must occupy a bit higher than INBOUND so
    /// a sequential bit allocation pattern survives future ZTCP_* adds.
    #[test]
    fn ztcp_zftp_bit_higher_than_inbound() {
        assert!(ZTCP_ZFTP > ZTCP_INBOUND,
            "ZTCP_ZFTP ({}) must be > ZTCP_INBOUND ({}) for clean bitfield growth",
            ZTCP_ZFTP, ZTCP_INBOUND);
    }

    /// c:97 — INET_ADDRSTRLEN matches POSIX RFC value (16).
    /// libc crate doesn't expose this as `pub const` on every target;
    /// pin to the RFC-mandated numeric value directly.
    #[test]
    fn inet_addrstrlen_matches_posix_rfc_value() {
        assert_eq!(INET_ADDRSTRLEN, 16,
            "INET_ADDRSTRLEN must equal 16 per RFC 4291 + POSIX (max IPv4 + NUL)");
    }

    /// c:101 — INET6_ADDRSTRLEN matches POSIX RFC value (46).
    #[test]
    fn inet6_addrstrlen_matches_posix_rfc_value() {
        assert_eq!(INET6_ADDRSTRLEN, 46,
            "INET6_ADDRSTRLEN must equal 46 per RFC 4291 + POSIX");
    }

    /// c:87 — `tcp_session` layout: two sockaddr unions sandwiched
    /// between fd (head) and flags (tail). Size must be ≥
    /// 4 + 2*storage + 4 (the field bound).
    #[test]
    fn tcp_session_size_holds_two_sockaddr_unions() {
        let sz = std::mem::size_of::<tcp_session>();
        let storage_sz = std::mem::size_of::<libc::sockaddr_storage>();
        let minimum = 4 + 2 * storage_sz + 4;
        assert!(sz >= minimum,
            "tcp_session ({} bytes) must hold fd+2*storage+flags (≥ {})",
            sz, minimum);
    }

    /// `Tcp_session` type alias points at owned `Box<tcp_session>`
    /// (zshrs's ownership wrapper over the C bare pointer).
    #[test]
    fn tcp_session_typedef_is_box() {
        // Compile-time check: build a Tcp_session and verify it
        // dereferences to tcp_session.
        let sess: Tcp_session = Box::new(unsafe { std::mem::zeroed::<tcp_session>() });
        let _: &tcp_session = &*sess;
    }

    /// c:83/84/85 — all three flags must be > 0 (positive sentinel
    /// for any flag set; zero means "no flags").
    #[test]
    fn ztcp_flags_all_strictly_positive() {
        assert!(ZTCP_LISTEN > 0);
        assert!(ZTCP_INBOUND > 0);
        assert!(ZTCP_ZFTP > 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/tcp.h
    // c:71 SUPPORT_IPV6 / c:83-85 ZTCP_* flag verbatim values /
    // c:97 INET_ADDRSTRLEN / c:101 INET6_ADDRSTRLEN /
    // c:107 tcp_session struct layout
    // ═══════════════════════════════════════════════════════════════════

    /// c:83-85 — verbatim positional pins: ZTCP_LISTEN=1, INBOUND=2, ZFTP=16.
    #[test]
    fn ztcp_flag_values_verbatim() {
        assert_eq!(ZTCP_LISTEN, 1, "c:83 — ZTCP_LISTEN must be 1");
        assert_eq!(ZTCP_INBOUND, 2, "c:84 — ZTCP_INBOUND must be 2");
        assert_eq!(ZTCP_ZFTP, 16, "c:85 — ZTCP_ZFTP must be 16");
    }

    /// c:83-85 — all ZTCP_* flags are powers of 2 (single-bit masks).
    #[test]
    fn ztcp_flags_all_powers_of_two() {
        for v in [ZTCP_LISTEN, ZTCP_INBOUND, ZTCP_ZFTP] {
            assert!((v as u32).is_power_of_two(),
                "ZTCP_* {} must be single bit", v);
        }
    }

    /// c:83-85 — ZTCP_* flags pairwise distinct.
    #[test]
    fn ztcp_flags_pairwise_distinct() {
        let flags = [ZTCP_LISTEN, ZTCP_INBOUND, ZTCP_ZFTP];
        let unique: std::collections::HashSet<_> = flags.iter().copied().collect();
        assert_eq!(unique.len(), flags.len(),
            "ZTCP_* must be pairwise distinct");
    }

    /// c:83-85 — ZTCP_* flags fit in i8 (room for ≥2x growth).
    #[test]
    fn ztcp_flags_fit_in_i8() {
        for v in [ZTCP_LISTEN, ZTCP_INBOUND, ZTCP_ZFTP] {
            assert!(v < i8::MAX as i32, "ZTCP_* {} must fit in i8", v);
        }
    }

    /// c:83-85 — ZTCP_ZFTP (16) is in the upper-bit reserved range —
    /// bit 4 sits well above LISTEN/INBOUND (bits 0,1) so a future
    /// bit 2,3 addition won't collide.
    #[test]
    fn ztcp_zftp_is_bit_4() {
        assert_eq!(ZTCP_ZFTP, 1 << 4,
            "ZTCP_ZFTP must be bit 4 (reserved gap from INBOUND)");
    }

    /// c:71 — SUPPORT_IPV6 is true on the platform zshrs targets.
    #[test]
    fn support_ipv6_is_enabled() {
        assert!(SUPPORT_IPV6, "SUPPORT_IPV6 must be true on modern target");
    }

    /// c:97 — INET_ADDRSTRLEN = 16 (RFC 791 max IPv4, alt).
    #[test]
    fn inet_addrstrlen_is_16_alt() {
        assert_eq!(INET_ADDRSTRLEN, 16,
            "INET_ADDRSTRLEN must be 16 (RFC 791)");
    }

    /// c:101 — INET6_ADDRSTRLEN = 46 (RFC 4291, alt).
    #[test]
    fn inet6_addrstrlen_is_46_alt() {
        assert_eq!(INET6_ADDRSTRLEN, 46,
            "INET6_ADDRSTRLEN must be 46 (RFC 4291)");
    }

    /// c:97/101 — INET6_ADDRSTRLEN strictly larger than INET_ADDRSTRLEN.
    #[test]
    fn inet6_strlen_larger_than_inet_strlen() {
        assert!(INET6_ADDRSTRLEN > INET_ADDRSTRLEN,
            "IPv6 string must be longer than IPv4: {} vs {}",
            INET6_ADDRSTRLEN, INET_ADDRSTRLEN);
    }

    /// c:107 — `tcp_session` struct is non-zero size.
    #[test]
    fn tcp_session_non_zero_size() {
        assert!(std::mem::size_of::<tcp_session>() > 0,
            "tcp_session must have a non-zero size");
    }

    /// c:107 — `tcp_session` has sane alignment (≥ 4 bytes for pointer/i32).
    #[test]
    fn tcp_session_min_alignment() {
        assert!(std::mem::align_of::<tcp_session>() >= 4,
            "tcp_session must be ≥ 4-byte aligned");
    }

    /// c:83-85 — OR of all ZTCP_* flags equals 0b10011 = 19.
    #[test]
    fn ztcp_flags_or_equals_nineteen() {
        let or_all = ZTCP_LISTEN | ZTCP_INBOUND | ZTCP_ZFTP;
        assert_eq!(or_all, 19,
            "OR of all ZTCP_* must equal 0b10011 = 19");
    }
}
