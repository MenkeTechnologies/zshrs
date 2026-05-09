//! `zsh.h` port — shared types, macros, and constants.
//!
//! Port of the public declarations from `Src/zsh.h`. zsh.h is the
//! umbrella header every C file includes; it defines:
//!   - integer type aliases (`zlong`, `zulong`)
//!   - tokenized character constants (`Meta`, `Pound`, `String`, etc.)
//!   - parameter-flag bitmask constants (`PM_*`)
//!   - scanpm-flag bitmask constants (`SCANPM_*`)
//!   - option-bitmap accessor macros (`OPT_ISSET`, `OPT_ARG`, ...)
//!   - the `mnumber` math-number value type (re-exported from
//!     `crate::ported::math` to keep the existing call sites working)
//!
//! The full zsh.h is ~3,375 lines of declarations; this file ports
//! the slices the rest of the tree actually consumes. Everything
//! else stays in its canonical home (e.g. `struct param` lives in
//! `params.rs`, the GSU dispatch tables in `params.rs`/`hashtable.rs`,
//! etc.) and is referenced by name where needed.
//!
//! Order of declarations roughly mirrors zsh.h: integer types first,
//! then tokens, then mnumber, then parameter flags, then option
//! accessors. Cite the matching `zsh.h:NNN` line in each block.

// ---------------------------------------------------------------------------
// Integer type aliases (zsh.h:36-90).
// ---------------------------------------------------------------------------

/// Port of `typedef ZSH_64_BIT_TYPE zlong;` from `Src/zsh.h:38`.
/// On every modern platform this is `int64_t` / `i64`. zsh's
/// configure.ac probes for the platform's 64-bit signed integer
/// type and typedefs `zlong` to it; the only place it isn't 64-bit
/// is on ancient systems with no 64-bit type, where it falls back
/// to `long` (handled by the `#else` arm at zsh.h:55).
#[allow(non_camel_case_types)]
pub type zlong = i64;                                                    // c:38

/// Port of `typedef ZSH_64_BIT_UTYPE zulong;` from `Src/zsh.h:50`.
/// Unsigned counterpart of `zlong`.
#[allow(non_camel_case_types)]
pub type zulong = u64;                                                   // c:50

/// Port of `ZLONG_MAX` from `Src/zsh.h:40-57` (whichever arm the
/// platform takes; for our target hosts this is the i64 max).
pub const ZLONG_MAX: zlong = i64::MAX;                                   // c:40-57

// ---------------------------------------------------------------------------
// Meta byte + tokenised parser characters (zsh.h:144-205).
//
// zsh's parser tokenises certain shell metacharacters into single
// non-ASCII bytes so the lexer can distinguish syntactically-active
// `(` from a literal `(`. These constants are used by `Src/lex.c`,
// `Src/parse.c`, `Src/subst.c`, etc.
// ---------------------------------------------------------------------------

/// Port of `#define Meta` from `Src/zsh.h:144`. The "metafy" prefix
/// byte. Every char in a metafied string after a `Meta` byte is the
/// original byte XOR'd with 0x20.
pub const META: char = '\u{83}';                                         // c:144

/// Port of `#define DEFAULT_IFS` from `Src/zsh.h:149`. Default
/// `$IFS` value in non-emulation mode: space, tab, newline, then the
/// metafied null byte (`Meta` 0x83 followed by 0x80 byte = ' ').
pub const DEFAULT_IFS: &str = " \t\n\u{83} ";                            // c:149

/// Port of `#define DEFAULT_IFS_SH` from `Src/zsh.h:153`. Default
/// `$IFS` value in POSIX/sh emulation: space, tab, newline only.
pub const DEFAULT_IFS_SH: &str = " \t\n";                                // c:153

// Port of the tokenised parser characters at `Src/zsh.h:159-185`.
// Each is the high-bit-set ASCII byte the lexer substitutes for the
// matching shell metacharacter when scanning the input. After
// substitution `Pound` is the parsed `#`, `String` is the parsed
// `$`, etc.; `untokenize()` reverses the mapping for output.
pub const POUND: char     = '\u{84}';                                    // c:159 #
pub const STRING_TOK: char = '\u{85}';                                   // c:160 $
pub const HAT: char       = '\u{86}';                                    // c:161 ^
pub const STAR: char      = '\u{87}';                                    // c:162 *
pub const INPAR: char     = '\u{88}';                                    // c:163 (
pub const INPARMATH: char = '\u{89}';                                    // c:164 ((
pub const OUTPAR: char    = '\u{8a}';                                    // c:165 )
pub const OUTPARMATH: char = '\u{8b}';                                   // c:166 ))
pub const QSTRING: char   = '\u{8c}';                                    // c:167 "$"
pub const EQUALS: char    = '\u{8d}';                                    // c:168 =
pub const BAR: char       = '\u{8e}';                                    // c:169 |
pub const INBRACE: char   = '\u{8f}';                                    // c:170 {
pub const OUTBRACE: char  = '\u{90}';                                    // c:171 }
pub const INBRACK: char   = '\u{91}';                                    // c:172 [
pub const OUTBRACK: char  = '\u{92}';                                    // c:173 ]
pub const TICK: char      = '\u{93}';                                    // c:174 `
pub const INANG: char     = '\u{94}';                                    // c:175 <
pub const OUTANG: char    = '\u{95}';                                    // c:176 >
pub const OUTANG_PROC: char = '\u{96}';                                  // c:177 >( ...)
pub const QUEST: char     = '\u{97}';                                    // c:178 ?
pub const TILDE: char     = '\u{98}';                                    // c:179 ~
pub const QTICK: char     = '\u{99}';                                    // c:180 "`"
pub const COMMA: char     = '\u{9a}';                                    // c:181 ,
pub const DASH: char      = '\u{9b}';                                    // c:182 - (patterns only)
pub const BANG: char      = '\u{9c}';                                    // c:183 ! (patterns only)

/// Port of `#define LAST_NORMAL_TOK Bang` from `Src/zsh.h:188`. The
/// inclusive upper bound of the "normal" tokenised range; bytes above
/// this byte value are special-purpose markers used by globbing and
/// completion.
pub const LAST_NORMAL_TOK: char = BANG;                                  // c:188

// Port of the quote-string-null markers at `Src/zsh.h:193-195`. Used
// to flag the start/end of single-quoted, double-quoted, and
// backslash-escaped runs inside an already-tokenised string.
pub const SNULL: char = '\u{9d}';                                        // c:193 'foo'
pub const DNULL: char = '\u{9e}';                                        // c:194 "foo"
pub const BNULL: char = '\u{9f}';                                        // c:195 \foo

// ---------------------------------------------------------------------------
// Math-number value type (zsh.h:95-135). Defined in
// `crate::ported::math`; re-exported here so consumers that mirror
// the C `#include "zsh.h"` style can do `use crate::ported::zsh::*;`.
// ---------------------------------------------------------------------------

pub use crate::ported::math::{Mnumber, MN_INTEGER, MN_FLOAT, MN_UNSET};   // c:95-105

// ---------------------------------------------------------------------------
// Parameter flags (zsh.h:1878-1949). Used by `params.rs`,
// `hashtable.rs`, every paramdef table, and the `private`/`local`
// builtins.
// ---------------------------------------------------------------------------

pub const PM_SCALAR:    u32 = 0;                                         // c:1878
pub const PM_ARRAY:     u32 = 1 << 0;                                    // c:1879
pub const PM_INTEGER:   u32 = 1 << 1;                                    // c:1880
pub const PM_EFLOAT:    u32 = 1 << 2;                                    // c:1881
pub const PM_FFLOAT:    u32 = 1 << 3;                                    // c:1882
pub const PM_HASHED:    u32 = 1 << 4;                                    // c:1883

/// Port of `PM_TYPE(X)` macro from `Src/zsh.h:1885`. Mask returning
/// only the type bits of a flag word.
#[inline]
#[allow(non_snake_case)]
pub const fn PM_TYPE(x: u32) -> u32 {                                    // c:1885
    x & (PM_SCALAR | PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_ARRAY | PM_HASHED | PM_NAMEREF)
}

pub const PM_LEFT:      u32 = 1 << 5;                                    // c:1888
pub const PM_RIGHT_B:   u32 = 1 << 6;                                    // c:1889
pub const PM_RIGHT_Z:   u32 = 1 << 7;                                    // c:1890
pub const PM_LOWER:     u32 = 1 << 8;                                    // c:1891
pub const PM_UPPER:     u32 = 1 << 9;                                    // c:1895
pub const PM_UNDEFINED: u32 = 1 << 9;                                    // c:1896
pub const PM_READONLY:  u32 = 1 << 10;                                   // c:1898
pub const PM_TAGGED:    u32 = 1 << 11;                                   // c:1899
pub const PM_EXPORTED:  u32 = 1 << 12;                                   // c:1900
pub const PM_ABSPATH_USED: u32 = 1 << 12;                                // c:1901
pub const PM_UNIQUE:    u32 = 1 << 13;                                   // c:1905
pub const PM_UNALIASED: u32 = 1 << 13;                                   // c:1906
pub const PM_HIDE:      u32 = 1 << 14;                                   // c:1908
pub const PM_CUR_FPATH: u32 = 1 << 14;                                   // c:1909
pub const PM_HIDEVAL:   u32 = 1 << 15;                                   // c:1910
pub const PM_WARNNESTED:u32 = 1 << 15;                                   // c:1911
pub const PM_TIED:      u32 = 1 << 16;                                   // c:1912
pub const PM_TAGGED_LOCAL: u32 = 1 << 16;                                // c:1913
pub const PM_DONTIMPORT_SUID: u32 = 1 << 17;                             // c:1916
pub const PM_LOADDIR:   u32 = 1 << 17;                                   // c:1917
pub const PM_SINGLE:    u32 = 1 << 18;                                   // c:1918
pub const PM_ANONYMOUS: u32 = 1 << 18;                                   // c:1919
pub const PM_LOCAL:     u32 = 1 << 19;                                   // c:1920
pub const PM_KSHSTORED: u32 = 1 << 19;                                   // c:1921
pub const PM_SPECIAL:   u32 = 1 << 20;                                   // c:1922
pub const PM_ZSHSTORED: u32 = 1 << 20;                                   // c:1923
pub const PM_RO_BY_DESIGN: u32 = 1 << 21;                                // c:1924
pub const PM_READONLY_SPECIAL: u32 = PM_SPECIAL | PM_READONLY | PM_RO_BY_DESIGN;  // c:1926
pub const PM_DONTIMPORT: u32 = 1 << 22;                                  // c:1927
pub const PM_DECLARED:  u32 = 1 << 22;                                   // c:1928
pub const PM_RESTRICTED: u32 = 1 << 23;                                  // c:1929
pub const PM_UNSET:     u32 = 1 << 24;                                   // c:1930
pub const PM_DEFAULTED: u32 = PM_DECLARED | PM_UNSET;                    // c:1934
pub const PM_REMOVABLE: u32 = 1 << 25;                                   // c:1935
pub const PM_AUTOLOAD:  u32 = 1 << 26;                                   // c:1936
pub const PM_NORESTORE: u32 = 1 << 27;                                   // c:1937
pub const PM_AUTOALL:   u32 = 1 << 27;                                   // c:1938
pub const PM_HASHELEM:  u32 = 1 << 28;                                   // c:1942
pub const PM_NAMEDDIR:  u32 = 1 << 29;                                   // c:1943
pub const PM_NAMEREF:   u32 = 1 << 30;                                   // c:1944

/// Port of `#define TYPESET_OPTSTR` from `Src/zsh.h:1947`. The
/// option-string passed to the `typeset` builtin's option parser.
pub const TYPESET_OPTSTR: &str = "aiEFALRZlurtxUhHT";                    // c:1947

/// Port of `#define TYPESET_OPTNUM` from `Src/zsh.h:1950`. Subset of
/// the typeset option string whose flags accept an optional numeric
/// argument.
pub const TYPESET_OPTNUM: &str = "LRZiEF";                               // c:1950

// ---------------------------------------------------------------------------
// Hash/array scan flags (zsh.h:1953-1961).
// ---------------------------------------------------------------------------

pub const SCANPM_WANTVALS:  u32 = 1 << 0;                                // c:1953
pub const SCANPM_WANTKEYS:  u32 = 1 << 1;                                // c:1954
pub const SCANPM_WANTINDEX: u32 = 1 << 2;                                // c:1955
pub const SCANPM_MATCHKEY:  u32 = 1 << 3;                                // c:1956
pub const SCANPM_MATCHVAL:  u32 = 1 << 4;                                // c:1957
pub const SCANPM_MATCHMANY: u32 = 1 << 5;                                // c:1958
pub const SCANPM_ASSIGNING: u32 = 1 << 6;                                // c:1959
pub const SCANPM_KEYMATCH:  u32 = 1 << 7;                                // c:1960

// ---------------------------------------------------------------------------
// `Options ops` accessor macros (zsh.h:1400-1414).
//
// In C, `Options ops` is `struct options *` carrying:
//   - `ind[256]`: per-option-char state (0=unset, &1=`-` form,
//     &2=`+` form, >>2=arg index)
//   - `args[]`: positional arg storage
//   - `err`: error code
//
// Per PORT_CHECKLIST.md rule 3, the Rust port takes a `[bool; 256]`
// bitmask in place of `Options ops`. These accessor fns work on
// either a bitmask (for the `ISSET` check) or take name+value pairs
// for callers using the parsed-args style. They preserve the C
// macro's call shape (`OPT_ISSET(ops, c)` → `opt_isset(ops, c)`).
// ---------------------------------------------------------------------------

/// Port of `#define OPT_ISSET(ops,c)` from `Src/zsh.h:1408`. Tests
/// whether option-character `c` was passed (as either `-c` or `+c`).
#[inline]
#[allow(non_snake_case)]
pub fn OPT_ISSET(ops: &[bool; 256], c: u8) -> bool {                     // c:1408
    ops[c as usize]
}

/// Port of `#define OPT_MINUS(ops,c)` from `Src/zsh.h:1400`. Tests
/// whether option-character `c` was passed in the `-c` form. Without
/// a separate +/- bitmap to consult, returns the same as `OPT_ISSET`.
#[inline]
#[allow(non_snake_case)]
pub fn OPT_MINUS(ops: &[bool; 256], c: u8) -> bool {                     // c:1400
    ops[c as usize]
}

/// Port of `#define OPT_PLUS(ops,c)` from `Src/zsh.h:1402`. Tests
/// whether option-character `c` was passed in the `+c` form. The
/// `-c` vs `+c` distinction is rare; without a separate +/- bitmap,
/// returns false.
#[inline]
#[allow(non_snake_case)]
pub fn OPT_PLUS(_ops: &[bool; 256], _c: u8) -> bool {                    // c:1402
    false
}

// ---------------------------------------------------------------------------
// Builtin `flags` field constants (zsh.h:1452+).
// ---------------------------------------------------------------------------

/// Port of `#define BINF_PREFIX` (zsh.h around line 1452 BIN_PREFIX
/// macro). Marks a builtin as a prefix command (e.g. `noglob`,
/// `command`, `nocorrect`) that modifies how the next word parses.
pub const BINF_PREFIX: u32 = 1 << 6;                                     // c:1452

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies `zlong` is `i64` on every supported host (c:38).
    #[test]
    fn zlong_is_i64() {
        let _: zlong = i64::MIN;
        let _: zlong = i64::MAX;
        assert_eq!(std::mem::size_of::<zlong>(), 8);
    }

    /// Verifies `zulong` is `u64` (c:50).
    #[test]
    fn zulong_is_u64() {
        let _: zulong = u64::MAX;
        assert_eq!(std::mem::size_of::<zulong>(), 8);
    }

    /// Verifies the meta byte mapping at c:144 — Meta is 0x83.
    #[test]
    fn meta_byte_value() {
        assert_eq!(META as u32, 0x83);
    }

    /// Verifies the IFS defaults at c:149/153.
    #[test]
    fn default_ifs_strings() {
        assert_eq!(DEFAULT_IFS, " \t\n\u{83} ");
        assert_eq!(DEFAULT_IFS_SH, " \t\n");
    }

    /// Verifies the parser tokens have the expected high-bit-set
    /// ASCII bytes (c:159-195).
    #[test]
    fn parser_tokens_have_correct_bytes() {
        assert_eq!(POUND as u32, 0x84);
        assert_eq!(STRING_TOK as u32, 0x85);
        assert_eq!(STAR as u32, 0x87);
        assert_eq!(INPAR as u32, 0x88);
        assert_eq!(BANG as u32, 0x9c);
        assert_eq!(SNULL as u32, 0x9d);
        assert_eq!(DNULL as u32, 0x9e);
        assert_eq!(BNULL as u32, 0x9f);
    }

    /// Verifies the PM_TYPE mask isolates type bits per c:1885.
    #[test]
    fn pm_type_isolates_type_bits() {
        let combined = PM_INTEGER | PM_EXPORTED;
        assert_eq!(PM_TYPE(combined), PM_INTEGER);
        let array_combined = PM_ARRAY | PM_READONLY;
        assert_eq!(PM_TYPE(array_combined), PM_ARRAY);
    }

    /// Verifies PM_READONLY_SPECIAL aggregate per c:1926.
    #[test]
    fn pm_readonly_special_aggregate() {
        assert_eq!(PM_READONLY_SPECIAL,
                   PM_SPECIAL | PM_READONLY | PM_RO_BY_DESIGN);
    }

    /// Verifies OPT_ISSET matches the C `OPT_ISSET(ops, c)` macro
    /// per c:1408.
    #[test]
    fn opt_isset_basic() {
        let mut ops = [false; 256];
        ops[b'l' as usize] = true;
        assert!(OPT_ISSET(&ops, b'l'));
        assert!(!OPT_ISSET(&ops, b'r'));
    }

    /// Verifies SCANPM_* flags are non-overlapping bits.
    #[test]
    fn scanpm_flags_are_distinct() {
        let all = SCANPM_WANTVALS | SCANPM_WANTKEYS | SCANPM_WANTINDEX
                | SCANPM_MATCHKEY | SCANPM_MATCHVAL | SCANPM_MATCHMANY
                | SCANPM_ASSIGNING | SCANPM_KEYMATCH;
        assert_eq!(all.count_ones(), 8);
    }
}
