//! `ztype.h` port — character classification table + predicates.
//!
//! Port of `Src/ztype.h`. Defines the I* type-bit constants and the
//! predicate macros (`idigit`/`ialnum`/`iblank`/`inblank`/`iword`/...)
//! that consult the `typtab[256]` lookup table. The table itself is
//! initialised by `inittyptab()` (Src/utils.c:4155) at shell startup
//! and refreshed when `IFS` / `WORDCHARS` change.
//!
//! C source: 17 type-bit constants (c:30-46), 1 lookup macro
//! (`zistype`, c:47), 16 predicate macros (`idigit`/`ialnum`/...,
//! c:48-63), 4 typtab-state flags (`ZTF_INIT`/..., c:69-72), plus 3
//! multibyte-conditional convenience macros (`WC_ZISTYPE`/`WC_ISPRINT`/
//! `ZISPRINT`, c:74-90). 0 structs/enums.
//!
//! The Rust port keeps the table as `static TYPTAB: [AtomicU32; 256]`
//! (lock-free Relaxed reads — see the TYPTAB doc below) mirroring C's
//! `mod_export short int typtab[256]` (utils.c:4148), widened to `u32`
//! so `INAMESPC` (`1 << 16`) fits cleanly. `inittyptab` lives in
//! `utils.rs` per its C home (utils.c:4155).

use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Type-bit constants (c:30-46). Each names one column of the
// `typtab[256]` lookup; `OR`-ing them together gives a per-character
// classification bitmap.
// ---------------------------------------------------------------------------
/// `IDIGIT` constant.
pub const IDIGIT: u16 = 1 << 0; // c:30
/// `IALNUM` constant.
pub const IALNUM: u16 = 1 << 1; // c:31
/// `IBLANK` constant.
pub const IBLANK: u16 = 1 << 2; // c:32
/// `INBLANK` constant.
pub const INBLANK: u16 = 1 << 3; // c:33
/// `ITOK` constant.
pub const ITOK: u16 = 1 << 4; // c:34
/// `ISEP` constant.
pub const ISEP: u16 = 1 << 5; // c:35
/// `IALPHA` constant.
pub const IALPHA: u16 = 1 << 6; // c:36
/// `IIDENT` constant.
pub const IIDENT: u16 = 1 << 7; // c:37
/// `IUSER` constant.
pub const IUSER: u16 = 1 << 8; // c:38
/// `ICNTRL` constant.
pub const ICNTRL: u16 = 1 << 9; // c:39
/// `IWORD` constant.
pub const IWORD: u16 = 1 << 10; // c:40
/// `ISPECIAL` constant.
pub const ISPECIAL: u16 = 1 << 11; // c:41
/// `IMETA` constant.
pub const IMETA: u16 = 1 << 12; // c:42
/// `IWSEP` constant.
pub const IWSEP: u16 = 1 << 13; // c:43
/// `INULL` constant.
pub const INULL: u16 = 1 << 14; // c:44
/// `IPATTERN` constant.
pub const IPATTERN: u16 = 1 << 15; // c:45
                                   // INAMESPC is `1 << 16` in C — overflows `short int` (16-bit) on the
                                   // C side, but C's `short` is at least 16-bit and `int` accumulates.
                                   // The Rust port widens TYPTAB to `u32` so this fits cleanly.
/// `INAMESPC` constant.
pub const INAMESPC: u32 = 1 << 16; // c:46

// ---------------------------------------------------------------------------
// the ztypes table                                                          // c:4145
// `typtab[256]` lookup table (utils.c:4148 `mod_export short int
// typtab[256]`).
// ---------------------------------------------------------------------------

/// Port of `mod_export short int typtab[256];` from `Src/utils.c:4148`.
/// Per-byte type-bit lookup. Widened from C's `short int` to `u32` so
/// `INAMESPC` (`1 << 16`) fits.
///
/// Storage is `[AtomicU32; 256]` (Relaxed), not a Mutex: C reads the
/// bare array with zero synchronization, and `zistype` is THE hottest
/// predicate in the shell — every char classification (itype_end,
/// lexer, metafy checks) hit a pthread mutex per byte, which profiled
/// as a top cost in tight shell loops (zpwr expandstats over 42k
/// records). Writers (`inittyptab` family, utils.c:4155) are rare
/// whole-table rebuilds; Relaxed per-slot stores match C's unlocked
/// `typtab[i] = x` semantics.
pub static TYPTAB: [std::sync::atomic::AtomicU32; 256] =
    [const { std::sync::atomic::AtomicU32::new(0) }; 256]; // utils.c:4148

/// Port of `static int typtab_flags = 0;` from `Src/utils.c:4149`.
/// State flags managed by `inittyptab()`. Atomic for the same reason
/// as TYPTAB (C reads it unlocked).
pub static TYPTAB_FLAGS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0); // utils.c:4149

// ZTF_* state flags (c:69-72) preserved across `inittyptab()` calls.
/// `ZTF_INIT` constant.
pub const ZTF_INIT: u32 = 0x0001; // c:69
/// `ZTF_INTERACT` constant.
pub const ZTF_INTERACT: u32 = 0x0002; // c:70
/// `ZTF_SP_COMMA` constant.
pub const ZTF_SP_COMMA: u32 = 0x0004; // c:71
/// `ZTF_BANGCHAR` constant.
pub const ZTF_BANGCHAR: u32 = 0x0008; // c:72

// ---------------------------------------------------------------------------
// `zistype(X, Y)` lookup macro (c:47) and the per-bit predicate
// macros (c:48-63).
//
// C: `#define zistype(X,Y) (typtab[(unsigned char) (X)] & Y)` — one
// table read masked by the requested bit. The result is the bit
// itself (truthy when set, 0 when clear) — C's macro returns int
// directly. Rust port returns `bool` since that's the natural type
// for predicates.
// ---------------------------------------------------------------------------

/// Port of `#define zistype(X,Y)` from `Src/ztype.h:47`. Look up the
/// type-bits for byte `x` and mask with `bits`. Returns true iff any
/// of the requested bits are set.
#[inline]
pub fn zistype(x: u8, bits: u32) -> bool {
    // c:47
    (TYPTAB[x as usize].load(std::sync::atomic::Ordering::Relaxed) & bits) != 0
}

/// Port of `#define idigit(X)` from `Src/ztype.h:48`.
#[inline]
pub fn idigit(x: u8) -> bool {
    zistype(x, IDIGIT as u32)
} // c:48

/// Port of `#define ialnum(X)` from `Src/ztype.h:49`.
#[inline]
pub fn ialnum(x: u8) -> bool {
    zistype(x, IALNUM as u32)
} // c:49

/// Port of `#define iblank(X)` from `Src/ztype.h:50`. Blank, not
/// including `\n`.
// blank, not including \n                                                  // c:50
/// `iblank` — see implementation.
#[inline]
pub fn iblank(x: u8) -> bool {
    zistype(x, IBLANK as u32)
} // c:50

/// Port of `#define inblank(X)` from `Src/ztype.h:51`. Blank or `\n`.
// blank or \n                                                              // c:51
/// `inblank` — see implementation.
#[inline]
pub fn inblank(x: u8) -> bool {
    zistype(x, INBLANK as u32)
} // c:51

/// Port of `#define itok(X)` from `Src/ztype.h:52`.
#[inline]
pub fn itok(x: u8) -> bool {
    zistype(x, ITOK as u32)
} // c:52

/// Port of `#define isep(X)` from `Src/ztype.h:53`.
#[inline]
pub fn isep(x: u8) -> bool {
    zistype(x, ISEP as u32)
} // c:53

/// Port of `#define ialpha(X)` from `Src/ztype.h:54`.
#[inline]
pub fn ialpha(x: u8) -> bool {
    zistype(x, IALPHA as u32)
} // c:54

/// Port of `#define iident(X)` from `Src/ztype.h:55`.
#[inline]
pub fn iident(x: u8) -> bool {
    zistype(x, IIDENT as u32)
} // c:55

/// Port of `#define iuser(X)` from `Src/ztype.h:56`. Username char.
// username char                                                            // c:56
/// `iuser` — see implementation.
#[inline]
pub fn iuser(x: u8) -> bool {
    zistype(x, IUSER as u32)
} // c:56

/// Port of `#define icntrl(X)` from `Src/ztype.h:57`.
#[inline]
pub fn icntrl(x: u8) -> bool {
    zistype(x, ICNTRL as u32)
} // c:57

/// Port of `#define iword(X)` from `Src/ztype.h:58`.
#[inline]
pub fn iword(x: u8) -> bool {
    zistype(x, IWORD as u32)
} // c:58

/// Port of `#define ispecial(X)` from `Src/ztype.h:59`.
#[inline]
pub fn ispecial(x: u8) -> bool {
    zistype(x, ISPECIAL as u32)
} // c:59

/// Port of `#define imeta(X)` from `Src/ztype.h:60`.
#[inline]
pub fn imeta(x: u8) -> bool {
    zistype(x, IMETA as u32)
} // c:60

/// Port of `#define iwsep(X)` from `Src/ztype.h:61`.
#[inline]
pub fn iwsep(x: u8) -> bool {
    zistype(x, IWSEP as u32)
} // c:61

/// Port of `#define inull(X)` from `Src/ztype.h:62`.
#[inline]
pub fn inull(x: u8) -> bool {
    zistype(x, INULL as u32)
} // c:62

/// Port of `#define ipattern(X)` from `Src/ztype.h:63`.
#[inline]
pub fn ipattern(x: u8) -> bool {
    zistype(x, IPATTERN as u32)
} // c:63

// ---------------------------------------------------------------------------
// Multibyte-conditional helpers (c:74-90).
//
// C: `#ifdef MULTIBYTE_SUPPORT` selects `wcsitype` / `iswprint` for
// wide chars, else falls back to the byte-level `zistype` / `isprint`.
// Rust's `char` is always 32-bit Unicode so the multibyte branch is
// always taken; `wcsitype` becomes a TYPTAB lookup on the lower 8
// bits with a Unicode-fallback for high code points.
// ---------------------------------------------------------------------------

/// Port of `#define WC_ZISTYPE(X,Y)` from `Src/ztype.h:75`. Wide-char
/// classification. ASCII path goes through TYPTAB; non-ASCII falls
/// back to Rust's Unicode predicates per the `wcsitype` C body
/// (Src/utils.c). Match the behavior `iword`/`iident`/etc. would
/// produce on the high code points.
#[inline]
#[allow(non_snake_case)]
pub fn WC_ZISTYPE(c: char, bits: u32) -> bool {
    // c:75
    if (c as u32) < 128 {
        zistype(c as u8, bits)
    } else {
        // Non-ASCII: defer to Unicode predicates that match
        // C `wcsitype`'s standard behavior.
        let mut hit = false;
        if (bits & IALPHA as u32) != 0 && c.is_alphabetic() {
            hit = true;
        }
        if (bits & IALNUM as u32) != 0 && c.is_alphanumeric() {
            hit = true;
        }
        if (bits & IDIGIT as u32) != 0 && c.is_numeric() {
            hit = true;
        }
        if (bits & IBLANK as u32) != 0 && (c == ' ' || c == '\t') {
            hit = true;
        }
        if (bits & INBLANK as u32) != 0 && (c == ' ' || c == '\t' || c == '\n') {
            hit = true;
        }
        if (bits & ICNTRL as u32) != 0 && c.is_control() {
            hit = true;
        }
        if (bits & IWORD as u32) != 0 && (c.is_alphanumeric() || c == '_') {
            hit = true;
        }
        if (bits & IIDENT as u32) != 0 && (c.is_alphanumeric() || c == '_') {
            hit = true;
        }
        if (bits & IUSER as u32) != 0 && (c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
            hit = true;
        }
        hit
    }
}

/// Port of `#define WC_ISPRINT(X)` from `Src/ztype.h:79`. Wide-char
/// printable test — `iswprint` in C, `!c.is_control()` in Rust.
#[inline]
#[allow(non_snake_case)]
pub fn WC_ISPRINT(c: char) -> bool {
    // c:79
    !c.is_control()
}

/// Port of `#define ZISPRINT(c)` from `Src/ztype.h:89`. Byte-level
/// printable test. Apple's `BROKEN_ISPRINT` quirk (c:86-89) doesn't
/// apply to the Rust port — `char::is_ascii_graphic` + space matches
/// the standard libc isprint semantics on all targets.
#[inline]
#[allow(non_snake_case)]
pub fn ZISPRINT(c: u8) -> bool {
    // c:89
    c == b' ' || c.is_ascii_graphic()
}

/// Process-wide lock serialising tests that read or write TYPTAB
/// ISEP/IWSEP bits. `ifssetfn` mutates the global IFS + typtab and
/// parallel reads race against the rebuild. No C counterpart.
///
/// Lives at module scope (still `#[cfg(test)]`) so cross-module
/// tests in `input.rs`, `params.rs`, etc. can serialize against the
/// same Mutex.
#[cfg(test)]
pub(crate) static TYPTAB_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies the type-bit constants are non-overlapping single-bit
    /// values per c:30-45. INAMESPC is u32-only (overflows u16).
    #[test]
    fn type_bits_are_distinct() {
        let _g = crate::test_util::global_state_lock();
        let all_u16: u16 = IDIGIT
            | IALNUM
            | IBLANK
            | INBLANK
            | ITOK
            | ISEP
            | IALPHA
            | IIDENT
            | IUSER
            | ICNTRL
            | IWORD
            | ISPECIAL
            | IMETA
            | IWSEP
            | INULL
            | IPATTERN;
        assert_eq!(all_u16.count_ones(), 16);
        assert_eq!(INAMESPC, 0x10000);
    }

    /// Verifies ZTF_* flag values per c:69-72.
    #[test]
    fn ztf_flags_correct() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ZTF_INIT, 0x0001);
        assert_eq!(ZTF_INTERACT, 0x0002);
        assert_eq!(ZTF_SP_COMMA, 0x0004);
        assert_eq!(ZTF_BANGCHAR, 0x0008);
    }

    /// Verifies `zistype` consults TYPTAB and masks (c:47).
    #[test]
    fn zistype_reads_typtab() {
        let _g = crate::test_util::global_state_lock();
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = TYPTAB[b'X' as usize].load(std::sync::atomic::Ordering::Relaxed);
        TYPTAB[b'X' as usize].store(
            (IDIGIT | IALNUM) as u32,
            std::sync::atomic::Ordering::Relaxed,
        );
        assert!(zistype(b'X', IDIGIT as u32));
        assert!(zistype(b'X', IALNUM as u32));
        assert!(!zistype(b'X', ICNTRL as u32));
        TYPTAB[b'X' as usize].store(saved, std::sync::atomic::Ordering::Relaxed);
    }

    /// Verifies the predicate ported dispatch through zistype.
    #[test]
    fn idigit_dispatches_through_typtab() {
        let _g = crate::test_util::global_state_lock();
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = TYPTAB[b'7' as usize].load(std::sync::atomic::Ordering::Relaxed);
        TYPTAB[b'7' as usize].store(IDIGIT as u32, std::sync::atomic::Ordering::Relaxed);
        assert!(idigit(b'7'));
        assert!(!ialpha(b'7'));
        TYPTAB[b'7' as usize].store(saved, std::sync::atomic::Ordering::Relaxed);
    }

    /// Verifies `wc_zistype` falls through to Unicode predicates for
    /// non-ASCII input (c:75 multibyte branch).
    #[test]
    fn wc_zistype_falls_back_to_unicode_for_high_code_points() {
        let _g = crate::test_util::global_state_lock();
        assert!(WC_ZISTYPE('é', IALPHA as u32));
        assert!(WC_ZISTYPE('é', IALNUM as u32));
        assert!(WC_ZISTYPE('é', IWORD as u32));
        assert!(!WC_ZISTYPE('é', IDIGIT as u32));
        let saved = TYPTAB[b'a' as usize].load(std::sync::atomic::Ordering::Relaxed);
        TYPTAB[b'a' as usize].store(IALPHA as u32, std::sync::atomic::Ordering::Relaxed);
        assert!(WC_ZISTYPE('a', IALPHA as u32));
        TYPTAB[b'a' as usize].store(saved, std::sync::atomic::Ordering::Relaxed);
    }

    /// Verifies `wc_isprint` rejects controls per c:79.
    #[test]
    fn wc_isprint_rejects_controls() {
        let _g = crate::test_util::global_state_lock();
        assert!(WC_ISPRINT('a'));
        assert!(WC_ISPRINT(' '));
        assert!(WC_ISPRINT('é'));
        assert!(!WC_ISPRINT('\u{0007}'));
        assert!(!WC_ISPRINT('\u{001b}'));
    }

    /// Verifies `zisprint` accepts space + graphic ASCII per c:89.
    #[test]
    fn zisprint_basic() {
        let _g = crate::test_util::global_state_lock();
        assert!(ZISPRINT(b' '));
        assert!(ZISPRINT(b'a'));
        assert!(ZISPRINT(b'~'));
        assert!(!ZISPRINT(b'\t'));
        assert!(!ZISPRINT(0x7f));
    }

    /// `Src/ztype.h:50` — `#define iblank(X) zistype(X, IBLANK)` —
    /// "blank, not including \\n". `Src/utils.c:4192-4193` ORs IBLANK
    /// only into `typtab[' ']` and `typtab['\t']`. Pin the canonical
    /// IBLANK set so a regression that widens it to include `\n` (or
    /// other iswspace chars) fails immediately.
    #[test]
    fn iblank_matches_ascii_typtab_set() {
        let _g = crate::test_util::global_state_lock();
        // Ensure typtab is initialised — running another test first
        // wins the race; if it hasn't run we seed it explicitly.
        crate::ported::utils::inittyptab();
        assert!(iblank(b' '), "c:4192 — space is IBLANK");
        assert!(iblank(b'\t'), "c:4193 — tab is IBLANK");
        // c:4194 — newline gets INBLANK only, NOT IBLANK.
        assert!(
            !iblank(b'\n'),
            "c:4194 — newline is INBLANK-only, NOT IBLANK"
        );
        assert!(!iblank(b'a'));
        assert!(!iblank(b'\r'), "narrow iblank rejects CR (no typtab entry)");
        assert!(!iblank(b'\x0c'));
    }

    /// `Src/ztype.h:51` — `#define inblank(X) zistype(X, INBLANK)` —
    /// "blank or \\n". `Src/utils.c:4192-4194` ORs INBLANK into space,
    /// tab, AND newline. Pin all three.
    #[test]
    fn inblank_matches_ascii_typtab_set() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::utils::inittyptab();
        assert!(inblank(b' '), "c:4192 — space");
        assert!(inblank(b'\t'), "c:4193 — tab");
        assert!(
            inblank(b'\n'),
            "c:4194 — newline IS INBLANK (the differentiator)"
        );
        // Narrow ASCII inblank does NOT extend to wide whitespace.
        assert!(!inblank(b'\r'), "narrow inblank rejects CR");
        assert!(!inblank(b'\x0c'), "narrow inblank rejects FF");
        assert!(!inblank(b'a'));
    }

    /// `Src/ztype.h:48` — `#define idigit(X) zistype(X, IDIGIT)`. The
    /// typtab init at `Src/utils.c:4178-4180` ORs IDIGIT into
    /// `typtab['0'..='9']`. Pin every digit + a couple of negative
    /// cases.
    #[test]
    fn idigit_covers_all_decimal_digits() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::utils::inittyptab();
        for d in b'0'..=b'9' {
            assert!(idigit(d), "'{}' must be IDIGIT", d as char);
        }
        assert!(!idigit(b'a'));
        assert!(!idigit(b'A'));
        assert!(!idigit(b' '));
    }

    // TYPTAB_TEST_LOCK moved to module scope for cross-module access.
    use super::TYPTAB_TEST_LOCK;

    /// `Src/ztype.h:53` — `#define isep(X) zistype(X, ISEP)`.
    /// `Src/utils.c:4216-4230` walks the IFS string and ORs ISEP on
    /// every char. The DEFAULT_IFS at `Src/zsh.h:149` is
    /// `" \\t\\n\\x83 "` — five bytes that demetafy to space, tab,
    /// newline, and NUL (`Meta+space = 0x00`). Semicolon is NOT in
    /// default IFS.
    #[test]
    fn isep_includes_default_ifs_chars() {
        let _g = crate::test_util::global_state_lock();
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::utils::inittyptab();
        assert!(isep(b' '), "c:4216 — IFS contains space");
        assert!(isep(b'\t'), "c:4216 — IFS contains tab");
        assert!(isep(b'\n'), "c:4216 — IFS contains newline");
        assert!(!isep(b'a'));
        assert!(!isep(b';'), "semicolon is NOT in default IFS");
    }

    /// `Src/ztype.h:61` — `#define iwsep(X) zistype(X, IWSEP)`. ISEP
    /// is the field-separator superset; IWSEP is the *whitespace*
    /// subset (drives the "whitespace-runs collapse" rule). Default
    /// IFS gives IWSEP on space, tab, and newline — every char that
    /// `inblank()` returns true for at the c:4224 check.
    #[test]
    fn iwsep_subset_of_isep_for_inblank_chars() {
        let _g = crate::test_util::global_state_lock();
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::utils::inittyptab();
        assert!(iwsep(b' '), "c:4228 — space is inblank → IWSEP");
        assert!(iwsep(b'\t'), "c:4228 — tab is inblank → IWSEP");
        assert!(iwsep(b'\n'), "c:4228 — newline is inblank → IWSEP");
        // Default IFS also contains Meta+space (→ NUL byte). NUL is
        // NOT inblank, so it gets ISEP but not IWSEP.
        assert!(
            isep(0x00),
            "c:4230 — NUL is in default IFS (Meta+space) → ISEP"
        );
        assert!(!iwsep(0x00), "c:4224 — NUL is NOT inblank → no IWSEP");
    }

    /// `Src/params.c:4795` — `ifssetfn` calls `inittyptab()` so the
    /// ISEP/IWSEP bits get rebuilt from the new IFS. Pin the
    /// rebuild end-to-end: set IFS to `":"`, then verify `isep(':')`
    /// becomes true and old separator chars are dropped.
    #[test]
    fn ifssetfn_rebuilds_isep_typtab_bits() {
        let _g = crate::test_util::global_state_lock();
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::utils::inittyptab();
        let mut __pm = crate::ported::zsh_h::param::default();
        let saved = crate::ported::params::ifsgetfn(&__pm);
        crate::ported::params::ifssetfn(&mut __pm, ":".to_string());
        assert!(
            isep(b':'),
            "c:4795 — new IFS chars must get ISEP after ifssetfn"
        );
        assert!(!isep(b' '), "c:4795 — old IFS chars dropped after ifssetfn");
        assert!(!isep(b'\t'));
        // Restore.
        crate::ported::params::ifssetfn(&mut __pm, saved);
        assert!(isep(b' '), "default IFS restored");
    }

    /// `Src/utils.c:4191` — `typtab['-'] = typtab['.'] = typtab[(unsigned char) Dash] = IUSER`.
    /// All three bytes get IUSER. The Rust port previously omitted
    /// `Dash` (0x9b per `Src/zsh.h:182`) — fixed in
    /// `utils.rs:2880-2883`. Pin all three so a regression reverting
    /// the fix fails.
    #[test]
    fn iuser_includes_dash_dot_and_dash_token() {
        let _g = crate::test_util::global_state_lock();
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::utils::inittyptab();
        assert!(iuser(b'-'), "c:4191 — '-' is IUSER");
        assert!(iuser(b'.'), "c:4191 — '.' is IUSER");
        assert!(iuser(0x9b), "c:4191 — Dash token (0x9b) is IUSER");
        // Negative case: ASCII letters are IUSER (via the 'a'..'z' block at
        // c:4174-4175) but the punctuation bytes around `-`/`.` are NOT.
        assert!(!iuser(b','));
        assert!(!iuser(b';'));
    }

    /// `Src/utils.c:4237-4252` — wordchars walk. ORs IWORD onto every
    /// ASCII byte of `$WORDCHARS` (or DEFAULT_WORDCHARS). The default
    /// is `"*?_-.[]~=/&;!#$%^(){}<>"` (`Src/zsh_system.h:427`).
    /// Pin a sample of those chars getting IWORD. Was missing before
    /// this iteration — pre-fix every glob-pattern compilation that
    /// consults IWORD silently fell through to "non-word".
    #[test]
    fn iword_includes_default_wordchars() {
        let _g = crate::test_util::global_state_lock();
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::utils::inittyptab();
        // Default WORDCHARS chars (DEFAULT_WORDCHARS) all → IWORD.
        for c in b"*?_-./~=&;!" {
            assert!(
                iword(*c),
                "c:4251 — '{}' is in DEFAULT_WORDCHARS and must be IWORD",
                *c as char
            );
        }
        // Alphanumerics are IWORD via the c:4172-4175 init too.
        assert!(iword(b'a'));
        assert!(iword(b'0'));
        // Non-WORDCHAR punctuation NOT in DEFAULT_WORDCHARS → not IWORD.
        // (`,` is not in DEFAULT_WORDCHARS).
        assert!(
            !iword(b','),
            "c:4237 — ',' is NOT in DEFAULT_WORDCHARS, must not be IWORD"
        );
        assert!(!iword(b'\''));
    }

    /// `Src/utils.c:4253-4254` — SPECCHARS walk. ORs ISPECIAL onto
    /// every byte of the hardcoded SPECCHARS string at
    /// `Src/zsh.h:228`. Drives glob-special / quote-special detection
    /// in pattern compilation.
    #[test]
    fn ispecial_includes_specchars_set() {
        let _g = crate::test_util::global_state_lock();
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::utils::inittyptab();
        // Sample of SPECCHARS = "#$^*()=|{}[]`<>?~;&\n\t \\\'\"".
        for c in b"#$^*()=|{}[]`<>?~;&" {
            assert!(
                ispecial(*c),
                "c:4254 — '{}' is in SPECCHARS and must be ISPECIAL",
                *c as char
            );
        }
        assert!(ispecial(b'\n'), "newline in SPECCHARS");
        assert!(ispecial(b'\t'), "tab in SPECCHARS");
        assert!(ispecial(b' '), "space in SPECCHARS");
        // Letters/digits NOT in SPECCHARS.
        assert!(!ispecial(b'a'));
        assert!(!ispecial(b'0'));
    }

    /// `Src/utils.c:4262-4263` — PATCHARS walk. ORs IPATTERN onto
    /// every byte of `Src/zsh.h:232 PATCHARS = "#^*()|[]<>?~\\"`.
    /// Used by pattern compilation to detect glob metachars.
    #[test]
    fn ipattern_includes_patchars_set() {
        let _g = crate::test_util::global_state_lock();
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::utils::inittyptab();
        // Every char in PATCHARS = "#^*()|[]<>?~\\".
        for c in b"#^*()|[]<>?~\\" {
            assert!(
                ipattern(*c),
                "c:4263 — '{}' is in PATCHARS and must be IPATTERN",
                *c as char
            );
        }
        // Non-pattern chars must NOT be IPATTERN.
        assert!(!ipattern(b'a'));
        assert!(!ipattern(b'0'));
        // `$` is in SPECCHARS but NOT PATCHARS — pin the distinction.
        assert!(!ipattern(b'$'), "c:4263 — '$' is SPECCHARS not PATCHARS");
    }

    /// `Src/params.c:5143` — `wordcharssetfn` calls `inittyptab()`
    /// to rebuild IWORD typtab bits from new WORDCHARS. Pin the
    /// rebuild end-to-end: set WORDCHARS to `":"`, then verify
    /// `iword(':')` becomes true and old WORDCHAR chars are dropped.
    #[test]
    fn wordcharssetfn_rebuilds_iword_typtab_bits() {
        let _g = crate::test_util::global_state_lock();
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::utils::inittyptab();
        let mut __pm = crate::ported::zsh_h::param::default();
        let saved = crate::ported::params::wordcharsgetfn(&__pm);
        crate::ported::params::wordcharssetfn(&mut __pm, ":".to_string());
        assert!(iword(b':'), "c:5143 — new WORDCHARS member must get IWORD");
        // Old DEFAULT_WORDCHARS chars (e.g. `*`, `?`) lose IWORD when
        // WORDCHARS becomes ":".
        assert!(
            !iword(b'*'),
            "c:5143 — old WORDCHAR `*` dropped after wordcharssetfn(\":\")"
        );
        // Alphanumerics retain IWORD via c:4172-4175 init (NOT
        // dependent on WORDCHARS).
        assert!(iword(b'a'));
        assert!(iword(b'0'));
        // Restore.
        crate::ported::params::wordcharssetfn(&mut __pm, saved);
        assert!(iword(b'*'), "DEFAULT_WORDCHARS restored");
    }

    /// `Src/utils.c:4255-4256` — `if (typtab_flags & ZTF_SP_COMMA)
    /// typtab[','] |= ISPECIAL`. `makecommaspecial(true)` sets the
    /// flag bit; subsequent `inittyptab()` re-applies the comma bit.
    /// Pin the round-trip: enable → re-init → comma is ISPECIAL.
    #[test]
    fn inittyptab_honours_ztf_sp_comma_flag() {
        let _g = crate::test_util::global_state_lock();
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Start clean.
        crate::ported::utils::inittyptab();
        assert!(!ispecial(b','), "comma is NOT ISPECIAL by default");
        // Enable.
        crate::ported::utils::makecommaspecial(true);
        // Run a fresh init — the c:4255 conditional must re-OR the bit.
        crate::ported::utils::inittyptab();
        assert!(
            ispecial(b','),
            "c:4256 — comma must be ISPECIAL after makecommaspecial(true)"
        );
        // Disable + re-init: comma drops back to non-special.
        crate::ported::utils::makecommaspecial(false);
        crate::ported::utils::inittyptab();
        assert!(
            !ispecial(b','),
            "c:4255 — comma reverts when ZTF_SP_COMMA clear"
        );
    }

    /// `Src/utils.c:4169-4171` — `typtab[t0] = typtab[t0+128] = ICNTRL`
    /// for `t0` in 0..32, plus `typtab[127] = ICNTRL`. C uses `=`
    /// (overwrite) so subsequent assignments at c:4190-4197 can
    /// reclassify specific bytes (e.g. `typtab[(unsigned char) Dash]
    /// = IUSER` at c:4191 overwrites the prior ICNTRL on byte 0x9b).
    /// Pin the C0 + DEL ranges; for C1 (128+) skip bytes that get
    /// reassigned later in the init.
    #[test]
    fn icntrl_covers_c0_controls_and_del() {
        let _g = crate::test_util::global_state_lock();
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::utils::inittyptab();
        for b in 0u8..32u8 {
            assert!(icntrl(b), "c:4169 — byte 0x{:02x} (C0) must be ICNTRL", b);
        }
        assert!(icntrl(127), "c:4171 — DEL (0x7f) is ICNTRL");
        // C1 controls (0x80..0xa0): all ICNTRL EXCEPT bytes the later
        // `=` lines reclassify (Meta=0x83, Marker, Pound..Nularg
        // token range, Dash=0x9b).
        // Spot-check a few definitely-still-control C1 bytes.
        assert!(icntrl(0x81), "c:4170 — byte 0x81 stays ICNTRL");
        assert!(icntrl(0x82), "c:4170 — byte 0x82 stays ICNTRL");
        // 0x9b (Dash) gets overwritten to IUSER per c:4191. NOT ICNTRL.
        assert!(!icntrl(0x9b), "c:4191 — Dash overwrites ICNTRL → IUSER");
        // Printable boundary.
        assert!(!icntrl(b' '));
        assert!(!icntrl(b'a'));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Per-predicate coverage of ztype_h classifiers. Each fn maps to a
    // typtab bit (IDIGIT, IALPHA, IBLANK, etc.) populated by inittyptab.
    // ═══════════════════════════════════════════════════════════════════

    fn with_typtab<F: FnOnce()>(body: F) {
        let _g = crate::test_util::global_state_lock();
        let _g2 = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::utils::inittyptab();
        body();
    }

    // ── idigit: ASCII digits 0..9 only ─────────────────────────────
    #[test]
    fn idigit_recognizes_ascii_digits() {
        with_typtab(|| {
            for d in b'0'..=b'9' {
                assert!(idigit(d), "{:?} must be IDIGIT", d as char);
            }
        });
    }

    #[test]
    fn idigit_rejects_non_digit_ascii() {
        with_typtab(|| {
            for c in [b'a', b'Z', b' ', b'.', b'-', b'!', 0] {
                assert!(!idigit(c), "{c:?} must NOT be IDIGIT");
            }
        });
    }

    // ── ialnum: digits + letters ───────────────────────────────────
    #[test]
    fn ialnum_recognizes_letters_and_digits() {
        with_typtab(|| {
            for c in b'a'..=b'z' {
                assert!(ialnum(c));
            }
            for c in b'A'..=b'Z' {
                assert!(ialnum(c));
            }
            for c in b'0'..=b'9' {
                assert!(ialnum(c));
            }
        });
    }

    #[test]
    fn ialnum_rejects_punctuation_and_whitespace() {
        with_typtab(|| {
            for c in [b' ', b'\t', b'!', b'@', b'.', b'/', b'-'] {
                assert!(!ialnum(c), "{c:?} must not be IALNUM");
            }
        });
    }

    // ── iblank: space or tab ───────────────────────────────────────
    #[test]
    fn iblank_matches_space_and_tab() {
        with_typtab(|| {
            assert!(iblank(b' '));
            assert!(iblank(b'\t'));
        });
    }

    #[test]
    fn iblank_rejects_newline_and_other_whitespace() {
        with_typtab(|| {
            // Per c:50 — IBLANK is space/tab ONLY. Newline isn't IBLANK.
            assert!(!iblank(b'\n'));
            assert!(!iblank(b'\r'));
            assert!(!iblank(b'\x0c')); // form feed
        });
    }

    // ── inblank: zsh extended "narrow blank" ───────────────────────
    #[test]
    fn inblank_matches_basic_whitespace() {
        with_typtab(|| {
            assert!(inblank(b' '));
            assert!(inblank(b'\t'));
        });
    }

    // ── ialpha: letters only ───────────────────────────────────────
    #[test]
    fn ialpha_matches_all_ascii_letters() {
        with_typtab(|| {
            for c in b'a'..=b'z' {
                assert!(ialpha(c));
            }
            for c in b'A'..=b'Z' {
                assert!(ialpha(c));
            }
        });
    }

    #[test]
    fn ialpha_rejects_digits_and_underscore() {
        with_typtab(|| {
            for c in b'0'..=b'9' {
                assert!(!ialpha(c), "{:?} digit must NOT be IALPHA", c as char);
            }
            // Underscore is IIDENT but NOT IALPHA.
            assert!(!ialpha(b'_'));
        });
    }

    // ── iident: identifier chars (letters + digits + underscore) ───
    #[test]
    fn iident_matches_letters_digits_and_underscore() {
        with_typtab(|| {
            assert!(iident(b'a'));
            assert!(iident(b'Z'));
            assert!(iident(b'0'));
            assert!(iident(b'_'));
        });
    }

    #[test]
    fn iident_rejects_punct() {
        with_typtab(|| {
            for c in [b' ', b'.', b'-', b'/', b'!', b'@', b'#'] {
                assert!(!iident(c), "{c:?} must NOT be IIDENT");
            }
        });
    }

    // ── iword: zsh word-char (matches WORDCHARS setting + default) ─
    #[test]
    fn iword_includes_letters_and_digits_at_minimum() {
        with_typtab(|| {
            assert!(iword(b'a'));
            assert!(iword(b'Z'));
            assert!(iword(b'5'));
        });
    }

    // ── iwsep: word separator ──────────────────────────────────────
    #[test]
    fn iwsep_includes_space_and_tab_at_minimum() {
        with_typtab(|| {
            assert!(iwsep(b' '));
            assert!(iwsep(b'\t'));
        });
    }

    // ── icntrl: control chars (0x00-0x1F + 0x7F) ───────────────────
    #[test]
    fn icntrl_matches_low_control_chars() {
        with_typtab(|| {
            // 0x01..=0x1f are control (skip 0x00 — special handling).
            for c in 0x01..=0x1fu8 {
                assert!(icntrl(c), "{c:#04x} must be ICNTRL");
            }
            assert!(icntrl(0x7f), "DEL is ICNTRL");
        });
    }

    // ── isep: ISEP is the IFS-default bitset (space, tab, newline) ──
    /// `isep` matches IFS chars (space/tab/newline) NOT shell `;`/`&`
    /// (those go through ISEP elsewhere via a different path).
    #[test]
    fn isep_includes_ifs_chars_only() {
        with_typtab(|| {
            assert!(isep(b' '));
            assert!(isep(b'\t'));
            assert!(isep(b'\n'));
            // Shell metas are NOT in the IFS-default set.
            assert!(!isep(b';'), "`;` is not IFS-default ISEP");
            assert!(!isep(b'&'), "`&` is not IFS-default ISEP");
        });
    }

    // ── inull: zsh null tokens (Snull..Nularg range) ───────────────
    #[test]
    fn inull_matches_zsh_null_byte_range() {
        with_typtab(|| {
            // Snull = 0x9d, Nularg = 0xa1.
            assert!(inull(0x9d));
            assert!(inull(0xa1));
            // Bytes outside the range are NOT inull.
            assert!(!inull(b'a'));
            assert!(!inull(b'0'));
        });
    }

    // ── ipattern: pattern meta-chars (raw ASCII bytes aren't pattern) ─
    #[test]
    fn ipattern_rejects_plain_ascii() {
        with_typtab(|| {
            // Pattern bits are set on metafied tokens (Star byte etc.),
            // NOT on raw ASCII chars. Plain `*` byte is just printable.
            assert!(!ipattern(b'a'));
            assert!(!ipattern(b'0'));
        });
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/ztype.h classifiers.
    // ═══════════════════════════════════════════════════════════════════

    /// c:48 — `idigit('5')` true, `idigit('a')` false.
    #[test]
    fn idigit_distinguishes_digit_from_letter() {
        with_typtab(|| {
            assert!(idigit(b'5'));
            assert!(!idigit(b'a'));
            assert!(!idigit(b'Z'));
            assert!(!idigit(b' '));
        });
    }

    /// c:48 — `idigit` for 0..9 all true.
    #[test]
    fn idigit_all_ascii_digits() {
        with_typtab(|| {
            for d in b'0'..=b'9' {
                assert!(idigit(d), "{:?} must be idigit", d as char);
            }
        });
    }

    /// c:54 — `ialpha` for ASCII letters returns true.
    #[test]
    fn ialpha_ascii_letters() {
        with_typtab(|| {
            for c in b'a'..=b'z' {
                assert!(ialpha(c), "{:?} must be ialpha", c as char);
            }
            for c in b'A'..=b'Z' {
                assert!(ialpha(c), "{:?} must be ialpha", c as char);
            }
        });
    }

    /// c:54 — `ialpha` rejects digits and punct.
    #[test]
    fn ialpha_rejects_non_letter() {
        with_typtab(|| {
            assert!(!ialpha(b'5'));
            assert!(!ialpha(b'.'));
            assert!(!ialpha(b' '));
            assert!(!ialpha(b'_'));
        });
    }

    /// c:55 — `iident` includes alphanumeric + underscore.
    #[test]
    fn iident_includes_underscore() {
        with_typtab(|| {
            assert!(iident(b'a'));
            assert!(iident(b'Z'));
            assert!(iident(b'5'));
            assert!(iident(b'_'));
            assert!(!iident(b'.'));
            assert!(!iident(b'-'));
            assert!(!iident(b' '));
        });
    }

    /// c:50 — `iblank` includes space + tab, excludes newline.
    #[test]
    fn iblank_space_tab_not_newline() {
        with_typtab(|| {
            assert!(iblank(b' '));
            assert!(iblank(b'\t'));
            assert!(!iblank(b'\n'), "newline NOT in iblank");
        });
    }

    /// c:51 — `inblank` includes space, tab, AND newline.
    #[test]
    fn inblank_includes_newline() {
        with_typtab(|| {
            assert!(inblank(b' '));
            assert!(inblank(b'\t'));
            assert!(inblank(b'\n'), "inblank includes newline");
        });
    }

    /// c:57 — `icntrl` recognizes ASCII control chars.
    #[test]
    fn icntrl_recognizes_control_chars() {
        with_typtab(|| {
            assert!(icntrl(0x00));
            assert!(icntrl(0x01));
            assert!(icntrl(0x1f));
            assert!(!icntrl(b' '));
            assert!(!icntrl(b'a'));
        });
    }

    /// c:53 — `isep` recognizes IFS separators (space, tab, newline).
    #[test]
    fn isep_recognizes_ifs_separators() {
        with_typtab(|| {
            assert!(isep(b' '));
            assert!(isep(b'\t'));
            assert!(isep(b'\n'));
            assert!(!isep(b'a'));
        });
    }

    /// c:47 — `zistype(byte, IDIGIT)` matches idigit for all bytes.
    #[test]
    fn zistype_matches_direct_predicates() {
        with_typtab(|| {
            for b in 0u8..=127 {
                assert_eq!(zistype(b, IDIGIT as u32), idigit(b));
                assert_eq!(zistype(b, IALPHA as u32), ialpha(b));
                assert_eq!(zistype(b, IBLANK as u32), iblank(b));
            }
        });
    }

    /// c:47 — `zistype` is deterministic for a given (byte, bits) pair.
    #[test]
    fn zistype_is_deterministic() {
        with_typtab(|| {
            for b in &[b'a', b'5', b' ', b'\n', b'_'] {
                for bits in [IDIGIT, IALPHA, IBLANK, IIDENT] {
                    let first = zistype(*b, bits as u32);
                    for _ in 0..3 {
                        assert_eq!(zistype(*b, bits as u32), first);
                    }
                }
            }
        });
    }

    /// c:60 — `imeta(0x83)` returns true (Meta lead byte).
    #[test]
    fn imeta_recognizes_meta_lead_byte() {
        with_typtab(|| {
            // Meta = 0x83 per zsh.h. The IMETA bit includes Meta + token range.
            assert!(imeta(0x83));
        });
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/ztype.h
    // c:47 zistype / c:52 itok / c:75 WC_ZISTYPE / c:89 ZISPRINT
    // ═══════════════════════════════════════════════════════════════════

    /// c:47 — every predicate returns bool (compile-time type pin).
    #[test]
    fn all_predicates_return_bool_type() {
        let _: bool = idigit(b'5');
        let _: bool = ialnum(b'a');
        let _: bool = iblank(b' ');
        let _: bool = inblank(b'\n');
        let _: bool = itok(0);
        let _: bool = isep(b' ');
        let _: bool = ialpha(b'a');
        let _: bool = iident(b'_');
        let _: bool = iuser(b'.');
        let _: bool = icntrl(0);
        let _: bool = iword(b'a');
        let _: bool = ispecial(b'$');
        let _: bool = imeta(0x83);
        let _: bool = iwsep(b' ');
        let _: bool = inull(0);
        let _: bool = ipattern(b'*');
    }

    /// c:52 — `itok` returns false for plain ASCII letters/digits.
    /// Token bytes are zsh-internal sentinels in the Meta range.
    #[test]
    fn itok_false_for_plain_ascii() {
        with_typtab(|| {
            for b in [b'a', b'Z', b'5', b' ', b'\t', b'\n', b'.', b'_'] {
                assert!(
                    !itok(b),
                    "plain ASCII byte {:?} (0x{:02x}) must NOT be a token",
                    b as char,
                    b
                );
            }
        });
    }

    /// c:47 — `zistype(byte, 0)` always returns false (no bits → no match).
    #[test]
    fn zistype_zero_bits_always_false() {
        with_typtab(|| {
            for b in 0u8..=127 {
                assert!(!zistype(b, 0), "byte 0x{:02x} with bits=0 must be false", b);
            }
        });
    }

    /// c:89 — `ZISPRINT(NUL)` returns false (NUL is not printable).
    #[test]
    fn zisprint_nul_returns_false() {
        assert!(!ZISPRINT(0), "NUL byte never printable");
    }

    /// c:89 — `ZISPRINT(DEL)` returns false (DEL is control).
    #[test]
    fn zisprint_del_returns_false() {
        assert!(!ZISPRINT(0x7f), "DEL is control char");
    }

    /// c:89 — `ZISPRINT` is deterministic for the same byte.
    #[test]
    fn zisprint_is_deterministic() {
        for b in 0u8..=127 {
            let first = ZISPRINT(b);
            for _ in 0..3 {
                assert_eq!(
                    ZISPRINT(b),
                    first,
                    "ZISPRINT(0x{:02x}) must be deterministic",
                    b
                );
            }
        }
    }

    /// c:79 — `WC_ISPRINT` is deterministic for fixed char.
    #[test]
    fn wc_isprint_is_deterministic() {
        for c in ['a', ' ', '0', '\n', '\t', 'é', '日', '\0', '\u{0007}'] {
            let first = WC_ISPRINT(c);
            for _ in 0..3 {
                assert_eq!(
                    WC_ISPRINT(c),
                    first,
                    "WC_ISPRINT({:?}) must be deterministic",
                    c
                );
            }
        }
    }

    /// c:75 — `WC_ZISTYPE` ASCII path matches `zistype` on the byte.
    #[test]
    fn wc_zistype_ascii_matches_zistype() {
        with_typtab(|| {
            for b in 0u8..128 {
                let c = b as char;
                for bits in [IDIGIT, IALPHA, IBLANK, IIDENT] {
                    assert_eq!(
                        WC_ZISTYPE(c, bits as u32),
                        zistype(b, bits as u32),
                        "WC_ZISTYPE({:?}, {}) must match zistype on ASCII",
                        c,
                        bits
                    );
                }
            }
        });
    }

    /// c:75 — `WC_ZISTYPE` on high CJK char `日` matches IALPHA/IALNUM/IWORD.
    #[test]
    fn wc_zistype_cjk_alphabetic_matches_word_bits() {
        assert!(WC_ZISTYPE('日', IALPHA as u32), "CJK char is alphabetic");
        assert!(WC_ZISTYPE('日', IALNUM as u32), "CJK char is alphanumeric");
        assert!(WC_ZISTYPE('日', IWORD as u32), "CJK char is a word char");
        assert!(!WC_ZISTYPE('日', IDIGIT as u32), "CJK char is NOT a digit");
    }

    /// c:62 — `inull(0)` (NUL byte) reflects whatever typtab says — pin no-panic.
    #[test]
    fn inull_nul_byte_no_panic() {
        with_typtab(|| {
            let _: bool = inull(0);
        });
    }

    /// c:75 — `WC_ZISTYPE` empty bits = 0 → always false (no matched class).
    #[test]
    fn wc_zistype_zero_bits_always_false() {
        for c in ['a', ' ', '0', 'é', '日'] {
            assert!(!WC_ZISTYPE(c, 0), "WC_ZISTYPE({:?}, 0) must be false", c);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/ztype.h
    // c:30-46 I* class bits / c:69-72 ZTF_* flags /
    // c:105 zistype / c:112-209 i* predicates
    // ═══════════════════════════════════════════════════════════════════

    /// c:30-45 — 16 main I* class bits (IDIGIT..IPATTERN) are all single
    /// bits in low 16 bits.
    #[test]
    fn i_class_bits_low_16_pairwise_distinct() {
        let bits = [
            IDIGIT, IALNUM, IBLANK, INBLANK, ITOK, ISEP, IALPHA, IIDENT, IUSER, ICNTRL, IWORD,
            ISPECIAL, IMETA, IWSEP, INULL, IPATTERN,
        ];
        let unique: std::collections::HashSet<_> = bits.iter().copied().collect();
        assert_eq!(
            unique.len(),
            bits.len(),
            "I* main class bits must be pairwise distinct"
        );
    }

    /// c:30-45 — every I* main class bit is a single bit (power of 2).
    #[test]
    fn i_class_bits_all_powers_of_two() {
        for v in [
            IDIGIT, IALNUM, IBLANK, INBLANK, ITOK, ISEP, IALPHA, IIDENT, IUSER, ICNTRL, IWORD,
            ISPECIAL, IMETA, IWSEP, INULL, IPATTERN,
        ] {
            assert!(v.is_power_of_two(), "I* {} must be single bit", v);
        }
    }

    /// c:30-45 — OR of all 16 main I* bits = 0xffff (full low 16 coverage).
    #[test]
    fn i_class_bits_or_covers_low_16() {
        let or_all = IDIGIT
            | IALNUM
            | IBLANK
            | INBLANK
            | ITOK
            | ISEP
            | IALPHA
            | IIDENT
            | IUSER
            | ICNTRL
            | IWORD
            | ISPECIAL
            | IMETA
            | IWSEP
            | INULL
            | IPATTERN;
        assert_eq!(
            or_all, 0xffff_u16,
            "16 main I* bits must cover low 16 (no gaps)"
        );
    }

    /// c:46 — INAMESPC sits in bit 16 (lives in the u32 extended namespace).
    #[test]
    fn inamespc_is_bit_16() {
        assert_eq!(
            INAMESPC,
            1u32 << 16,
            "INAMESPC must be bit 16 (above the u16 main I* set)"
        );
    }

    /// c:69-72 — ZTF_* flags pairwise distinct.
    #[test]
    fn ztf_flags_pairwise_distinct() {
        let bits = [ZTF_INIT, ZTF_INTERACT, ZTF_SP_COMMA, ZTF_BANGCHAR];
        let unique: std::collections::HashSet<_> = bits.iter().copied().collect();
        assert_eq!(unique.len(), bits.len(), "ZTF_* must be distinct");
    }

    /// c:69-72 — every ZTF_* is a single bit (power of 2).
    #[test]
    fn ztf_flags_all_powers_of_two() {
        for v in [ZTF_INIT, ZTF_INTERACT, ZTF_SP_COMMA, ZTF_BANGCHAR] {
            assert!(v.is_power_of_two(), "ZTF_* {:#x} must be single bit", v);
        }
    }

    /// c:69-72 — ZTF_* OR equals 0x000f (covers low 4 bits).
    #[test]
    fn ztf_flags_or_covers_low_4_bits() {
        let or_all = ZTF_INIT | ZTF_INTERACT | ZTF_SP_COMMA | ZTF_BANGCHAR;
        assert_eq!(or_all, 0x000f, "ZTF_* must cover bits 0..=3");
    }

    /// c:112 — `idigit('0'..'9')` true; ASCII letters false.
    #[test]
    fn idigit_recognizes_ascii_digits_only() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::utils::inittyptab();
        for d in b'0'..=b'9' {
            assert!(idigit(d), "{:?} must be idigit", d as char);
        }
        for c in [b'a', b'A', b'_', b' ', b'!'] {
            assert!(!idigit(c), "{:?} must NOT be idigit", c as char);
        }
    }

    /// c:118 — `ialnum` covers digits + letters.
    #[test]
    fn ialnum_covers_digits_and_ascii_letters() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::utils::inittyptab();
        for c in [b'0', b'5', b'9', b'a', b'M', b'Z'] {
            assert!(ialnum(c), "{:?} must be ialnum", c as char);
        }
        for c in [b' ', b'!', b'.', b'\t'] {
            assert!(!ialnum(c), "{:?} must NOT be ialnum", c as char);
        }
    }

    /// c:173 — `icntrl` recognizes control bytes (0x00..0x1f, 0x7f).
    #[test]
    fn icntrl_recognizes_control_bytes() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::utils::inittyptab();
        for c in [0u8, 0x01, 0x1f, 0x7f] {
            assert!(icntrl(c), "{:#x} must be icntrl", c);
        }
        for c in [b' ', b'a', b'5'] {
            assert!(!icntrl(c), "{:?} must NOT be icntrl", c as char);
        }
    }

    /// c:105 — `zistype(x, all bits)` matches at least one class for ASCII letters.
    #[test]
    fn zistype_letters_match_some_class() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::utils::inittyptab();
        // ASCII letters are in IALNUM and IALPHA at minimum.
        for c in [b'a', b'X', b'm'] {
            assert!(
                zistype(c, IALNUM as u32),
                "{:?} must match IALNUM",
                c as char
            );
            assert!(
                zistype(c, IALPHA as u32),
                "{:?} must match IALPHA",
                c as char
            );
        }
    }

    /// c:105 — `zistype` with all-bits-set returns true for any classified char.
    #[test]
    fn zistype_full_mask_returns_true_for_any_class() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::utils::inittyptab();
        let full: u32 = u32::MAX;
        // Pick chars that definitely have at least one class bit.
        for c in [b'a', b'0', b' ', b'_'] {
            assert!(
                zistype(c, full),
                "{:?} with full mask must hit some class",
                c as char
            );
        }
    }
}
