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
//! The Rust port keeps the table as `static TYPTAB: Mutex<[u32; 256]>`
//! to mirror C's `mod_export short int typtab[256]` (utils.c:4148),
//! widened to `u32` so `INAMESPC` (`1 << 16`) fits cleanly.
//! `inittyptab` lives in `utils.rs` per its C home (utils.c:4155).

use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Type-bit constants (c:30-46). Each names one column of the
// `typtab[256]` lookup; `OR`-ing them together gives a per-character
// classification bitmap.
// ---------------------------------------------------------------------------

pub const IDIGIT:   u16 = 1 <<  0;                                       // c:30
pub const IALNUM:   u16 = 1 <<  1;                                       // c:31
pub const IBLANK:   u16 = 1 <<  2;                                       // c:32
pub const INBLANK:  u16 = 1 <<  3;                                       // c:33
pub const ITOK:     u16 = 1 <<  4;                                       // c:34
pub const ISEP:     u16 = 1 <<  5;                                       // c:35
pub const IALPHA:   u16 = 1 <<  6;                                       // c:36
pub const IIDENT:   u16 = 1 <<  7;                                       // c:37
pub const IUSER:    u16 = 1 <<  8;                                       // c:38
pub const ICNTRL:   u16 = 1 <<  9;                                       // c:39
pub const IWORD:    u16 = 1 << 10;                                       // c:40
pub const ISPECIAL: u16 = 1 << 11;                                       // c:41
pub const IMETA:    u16 = 1 << 12;                                       // c:42
pub const IWSEP:    u16 = 1 << 13;                                       // c:43
pub const INULL:    u16 = 1 << 14;                                       // c:44
pub const IPATTERN: u16 = 1 << 15;                                       // c:45
// INAMESPC is `1 << 16` in C — overflows `short int` (16-bit) on the
// C side, but C's `short` is at least 16-bit and `int` accumulates.
// The Rust port widens TYPTAB to `u32` so this fits cleanly.
pub const INAMESPC: u32 = 1 << 16;                                       // c:46

// ---------------------------------------------------------------------------
// the ztypes table                                                          // c:4145
// `typtab[256]` lookup table (utils.c:4148 `mod_export short int
// typtab[256]`).
// ---------------------------------------------------------------------------

/// Port of `mod_export short int typtab[256];` from `Src/utils.c:4148`.
/// Per-byte type-bit lookup. Widened from C's `short int` to `u32` so
/// `INAMESPC` (`1 << 16`) fits.
pub static TYPTAB: Mutex<[u32; 256]> = Mutex::new([0; 256]);             // utils.c:4148

/// Port of `static int typtab_flags = 0;` from `Src/utils.c:4149`.
/// State flags managed by `inittyptab()`.
pub static TYPTAB_FLAGS: Mutex<u32> = Mutex::new(0);                     // utils.c:4149

// ZTF_* state flags (c:69-72) preserved across `inittyptab()` calls.
pub const ZTF_INIT:     u32 = 0x0001;                                    // c:69
pub const ZTF_INTERACT: u32 = 0x0002;                                    // c:70
pub const ZTF_SP_COMMA: u32 = 0x0004;                                    // c:71
pub const ZTF_BANGCHAR: u32 = 0x0008;                                    // c:72

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
pub fn zistype(x: u8, bits: u32) -> bool {                               // c:47
    (TYPTAB.lock().unwrap()[x as usize] & bits) != 0
}

/// Port of `#define idigit(X)` from `Src/ztype.h:48`.
#[inline] pub fn idigit(x: u8) -> bool { zistype(x, IDIGIT as u32) }     // c:48

/// Port of `#define ialnum(X)` from `Src/ztype.h:49`.
#[inline] pub fn ialnum(x: u8) -> bool { zistype(x, IALNUM as u32) }     // c:49

/// Port of `#define iblank(X)` from `Src/ztype.h:50`. Blank, not
/// including `\n`.
// blank, not including \n                                                  // c:50
#[inline] pub fn iblank(x: u8) -> bool { zistype(x, IBLANK as u32) }     // c:50

/// Port of `#define inblank(X)` from `Src/ztype.h:51`. Blank or `\n`.
// blank or \n                                                              // c:51
#[inline] pub fn inblank(x: u8) -> bool { zistype(x, INBLANK as u32) }   // c:51

/// Port of `#define itok(X)` from `Src/ztype.h:52`.
#[inline] pub fn itok(x: u8) -> bool { zistype(x, ITOK as u32) }         // c:52

/// Port of `#define isep(X)` from `Src/ztype.h:53`.
#[inline] pub fn isep(x: u8) -> bool { zistype(x, ISEP as u32) }         // c:53

/// Port of `#define ialpha(X)` from `Src/ztype.h:54`.
#[inline] pub fn ialpha(x: u8) -> bool { zistype(x, IALPHA as u32) }     // c:54

/// Port of `#define iident(X)` from `Src/ztype.h:55`.
#[inline] pub fn iident(x: u8) -> bool { zistype(x, IIDENT as u32) }     // c:55

/// Port of `#define iuser(X)` from `Src/ztype.h:56`. Username char.
// username char                                                            // c:56
#[inline] pub fn iuser(x: u8) -> bool { zistype(x, IUSER as u32) }       // c:56

/// Port of `#define icntrl(X)` from `Src/ztype.h:57`.
#[inline] pub fn icntrl(x: u8) -> bool { zistype(x, ICNTRL as u32) }     // c:57

/// Port of `#define iword(X)` from `Src/ztype.h:58`.
#[inline] pub fn iword(x: u8) -> bool { zistype(x, IWORD as u32) }       // c:58

/// Port of `#define ispecial(X)` from `Src/ztype.h:59`.
#[inline] pub fn ispecial(x: u8) -> bool { zistype(x, ISPECIAL as u32) } // c:59

/// Port of `#define imeta(X)` from `Src/ztype.h:60`.
#[inline] pub fn imeta(x: u8) -> bool { zistype(x, IMETA as u32) }       // c:60

/// Port of `#define iwsep(X)` from `Src/ztype.h:61`.
#[inline] pub fn iwsep(x: u8) -> bool { zistype(x, IWSEP as u32) }       // c:61

/// Port of `#define inull(X)` from `Src/ztype.h:62`.
#[inline] pub fn inull(x: u8) -> bool { zistype(x, INULL as u32) }       // c:62

/// Port of `#define ipattern(X)` from `Src/ztype.h:63`.
#[inline] pub fn ipattern(x: u8) -> bool { zistype(x, IPATTERN as u32) } // c:63

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
pub fn WC_ZISTYPE(c: char, bits: u32) -> bool {                          // c:75
    if (c as u32) < 128 {
        zistype(c as u8, bits)
    } else {
        // Non-ASCII: defer to Unicode predicates that match
        // C `wcsitype`'s standard behavior.
        let mut hit = false;
        if (bits & IALPHA as u32) != 0 && c.is_alphabetic()         { hit = true; }
        if (bits & IALNUM as u32) != 0 && c.is_alphanumeric()       { hit = true; }
        if (bits & IDIGIT as u32) != 0 && c.is_numeric()            { hit = true; }
        if (bits & IBLANK as u32) != 0 && (c == ' ' || c == '\t')   { hit = true; }
        if (bits & INBLANK as u32) != 0 && (c == ' ' || c == '\t' || c == '\n') { hit = true; }
        if (bits & ICNTRL as u32) != 0 && c.is_control()            { hit = true; }
        if (bits & IWORD as u32) != 0 && (c.is_alphanumeric() || c == '_') { hit = true; }
        if (bits & IIDENT as u32) != 0 && (c.is_alphanumeric() || c == '_') { hit = true; }
        if (bits & IUSER as u32) != 0 && (c.is_alphanumeric() || c == '_' || c == '-' || c == '.') { hit = true; }
        hit
    }
}

/// Port of `#define WC_ISPRINT(X)` from `Src/ztype.h:79`. Wide-char
/// printable test — `iswprint` in C, `!c.is_control()` in Rust.
#[inline]
#[allow(non_snake_case)]
pub fn WC_ISPRINT(c: char) -> bool {                                     // c:79
    !c.is_control()
}

/// Port of `#define ZISPRINT(c)` from `Src/ztype.h:89`. Byte-level
/// printable test. Apple's `BROKEN_ISPRINT` quirk (c:86-89) doesn't
/// apply to the Rust port — `char::is_ascii_graphic` + space matches
/// the standard libc isprint semantics on all targets.
#[inline]
#[allow(non_snake_case)]
pub fn ZISPRINT(c: u8) -> bool {                                         // c:89
    c == b' ' || c.is_ascii_graphic()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies the type-bit constants are non-overlapping single-bit
    /// values per c:30-45. INAMESPC is u32-only (overflows u16).
    #[test]
    fn type_bits_are_distinct() {
        let all_u16: u16 = IDIGIT | IALNUM | IBLANK | INBLANK | ITOK
                         | ISEP | IALPHA | IIDENT | IUSER | ICNTRL
                         | IWORD | ISPECIAL | IMETA | IWSEP | INULL
                         | IPATTERN;
        assert_eq!(all_u16.count_ones(), 16);
        assert_eq!(INAMESPC, 0x10000);
    }

    /// Verifies ZTF_* flag values per c:69-72.
    #[test]
    fn ztf_flags_correct() {
        assert_eq!(ZTF_INIT, 0x0001);
        assert_eq!(ZTF_INTERACT, 0x0002);
        assert_eq!(ZTF_SP_COMMA, 0x0004);
        assert_eq!(ZTF_BANGCHAR, 0x0008);
    }

    /// Verifies `zistype` consults TYPTAB and masks (c:47).
    #[test]
    fn zistype_reads_typtab() {
        let saved = TYPTAB.lock().unwrap()[b'X' as usize];
        TYPTAB.lock().unwrap()[b'X' as usize] = (IDIGIT | IALNUM) as u32;
        assert!(zistype(b'X', IDIGIT as u32));
        assert!(zistype(b'X', IALNUM as u32));
        assert!(!zistype(b'X', ICNTRL as u32));
        TYPTAB.lock().unwrap()[b'X' as usize] = saved;
    }

    /// Verifies the predicate fns dispatch through zistype.
    #[test]
    fn idigit_dispatches_through_typtab() {
        let saved = TYPTAB.lock().unwrap()[b'7' as usize];
        TYPTAB.lock().unwrap()[b'7' as usize] = IDIGIT as u32;
        assert!(idigit(b'7'));
        assert!(!ialpha(b'7'));
        TYPTAB.lock().unwrap()[b'7' as usize] = saved;
    }

    /// Verifies `wc_zistype` falls through to Unicode predicates for
    /// non-ASCII input (c:75 multibyte branch).
    #[test]
    fn wc_zistype_falls_back_to_unicode_for_high_code_points() {
        assert!(WC_ZISTYPE('é', IALPHA as u32));
        assert!(WC_ZISTYPE('é', IALNUM as u32));
        assert!(WC_ZISTYPE('é', IWORD as u32));
        assert!(!WC_ZISTYPE('é', IDIGIT as u32));
        let saved = TYPTAB.lock().unwrap()[b'a' as usize];
        TYPTAB.lock().unwrap()[b'a' as usize] = IALPHA as u32;
        assert!(WC_ZISTYPE('a', IALPHA as u32));
        TYPTAB.lock().unwrap()[b'a' as usize] = saved;
    }

    /// Verifies `wc_isprint` rejects controls per c:79.
    #[test]
    fn wc_isprint_rejects_controls() {
        assert!(WC_ISPRINT('a'));
        assert!(WC_ISPRINT(' '));
        assert!(WC_ISPRINT('é'));
        assert!(!WC_ISPRINT('\u{0007}'));
        assert!(!WC_ISPRINT('\u{001b}'));
    }

    /// Verifies `zisprint` accepts space + graphic ASCII per c:89.
    #[test]
    fn zisprint_basic() {
        assert!(ZISPRINT(b' '));
        assert!(ZISPRINT(b'a'));
        assert!(ZISPRINT(b'~'));
        assert!(!ZISPRINT(b'\t'));
        assert!(!ZISPRINT(0x7f));
    }
}
