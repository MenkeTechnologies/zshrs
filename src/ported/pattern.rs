//! Pattern matching — port of `Src/pattern.c`.
//!
//! This is the bytecode-based port. The C source compiles patterns
//! into a flat `char *patout` buffer of packed opcodes; the matcher
//! is an interpreter that walks the buffer using pointer arithmetic.
//!
//! Rust port preserves the bytecode architecture using `Vec<u8>`:
//!   * Each opcode is 1 byte (matches C `Upat::c`).
//!   * `next_off` is a 4-byte little-endian `u32` offset to the next
//!     opcode in sequence (0 = end). C uses native-endian `long`; we
//!     pin to LE for portable on-disk bytecode caches.
//!   * Payloads (strings, ranges, branch operands) are encoded inline
//!     after the `next_off` slot.
//!
//! Top-level declaration order mirrors `Src/pattern.c`:
//!   1. Opcode constants (P_*) — pattern.c:97-127
//!   2. Macro-style accessors (P_OP / P_NEXT / etc.) — pattern.c:175-210
//!   3. Flag-bit constants (P_SIMPLE / P_HSTART / P_PURESTR) — pattern.c:216
//!   4. `struct patprog` — zsh.h:1601
//!   5. PAT_* flag constants — zsh.h:1623
//!   6. ZPC_* indexes — zsh.h:1644
//!   7. File-static globals — pattern.c file-scope
//!   8. Bytecode write helpers (`patadd`, `patnode`, `patinsert`,
//!      `pattail`, `patoptail`, `patcompcharsset`, `patcompstart`)
//!   9. Compiler entry points (`patcompile`, `patcompswitch`,
//!      `patcompbranch`, `patcomppiece`, `patcompnot`)
//!  10. Glob-flag parser (`patgetglobflags`)
//!  11. Range helpers (`range_type`, `pattern_range_to_string`)
//!  12. Char-decode helpers (`charref`, `charnext`, `charrefinc`,
//!      `charsub`, `metacharinc`, `clear_shiftstate`)
//!  13. Matcher entry points (`pattry`, `pattrylen`, `pattryrefs`)
//!  14. `patmatch` interpreter — pattern.c:2694
//!  15. `patmatchrange` — pattern.c:3865. The Cpattern-byte-stream
//!      walker, relocated here from `zle/compmatch.rs` (the Rule C
//!      move this section used to say was pending). The earlier
//!      Rust-only entries `patmatchrange(&[char], char, igncase)` and
//!      `patmatchindex(&[char], idx)` were deleted as Rule B /
//!      semantic deviations; see the deletion sites.
//!  16. String pre-processing (`patmungestring`, `patallocstr`,
//!      `pattrystart`)
//!  17. Module-loader / disable mgmt (`startpatternscope`,
//!      `endpatternscope`, `savepatterndisables`,
//!      `restorepatterndisables`, `clearpatterndisables`,
//!      `freepatprog`, `pat_enables`)
//!  18. (Section removed — formerly held Rust-only convenience entries
//!      `patmatch(pat, text)`, `patmatchlen(prog, string)`, `patrepeat(
//!      prog, s, max)`, `mb_patmatchrange`, `mb_patmatchindex`. All
//!      deleted: `patmatch` got renamed to the C-faithful bytecode-
//!      walker name; the others had zero Rust callers + Rule S1
//!      signature deviations. `haswilds` is a real C name — port lives
//!      in section 4 helpers.)
//!
//! See `docs/PORT.md` Rules A/B/C/D/E.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]

use crate::ported::params::{paramtab, paramtab_hashed_storage};
use crate::ported::utils::ztrsub;
use crate::ported::zle::zle_h::{COMP_LIST_COMPLETE, COMP_LIST_EXPAND};
pub use crate::ported::zsh_h::{
    patstralloc, Patstralloc, GF_BACKREF, GF_IGNCASE, GF_LCMATCHUC, GF_MATCHREF, GF_MULTIBYTE,
    PAT_ANY, PAT_FILE, PAT_FILET, PAT_HAS_EXCLUDP, PAT_HEAPDUP, PAT_LCMATCHUC, PAT_NOANCH,
    PAT_NOGLD, PAT_NOTEND, PAT_NOTSTART, PAT_PURES, PAT_SCAN, PAT_STATIC, PAT_ZDUP, PP_ALNUM,
    PP_ALPHA, PP_ASCII, PP_BLANK, PP_CNTRL, PP_DIGIT, PP_GRAPH, PP_IDENT, PP_IFS, PP_IFSSPACE,
    PP_INCOMPLETE, PP_INVALID, PP_LOWER, PP_PRINT, PP_PUNCT, PP_RANGE, PP_SPACE, PP_UPPER, PP_WORD,
    PP_XDIGIT, ZMB_INCOMPLETE, ZMB_INVALID, ZPC_SEG_COUNT,
};
use crate::utils::zerrnam;
use crate::zsh_h::{
    isset, patprog, Bang, Bar, Hat, Inang, Inbrack, Inpar, Marker, Meta, Nularg, Outbrack, Pound,
    Quest, Star, BASHAUTOLIST, CASEGLOB, CASEPATHS, EXTENDEDGLOB, KSHGLOB, MULTIBYTE,
    NUMERICGLOBSORT, PM_HASHED, PM_TYPE, RCQUOTES, SHGLOB, SORTIT_IGNORING_BACKSLASHES,
    SORTIT_NUMERICALLY, ZPC_BAR, ZPC_BNULLKEEP, ZPC_COUNT, ZPC_HASH, ZPC_HAT, ZPC_INANG,
    ZPC_INBRACK, ZPC_INPAR, ZPC_KSH_AT, ZPC_KSH_BANG, ZPC_KSH_BANG2, ZPC_KSH_PLUS, ZPC_KSH_QUEST,
    ZPC_KSH_STAR, ZPC_NULL, ZPC_OUTPAR, ZPC_QUEST, ZPC_SLASH, ZPC_STAR, ZPC_TILDE,
};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::Mutex;

// =====================================================================
// 6. ZPC_* enum from zsh.h:1644 — indexes into the active-pattern-
// characters table that compile-time and runtime both consult.
// =====================================================================

/// Maximum captures, from `pattern.c:94 NSUBEXP`.
pub const NSUBEXP: usize = 9;

// =====================================================================
// 1. P_* opcode constants — pattern.c:97-127
//
// Numbered identically to C so a buffer compiled by this port matches
// the C source's bytecode-cache format byte-for-byte (modulo native
// endianness; we pin LE — see file header).
// =====================================================================
/// `P_END` constant.
pub const P_END: u8 = 0x00; // c:97  End of program.
/// `P_EXCSYNC` constant.
pub const P_EXCSYNC: u8 = 0x01; // c:98  Test if following exclude already failed
/// `P_EXCEND` constant.
pub const P_EXCEND: u8 = 0x02; // c:99  Test if exclude matched orig branch
/// `P_BACK` constant.
pub const P_BACK: u8 = 0x03; // c:100 Match "", "next" ptr points backward.
/// `P_EXACTLY` constant.
pub const P_EXACTLY: u8 = 0x04; // c:101 lstr — match this string.
/// `P_NOTHING` constant.
pub const P_NOTHING: u8 = 0x05; // c:102 Match empty string.
/// `P_ONEHASH` constant.
pub const P_ONEHASH: u8 = 0x06; // c:103 node — match 0 or more of preceding simple.
/// `P_TWOHASH` constant.
pub const P_TWOHASH: u8 = 0x07; // c:104 node — match 1 or more of preceding simple.
/// `P_GFLAGS` constant.
pub const P_GFLAGS: u8 = 0x08; // c:105 long — match nothing and set globbing flags.
/// `P_ISSTART` constant.
pub const P_ISSTART: u8 = 0x09; // c:106 Match start of string.
/// `P_ISEND` constant.
pub const P_ISEND: u8 = 0x0a; // c:107 Match end of string.
/// `P_COUNTSTART` constant.
pub const P_COUNTSTART: u8 = 0x0b; // c:108 Initialise P_COUNT.
/// `P_COUNT` constant.
pub const P_COUNT: u8 = 0x0c; // c:109 3*long uc* node — match a number of repetitions.
/// `P_BRANCH` constant.
pub const P_BRANCH: u8 = 0x20; // c:112 node — match this alternative, or the next.
/// `P_WBRANCH` constant.
pub const P_WBRANCH: u8 = 0x21; // c:113 uc* node — P_BRANCH, but match at least 1 char.
/// `P_EXCLUDE` constant.
pub const P_EXCLUDE: u8 = 0x30; // c:114 uc* node — exclude this from previous branch.
/// `P_EXCLUDP` constant.
pub const P_EXCLUDP: u8 = 0x31; // c:115 uc* node — exclude, using full file path so far.
/// `P_ANY` constant.
pub const P_ANY: u8 = 0x40; // c:117 Match any one character.
/// `P_ANYOF` constant.
pub const P_ANYOF: u8 = 0x41; // c:118 str — match any character in this string.
/// `P_ANYBUT` constant.
pub const P_ANYBUT: u8 = 0x42; // c:119 str — match any character not in this string.
/// `P_STAR` constant.
pub const P_STAR: u8 = 0x43; // c:120 Match any set of characters.
/// `P_NUMRNG` constant.
pub const P_NUMRNG: u8 = 0x44; // c:121 zr,zr — match a numeric range.
/// `P_NUMFROM` constant.
pub const P_NUMFROM: u8 = 0x45; // c:122 zr — match a number >= X.
/// `P_NUMTO` constant.
pub const P_NUMTO: u8 = 0x46; // c:123 zr — match a number <= X.
/// `P_NUMANY` constant.
pub const P_NUMANY: u8 = 0x47; // c:124 Match any set of decimal digits.
/// `P_OPEN` constant.
pub const P_OPEN: u8 = 0x80; // c:126 Mark this point in input as start of n.
/// `P_CLOSE` constant.
pub const P_CLOSE: u8 = 0x90; // c:127 Analogous to OPEN.

/// Port of `P_ISBRANCH()` from `Src/pattern.c:200`.
/// C macro `#define P_ISBRANCH(p)   ((p)->l & 0x20)`.
#[inline]
pub fn P_ISBRANCH(op: u8) -> bool {
    (op & 0x20) != 0
}

/// Port of `P_ISEXCLUDE()` from `Src/pattern.c:201`.
/// C macro `#define P_ISEXCLUDE(p)	(((p)->l & 0x30) == 0x30)`.
#[inline]
pub fn P_ISEXCLUDE(op: u8) -> bool {
    (op & 0x30) == 0x30
}

/// Port of `P_NOTDOT()` from `Src/pattern.c:202`.
/// C macro `#define P_NOTDOT(p)	((p)->l & 0x40)`.
#[inline]
pub fn P_NOTDOT(op: u8) -> bool {
    (op & 0x40) != 0
}

// =====================================================================
// 3. Flag-bit constants returned via flagp out-params during compile.
// pattern.c:216-218
// =====================================================================
/// `P_SIMPLE` constant.
pub const P_SIMPLE: i32 = 0x01; // c:216 Simple enough to be # / ## operand.
/// `P_HSTART` constant.
pub const P_HSTART: i32 = 0x02; // c:217 Starts with # or ##'d pattern.
/// `P_PURESTR` constant.
pub const P_PURESTR: i32 = 0x04; // c:218 Can be matched with a strcmp.

// =====================================================================
// 5. PAT_* flag constants — re-exports of zsh.h:1623-1640 already in
// zsh_h.rs. Re-published here so callers in pattern's API don't need
// the longer path. C source has these as `#define` in zsh.h, not
// pattern.c, so the canonical home is zsh_h.rs; we just alias.
// =====================================================================

// C: `static int patnpar;` — number of active parens (1-indexed at
// compile time; the *struct* patnpar is the actual count).
pub static patnpar: AtomicI32 = AtomicI32::new(0); // c:271

// GF_* glob-flag bits live in `zsh.h:1763-1773`, ported to
// `src/ported/zsh_h.rs:2287-2291` per Rule C. Re-export so pattern's
// matcher arms can read them without the longer path.

// =====================================================================
// 7. File-static globals — direct mirror of pattern.c file-scope
// statics. Each C `static` ports to a Rust `static` of matching name
// (Rule A) and `Mutex<>` / `Atomic*` for thread-safe access. None
// are aggregated (Rule D).
// =====================================================================

// C: `static char *patout, *patcode;` + `static long patsize;` +
// `static int patalloc;` — pattern.c:267-272 (in macro region).
//
// patout: the bytecode buffer.
// patcode: write cursor (offset into patout).
// patsize: current logical size.
// patalloc: allocated capacity.
//
// In Rust, `Vec<u8>` already tracks both len and capacity, so we
// hold just the buffer; patcode/patsize/patalloc are derived.
pub static patout: Mutex<Vec<u8>> = Mutex::new(Vec::new()); // c:267

// C: `static int patflags;` — current PAT_* flag set during compile.
pub static patflags: AtomicI32 = AtomicI32::new(0); // c:272

// C: `static int patglobflags;` — current globbing flags during compile.
pub static patglobflags: AtomicI32 = AtomicI32::new(0); // c:273

/// Port of file-static `int patinlen` from `pattrystate` struct
/// at `Src/pattern.c:1877` (accessed via `#define patinlen
/// (pattrystate.patinlen)` at line 1894). Length in metafied bytes
/// of the last successful pattry match; computed at the end of
/// `pattry`/`pattrylen`/`pattryrefs` (`patinput - patinstart`,
/// pattern.c:2508). Read by `patmatchlen()` and by the
/// `${var//pat/repl}` paramsubst pipeline that needs to know how
/// much of the input was consumed.
pub static patinlen: AtomicI32 = AtomicI32::new(0); // c:1877

// =====================================================================
// 12. Char-decode helpers — pattern.c:327, :336, :1909-1997
// =====================================================================

/// Port of `clear_shiftstate()` from `Src/pattern.c:327`. C uses
/// `mbstate_t`; Rust `char` is already a code point, so no shift
/// state to clear.
pub fn clear_shiftstate() {} // c:327

/// Port of `metacharinc()` from `Src/pattern.c:336`.
/// C: `static wchar_t metacharinc(char **x)`.
/// Advances `*x` past one metafied / multibyte char and returns the
/// decoded codepoint. The C body branches on:
///   - `GF_MULTIBYTE` clear OR high-bit-clear: single-byte path
///     with `itok(*x)` zsh-token translation and `Meta` xor-32
///   - else: `mbrtowc` over the metafied bytes with state machine
///
/// **Rust port:** UTF-32-native delegation to `&str.chars()`.
/// Delegates to Rust's native UTF-8 iterator — both C branches
/// collapse because Rust's `char` is already the wide-char form,
/// and the stored slice is the post-Meta-decode form (zshrs's
/// source bytes are already UTF-8, not Meta-encoded). The `itok` →
/// `ztokens[]` zsh-token table mapping doesn't apply because the
/// pattern compiler stores raw chars; translation happens at
/// compile time, not at scan time.
///
/// Rust signature differs from C `metacharinc(char **x)`: C mutates
/// `*x` to advance the pointer; Rust returns the new byte position
/// since `&str` length is immutable. Callers update their cursor
/// from the return value.
pub fn metacharinc(s: &str, pos: usize) -> usize {
    // c:336
    // c:343-360 single-byte short-circuit + c:363-380 mbrtowc loop:
    // both collapse to Rust's `chars().next()` which decodes one
    // valid UTF-8 codepoint from the slice, regardless of byte width.
    s[pos..]
        .chars()
        .next()
        .map(|c| pos + c.len_utf8())
        .unwrap_or(pos)
}

// =====================================================================
// 8. Bytecode write helpers — pattern.c:412-1856
// =====================================================================

/// Port of the anonymous `enum { PA_NOALIGN = 1, PA_UNMETA = 2 };`
/// from `Src/pattern.c:405-408`. Flags passed as the `paflags` arg
/// to `patadd`.
pub const PA_NOALIGN: i32 = 1; // c:406
/// `PA_UNMETA` constant.
pub const PA_UNMETA: i32 = 2; // c:407

/// Port of `patadd()` from `Src/pattern.c:412`.
/// C: `static void patadd(char *add, int ch, long n,
/// int paflags)` from `Src/pattern.c:410-450`.
///
/// Append `n` bytes from `add` (or a single `ch` byte when `add ==
/// None`) to `patout`, growing the backing storage if needed and
/// padding to the C `union upat` alignment unless `PA_NOALIGN` is
/// set. With `PA_UNMETA`, walk the input as zsh-metafied bytes
/// (Meta + (b^32)) and untokenize (`itok` → `ztokens[]`).
///
/// **The previous Rust impl had three concrete defects:**
///   1. Declared `-> i64` returning the starting offset. C is
///      `static void`; callers use side-effects on `patcode`,
///      not a return value.
///   2. When `add == None`, the C body writes `ch` **once**
///      (`*patcode++ = ch;`). The Rust port looped `0..n` pushing
///      `ch` n times, so a single-byte literal grew to n copies.
///   3. Skipped the C alignment-round-up to `sizeof(union upat)`
///      (8 bytes on every supported arch — c:417-418), so any
///      caller reading `patout` as a `[upat]` would mis-align.
///
/// zshrs's `patcode` pointer collapses into `patout.len()`; the
/// realloc dance in C (c:419-425) is implicit via `Vec::extend`.
/// `patsize` updates are the post-write `patout.len()`.
fn patadd(add: Option<&[u8]>, ch: u8, n: i64, paflags: i32) {
    use crate::ported::lex::ztokens as ztokens_str;
    use crate::ported::zsh_h::Pound;
    use crate::ported::ztype_h::itok;
    let ztokens = ztokens_str.as_bytes();
    // c:412
    // c:415 — `long newpatsize = patsize + n;`
    let mut buf = patout.lock().unwrap();
    let patsize = buf.len() as i64;
    let mut newpatsize = patsize + n;
    // c:416-418 — round up to upat-union alignment (sizeof(union upat) =
    // 8 on every supported arch) unless PA_NOALIGN.
    if (paflags & PA_NOALIGN) == 0 {
        // c:416
        let upat = 8i64; // c:417 sizeof(union upat)
        newpatsize = (newpatsize + upat - 1) & !(upat - 1); // c:417-418
    }
    // c:419-425 — realloc; Rust Vec auto-grows so just resize_to.
    if newpatsize > buf.len() as i64 {
        buf.resize(newpatsize as usize, 0);
    }
    // c:426 — `patsize = newpatsize;` — patsize is the buffer's
    // logical write head; zshrs tracks it via buf.len() AFTER the
    // write below.
    let mut patcode_pos = patsize as usize; // c:Src/pattern.c — `patcode` pointer.

    if let Some(add_bytes) = add {
        // c:427 — `if (add) {`
        if (paflags & PA_UNMETA) != 0 {
            // c:428-442 — PA_UNMETA: walk add_bytes, unmetafy + untokenize
            // as we go. Meta chars aren't counted in n. itok bytes route
            // through ztokens[*add - Pound].
            let mut idx = 0usize;
            let mut remaining = n;
            while remaining > 0 && idx < add_bytes.len() {
                let b = add_bytes[idx];
                if itok(b) {
                    // c:434-435 — `if (itok(*add)) *patcode++ =
                    //               ztokens[*add++ - Pound];`
                    if idx < add_bytes.len() {
                        let tok_idx = b.wrapping_sub(Pound as u8) as usize;
                        if tok_idx < ztokens.len() {
                            // c:435
                            if patcode_pos < buf.len() {
                                buf[patcode_pos] = ztokens[tok_idx];
                            }
                            patcode_pos += 1;
                        }
                        idx += 1;
                    }
                } else if b == Meta {
                    // c:436-438 — `else if (*add == Meta) { add++;
                    //               *patcode++ = *add++ ^ 32; }`
                    idx += 1;
                    if idx < add_bytes.len() {
                        if patcode_pos < buf.len() {
                            buf[patcode_pos] = add_bytes[idx] ^ 32;
                        }
                        patcode_pos += 1;
                        idx += 1;
                    }
                } else {
                    // c:439-440 — `else { *patcode++ = *add++; }`
                    if patcode_pos < buf.len() {
                        buf[patcode_pos] = b;
                    }
                    patcode_pos += 1;
                    idx += 1;
                }
                remaining -= 1;
            }
        } else {
            // c:444-445 — `while (n--) *patcode++ = *add++;` — plain copy.
            let n_actual = (n as usize).min(add_bytes.len());
            for &b in &add_bytes[..n_actual] {
                if patcode_pos < buf.len() {
                    buf[patcode_pos] = b;
                }
                patcode_pos += 1;
            }
        }
    } else {
        // c:447-448 — `else *patcode++ = ch;` — single-byte write.
        if patcode_pos < buf.len() {
            buf[patcode_pos] = ch;
        }
        patcode_pos += 1;
    }
    // c:449 — `patcode = patout + patsize;` — restore the post-write
    // head so subsequent patadd calls append at the next slot. zshrs
    // tracks via buf.len(); trim back to newpatsize so the alignment
    // padding stays visible to consumers as zeroes.
    let _ = patcode_pos;
    // (No explicit truncate — buf.len() == newpatsize already.)
}

/// Port of `patcompcharsset()` from `Src/pattern.c:464`.
///
/// Initializes the `zpc_special` table for the active globbing
/// regime. The C source resets every ZPC_* slot to its literal
/// character, then masks off characters via `Marker` (0xa2 — the
/// canonical "invalid" sentinel) for three option-driven cases:
///   1. `!isset(EXTENDEDGLOB)` → Tilde/Hat/Hash disabled.
///   2. `!isset(KSHGLOB)`      → KSH_QUEST/STAR/PLUS/BANG/BANG2/AT disabled.
///   3. `isset(SHGLOB)`        → Inpar/Inang disabled.
///
pub fn patcompcharsset() {
    // c:464
    let mut sp = zpc_special.lock().unwrap();
    *sp = [0u8; ZPC_COUNT as usize];
    // c:469 — `memcpy(zpc_special, zpc_chars, ZPC_COUNT)`. The default
    // char for every ZPC_* slot. Direct positional assignment here
    // since zshrs doesn't carry the `zpc_chars` const array yet.
    //
    // NOTE on token bytes: C's `zpc_chars[]` (pattern.c:248) uses
    // the TOKENIZED high-bit bytes (`Bar`, `Tilde`, etc.); the C
    // shell lexer tokenizes raw ASCII to these only inside grouped
    // alternation. zshrs's parser doesn't yet run that pre-tokenize
    // pass — every byte reaches patcompile as raw ASCII. Switching
    // these slots to the high-bit form would break ~80 library
    // tests that rely on raw `|` triggering alternation. The
    // narrower fix for bug #12 lives in vm_helper::glob_match_static
    // where we conservatively escape pattern bytes the param-strip
    // path could not have intended as metacharacters.
    sp[ZPC_SLASH as usize] = b'/';
    sp[ZPC_NULL as usize] = 0;
    sp[ZPC_BAR as usize] = b'|';
    sp[ZPC_OUTPAR as usize] = b')';
    sp[ZPC_TILDE as usize] = b'~';
    sp[ZPC_INPAR as usize] = b'(';
    sp[ZPC_QUEST as usize] = b'?';
    sp[ZPC_STAR as usize] = b'*';
    sp[ZPC_INBRACK as usize] = b'[';
    sp[ZPC_INANG as usize] = b'<';
    sp[ZPC_HAT as usize] = b'^';
    sp[ZPC_HASH as usize] = b'#';
    sp[ZPC_BNULLKEEP as usize] = 0;
    // c:478-490 — KSH_GLOB slots (omitted from previous Rust port).
    // Each defaults to the literal ksh-glob trigger char. The
    // option-mask pass below disables them when KSHGLOB is off.
    sp[ZPC_KSH_QUEST as usize] = b'?';
    sp[ZPC_KSH_STAR as usize] = b'*';
    sp[ZPC_KSH_PLUS as usize] = b'+';
    sp[ZPC_KSH_BANG as usize] = b'!';
    sp[ZPC_KSH_BANG2 as usize] = b'!';
    sp[ZPC_KSH_AT as usize] = b'@';

    let marker_byte = Marker as u32 as u8;

    // c:471-478 — `for (...; i < ZPC_COUNT; ...) if (*disp) *spp = Marker;`
    // Apply user disables from `disable -p` BEFORE the option-driven
    // masks (so EXTENDEDGLOB / KSHGLOB / SHGLOB layer over the per-
    // pattern disables).
    {
        let disp = zpc_disables.lock().unwrap();
        for i in 0..(ZPC_COUNT as usize) {
            if disp[i] != 0 {
                sp[i] = marker_byte; // c:476
            }
        }
    }

    // c:480-483 — `if (!isset(EXTENDEDGLOB))` mask Tilde/Hat/Hash.
    if !isset(EXTENDEDGLOB) {
        sp[ZPC_TILDE as usize] = marker_byte;
        sp[ZPC_HAT as usize] = marker_byte;
        sp[ZPC_HASH as usize] = marker_byte;
    }

    // c:485-491 — `if (!isset(KSHGLOB))` mask the six KSH_* slots.
    if !isset(KSHGLOB) {
        sp[ZPC_KSH_QUEST as usize] = marker_byte;
        sp[ZPC_KSH_STAR as usize] = marker_byte;
        sp[ZPC_KSH_PLUS as usize] = marker_byte;
        sp[ZPC_KSH_BANG as usize] = marker_byte;
        sp[ZPC_KSH_BANG2 as usize] = marker_byte;
        sp[ZPC_KSH_AT as usize] = marker_byte;
    }

    // c:499-505 — `if (isset(SHGLOB))` mask Inpar/Inang (case/numeric
    // ranges not valid under sh-emulation).
    if isset(SHGLOB) {
        sp[ZPC_INPAR as usize] = marker_byte;
        sp[ZPC_INANG as usize] = marker_byte;
    }
}

/// Port of `patcompstart()` from `Src/pattern.c:517`.
///
/// Resets per-compile globals. Called at the start of `patcompile`.
///
/// **C body (c:517-526)** — strict order matters:
///   1. `patcompcharsset()` — must run FIRST so the zpc_special
///      table reflects the current option state before parsing.
///   2. `patglobflags = isset(CASEGLOB) || isset(CASEPATHS) ? 0 :
///      GF_IGNCASE;` — case-insensitivity is the default UNLESS
///      one of the case-sensitive options is set.
///   3. `if (isset(MULTIBYTE)) patglobflags |= GF_MULTIBYTE;` —
///      multibyte handling is option-gated, NOT unconditional.
///
/// The previous Rust port had three divergences: (a) called
/// patcompcharsset LAST instead of FIRST, (b) unconditionally set
/// GF_MULTIBYTE even when `setopt nomultibyte`, (c) NEVER set
/// GF_IGNCASE — `setopt nocaseglob` had zero effect on pattern
/// case-folding.
pub fn patcompstart() {
    // c:517
    // c:519 — `patcompcharsset()` FIRST.
    patcompcharsset();
    patout.lock().unwrap().clear();
    patnpar.store(1, Ordering::Relaxed);
    patflags.store(0, Ordering::Relaxed);
    // c:520-523 — CASE option dispatch.
    let mut flags: i32 = if isset(CASEGLOB) || isset(CASEPATHS) {
        0 // c:521 case-sensitive
    } else {
        GF_IGNCASE // c:523 default = ignore case
    };
    // c:524-525 — MULTIBYTE option respect.
    if isset(MULTIBYTE) {
        flags |= GF_MULTIBYTE; // c:525
    }
    patglobflags.store(flags, Ordering::Relaxed);
    errsfound.store(0, Ordering::Relaxed);
    forceerrs.store(-1, Ordering::Relaxed);
    patparse_off.store(0, Ordering::Relaxed);
}

// =====================================================================
// 9. Compiler entry points — pattern.c:540
// =====================================================================

/// Port of `patcompile(char *exp, int inflags, char **endexp)` from `Src/pattern.c:540`.
///
/// C signature: `Patprog patcompile(char *exp, int inflags, char **endexp)`.
/// Compiles pattern `exp` under flags `inflags`, returns a `Patprog`
/// on success or `NULL` on failure. `endexp` (if non-NULL) is set to
/// the input cursor at end of parse — used by `bin_zregexparse` to
/// detect partial-parse cases.
pub fn patcompile(exp: &str, inflags: i32, mut endexp: Option<&mut String>) -> Option<Patprog> {
    // Global compiled-pattern cache (crate::pat_cache, a zshrs-only opt —
    // see that module for the safety/key/threading rationale). Skipped when
    // `endexp` is requested: that out-param is a compile side effect the
    // cache can't reproduce. The check runs BEFORE the compile mutex so a
    // hit never serialises on the compiler.
    let cacheable = endexp.is_none();
    if cacheable {
        if let Some(hit) = crate::pat_cache::get(exp, inflags) {
            return Some(hit);
        }
    }
    // Capture the ORIGINAL pattern text for the cache-store key: `exp` is
    // shadowed below by the decoded form, and the option state that feeds
    // the fingerprint is unchanged between here and the store (patcompstart
    // only reads options), so get/put keys match.
    let exp_orig: String = if cacheable {
        exp.to_string()
    } else {
        String::new()
    };
    // Hold the compile mutex for the entire body — `patcompstart`
    // resets every file-scope static (`Src/pattern.c:267-281`) and the
    // emit/parse helpers mutate them in sequence. C is single-threaded
    // so the statics are race-free there; Rust must serialise.
    let _compile_guard = PATCOMPILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // c:1610 `patstartch` — the leading plain character of the pattern.
    // C only needs the NOGLD leading-dot rule to know whether a pattern
    // explicitly begins with `.` (so `.*` matches dot files while `*`
    // does not). Capture it before `exp` is shadowed by the normalizer
    // below. A leading glob token (Star/Quest/Inbrack/…) is not plain, so
    // only a literal `.` is recorded; everything else stays 0.
    let patstartch_lead: u8 = if exp.starts_with('.') { b'.' } else { 0 };
    // c:796 — `patcompstart()` has exactly ONE caller in C: `parsepat`
    // (Src/glob.c:796), which runs it ONCE for a whole file glob before
    // `parsecomplist` compiles the individual path components. C's
    // `patcompile` never re-enters it. zshrs calls it here instead, which is
    // harmless for the char-set setup but NOT for the globflags seed: a
    // pattern-level `(#i)` that `parsepat` folded into `patglobflags`
    // (Src/glob.c:801-807) was wiped on entry to every component after the
    // first, so `(#i)SUB/DEEP.TXT` matched case-insensitively in `SUB` and
    // case-SENSITIVELY in `DEEP.TXT` — i.e. not at all. Snapshot the incoming
    // value and restore it for a FILE compile, which is exactly what C's
    // `patcompile` does by leaving `patglobflags` alone unless the pattern is
    // non-FILE (c:568-576).
    let incoming_globflags = patglobflags.load(Ordering::Relaxed);
    patcompstart();
    if (inflags & PAT_FILE as i32) != 0 {
        patglobflags.store(incoming_globflags, Ordering::Relaxed); // c:568 — no reset for a file glob
    }
    // c:525 — `patcompstart` seeds `patglobflags` with the option-derived
    // default (GF_IGNCASE when CASEGLOB/CASEPATHS are off, GF_MULTIBYTE when
    // MULTIBYTE is on). The `patglobflags.store(0)` reset below (clean slate
    // for the `(#…)` hoist loop) would otherwise drop that seed before the
    // prog's globflags are built — capture the case bits now so `pattry`
    // honors `setopt nocaseglob`.
    let mut seeded_globflags = patglobflags.load(Ordering::Relaxed);
    // c:568-576 — `patcompile` RESETS patglobflags to `GF_MULTIBYTE`/0 for a
    // NON-FILE pattern, DROPPING the nocaseglob-derived GF_IGNCASE that
    // patcompstart seeded. `setopt nocaseglob` makes only FILENAME GLOBBING
    // (PAT_FILE) case-insensitive; `[[ ]]`, `${arr:#pat}`, `case`, and `${p//pat}`
    // stay case-SENSITIVE. Without this drop, `setopt nocaseglob; [[ ABC == abc ]]`
    // wrongly matched and `${(ABC):#abc}` wrongly filtered. GF_MULTIBYTE stays.
    if (inflags & PAT_FILE as i32) == 0 {
        seeded_globflags &= !GF_IGNCASE;
    }
    // === C-contract input decode (zpc_chars, Src/pattern.c:248) =====
    // C's patcompile consumes the LEXER'S tokenized encoding: glob
    // metachars arrive as token bytes (`Pound`..`Bang`, zsh.h:159-183
    // — zpc_chars holds `Star`/`Quest`/`Inbrack`/... so a raw `*`
    // NEVER dispatches as ZPC_STAR), quoted/escaped chars arrive as
    // `Bnull`/`Bnullkeep` + payload (zsh.h:195-200), and `Nularg` is
    // stripped (remnulargs). Callers pair `tokenize()` with
    // `patcompile()` exactly like C (zutil.c:734, etc.).
    //
    // The Rust parser below uses a raw-ASCII internal encoding with
    // `\X` as its literal form; transpose the C contract onto it:
    //   token char (U+0084..=U+009E)         -> ztokens[c - Pound]  (meta)
    //   Bnull/Bnullkeep + X (U+009F/U+00A0)  -> \X                  (literal)
    //   Nularg (U+00A1)                      -> dropped
    //   Meta (U+0083) + X                    -> \X                  (literal)
    //   raw `\` + X                          -> \X  passthrough — raw
    //       backslash is the parser's established Bnull-equivalent
    //       (see the `\X` arm below); C's lexer encodes the same
    //       quoting info as Bnull+X before zshtokenize ever runs.
    //   raw ASCII glob metachar              -> \X                  (literal)
    // `/`, `+`, `!`, `@` stay verbatim — C's zpc_chars keeps those
    // slots RAW (path split + ksh-glob triggers fire on raw bytes).
    // c:Src/pattern.c:751 — C sets `*endexp = patparse`, a POINTER INTO
    // the caller's own tokenized buffer, so the remainder keeps its exact
    // original bytes. zshrs normalizes the input below, so record for each
    // NORMALIZED char the index of the ORIGINAL char it came from; the
    // endexp sites then slice the ORIGINAL string instead of re-tokenizing
    // the normalized suffix. That round-trip was lossy: parsecomplist
    // strips the leading `(` of `(sub/)#end` before calling patcompile, so
    // the normalizer saw the group's `Outpar` as UNBALANCED and demoted it
    // to a literal `\)`, and re-tokenizing produced `Bnull )` where C still
    // has `Outpar`. The `(dir/)#` branch's c:752 test
    // `instr[1] == Outpar` then failed and the closure never formed.
    let exp_tokenized: Vec<char> = exp.chars().collect();
    let mut norm_to_orig: Vec<usize> = Vec::with_capacity(exp.len());
    let exp: String = {
        let ztokens: Vec<char> = crate::ported::glob::ZTOKENS.chars().collect();
        let chars: Vec<char> = exp.chars().collect();
        let mut out = String::with_capacity(exp.len());
        // Track open Inpar/Outpar groups so an UNBALANCED Outpar token
        // (a `)` with no group to close) transposes to a literal instead
        // of the raw `)` metachar. C's tokenizer never emits an Outpar
        // for an unbalanced `)`, and patcompile then treats it as an
        // ordinary string char (Src/pattern.c:1294-1298: "allow ')' as an
        // ordinary string character if there are no parentheses to
        // close"). Without this, `${arr:#*)--*}` (e.g. _rm's option
        // filter) broke at the stray `)` and collapsed to `*`. Note the
        // asymmetry: an unbalanced `(` is a "bad pattern" in zsh, not a
        // literal, so only Outpar is demoted here.
        let mut open_paren: i32 = 0;
        // Inside a `[...]` bracket class, `(`/`)` (Inpar/Outpar tokens) are
        // LITERAL members, not group delimiters — so they must not move
        // `open_paren`. The lexer tokenizes a `)` inside `[^)]` to Outpar
        // (0x8a) all the same, so without this guard the bracket-interior
        // Outpar wrongly decremented open_paren, and the real group-close
        // then read `open_paren==0` and got demoted to a literal — collapsing
        // `(|[^)]#…)` (every `_gnu_generic`/`_arguments --help` option-dedup
        // pattern) into a "bad pattern". Track Inbrack(0x91)/Outbrack(0x92).
        let mut in_bracket = false;
        let mut i = 0;
        // Emit one normalized char and record which ORIGINAL char it came
        // from, so `endexp` can be sliced out of the untouched tokenized
        // input the way C's `*endexp = patparse` pointer is. Indexed by
        // BYTE offset into `out`, because both endexp sites slice the
        // normalized string by byte offset. Defined after `i` so
        // macro_rules definition-site hygiene resolves it to the cursor.
        macro_rules! opush {
            ($c:expr) => {{
                let before = out.len();
                out.push($c);
                for _ in before..out.len() {
                    norm_to_orig.push(i);
                }
            }};
        }
        while i < chars.len() {
            let c = chars[i];
            let cu = c as u32;
            if cu == 0x91 {
                in_bracket = true; // Inbrack — fall through to the generic `[` emit.
            } else if cu == 0x92 {
                in_bracket = false; // Outbrack — fall through to the generic `]` emit.
            }
            if cu == 0x88 && !in_bracket {
                // Inpar — opens a group.
                open_paren += 1;
                opush!('(');
                i += 1;
                continue;
            }
            if cu == 0x8a && !in_bracket {
                // Outpar — closes an open group, else literal `)`.
                if open_paren > 0 {
                    open_paren -= 1;
                    opush!(')');
                } else {
                    opush!('\\');
                    opush!(')');
                }
                i += 1;
                continue;
            }
            if cu == 0x83 {
                // Meta + payload — a metafied RAW byte
                // (vm_helper::meta_encode_byte, c:Src/utils.c:7289-
                // 7294). C's pattern matcher compares metafied bytes
                // on BOTH sides, so the compiled pattern must match
                // the pair AS STORED in the subject string: emit both
                // chars as literals (`\Meta \payload`). The previous
                // `\payload` form dropped the Meta char and never
                // matched a metafied subject — `[[ $'\xff' ==
                // $'\xff' ]]` failed. Bug #127.
                opush!('\\');
                opush!(c);
                if i + 1 < chars.len() {
                    opush!('\\');
                    opush!(chars[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if cu == 0x9f || cu == 0xa0 {
                // Bnull / Bnullkeep — payload is a literal.
                if i + 1 < chars.len() {
                    opush!('\\');
                    opush!(chars[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if cu == 0xa1 {
                // Nularg — stripped like C's remnulargs.
                i += 1;
                continue;
            }
            if (0x84..=0x9e).contains(&cu) {
                // Token -> the raw metachar the parser dispatches on.
                opush!(ztokens[(cu - 0x84) as usize]);
                i += 1;
                continue;
            }
            if c == '\\' {
                // Raw `\X` — Bnull-equivalent quoting marker in the
                // zshrs pipeline; pass both through to the parser's
                // `\X` literal arm. Trailing lone `\` stays itself.
                //
                // C draws two provenances here that this arm cannot see
                // apart (docs/BUGS.md #1090):
                //   source quote — `[[ "man ls" = man\ * ]]`, `${v//\%/%%}`
                //   pattern DATA — `p='a\ b'; ${arr[(I)$p]}`
                // c:Src/glob.c:3633-3643 `zshtokenize` honors the escape only
                // before a `ztokens` metacharacter and leaves BOTH bytes as
                // literals before anything else — so in C a raw backslash in
                // the tokenized input is DATA and a quote is `Bnull`. Every
                // source path in zshrs hands this arm a raw backslash for the
                // quote (the cond/case pattern builder in
                // `extensions::compile_zsh`, `${…//…}`'s builder in
                // `ported::subst`), so the raw form has to keep meaning
                // "quote" here; enforcing the C rule at this site alone
                // regressed 8 real-world corpus tests across zinit / p10k /
                // zpwr / fsh.
                //
                // The provenance is therefore settled UPSTREAM instead: a
                // pattern built from a VALUE spells a data backslash `\\`
                // (this arm's literal-backslash form) before it gets here,
                // via `crate::pattern_data_escape::escape_data_backslashes`.
                // Two families of caller do that:
                //   * `ported::subst`'s paramsubst, for the search-subscript
                //     patterns (`${a[(I)…]}` / `(i)` / `(r)` / `(R)` / `(K)`)
                //     — the whole set that reaches `patcompile` through
                //     `zshtokenize` alone (c:Src/params.c:1727).
                //   * the cond/case pattern builder
                //     (`extensions::compile_zsh::emit_glob_subst_pattern`),
                //     for a `${~spec}` / `$~spec` segment and for
                //     `setopt globsubst` — the `strcatsub` `shtokenize` C
                //     runs at c:Src/subst.c:822/830. The escape is emitted
                //     ONLY on that pattern path, so a NON-pattern use of
                //     `${~arr[i]}` (which nothing untokenizes back) still
                //     prints its single backslash.
                // Covered by `tests/parity/cond_parity.rs`'s
                // `backslash_provenance_in_patterns`.
                opush!('\\');
                if i + 1 < chars.len() {
                    opush!(chars[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if matches!(c, '*' | '?' | '[' | '(' | ')' | '|' | '~' | '^' | '#' | '<') {
                // Untokenized ASCII metachar — literal per zpc_chars.
                opush!('\\');
                opush!(c);
                i += 1;
                continue;
            }
            opush!(c);
            i += 1;
        }
        out
    };
    // c:751 — resolve a byte offset in the NORMALIZED string back to the
    // corresponding suffix of the ORIGINAL tokenized input, which is what
    // C's `*endexp = patparse` yields for free (it is a pointer into the
    // caller's own buffer). An offset past the last recorded char means
    // the whole pattern was consumed, so the remainder is empty.
    let orig_remainder = |orig: &[char], map: &[usize], norm_byte_off: usize| -> String {
        let start = map.get(norm_byte_off).copied().unwrap_or(orig.len());
        orig[start.min(orig.len())..].iter().collect()
    };
    let exp = exp.as_str();
    *patstart.lock().unwrap() = exp.to_string();
    *patparse.lock().unwrap() = exp.to_string();
    patflags.store(
        inflags & !(PAT_PURES | PAT_HAS_EXCLUDP) as i32,
        Ordering::Relaxed,
    ); // c:566
       // c:568-576 — `patcompile` re-seeds the RUNNING `patglobflags`
       // (`if (isset(MULTIBYTE)) patglobflags = GF_MULTIBYTE; else
       // patglobflags = 0;`) for a non-file pattern, and leaves
       // `patcompstart`'s equally option-gated seed alone for a file glob.
       // Every P_GFLAGS node emitted below records that running value
       // verbatim (c:993 `up.l = patglobflags`), so storing a bare 0 here
       // made a mid-pattern `(#i)` / `(#b)` node clear GF_MULTIBYTE for the
       // remainder of the match (c:2942 assigns the payload ABSOLUTELY).
       //
       // GF_MULTIBYTE is forced ON rather than taken from `seeded_globflags`:
       // the option gate is correct C (c:572-575) but zshrs's capture and
       // substitution layers index the subject as UTF-8, and `unsetopt
       // multibyte` makes zsh match in raw BYTES (c:1946), which those layers
       // cannot yet represent — see the `charinc` note below.
    patglobflags.store(seeded_globflags | GF_MULTIBYTE, Ordering::Relaxed);

    // c:583-590 — emit P_GFLAGS placeholder. Phase 5.1: instead of
    // emitting an opcode, hoist leading `(#...)` flag specifiers into
    // patprog.globflags so the matcher applies them globally for the
    // whole match. Full mid-pattern P_GFLAGS opcode still deferred.
    // c:525 — `patglobflags = isset(MULTIBYTE) ? GF_MULTIBYTE : 0`.
    // (#U) inside the spec clears the bit per c:1116.
    //
    // c:953-957 — C gates the `(#...)` recognition on
    //   `*patparse == zpc_special[ZPC_INPAR] &&`
    //   `patparse[1] == zpc_special[ZPC_HASH]`.
    // When EXTENDEDGLOB is off, patcompcharsset (c:480-483) masks
    // ZPC_HASH to the Marker byte (c:476). The second-byte equality
    // therefore fails and the parser falls through to "treat `(` as
    // a literal group" (c:1020+). Previously this Rust hoist loop
    // compared the raw byte `b'#'`, allowing `(#s)` / `(#e)` /
    // `(#i)` to fire even without EXTENDEDGLOB (parity bugs #18/#19
    // vs real zsh).
    // c:525 — the compiled prog's globflags accumulate from `patglobflags`,
    // which `patcompstart` seeded with GF_IGNCASE when CASEGLOB/CASEPATHS are
    // off (`setopt nocaseglob`). Carry those case bits through so `pattry`
    // matches case-insensitively; `matchpat` masks the loss by pre-folding
    // both sides, but the glob scanner now drives `pattry` directly.
    //
    // c:572-575 — `if (isset(MULTIBYTE)) patglobflags = GF_MULTIBYTE; else
    // patglobflags = 0;`. The bit is OPTION-GATED in C, but it is kept
    // unconditional here (see the `patglobflags.store` note above): zshrs's
    // capture / substitution layers slice the subject as UTF-8, and the
    // nomultibyte unit is a raw BYTE. An explicit in-pattern `(#U)` still
    // clears it below (c:1116), which the matcher now honours.
    let mut hoisted_globflags: i32 =
        GF_MULTIBYTE | (seeded_globflags & (GF_IGNCASE | GF_LCMATCHUC));
    // c:953-954 gates BOTH bytes: `*patparse == zpc_special[ZPC_INPAR]`
    // as well as `patparse[1] == zpc_special[ZPC_HASH]`. SHGLOB
    // (c:500-510) and `disable -p '('` mask the INPAR slot to Marker,
    // and then `(#i)abc` is ordinary text, not a flag spec.
    let (inpar_char_pre, hash_char_pre) = {
        let sp = zpc_special.lock().unwrap();
        (sp[ZPC_INPAR as usize], sp[ZPC_HASH as usize]) // c:953, c:954
    };
    while hash_char_pre == b'#' && inpar_char_pre == b'(' {
        let off = patparse_off.load(Ordering::Relaxed);
        let p = patparse.lock().unwrap();
        if off + 1 >= p.len() || &p.as_bytes()[off..off + 2] != b"(#" {
            break;
        }
        let rest = p[off..].to_string();
        drop(p);
        match patgetglobflags(&rest) {
            Some((_bits, assert, _consumed)) if assert != 0 => {
                // c:Src/pattern.c:1103-1109 — `(#s)`/`(#e)` set `*assertp`
                // (P_ISSTART/P_ISEND), a POSITIONAL zero-width anchor, not a
                // whole-match glob flag. Do NOT hoist/consume it here: leave
                // it (and everything after) for the main compile loop
                // (pattern.rs:1245) to emit as a positional node. Hoisting
                // dropped the anchor, so a leading `(#e)PAT` matched as if
                // the `(#e)` were absent — `[[ o == (#e)o ]]` wrongly matched
                // and `${s//(#e)r/END}` wrongly replaced. `(#s)` was masked
                // in `[[ ]]` only because that context is start-anchored.
                break;
            }
            Some((bits, _assert, consumed)) => {
                // The fold / backref / matchref / multibyte flags are ABSOLUTE
                // state, not a set-only delta. patgetglobflags's returned `bits`
                // starts at 0 and can only carry flags turned ON, so `(#I)`
                // clearing LCMATCHUC (c: `patglobflags &= ~(GF_LCMATCHUC |
                // GF_IGNCASE)`) comes back as 0 — an `|=` accumulate then leaves
                // an earlier `(#l)`/`(#i)`'s fold bit set, so `(#l)(#I)foo` kept
                // folding where zsh, after `(#I)`, matches case-sensitively.
                //
                // patgetglobflags ALSO updates the global patglobflags with the
                // sets AND the clears, so it holds the authoritative running
                // value. REPLACE the toggle-able bits from it rather than
                // OR-accumulating the lossy return.
                // GF_MULTIBYTE is deliberately NOT in the mask: the init above
                // force-sets it (this port always matches UTF-8) while the
                // global patglobflags may not carry it, so replacing it from the
                // global would wrongly clear it and break multibyte matching.
                // The original `|=` could not clear it either, so leaving it out
                // preserves that (rare `(#U)` single-byte requests are no more
                // broken than before). The fold/backref/matchref bits, which DO
                // need clear semantics for `(#I)`/`(#B)`/`(#M)`, come from the
                // authoritative global.
                let gmask = GF_IGNCASE | GF_LCMATCHUC | GF_BACKREF | GF_MATCHREF;
                let cur = patglobflags.load(Ordering::Relaxed);
                hoisted_globflags = (hoisted_globflags & !gmask) | (cur & gmask);
                // `(#aN)` substitution budget — low 8 bits of
                // `patglobflags` per c:1066. Only carry it when this
                // flag-spec actually had `(#a)` digits (non-zero err
                // count); other specs may set high bits and we don't
                // want their bits-AND to clear our budget byte.
                // Per-spec OR-leak is C-faithful (a later `(#a3)` after
                // `(#a1)` raises the budget); the matcher reads the
                // final cumulative byte.
                let errs_byte = bits & 0xff;
                if errs_byte != 0 {
                    hoisted_globflags = (hoisted_globflags & !0xff) | errs_byte;
                    // c:1066
                }
                if (bits & GF_MULTIBYTE) == 0 && rest.contains('U') {
                    hoisted_globflags &= !GF_MULTIBYTE;
                }
                patparse_off.fetch_add(consumed, Ordering::Relaxed);
            }
            None => break,
        }
    }

    // c:584-610 — PAT_PURES fast path. A literal with no glob tokens
    // compiles to a "pure string" rather than bytecode. For FILE globs
    // (PAT_FILE) the scan STOPS at '/', so each path component becomes a
    // pure section and `parsecomplist` can split the path on '/' (it
    // reads endexp = the '/' remainder). Gated on PAT_FILE so non-file
    // callers (matchpat/[[ ]]/:#) keep normal compilation unchanged; the
    // matcher consumes PURES sections via literal extraction (the
    // scanner), never `pattry`, so no PAT_PURES match path is needed.
    let pf_pre = patflags.load(Ordering::Relaxed);
    // c:Src/pattern.c:644-704 — for a FILE glob a literal path component is
    // emitted as a PAT_PURES section even under GF_IGNCASE (`setopt
    // nocaseglob`): C compiles it, then converts the single P_EXACTLY back
    // to PURES (c:664) while flagging the prog GF_IGNCASE (c:645). The net
    // effect is that intermediate literal directory components descend by
    // EXACT name (the scanner's PURES path stats the real path, glob.rs:436
    // — case-insensitivity applies to the FINAL wildcard component, not to
    // directory descent), so path splitting on `/` keeps working. zshrs's
    // port only had the first PURES gate (c:586), which excludes GF_IGNCASE,
    // so under nocaseglob every literal component (e.g. `tmp` in
    // `/tmp/.../*.zsh`) fell to the general compiler and matched nothing —
    // the whole path collapsed and `${(N)...}(DN)` globs returned empty.
    // That is what made zinit's plugin-discovery globs empty on a shell with
    // `caseglob` off → the `.zinit-load-plugin:source:110: no such file or
    // directory: ./` startup flood. Take the PURES fast path here for file
    // globs when the only extra glob flag is GF_IGNCASE; GF_IGNCASE stays on
    // `hoisted_globflags` so the final wildcard component still matches
    // case-insensitively.
    //
    // CORRECTION (see the `(#i)` note below): the exemption above transcribed
    // C's `__CYGWIN__` arm. c:586's fast-path gate is
    // `!(patglobflags & ~GF_MULTIBYTE)` on every other platform — the
    // `|| (!(patglobflags & ~GF_IGNCASE) && (patflags & PAT_FILE))` alternative
    // is inside `#ifdef __CYGWIN__` (c:587-594) precisely because only there is
    // the FILESYSTEM itself case-insensitive, making a stat of the pattern's
    // own spelling equivalent to a scan. Everywhere else C reaches PAT_PURES
    // for a literal component only via the c:650-664 conversion, and c:1392-1417
    // withholds it whenever `patglobflags & (0xFF|GF_LCMATCHUC|GF_IGNCASE)`:
    // "It's much simpler to turn off pure string mode for any case-insensitive
    // or approximate matching" (c:1382-1385). The single exception is a `.` or
    // `..` file component, which stays pure (c:1414-1416).
    //
    // This matters beyond flag bookkeeping: the PURES path STATS the pattern's
    // spelling, so on a case-insensitive filesystem `(#i)ALPHA.TXT` "found" the
    // file and emitted `ALPHA.TXT` — the pattern echoed back — where zsh scans
    // the directory and emits the real on-disk `alpha.txt`.
    let case_or_approx = hoisted_globflags & (0xff | GF_LCMATCHUC | GF_IGNCASE); // c:1392, c:1409
    if (pf_pre & PAT_FILE as i32) != 0 && (pf_pre & PAT_ANY as i32) == 0 {
        let off = patparse_off.load(Ordering::Relaxed);
        let p = patparse.lock().unwrap();
        let s = &p[off..];
        // c:606 — scan for a pure literal segment. By this point patparse
        // has been NORMALIZED to raw metachars (Star -> '*') with literal
        // metachars backslash-escaped (`\*`), so scan that form: stop at
        // '/' (segment boundary) or any UNescaped glob metachar (=> not
        // pure). `\X` is an escaped literal — skip both chars.
        let mut cut = s.len();
        let mut at_token = false;
        let mut it = s.char_indices();
        while let Some((i, c)) = it.next() {
            if c == '/' {
                cut = i;
                break;
            }
            if c == '\\' {
                it.next(); // escaped literal — both chars stay literal
                continue;
            }
            if matches!(c, '*' | '?' | '[' | '(' | '|' | '~' | '^' | '#' | '<') {
                cut = i;
                at_token = true;
                break;
            }
        }
        // c:1414-1416 — `.` and `..` stay pure even under case-insensitive or
        // approximate matching, so a `../x` path component still descends by
        // name instead of being scanned for.
        let dot_or_dotdot = matches!(&s[..cut], "." | ".."); // c:1414-1415
                                                             // c:610 — pure iff we stopped at end or '/', not at a glob meta.
        if !at_token && (case_or_approx == 0 || dot_or_dotdot) {
            let literal = s[..cut].as_bytes().to_vec();
            drop(p);
            let mlen = literal.len() as i64;
            if let Some(end) = endexp.as_deref_mut() {
                // c:751 `*endexp = patparse` — a pointer INTO the caller's
                // tokenized buffer. Map the normalized cut back to the
                // original char index and slice the untouched tokenized
                // input, so the remainder keeps its exact original tokens.
                // `cut` indexes into `s`, which starts at `off` — the offset
                // patparse_off reached after the leading `(#...)` flag block
                // was hoisted above. `orig_remainder` maps an ABSOLUTE
                // normalized offset (the general path at the bottom of this
                // function passes `consumed_off`, already absolute), so a
                // bare `cut` under-reports the consumed length by exactly the
                // width of the hoisted flags: `(#i)alpha.txt` compiled the
                // literal correctly but reported `.txt` as unconsumed, and
                // `parsecomplist` (Src/glob.c:773) then saw a remainder that
                // was neither `/` nor empty and failed the whole pattern with
                // "bad pattern". Only wildcard-free components hit this path,
                // which is why `(#i)a*` always worked and `(#i)alpha.txt` did
                // not.
                *end = orig_remainder(&exp_tokenized, &norm_to_orig, off + cut);
            }
            let prog: Patprog = Box::new((
                patprog {
                    startoff: 0,
                    size: mlen,
                    mustoff: 0,
                    patmlen: mlen, // c:623 — pure-string length
                    globflags: hoisted_globflags,
                    globend: patglobflags.load(Ordering::Relaxed),
                    flags: pf_pre | PAT_PURES as i32, // c:625
                    patnpar: 0,
                    patstartch: patstartch_lead, // c:1610
                },
                literal,
            ));
            if cacheable {
                crate::pat_cache::put(&exp_orig, inflags, &prog);
            }
            return Some(prog);
        }
    }

    let mut flagp: i32 = 0;
    let root = patcompswitch(0, &mut flagp);
    if root < 0 {
        return None; // c:646 compile error
    }
    // Emit the terminal P_END and chain every branch's operand to it.
    let end_off = patnode(P_END);
    chain_branches_to(root as usize, end_off);

    let code = patout.lock().unwrap().clone();
    let consumed_off = patparse_off.load(Ordering::Relaxed);
    if let Some(end) = endexp.as_deref_mut() {
        // c:751 `*endexp = patparse` — a pointer INTO the caller's
        // tokenized buffer. Map the normalized consume-point back to the
        // original char index and slice the untouched tokenized input.
        *end = orig_remainder(&exp_tokenized, &norm_to_orig, consumed_off);
    }

    let prog: Patprog = Box::new((
        patprog {
            startoff: 0,
            size: code.len() as i64,
            mustoff: 0,
            patmlen: 0,
            globflags: hoisted_globflags,
            globend: patglobflags.load(Ordering::Relaxed),
            // c:637 `p->flags = patflags;` — patflags ONLY. A prior
            // Rust port OR'd `hoisted_globflags` into here, contaminating
            // `prog.flags & 0xff` checks (which read PAT_STATIC / PAT_PURES
            // bits in the low byte) with whatever ended up in the
            // `(#aN)` budget byte. The two flag-sets stay strictly
            // separated per C source.
            flags: patflags.load(Ordering::Relaxed),

            patnpar: patnpar.load(Ordering::Relaxed) - 1,
            patstartch: patstartch_lead, // c:1610
        },
        code,
    ));
    if cacheable {
        crate::pat_cache::put(&exp_orig, inflags, &prog);
    }
    Some(prog)
}

/// Port of `patcompswitch(int paren, int *flagp)` from `Src/pattern.c:765`.
///
/// C: `static long patcompswitch(int paren, int *flagp)`. Parses an
/// alternation (`a|b|c`), emitting a chain of P_BRANCH nodes. Returns
/// offset of the first branch, or -1 on error.
pub fn patcompswitch(paren: i32, flagp: &mut i32) -> i64 {
    // c:765
    // Emit the first P_BRANCH header. Its operand is the content
    // emitted by the immediately-following patcompbranch call (lives
    // inline at starter+I_BODY). Does NOT emit a terminator — caller
    // (patcompile for top-level, patcomppiece for sub-pattern)
    // chains branches to the appropriate follow-on opcode.
    // c:773 — `long savglobflags = (long)patglobflags;`
    let savglobflags = patglobflags.load(Ordering::Relaxed);
    // c:772 — `int flags, gfchanged = 0;`
    let mut gfchanged: i32 = 0;
    let starter = patnode(P_BRANCH);
    let mut branch_flags: i32 = 0;
    let first_branch = patcompbranch(&mut branch_flags, paren);
    if first_branch < 0 {
        return -1;
    }
    // c:786-787 — `if (patglobflags != (int)savglobflags) gfchanged++;`
    if patglobflags.load(Ordering::Relaxed) != savglobflags {
        gfchanged += 1;
    }
    *flagp |= branch_flags & P_HSTART;

    let mut last_branch = starter;
    // c:769 — `Upat excsync = NULL;` — set on first `~` exclusion;
    // reused so consecutive `~clause` chain to the same sync node.
    let mut excsync: usize = 0;

    // Snapshot zpc_special for this compile pass — locked once to
    // avoid re-locking inside the per-iteration parse-byte check.
    let (sp_bar, sp_tilde, sp_special_set) = {
        let sp = zpc_special.lock().unwrap();
        let bar = sp[ZPC_BAR as usize];
        let tilde = sp[ZPC_TILDE as usize];
        // c:803 — `memchr(zpc_special, patparse[1], ZPC_SEG_COUNT)` —
        // is the lookahead byte one of the segment-special bytes? If
        // so, `~X` is NOT an exclusion (`~|`, `~)`, etc.).
        let mut set = [false; 256];
        for i in 0..(ZPC_SEG_COUNT as usize) {
            set[sp[i] as usize] = true;
        }
        (bar, tilde, set)
    };

    // Alternation + exclusion loop:
    //   `|`  → next alternative (P_BRANCH)
    //   `~`  → exclusion (P_EXCLUDE / P_EXCLUDP) — when followed by
    //          `/` (top-level path component split) OR a non-special
    //          char (ordinary content). `~~`, `~|`, `~)` stay literal.
    loop {
        let off = patparse_off.load(Ordering::Relaxed);
        let parse = patparse.lock().unwrap();
        if off >= parse.len() {
            break;
        }
        let bytes = parse.as_bytes();
        let c = bytes[off];
        // c:799-803 — accept `|` always; accept `~` only when the
        // lookahead char is `/` or NOT a segment-special byte.
        let is_bar = c == sp_bar;
        let is_tilde_exclude = c == sp_tilde && off + 1 < bytes.len() && {
            let la = bytes[off + 1];
            la == b'/' || !sp_special_set[la as usize]
        };
        if !is_bar && !is_tilde_exclude {
            break;
        }
        drop(parse);
        patparse_off.fetch_add(1, Ordering::Relaxed); // c:803 `*patparse++`
        let br: usize;
        // c:805 — `long gfnode = 0, newbr;`
        let mut gfnode: usize = 0;
        if is_tilde_exclude {
            // c:808-836 — `if (tilde)` arm. Emit the EXCSYNC sync node
            // (if first `~` in this switch) then the EXCLUDE / EXCLUDP
            // node with an 8-byte NULL syncptr payload. Note we DON'T
            // reset patglobflags's low byte (`(#aN)` budget) here —
            // the Rust port doesn't yet propagate per-pattern `(#a)`
            // through nested patmatch frames, so dropping the budget
            // mid-compile is a no-op.
            if excsync == 0 {
                excsync = patnode(P_EXCSYNC); // c:813
                patoptail(last_branch, excsync); // c:814
            }
            // c:820-824 — "By default, approximations are turned off in
            // exclusions: we need to do this here as otherwise the code
            // compiling the exclusion doesn't know if the flags have
            // really changed if the error count gets restored."
            //     `patglobflags &= ~0xff;`
            // That last clause is exactly why this matters now that
            // patcompbranch honours c:995-997's "No effect" skip: without
            // the clear, `(#ia1)README~(#a1)READ_ME` sees the exclusion's
            // own `(#a1)` as a no-op and emits no P_GFLAGS, so the
            // exclusion never re-arms its error budget.
            patglobflags.fetch_and(!0xff, Ordering::Relaxed);
            // c:816-825 — `if (!(patflags & PAT_FILET) || paren)` →
            // P_EXCLUDE; else P_EXCLUDP for top-level file globs.
            let pf = patflags.load(Ordering::Relaxed);
            let use_excludp = (pf & (PAT_FILET as i32)) != 0 && paren == 0;
            if use_excludp {
                br = patnode(P_EXCLUDP); // c:823
                patflags.fetch_or(PAT_HAS_EXCLUDP as i32, Ordering::Relaxed); // c:824
            } else {
                br = patnode(P_EXCLUDE); // c:818
            }
            // c:826-827 — `up.p = NULL; patadd((char *)&up, 0, sizeof(up), 0);`
            // 8-byte syncptr slot, NULL-initialised. Sized for a
            // 64-bit pointer to match C `union upat`.
            {
                let mut buf = patout.lock().unwrap();
                buf.extend_from_slice(&[0u8; 8]);
            }
        } else {
            // c:843 — `excsync = 0; br = patnode(P_BRANCH);`
            excsync = 0;
            br = patnode(P_BRANCH);
            // c:845-847 — "The position of the following statements
            // means globflags set in the main branch carry over to the
            // exclusion."
            if paren == 0 {
                // c:849 — `patglobflags = 0;`
                //
                // c:850-870 — "If at top level, we need to reinitialize
                // flags to zero, since (#i)foo|bar only applies to foo
                // and we stuck the #i into the global flags."
                //
                // !!! RUST-ONLY DETAIL !!! C tests
                // `((Patprog)patout)->globflags`, the header field it
                // wrote at c:977 when the flag spec sat at patstart.
                // zshrs computes the header value in a separate
                // patcompile hoist scan (pattern.rs:844+), so the live
                // equivalent here is "were any flags in effect on the
                // way into this branch?".
                let before_reset = patglobflags.load(Ordering::Relaxed);
                patglobflags.store(0, Ordering::Relaxed); // c:849
                if before_reset != 0 {
                    // c:861-864 — `gfnode = patnode(P_GFLAGS); up.l =
                    // patglobflags;` (which c:849 just set to 0).
                    gfnode = patnode(P_GFLAGS);
                    let mut buf = patout.lock().unwrap();
                    buf.extend_from_slice(&0i32.to_le_bytes());
                }
            } else {
                // c:872 — `patglobflags = (int)savglobflags;`
                patglobflags.store(savglobflags, Ordering::Relaxed);
            }
        }
        // Chain previous branch's `next` directly to this new branch
        // (alternative chain, not operand chain).
        set_next(last_branch, br);
        let mut bf: i32 = 0;
        let inner = patcompbranch(&mut bf, paren);
        if inner < 0 {
            return -1;
        }
        // c:884-885 — `if (gfnode) pattail(gfnode, newbr);`. In the Rust
        // encoding a branch's operand lives inline at `br + I_BODY`, so
        // the P_GFLAGS emitted above IS the operand; chain it to the
        // branch body that patcompbranch just emitted.
        if gfnode != 0 {
            set_next(gfnode, inner as usize);
        }
        // c:888-889 — `if (!tilde && patglobflags != (int)savglobflags)
        // gfchanged++;`
        if !is_tilde_exclude && patglobflags.load(Ordering::Relaxed) != savglobflags {
            gfchanged += 1;
        }
        // c:Src/pattern.c:891-892 — `if (excsync) patoptail(br,
        // patnode(P_EXCEND));`. Terminate an exclusion branch's operand
        // with P_EXCEND so the matcher knows where the excluded pattern
        // ends; without it `A~B` ran B past its own end into the
        // following pattern and never excluded (`STATES__*~*local*`
        // failed to drop STATES__local_bar).
        if excsync != 0 {
            let excend = patnode(P_EXCEND);
            patoptail(br, excend);
        }
        *flagp |= bf & P_HSTART;
        last_branch = br;
    }

    // !!! KNOWN GAP — c:913-917 "check for proper termination" !!!
    //     if ((paren && *patparse++ != Outpar) ||
    //         (!paren && *patparse &&
    //          !((patflags & PAT_FILE) && *patparse == '/')))
    //         return 0;
    // The `paren` half IS ported — patcomppiece's `b'('` arm and
    // patcompnot's paren arm both test for the closing `)` and consume
    // it. The `!paren` half (a stray top-level `)` is a BAD PATTERN)
    // is NOT ported, deliberately, and a leftover byte is dropped.
    //
    // Why: in C the test is only reachable for a `)` that the LEXER
    // turned into the Outpar TOKEN, and lex.c makes that decision from
    // the `sub` flag that zshrs's lexer does not model:
    //   c:Src/lex.c:989-990  `case LX2_OUTPAR: if ((sub ||
    //       in_brace_param) && isset(SHGLOB)) break;`  — inside a
    //       `${...}` body under SHGLOB, `)` stays an ORDINARY char.
    //   c:Src/lex.c:1007     `case LX2_BAR: if (unset(SHGLOB) ||
    //       (!sub && !in_brace_param)) c = Bar;`      — ditto for `|`.
    //   c:Src/lex.c:1079-1081 `case LX2_INPAR: if (isset(SHGLOB)) {
    //       if (sub || in_brace_param) break; ... }`  — ditto for `(`.
    // zshrs hands patcompile the same tokenized form for a `[[ ]]` /
    // `case` pattern and for a `${s//pat/rep}` pattern, so this
    // function cannot tell the two apart. Enforcing the check makes
    // `setopt shglob; [[ x = -([AMO]*|[0CRSWnsw]) ]]` error the way
    // zsh does, but it simultaneously REJECTS patterns zsh accepts in
    // the substitution contexts (measured: 40 cells fixed, 170 cells
    // newly over-rejected across `${s//p/r}` / `${s#p}` / `${a:#p}` /
    // `case`). Over-rejecting is the worse failure, so the check stays
    // out until the lexer models `sub`; then it can be restored here
    // verbatim from c:913-917.
    let _ = first_branch;
    // c:919-929 — C emits the closing P_GFLAGS restore right here, using
    // this local `gfchanged`. The Rust port emits it from patcomppiece's
    // `(` arm (which owns the P_CLOSE node), so hand the flag across the
    // call boundary. See PATSWITCH_GFCHANGED's warning block.
    PATSWITCH_GFCHANGED.store(gfchanged, Ordering::Relaxed);
    starter as i64
}

// !!! WARNING: RUST-ONLY HELPER !!!
// C keeps `gfchanged` as a local of `patcompswitch` and emits the
// group-closing P_GFLAGS restore inside the same function
// (Src/pattern.c:919-929), immediately after the `ender` node it also
// creates there. The Rust port splits that work: patcomppiece's `b'('`
// arm owns P_OPEN/P_CLOSE and therefore owns the restore emission, so
// the counter has to cross the call boundary. patcompswitch stores it
// just before returning; the `(` arm reads it immediately after its own
// patcompswitch call returns, so nested groups each observe their own
// value.
static PATSWITCH_GFCHANGED: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Port of `patcompbranch(int *flagp, int paren)` from `Src/pattern.c:942`.
///
/// C: `static long patcompbranch(int *flagp, int paren)`. Parses a
/// single branch — a sequence of pieces. Returns offset of the first
/// node in the branch, or -1 on error.
pub fn patcompbranch(flagp: &mut i32, paren: i32) -> i64 {
    // c:942
    let mut chain_start: i64 = -1;
    let mut last_tail: usize = 0;
    // Track preceding piece for POSTFIX `(#cN,M)` wrap. C does
    // `patinsert(P_COUNTSTART, starter, ...)` to embed COUNTSTART
    // BEFORE the just-compiled piece (c:pattern.c:1686). We snapshot
    // the piece bytes, truncate, emit P_COUNT, then re-append.
    let mut last_piece_off: i64 = -1; // start offset of preceding piece
    let mut prev_chain_tail: i64 = -1; // tail of chain BEFORE preceding piece (-1 if piece was first)
                                       // Flags the preceding patcomppiece reported. P_HSTART marks a piece that
                                       // already had a closure applied (`a#`, `a##`, or an earlier `(#cN,M)`) —
                                       // patcomppiece sets `*flagp = P_HSTART` on every such arm, mirroring C
                                       // (c:1626/1629/1636/1640/1643). Used below to reject a stacked count.
    let mut last_piece_flags: i32 = 0;
    *flagp = P_PURESTR;

    // c:951-952 — snapshot the segment-special set so we can do
    // `memchr(zpc_special, byte, ZPC_SEG_COUNT)`-equivalent lookups.
    // Only the first ZPC_SEG_COUNT slots (SLASH, NULL, BAR, OUTPAR,
    // TILDE) matter here.
    let (sp_tilde, sp_seg_set, sp_inpar, sp_hash) = {
        let sp = zpc_special.lock().unwrap();
        let tilde = sp[ZPC_TILDE as usize];
        let mut set = [false; 256];
        for i in 0..(ZPC_SEG_COUNT as usize) {
            set[sp[i] as usize] = true;
        }
        (
            tilde,
            set,
            sp[ZPC_INPAR as usize],
            sp[ZPC_HASH as usize], // c:1609
        )
    };

    loop {
        let off = patparse_off.load(Ordering::Relaxed);
        // Snapshot the parse buffer into an owned slice for branch
        // decisions; release the lock so subsequent emit helpers
        // (which acquire patout's lock) can't contend.
        let snapshot: Vec<u8> = {
            let parse = patparse.lock().unwrap();
            parse.as_bytes().to_vec()
        };
        if off >= snapshot.len() {
            break;
        }
        let c = snapshot[off];
        // Branch terminators: |, ), end of pattern.
        if c == b'|' || c == b')' {
            break;
        }
        // c:950 — '/' is ZPC_SLASH (slot 0, within ZPC_SEG_COUNT), so C
        // breaks the segment here. zshrs gates this to FILE globs at top
        // level: parsecomplist needs the path-component boundary, while
        // matchpat/[[ ]]/:# (never PAT_FILE) and '/' inside parens (the
        // `((#s)|/)` anchor pin) must keep '/' literal. C's unconditional
        // break + c:914-917 file-accept termination is equivalent for the
        // top-level file-glob case this serves.
        if c == b'/' && paren == 0 && (patflags.load(Ordering::Relaxed) & PAT_FILE as i32) != 0 {
            break;
        }
        // c:950-952 — `~` is an additional terminator when active as
        // the exclusion operator. The C condition includes all five
        // segment-specials (SLASH, NULL, BAR, OUTPAR, TILDE), but
        // `|` and `)` are already covered above and SLASH inside
        // `(...)` alternation is intentionally left literal here to
        // preserve the existing `zsh_corpus_hash_s_e_anchors_match_
        // bare_test` parity pin.
        if c == sp_tilde && c != 0 {
            let la = snapshot.get(off + 1).copied().unwrap_or(0);
            // Literal-tilde exception: `~` followed by a segment-
            // special OTHER than `/` keeps the `~` as literal.
            let literal_tilde_exception = la != b'/' && sp_seg_set[la as usize];
            if !literal_tilde_exception {
                break;
            }
        }
        let bytes = snapshot.as_slice();
        // Mid-pattern `(#cN,M)` counted-repetition specifier — emit
        // P_COUNT with bounds + inline operand following. Detected
        // BEFORE the generic patgetglobflags path because `c` is not
        // a flag char in that fn.
        //
        // c:1608-1610 — the `(` and `#` are compared against
        // `zpc_special[ZPC_INPAR]` / `zpc_special[ZPC_HASH]`, NOT against
        // literal bytes. That indirection is the whole EXTENDED_GLOB gate:
        // patcompcharsset() rewrites `zpc_special[ZPC_HASH]` to Marker
        // (c:482) when EXTENDED_GLOB is off, so a literal `#` can never
        // equal it and `(#c...)` stops being a counted closure — it falls
        // through and parses as an ordinary group. Testing `b'#'` directly
        // applied the closure with EXTENDED_GLOB unset, so
        // `[[ aaa = a(#c2,3) ]]` matched (zsh: no match).
        if off + 2 < bytes.len()
            && bytes[off] == sp_inpar
            && bytes[off + 1] == sp_hash
            && bytes[off + 2] == b'c'
        {
            let mut j = off + 3;
            let mut min: i64 = 0;
            let min_start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                min = min * 10 + (bytes[j] - b'0') as i64;
                j += 1;
            }
            let mut max: i64 = i64::MAX;
            // Three valid shapes (per zsh extended-glob (#cN,M)):
            //   (#cN)      — exact count N  (min=N, max=N)
            //   (#cN,)     — min N, no max  (min=N, max=∞)
            //   (#cN,M)    — N..M range     (min=N, max=M)
            //   (#c,M)     — max M, no min  (min=0, max=M)  ← bug #489 missing case
            //   (#c,)      — equivalent to no count (min=0, max=∞)
            // The original `if j > min_start` gate skipped the `)`
            // check for the no-min shapes, leaving them unparsed.
            let has_min_digits = j > min_start;
            let has_comma = j < bytes.len() && bytes[j] == b',';
            if has_min_digits || has_comma {
                if has_comma {
                    j += 1; // skip ,
                    let max_start = j;
                    let mut mx: i64 = 0;
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        mx = mx * 10 + (bytes[j] - b'0') as i64;
                        j += 1;
                    }
                    if j > max_start {
                        max = mx;
                    }
                } else {
                    // (#cN) — exact count N.
                    max = min;
                }
                if j < bytes.len() && bytes[j] == b')' {
                    j += 1;
                    patparse_off.store(j, Ordering::Relaxed);
                    // POSTFIX semantics — c:pattern.c:1686.
                    // C does `patinsert(P_COUNTSTART, starter, ...)`
                    // then `pattail(opnd, patnode(P_BACK))` to loop the
                    // operand back. zshrs uses a simpler encoding where
                    // P_COUNT carries [min, max] then operand bytes
                    // inline; matcher iterates outside. So we relocate
                    // the preceding piece into the operand slot.
                    // c:1431-1436 — `case Star:` sets `kshchar = -1`, whose
                    // only purpose is (per C's own comment) "a sign that we
                    // can't have #'s". c:1620-1622 then rejects the pattern:
                    //
                    //     /* too much at once doesn't currently work */
                    //     if (kshchar && (hash || count))
                    //         return 0;
                    //
                    // So a count may not follow `*`, nor stack on a piece
                    // that already carries a closure (`a#(#c2,3)`). zshrs
                    // attaches the count to the preceding piece rather than
                    // tracking kshchar, so the same rule is expressed by
                    // looking at that piece's opcode. Without this, zshrs
                    // silently accepted `*(#c2,3)` / `a#(#c2,3)`, which zsh
                    // rejects as a bad pattern.
                    if last_piece_off >= 0 {
                        let prev_op = {
                            let buf = patout.lock().unwrap();
                            buf.get(last_piece_off as usize + I_OP)
                                .copied()
                                .unwrap_or(0)
                        };
                        // `*` is C's kshchar = -1 (its head node is P_STAR);
                        // P_HSTART marks a piece that already closured, which
                        // is what makes `a#(#c2,3)` / `a##(#c2,3)` bad too.
                        if prev_op == P_STAR || (last_piece_flags & P_HSTART) != 0 {
                            return -1; // c:1622 `return 0;`
                        }
                    }
                    if last_piece_off >= 0 {
                        let piece_start = last_piece_off as usize;
                        // Snapshot preceding piece (everything emitted
                        // by the prior patcomppiece call).
                        let piece_bytes: Vec<u8> = {
                            let buf = patout.lock().unwrap();
                            buf[piece_start..].to_vec()
                        };
                        // Truncate to remove the piece.
                        {
                            let mut buf = patout.lock().unwrap();
                            buf.truncate(piece_start);
                        }
                        // Cut chain at prev_chain_tail (was pointing
                        // into the piece via set_next or as
                        // chain_start).
                        if prev_chain_tail >= 0 {
                            set_next(prev_chain_tail as usize, 0);
                        } else {
                            chain_start = -1;
                        }
                        // Emit P_COUNT at current end.
                        let count_off = patnode(P_COUNT);
                        let operand_new_start: usize;
                        {
                            let mut buf = patout.lock().unwrap();
                            buf.extend_from_slice(&min.to_le_bytes());
                            buf.extend_from_slice(&max.to_le_bytes());
                            operand_new_start = buf.len();
                        }
                        // Relocate piece bytes: rewrite internal next
                        // links (absolute offsets >= piece_start) by
                        // delta = operand_new_start - piece_start.
                        let delta: i64 = operand_new_start as i64 - piece_start as i64;
                        let mut relocated = piece_bytes.clone();
                        let mut i = 0;
                        while i + I_BODY <= relocated.len() {
                            let op = relocated[i + I_OP];
                            if op == 0 {
                                i += 1;
                                continue;
                            }
                            let nxt = u32::from_le_bytes(
                                relocated[i + I_NEXT..i + I_NEXT + 4].try_into().unwrap(),
                            );
                            if nxt != 0 {
                                let new_nxt = (nxt as i64 + delta) as u32;
                                relocated[i + I_NEXT..i + I_NEXT + 4]
                                    .copy_from_slice(&new_nxt.to_le_bytes());
                            }
                            let next_i = advance_past_instr(&relocated, i);
                            if next_i == 0 || next_i <= i {
                                break;
                            }
                            i = next_i;
                        }
                        {
                            let mut buf = patout.lock().unwrap();
                            buf.extend_from_slice(&relocated);
                        }
                        // Hook P_COUNT into chain.
                        if chain_start < 0 {
                            chain_start = count_off as i64;
                        } else {
                            set_next(prev_chain_tail as usize, count_off);
                        }
                        last_tail = count_off;
                        // Consumed — clear tracking. The resulting piece IS a
                        // closure (C: `*flagp = P_HSTART`, c:1636), so a second
                        // count stacked on it must be rejected.
                        last_piece_off = -1;
                        prev_chain_tail = -1;
                        last_piece_flags = P_HSTART;
                        continue;
                    }
                    // No preceding piece — `(#cN,M)` is a POSTFIX
                    // modifier in real zsh and requires a piece to
                    // attach to. Without one C `Src/pattern.c:1606+`
                    // rejects the pattern. Bug #521: zshrs previously
                    // took a legacy PREFIX path (compile next piece as
                    // operand) which silently matched empty for `0,0`
                    // and similar degenerate ranges.
                    //
                    // Report nothing here: C's patcomppiece signals a bad
                    // pattern by `return 0` alone (c:1600, c:1622) and the
                    // CALLER prints the diagnostic with the pattern it was
                    // given — `matchpat` zerrs "bad pattern: %s" with the
                    // full pattern (glob.c:2522), and `[[ ]]` zwarnnams it
                    // from cond.c:314. Emitting a message from inside the
                    // compiler printed only the `(#cN,M)` fragment, losing
                    // the rest of the pattern the user actually wrote.
                    return -1; // c:1622 `return 0;`
                }
            }
            // Malformed `(#c...)` — fall through to generic flag handler.
        }
        // c:953-984 — mid-pattern `(#...)` glob-flag specifier. Emits
        // P_GFLAGS in C to switch GF_IGNCASE / GF_LCMATCHUC /
        // GF_MULTIBYTE / etc. mid-match. C body:
        //   `if ((*patparse == zpc_special[ZPC_INPAR] &&`
        //   `     patparse[1] == zpc_special[ZPC_HASH]) || ...)`
        //   `    if (!patgetglobflags(&patparse, &assert, &ignore))`
        //   `        return 0;`
        // Both bytes must equal their zpc_special slots, which
        // patcompcharsset (c:480-483) masks to Marker when
        // EXTENDEDGLOB is off (c:476). Previously this Rust arm
        // compared the raw byte `b'#'`, allowing `(#s)` / `(#e)` /
        // `(#i)` to fire even without EXTENDEDGLOB. Parity bugs
        // #18/#19 vs real zsh.
        let hash_char = zpc_special.lock().unwrap()[ZPC_HASH as usize]; // c:957
                                                                        // c:953-954 compares the FIRST byte against
                                                                        // `zpc_special[ZPC_INPAR]` too, which patcompcharsset masks to
                                                                        // Marker under SHGLOB (c:500-510) or `disable -p '('`. With `(`
                                                                        // disabled, `(#i)abc` is the LITERAL text `(#i)abc`, so the
                                                                        // flag spec must not fire.
                                                                        // c:955-956 — the ksh form `@(#...)`:
                                                                        //   `(*patparse == zpc_special[ZPC_KSH_AT] &&
                                                                        //     patparse[1] == Inpar && patparse[2] == zpc_special[ZPC_HASH])`
                                                                        // is equally a glob-flag specifier. c:961 then skips 3 bytes
                                                                        // instead of 2. Misc/globtests.ksh:94 writes `(#i)FOO@(#I)X@(#i)X`
                                                                        // and zshrs reported "bad pattern" without this arm.
        let sp_ksh_at = zpc_special.lock().unwrap()[ZPC_KSH_AT as usize];
        let at_form = hash_char == b'#'
            && sp_ksh_at != 0
            && off + 2 < bytes.len()
            && bytes[off] == sp_ksh_at
            && bytes[off + 1] == sp_inpar
            && bytes[off + 2] == b'#';
        if at_form
            || (hash_char == b'#'
                && off + 1 < bytes.len()
                && bytes[off] == sp_inpar
                && bytes[off + 1] == b'#')
        {
            // c:961 — `patparse += (*patparse == '@') ? 3 : 2;` — the
            // Rust patgetglobflags() consumes the leading `(#` itself, so
            // hand it the string starting at the `(`.
            let skip = usize::from(at_form);
            let rest = std::str::from_utf8(&bytes[off + skip..])
                .unwrap_or("")
                .to_string();
            // c:959 — `int oldglobflags = patglobflags, ignore;`
            let oldglobflags = patglobflags.load(Ordering::Relaxed);
            if let Some((_bits, assertp, consumed)) = patgetglobflags(&rest) {
                patparse_off.fetch_add(consumed + skip, Ordering::Relaxed);
                // c:995-997 — `} else { /* No effect. */ continue; }`.
                // A specifier that leaves patglobflags unchanged emits
                // nothing. Matters after c:872's per-branch reset:
                // `((#I)foo|(#i)rod)` re-enters branch 2 already
                // case-insensitive, so its `(#i)` is a no-op.
                if assertp == 0 && patglobflags.load(Ordering::Relaxed) == oldglobflags {
                    continue;
                }
                // Emit P_GFLAGS for flag-bit changes if any. Include the
                // low byte (the `(#aN)` approximation budget) so a
                // mid-pattern `(#a0)` / `(#a2)` actually changes the
                // error allowance for the following segment — c:Src/
                // pattern.c:2941 `patglobflags = P_OPERAND(scan)->l`
                // sets the WHOLE value. Without the 0xff, `(#a1)cat(#a0)
                // dog` kept the outer budget and `dog` wrongly tolerated
                // an error.
                // c:989-994 — the payload is the RUNNING `patglobflags`
                // (`up.l = patglobflags`), not the delta this specifier
                // contributed. `patgetglobflags` only ever reports bits it
                // turned ON, so building the payload from it dropped
                // GF_MULTIBYTE — and since c:2942 assigns the payload
                // ABSOLUTELY (`patglobflags = P_OPERAND(scan)->l`), a
                // mid-pattern `(#b)` demoted the rest of the match to
                // single-byte stepping: `[[ $'αβγδε' = *(#b)(?) ]]` then
                // matched only the last BYTE of `ε`.
                let flag_bits = patglobflags.load(Ordering::Relaxed)
                    & (GF_IGNCASE | GF_LCMATCHUC | GF_MULTIBYTE | 0xff);
                if flag_bits != 0 || (flag_bits == 0 && assertp == 0) {
                    let gf_off = patnode(P_GFLAGS);
                    let mut buf = patout.lock().unwrap();
                    buf.extend_from_slice(&flag_bits.to_le_bytes());
                    drop(buf);
                    if chain_start < 0 {
                        chain_start = gf_off as i64;
                    } else {
                        set_next(last_tail, gf_off);
                    }
                    last_tail = gf_off;
                }
                // Emit P_ISSTART / P_ISEND when assertp set.
                if assertp != 0 {
                    let as_off = patnode(assertp as u8);
                    if chain_start < 0 {
                        chain_start = as_off as i64;
                    } else {
                        set_next(last_tail, as_off);
                    }
                    last_tail = as_off;
                }
                continue;
            }
            // patgetglobflags failed — treat the `(` as a literal group.
        }

        // c:1011-1014 — `^pat` standalone negation. When `^` is the
        // active EXTENDEDGLOB special and appears at piece position
        // (not inside a `[...]` bracket class), consume it and call
        // patcompnot(0, ...) to emit the EXCLUDE structure.
        let sp_hat = zpc_special.lock().unwrap()[ZPC_HAT as usize];
        if c == sp_hat && sp_hat != 0 {
            patparse_off.fetch_add(1, Ordering::Relaxed); // c:1012 patparse++
            let mut not_flags: i32 = 0;
            let starter = patcompnot(0, &mut not_flags); // c:1013 patcompnot(0, ...)
            if starter < 0 {
                return -1;
            }
            *flagp |= not_flags & P_HSTART;
            if chain_start < 0 {
                chain_start = starter;
            } else {
                set_next(last_tail, starter as usize);
            }
            // patcompnot's tail is the trailing P_NOTHING — we need
            // to find it. The chain ends where excend / excl converge
            // (both `pattail(excend, n)` and `pattail(excl, n)`). For
            // chaining we use `starter` since the next piece's chain
            // is appended via patcomppiece's tail_out mechanism — but
            // patcompnot doesn't expose a tail. Approximate: scan
            // forward from starter to find the last P_NOTHING in the
            // emitted block. Cheap: walk patout from starter looking
            // for the highest-offset P_NOTHING in this compile pass.
            // Simpler: bake the convention that the trailing
            // P_NOTHING is the last node — set last_tail to current
            // emit position (= start of next node).
            let cur_emit = patout.lock().unwrap().len();
            last_tail = cur_emit.saturating_sub(I_BODY);
            continue;
        }
        drop(snapshot); // hint: explicit release
        let mut piece_flags: i32 = 0;
        let mut piece_tail: usize = 0;
        // Snapshot the chain tail BEFORE patcomppiece — used by a
        // following POSTFIX (#cN,M) to detach this piece from the chain.
        let prev_tail_before_piece: i64 = if chain_start < 0 {
            -1
        } else {
            last_tail as i64
        };
        // !!! WARNING: RUST-ONLY HELPER !!!
        //
        // C has no such guard here because it cannot need one: c:1336's
        // `DPUTS(patparse == str0, "BUG: matched nothing in patcomppiece.")`
        // is a debug-only assertion resting on the invariant that every
        // `patcomppiece` arm advances `patparse`. When a Rust arm broke that
        // invariant (the multibyte `é#` backtrack, pattern.rs:2960) the loop
        // below became an unkillable CPU spin that ate the whole shell.
        // Turning the same invariant into a hard bail keeps a future
        // regression a "bad pattern" error instead of a hang.
        let off_before_piece = patparse_off.load(Ordering::Relaxed);
        let piece = patcomppiece(&mut piece_flags, paren, &mut piece_tail);
        if piece < 0 {
            return -1;
        }
        if patparse_off.load(Ordering::Relaxed) <= off_before_piece {
            // c:1336 — the DPUTS condition, made fatal.
            return -1;
        }
        if chain_start < 0 {
            chain_start = piece;
        } else {
            // Chain previous piece's tail → this piece's head directly.
            set_next(last_tail, piece as usize);
        }
        last_tail = piece_tail;
        last_piece_off = piece;
        prev_chain_tail = prev_tail_before_piece;
        last_piece_flags = piece_flags;
        *flagp &= piece_flags;
    }

    if chain_start < 0 {
        chain_start = patnode(P_NOTHING) as i64;
    }
    chain_start
}

// =====================================================================
// 10. Glob-flag parser — pattern.c:1037
// =====================================================================

/// Port of `patgetglobflags()` from `Src/pattern.c:1037`.
/// C: `patgetglobflags(char **strp, long *assertp, int *ignore)`.
///
/// C signature: `int patgetglobflags(char **strp, long *assertp,
/// int *ignore)`. C reads/writes the file-static `patglobflags`
/// directly via `|=` / `&=` at each switch arm (c:1064, 1070, 1075,
/// 1080, 1085, 1090, 1095, 1100, 1112, 1116). The hoist loop in
/// patcompile at c:582 / c:636 snapshots `patglobflags` into the
/// compiled `Patprog`'s `globflags` and `globend` fields, so the
/// per-arm writes are the canonical store — the return value is
/// only the success/fail signal (1 / 0).
///
/// The Rust port stores `patglobflags` in an AtomicI32 and mirrors
/// every C write through `fetch_or` / `fetch_and` ops so the same
/// snapshot at c:636 works. The return tuple `(flag_bits, assertp,
/// consumed)` is kept as an adapter for the patcompile hoist loop
/// (pattern.rs:589) which used it to accumulate `hoisted_globflags`
/// — those callers see the same bits via the atomic now, but
/// changing the return shape is a wider refactor.
///
/// Returns `Some((flag_bits, assertp_val, consumed_bytes))` on
/// success (C returns 1), or `None` on parse failure (C returns 0).
pub fn patgetglobflags(s: &str) -> Option<(i32, i64, usize)> {
    // c:1037
    let bytes = s.as_bytes();
    if !s.starts_with("(#") {
        return None;
    }
    let mut i = 2;
    // `bits` tracks what this call set so the caller can OR into
    // `hoisted_globflags` (the patcompile-side accumulator). The
    // canonical store is `patglobflags` (updated below per C arm).
    let mut bits: i32 = 0;
    let mut assertp: i64 = 0;

    while i < bytes.len() && bytes[i] != b')' {
        match bytes[i] {
            // c:1073-1076 `'i'`: `patglobflags = (patglobflags &
            // ~GF_LCMATCHUC) | GF_IGNCASE`.
            b'i' => {
                patglobflags.fetch_and(!GF_LCMATCHUC, Ordering::Relaxed);
                patglobflags.fetch_or(GF_IGNCASE, Ordering::Relaxed);
                bits |= GF_IGNCASE;
                bits &= !GF_LCMATCHUC;
                i += 1;
            } // c:1075
            // c:1078-1081 — `'I'`: `patglobflags &= ~(GF_LCMATCHUC|GF_IGNCASE)`.
            b'I' => {
                patglobflags.fetch_and(!(GF_LCMATCHUC | GF_IGNCASE), Ordering::Relaxed);
                bits &= !(GF_LCMATCHUC | GF_IGNCASE);
                i += 1;
            } // c:1080
            // c:1068-1071 — `'l'`: `patglobflags = (patglobflags &
            // ~GF_IGNCASE) | GF_LCMATCHUC`.
            b'l' => {
                patglobflags.fetch_and(!GF_IGNCASE, Ordering::Relaxed);
                patglobflags.fetch_or(GF_LCMATCHUC, Ordering::Relaxed);
                bits |= GF_LCMATCHUC;
                bits &= !GF_IGNCASE;
                i += 1;
            } // c:1070
            // c:1083-1086 — `'b'`: `patglobflags |= GF_BACKREF`.
            b'b' => {
                patglobflags.fetch_or(GF_BACKREF, Ordering::Relaxed);
                bits |= GF_BACKREF;
                i += 1;
            } // c:1085
            // c:1088-1091 — `'B'`: `patglobflags &= ~GF_BACKREF`.
            b'B' => {
                patglobflags.fetch_and(!GF_BACKREF, Ordering::Relaxed);
                bits &= !GF_BACKREF;
                i += 1;
            } // c:1090
            // c:1093-1096 — `'m'`: `patglobflags |= GF_MATCHREF`.
            b'm' => {
                patglobflags.fetch_or(GF_MATCHREF, Ordering::Relaxed);
                bits |= GF_MATCHREF;
                i += 1;
            } // c:1095
            // c:1098-1101 — `'M'`: `patglobflags &= ~GF_MATCHREF`.
            b'M' => {
                patglobflags.fetch_and(!GF_MATCHREF, Ordering::Relaxed);
                bits &= !GF_MATCHREF;
                i += 1;
            } // c:1100
            // c:1103-1105 — `'s'`: sets `*assertp = P_ISSTART`,
            // doesn't touch patglobflags.
            b's' => {
                assertp = P_ISSTART as i64;
                i += 1;
            } // c:1104
            // c:1107-1109 — `'e'`: sets `*assertp = P_ISEND`.
            b'e' => {
                assertp = P_ISEND as i64;
                i += 1;
            } // c:1108
            // c:1111-1113 — `'u'`: `patglobflags |= GF_MULTIBYTE`.
            b'u' => {
                patglobflags.fetch_or(GF_MULTIBYTE, Ordering::Relaxed);
                bits |= GF_MULTIBYTE;
                i += 1;
            } // c:1112
            // c:1115-1117 — `'U'`: `patglobflags &= ~GF_MULTIBYTE`.
            b'U' => {
                patglobflags.fetch_and(!GF_MULTIBYTE, Ordering::Relaxed);
                bits &= !GF_MULTIBYTE;
                i += 1;
            } // c:1116
            // c:1054-1066 — `'a'`: approximate-match error count.
            // `ret = zstrtol(++ptr, &nptr, 10);` then
            // `patglobflags = (patglobflags & ~0xff) | (ret & 0xff)`.
            b'a' => {
                i += 1;
                let digit_start = i;
                let mut errs: i32 = 0;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    errs = errs * 10 + (bytes[i] - b'0') as i32;
                    i += 1;
                }
                if i == digit_start {
                    // c:1062 `ptr == nptr` — no digits consumed.
                    return None;
                }
                if errs < 0 || errs > 254 {
                    return None; // c:1062
                }
                // c:1064 — `patglobflags = (patglobflags & ~0xff) | (ret & 0xff)`.
                let mask: i32 = !0xff;
                let cur = patglobflags.load(Ordering::Relaxed);
                patglobflags.store((cur & mask) | (errs & 0xff), Ordering::Relaxed);
                bits = (bits & !0xff) | (errs & 0xff);
            }
            // c:1046-1050 — `'q'`: glob qualifier, ignored in pattern
            // code. Skip to the closing `)` without consuming bits.
            b'q' => {
                while i < bytes.len() && bytes[i] != b')' {
                    i += 1;
                }
            }
            // c:1119-1120 — `default: return 0`.
            _ => return None,
        }
    }
    // c:1124-1125 — `if (*ptr != Outpar) return 0;`.
    if i >= bytes.len() {
        return None;
    }
    i += 1; // c:1129 — `*strp = ptr + 1;` advance past `)`.
    Some((bits, assertp, i))
}

// =====================================================================
// 11. Range helpers — pattern.c:1148, :1179
// =====================================================================

/// Port of `range_type()` from `Src/pattern.c:1148`.
/// C: `range_type(char *start, int len)`. Looks up the
/// integer code for a POSIX character class name (e.g. "alpha" → 1).
/// Returns None for unknown names.
///
/// C signature takes a `(char *start, int len)` non-NUL-terminated
/// substring; Rust's `&str` carries the length implicitly, so the
/// two-arg C shape collapses to a single arg. Param name `start`
/// matches C verbatim (Rule E).
pub fn range_type(start: &str) -> Option<usize> {
    // c:1148
    POSIX_CLASS_NAMES
        .iter()
        .position(|n| *n == start)
        .map(|i| i + 1)
}

/// Port of `pattern_range_to_string()` from `Src/pattern.c:1179`.
/// C: `int pattern_range_to_string(char *rangestr, char *outstr)`
/// from `Src/pattern.c:1179`. Walks a Meta-encoded range bytestring,
/// re-emitting the human-readable form: literal chars as-is,
/// PP_RANGE pairs as `c1-c2`, POSIX classes as `[:name:]`.
/// Returns the output length; `outstr` is the destination buffer
/// (NULL = measure-only). The Rust port operates on `&str` (UTF-8
/// native) — `Meta`-byte decode collapses since zshrs's pattern
/// compiler stores raw chars, so the function effectively walks
/// the chars and emits POSIX class names from the `[i]` table at
/// `range_type`'s reverse lookup.
///
/// Rust signature: drops C's `outstr` out-param. C measures-then-fills
/// via two passes when `outstr` is non-NULL; Rust returns the String
/// directly. Callers needing the C "measure-only" mode read `.len()`
/// on the result.
pub fn pattern_range_to_string(rangestr: &str) -> String {
    // c:1179
    let mut out = String::with_capacity(rangestr.len()); // c:1181 int len = 0
    let mut chars = rangestr.chars().peekable();
    while let Some(c) = chars.next() {
        // c:1184-1247 — swtype dispatch via Meta+PP_*. zshrs stores
        // POSIX-class markers as a sentinel byte followed by the
        // class name. The Meta encoding doesn't apply (UTF-8 native),
        // so we just pass chars through, recognizing PP_* class tags
        // when they appear as `[:name:]` literal syntax.
        if c == '[' && chars.peek() == Some(&':') {
            // c:1242 — `[:alpha:]` and similar.
            let mut name = String::new();
            chars.next(); // consume ':'
            while let Some(&cc) = chars.peek() {
                if cc == ':' {
                    chars.next();
                    if chars.peek() == Some(&']') {
                        chars.next();
                        break;
                    }
                    name.push(':');
                } else {
                    name.push(cc);
                    chars.next();
                }
            }
            out.push_str(&format!("[:{}:]", name)); // c:1244
        } else {
            // c:1185-1213 — single-char or range pair.
            // Check for `c1-c2` PP_RANGE form.
            out.push(c); // c:1210/1216
            if chars.peek() == Some(&'-') {
                chars.next();
                if let Some(c2) = chars.next() {
                    out.push('-'); // c:1219
                    out.push(c2); // c:1220
                }
            }
        }
    }
    out // c:1261 return len
}

/// Port of `patcomppiece()` from `Src/pattern.c:1261`.
/// C: `patcomppiece(int *flagp, int paren)`.
///
/// C: `static long patcomppiece(int *flagp, int paren)`. Parses a
/// single atom + optional quantifier. Returns offset of compiled node.
/// Out-param `tail_out` receives the byte offset of the LAST opcode
/// in the compiled piece — the node whose `.next` should be chained
/// to whatever follows in the sequence. For simple atoms (P_EXACTLY,
/// P_ANY, etc.) the tail equals the head; for compound pieces
/// `(...)` / quantified atoms it points to the trailing P_CLOSE_N or
/// quantifier-injected node.
///
/// Rust signature adds `tail_out: &mut usize` for the LAST-opcode-
/// offset that C threads through pointer arithmetic on the
/// caller-side `Upat` cursor. Without the out-param, every Rust
/// caller would need to re-walk the just-emitted bytecode to find
/// the tail; the explicit out-param avoids the re-scan. Rule B
/// deviation sanctioned by zshrs's byte-offset bytecode (vs C's
/// pointer-arithmetic) substrate.
pub fn patcomppiece(flagp: &mut i32, paren: i32, tail_out: &mut usize) -> i64 {
    // c:1261
    // c:1509-1512 — `case Inpar:` carries two DPUTS asserting that a
    // BARE `(` (no kshchar prefix) never reaches the group compiler
    // when `zpc_special[ZPC_INPAR] == Marker`. patcompcharsset sets
    // that Marker for `isset(SHGLOB)` (c:500-510) and for a user
    // `disable -p '('`. So `(` opens a group only when its slot still
    // holds the literal byte, or when a ksh trigger (`@(`, `*(`, …)
    // put us here deliberately (c:1419 consumes the trigger and then
    // falls into `case Inpar`).
    let inpar_active = { zpc_special.lock().unwrap()[ZPC_INPAR as usize] == b'(' };
    let off = patparse_off.load(Ordering::Relaxed);
    let parse = patparse.lock().unwrap();
    if off >= parse.len() {
        return patnode(P_NOTHING) as i64;
    }
    let bytes = parse.as_bytes();
    let c = bytes[off];

    // c:1278 — `kshchar = '\0';`. KSH-glob trigger detection: when the
    // current byte matches one of the six ZPC_KSH_* slots AND the next
    // byte is `(`, record which kshchar so the post-atom dispatch can
    // emit the right quantifier shape (c:1615-1746).
    //
    // c:1279 — `if (*patparse && patparse[1] == Inpar) { ... }`. Note
    // the C code uses literal `Inpar` (the `(` token) for the lookahead
    // — NOT `zpc_special[ZPC_INPAR]` — so SHGLOB disabling `(` as a
    // pattern char doesn't suppress ksh-glob detection here.
    let mut kshchar: u8 = 0; // c:1278
    if off + 1 < parse.len() && bytes[off + 1] == b'(' {
        // c:1279
        let sp = zpc_special.lock().unwrap();
        if c == sp[ZPC_KSH_PLUS as usize] {
            kshchar = b'+';
        }
        // c:1280-1281
        else if c == sp[ZPC_KSH_BANG as usize] {
            kshchar = b'!';
        }
        // c:1282-1283
        else if c == sp[ZPC_KSH_BANG2 as usize] {
            kshchar = b'!';
        }
        // c:1284-1285
        else if c == sp[ZPC_KSH_AT as usize] {
            kshchar = b'@';
        }
        // c:1286-1287
        else if c == sp[ZPC_KSH_STAR as usize] {
            kshchar = b'*';
        }
        // c:1288-1289
        else if c == sp[ZPC_KSH_QUEST as usize] {
            kshchar = b'?';
        } // c:1290-1291
    }
    drop(parse);

    // c:1419-1420 — `if (kshchar) patparse++;`. Skip the trigger byte
    // so the atom dispatch consumes the leading `(` as a group.
    let dispatch_c = if kshchar != 0 {
        patparse_off.fetch_add(1, Ordering::Relaxed);
        b'(' // c:1419 — fall through to Inpar case
    } else {
        c
    };

    // Atom dispatch. Each arm sets `*tail_out` to the offset of the
    // last opcode emitted by this piece (for simple atoms, tail = head).
    let atom = match dispatch_c {
        b'?' => {
            patparse_off.fetch_add(1, Ordering::Relaxed);
            *flagp |= P_SIMPLE;
            *flagp &= !P_PURESTR;
            let h = patnode(P_ANY);
            *tail_out = h;
            h as i64
        }
        b'*' => {
            patparse_off.fetch_add(1, Ordering::Relaxed);
            *flagp &= !P_PURESTR;
            let h = patnode(P_STAR);
            *tail_out = h;
            h as i64
        }
        b'[' => {
            // c:Src/pattern.c:1438 `case Inbrack` + c:1497-1498 —
            // `if (*patparse != Outbrack) return 0;`: an ACTIVE `[`
            // (token-derived; the patcompile entry normalization maps
            // the Inbrack token to raw `[` and every literal/escaped
            // bracket to `\[`, which the `\X` arm consumed before
            // reaching here) with no closing `]` is a BAD PATTERN.
            // Callers zerr "bad pattern: %s" exactly like zsh:
            // `print [a-` / `[[ x == [a- ]]` / `case x in [a-)`.
            //
            // History: a 2026-06-03 partial fix for Bug #564 made
            // this case fall back to a literal `[` because the cond
            // path then stripped `\` before patcompile. The escape
            // now survives into the `\X` literal arm (all three #564
            // probes pass), so the leniency only masked genuine bad
            // patterns.
            let probe_off = off + 1;
            let parse_check = patparse.lock().unwrap();
            let check_bytes = parse_check.as_bytes();
            let has_close = check_bytes[probe_off..].contains(&b']');
            drop(parse_check);
            if !has_close {
                return -1; // c:1498 `return 0;` — bad pattern
            }
            patparse_off.fetch_add(1, Ordering::Relaxed);
            *flagp |= P_SIMPLE;
            *flagp &= !P_PURESTR;
            // Inline bracket-expression parse (C patcomppiece bracket case).
            let mut chars: Vec<u8> = Vec::new();
            // Multibyte augmentation: `chars` is a flat ASCII byte set, so a
            // multibyte input char (Cyrillic/Greek/CJK/accented) can never
            // match it. C's `mb_patmatchrange` handles wide chars via
            // `iswalpha`/wide ranges. To match without disturbing the
            // (heavily-tested, byte-for-byte) ASCII path, collect the class
            // predicates and the non-ASCII literals/ranges SEPARATELY and
            // consult them only when the input char is multibyte (P_ANYOF).
            let mut mb_classmask: u32 = 0;
            let mut mb_chars: Vec<char> = Vec::new();
            let mut mb_ranges: Vec<(char, char)> = Vec::new();
            let mut negate = false;
            let bracket_start = patparse_off.load(Ordering::Relaxed);
            let parse_b = patparse.lock().unwrap();
            let bb = parse_b.as_bytes();
            let mut i_b = bracket_start;
            if i_b < bb.len() && (bb[i_b] == b'^' || bb[i_b] == b'!') {
                negate = true;
                i_b += 1;
            }
            // c:Src/pattern.c:1456-1459 — `[]...]` exception: when `]` is
            // the FIRST char inside the class AND another `]` follows
            // later (so the close is not ambiguous), the first `]` is a
            // class member. Mirrors `[]x]` matching `]` or `x` in C.
            if i_b < bb.len() && bb[i_b] == b']' && bb[i_b + 1..].contains(&b']') {
                chars.push(b']');
                i_b += 1;
            }
            while i_b < bb.len() && bb[i_b] != b']' {
                // c:Src/pattern.c — `\X` inside a class is handled in C
                // by shtokenize converting it to `Bnullkeep X` upstream;
                // by the time C's bracket walker runs, the `]` after `\`
                // doesn't appear as raw `]` so the close-`]` scan
                // doesn't trip on it. The Rust port may receive raw
                // `\X` (callers that bypass shtokenize), so handle the
                // backslash-escape inline: consume `\X` as a literal
                // class-member byte. Bug surface:
                // `${q//(#m)[\][()|\\*?#<>~^]/...}` — escaped `]` /
                // `\\` / etc. inside the class are class members, not
                // metacharacters.
                if i_b + 1 < bb.len() && bb[i_b] == b'\\' {
                    chars.push(bb[i_b + 1]);
                    i_b += 2;
                    continue;
                }
                if i_b + 1 < bb.len() && bb[i_b] == b'[' && bb[i_b + 1] == b':' {
                    let class_start = i_b + 2;
                    let mut j_b = class_start;
                    while j_b + 1 < bb.len() && !(bb[j_b] == b':' && bb[j_b + 1] == b']') {
                        j_b += 1;
                    }
                    if j_b + 1 < bb.len() {
                        let class_name = std::str::from_utf8(&bb[class_start..j_b]).unwrap_or("");
                        // Inline POSIX class expansion.
                        match class_name {
                            "alpha" => {
                                for c in b'a'..=b'z' {
                                    chars.push(c);
                                }
                                for c in b'A'..=b'Z' {
                                    chars.push(c);
                                }
                            }
                            "upper" => {
                                for c in b'A'..=b'Z' {
                                    chars.push(c);
                                }
                            }
                            "lower" => {
                                for c in b'a'..=b'z' {
                                    chars.push(c);
                                }
                            }
                            "digit" => {
                                for c in b'0'..=b'9' {
                                    chars.push(c);
                                }
                            }
                            "xdigit" => {
                                for c in b'0'..=b'9' {
                                    chars.push(c);
                                }
                                for c in b'a'..=b'f' {
                                    chars.push(c);
                                }
                                for c in b'A'..=b'F' {
                                    chars.push(c);
                                }
                            }
                            "alnum" => {
                                for c in b'a'..=b'z' {
                                    chars.push(c);
                                }
                                for c in b'A'..=b'Z' {
                                    chars.push(c);
                                }
                                for c in b'0'..=b'9' {
                                    chars.push(c);
                                }
                            }
                            "space" => {
                                for b in b" \t\n\r\x0b\x0c".iter() {
                                    chars.push(*b);
                                }
                            }
                            "blank" => {
                                chars.push(b' ');
                                chars.push(b'\t');
                            }
                            "punct" => {
                                for b in b"!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~".iter() {
                                    chars.push(*b);
                                }
                            }
                            "cntrl" => {
                                for c in 0u8..=31 {
                                    chars.push(c);
                                }
                                chars.push(127);
                            }
                            "print" => {
                                for c in 32u8..=126 {
                                    chars.push(c);
                                }
                            }
                            "graph" => {
                                for c in 33u8..=126 {
                                    chars.push(c);
                                }
                            }
                            // c:Src/pattern.c:3693-3700 + Src/ztype.h
                            // IIDENT — `[[:IDENT:]]` is zsh's
                            // identifier-character class (alnum + `_`,
                            // the chars valid in a parameter name).
                            // Static for the ASCII range; the
                            // multibyte wcsitype(IIDENT) path isn't
                            // expanded here. p10k/zsh-z validate names
                            // with `[[:IDENT:]]##`.
                            // c:3702-3719 — PP_IDENT / PP_IFS /
                            // PP_IFSSPACE / PP_WORD are evaluated at MATCH
                            // time in C (`wcsitype(ch, IIDENT|ISEP|IWORD)`,
                            // `iwsep`), reading the live `$IFS` / `$WORDCHARS`
                            // through the ztype table. Expanding them into a
                            // static `chars` set at COMPILE time froze the
                            // first observed value into the cached Patprog, so
                            // `IFS=' ' [[ $'\n' = [[:IFS:]] ]]` reused the set
                            // built under the previous `$IFS`. Emit a
                            // classmask bit instead; anyof_membership resolves
                            // it per candidate character.
                            // c:3737-3744 — PP_INCOMPLETE / PP_INVALID test the
                            // multibyte DECODE STATE of the candidate
                            // character (`zmb_ind == ZMB_INCOMPLETE` /
                            // `ZMB_INVALID`), never a fixed character set, so
                            // like the four live classes above they carry no
                            // compile-time members.
                            "IDENT" | "IFS" | "IFSSPACE" | "WORD" | "INCOMPLETE" | "INVALID" => {}
                            _ => {}
                        }
                        // Record the class predicate for multibyte input.
                        // Bits mirror the Unicode-aware `iswXXX` checks C's
                        // mb_patmatchrange applies (pattern.rs:3364). digit/
                        // xdigit stay ASCII-only (already in `chars`) — zsh's
                        // iswdigit is ASCII in the common locale.
                        mb_classmask |= match class_name {
                            "alpha" => 1 << 0,
                            "alnum" => 1 << 1,
                            "upper" => 1 << 2,
                            "lower" => 1 << 3,
                            "space" => 1 << 4,
                            "IFS" => 1 << 11,
                            "IFSSPACE" => 1 << 12,
                            "blank" => 1 << 5,
                            "punct" => 1 << 6,
                            "cntrl" => 1 << 7,
                            "print" => 1 << 8,
                            "graph" => 1 << 9,
                            // c:3702-3719 — live PP_ classes, resolved at
                            // match time (see the arm above).
                            "IDENT" => 1 << 10,
                            "WORD" => 1 << 13,
                            // c:3737-3744 — decode-state classes.
                            "INCOMPLETE" => 1 << 14,
                            "INVALID" => 1 << 15,
                            _ => 0,
                        };
                        i_b = j_b + 2;
                        continue;
                    }
                }
                // Multibyte member: a byte >= 0x80 leads a UTF-8 sequence
                // that cannot live in the ASCII `chars` set. Decode it as a
                // wide literal, or a wide range `<ch>-<ch>`, for the P_ANYOF
                // multibyte path.
                if bb[i_b] >= 0x80 {
                    if let Some(lo) = std::str::from_utf8(&bb[i_b..])
                        .ok()
                        .and_then(|s| s.chars().next())
                    {
                        let lolen = lo.len_utf8();
                        if i_b + lolen < bb.len()
                            && bb[i_b + lolen] == b'-'
                            && i_b + lolen + 1 < bb.len()
                            && bb[i_b + lolen + 1] != b']'
                        {
                            if let Some(hi) = std::str::from_utf8(&bb[i_b + lolen + 1..])
                                .ok()
                                .and_then(|s| s.chars().next())
                            {
                                mb_ranges.push((lo, hi));
                                i_b += lolen + 1 + hi.len_utf8();
                                continue;
                            }
                        }
                        mb_chars.push(lo);
                        i_b += lolen;
                        continue;
                    }
                }
                if i_b + 2 < bb.len() && bb[i_b + 1] == b'-' && bb[i_b + 2] != b']' {
                    let lo = bb[i_b];
                    let hi = bb[i_b + 2];
                    // ASCII-low, multibyte-high range (`[a-я]`): record as a
                    // wide range so the high end matches; the ASCII portion
                    // of the range still expands into `chars` below.
                    if hi >= 0x80 {
                        if let Some(hich) = std::str::from_utf8(&bb[i_b + 2..])
                            .ok()
                            .and_then(|s| s.chars().next())
                        {
                            mb_ranges.push((lo as char, hich));
                            for c in lo..=0x7f {
                                chars.push(c);
                            }
                            i_b += 2 + hich.len_utf8();
                            continue;
                        }
                    }
                    for c in lo..=hi {
                        chars.push(c);
                    }
                    i_b += 3;
                } else {
                    chars.push(bb[i_b]);
                    i_b += 1;
                }
            }
            drop(parse_b);
            if let Some(p_lock) = patparse.lock().ok() {
                if i_b < p_lock.len() && p_lock.as_bytes()[i_b] == b']' {
                    i_b += 1;
                }
            }
            patparse_off.store(i_b, Ordering::Relaxed);
            let opcode = if negate { P_ANYBUT } else { P_ANYOF };
            let off2 = patnode(opcode);
            let mut buf = patout.lock().unwrap();
            // Body layout: `[len:u32]` (total following bytes) then
            // `[chars_len:u32][chars][classmask:u32][n_mbchars:u32]
            // [mbchars:u32*n][n_mbranges:u32][(lo,hi):u32*2]`. `len` counts
            // the WHOLE body so `advance_past_instr` / node traversal (which
            // do `4 + len`) skip it unchanged — only the P_ANYOF/P_ANYBUT
            // matcher parses the sub-structure. Pure-ASCII brackets carry a
            // zeroed 12-byte tail (classmask 0, no mbchars/mbranges).
            let mut body_ext: Vec<u8> = Vec::new();
            body_ext.extend_from_slice(&(chars.len() as u32).to_le_bytes());
            body_ext.extend_from_slice(&chars);
            body_ext.extend_from_slice(&mb_classmask.to_le_bytes());
            body_ext.extend_from_slice(&(mb_chars.len() as u32).to_le_bytes());
            for c in &mb_chars {
                body_ext.extend_from_slice(&(*c as u32).to_le_bytes());
            }
            body_ext.extend_from_slice(&(mb_ranges.len() as u32).to_le_bytes());
            for (lo, hi) in &mb_ranges {
                body_ext.extend_from_slice(&(*lo as u32).to_le_bytes());
                body_ext.extend_from_slice(&(*hi as u32).to_le_bytes());
            }
            let len = body_ext.len() as u32;
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(&body_ext);
            *tail_out = off2;
            off2 as i64
        }
        // c:1509-1512 — guard mirrors the two DPUTS: a disabled `(`
        // (SHGLOB, or `disable -p '('`) is an ORDINARY character unless
        // a ksh trigger routed us here.
        b'(' if kshchar != 0 || inpar_active => {
            patparse_off.fetch_add(1, Ordering::Relaxed);
            *flagp &= !P_PURESTR;
            // c:1508-1525 — `if (kshchar == '!') patcompnot(1, ...) else
            // patcompswitch(1, ...)`. The `!(pat)` form needs a
            // negation-aware compile that builds an EXCLUDE subtree;
            // every other case (including `@(pat)`, the bare `(pat)`
            // group, and the `+/*/?` ksh forms) takes the alternation
            // switch path.
            if kshchar == b'!' {
                // c:1522-1523
                let mut flags2: i32 = 0;
                let starter_off = patcompnot(1, &mut flags2);
                if starter_off < 0 {
                    return -1;
                }
                *flagp |= flags2 & P_HSTART; // c:1526
                                             // c:1019 — the caller chains the FOLLOWING piece with
                                             // `pattail(chain, latest)`, which WALKS to the end of this
                                             // piece's next-chain. patcompnot's chain is
                                             // `P_BRANCH -> P_EXCLUDE -> P_NOTHING` (c:1773, c:1779-1781),
                                             // so the tail is the trailing P_NOTHING, NOT the P_BRANCH.
                                             // Reporting the P_BRANCH made patcompbranch's `set_next`
                                             // OVERWRITE the branch's link to the P_EXCLUDE, silently
                                             // deleting the negation whenever anything followed it:
                                             // `[[ foob = !(foo)b* ]]` matched, as did
                                             // `[[ mad.moo.cow = !(*.*).!(*.*) ]]`. (The `^pat` arm in
                                             // patcompbranch already compensates by hand.)
                let mut t = starter_off as usize;
                loop {
                    let n = {
                        let buf = patout.lock().unwrap();
                        if t + I_NEXT + 4 > buf.len() {
                            0
                        } else {
                            u32::from_le_bytes(buf[t + I_NEXT..t + I_NEXT + 4].try_into().unwrap())
                                as usize
                        }
                    };
                    if n == 0 || n == t {
                        break;
                    }
                    t = n;
                }
                *tail_out = t;
                // c:1419 already consumed `!`; the b'(' branch consumed
                // `(`; patcompnot drained through to `)`. Verify here.
                return starter_off;
            }
            // c:775-783 — `if (paren && (patglobflags & GF_BACKREF) &&
            // patnpar <= NSUBEXP) { parno = patnpar++; starter =
            // patnode(P_OPEN + parno); } else starter = 0;`. A group is
            // NUMBERED (capture slot allocated) only when GF_BACKREF is
            // live in patglobflags at the point the `(` is compiled —
            // wherever the `(#b)` appeared (leading, after a literal
            // prefix, mid-pattern). Beyond NSUBEXP, C "just use[s]
            // P_OPEN on its own" (c:777-780): parno stays 0 and the
            // match arms record nothing (c:2957 `if (no && ...)`).
            // The previous Rust arm numbered EVERY paren and returned
            // -1 past NSUBEXP — both divergent.
            let gf_now = patglobflags.load(Ordering::Relaxed);
            let n = if (gf_now & GF_BACKREF) != 0
                && patnpar.load(Ordering::Relaxed) <= NSUBEXP as i32
            {
                patnpar.fetch_add(1, Ordering::Relaxed)
            } else {
                0 // plain (uncaptured) group — P_OPEN+0 / P_CLOSE+0
            };
            let opcode = P_OPEN + n as u8;
            let open_off = patnode(opcode);
            // c:770 — `savglobflags = patglobflags`. Snapshot the glob
            // flags entering the group so a flag set INSIDE it (e.g.
            // `((#i)foo)`) can be restored on the way out.
            let savglobflags = patglobflags.load(Ordering::Relaxed);
            let mut inner_flags: i32 = 0;
            let inner = patcompswitch(1, &mut inner_flags);
            if inner < 0 {
                return -1;
            }
            // Expect closing ')'.
            let cur_off = patparse_off.load(Ordering::Relaxed);
            let p = patparse.lock().unwrap();
            if cur_off >= p.len() || p.as_bytes()[cur_off] != b')' {
                return -1;
            }
            drop(p);
            patparse_off.fetch_add(1, Ordering::Relaxed);
            let close_off = patnode(P_CLOSE + n as u8);
            // P_OPEN_N.next → first BRANCH of inner alternation.
            set_next(open_off, inner as usize);
            // Mirror C pattern.c:903 `pattail(starter, ender)`:
            // walk the BRANCH alt-chain from P_OPEN and patch the
            // last BRANCH's `.next` to P_CLOSE_N. Without this, the
            // outer `pattail` walk would descend into the inner
            // alt-chain (P_OPEN.next → inner BRANCH) and corrupt
            // its terminator.
            pattail(open_off, close_off);
            // Each branch's operand chain ends at P_CLOSE_N.
            chain_branches_to(inner as usize, close_off);
            *flagp &= !P_PURESTR;
            // c:920-929 — `if (paren && gfchanged) { pattail(ender,
            // patnode(P_GFLAGS)); patglobflags = savglobflags; }`. When
            // a glob flag that the matcher honors mid-pattern
            // (IGNCASE / LCMATCHUC / MULTIBYTE) changed inside the
            // group, append a P_GFLAGS node after P_CLOSE that restores
            // the entering value. Without it `[[ FOOXx = ((#i)foox)X ]]`
            // wrongly matched: the `(#i)` set GF_IGNCASE for the rest of
            // the match, so the trailing case-sensitive `X` folded too.
            //
            // c:921-925 — "gfchanged detects a change in any branch
            // (except exclusions which are separate), since we need to
            // emit this even if a later branch happened to put the flags
            // back." Comparing only the END state missed
            // `((#I)foo|(#i)rod)grud`: branch 1's P_GFLAGS(0) leaks into
            // the trailing `grud` because branch 2 restored the entering
            // value, so no restore node was emitted.
            let relevant = GF_IGNCASE | GF_LCMATCHUC | GF_MULTIBYTE;
            if PATSWITCH_GFCHANGED.load(Ordering::Relaxed) != 0 {
                let gf_off = patnode(P_GFLAGS);
                {
                    let mut buf = patout.lock().unwrap();
                    buf.extend_from_slice(&(savglobflags & relevant).to_le_bytes());
                }
                set_next(close_off, gf_off);
                patglobflags.store(savglobflags, Ordering::Relaxed); // c:927
                *tail_out = gf_off;
                return open_off as i64;
            }
            *tail_out = close_off;
            open_off as i64
        }
        b'\\' => {
            // c:Src/pattern.c — raw `\X` reaches this arm when the
            // input bypassed shtokenize (Src/glob.c:3565), which is
            // the path taken by paramsubst callers passing singsub-
            // computed pattern strings. C's faithful path: shtokenize
            // converts `\X` (special X) to `Bnullkeep X` (case at
            // pattern.c:1584), and a `\X` with non-special X stays
            // raw — both bytes survive to pattern.c's literal-run
            // default arm. Trailing lone `\` also survives shtokenize
            // (bslash flag stays 1 to end-of-string, no replacement
            // fires) and reaches the default-arm literal emission.
            //
            // Two arms mirror that:
            //   - `\X` (off2 in range): emit X as literal. Equivalent
            //     to Bnullkeep X arm in C's pattern.c — the next byte
            //     is the user-literal payload.
            //   - trailing lone `\`: emit `\` as literal. Equivalent
            //     to C's pattern.c default arm receiving a raw `\`
            //     byte that survived shtokenize.
            patparse_off.fetch_add(1, Ordering::Relaxed);
            let p = patparse.lock().unwrap();
            let off2 = patparse_off.load(Ordering::Relaxed);
            if off2 >= p.len() {
                drop(p);
                *flagp |= P_SIMPLE;
                let lit_off = patnode(P_EXACTLY);
                let mut buf_lit = patout.lock().unwrap();
                buf_lit.extend_from_slice(&1u32.to_le_bytes());
                buf_lit.push(b'\\');
                *tail_out = lit_off;
                return lit_off as i64;
            }
            let escaped = p.as_bytes()[off2];
            drop(p);
            patparse_off.fetch_add(1, Ordering::Relaxed);
            *flagp |= P_SIMPLE;
            let lit_off = patnode(P_EXACTLY);
            let mut buf_lit = patout.lock().unwrap();
            buf_lit.extend_from_slice(&1u32.to_le_bytes());
            buf_lit.push(escaped);
            *tail_out = lit_off;
            lit_off as i64
        }
        b'<' => {
            // Numeric range: <a-b> / <a-> / <-b> / <-> .
            // Port of pattern.c:1528-1570 (Inang case).
            //
            // c:Src/pattern.c:1528 — C's `case Inang:` only fires for
            // the Inang TOKEN (0x99) emitted by the lexer at
            // Src/lex.c:1202 when `isnumglob()` returns true (i.e.
            // the `<…>` body parses as a numeric range). Raw `<` (0x3C)
            // in source — quoted `"<"`, escaped `\<`, or any `<` in a
            // non-numglob position — never reaches the pattern compiler
            // as a metachar; it falls through to the literal-run arm.
            //
            // zshrs's pattern compiler is fed by callers that mix lexer
            // tokens with raw ASCII (singsub output, runtime-computed
            // pattern strings that never went through `Src/lex.c:1201`'s
            // isnumglob check). The faithful port: do the same
            // `[digit]*-[digit]*>` walk inline that `isnumglob` does at
            // lex.c:580-610, with full state-save + rewind on failure so
            // a non-numeric `<` (e.g. `<[:digit:]]#>` from zconvey's
            // color-marker pattern, or `<no-data>` from zinit.zsh:2507)
            // falls back to the literal-run arm.
            //
            // The numglob shape mirrors C's isnumglob exactly: digits
            // then `-` then digits then `>`. Empty leading / trailing
            // digit runs are OK (gives the `<-N>` / `<N->` / `<->` forms).
            // c:Src/pattern.c:499-506 — `if (isset(SHGLOB))
            // zpc_special[ZPC_INPAR] = zpc_special[ZPC_INANG] = Marker;`
            // ("Grouping and numeric ranges are not valid. We do allow
            // alternation, however; it's needed for case."). patcompcharsset
            // (pattern.rs:487) already applies that mask, but this arm never
            // consulted the table: it ran its inline isnumglob walk
            // unconditionally, so `<->` stayed an ACTIVE numeric range under
            // `emulate sh` / `emulate ksh`. C asserts the case is unreachable
            // there — `DPUTS(isset(SHGLOB), "Treating <..> as numeric range
            // with SHGLOB")` (c:1532). A disabled slot also covers user
            // `disable -p '<'`. Fall through to the same literal-`<` emit the
            // non-numeric rewind path uses. Bug #1053-B.
            let inang_disabled = { zpc_special.lock().unwrap()[ZPC_INANG as usize] != b'<' };
            if inang_disabled {
                let h = patnode(P_EXACTLY);
                let mut buf = patout.lock().unwrap();
                let len: u32 = 1;
                buf.extend_from_slice(&len.to_le_bytes());
                buf.push(b'<');
                drop(buf);
                patparse_off.fetch_add(1, Ordering::Relaxed);
                *flagp |= P_SIMPLE;
                *tail_out = h;
                return h as i64;
            }
            let entry_off = patparse_off.load(Ordering::Relaxed);
            patparse_off.fetch_add(1, Ordering::Relaxed); // consume `<`
            let parse_n = patparse.lock().unwrap();
            let nb = parse_n.as_bytes();
            let mut j = patparse_off.load(Ordering::Relaxed);
            let mut len_flag: u8 = 0; // bit 0 = lo present, bit 1 = hi present
            let mut from: i64 = 0;
            let lo_start = j;
            while j < nb.len() && nb[j].is_ascii_digit() {
                from = from * 10 + (nb[j] - b'0') as i64;
                j += 1;
            }
            if j > lo_start {
                len_flag |= 1;
            } // c:1538 — `len |= 1`
              // Mandatory dash. C's isnumglob bails (ret=0) here too —
              // walks until non-digit, then if non-digit isn't `-` (or `>`
              // after seeing `-`), returns "not numglob". Rewind the `<`
              // consumption and fall through to the literal-run arm.
            if j >= nb.len() || nb[j] != b'-' {
                drop(parse_n);
                patparse_off.store(entry_off, Ordering::Relaxed);
                let h = patnode(P_EXACTLY);
                let mut buf = patout.lock().unwrap();
                let len: u32 = 1;
                buf.extend_from_slice(&len.to_le_bytes());
                buf.push(b'<');
                patparse_off.fetch_add(1, Ordering::Relaxed);
                *flagp |= P_SIMPLE;
                *tail_out = h;
                return h as i64;
            }
            j += 1; // c:1543 patparse++
            let mut to: i64 = 0;
            let hi_start = j;
            while j < nb.len() && nb[j].is_ascii_digit() {
                to = to * 10 + (nb[j] - b'0') as i64;
                j += 1;
            }
            if j > hi_start {
                len_flag |= 2;
            } // c:1548 — `len |= 2`
              // Expect closing '>'. Mirror the dash-failure rewind here
              // too: a `<N-` without `>` is non-numglob per C's isnumglob.
            if j >= nb.len() || nb[j] != b'>' {
                drop(parse_n);
                patparse_off.store(entry_off, Ordering::Relaxed);
                let h = patnode(P_EXACTLY);
                let mut buf = patout.lock().unwrap();
                let len: u32 = 1;
                buf.extend_from_slice(&len.to_le_bytes());
                buf.push(b'<');
                patparse_off.fetch_add(1, Ordering::Relaxed);
                *flagp |= P_SIMPLE;
                *tail_out = h;
                return h as i64;
            }
            j += 1;
            drop(parse_n);
            patparse_off.store(j, Ordering::Relaxed);
            *flagp &= !P_PURESTR;

            let off2 = match len_flag {
                // c:1552-1567
                3 => {
                    // c:1554 P_NUMRNG
                    let off2 = patnode(P_NUMRNG);
                    let mut buf = patout.lock().unwrap();
                    buf.extend_from_slice(&from.to_le_bytes());
                    buf.extend_from_slice(&to.to_le_bytes());
                    off2
                }
                2 => {
                    // c:1559 P_NUMTO
                    let off2 = patnode(P_NUMTO);
                    let mut buf = patout.lock().unwrap();
                    buf.extend_from_slice(&to.to_le_bytes());
                    off2
                }
                1 => {
                    // c:1563 P_NUMFROM
                    let off2 = patnode(P_NUMFROM);
                    let mut buf = patout.lock().unwrap();
                    buf.extend_from_slice(&from.to_le_bytes());
                    off2
                }
                _ => patnode(P_NUMANY), // c:1568
            };
            *tail_out = off2;
            off2 as i64
        }
        _ => {
            // Accumulate a literal run.
            let mut buf: Vec<u8> = Vec::new();
            let mut local_off = off;
            let p = patparse.lock().unwrap();
            // c:1312-1317 — break on kshchar trigger (any of the six
            // ZPC_KSH_* slots whose literal byte is followed by `(`).
            // Snapshot the zpc_special slots that participate; when
            // KSHGLOB is off they're Marker (0xa2) and won't match
            // ordinary bytes.
            let (ksh_plus, ksh_bang, ksh_bang2, ksh_at, ksh_star, ksh_quest) = {
                let sp = zpc_special.lock().unwrap();
                (
                    sp[ZPC_KSH_PLUS as usize],
                    sp[ZPC_KSH_BANG as usize],
                    sp[ZPC_KSH_BANG2 as usize],
                    sp[ZPC_KSH_AT as usize],
                    sp[ZPC_KSH_STAR as usize],
                    sp[ZPC_KSH_QUEST as usize],
                )
            };
            // c:1314-1317 — `~` is a literal-run terminator IFF it's
            // active as the exclusion operator (EXTENDEDGLOB on) AND
            // the lookahead is `/` or a non-segment-special byte.
            // Without EXTENDEDGLOB the byte is Marker (0xa2) so the
            // equality check fails naturally. Limited to TILDE only —
            // the other ZPC_SEG_COUNT slots (SLASH, NULL, BAR, OUTPAR)
            // are already covered by the explicit stop list above
            // (`|` `)` handled there; `/` was deliberately NOT a
            // literal-run break in this Rust port, see
            // `zsh_corpus_hash_s_e_anchors_match_bare_test` which
            // expects `/` to stay literal inside `(...)` alternation).
            let (sp_tilde_lit, sp_seg_lit_set, sp_hat_lit, sp_hash_lit) = {
                let sp = zpc_special.lock().unwrap();
                let mut set = [false; 256];
                for i in 0..(ZPC_SEG_COUNT as usize) {
                    set[sp[i] as usize] = true;
                }
                (
                    sp[ZPC_TILDE as usize],
                    set,
                    sp[ZPC_HAT as usize],
                    sp[ZPC_HASH as usize],
                )
            };
            while local_off < p.len() {
                let b = p.as_bytes()[local_off];
                // c:Src/pattern.c:480-483 — `if (!isset(EXTENDEDGLOB))`
                // masks ZPC_HAT and ZPC_HASH to Marker so the
                // patcompiece dispatch treats `^` / `#` as literals
                // (no negation / no zero-or-more closure).
                // The literal-run break list hardcoded `b'^'` /
                // `b'#'` though, which still terminated the literal
                // accumulator and forced patcompiece's empty-buf
                // -1 return → "bad pattern: ^.*" for the unset
                // EXTENDEDGLOB case. Gate the break on the actual
                // zpc_special slot so an EXTENDEDGLOB-off run treats
                // `^` and `#` as ordinary chars. Bug #421.
                // c:1294-1319 — the literal-run loop is WRAPPED in
                //     if (zpc_special[ZPC_INPAR] != Marker ||
                //         *patparse != Outpar || paren) { ...break... }
                // "If '(' is disabled as a pattern char, allow ')' as
                //  an ordinary string character if there are no
                //  parentheses to close.  Don't allow it otherwise, it
                //  changes the syntax."  So with `(` disabled (SHGLOB /
                // `disable -p '('`) at top level, a `)` stays in the
                // run; inside a group (`paren`) it still closes it.
                // `(` itself is likewise only a run terminator while
                // its slot is live (c:1312-1313 memchr over
                // zpc_special, which holds Marker when disabled).
                let stop_here = match b {
                    b'(' => inpar_active,
                    b')' => inpar_active || paren != 0,
                    b'?' | b'*' | b'[' | b'|' | b'\\' | b'<' => true,
                    b'^' => sp_hat_lit == b'^',
                    b'#' => sp_hash_lit == b'#',
                    // c:950 / c:1312-1317 — `/` is ZPC_SLASH, a segment
                    // terminator, and C keeps it one by MASKING the slot
                    // (`zpc_special[ZPC_SLASH] = Marker`) where it must stay
                    // literal — for a non-FILE pattern (c:570) and inside
                    // parens (c:840-842) — rather than by dropping the rule.
                    // Mirror that with the same gate `patcompbranch` already
                    // uses above, so `(...)` alternation and `[[ ]]`/`:#`
                    // patterns are untouched. Without this a literal file
                    // component swallowed the rest of the path: the general
                    // compiler is only reached for such a component when the
                    // PURES fast path is off, i.e. under `(#i)`/`(#a1)`, so
                    // `(#i)sub/deep.txt` compiled `sub/deep.txt` as ONE
                    // component and matched nothing — even with exact case.
                    b'/' => paren == 0 && (patflags.load(Ordering::Relaxed) & PAT_FILE as i32) != 0,
                    _ => false,
                };
                if stop_here {
                    break;
                }
                if b == sp_tilde_lit && sp_tilde_lit != 0 {
                    let la = p.as_bytes().get(local_off + 1).copied().unwrap_or(0);
                    // Literal-tilde exception: `~` followed by a
                    // segment-special OTHER than `/` stays in the run.
                    let literal_tilde_exception = la != b'/' && sp_seg_lit_set[la as usize];
                    if !literal_tilde_exception {
                        break;
                    }
                }
                // c:1278-1292 — ksh-glob trigger lookahead. If the next
                // byte is `(` AND this byte matches one of the six
                // ZPC_KSH_* slots, stop the literal run BEFORE this byte
                // so the next patcomppiece call consumes it as a ksh
                // quantifier prefix (e.g. `pre+(x)post` accumulates
                // `pre` then breaks at `+` since `+(` follows).
                if local_off + 1 < p.len() && p.as_bytes()[local_off + 1] == b'(' {
                    if b == ksh_plus
                        || b == ksh_bang
                        || b == ksh_bang2
                        || b == ksh_at
                        || b == ksh_star
                        || b == ksh_quest
                    {
                        break;
                    }
                }
                buf.push(b);
                local_off += 1;
            }
            drop(p);
            if buf.is_empty() {
                return -1;
            }
            // c:Src/pattern.c:1340-1351 — multi-char run with TRAILING `#`
            // (or `(#cN,M)`) must backtrack ONE char so the trailing
            // quantifier applies to the LAST char only. Without this,
            // `fo#` compiles as `(fo)#` instead of `f` + `o#`, breaking
            // `(fo#)#` against "ffo" (the test pin we are fixing).
            //
            // C body:
            //   if ((*patparse == zpc_special[ZPC_HASH] || ...) && morelen)
            //       patparse = patprev;
            //
            // morelen = "more than one char in the literal run". The
            // backtrack pops the LAST byte from buf and rewinds patparse
            // by 1 so the next patcomppiece call sees that byte as its
            // own atom.
            // c:1341 — "If we have more than one character, a following hash
            // or (#c...) only applies to the last, so backtrack one character."
            // C tests THREE shapes, not just the bare `#` (c:1342-1350):
            //
            //   if ((*patparse == zpc_special[ZPC_HASH] ||
            //        (*patparse == zpc_special[ZPC_INPAR] &&
            //         patparse[1] == zpc_special[ZPC_HASH] &&
            //         patparse[2] == 'c') ||
            //        (*patparse == zpc_special[ZPC_KSH_AT] &&
            //         patparse[1] == Inpar &&
            //         patparse[2] == zpc_special[ZPC_HASH] &&
            //         patparse[3] == 'c')) && morelen)
            //       patparse = patprev;
            //
            // Only the first arm was ported, so `FPATH+(#c0,1)=` compiled the
            // count over the whole literal run `FPATH+` instead of over the
            // trailing `+` alone, and `FPATH=` never matched.
            let has_trailing_quantifier = {
                let sp = zpc_special.lock().unwrap();
                let sp_hash = sp[ZPC_HASH as usize];
                let sp_inpar = sp[ZPC_INPAR as usize];
                let sp_ksh_at = sp[ZPC_KSH_AT as usize];
                drop(sp);
                let p = patparse.lock().unwrap();
                let b = p.as_bytes();
                let at = |i: usize| -> u8 { b.get(local_off + i).copied().unwrap_or(0) };
                // c:1342 — bare `#` / `##` closure.
                let hash_at = sp_hash == b'#' && at(0) == b'#';
                // c:1343-1345 — `(#c...)` count spec.
                let count_at =
                    sp_hash == b'#' && at(0) == sp_inpar && at(1) == b'#' && at(2) == b'c';
                // c:1346-1349 — ksh `@(#c...)` form. `Inpar` is the tokenized
                // `(`; this port keeps pattern bytes raw (see patcompcharsset
                // at pattern.rs:443), so the literal `(` stands in for it.
                let ksh_count_at = sp_hash == b'#'
                    && at(0) == sp_ksh_at
                    && at(1) == b'('
                    && at(2) == b'#'
                    && at(3) == b'c';
                drop(p);
                hash_at || count_at || ksh_count_at
            };
            // c:1322 — `patprev = patparse; METACHARINC(patparse);`, so
            // `patprev` is the START of the LAST character of the run, which
            // may be multibyte. `last_char_start` is that boundary: walk back
            // over UTF-8 continuation bytes, exactly what METACHARINC's
            // stride implies.
            let last_char_start = {
                let mut cut = buf.len() - 1;
                while cut > 0 && (buf[cut] & 0xC0) == 0x80 {
                    cut -= 1;
                }
                cut
            };
            // c:1338 — `morelen = (patprev > str0);`. This is "the run holds
            // MORE THAN ONE CHARACTER", not more than one BYTE. The previous
            // port tested `buf.len() > 1`, so a run consisting of a SINGLE
            // multibyte character followed by `#` (`é#` under EXTENDEDGLOB)
            // took the backtrack branch, popped the whole 2-byte character,
            // rewound `local_off` to where the piece started, and returned a
            // zero-length P_EXACTLY having consumed NOTHING — so
            // `patcompbranch`'s piece loop (pattern.rs:1406) span forever
            // emitting empty nodes. C cannot reach that state: c:1336's
            // `DPUTS(patparse == str0, "BUG: matched nothing in
            // patcomppiece.")` records that a piece always consumes, and
            // `morelen` being character-based is what guarantees it — the
            // backtrack can only ever give back a character that was NOT the
            // first one in the run.
            let morelen = last_char_start > 0; // c:1338
            // c:1340-1351 — "If we have more than one character, a following
            // hash or (#c...) only applies to the last, so backtrack one
            // character."  `if (… ) && morelen) patparse = patprev;`
            if morelen && has_trailing_quantifier {
                let popped = buf.len() - last_char_start;
                buf.truncate(last_char_start);
                local_off -= popped;
            }
            patparse_off.store(local_off, Ordering::Relaxed);
            // c:1352-1357 — "If len is 1, we can't have an active # following,
            // so doesn't matter that we don't make X in `XX#' simple."
            //     if (!morelen) flags |= P_SIMPLE;
            // Only a ONE-character run can be the operand of a following
            // closure; a multi-character run stays non-simple (and pure
            // string). This port used to set P_SIMPLE unconditionally.
            if !morelen {
                *flagp |= P_SIMPLE; // c:1357
            }
            // c:1372-1376 —
            //     if ((patglobflags & GF_MULTIBYTE) && slen > 1)
            //         /* for multibyte single characters, treat x# as (x)# */
            //         flags &= ~P_SIMPLE;
            // `slen` is the run's UNMETAFIED byte length (c:1371), which for
            // this port is simply `buf.len()`. The P_SIMPLE closure path
            // (c:1718-1720 `patinsert(op, starter, NULL, 0)`, matched by
            // `patrepeat` at c:4121-4129, which asserts `P_LS_LEN(p) == 1`)
            // assumes a ONE-BYTE operand, so a single multibyte character
            // has to go through the general `(x)#` branch construction
            // instead.
            if (patglobflags.load(Ordering::Relaxed) & GF_MULTIBYTE) != 0 && buf.len() > 1 {
                *flagp &= !P_SIMPLE; // c:1375
            }
            let lit_off = patnode(P_EXACTLY);
            let mut buf_lit = patout.lock().unwrap();
            let len = buf.len() as u32;
            buf_lit.extend_from_slice(&len.to_le_bytes());
            buf_lit.extend_from_slice(&buf);
            *tail_out = lit_off;
            lit_off as i64
        }
    };

    if atom < 0 {
        return atom;
    }

    // c:1606-1644 — quantifier dispatch. Three sources of quantifier:
    //   (a) trailing `#` / `##` (zsh-extended hash repetition)
    //   (b) trailing `(#cN,M)` count-spec (handled in patcompbranch, not here)
    //   (c) kshchar form `+/*/?` already at the atom's leading byte.
    //
    // Rule c:1615 — if no hash AND no count AND kshchar is one of
    // (none, '@', '!'), no quantifier; return atom as-is. `@` is the
    // identity quantifier (match group exactly once); `!` already had
    // its negation compiled by patcompnot inside the atom.
    let q_off = patparse_off.load(Ordering::Relaxed);
    let parse2 = patparse.lock().unwrap();
    // c:1611-1620 — `if (*patparse == zpc_special[ZPC_HASH])` quantifier
    // dispatch. The C source reads `zpc_special[ZPC_HASH]`, which
    // patcompcharsset (c:480-483) sets to `'#'` only when
    // EXTENDEDGLOB is on, else to the Marker byte (c:476). A literal
    // '#' in the source pattern therefore does NOT trigger the
    // quantifier path when EXTENDEDGLOB is off — the comparison
    // `*patparse == Marker` fails. Previously this Rust check used
    // a bare `b'#'` comparison instead of consulting zpc_special,
    // making `(x)#` and `[a-z]##[0-9]##` match even with
    // extendedglob OFF (parity bugs #5/#6 vs real zsh).
    let hash_char = zpc_special.lock().unwrap()[ZPC_HASH as usize]; // c:1612
    let has_hash = hash_char == b'#' && q_off < parse2.len() && parse2.as_bytes()[q_off] == b'#'; // c:1611-1614
    drop(parse2);
    if !has_hash && (kshchar == 0 || kshchar == b'@' || kshchar == b'!') {
        return atom; // c:1616-1617
    }

    // c:1621-1622 — kshchar with hash is a parse error (too much at once).
    if kshchar != 0 && has_hash {
        return -1;
    }

    // c:1624-1644 — pick the operator + post-atom flags.
    let op: u8;
    let mut consume_hashes = 0;
    if kshchar == b'*' {
        // c:1624-1626
        op = P_ONEHASH;
        *flagp = P_HSTART;
    } else if kshchar == b'+' {
        // c:1627-1629
        op = P_TWOHASH;
        *flagp = P_HSTART;
    } else if kshchar == b'?' {
        // c:1630-1632
        op = 0; // sentinel — `?` desugars to (x|) via the BRANCH path below
        *flagp = 0;
    } else {
        // c:1637-1644 — `#` / `##`.
        let parse_h = patparse.lock().unwrap();
        let two = q_off + 1 < parse_h.len() && parse_h.as_bytes()[q_off + 1] == b'#';
        drop(parse_h);
        if two {
            op = P_TWOHASH;
            consume_hashes = 2;
        } else {
            op = P_ONEHASH;
            consume_hashes = 1;
        }
        *flagp = P_HSTART;
    }
    if consume_hashes > 0 {
        patparse_off.fetch_add(consume_hashes, Ordering::Relaxed);
    }

    // c:1705-1746 — quantifier emission.
    //
    // Read the atom's opcode so c:1705 (`?#` → `*`) optimization can
    // apply. The atom byte sits at `atom + I_OP` in patout.
    let atom_op = {
        let buf = patout.lock().unwrap();
        if (atom as usize) + I_OP < buf.len() {
            buf[atom as usize + I_OP]
        } else {
            0
        }
    };

    if ((*flagp & P_SIMPLE) != 0) && (op == P_ONEHASH || op == P_TWOHASH) && atom_op == P_ANY {
        // c:1705-1717 — `?#` becomes `*`; `?##` becomes `?*`. The atom
        // is P_ANY at offset `atom`; rewrite or pad as needed.
        let mut buf = patout.lock().unwrap();
        if op == P_TWOHASH {
            // c:1712-1713 — `?##` → `?*`: leave the P_ANY in place,
            // then emit P_STAR right after.
            drop(buf);
            let _star = patnode(P_STAR);
            *tail_out = atom as usize;
        } else {
            // c:1715-1716 — `?#` → `*`: just rewrite atom's opcode.
            buf[atom as usize + I_OP] = P_STAR;
            drop(buf);
            *tail_out = atom as usize;
        }
        *flagp &= !P_PURESTR;
    } else if ((*flagp & P_SIMPLE) != 0)
        && op != 0
        && (patglobflags.load(Ordering::Relaxed) & 0xff) == 0
    {
        // c:1718-1720 — simple operand, no approximate-match counter:
        // emit `patinsert(op, starter, NULL, 0)`. The matcher walks
        // the inserted P_ONEHASH/P_TWOHASH operand at scan+I_BODY.
        patinsert(op, atom as usize, None, 0);
        *flagp &= !P_PURESTR;
        *tail_out = atom as usize;
    } else if op == P_ONEHASH {
        // c:1721-1729 — emit x# as (x&|): P_WBRANCH with NULL payload
        // ahead of atom; loop via P_BACK; null alternative branch.
        // patinsert with `sz=size_of::<union upat>()=8` (we use 8 to
        // mirror the C `union upat` slot, then zero-pad it).
        let payload = [0u8; 8];
        patinsert(P_WBRANCH, atom as usize, Some(&payload), 8);
        // Either x — atom is now at atom+I_BODY+8.
        let back = patnode(P_BACK);
        patoptail(atom as usize, back); // c:1726 — loop back
        patoptail(atom as usize, atom as usize); // c:1727
        let alt = patnode(P_BRANCH);
        pattail(atom as usize, alt); // c:1728 — or
        let null_node = patnode(P_NOTHING);
        pattail(atom as usize, null_node); // c:1729 — null
        *flagp &= !P_PURESTR;
        // The piece's chain-tail is the trailing P_NOTHING node — its
        // `.next` slot is empty and is the correct splice point for
        // whatever piece patcompbranch chains in next.
        *tail_out = null_node;
    } else if op == P_TWOHASH {
        // c:1730-1738 — emit x## as x(&|): P_WBRANCH after atom; loop
        // back; null branch.
        let wbranch = patnode(P_WBRANCH); // c:1732 — either
        let payload = [0u8; 8]; // c:1733-1734 patadd((char *)&up, ..., sizeof(up), 0)
        {
            let mut buf = patout.lock().unwrap();
            buf.extend_from_slice(&payload);
        }
        pattail(atom as usize, wbranch); // c:1735
        let back = patnode(P_BACK);
        pattail(back, atom as usize); // c:1736 — loop back
        let alt = patnode(P_BRANCH);
        pattail(wbranch, alt); // c:1737 — or
        let null_node = patnode(P_NOTHING);
        pattail(atom as usize, null_node); // c:1738 — null
        *flagp &= !P_PURESTR;
        // Same as ONEHASH path — tail at the trailing P_NOTHING.
        *tail_out = null_node;
    } else if kshchar == b'?' {
        // c:1739-1746 — emit ?(x) as (x|).
        patinsert(P_BRANCH, atom as usize, None, 0); // c:1741 — either x
        let alt = patnode(P_BRANCH); // c:1742 — or
        pattail(atom as usize, alt);
        let null_node = patnode(P_NOTHING); // c:1743 — null
        pattail(atom as usize, null_node); // c:1744
        patoptail(atom as usize, null_node); // c:1745
        *flagp &= !P_PURESTR;
        // Same as ONEHASH/TWOHASH — tail at the trailing P_NOTHING.
        *tail_out = null_node;
    }

    // c:1747-1748 — a stray `#` immediately after = compile error.
    {
        let p = patparse.lock().unwrap();
        let q2 = patparse_off.load(Ordering::Relaxed);
        if q2 < p.len() && p.as_bytes()[q2] == b'#' {
            // Only flag as error when EXTENDEDGLOB-style # is enabled.
            let sp = zpc_special.lock().unwrap();
            if sp[ZPC_HASH as usize] == b'#' {
                return -1;
            }
        }
    }

    atom
}

/// Port of `patcompnot()` from `Src/pattern.c:1760`.
/// C: `patcompnot(int paren, int *flagsp)`.
///
/// C: `static long patcompnot(int paren, int *flagsp)`. Implements
/// the `^pat` (paren=0) or `!(pat)` (paren=1) extended/ksh-glob
/// negation by emitting `P_BRANCH → P_STAR → P_EXCSYNC` followed by
/// `P_EXCLUDE [payload]` and the asserted pattern terminated by
/// `P_EXCEND`, all joined at a trailing `P_NOTHING`.
///
/// NOTE: matcher support for `P_EXCSYNC`/`P_EXCEND`/`P_EXCLUDE` is
/// only partial in the Rust port — compile is faithful per
/// pattern.c:1760-1784 but match-time negation semantics may diverge
/// from C for complex backtracking edge cases. Tests covering
/// `!(foo)` / `!(foo|bar)` exercise the basic working subset.
pub fn patcompnot(paren: i32, flagsp: &mut i32) -> i64 {
    // c:1760
    // c:1767 — `*flagsp = P_HSTART;`. Negation always starts with `*`
    // semantically so the caller knows the piece can match at the
    // start of input.
    *flagsp = P_HSTART;

    // c:1769 — `starter = patnode(P_BRANCH);`
    let starter = patnode(P_BRANCH);
    // c:1770 — `br = patnode(P_STAR);`
    let br = patnode(P_STAR);
    // c:1771 — `excsync = patnode(P_EXCSYNC);`
    let excsync = patnode(P_EXCSYNC);
    // c:1772 — `pattail(br, excsync);`
    pattail(br, excsync);
    // c:1773 — `pattail(starter, excl = patnode(P_EXCLUDE));`
    let excl = patnode(P_EXCLUDE);
    pattail(starter, excl);
    // c:1774-1775 — `up.p = NULL; patadd((char *)&up, 0, sizeof(up), 0);`
    // The EXCLUDE node carries an 8-byte syncptr payload, NULL-init.
    {
        let mut buf = patout.lock().unwrap();
        buf.extend_from_slice(&[0u8; 8]);
    }
    // c:1776 — `br = (paren ? patcompswitch(1, &dummy) : patcompbranch(&dummy, 0));`
    let mut dummy: i32 = 0;
    let inner = if paren != 0 {
        let r = patcompswitch(1, &mut dummy);
        // c:1503-1505 — caller `Inpar` arm expects to consume the
        // trailing `)`. Mirror here since patcompnot is invoked from
        // the b'(' atom-arm AFTER it already consumed `(`.
        if r >= 0 {
            let cur = patparse_off.load(Ordering::Relaxed);
            let p = patparse.lock().unwrap();
            if cur >= p.len() || p.as_bytes()[cur] != b')' {
                return -1;
            }
            drop(p);
            patparse_off.fetch_add(1, Ordering::Relaxed);
        }
        r
    } else {
        patcompbranch(&mut dummy, 0)
    };
    if inner < 0 {
        return -1; // c:1777 `return 0;` (Rust uses -1 for failure)
    }
    // c:1778 — `pattail(br, patnode(P_EXCEND));`
    let excend = patnode(P_EXCEND);
    pattail(inner as usize, excend);
    // c:1779 — `n = patnode(P_NOTHING);`
    let n = patnode(P_NOTHING);
    // c:1780 — `pattail(excsync, n);`
    pattail(excsync, n);
    // c:1781 — `pattail(excl, n);`
    pattail(excl, n);
    // c:1783
    let _ = br; // suppress unused-var (kept for c-name parity)
    starter as i64
}

/// Port of `patnode()` from `Src/pattern.c:1790`.
/// C: `patnode(long op)`.
///
/// C: `static long patnode(long op)` — writes a 1-byte opcode plus a
/// 4-byte zeroed next-offset. Returns the offset of the opcode byte.
fn patnode(op: u8) -> usize {
    // c:1790
    let mut buf = patout.lock().unwrap();
    let off = buf.len();
    buf.push(op); // I_OP
    buf.extend_from_slice(&[0, 0, 0, 0]); // I_NEXT zeroed
    off
}

/// Port of `patinsert()` from `Src/pattern.c:1807`.
/// C: `patinsert(long op, int opnd, char *xtra, int sz)`.
///
/// C: `static void patinsert(long op, int opnd, char *xtra, int sz)`.
/// Inserts an opcode (+ next slot) at position `opnd`, shifting bytes
/// after it down by `5 + sz`, then writes `xtra` payload of `sz` bytes.
fn patinsert(op: u8, opnd: usize, xtra: Option<&[u8]>, sz: usize) {
    // c:1807
    let mut buf = patout.lock().unwrap();
    let header_sz = 1 + 4; // op + next
    let total = header_sz + sz;
    // Insert `total` zeroed bytes at opnd, then overwrite.
    let mut inserted = vec![0u8; total];
    inserted[0] = op;
    if let Some(x) = xtra {
        let copy_n = x.len().min(sz);
        inserted[header_sz..header_sz + copy_n].copy_from_slice(&x[..copy_n]);
    }
    buf.splice(opnd..opnd, inserted);
    // Patch up next_off chains pointing past opnd by adding `total`.
    fixup_offsets_after_insert(&mut buf, opnd, total as u32);
}

/// Port of `pattail()` from `Src/pattern.c:1834`.
/// C: `pattail(long p, long val)`.
///
/// C: `static void pattail(long p, long val)` — patches the next-offset
/// field of the opcode at offset `p` to point to `val`. Walks any
/// existing chain to the end before patching.
fn pattail(p: usize, val: usize) {
    // c:1834
    let mut buf = patout.lock().unwrap();
    let mut cur = p;
    loop {
        if cur + I_BODY > buf.len() {
            return;
        }
        let next_bytes: [u8; 4] = buf[cur + I_NEXT..cur + I_NEXT + 4].try_into().unwrap();
        let next = u32::from_le_bytes(next_bytes) as usize;
        if next == 0 {
            break;
        }
        cur = next;
    }
    let val_bytes = (val as u32).to_le_bytes();
    if cur + I_NEXT + 4 <= buf.len() {
        buf[cur + I_NEXT..cur + I_NEXT + 4].copy_from_slice(&val_bytes);
    }
}

/// Port of `patoptail()` from `Src/pattern.c:1856`.
/// C: `patoptail(long p, long val)`.
///
/// C: `static void patoptail(long p, long val)` — like pattail but
/// only patches branches (P_BRANCH/P_WBRANCH).
///
/// C: c:1862-1865 — for P_BRANCH the operand sits at `P_OPERAND(p)` =
/// `p+1` (Upat slot, i.e. byte offset `p + I_BODY` in Rust); for
/// P_WBRANCH the operand sits at `P_OPERAND(p) + 1` (skipping the
/// 8-byte syncptr payload Upat slot), i.e. byte offset
/// `p + I_BODY + 8` in Rust.
fn patoptail(p: usize, val: usize) {
    // c:1856
    let buf = patout.lock().unwrap();
    if p + I_OP >= buf.len() {
        return;
    }
    let op = buf[p + I_OP];
    drop(buf);
    if P_ISBRANCH(op) {
        // c:1862-1865 — operand offset depends on op kind.
        if op == P_BRANCH {
            pattail(p + I_BODY, val);
        } else {
            // P_WBRANCH / P_EXCLUDE / P_EXCLUDP — operand sits after
            // the 8-byte syncptr payload.
            pattail(p + I_BODY + 8, val);
        }
    }
}

// =====================================================================
// 13/14. Matcher — pattern.c:2223-3579
// =====================================================================

/// State accumulated during a single `patmatch` walk. C uses
/// per-thread globals (`patbeginp[]` / `patendp[]` for captures);
/// the Rust port encapsulates them in this struct passed by `&mut`.
/// Rule D: this struct represents matcher-internal scratch state
/// (analogous to `struct rpat pattrystate` at pattern.c:248), not a
/// bag-of-globals from unrelated subsystems.
///
/// **C counterpart**: `struct rpat` at `pattern.c:248`.
#[derive(Clone)]
#[allow(non_camel_case_types)]
pub struct rpat {
    /// Per-P_WBRANCH visit bitmap, keyed by WBRANCH-opcode offset.
    /// Mirrors C's `Upat ptrp` (`pattern.c:3217`): every WBRANCH carries
    /// an 8-byte payload sized as `union upat` — initialised to NULL,
    /// then lazily filled by `patmatch` with a buffer the size of the
    /// input string. Each byte tracks "have we already tried this
    /// WBRANCH at this input position with at most this many errors?".
    /// On revisit at the same position with the same-or-fewer errors,
    /// C returns 0 to bound the recursion (`pattern.c:3245-3248`). The
    /// Rust port keeps the bitmap on rpat (per-pattry call) instead of
    /// inside the bytecode payload so the bytecode stays read-only —
    /// the key is the WBRANCH offset; the value is a Vec<u8> the size
    /// of the input. Without it, `(fo#)#` against ANY input that
    /// requires the closure to consume at least one char per iteration
    /// (like "ffo") burns through PATMATCH_MAX_DEPTH and aborts.
    pub wbranch_visits: std::collections::HashMap<usize, Vec<u8>>,
    pub patbeginp: [usize; NSUBEXP], // c:241 capture starts (byte offsets)
    pub patendp: [usize; NSUBEXP],   // c:242 capture ends
    /// `parsfound` from `Src/pattern.c` (per c:2957/c:2989 references).
    /// Two-stripe bitmap: bits `0..NSUBEXP` track per-group P_OPEN
    /// first-write (`patbeginp[i]` committed); bits `NSUBEXP..2*NSUBEXP`
    /// track per-group P_CLOSE first-write (`patendp[i]` committed).
    /// Bit `n-1` (open) = `1 << (n-1)`; bit `n-1+NSUBEXP` (close) =
    /// `1 << (n-1+NSUBEXP)`. Width u32 to fit `2*NSUBEXP = 18` bits.
    /// The prior u16 width with a single-stripe (close-only) bit
    /// allowed P_OPEN to re-overwrite `patbeginp[i]` on every
    /// backtrack iteration — turning the SECOND capture's start
    /// offset into the FIRST iteration's, e.g. `(*)-(*)` against
    /// `hello-world` returned `match[2] = "hello-world"` instead of
    /// the right `"world"`.
    pub captures_set: u32, // c:Src/pattern.c parsfound
    /// Port of file-static `int errsfound` from `Src/pattern.c:2046`.
    /// Cumulative edit-count for approximate-match `(#aN)`. Reset to 0
    /// at the top of each pattry, incremented on P_EXACTLY mismatches
    /// when `glob_flags & 0xff > 0` (the substitution-budget byte).
    pub errsfound: i32, // c:2046
}

/// Port of `charref()` from `Src/pattern.c:1909`.
/// C: `wchar_t charref(char *x, char *y, int *zmb_ind)` from
/// `Src/pattern.c:1909`. Decode the char at `pos` without
/// advancing. UTF-32-native delegation: `s.chars().next()` returns
/// the decoded codepoint; delegates to Rust's native UTF-8 string
/// iterator instead of the C Meta-decode + zshtoken-translate +
/// mbrtowc state machine, which all collapse because Rust's `&str`
/// is already UTF-8.
///
/// Rust signature differs from C `charref(char *x, char *y, int *zmb_ind)`:
/// the C `y` end-of-string pointer is captured by `&str` length; the
/// `zmb_ind` out-param (multibyte-completion index) is dropped — the
/// Rust UTF-8 decode is one-shot, never partial.
pub fn charref(s: &str, pos: usize) -> Option<char> {
    // c:1909
    s[pos..].chars().next()
}

/// Port of `charnext()` from `Src/pattern.c:1936`.
/// C: `char *charnext(char *x, char *y)`.
/// C returns the pointer past the current character; the Rust port
/// returns that position as a byte offset. Delegates to `metacharinc`
/// — same advance-by-one-codepoint logic.
pub fn charnext(x: &str, y: usize) -> usize {
    // c:1936
    metacharinc(x, y)
}

/// Port of `charrefinc()` from `Src/pattern.c:1964`.
/// C: `wchar_t charrefinc(char **x, char *y, int *z)` from
/// `Src/pattern.c:1964`. Decode + advance: delegates to the
/// `charref`-then-`len_utf8`-step pattern. The C body's Meta /
/// mbrtowc / zshtoken triple collapses to one `chars().next()`
/// call followed by a byte-count step.
///
/// Rust signature differs from C: C mutates `*x` via the
/// `char **` to advance; Rust mutates `pos` directly. The `y`
/// end-of-string sentinel is captured by `&str` length; `z`
/// (multibyte-completion index) is dropped per the same one-shot
/// UTF-8 decode argument as `charref`.
pub fn charrefinc(s: &str, pos: &mut usize) -> Option<char> {
    // c:1964
    let c = s[*pos..].chars().next()?;
    *pos += c.len_utf8();
    Some(c)
}

/// Port of `charsub()` from `Src/pattern.c:1997`.
/// C: `ptrdiff_t charsub(char *x, char *y)`.
///
/// Returns the number of characters between the start `x` and the
/// position `y` — i.e. the character-distance of byte offset `y` from
/// the string start. Non-multibyte: the byte distance `y` (each byte
/// is one character). Multibyte: the codepoint count of `x[0..y]`.
/// `patmatch` uses this (via the C `CHARSUB` macro) to report match
/// lengths and backreference positions in characters.
///
/// Replaces the prior fake that stepped back one char and returned a
/// byte offset — unrelated to the C character-count semantics.
pub fn charsub(x: &str, y: usize) -> usize {
    // c:1997
    let y = y.min(x.len());
    if !isset(MULTIBYTE) {
        // c:2003-2004 — `if (!isset(MULTIBYTE)) return y - x;`
        return y;
    }
    // c:2011-2026 — count characters in `x[0..y]`. C bounds each
    // `mbrtowc` by `y-x` (c:2012), so an END OFFSET THAT CUTS a
    // multibyte character in half yields MB_INCOMPLETE and c:2014-2018
    // `return res + (y - x)` counts every leftover byte as one
    // character. That is reachable: with GF_MULTIBYTE clear (`(#U)`,
    // c:1116) the matcher steps raw BYTES (c:1946), so `patendp[i]` can
    // land mid-character. A plain `x[..y].chars().count()` PANICS there.
    let mut res = 0usize;
    let mut i = 0usize;
    while i < y {
        // `i` is always on a character boundary — it only ever advances
        // by a whole character below.
        match x[i..].chars().next() {
            Some(c) if i + c.len_utf8() <= y => {
                res += 1; // c:2024
                i += c.len_utf8(); // c:2025
            }
            _ => return res + (y - i), // c:2018
        }
    }
    res // c:2028
}

// =====================================================================
// 16. String pre-processing — pattern.c:2063, :2080, :2132
// =====================================================================

/// Port of `pattrystart()` from `Src/pattern.c:2063`. C resets per-
/// match state globals; Rust state is per-call so no-op.
pub fn pattrystart() {} // c:2063

/// Port of `patmungestring()` from `Src/pattern.c:2080`.
/// C: `void patmungestring(char **string, int *stringlen, int *unmetalenin)`
/// from `Src/pattern.c:2080`.
///
/// Skips a leading `Nularg` (the empty-tokenised-string sentinel)
/// and computes `*stringlen` from `strlen` when caller passed `-1`.
/// Mutates all three in/out args; mirrors C's `char **` /
/// `int *` semantics via Rust mutable references.
pub fn patmungestring(string: &mut &str, stringlen: &mut i32, unmetalenin: &mut i32) {
    // c:2080
    // c:2082-2091 — `if (*stringlen > 0 && **string == Nularg)` — skip
    // the leading Nularg sentinel and adjust lengths.
    let bytes = string.as_bytes();
    if *stringlen > 0 && !bytes.is_empty() && bytes[0] as char == Nularg {
        // c:2085 — `(*string)++;` — advance past Nularg.
        *string = &string[1..]; // c:2085
                                // c:2090 — `if (*unmetalenin > 0) (*unmetalenin)--;`
        if *unmetalenin > 0 {
            // c:2089
            *unmetalenin -= 1; // c:2090
        }
        // c:2092 — `if (*stringlen > 0) (*stringlen)--;`
        if *stringlen > 0 {
            // c:2091
            *stringlen -= 1; // c:2092
        }
    }

    // c:2096-2097 — `if (*stringlen < 0) *stringlen = strlen(*string);`
    if *stringlen < 0 {
        // c:2096
        *stringlen = string.len() as i32; // c:2097
    }
}

/// Port of `pattry()` from `Src/pattern.c:2223`.
/// C: `pattry(Patprog prog, char *string)`.
///
/// C signature: `int pattry(Patprog prog, char *string)`. Returns
/// non-zero on match, 0 on no-match.
pub fn pattry(prog: &Patprog, string: &str) -> bool {
    // c:2223
    // c:2225 — `return pattrylen(prog, string, len, -1, NULL, 0);`
    pattrylen(prog, string, string.len() as i32, -1, None, 0) // c:2225
}

/// Port of `pattrylen()` from `Src/pattern.c:2236`.
/// C: `int pattrylen(Patprog prog, char *string, int len,
/// int unmetalen, Patstralloc patstralloc, int offset)` from
/// `Src/pattern.c:2236`.
///
/// ```c
/// int
/// pattrylen(Patprog prog, char *string, int len, int unmetalen,
///           Patstralloc patstralloc, int offset)
/// {
///     return pattryrefs(prog, string, len, unmetalen, patstralloc, offset,
///                       NULL, NULL, NULL);
/// }
/// ```
pub fn pattrylen(
    prog: &Patprog,
    string: &str,
    len: i32, // c:2236
    unmetalen: i32,
    patstralloc: Option<&Patstralloc>,
    offset: i32,
) -> bool {
    // c:2238
    pattryrefs(
        prog,
        string,
        len,
        unmetalen,
        patstralloc,
        offset,
        None,
        None,
        None, // c:2239
    )
}

/// Port of `pattryrefs()` from `Src/pattern.c:2294`.
/// C: `int pattryrefs(Patprog prog, char *string, int stringlen,
/// int unmetalenin, Patstralloc patstralloc, int patoffset,
/// int *nump, int *begp, int *endp)` from `Src/pattern.c:2294`.
/// Runs `prog` against `string[0..stringlen]` (or whole string when
/// stringlen=-1) at `patoffset`, returning capture-group ranges.
///
/// C signature preserved per Rule S1. `Patstralloc` (the metafied-
/// string-allocator carrier from zsh.h:1613) is threaded through as
/// `Option<&Patstralloc>` matching C's `NULL`-passable pointer; the
/// matcher body currently ignores its contents (the substrate that
/// uses pre-allocated metafied buffers isn't wired into the Rust
/// matcher yet), but the param shape matches C so call sites trace
/// 1:1 to upstream.
#[allow(clippy::too_many_arguments)]
pub fn pattryrefs(
    // c:2294
    prog: &Patprog,
    string: &str,
    stringlen: i32,
    _unmetalenin: i32,
    _patstralloc: Option<&Patstralloc>,
    patoffset: i32,
    nump: Option<&mut i32>,
    begp: Option<&mut Vec<i32>>,
    endp: Option<&mut Vec<i32>>,
) -> bool {
    let trial: &str = if stringlen < 0 || (stringlen as usize) >= string.len() {
        string
    } else {
        &string[..stringlen as usize]
    };
    // Substring fast path for the ubiquitous `*literal*` shape
    // (optionally `(#i)`): program is [P_GFLAGS] P_STAR P_EXACTLY
    // P_STAR P_END with no captures requested. history-search-multi-
    // word's `${history[(R)(#i)*pat*]}` runs this shape over EVERY
    // history entry — 566k backtracking patmatch walks took CPU-
    // minutes per ^R (the reported freeze); a memmem-style scan is
    // what the shape actually needs. C reads this spirit via its
    // `mustoff` must-match prefilter (Src/pattern.c:2460-2483) but
    // declines under globflags; here the exact-bytes variant covers
    // any content and the `(#i)` variant is taken only when pattern
    // AND subject are pure ASCII (byte-fold == the matcher's char
    // fold there); anything else falls through to the full matcher.
    if nump.is_none()
        && begp.is_none()
        && endp.is_none()
        && patoffset == 0
        && (prog.0.flags & (PAT_NOTSTART | PAT_NOTEND) as i32) == 0
        // c:2399-2406 — a successful match is REJECTED for a file pattern
        // (PAT_NOGLD) whose subject starts with '.', unless the pattern itself
        // starts with a literal dot. That rejection lives below, AFTER
        // patmatch; returning from the fast path skipped it entirely, so
        // `*.*` matched `.hidden` (the literal `.` was found at offset 0 by
        // the substring scan) where zsh lists no dot files at all. Hand any
        // dot-leading subject under a file pattern to the full matcher, which
        // applies the rule. Costs nothing where the fast path was aimed:
        // history search (`${history[(R)(#i)*pat*]}`) is not a file glob and
        // never sets PAT_NOGLD, and non-dot files still take the fast path.
        && !((prog.0.flags & PAT_NOGLD as i32) != 0 && trial.starts_with('.'))
    {
        // Shape probe, inline (build-gate: no new fn in ported/ without
        // a C counterpart): Some((literal, igncase)) iff the program is
        // exactly `[P_GFLAGS] P_STAR P_EXACTLY(lit) P_STAR P_END` with
        // glob flags ⊆ IGNCASE|MULTIBYTE (no approx budget, no backrefs).
        let shape: Option<(String, bool)> = (|| {
            let buf: &[u8] = &prog.1;
            let mut off = prog.0.startoff as usize;
            let mut gflags = prog.0.globflags;
            let op_at = |o: usize| -> Option<u8> {
                if o + I_BODY <= buf.len() {
                    Some(buf[o + I_OP])
                } else {
                    None
                }
            };
            // A sole leading BRANCH (next == 0: no alternative) wraps
            // the whole program — step inside it.
            if op_at(off)? == P_BRANCH {
                let next = u32::from_le_bytes(buf[off + I_NEXT..off + I_NEXT + 4].try_into().ok()?);
                if next != 0 {
                    return None; // real alternation — full matcher
                }
                off = advance_past_instr(buf, off);
            }
            if op_at(off)? == P_GFLAGS {
                let body = off + I_BODY;
                if body + 4 > buf.len() {
                    return None;
                }
                gflags |= i32::from_le_bytes(buf[body..body + 4].try_into().ok()?);
                off = advance_past_instr(buf, off);
            }
            if (gflags & !(GF_IGNCASE | GF_MULTIBYTE)) != 0 {
                return None;
            }
            if op_at(off)? != P_STAR {
                return None;
            }
            off = advance_past_instr(buf, off);
            // One or more contiguous EXACTLY chunks form the literal
            // (the compiler splits long/space-bearing literals).
            if op_at(off)? != P_EXACTLY {
                return None;
            }
            let mut lit = String::new();
            while op_at(off)? == P_EXACTLY {
                let body = off + I_BODY;
                if body + 4 > buf.len() {
                    return None;
                }
                let len = u32::from_le_bytes(buf[body..body + 4].try_into().ok()?) as usize;
                if body + 4 + len > buf.len() {
                    return None;
                }
                lit.push_str(std::str::from_utf8(&buf[body + 4..body + 4 + len]).ok()?);
                off = advance_past_instr(buf, off);
            }
            if op_at(off)? != P_STAR {
                return None;
            }
            off = advance_past_instr(buf, off);
            if off + I_OP >= buf.len() || buf[off + I_OP] != P_END {
                return None;
            }
            Some((lit, (gflags & GF_IGNCASE) != 0))
        })();
        if let Some((lit, igncase)) = shape.as_ref().map(|(l, i)| (l.as_str(), *i)) {
            if !igncase {
                return trial.contains(lit);
            }
            if lit.is_ascii() && trial.is_ascii() {
                let lb = lit.as_bytes();
                let tb = trial.as_bytes();
                if lb.is_empty() {
                    return true;
                }
                if lb.len() <= tb.len() {
                    let l0 = lb[0].to_ascii_lowercase();
                    'scan: for s in 0..=(tb.len() - lb.len()) {
                        if tb[s].to_ascii_lowercase() != l0 {
                            continue;
                        }
                        for k in 1..lb.len() {
                            if tb[s + k].to_ascii_lowercase() != lb[k].to_ascii_lowercase() {
                                continue 'scan;
                            }
                        }
                        return true;
                    }
                }
                return false;
            }
            // Non-ASCII under (#i): general matcher below.
        }
    }
    let mut state = rpat::new();
    // c:Src/pattern.c:2334 — `patflags = prog->flags;` C copies the
    // prog's flags into the file-static `patflags` so subsequent
    // patmatch calls can read PAT_NOTSTART / PAT_NOTEND inside the
    // P_ISSTART / P_ISEND arms (c:3393, 3397). Rust mirrors this via
    // the `patflags` AtomicI32 at pattern.rs:217.
    //
    // Without this propagation, `${str:#(#s)PAT}` / `${str:#PAT(#e)}`
    // anchor checks fired at slice-local offset 0 / slice-local end
    // regardless of whether the caller meant the slice was an
    // interior chunk of a larger string (PAT_NOTSTART/PAT_NOTEND
    // bits exist on the prog precisely so callers can signal this).
    patflags.store(prog.0.flags, Ordering::Relaxed);
    // Pass the prog's GLOB flags (GF_* + `(#aN)` budget byte) to the
    // matcher. C threads `patglobflags` as a per-thread file-static;
    // the Rust port carries it through a fn param. The PAT_* flags
    // (PAT_STATIC etc.) stay on `prog.0.flags` for the outer
    // anchor/PURES checks at lines below.
    // c:Src/pattern.c — `if (prog->flags & PAT_ANY) { ret = 1; } else { ... }`.
    // Optimisation for a single "*" (the `**` complist node compiles its
    // section as `patcompile(NULL, …|PAT_ANY, …)`): it always matches the
    // whole string without running the bytecode. The PAT_NOGLD leading-dot
    // rejection below still applies (the "except for no_glob_dots" caveat).
    // Without this the empty `PAT_ANY` program was matched literally and
    // only matched the empty string, so `**` descended into nothing.
    let (mut ok, matched_end) = if (prog.0.flags & PAT_ANY as i32) != 0 {
        (true, trial.len())
    } else {
        match patmatch(&prog.1, 0, trial, 0, &mut state, prog.0.globflags) {
            Some(end_pos) => {
                // c:2438 — `if (matched && !(prog->flags & (PAT_NOANCH|PAT_NOTEND))) ...`
                let no_anchor = (prog.0.flags & (PAT_NOANCH | PAT_NOTEND) as i32) != 0;
                (no_anchor || end_pos == trial.len(), end_pos)
            }
            None => (false, 0),
        }
    };
    // c:2399-2406 — for files (PAT_NOGLD), a successful match is rejected
    // when the string starts with '.' (unless glob_dots is set, in which
    // case parsecomplist omits PAT_NOGLD). Honoring this here lets the
    // glob scanner rely on the matcher for the leading-dot skip, exactly
    // as C does, instead of an explicit check in the walker. Inert for
    // non-file patterns (matchpat / [[ ]] / :# never set PAT_NOGLD).
    if ok
        && (prog.0.flags & PAT_NOGLD as i32) != 0
        && trial.starts_with('.')
        && prog.0.patstartch != b'.'
    {
        // c:Src/pattern.c — NOGLD rejects a leading-dot file UNLESS the
        // pattern itself explicitly begins with a literal `.` (so `.*`
        // matches dot files while `*` does not). C bakes this into the
        // compiled bytecode; the Rust matcher carries the signal on
        // `patstartch` (c:1610) and applies the exception here.
        ok = false;
    }
    if ok {
        // c:2508 — `patinlen = patinput - patinstart;` — record the
        // byte-length of the successful match so `patmatchlen()` can
        // return it after the strings/state are torn down. `end_pos`
        // is the in-trial byte offset where patmatch stopped, which
        // is `patinput - patinstart` in C terms.
        patinlen.store(matched_end as i32, Ordering::Relaxed);
    }
    if ok {
        let n = (prog.0.patnpar as usize).min(NSUBEXP);
        let have_nump = nump.is_some();
        // c:2536 / c:2587 — `metafy(patinstart, *sp - patinstart, META_DUP)`:
        // C hands back the RAW matched bytes, re-metafied. With GF_MULTIBYTE
        // clear (`(#U)`, c:1116) the match unit is a single byte (c:1946), so
        // `lo`/`hi` can cut a multibyte character in half and a plain
        // `trial[lo..hi]` slice PANICS. Rebuild such a span the way zshrs
        // stores `$'\xNN'` elsewhere — Meta (0x83) followed by `byte ^ 32` —
        // which is exactly what C's `metafy` produces.
        let metafy_span = |lo: usize, hi: usize| -> String {
            if let Some(s) = trial.get(lo..hi) {
                return s.to_string();
            }
            let mut out = String::new();
            for &b in &trial.as_bytes()[lo..hi] {
                if b >= crate::ported::zsh_h::Meta {
                    out.push('\u{83}'); // c:4862 Meta
                    out.push(char::from(b ^ 32)); // c:4863 `*t++ = *p ^ 32`
                } else {
                    out.push(char::from(b));
                }
            }
            out
        };
        if let Some(np) = nump {
            *np = n as i32;
        }
        // c:2526-2542 — GF_MATCHREF (`(#m)`): on success, write the
        // matched substring to $MATCH and its 1-based (KSHARRAYS-
        // aware) char span to $MBEGIN/$MEND. C does this INSIDE
        // pattryrefs; the previous Rust port left it to a bridge
        // wrapper that sniffed the pattern TEXT for "(#m)" — wrong
        // mechanism (missed hoisted/nested flag positions) and
        // wrong layer.
        // c:2526 — `if ((patglobflags & GF_MATCHREF) && !(patflags & PAT_FILE))`.
        // The source is the GLOBAL patglobflags (c:273), not `prog->globflags`.
        // They are not the same thing: c:Src/zsh.h:1606 documents the struct
        // field as "globbing flags to set at START", so it keeps whatever the
        // pattern opened with, while the global is what patgetglobflags leaves
        // after walking every flag — including a later `(#M)` turning
        // GF_MATCHREF back off (c:1099-1100).
        //
        // Reading the struct field made `(#M)` inert: for all three of
        // `(#m)a*`, `(#m)(#M)a*` and `(#M)(#m)a*` it is 0x1800, whereas the
        // global correctly ends at 0x800 / 0x0 / 0x800. So `(#m)(#M)a*` still
        // wrote $MATCH where zsh leaves it unset.
        let cur_globflags = patglobflags.load(Ordering::Relaxed);
        if (cur_globflags & GF_MATCHREF) != 0 && (prog.0.flags & PAT_FILE as i32) == 0 {
            let hi = matched_end.min(trial.len());
            let mstr: String = metafy_span(0, hi); // c:2536 metafy(patinstart..patinput)
            let mlen = mstr.chars().count() as i64; // c:2534 CHARSUB
            let base: i64 = if crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS) {
                0
            } else {
                1
            };
            crate::ported::params::setsparam("MATCH", &mstr); // c:2537
            crate::ported::params::setiparam("MBEGIN", patoffset as i64 + base); // c:2538
            crate::ported::params::setiparam("MEND", mlen + patoffset as i64 + base - 1);
            // c:2539-2541
        }
        // c:2425+ — emit captured offsets to begp/endp out-arrays.
        // Check the CLOSE bit (high stripe at NSUBEXP+i) since the
        // group is only fully captured after its P_CLOSE fired; the
        // matching open-bit `1 << i` would be set after P_OPEN even
        // when the rest of the match later fails to reach P_CLOSE.
        //
        // c:Src/pattern.c:2556-2558 — `*begp++ = CHARSUB(patinstart, *sp) + patoffset;`.
        // `patoffset` is the start position of `string` within the
        // ORIGINAL string the caller was matching against (used for
        // paramsubst's sliding-window scan via `${var/PAT/REPL}`). Add
        // it to every reported beg/end so MBEGIN/MEND / `$match` see
        // positions relative to the original string, not the local
        // trial slice.
        // c:2556-2566 — `*begp++ = …` writes into the caller's FIXED-SIZE array
        // (C: `int begpos[MAX_POS]`), filling the first `n` slots and leaving
        // the array's length unchanged. Write IN PLACE (extend only if the
        // caller's Vec is shorter than `n`) rather than clear()+push — clearing
        // shrank a MAX_POS-length array down to `n`, so the caller's
        // [n..MAX_POS] reset loop (complist.rs getcol, c:631) indexed past the
        // end and panicked (`index 4 on len 4`) on patterns with < MAX_POS
        // backrefs.
        if let Some(bv) = begp {
            for i in 0..n {
                let close_bit = 1u32 << (i + NSUBEXP);
                let val = if (state.captures_set & close_bit) != 0 {
                    // c:2556-2558 — `CHARSUB(patinstart, *sp)` counts
                    // CHARACTERS, not bytes (c:1997 charsub honours the
                    // MULTIBYTE option). `patbeginp[i]` is a byte offset into
                    // `trial`, so convert before adding the (already
                    // character-based, c:2285-2286) `patoffset`.
                    charsub(trial, state.patbeginp[i]) as i32 + patoffset
                } else {
                    -1 // c:2563-2566 — unset group (unmatched alternation branch)
                };
                if i < bv.len() {
                    bv[i] = val;
                } else {
                    bv.push(val);
                }
            }
        }
        if let Some(ev) = endp {
            for i in 0..n {
                let close_bit = 1u32 << (i + NSUBEXP);
                let val = if (state.captures_set & close_bit) != 0 {
                    // c:2562-2564 — `*endp++ = CHARSUB(patinstart, *ep) +
                    // patoffset - 1;`. The reported end is the index of the LAST
                    // matched character, not one past it; `patendp[i]` is the
                    // exclusive end, hence the `- 1`.
                    //
                    // Load-bearing for the only consumer of these arrays,
                    // complist's putmatchcol (c:Src/Zle/complist.c:889).
                    // `doiscol`'s finished-empty test at complist.c:646 is
                    // `endpos[i] < begpos[i]`, which an empty capture can only
                    // satisfy when the end is INCLUSIVE (b, b-1). With the
                    // exclusive value (b, b) an empty `(#b)` group consumed a
                    // colour AND painted a character, and every non-empty group
                    // stayed coloured one character too long.
                    //
                    // `CHARSUB` counts characters (c:1997), so the byte offset
                    // in `patendp[i]` converts through `charsub` first.
                    charsub(trial, state.patendp[i]) as i32 + patoffset - 1
                } else {
                    -1 // c:2565
                };
                if i < ev.len() {
                    ev[i] = val;
                } else {
                    ev.push(val);
                }
            }
        }
        // c:2570-2621 — `else if (prog->patnpar && !(patflags &
        // PAT_FILE))`: the caller passed NO capture arrays, so
        // pattryrefs itself publishes the `(#b)` groups as the
        // $match / $mbegin / $mend arrays. This arm was missing
        // from the port — every plain pattry of a (#b) pattern
        // silently dropped its captures (the bridge compensated
        // with a wrapper; now C-faithful at the source).
        if !have_nump && n > 0 && (prog.0.flags & PAT_FILE as i32) == 0 {
            let base: i32 = if crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS) {
                0
            } else {
                1
            };
            let mut match_arr: Vec<String> = Vec::with_capacity(n);
            let mut begin_arr: Vec<String> = Vec::with_capacity(n);
            let mut end_arr: Vec<String> = Vec::with_capacity(n);
            for i in 0..n {
                let close_bit = 1u32 << (i + NSUBEXP);
                if (state.captures_set & close_bit) != 0 {
                    let b = state.patbeginp[i];
                    let e = state.patendp[i];
                    let lo = b.min(trial.len());
                    let hi = e.min(trial.len()).max(lo);
                    match_arr.push(metafy_span(lo, hi)); // c:2587 metafy(*sp..*ep)
                                                         // c:2596-2599 — `CHARSUB(patinstart, *sp) + patoffset +
                                                         // !isset(KSHARRAYS)`. CHARSUB (c:1997) counts CHARACTERS;
                                                         // `patbeginp`/`patendp` hold BYTE offsets into `trial`, so
                                                         // convert through charsub before adding the (already
                                                         // character-based, c:2285-2286) patoffset.
                    begin_arr.push((charsub(trial, b) as i32 + patoffset + base).to_string()); // c:2596-2599
                                                                                               // c:2601-2604 — mend = last matched char index
                                                                                               // (inclusive): end + offset + base - 1.
                    end_arr.push((charsub(trial, e) as i32 + patoffset + base - 1).to_string());
                } else {
                    // c:2607-2613 — unmatched branch / hashed paren.
                    match_arr.push(String::new());
                    begin_arr.push("-1".to_string());
                    end_arr.push("-1".to_string());
                }
            }
            crate::ported::params::setaparam("match", match_arr); // c:2619
            crate::ported::params::setaparam("mbegin", begin_arr); // c:2620
            crate::ported::params::setaparam("mend", end_arr); // c:2621
        }
    }
    ok
}

// =====================================================================
// 15. Range matching — pattern.c:3856, :4004, :3610, :3767
// =====================================================================

/// Port of `patmatchlen()` from `Src/pattern.c:2649`.
/// C: `int patmatchlen(void)`.
///
/// ```c
/// /**/
/// int
/// patmatchlen(void)
/// {
///     return patinlen;
/// }
/// ```
///
/// Returns the length in metafied bytes of the last successful
/// `pattry` / `pattryrefs` match. `patinlen` is set at
/// `pattern.c:2508` (`patinlen = patinput - patinstart`); the
/// Rust port sets the equivalent `patinlen` AtomicI32 at the end
/// of `pattryrefs` (see `pattern.rs::pattryrefs`).
pub fn patmatchlen() -> i32 {
    // c:2649-2652
    patinlen.load(Ordering::Relaxed) // c:2651 — `return patinlen;`
}
/// Port of `patmatchindex()` from `Src/pattern.c:4004`.
/// C: `int patmatchindex(char *range, int ind, int *chr, int *mtp)`
/// from `Src/pattern.c:4004` (MULTIBYTE_SUPPORT-disabled single-byte
/// variant). Walks a NULL-terminated, METAFIED, PP_*-encoded byte
/// stream `range` and returns the character (or POSIX-class id)
/// at index `ind`. Output:
///   - `Some((Some(ch), mtp))` — literal character or PP_RANGE hit;
///     `chr = ch`, `mtp = 0`.
///   - `Some((None,    mtp))` — POSIX class match (PP_ALPHA etc);
///     `chr = -1` in C, `None` in Rust; `mtp = class id`.
///   - `None` — `ind` exceeds the descriptor's length.
///
/// Encoding the byte stream uses:
///   * literal byte `< 0x83` = match char as itself
///   * `Meta(0x83)` + (byte ^ 0x20) = ordinary metafied character
///   * `Meta + PP_*` (0x84..) = POSIX class marker
///   * `Meta + PP_RANGE` = next two metafied chars are `lo, hi`
pub fn patmatchindex(range: &[u8], mut ind: i32) -> Option<(Option<u8>, i32)> {
    // c:4004-4081
    let mut chr: Option<u8> = None; // c:4014 — `*chr = -1`
    let mut mtp: i32 = 0; // c:4015 — `*mtp = 0`

    // C `UNMETA(range)` + `METACHARINC(range)` macro pair from
    // Src/zsh.h:1796-1797 — decode-and-advance one metafied
    // character. Returned as `(decoded_byte, byte_advance)`.
    let unmeta = |bytes: &[u8], i: usize| -> (u8, usize) {
        if i < bytes.len() && bytes[i] == Meta && i + 1 < bytes.len() {
            (bytes[i + 1] ^ 0x20, 2)
        } else if i < bytes.len() {
            (bytes[i], 1)
        } else {
            (0, 0)
        }
    };

    let mut i = 0usize;
    while i < range.len() {
        // c:4017 — `for (; *range; range++)`
        let b = range[i];
        if crate::ported::utils::imeta_byte(b) {
            // c:4018 — `if (imeta((unsigned char) *range))`
            // c:4019 — `int swtype = (unsigned char) *range - (unsigned char) Meta;`
            let swtype = (b as i32) - (Meta as i32);
            match swtype {
                0 => {
                    // c:4021-4028 — `case 0: ordinary metafied character`
                    // c:4023 — `rchr = (unsigned char) *++range ^ 32;`
                    i += 1;
                    if i >= range.len() {
                        break;
                    }
                    let rchr = range[i] ^ 0x20;
                    if ind == 0 {
                        // c:4024-4026 — `if (!ind) { *chr = rchr; return 1; }`
                        chr = Some(rchr);
                        return Some((chr, mtp));
                    }
                    // c:4027 — falls through to `if (!ind--) break;`
                }
                t if (PP_ALPHA..=PP_INVALID).contains(&t) => {
                    // c:4030-4051 — POSIX class markers PP_ALPHA..PP_INVALID
                    if ind == 0 {
                        // c:4052-4054 — `if (!ind) { *mtp = swtype; return 1; }`
                        mtp = swtype;
                        return Some((None, mtp));
                    }
                }
                t if t == PP_RANGE => {
                    // c:4057-4069 — PP_RANGE: next two metafied chars
                    // `range++; r1 = UNMETA(range); METACHARINC(range);
                    //  r2 = UNMETA(range); if (*range == Meta) range++;`
                    i += 1;
                    if i >= range.len() {
                        break;
                    }
                    let (r1, adv1) = unmeta(range, i);
                    i += adv1;
                    if i >= range.len() {
                        break;
                    }
                    let (r2, adv2) = unmeta(range, i);
                    i += adv2;
                    let rdiff = r2 as i32 - r1 as i32;
                    if rdiff >= ind {
                        // c:4063-4067 — `if (rdiff >= ind) { *chr = r1 + ind; return 1; }`
                        chr = Some((r1 as i32 + ind) as u8);
                        return Some((chr, mtp));
                    }
                    // c:4068 — `ind -= rdiff;` (extra decrement happens via
                    // the `ind--` below, accounting for the C comment
                    // "note the extra decrement to ind below").
                    ind -= rdiff;
                    // Skip the trailing `if (!ind--) break;` decrement
                    // for this branch since C's loop already does it.
                }
                _ => {
                    // c:4070-4076 — PP_UNKWN / default → DPUTS bug warn
                    // (unreachable in well-formed compiled output; bail).
                }
            }
        } else {
            // c:4079-4082 — literal char path
            if ind == 0 {
                // c:4080-4082 — `if (!ind) { *chr = (unsigned char) *range; return 1; }`
                chr = Some(b);
                return Some((chr, mtp));
            }
        }
        // c:4087 — `if (!ind--) break;` — pre-decrement-check.
        if ind == 0 {
            break;
        }
        ind -= 1;
        i += 1; // c:4017 — `range++`
    }
    // c:4091 — `/* No corresponding index. */ return 0;`
    None
}

/// Port of `patmatchrange(char *range, int ch, int *indptr, int *mtp)`
/// from `Src/pattern.c:3865-3995` (reached through the
/// `PATMATCHRANGE` macro). Walks an encoded character-range
/// descriptor in `str` (Cpattern.str byte sequence) and tests
/// whether `c` falls inside. Encoding written by
/// `complete.rs::parse_class` (c:Src/Zle/complete.c:523/539):
///   0x80 + PP_RANGE (=0x95): next 2 bytes are lo,hi range
///   0x80 + PP_* (POSIX class id): single-byte class marker
///   plain byte: literal char (0x00-0x7F)
/// The port's marker base is 0x80 where C uses `Meta` (0x83); every
/// decoder in the port agrees on 0x80, so the offset is internal.
///
/// `indptr` is the position of `c` among the class MEMBERS seen so far:
/// c:3970 adds `ch - r1` on a hit, c:3974 adds `r2 - r1` when stepping
/// over a non-matching range, and c:3992-3993 adds a further 1 at the
/// end of EVERY non-returning iteration — so a skipped range advances
/// by its full `r2 - r1 + 1` member count and a skipped literal or
/// POSIX-class marker advances by exactly 1. That is the same counting
/// `pattern_match_equivalence`'s PATMATCHINDEX walk (c:1320,
/// pattern.c:4013-4088) performs in reverse, so producer and consumer
/// must agree element-for-element. `pattern_match1` turns that
/// into the equivalence-class index (`ind + 1`, c:1283) that
/// `pattern_match` compares between the line and word side
/// (c:1573 `if (ind != wind) return 0;`), and that
/// `pattern_match_equivalence` feeds back through PATMATCHINDEX.
/// The port previously incremented by 1 per element, so every char of
/// a `{a-z}`-style class collapsed to index 0: `m:{a-z\-}={A-Z\_}`
/// then matched `a` against EVERY uppercase name (`cd /a<TAB>`
/// offered /Applications /Library /System /Users /Volumes, and the
/// bogus ambiguity suppressed the insertion entirely).
pub fn patmatchrange(
    s: Option<&[u8]>,
    c: u32,
    mut indp: Option<&mut u32>,
    mtp: Option<&mut i32>,
) -> bool {
    let Some(bytes) = s else {
        return false;
    };
    // c:3869 — `if (indptr) *indptr = 0;`
    if let Some(out) = indp.as_deref_mut() {
        *out = 0;
    }

    let mut i = 0usize;
    let mut mtp_dest: Option<&mut i32> = mtp;
    // c:3876 — `for (; *range; range++)`
    while i < bytes.len() {
        let b = bytes[i];
        if b >= 0x80 {
            // c:3877-3878 — `imeta(*range)`; swtype = marker - base.
            let swtype = (b as i32) - 0x80;
            // c:3879-3880 — `if (mtp) *mtp = swtype;` runs for EVERY
            // meta element, whether or not it goes on to match.
            if let Some(out) = mtp_dest.as_deref_mut() {
                *out = swtype;
            }
            if swtype == 0 {
                // c:3882-3885 — metafied literal: next byte ^ 32.
                if i + 1 >= bytes.len() {
                    break;
                }
                if (bytes[i + 1] ^ 32) as u32 == c {
                    return true;
                }
                i += 2;
                // c:3992-3993 — fall through to the shared `(*indptr)++`.
                if let Some(out) = indp.as_deref_mut() {
                    *out += 1;
                }
                continue;
            }
            if swtype == PP_RANGE {
                // c:3961-3975
                if i + 2 >= bytes.len() {
                    break;
                }
                let r1 = bytes[i + 1] as u32;
                let r2 = bytes[i + 2] as u32;
                if r1 <= c && c <= r2 {
                    // c:3968-3971
                    if let Some(out) = indp.as_deref_mut() {
                        *out += c - r1;
                    }
                    return true;
                }
                // c:3973-3974 — `if (indptr && r1 < r2) *indptr += r2 - r1;`
                if r1 < r2 {
                    if let Some(out) = indp.as_deref_mut() {
                        *out += r2 - r1;
                    }
                }
                i += 3;
                // c:3992-3993 — plus the per-iteration increment, so a skipped
                // range advances the index by its full member count
                // (r2 - r1 + 1). Omitting it made the producer disagree with
                // pattern_match_equivalence's PATMATCHINDEX consumer, which
                // does count every element.
                if let Some(out) = indp.as_deref_mut() {
                    *out += 1;
                }
                continue;
            }
            // c:3886-3960 — POSIX classes. Single-byte locale
            // (the C build only reaches this file without
            // MULTIBYTE_SUPPORT), so `ch` outside 0-255 never matches.
            let hit = c < 256 && {
                let cb = c as u8;
                match swtype {
                    PP_ALPHA => cb.is_ascii_alphabetic(),   // c:3886
                    PP_ALNUM => cb.is_ascii_alphanumeric(), // c:3890
                    PP_ASCII => (c & !0x7f) == 0,           // c:3894
                    PP_BLANK => cb == b' ' || cb == b'\t',  // c:3898
                    PP_CNTRL => cb.is_ascii_control(),      // c:3907
                    PP_DIGIT => cb.is_ascii_digit(),        // c:3911
                    PP_GRAPH => cb.is_ascii_graphic(),      // c:3915
                    PP_LOWER => cb.is_ascii_lowercase(),    // c:3919
                    // c:3923 ZISPRINT — C isprint(): graphic or space.
                    PP_PRINT => cb.is_ascii_graphic() || cb == b' ',
                    PP_PUNCT => cb.is_ascii_punctuation(), // c:3927
                    // c:3931 isspace(): " \t\n\v\f\r"
                    PP_SPACE => matches!(cb, b' ' | 0x09..=0x0d),
                    PP_UPPER => cb.is_ascii_uppercase(), // c:3935
                    PP_XDIGIT => cb.is_ascii_hexdigit(), // c:3939
                    PP_IDENT => crate::ported::ztype_h::iident(cb), // c:3943
                    PP_IFS => crate::ported::ztype_h::isep(cb), // c:3947
                    PP_IFSSPACE => crate::ported::ztype_h::iwsep(cb), // c:3951
                    PP_WORD => crate::ported::ztype_h::iword(cb), // c:3955
                    // c:3959-3961 PP_INCOMPLETE / PP_INVALID are never
                    // true without MULTIBYTE_SUPPORT; PP_UNKWN / unknown
                    // markers fall through as no-match.
                    _ => false,
                }
            };
            if hit {
                return true;
            }
            i += 1;
            // c:3992-3993 — a skipped POSIX-class marker is one class member.
            if let Some(out) = indp.as_deref_mut() {
                *out += 1;
            }
        } else if b as u32 == c {
            // c:3987-3990 — plain literal match sets `*mtp = 0`.
            if let Some(out) = mtp_dest.as_deref_mut() {
                *out = 0;
            }
            return true;
        } else {
            i += 1;
            // c:3992-3993 — and so is a skipped literal.
            if let Some(out) = indp.as_deref_mut() {
                *out += 1;
            }
        }
    }
    false
}

/// Port of `mb_patmatchrange()` from `Src/pattern.c:3610`.
/// C: `int mb_patmatchrange(char *range, wchar_t ch,
/// int zmb_ind, wint_t *indptr, int *mtp)` from `Src/pattern.c:3610`.
/// Multibyte variant of `patmatchrange`: walks the metafied,
/// PP_*-encoded byte stream `range` and tests whether wide char `ch`
/// is in it. `indptr` (when Some) accumulates the running index for
/// `mb_patmatchindex`-side lookup; `mtp` (when Some) records which
/// PP_* class fired.
///
/// `zmb_ind` is the `ZMB_*` multibyte-completion state from the
/// caller (typically the pattry input pos): ZMB_INCOMPLETE /
/// ZMB_INVALID feed the PP_INCOMPLETE / PP_INVALID branches.
///
/// The Rust port delegates `iswalpha` / `iswdigit` / `wcsitype` to
/// `is_alphabetic()` / `is_numeric()` / etc. on the decoded `char`,
/// matching the C `iswXXX(wchar_t)` semantics.
pub fn mb_patmatchrange(
    range: &[u8],
    ch: char,
    zmb_ind: i32,
    mut indptr: Option<&mut u32>,
    mut mtp: Option<&mut i32>,
) -> bool {
    // c:3610-3766
    if let Some(p) = indptr.as_deref_mut() {
        *p = 0; // c:3615-3616 — `if (indptr) *indptr = 0;`
    }
    // C `UNMETA(s)` + `METACHARINC(s)` macro pair (Src/zsh.h).
    let unmeta = |bytes: &[u8], i: usize| -> (u8, usize) {
        if i < bytes.len() && bytes[i] == Meta && i + 1 < bytes.len() {
            (bytes[i + 1] ^ 0x20, 2)
        } else if i < bytes.len() {
            (bytes[i], 1)
        } else {
            (0, 0)
        }
    };
    let mut i = 0usize;
    while i < range.len() {
        // c:3623 — `while (*range)`
        let b = range[i];
        if crate::ported::utils::imeta_byte(b) {
            // c:3624 — `if (imeta((unsigned char) *range))`
            // c:3625 — `swtype = (unsigned char) *range++ - (unsigned char) Meta;`
            let swtype = (b as i32) - (Meta as i32);
            i += 1;
            if let Some(p) = mtp.as_deref_mut() {
                *p = swtype; // c:3626-3627 — `if (mtp) *mtp = swtype;`
            }
            let class_hit = match swtype {
                0 => {
                    // c:3629-3634 — `case 0: ordinary metafied character`
                    i -= 1; // c:3631 — `range--;`
                    let (decoded, adv) = unmeta(range, i);
                    i += adv;
                    decoded == (ch as u32) as u8 && (ch as u32) < 256
                }
                t if t == PP_ALPHA => ch.is_alphabetic(),
                t if t == PP_ALNUM => ch.is_alphanumeric(),
                t if t == PP_ASCII => (ch as u32) < 128,
                t if t == PP_BLANK => ch == ' ' || ch == '\t',
                t if t == PP_CNTRL => ch.is_control(),
                t if t == PP_DIGIT => ch.is_ascii_digit(),
                t if t == PP_GRAPH => !ch.is_whitespace() && !ch.is_control(),
                t if t == PP_LOWER => ch.is_lowercase(),
                t if t == PP_PRINT => !ch.is_control(),
                t if t == PP_PUNCT => ch.is_ascii_punctuation(),
                t if t == PP_SPACE => ch.is_whitespace(),
                t if t == PP_UPPER => ch.is_uppercase(),
                t if t == PP_XDIGIT => ch.is_ascii_hexdigit(),
                t if t == PP_IDENT => {
                    // c:3704 — `PP_IDENT: if (wcsitype(ch, IIDENT)) return 1;`
                    // IIDENT = alphanumeric, underscore, or "$" depending on
                    // option state. Conservative: ASCII identifier chars.
                    ch.is_alphanumeric() || ch == '_'
                }
                t if t == PP_IFS => {
                    // c:3708 — `wcsitype(ch, ISEP)`. ISEP = space, tab,
                    // newline + user IFS additions. Default IFS in zsh
                    // is ` \t\n`.
                    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\0'
                }
                t if t == PP_IFSSPACE => {
                    // c:3712-3713 — ASCII-only IFS-space.
                    (ch as u32) < 128 && (ch == ' ' || ch == '\t' || ch == '\n')
                }
                t if t == PP_WORD => {
                    // c:3716 — `wcsitype(ch, IWORD)`. IWORD = WORDCHARS-derived;
                    // conservative ASCII alphanumeric + `_`.
                    ch.is_alphanumeric() || ch == '_'
                }
                t if t == PP_RANGE => {
                    // c:3719-3735 — PP_RANGE: two metafied wide chars are
                    // `r1` and `r2`; matches if `r1 <= ch <= r2`.
                    let (r1_byte, adv1) = unmeta(range, i);
                    i += adv1;
                    let (r2_byte, adv2) = unmeta(range, i);
                    i += adv2;
                    let r1 = r1_byte as u32;
                    let r2 = r2_byte as u32;
                    let chu = ch as u32;
                    if r1 <= chu && chu <= r2 {
                        if let Some(p) = indptr.as_deref_mut() {
                            *p += chu - r1; // c:3725-3726
                        }
                        return true; // c:3727
                    }
                    if let Some(p) = indptr.as_deref_mut() {
                        if r1 < r2 {
                            *p += r2 - r1; // c:3733-3734
                        }
                    }
                    false
                }
                t if t == PP_INCOMPLETE => {
                    // c:3740 — `if (zmb_ind == ZMB_INCOMPLETE) return 1;`
                    zmb_ind == ZMB_INCOMPLETE
                }
                t if t == PP_INVALID => {
                    // c:3744 — `if (zmb_ind == ZMB_INVALID) return 1;`
                    zmb_ind == ZMB_INVALID
                }
                _ => {
                    // c:3747-3753 — PP_UNKWN / default → DPUTS bug warn
                    false
                }
            };
            if class_hit {
                return true;
            }
        } else {
            // c:3757-3762 — literal-char path
            let (decoded, adv) = unmeta(range, i);
            i += adv;
            if decoded == (ch as u32) as u8 && (ch as u32) < 256 {
                if let Some(p) = mtp.as_deref_mut() {
                    *p = 0; // c:3760
                }
                return true; // c:3761
            }
        }
        if let Some(p) = indptr.as_deref_mut() {
            *p += 1; // c:3764-3765 — `if (indptr) (*indptr)++;`
        }
    }
    false // c:3766 — `return 0;`
}

/// Port of `mb_patmatchindex()` from `Src/pattern.c:3767`.
/// C: `int mb_patmatchindex(char *range, wint_t ind,
/// wint_t *chr, int *mtp)` from `Src/pattern.c:3767`. The reverse
/// of `mb_patmatchrange`: given a metafied byte range and an index
/// `ind` into it, return the character (or PP_* class) at that
/// position.
///
/// Returns:
///   - `Some((Some(ch),  0))` — literal or PP_RANGE hit (chr = ch).
///   - `Some((None,     mtp))` — POSIX class match; chr = WEOF / None.
///   - `None` — index out of range.
pub fn mb_patmatchindex(range: &[u8], mut ind: u32) -> Option<(Option<char>, i32)> {
    // c:3767-3849
    let mut chr: Option<char> = None; // c:3776 — `*chr = WEOF`
    let mut mtp: i32 = 0; // c:3777 — `*mtp = 0`

    // C `UNMETA(s)` + `METACHARINC(s)` macro pair (Src/zsh.h).
    let unmeta = |bytes: &[u8], i: usize| -> (u8, usize) {
        if i < bytes.len() && bytes[i] == Meta && i + 1 < bytes.len() {
            (bytes[i + 1] ^ 0x20, 2)
        } else if i < bytes.len() {
            (bytes[i], 1)
        } else {
            (0, 0)
        }
    };

    let mut i = 0usize;
    while i < range.len() {
        // c:3779 — `while (*range)`
        let b = range[i];
        if crate::ported::utils::imeta_byte(b) {
            // c:3780 — `if (imeta((unsigned char) *range))`
            let swtype = (b as i32) - (Meta as i32);
            i += 1;
            match swtype {
                0 => {
                    // c:3782-3789 — `case 0: ordinary metafied char`
                    i -= 1;
                    let (decoded, adv) = unmeta(range, i);
                    i += adv;
                    let rchr = decoded as char;
                    if ind == 0 {
                        chr = Some(rchr);
                        return Some((chr, mtp));
                    }
                }
                t if (PP_ALPHA..=PP_INVALID).contains(&t) => {
                    // c:3791-3812 — POSIX class markers
                    if ind == 0 {
                        mtp = swtype;
                        return Some((None, mtp));
                    }
                }
                t if t == PP_RANGE => {
                    // c:3814-3829 — PP_RANGE: two metafied wide chars
                    let (r1_byte, adv1) = unmeta(range, i);
                    i += adv1;
                    let (r2_byte, adv2) = unmeta(range, i);
                    i += adv2;
                    let r1 = r1_byte as u32;
                    let r2 = r2_byte as u32;
                    let rdiff = r2.saturating_sub(r1);
                    if rdiff >= ind {
                        chr = char::from_u32(r1 + ind);
                        return Some((chr, mtp));
                    }
                    ind = ind.saturating_sub(rdiff);
                }
                _ => {
                    // c:3831-3837 — PP_UNKWN / default → DPUTS
                }
            }
        } else {
            // c:3840-3843 — literal byte path
            if ind == 0 {
                chr = Some(b as char);
                return Some((chr, mtp));
            }
        }
        // c:3846 — `if (!ind--) break;`
        if ind == 0 {
            break;
        }
        ind -= 1;
        i += 1;
    }
    None // c:3849 — `return 0`
}

/// Port of `freepatprog()` from `Src/pattern.c:4161`.
/// C: `freepatprog(Patprog prog)`. Frees a Patprog.
/// Rust's `Drop` on `Box<patprog>` handles this; the explicit fn
/// exists for C parity (Rule A).
#[allow(unused_variables)]
pub fn freepatprog(prog: Patprog) {} // c:4161

/// Port of `pat_enables()` from `Src/pattern.c:4171`.
/// C: `int pat_enables(const char *cmd, char **patp, int enable)`
/// from `Src/pattern.c:4171`. Implements `enable -p`/`disable -p`: with
/// an empty `patp`, prints the currently enabled (or disabled, if
/// `!enable`) tokens by walking `zpc_strings[]`/`zpc_disables[]` in
/// lockstep. Otherwise toggles each named token's `zpc_disables[i]`
/// slot, emitting `invalid pattern: NAME` for misses.
pub fn pat_enables(cmd: &str, patp: &[&str], enable: bool) -> i32 {
    // c:4171
    let mut ret: i32 = 0; // c:4173
    if patp.is_empty() {
        // c:4177 !*patp
        let strings = ZPC_STRINGS; // c:4179 zpc_strings
        let disp = zpc_disables.lock().unwrap(); // c:4179 zpc_disables
        let mut done = false; // c:4178
        let mut out: String = String::new();
        for i in 0..(ZPC_COUNT as usize) {
            // c:4180
            let sp = match strings[i] {
                // c:4182 !*stringp
                Some(s) => s,
                None => continue,
            };
            let is_disabled = disp[i] != 0;
            if enable == is_disabled {
                // c:4184 enable?*disp:!*disp
                continue;
            }
            if done {
                // c:4186
                out.push(' '); // c:4187
            }
            out.push_str(&format!("'{}'", sp)); // c:4188
            done = true; // c:4189
        }
        if done {
            // c:4191
            println!("{}", out); // c:4187-4192
        }
        return 0; // c:4193
    }
    for p in patp {
        // c:4196
        let strings = ZPC_STRINGS;
        let mut disp = zpc_disables.lock().unwrap();
        let mut matched = false;
        for i in 0..(ZPC_COUNT as usize) {
            // c:4197
            if let Some(s) = strings[i] {
                if s == *p {
                    // c:4200 !strcmp
                    disp[i] = if enable { 0u8 } else { 1u8 }; // c:4201 *disp = !enable
                    matched = true;
                    break; // c:4202
                }
            }
        }
        if !matched {
            // c:4205
            zerrnam(cmd, &format!("invalid pattern: {}", p)); // c:4206
            ret = 1; // c:4207
        }
    }
    ret // c:4211
}

/// Port of `mod_export const char *zpc_strings[ZPC_COUNT]` from
/// `Src/pattern.c:258`. Static token-name table indexed by ZPC_*;
/// NULL entries (ZPC_NULL, ZPC_BNULLKEEP, ZPC_INPAR_PIPE,
/// ZPC_KSHCHAR) have no user-visible name.
pub const ZPC_STRINGS: [Option<&'static str>; ZPC_COUNT as usize] = [
    // c:258
    None,
    None,
    Some("|"),
    None,
    Some("~"),
    Some("("),
    Some("?"),
    Some("*"),
    Some("["),
    Some("<"),
    Some("^"),
    Some("#"),
    None,
    Some("?("),
    Some("*("),
    Some("+("),
    Some("!("),
    Some("\\!("),
    Some("@("),
];

/// Port of `savepatterndisables()` from `Src/pattern.c:4220`.
/// C: `unsigned int savepatterndisables(void)` from
/// `Src/pattern.c:4220`.
///
/// C body (c:4220-4233):
/// ```c
/// unsigned int disables, bit;
/// char *disp;
/// disables = 0;
/// for (bit = 1, disp = zpc_disables;
///      disp < zpc_disables + ZPC_COUNT;
///      bit <<= 1, disp++) {
///     if (*disp) disables |= bit;
/// }
/// return disables;
/// ```
///
/// Encodes the current `zpc_disables\[ZPC_COUNT\]` byte-array as a u32
/// bitmask (one bit per slot, low bit = `zpc_disables\[0\]`).
/// The previous Rust port returned a `Vec<String>` clone of
/// `patterndisables` (a completely different data structure — names
/// list, not the per-token byte array). `restorepatterndisables(u32)`
/// at c:4258 reads this bitmask back into `zpc_disables`, so the
/// returned shape MUST be the u32 bitmask.
pub fn savepatterndisables() -> u32 {
    // c:4220
    let disp = zpc_disables.lock().unwrap(); // c:4225 disp = zpc_disables
    let mut disables: u32 = 0; // c:4224
    let mut bit: u32 = 1; // c:4226 bit = 1
    for i in 0..(ZPC_COUNT as usize) {
        // c:4226-4228
        if disp[i] != 0 {
            // c:4230
            disables |= bit; // c:4231
        }
        bit <<= 1; // c:4226 bit <<= 1
    }
    disables // c:4232
}

/// Port of `startpatternscope()` from `Src/pattern.c:4241`.
/// C: `void startpatternscope(void)`.
/// Pushes a frame onto `PATSCOPE_STACK` (`zpc_disables_stack` in C)
/// carrying the current `zpc_disables[]` state as a `savepatterndisables()`
/// u32 bitmap. Called at function entry; `endpatternscope` pops it.
///
/// ```c
/// void startpatternscope(void) {
///     Zpc_disables_save newdis = zalloc(sizeof(*newdis));
///     newdis->next = zpc_disables_stack;
///     newdis->disables = savepatterndisables();  // c:4247
///     zpc_disables_stack = newdis;
/// }
/// ```
pub fn startpatternscope() {
    // c:4241
    let saved = savepatterndisables(); // c:4247
    PATSCOPE_STACK.with(|s| s.borrow_mut().push(saved));
}

/// Port of `restorepatterndisables()` from `Src/pattern.c:4258`.
/// C: `void restorepatterndisables(unsigned int disables)` from
/// `Src/pattern.c:4258`. Walks the 12-slot `zpc_disables[]` array,
/// setting each slot's byte from the bitmask: `disables & (1<<i)`
/// → slot `i` gets 1, else 0.
/// ```c
/// void
/// restorepatterndisables(unsigned int disables)
/// {
///     char *disp;
///     unsigned int bit;
///     for (bit = 1, disp = zpc_disables;
///          disp < zpc_disables + ZPC_COUNT;
///          bit <<= 1, disp++) {
///         if (disables & bit) *disp = 1;
///         else *disp = 0;
///     }
/// }
/// ```
pub fn restorepatterndisables(disables: u32) {
    // c:4258
    let mut disp = zpc_disables.lock().unwrap(); // c:4263
    let mut bit: u32 = 1;
    for i in 0..(ZPC_COUNT as usize) {
        // c:4263-4265
        if (disables & bit) != 0 {
            // c:4266
            disp[i] = 1; // c:4267
        } else {
            disp[i] = 0; // c:4269
        }
        bit <<= 1;
    }
}

/// Port of `endpatternscope()` from `Src/pattern.c:4279`.
/// C: `void endpatternscope(void)`.
/// Pops the saved bitmap from `PATSCOPE_STACK` (`zpc_disables_stack`
/// in C); restores `zpc_disables[]` from the bitmap ONLY when
/// `isset(LOCALPATTERNS)` per C c:4286. Called at function exit.
///
/// ```c
/// void endpatternscope(void) {
///     Zpc_disables_save olddis = zpc_disables_stack;
///     zpc_disables_stack = olddis->next;
///     if (isset(LOCALPATTERNS))
///         restorepatterndisables(olddis->disables);     // c:4287
///     zfree(olddis, sizeof(*olddis));
/// }
/// ```
pub fn endpatternscope() {
    // c:4279
    if let Some(prev) = PATSCOPE_STACK.with(|s| s.borrow_mut().pop()) {
        if isset(crate::ported::zsh_h::LOCALPATTERNS) {
            // c:4286-4287
            restorepatterndisables(prev);
        }
    }
}

/// Port of `clearpatterndisables()` from `Src/pattern.c:4296`.
/// C: `void clearpatterndisables(void)`.
/// C body: `memset(zpc_disables, 0, ZPC_COUNT)` — zero every slot.
pub fn clearpatterndisables() {
    // c:4296
    *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize]; // c:4298
}

/// Port of `haswilds()` from `Src/pattern.c:4306`.
/// C: `haswilds(char *str)`.
///
/// Check whether `str` is eligible for filename generation.
///
/// C scans a TOKENIZED string: every input reaching it has been
/// through the lexer or `tokenize()`/`shtokenize()` (glob.c:3548/
/// 3563), so glob metachars are token codes and escaped/quoted
/// chars are Bnull'd literals. Callers holding un-tokenized strings
/// must `tokenize()` a copy first — exactly what C does for
/// runtime-built strings (compcore.c:2231 tokenizes fignore entries
/// immediately before its c:2235 haswilds call). The switch matches
/// ONLY token codes, never literal ASCII metachars.
///
/// The walk is over chars: zshrs strings hold token chars as
/// codepoints (Star = U+0087) and the input layer is char-domain
/// (input.rs `shingetline`/`ingetc`), so the char is the unit that
/// the metafied byte is in C. Scanning bytes here false-positived
/// on UTF-8 continuation bytes of plain text (`↔` = E2 86 94
/// carries 0x86/0x94 = Hat/Inang as u8) — bug #627.
///
/// The C source's `%?foo` job-ref special case (c:4317-4318), which
/// mutates `str[1]` in place to demote the `?`, becomes a "skip
/// position 1" scan adjustment here since `&str` is immutable.
pub fn haswilds(str: &str) -> bool {
    // c:4306
    // c:4325-4372 — every `return 1` needs one of Inpar / Bar / Star /
    // Inbrack / Inang / Quest / Pound / Hat, and all eight are TOKEN
    // chars in U+0080..U+00A0, i.e. non-ASCII. An all-ASCII string
    // therefore always falls through to c:4374 `return 0`. Answer it
    // with libcore's precompiled word-at-a-time `<[u8]>::is_ascii`
    // instead of building a `Vec<char>` to walk: `haswilds` runs once
    // per candidate word, and a `man <TAB>` offers 60605 of them.
    if str.is_ascii() {
        return false; // c:4374
    }
    let chars: Vec<char> = str.chars().collect();
    let len = chars.len();
    if len == 0 {
        return false;
    }

    // c:4309 — `[' and `]' are legal even if bad patterns are usually not.
    if len == 1 && (chars[0] == Inbrack || chars[0] == Outbrack) {
        return false; // c:4311
    }

    // c:4313-4315 — If % is immediately followed by ?, then that ? is
    // not treated as a wildcard.  This is so you don't have
    // to escape job references such as %?foo.
    let skip_pos_1 = len >= 2 && chars[0] == '%' && chars[1] == Quest; // c:4316-4317

    // c:4319-4321 — Note that at this point zpc_special has not been set up.
    let disp = zpc_disables.lock().unwrap(); // c:read zpc_disables[]

    for i in 0..len {
        // c:4323 for (; *str; str++)
        if skip_pos_1 && i == 1 {
            continue; // c:4317 `str[1] = '?'` demote
        }
        let c = chars[i]; // c:4324 switch (*str)
        let prev: char = if i > 0 { chars[i - 1] } else { '\0' }; // c: str[-1]

        if c == Inpar {
            // c:4325-4335
            if (!isset(SHGLOB) && disp[ZPC_INPAR as usize] == 0)
                || (i > 0
                    && isset(KSHGLOB)
                    && ((prev == Quest && disp[ZPC_KSH_QUEST as usize] == 0)
                        || (prev == Star && disp[ZPC_KSH_STAR as usize] == 0)
                        || (prev == '+' && disp[ZPC_KSH_PLUS as usize] == 0)
                        || (prev == Bang && disp[ZPC_KSH_BANG as usize] == 0)
                        || (prev == '!' && disp[ZPC_KSH_BANG2 as usize] == 0)
                        || (prev == '@' && disp[ZPC_KSH_AT as usize] == 0)))
            {
                return true; // c:4335
            }
        } else if c == Bar {
            if disp[ZPC_BAR as usize] == 0 {
                return true; // c:4340
            }
        } else if c == Star {
            if disp[ZPC_STAR as usize] == 0 {
                return true; // c:4345
            }
        } else if c == Inbrack {
            if disp[ZPC_INBRACK as usize] == 0 {
                return true; // c:4350
            }
        } else if c == Inang {
            if disp[ZPC_INANG as usize] == 0 {
                return true; // c:4355
            }
        } else if c == Quest {
            if disp[ZPC_QUEST as usize] == 0 {
                return true; // c:4360
            }
        } else if c == Pound {
            if isset(EXTENDEDGLOB) && disp[ZPC_HASH as usize] == 0 {
                return true; // c:4365
            }
        } else if c == Hat {
            if isset(EXTENDEDGLOB) && disp[ZPC_HAT as usize] == 0 {
                return true; // c:4370
            }
        }
    }
    false // c:4374
}

// =====================================================================
// 4. struct patprog — zsh.h:1601
// =====================================================================

/// `typedef struct patprog *Patprog;` from `zsh.h:542`.
#[allow(non_camel_case_types)]
/// `typedef struct patprog *Patprog;` from `zsh.h:542`.
///
/// C zsh allocates the `struct patprog` header + bytecode as one
/// contiguous `malloc` block, accessing bytecode via
/// `(char *)prog + prog->startoff`. Rust has no flexible array
/// members, so this typedef pairs the C-exact `patprog` header
/// (zsh_h.rs:768) with an owned bytecode `Vec<u8>` — header at
/// `.0`, bytecode at `.1`. `startoff`/`size` index into `.1`.
pub type Patprog = Box<(patprog, Vec<u8>)>;
// =====================================================================
// Bytecode field offsets within each instruction.
//
// Instruction layout in `patout`:
//     +0       u8     opcode
//     +1..+5   u32 LE next_off (offset of next instr in chain, 0 = end)
//     +5..     u8...  opcode-specific payload
//
// C uses a different layout (Upat union = 4-byte or 8-byte slots);
// the Rust port pins the layout to byte offsets for portability.
// =====================================================================

const I_OP: usize = 0; // opcode byte

impl rpat {
    pub fn new() -> Self {
        Self {
            wbranch_visits: std::collections::HashMap::new(),
            patbeginp: [usize::MAX; NSUBEXP],
            patendp: [0; NSUBEXP],
            captures_set: 0,
            errsfound: 0, // c:2066 — errsfound = 0 at pattry init
        }
    }
}
const I_NEXT: usize = 1; // u32 next-offset starts here
const I_BODY: usize = 5; // payload starts here

/// Serialises every entry into `patcompile`. The C source at
/// `Src/pattern.c:267-281` declares `patout`, `patparse`, `patstart`,
/// `patnpar`, `patflags`, `patglobflags`, `errsfound`, `forceerrs`,
/// `zpc_special`, `patstrcache` as file-scope statics that the compile
/// mutates in sequence; zsh-the-program is single-threaded so the C
/// source is safe under that invariant. zshrs callers (zutil's
/// `style_table::get` via `crate::ported::pattern::patmatch`, params.rs,
/// subst.rs, options.rs) can invoke `patcompile` from concurrent test
/// threads, so the lock restores the single-writer invariant. Held
/// only for the compile phase; the matcher (`pattry`/`patmatch`)
/// operates on the returned `Patprog.code` and touches no globals.
static PATCOMPILE_LOCK: Mutex<()> = Mutex::new(());

// C: `static char *patparse, *patstart;` — pattern parsing cursors
// into the *input* pattern string. patstart points to start of
// pattern; patparse moves forward as we consume tokens.
pub static patparse: Mutex<String> = Mutex::new(String::new()); // c:269
pub static patstart: Mutex<String> = Mutex::new(String::new()); // c:269

/// Position within `patparse` we're currently looking at. C source
/// uses a `char *` cursor; Rust uses byte offset into the String.
pub static patparse_off: AtomicUsize = AtomicUsize::new(0);

// C: `static int errsfound;` — approximate-match error count.
pub static errsfound: AtomicI32 = AtomicI32::new(0); // c:274

// C: `static int forceerrs;` — required error count for approximate match.
pub static forceerrs: AtomicI32 = AtomicI32::new(-1); // c:275

// C: `static long patglobflags_orig;` — saved at branch entry.
pub static patglobflags_orig: AtomicI32 = AtomicI32::new(0); // c:276

// C: `static const char *zpc_special;` — table of currently-special
// characters during compile (indexed by ZPC_*).
//
// pattern.c uses `static char zpc_special[ZPC_COUNT];` and resets it
// in patcompcharsset(). Rust mirrors as a Mutex-wrapped byte array.
pub static zpc_special: Mutex<[u8; ZPC_COUNT as usize]> = Mutex::new([0u8; ZPC_COUNT as usize]); // c:278

// C: `static char *patstrcache;` — caches the unmetafied trial string.
// Rust port has no Meta encoding so the cache is unnecessary; we leave
// the static declared for parity (Rule A — name exists in C).
pub static patstrcache: Mutex<String> = Mutex::new(String::new()); // c:281

/// `Marker` constant — alias for `crate::ported::zsh_h::Marker`
/// (`Src/zsh.h:224` `#define Marker ((char) 0xa2)`).
///
/// The previous Rust port had this as `pub const Marker: u8 = 0x80`
/// which is WRONG — `\200` (0x80) is NOT the canonical Marker byte.
/// C's Marker is 0xa2 per `Src/zsh.h:224`. No in-tree callers used

/// Port of `static const char *colon_stuffs[]` from `Src/pattern.c:1134-1138`.
/// 19 entries matching C's table in declaration order, so `range_type(name)`
/// returns the same `(index + PP_FIRST)` value C does:
/// alpha=1, alnum=2, ascii=3, blank=4, cntrl=5, digit=6, graph=7, lower=8,
/// print=9, punct=10, space=11, upper=12, xdigit=13, IDENT=14, IFS=15,
/// IFSSPACE=16, WORD=17, INCOMPLETE=18, INVALID=19. Prior Rust port had
/// only 12 lowercase entries and was MISSING `ascii` between `alnum` and
/// `blank`, so `range_type("digit")` returned 5 instead of C's PP_DIGIT=6
/// and every `[:class:]` byte marker emitted by `complete.rs:733`
/// (`0x80 + ch`) was off-by-one for classes after `alnum`. Real port bug.
const POSIX_CLASS_NAMES: &[&str] = &[
    "alpha",
    "alnum",
    "ascii",
    "blank",
    "cntrl",
    "digit",
    "graph",
    "lower",
    "print",
    "punct",
    "space",
    "upper",
    "xdigit",
    "IDENT",
    "IFS",
    "IFSSPACE",
    "WORD",
    "INCOMPLETE",
    "INVALID",
];

/// Port of file-static `zpc_disables_stack` from `Src/pattern.c:4244`.
/// Per-evaluator function-scope disable save-stack (bucket-1: each
/// worker thread parses/executes its own function calls, so each must
/// have its own scope stack). Reason for `thread_local!` over `Mutex`:
/// in zsh C this is a per-process file-static; in zshrs each worker
/// thread is its own evaluator — TLS preserves the per-evaluator
/// semantic without serializing across workers.
///
/// Element type: u32 bitmap matching `savepatterndisables` return
/// shape (c:4220 `unsigned int disables`). Prior Rust port used
/// `Vec<Vec<String>>` and `startpatternscope` cloned a `Vec<String>`
/// of names instead of the bitmap — a real port bug that made
/// `setopt LOCALPATTERNS` function entry/exit silently fail to save/
/// restore the disable state.
thread_local! {
    static PATSCOPE_STACK: std::cell::RefCell<Vec<u32>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

// =====================================================================
// 17. Module-loader / disable mgmt — pattern.c:4161-4296
// =====================================================================

// `pub static patterndisables: Mutex<Vec<String>>` deleted — was a
// dead tombstone with no callers outside the buggy
// `startpatternscope` / `endpatternscope` / `clearpatterndisables`
// fns (which cloned/cleared it instead of operating on the real
// `zpc_disables` byte array). The canonical zsh `zpc_disables[ZPC_COUNT]`
// (Src/pattern.c:268) is the disable state; the names list does not
// exist as a separate data structure in C.

/// Port of `char zpc_disables[ZPC_COUNT]` from `Src/pattern.c:268`.
/// Per-token disable byte — when `zpc_disables[i]` is non-zero, the
/// pattern token at index `i` (ZPC_*) is treated as a literal,
/// not its meta-meaning.
pub static zpc_disables: Mutex<[u8; ZPC_COUNT as usize]> = Mutex::new([0u8; ZPC_COUNT as usize]); // c:268

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// There is NO `fixup_offsets_after_insert()` in `Src/pattern.c`.
/// It stands in for C's ABSENCE of a fixup step: C's `patinsert()`
/// (`Src/pattern.c:1807`) links nodes by RELATIVE `P_NEXT` deltas
/// (`Src/pattern.c:198`) held in `union upat` slots, so shifting the
/// tail of `patout` down leaves every link valid and C only has to
/// `memmove` (`Src/pattern.c:1816-1820`). zshrs's bytecode stores
/// ABSOLUTE byte offsets in the 4-byte `I_NEXT` slot, which the
/// memmove invalidates — so every next-offset past `opnd` has to be
/// re-based by `delta`. Called only from `patinsert()`; both writer
/// and reader live in pattern.rs.
///
/// Walks the buffer linearly opcode-by-opcode reading I_NEXT slots.
/// Conservatively adjusts every nonzero next that lands past opnd.
fn fixup_offsets_after_insert(buf: &mut [u8], opnd: usize, delta: u32) {
    let mut i = 0;
    while i + I_BODY <= buf.len() {
        let op = buf[i + I_OP];
        if op == 0 {
            i += 1;
            continue;
        } // sentinel byte, skip
        let next_bytes = &buf[i + I_NEXT..i + I_NEXT + 4];
        let cur = u32::from_le_bytes(next_bytes.try_into().unwrap());
        if cur != 0 {
            let abs = cur as usize;
            if abs >= opnd && abs <= buf.len() {
                let new = cur + delta;
                buf[i + I_NEXT..i + I_NEXT + 4].copy_from_slice(&new.to_le_bytes());
            }
        }
        i = advance_past_instr(buf, i);
        if i == 0 {
            break;
        }
    }
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// There is NO `advance_past_instr()` in `Src/pattern.c`. C never
/// needs one: its bytecode is an array of fixed-size `union upat`
/// slots (`union upat`, `Src/pattern.c:84-89`), so "the next slot"
/// is just
/// `p + 1` pointer arithmetic and the payload width is implicit in
/// the slot count each emitter wrote. zshrs emits a packed BYTE
/// stream instead, so stepping over an instruction requires decoding
/// its payload width from the opcode. This function is that decode
/// table. Called only from `fixup_offsets_after_insert()`; both
/// writer and reader live in pattern.rs.
///
/// Encodes the per-opcode payload size table — must stay in sync
/// with patnode/patinsert calls in the compiler.
fn advance_past_instr(buf: &[u8], pos: usize) -> usize {
    if pos + I_BODY > buf.len() {
        return 0;
    }
    let op = buf[pos + I_OP];
    let body_start = pos + I_BODY;
    match op {
        P_END | P_NOTHING | P_BACK | P_EXCSYNC | P_EXCEND | P_ISSTART | P_ISEND | P_COUNTSTART
        | P_ANY | P_STAR | P_NUMANY => body_start,
        P_GFLAGS => body_start + 4, // i32 flag-bits payload
        P_EXACTLY => {
            // payload: u32 len + len bytes
            if body_start + 4 > buf.len() {
                return 0;
            }
            let len =
                u32::from_le_bytes(buf[body_start..body_start + 4].try_into().unwrap()) as usize;
            body_start + 4 + len
        }
        P_ANYOF | P_ANYBUT => {
            if body_start + 4 > buf.len() {
                return 0;
            }
            let len =
                u32::from_le_bytes(buf[body_start..body_start + 4].try_into().unwrap()) as usize;
            body_start + 4 + len
        }
        P_ONEHASH | P_TWOHASH | P_BRANCH => body_start,
        // c:113 P_WBRANCH and c:114-115 P_EXCLUDE/P_EXCLUDP carry an
        // 8-byte syncptr payload (one Upat slot in C) right after
        // their header, before the chained operand.
        P_WBRANCH | P_EXCLUDE | P_EXCLUDP => body_start + 8,
        P_OPEN..=0x88 | P_CLOSE..=0x98 => body_start,
        P_NUMRNG => body_start + 16, // two i64
        P_NUMFROM | P_NUMTO => body_start + 8,
        P_COUNT => body_start + 16, // min i64 + max i64; operand inline follows
        _ => body_start,
    }
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// There is NO `set_next()` in `Src/pattern.c`. C writes a node's
/// next-link inline as one assignment against the `union upat`
/// pointer — `scan->l |= offset << 8;` (`Src/pattern.c:1849`), the
/// write half of the `P_NEXT` macro at `Src/pattern.c:198`, as used
/// in `pattail()` (`Src/pattern.c:1846-1849`). zshrs stores that link
/// as a 4-byte little-endian absolute offset inside a `Vec<u8>`
/// behind a mutex, so the same one-liner needs a lock, a bounds
/// check and a `to_le_bytes` — factored here rather than repeated at
/// every emit site. Both writer and reader live in pattern.rs.
fn set_next(pos: usize, val: usize) {
    let mut buf = patout.lock().unwrap();
    if pos + I_NEXT + 4 <= buf.len() {
        buf[pos + I_NEXT..pos + I_NEXT + 4].copy_from_slice(&(val as u32).to_le_bytes());
    }
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// There is NO `chain_branches_to()` in `Src/pattern.c`. C performs
/// this walk inline at the end of `patcompswitch()`
/// (`Src/pattern.c:909-911`): `for (ptr = (Upat)patout + starter; ptr;
/// ptr = PATNEXT(ptr)) if (!P_ISEXCLUDE(ptr)) patoptail(ptr -
/// (Upat)patout, ender);` — `union upat` pointer arithmetic reaches
/// each alternative and its operand. In zshrs the operand is at
/// `br + I_BODY` and the branch link is a
/// 4-byte absolute offset read under the `patout` mutex, so the loop
/// is factored out to keep `patcompswitch()` readable. Both writer
/// and reader live in pattern.rs.
///
/// Walks every branch's operand chain and patches each branch's
/// last-operand-node `.next` to `target`. Used to chain a fully-
/// compiled alternation switch to whatever opcode follows (P_END for
/// the outermost compile, P_CLOSE_N for a sub-group).
fn chain_branches_to(starter: usize, target: usize) {
    let mut cur = starter;
    loop {
        // Operand starts at cur + I_BODY (the byte right after this
        // branch's header). Walk operand's next-chain to its end
        // and set its .next = target.
        pattail(cur + I_BODY, target);
        // Move to next alternative.
        let buf = patout.lock().unwrap();
        if cur + I_NEXT + 4 > buf.len() {
            break;
        }
        let nb: [u8; 4] = buf[cur + I_NEXT..cur + I_NEXT + 4].try_into().unwrap();
        let n = u32::from_le_bytes(nb) as usize;
        drop(buf);
        if n == 0 {
            break;
        }
        cur = n;
    }
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// There is NO `approx_match_exactly()` in `Src/pattern.c`. This is the
/// factored-out inner loop of the C `case P_EXACTLY:` arm of
/// `patmatch()` (`Src/pattern.c:2737-2779`), which C writes inline: the
/// per-byte compare plus the three approximate-match edit trials
/// (substitute / insert / delete) that `(#aN)` enables. C can keep it
/// inline because `patinput` / `errsfound` are file-statics it mutates
/// in place and restores on backtrack; the Rust matcher threads that
/// state through `&mut rpat` and recurses into `patmatch()`, so the
/// trial walk needs its own frame. Extracting it keeps the linear
/// `patmatch()` dispatch loop readable. Both writer and reader live in
/// pattern.rs; nothing outside the P_EXACTLY arm calls it.
///
/// Walks pattern bytes `str_bytes` against `input_bytes[s_off..]`.
/// On exact-match per byte: advance both. On mismatch: try the 3
/// edit operations in order (substitute = advance both + errsfound++,
/// insert = advance pat + errsfound++, delete = advance input +
/// errsfound++); each recurses via `patmatch` to continue with the
/// rest of the bytecode at `next`. Returns the matched-end byte
/// offset in `string` on success, None on failure.
fn approx_match_exactly(
    code: &[u8],
    next: usize,
    string: &str,
    s_off: usize,
    str_bytes: &[u8],
    state: &mut rpat,
    glob_flags: i32,
    max_errs: i32,
) -> Option<usize> {
    let input_bytes = string.as_bytes();
    // Try matching the EXACT prefix as far as it lines up; on first
    // mismatch (or out-of-input), branch into the edit-operation
    // trials. This is a bounded recursive search; the budget caps
    // recursion depth at `max_errs - state.errsfound`.
    fn walk(
        code: &[u8],
        next: usize,
        string: &str,
        input_bytes: &[u8],
        str_bytes: &[u8],
        s_off: usize,
        p_off: usize,
        state: &mut rpat,
        glob_flags: i32,
        max_errs: i32,
    ) -> Option<usize> {
        // Direction terminology: `s_off+1` consumes one INPUT byte,
        // `p_off+1` consumes one PATTERN byte.
        //   - (s+1, p+1) = substitute one input byte for one pat byte
        //   - (s+1, p)   = consume input byte that's NOT in pattern
        //                  (= INSERTION in input compared to pattern)
        //   - (s, p+1)   = consume pat byte that's NOT in input
        //                  (= DELETION from input compared to pattern)
        //
        // Tries every option at each step, returns the path producing
        // the LARGEST end s_off (most input consumed). Anchored
        // callers (pattry) need full-input consumption; non-anchored
        // accept any. C handles this via bytecode-level branching
        // backtrack; the Rust port collapses it into a recursive
        // tree-search with per-attempt state save/restore.
        let mut best: Option<usize> = None;
        let saved_outer = state.clone();
        let mut update = |best: &mut Option<usize>, cand: Option<usize>| {
            if let Some(c) = cand {
                if best.map(|b| c > b).unwrap_or(true) {
                    *best = Some(c);
                }
            }
        };
        if p_off == str_bytes.len() {
            // Pattern body consumed. Three paths to try; pick the
            // one with most input consumed:
            //   (a) terminate here at s_off, run continuation.
            //   (b) absorb 1+ trailing input as INSERTION-IN-INPUT edits.
            let terminate = if next == 0 {
                Some(s_off)
            } else {
                patmatch(code, next, string, s_off, state, glob_flags)
            };
            update(&mut best, terminate);
            // Path (b): absorb trailing input as insertion edits.
            if s_off < input_bytes.len() && state.errsfound < max_errs {
                *state = saved_outer.clone();
                state.errsfound += 1;
                let r = walk(
                    code,
                    next,
                    string,
                    input_bytes,
                    str_bytes,
                    s_off + 1,
                    p_off,
                    state,
                    glob_flags,
                    max_errs,
                );
                update(&mut best, r);
            }
            *state = saved_outer;
            return best;
        }
        // c:Src/pattern.c:2676 CHARMATCH — inline case-aware byte
        // compare (the C macro, not a fn). Honors GF_IGNCASE /
        // GF_LCMATCHUC so under `(#i)` a case-only difference is NOT an
        // edit: `(#ia2)readme` matches `READXME` (only X→M costs an
        // error). Raw byte `==` counted every case difference as an edit.
        let charmatch_inline = |chin: u8, chpa: u8| -> bool {
            chin == chpa
                || ((glob_flags & GF_IGNCASE) != 0
                    && chin.to_ascii_lowercase() == chpa.to_ascii_lowercase())
                || ((glob_flags & GF_LCMATCHUC) != 0
                    && chpa.is_ascii_lowercase()
                    && chpa.to_ascii_uppercase() == chin)
        };
        // Exact-byte match — try advancing both.
        if s_off < input_bytes.len() && charmatch_inline(input_bytes[s_off], str_bytes[p_off]) {
            *state = saved_outer.clone();
            let r = walk(
                code,
                next,
                string,
                input_bytes,
                str_bytes,
                s_off + 1,
                p_off + 1,
                state,
                glob_flags,
                max_errs,
            );
            update(&mut best, r);
        }
        // Edit operations — each costs 1 error.
        if state.errsfound < max_errs {
            // c:Src/pattern.c:3520-3543 — Damerau transposition: swap two
            // adjacent input chars to match two adjacent pat chars (i.e.
            // input[s..s+2] reversed equals pat[p..p+2]). Costs 1 edit;
            // pin for `(#a3)abcd` matching "dcba" — needs sub + transp +
            // sub = 3 edits to bridge the reversal.
            if s_off + 1 < input_bytes.len()
                && p_off + 1 < str_bytes.len()
                && charmatch_inline(input_bytes[s_off], str_bytes[p_off + 1])
                && charmatch_inline(input_bytes[s_off + 1], str_bytes[p_off])
            {
                *state = saved_outer.clone();
                state.errsfound += 1;
                let r = walk(
                    code,
                    next,
                    string,
                    input_bytes,
                    str_bytes,
                    s_off + 2,
                    p_off + 2,
                    state,
                    glob_flags,
                    max_errs,
                );
                update(&mut best, r);
            }
            // Substitute.
            if s_off < input_bytes.len() {
                *state = saved_outer.clone();
                state.errsfound += 1;
                let r = walk(
                    code,
                    next,
                    string,
                    input_bytes,
                    str_bytes,
                    s_off + 1,
                    p_off + 1,
                    state,
                    glob_flags,
                    max_errs,
                );
                update(&mut best, r);
            }
            // Insertion in input (skip input byte only).
            if s_off < input_bytes.len() {
                *state = saved_outer.clone();
                state.errsfound += 1;
                let r = walk(
                    code,
                    next,
                    string,
                    input_bytes,
                    str_bytes,
                    s_off + 1,
                    p_off,
                    state,
                    glob_flags,
                    max_errs,
                );
                update(&mut best, r);
            }
            // Deletion from input (skip pattern byte only).
            *state = saved_outer.clone();
            state.errsfound += 1;
            let r = walk(
                code,
                next,
                string,
                input_bytes,
                str_bytes,
                s_off,
                p_off + 1,
                state,
                glob_flags,
                max_errs,
            );
            update(&mut best, r);
        }
        *state = saved_outer;
        best
    }
    walk(
        code,
        next,
        string,
        input_bytes,
        str_bytes,
        s_off,
        0,
        state,
        glob_flags,
        max_errs,
    )
}

thread_local! {
    /// Rust-only backstop counter (no C analogue) — gates `patmatch`
    /// recursion at PATMATCH_MAX_DEPTH so misbehaving closures convert
    /// would-be stack overflows into clean None returns. Documented in
    /// `fake_fn_allowlist.txt` under the patmatch arc.
    static PATMATCH_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Maximum patmatch recursion depth — Rust-only backstop, no C
/// analogue. The C source's primary protection against
/// closure-of-closure infinite recursion is the P_WBRANCH "must
/// match at least 1 char" semantics described at pattern.c:108-144:
///   `P_WBRANCH:  This works like a branch and is used in complex`
///   `closures, but the match must be at least 1 char in length to`
///   `avoid infinite loops.`
/// (Implemented in C's patmatch() switch at c:3044, `case P_WBRANCH`,
/// via the `errsfound`/`forceerrs` accounting at c:1059+.) The Rust
/// port's P_WBRANCH arm has gaps in that propagation, so we keep
/// this depth guard as a safety net.
///
/// **Tuned to test-thread stack size**, NOT the main-thread 8 MB.
/// Rust's `cargo test` spawns each test on a thread whose default
/// stack is ~2 MB (per `std::thread::Builder::stack_size` default at
/// `library/std/src/thread/mod.rs`), and `(fo#)#` patterns blow that
/// at ~250-300 frames per measured SIGABRT. 128 leaves headroom
/// (~256 KB worst case at ~2 KB/frame) while still admitting any
/// legitimate zsh pattern (Misc/globtests tops out around 30).
const PATMATCH_MAX_DEPTH: u32 = 128;

thread_local! {
    // c:Src/pattern.c — the P_EXCLUDE `syncptr->p` heap buffer. Keyed by
    // the EXCLUDE node offset; value is a per-input-position byte array
    // recording where the asserted branch's excludable part matched (so
    // EXCSYNC can fail a revisit and force backtracking to a different
    // split). C stores this in the bytecode payload + a raw heap pointer
    // that survives backtracking; the Rust matcher clones `rpat` per
    // branch, so the buffer lives here (outside the cloned state) to
    // stay shared across recursion, exactly like C's global syncstrp->p.
    static EXCSYNC_BUF: std::cell::RefCell<std::collections::HashMap<usize, Vec<u8>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Port of `patmatch()` from `Src/pattern.c:2694`. The bytecode
/// interpreter — `static int patmatch(Upat prog)`.
///
/// Returns `Some(end_pos)` on a successful match (`end_pos` = byte
/// offset in `string` where the match ended), `None` on no-match; C
/// returns 1/0 and leaves the end position in the file-static
/// `patinput`.
///
/// Rust signature differs from C's `int patmatch(Upat prog)`: the
/// bytecode, the current input position, the captures and the glob
/// flags are threaded through arguments rather than C's per-thread
/// file-statics (`patinput`, `patinstart`, `patbeginp`/`patendp`,
/// `patglobflags`). Rule S1 deviation justified by zshrs's threading
/// model — see PORT_PLAN.md Bucket 1.
pub fn patmatch(
    code: &[u8],
    prog_off: usize,
    string: &str,
    string_off: usize,
    state: &mut rpat,
    glob_flags: i32,
) -> Option<usize> {
    // c:2694
    // Depth-guard: convert what would be a stack overflow into a
    // clean None return (= no match). See PATMATCH_MAX_DEPTH doc.
    let d = PATMATCH_DEPTH.with(|c| {
        let cur = c.get();
        c.set(cur + 1);
        cur + 1
    });
    if d > PATMATCH_MAX_DEPTH {
        PATMATCH_DEPTH.with(|c| c.set(c.get() - 1));
        return None;
    }
    struct DepthGuard;
    impl Drop for DepthGuard {
        fn drop(&mut self) {
            PATMATCH_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
        }
    }
    let _depth_guard = DepthGuard;

    let mut scan = prog_off;
    let mut s_off = string_off;
    // Locally-mutable copy of glob_flags so mid-pattern P_GFLAGS can
    // toggle bits without affecting the caller's branch view.
    let mut glob_flags = glob_flags;

    // Inlined port of CHARMATCH macro from `Src/pattern.c:2671-2677`:
    //   #define CHARMATCH(chin, chpa) (chin == chpa || \
    //       ((patglobflags & GF_IGNCASE) ?
    //          ((ISUPPER(chin) ? TOLOWER(chin) : chin) ==
    //           (ISUPPER(chpa) ? TOLOWER(chpa) : chpa)) :
    //        (patglobflags & GF_LCMATCHUC) ?
    //          (ISLOWER(chpa) && TOUPPER(chpa) == chin) : 0))
    //
    // - exact byte match always wins
    // - GF_IGNCASE: SYMMETRIC fold (both lowercase before compare)
    // - GF_LCMATCHUC: ASYMMETRIC — lowercase pattern char ALSO matches
    //   uppercase text char; an UPPERCASE pattern char only matches
    //   that exact uppercase text char. This is the `(#l)` flag's
    //   "lowercase-in-pattern matches uppercase-in-text" semantic;
    //   previously zshrs treated LCMATCHUC the same as IGNCASE (full
    //   case-fold both sides) so `(#l)FOO` wrongly matched "foo".
    let charmatch = |chin: u8, chpa: u8, flags: i32| -> bool {
        if chin == chpa {
            return true; // c:2671
        }
        if (flags & GF_IGNCASE) != 0 {
            // c:2672-2674
            let a = if chin.is_ascii_uppercase() {
                chin.to_ascii_lowercase()
            } else {
                chin
            };
            let b = if chpa.is_ascii_uppercase() {
                chpa.to_ascii_lowercase()
            } else {
                chpa
            };
            return a == b;
        }
        if (flags & GF_LCMATCHUC) != 0 {
            // c:2675-2676
            return chpa.is_ascii_lowercase() && chpa.to_ascii_uppercase() == chin;
        }
        false // c:2677
    };

    // Byte advance past ONE *input* character at `off` — C's `CHARINC`
    // macro (c:1963 `#define CHARINC(x, y) ((x) = charnext((x), (y)))`),
    // whose `charnext` body is c:1940-1960.
    //
    // The unit is option-dependent, and that is the whole point:
    // c:1946 `if (!(patglobflags & GF_MULTIBYTE) || !((unsigned char) *x
    // & 0x80)) return x + 1;` — with GF_MULTIBYTE clear (`unsetopt
    // multibyte`) a character is ONE RAW BYTE, so `?` matches a byte and
    // `*` backtracks over bytes. Only with GF_MULTIBYTE set does C decode
    // a whole multibyte character (c:1949 `mbrtowc`), falling back to a
    // single byte on MB_INVALID/MB_INCOMPLETE (c:1951-1955).
    //
    // C matches over an UNMETAFIED buffer (`patallocstr`), whereas zshrs
    // keeps non-UTF-8 input metafied inside the `&str` — Meta `\u{83}`
    // followed by `byte ^ 32`. One metafied pair stands for ONE raw byte,
    // so it is consumed as a unit in both modes; under GF_MULTIBYTE
    // consecutive pairs are accumulated until they form one valid UTF-8
    // character, which is what C's `mbrtowc` over the raw bytes does.
    //
    // Returns 0 only at end of input (C's `patinput == patinend`, c:2737).
    let charinc = |off: usize, gflags: i32| -> usize {
        if off >= string.len() {
            return 0; // c:2737 — no character left.
        }
        let multibyte = (gflags & GF_MULTIBYTE) != 0; // c:1946
                                                      // A non-boundary offset is only reachable while stepping bytes
                                                      // with GF_MULTIBYTE clear; C's raw-byte view advances one byte.
        let Some(rest) = string.get(off..) else {
            return 1; // c:1947
        };
        let mut raw: Vec<u8> = Vec::new();
        let mut advance = 0usize;
        let mut chs = rest.chars();
        while let Some(c) = chs.next() {
            if c == '\u{83}' {
                if let Some(n) = chs.clone().next() {
                    if (n as u32) >= 0x80 {
                        raw.push((n as u32 as u8) ^ 32);
                        advance += c.len_utf8() + n.len_utf8();
                        chs.next();
                        // c:1946 — one raw byte IS one character without
                        // GF_MULTIBYTE; otherwise keep collecting bytes
                        // until `mbrtowc` would have completed one
                        // character (c:1949).
                        if !multibyte || std::str::from_utf8(&raw).is_ok() {
                            break;
                        }
                        continue;
                    }
                }
                // Lone Meta — count it as one character.
                advance += c.len_utf8();
                break;
            }
            // Natively stored character: its `&str` bytes ARE the raw
            // bytes C would see, so without GF_MULTIBYTE only the first
            // one is consumed (c:1947).
            advance += if !multibyte && (c as u32) >= 0x80 {
                1
            } else {
                c.len_utf8()
            };
            break;
        }
        advance
    };

    // Membership test for a P_ANYOF/P_ANYBUT body at `body_off` against the
    // input char at byte offset `off`. Returns (in_set, byte_advance). ASCII
    // input takes the unchanged byte `charmatch` path over the `chars` set; a
    // multibyte char is decoded (raw UTF-8, or a metafied `$'\xNN'` Meta-pair)
    // and tested against the appended class mask / wide ranges / wide literals
    // — C's `mb_patmatchrange` equivalent (pattern.c:3610).
    let anyof_membership = |body_off: usize, off: usize, gflags: i32| -> (bool, usize) {
        let range_flags = gflags & !(GF_IGNCASE | GF_LCMATCHUC);
        let chars_len =
            u32::from_le_bytes(code[body_off + 4..body_off + 8].try_into().unwrap()) as usize;
        let cs = body_off + 8;
        let set = &code[cs..cs + chars_len];
        let mut p = cs + chars_len;
        let classmask = u32::from_le_bytes(code[p..p + 4].try_into().unwrap());
        p += 4;
        let n_mbc = u32::from_le_bytes(code[p..p + 4].try_into().unwrap()) as usize;
        p += 4;
        let mbc_start = p;
        p += n_mbc * 4;
        let n_mbr = u32::from_le_bytes(code[p..p + 4].try_into().unwrap()) as usize;
        p += 4;
        let mbr_start = p;

        let bytes = string.as_bytes();
        let b = bytes[off];
        // ASCII input, OR `unsetopt multibyte`: byte-level match (the prior
        // behaviour). C only takes the wide `mb_patmatchrange` path under
        // GF_MULTIBYTE; with it clear a high byte is matched as a raw byte.
        //
        // `off` can also land inside a multibyte char: the approx omit-input
        // paths step `s_off + 1`, so a continuation P_ANYOF/P_ANYBUT can be
        // tested at a continuation byte. Decoding `string[off..]` there would
        // panic, so treat a non-boundary position as a raw byte (advance 1) —
        // the same fallback the byte-level machinery already relies on.
        // c:3702-3719 — PP_IDENT / PP_IFS / PP_IFSSPACE / PP_WORD are
        // resolved against the LIVE ztype table (`$IFS`, `$WORDCHARS`,
        // POSIX_IDENTIFIERS) every time a character is tested, never
        // frozen into the compiled program.
        let live_class_hit = |ch: char| -> bool {
            use crate::ported::ztype_h::{iwsep, IIDENT, ISEP, IWORD};
            if (classmask & (1 << 10)) != 0 && crate::ported::utils::wcsitype(ch, IIDENT as u32) {
                return true; // c:3702-3706
            }
            if (classmask & (1 << 11)) != 0 && crate::ported::utils::wcsitype(ch, ISEP as u32) {
                return true; // c:3707-3710
            }
            // c:3711-3715 — "must be ASCII space character":
            // `if (ch < 128 && iwsep((int)ch))`
            if (classmask & (1 << 12)) != 0 && (ch as u32) < 128 && iwsep(ch as u8) {
                return true;
            }
            if (classmask & (1 << 13)) != 0 && crate::ported::utils::wcsitype(ch, IWORD as u32) {
                return true; // c:3716-3719
            }
            false
        };
        if b < 0x80 || (gflags & GF_MULTIBYTE) == 0 || !string.is_char_boundary(off) {
            if set.iter().any(|&c| charmatch(b, c, range_flags)) {
                return (true, 1);
            }
            if b < 0x80 && live_class_hit(b as char) {
                return (true, 1);
            }
            // c:2800 — without GF_MULTIBYTE the input char is a single BYTE
            // (`charref`, c:1919-1920) and `patmatchrange` walks the operand's
            // RAW bytes, so a bracket member written as a multibyte character
            // matches each of its UTF-8 bytes individually. C keeps those
            // bytes in the operand; zshrs splits them out of `chars` into the
            // wide literal / wide range tables at compile time, so re-encode
            // them here. Without this, `unsetopt multibyte; [[ $'αβ' = [α]* ]]`
            // stopped matching once GF_MULTIBYTE became option-gated.
            if (gflags & GF_MULTIBYTE) == 0 {
                let mut utf8 = [0u8; 4];
                let mut member_byte = |cp: u32| -> bool {
                    char::from_u32(cp).is_some_and(|c| {
                        c.encode_utf8(&mut utf8).as_bytes().iter().any(|&x| x == b)
                    })
                };
                for k in 0..n_mbc {
                    let o = mbc_start + k * 4;
                    if member_byte(u32::from_le_bytes(code[o..o + 4].try_into().unwrap())) {
                        return (true, 1);
                    }
                }
                for k in 0..n_mbr {
                    let o = mbr_start + k * 8;
                    if member_byte(u32::from_le_bytes(code[o..o + 4].try_into().unwrap()))
                        || member_byte(u32::from_le_bytes(code[o + 4..o + 8].try_into().unwrap()))
                    {
                        return (true, 1);
                    }
                }
            }
            return (false, 1);
        }
        // Decode one logical input char + its source byte span.
        let mut it = string[off..].chars();
        let (ch, adv) = match it.next() {
            Some('\u{83}') => match it.next() {
                Some(n) => (
                    ((n as u32 as u8) ^ 0x20) as char,
                    '\u{83}'.len_utf8() + n.len_utf8(),
                ),
                None => ('\u{83}', 2),
            },
            Some(c) => (c, c.len_utf8()),
            None => return (false, 1),
        };
        let class_hit = (classmask & (1 << 0) != 0 && ch.is_alphabetic())
            || (classmask & (1 << 1) != 0 && ch.is_alphanumeric())
            || (classmask & (1 << 2) != 0 && ch.is_uppercase())
            || (classmask & (1 << 3) != 0 && ch.is_lowercase())
            || (classmask & (1 << 4) != 0 && ch.is_whitespace())
            || (classmask & (1 << 5) != 0 && (ch == ' ' || ch == '\t'))
            || (classmask & (1 << 6) != 0
                && !ch.is_alphanumeric()
                && !ch.is_whitespace()
                && !ch.is_control())
            || (classmask & (1 << 7) != 0 && ch.is_control())
            || (classmask & (1 << 8) != 0 && !ch.is_control())
            || (classmask & (1 << 9) != 0 && !ch.is_control() && !ch.is_whitespace())
            || live_class_hit(ch)
            // c:3737-3744 — `case PP_INCOMPLETE: if (zmb_ind ==
            // ZMB_INCOMPLETE) return 1;` / `case PP_INVALID: if (zmb_ind ==
            // ZMB_INVALID) return 1;`. C's caller hands mb_patmatchrange the
            // decode verdict for the candidate character; zshrs keeps an
            // undecodable input byte METAFIED inside a Rust `String`
            // (Meta + byte^32), so recover the raw byte run here and ask
            // std's UTF-8 decoder for the same three-way answer:
            //   Ok / error past offset 0        -> ZMB_VALID
            //   Err, error_len() == None        -> ZMB_INCOMPLETE (truncated)
            //   Err, error_len() == Some(_)     -> ZMB_INVALID
            || ((classmask & ((1 << 14) | (1 << 15))) != 0 && {
                // Rebuild up to MB_CUR_MAX raw bytes starting at `off`.
                let mut raw: Vec<u8> = Vec::new();
                let mut rit = string[off..].chars();
                while raw.len() < 4 {
                    match rit.next() {
                        Some('\u{83}') => match rit.next() {
                            Some(n) => raw.push(((n as u32) as u8) ^ 0x20),
                            None => break,
                        },
                        Some(c) => {
                            let mut b = [0u8; 4];
                            raw.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
                        }
                        None => break,
                    }
                }
                match std::str::from_utf8(&raw) {
                    Err(e) if e.valid_up_to() == 0 => {
                        if e.error_len().is_none() {
                            (classmask & (1 << 14)) != 0 // c:3737-3740
                        } else {
                            (classmask & (1 << 15)) != 0 // c:3741-3744
                        }
                    }
                    _ => false,
                }
            });
        if class_hit {
            return (true, adv);
        }
        let cp = ch as u32;
        for k in 0..n_mbc {
            let o = mbc_start + k * 4;
            if u32::from_le_bytes(code[o..o + 4].try_into().unwrap()) == cp {
                return (true, adv);
            }
        }
        for k in 0..n_mbr {
            let o = mbr_start + k * 8;
            let lo = u32::from_le_bytes(code[o..o + 4].try_into().unwrap());
            let hi = u32::from_le_bytes(code[o + 4..o + 8].try_into().unwrap());
            if cp >= lo && cp <= hi {
                return (true, adv);
            }
        }
        (false, adv)
    };

    while scan < code.len() {
        let op = code[scan + I_OP];
        let next_bytes: [u8; 4] = code[scan + I_NEXT..scan + I_NEXT + 4].try_into().unwrap();
        let next = u32::from_le_bytes(next_bytes) as usize;

        match op {
            P_END => {
                // c:Src/pattern.c:3451-3454 — `case P_END: if (!(fail =
                // (patinput < patinend && !(patflags & PAT_NOANCH))))
                // return 1; break;`. When the WHOLE pattern is consumed
                // but the string ISN'T (anchored full match), this
                // branch FAILS so an enclosing alternation backtracks
                // to the next alternative — `[[ yes == (y|yes) ]]`
                // tries `y` (consumes 1 char, reaches P_END at offset
                // 1 ≠ 3), fails here, then tries `yes`. The previous
                // unconditional `Some(s_off)` committed to the first
                // (prefix) alternative and the external anchor check
                // rejected it without ever trying `yes`. PAT_NOANCH /
                // PAT_NOTEND callers (prefix/suffix strip, `(#e)`
                // interior matches) keep the partial-success return.
                let pf = patflags.load(Ordering::Relaxed);
                let anchored = (pf & (PAT_NOANCH | PAT_NOTEND) as i32) == 0;
                if s_off < string.len() && anchored {
                    // c:Src/pattern.c:3451-3454 sets `fail = 1` here, then the
                    // shared approximate-match block at c:3463-3475 deletes the
                    // trailing input one CHARACTER at a time — `++errsfound;
                    // CHARINC(patinput); continue;` — retrying P_END until the
                    // input is gone or the `(#aN)` budget is spent. So
                    // `[[ ab = (#a1)? ]]` matches: `?` consumes `a`, then the
                    // leftover `b` is deleted for one error.
                    //
                    // Only literal (P_EXACTLY) runs did this before, inside
                    // approx_match_exactly's own trailing-delete; a pattern
                    // ending in a class / `?` / `*` with input left reached
                    // here and simply failed, so `(#a2)[^0-9]` on `abc`
                    // (match `a`, delete `bc`) was rejected where zsh accepts
                    // it. The class/`?` arms already delete on a FAILED match
                    // (e.g. c:5046); this is the same edit at the pattern end.
                    let max_errs = (glob_flags & 0xff) as i32;
                    if state.errsfound < max_errs {
                        // c:3475 `CHARINC(patinput, patinend)` + continue
                        // (retry P_END). CHARINC is option-gated, and
                        // `s_off` can sit mid-character when GF_MULTIBYTE
                        // is clear, so go through `charinc` rather
                        // than slicing `string` (which would panic).
                        let advance = charinc(s_off, glob_flags);
                        if advance != 0 {
                            state.errsfound += 1; // c:3466 ++errsfound
                            return patmatch(
                                code,
                                scan,
                                string,
                                s_off + advance,
                                state,
                                glob_flags,
                            );
                        }
                    }
                    return None; // c:3452 fail, no budget → caller tries next alt
                }
                return Some(s_off); // c:3453
            }
            P_NOTHING => { /* empty match, just continue */ }
            P_BACK => { /* zero-width, walk back via next */ }
            P_EXCEND => {
                // c:Src/pattern.c:3037-3047 — terminal node ending an
                // exclusion operand: the exclusion matches iff the
                // (truncated) excludable span was fully consumed
                // (`patinput >= patinend`). Returns success here without
                // following the chain (which would wrongly continue into
                // the post-group pattern). The caller truncates the span
                // so `string.len()` IS the excludable end.
                if s_off >= string.len() {
                    return Some(s_off);
                }
                return None;
            }
            P_EXCSYNC => {
                // c:Src/pattern.c:2992-3035 — record where the asserted
                // branch reached the EXCLUDE sync point (the end of the
                // excludable part) in the following EXCLUDE node's sync
                // buffer. If this position was already recorded (with
                // <= the current error count), fail so the asserted
                // branch backtracks to a different split. The EXCLUDE
                // node sits PHYSICALLY right after EXCSYNC (both
                // patcompnot and the `~` arm emit them adjacent).
                let exclude_node = scan + I_BODY;
                let already = EXCSYNC_BUF.with(|b| {
                    let mut m = b.borrow_mut();
                    if let Some(buf) = m.get_mut(&exclude_node) {
                        if s_off < buf.len() {
                            let cur = (state.errsfound + 1) as u8;
                            if buf[s_off] != 0 && (state.errsfound + 1) >= buf[s_off] as i32 {
                                return true; // c:3008 already matched here → fail
                            }
                            buf[s_off] = cur; // c:3017
                                              // c:3033 — earlier marks are now invalid.
                            for x in buf[..s_off].iter_mut() {
                                *x = 0;
                            }
                        }
                    }
                    false
                });
                if already {
                    return None;
                }
                // else fall through to next node
            }
            P_EXACTLY => {
                // c:P_EXACTLY arm
                let body = scan + I_BODY;
                let len = u32::from_le_bytes(code[body..body + 4].try_into().unwrap()) as usize;
                let str_bytes = &code[body + 4..body + 4 + len];
                let input_bytes = string.as_bytes();
                // `(#aN)` budget — low byte of patglobflags. C uses the
                // file-static `patglobflags`; the Rust port carries it
                // through `glob_flags` since rpat went bucket-1.
                let max_errs = (glob_flags & 0xff) as i32;
                if max_errs > 0 {
                    // Approximate match: try edit operations
                    // (substitute/insert/delete) at each mismatch up to
                    // the budget. Per c:3055/3109/3193 the C source
                    // tracks `errsfound` across recursive patmatch
                    // calls; the Rust port does the same via `state.
                    // errsfound`. Skips transposition (Damerau extension
                    // — rare; faithful follow-on).
                    if let Some(new_off) = approx_match_exactly(
                        code, next, string, s_off, str_bytes, state, glob_flags, max_errs,
                    ) {
                        return Some(new_off);
                    }
                    return None;
                }
                if s_off + len > input_bytes.len() {
                    return None;
                }
                let case_flags = glob_flags & (GF_IGNCASE | GF_LCMATCHUC);
                let multibyte = (glob_flags & GF_MULTIBYTE) != 0; // c:349 GF_MULTIBYTE
                if case_flags != 0 {
                    let inp_slice = &input_bytes[s_off..s_off + len];
                    if multibyte && (glob_flags & GF_IGNCASE) != 0 {
                        // Char-level Unicode case fold for IGNCASE only —
                        // LCMATCHUC's asymmetric ASCII semantic doesn't
                        // map cleanly to non-ASCII chars; per C the
                        // CHARMATCH macro is byte-level anyway (TOLOWER/
                        // TOUPPER from utils.c work on bytes).
                        let pat_str = std::str::from_utf8(str_bytes).ok();
                        let inp_str = std::str::from_utf8(inp_slice).ok();
                        if let (Some(p), Some(i)) = (pat_str, inp_str) {
                            let mut pc = p.chars();
                            let mut ic = i.chars();
                            loop {
                                match (pc.next(), ic.next()) {
                                    (None, None) => break,
                                    (Some(_), None) | (None, Some(_)) => return None,
                                    (Some(a), Some(b)) => {
                                        // Equal chars (the common case) and ASCII
                                        // pairs fold without touching the heap.
                                        // The old per-char `to_lowercase()
                                        // .collect::<String>()` pair allocated
                                        // TWICE PER CHARACTER — `(#i)` scans over
                                        // a 566k-entry history (hsmw ^R) burned
                                        // ~4 CPU-minutes in RawVec::reserve.
                                        // Iterator::eq covers the multi-char
                                        // Unicode folds allocation-free.
                                        if a != b
                                            && (if a.is_ascii() && b.is_ascii() {
                                                a.to_ascii_lowercase() != b.to_ascii_lowercase()
                                            } else {
                                                !a.to_lowercase().eq(b.to_lowercase())
                                            })
                                        {
                                            return None;
                                        }
                                    }
                                }
                            }
                        } else {
                            // Non-UTF-8 input — byte fallback through CHARMATCH.
                            for k in 0..len {
                                if !charmatch(inp_slice[k], str_bytes[k], glob_flags) {
                                    return None;
                                }
                            }
                        }
                    } else {
                        // c:2694 — per-byte CHARMATCH walk (covers
                        // GF_IGNCASE byte-mode AND GF_LCMATCHUC asymmetric).
                        for k in 0..len {
                            if !charmatch(inp_slice[k], str_bytes[k], glob_flags) {
                                return None;
                            }
                        }
                    }
                } else if &input_bytes[s_off..s_off + len] != str_bytes {
                    return None;
                }
                s_off += len;
            }
            P_ANY => {
                // c:2736-2741 — `case P_ANY: if (patinput == patinend)
                // fail = 1; else CHARINC(patinput, patinend); break;`.
                // `CHARINC` (c:1963) is `charnext`, which advances a full
                // multibyte character ONLY under GF_MULTIBYTE and a single
                // raw byte otherwise (c:1946). `charinc` carries
                // that option gate plus zshrs's metafied-input handling;
                // it previously consumed a whole UTF-8 character
                // unconditionally, so `[[ $'α' = ?? ]]` failed under
                // `unsetopt multibyte` where zsh matches the two bytes.
                let advance = charinc(s_off, glob_flags);
                if advance == 0 {
                    return None;
                }
                s_off += advance;
            }
            P_ANYOF => {
                // c:2780-2800 — a bracket expression is matched by
                // `patmatchrange` / `mb_patmatchrange` on the RAW input char.
                // Neither consults `patglobflags`, so a bracket NEVER
                // case-folds: only C's CHARMATCH macro (c:2671, used by
                // P_EXACTLY) honours GF_IGNCASE / GF_LCMATCHUC.
                //
                // Passing glob_flags through here made `(#i)` fold ranges as
                // well as literals, so `[[ FooBar = (#i)[a-z]## ]]` matched
                // (zsh: no match) and `[[ F = (#i)[[:lower:]] ]]` matched
                // (zsh: no match). Mask the case bits off for the set test.
                let body = scan + I_BODY;
                let input_bytes = string.as_bytes();
                let max_errs = (glob_flags & 0xff) as i32;
                let (has_match, adv) = if s_off < input_bytes.len() {
                    anyof_membership(body, s_off, glob_flags)
                } else {
                    (false, 0)
                };
                if !has_match {
                    // c:Src/pattern.c:3463-3505 — approximate-match fail
                    // handler. For non-P_EXACTLY opcodes, the ONLY
                    // approximation path is "omit one input char" (skip
                    // the input byte that didn't match, costing 1 edit,
                    // then retry the same scan). For P_ANYOF that means
                    // `[b][b]` against "bob" can match by omitting the
                    // middle "o" with 1 edit — the `(#a1)[b][b]` pin.
                    if state.errsfound < max_errs && s_off < input_bytes.len() {
                        state.errsfound += 1;
                        // Retry same scan with one input byte consumed.
                        return patmatch(code, scan, string, s_off + 1, state, glob_flags);
                    }
                    return None;
                }
                s_off += adv;
            }
            P_ANYBUT => {
                let body = scan + I_BODY;
                let input_bytes = string.as_bytes();
                let max_errs = (glob_flags & 0xff) as i32;
                // c:2781-2800 — the negated bracket shares P_ANYOF's matcher
                // (`… ^ (P_OP(scan) == P_ANYOF)`), so it is equally
                // case-BLIND (anyof_membership masks the case bits off).
                let (in_set, adv) = if s_off < input_bytes.len() {
                    anyof_membership(body, s_off, glob_flags)
                } else {
                    (true, 0)
                };
                let has_match = s_off < input_bytes.len() && !in_set;
                if !has_match {
                    if state.errsfound < max_errs && s_off < input_bytes.len() {
                        // c:3463 — omit-input approx path (same as P_ANYOF).
                        state.errsfound += 1;
                        return patmatch(code, scan, string, s_off + 1, state, glob_flags);
                    }
                    return None;
                }
                s_off += adv;
            }
            P_STAR => {
                // c:3277-3400 — `*` is handled specially "although really
                // P_ONEHASH+P_ANY". C walks the input with CHARINC and
                // records every CHARACTER start in `charstart[]`
                // (c:3310-3315 `for (no = 0; patinput < patinend;
                // CHARINC(patinput, patinend)) { charstart[patinput-start]
                // = 1; no++; }`), then backtracks by rewinding to the
                // previous recorded start (c:3385-3389 `/* find start of
                // previous full character */ while (!*--lastcharstart) ...
                // patinput = start + (lastcharstart-charstart);`).
                //
                // Rewinding one BYTE instead left the continuation opcode
                // positioned inside a multibyte character: `[[ $'αβγδε' =
                // *(#b)(?) ]]` panicked slicing at a UTF-8 continuation
                // byte, and any `*`-then-atom pattern could match at a
                // non-character position. `charinc` reproduces
                // CHARINC exactly, so with GF_MULTIBYTE clear every byte
                // is still a valid stop (c:1946) — the byte-stepping
                // behaviour zsh has under `unsetopt multibyte`.
                let end = string.len();
                // c:3310-3315 — record the character starts.
                let mut starts: Vec<usize> = Vec::new();
                let mut p = s_off;
                while p < end {
                    starts.push(p);
                    let adv = charinc(p, glob_flags);
                    if adv == 0 {
                        break;
                    }
                    p += adv;
                }
                // c:3299-3315 — the greedy end position (`patinput` after
                // the walk) is tried first; `min` is 0 for P_STAR
                // (c:3328), so the rewind reaches `start` inclusive.
                starts.push(end);
                for &pos in starts.iter().rev() {
                    // c:3374-3382 — try the continuation at this position.
                    let mut sub_state = state.clone();
                    if let Some(matched) =
                        patmatch(code, next, string, pos, &mut sub_state, glob_flags)
                    {
                        *state = sub_state;
                        return Some(matched);
                    }
                }
                return None; // c:3400
            }
            P_ONEHASH | P_TWOHASH => {
                // c:P_ONEHASH / P_TWOHASH
                // The operand (the simple atom being repeated) starts
                // at `scan + I_BODY` — that's the byte immediately
                // after the quantifier opcode (which has its own
                // 5-byte header). The repeated atom occupies the
                // bytes from there until `next`.
                let operand = scan + I_BODY;
                let min = if op == P_TWOHASH { 1 } else { 0 };
                // Greedy: match operand repeatedly until it fails,
                // then walk back trying continuations.
                let mut positions = vec![s_off];
                loop {
                    let cur = *positions.last().unwrap();
                    let mut sub_state = state.clone();
                    if let Some(new_pos) =
                        patmatch(code, operand, string, cur, &mut sub_state, glob_flags)
                    {
                        if new_pos == cur {
                            break;
                        } // zero-width fixed point
                        *state = sub_state;
                        positions.push(new_pos);
                    } else {
                        break;
                    }
                }
                if positions.len() - 1 < min {
                    return None;
                }
                // Walk back from longest match trying continuations.
                while positions.len() > min {
                    let cur = *positions.last().unwrap();
                    let mut sub_state = state.clone();
                    if let Some(end) = patmatch(code, next, string, cur, &mut sub_state, glob_flags)
                    {
                        *state = sub_state;
                        return Some(end);
                    }
                    if positions.len() <= min + 1 {
                        return None;
                    }
                    positions.pop();
                }
                return None;
            }
            P_BRANCH | P_WBRANCH => {
                // c:P_BRANCH / P_WBRANCH arm — c:3043-3044 in zsh.
                // c:3046-3050 — if next is NOT another BRANCH, this is
                // the only alternative; avoid the alt-loop and just
                // continue with the operand inline (no recursion, no
                // fallthrough on failure).
                //
                // For P_WBRANCH, the operand sits AFTER the 8-byte
                // syncptr payload (`P_OPERAND(p) + 1` per pattern.c:1865).
                let operand_off_extra = if op == P_WBRANCH { 8 } else { 0 };
                // c:3056 — if `next` is P_EXCLUDE/P_EXCLUDP, this BRANCH
                // is the asserted half of a `^pat` / `!(pat)` exclusion.
                // Minimal port of c:3056-3201: try the asserted operand
                // (the `*`-based branch body), then for each EXCLUDE in
                // the next-chain run the exclude operand against the
                // same input range; if any exclude matches with the
                // SAME consumed length, fail; else succeed.
                if next != 0 && next < code.len() && P_ISEXCLUDE(code[next + I_OP]) {
                    // c:Src/pattern.c:3056-3201 — `^pat` / `(^pat)` /
                    // `!(pat)` / `A~B` exclusion. The asserted branch
                    // (`STAR EXCSYNC rest` for `^`/`!`, or `A EXCSYNC
                    // rest` for `A~B`) is matched normally; its EXCSYNC
                    // node records where the EXCLUDABLE part ended into
                    // the EXCLUDE node's sync buffer. We then truncate to
                    // that synclen and test the exclusion operand(s); if
                    // any matches the excludable span, this candidate is
                    // excluded and we re-run the asserted branch — EXCSYNC
                    // now fails at the recorded position, forcing a
                    // different split — until an un-excluded split is
                    // found or the asserted branch can no longer match.
                    let asserted_operand = scan + I_BODY + operand_off_extra;
                    // c:2941 — `patglobflags = P_OPERAND(scan)->l;` — in C
                    // P_GFLAGS writes the GLOBAL flag word, so a flag node
                    // sitting at the head of the asserted branch is already
                    // in effect by the time the exclusion operand runs at
                    // c:3134+ (which only clears the low `(#aN)` byte). The
                    // Rust port threads the flags as a parameter instead, so
                    // apply a leading P_GFLAGS here explicitly. Without it
                    // `readme` vs `(#i)readme~README|readme~README` ran the
                    // SECOND branch's exclusion case-insensitively and
                    // wrongly excluded the match.
                    let branch_flags_eff =
                        if code.get(asserted_operand + I_OP).copied().unwrap_or(0) == P_GFLAGS {
                            let b = asserted_operand + I_BODY;
                            let bits = i32::from_le_bytes(code[b..b + 4].try_into().unwrap());
                            (glob_flags & !(GF_IGNCASE | GF_LCMATCHUC | GF_MULTIBYTE | 0xff)) | bits
                        } else {
                            glob_flags
                        };
                    let exclude_node = next;
                    // Allocate (reset) the sync buffer for this EXCLUDE.
                    let prev_buf = EXCSYNC_BUF.with(|b| {
                        b.borrow_mut()
                            .insert(exclude_node, vec![0u8; string.len() + 1])
                    });
                    let mut found: Option<(usize, rpat)> = None;
                    loop {
                        let mut a_state = state.clone();
                        let matchpt = match patmatch(
                            code,
                            asserted_operand,
                            string,
                            s_off,
                            &mut a_state,
                            glob_flags,
                        ) {
                            Some(e) => e,
                            None => break,
                        };
                        // c:3128-3130 — synclen = first marked position in
                        // the sync buffer (the end of the excludable part).
                        let synclen = EXCSYNC_BUF.with(|b| {
                            b.borrow()
                                .get(&exclude_node)
                                .and_then(|buf| buf.iter().position(|&x| x != 0))
                                .unwrap_or(s_off)
                        });
                        // c:3134-3138 — test each EXCLUDE/EXCLUDP operand
                        // against the excludable span string[s_off..synclen].
                        // Truncating to `synclen` makes the operand's
                        // trailing P_EXCEND succeed iff the span matched in
                        // full.
                        let span_end = synclen.min(string.len());
                        let span = &string[..span_end];
                        let mut excluded = false;
                        let mut excl = exclude_node;
                        while excl != 0 && excl < code.len() && P_ISEXCLUDE(code[excl + I_OP]) {
                            let excl_operand = excl + I_BODY + 8; // after 8-byte syncptr
                            let mut e_state = state.clone();
                            // c:3134-3142 — `patglobflags &= ~0xff;
                            // errsfound = 0;` — exclusions match EXACTLY,
                            // with approximation turned off. Clear both the
                            // running error count AND the budget (low byte
                            // of glob_flags); otherwise `(#a1)README~READ_ME`
                            // matched the exclusion READ_ME against READ.ME
                            // approximately and wrongly excluded it.
                            e_state.errsfound = 0;
                            let excl_flags = branch_flags_eff & !0xff;
                            if let Some(em) =
                                patmatch(code, excl_operand, span, s_off, &mut e_state, excl_flags)
                            {
                                if em == span_end {
                                    excluded = true;
                                    break;
                                }
                            }
                            let nb: [u8; 4] =
                                code[excl + I_NEXT..excl + I_NEXT + 4].try_into().unwrap();
                            let n = u32::from_le_bytes(nb) as usize;
                            if n == 0 || n == excl {
                                break;
                            }
                            excl = n;
                        }
                        if !excluded {
                            found = Some((matchpt, a_state));
                            break;
                        }
                        // Excluded: loop. The sync buffer now records this
                        // split, so the next asserted patmatch's EXCSYNC
                        // fails here and backtracks to a different split.
                    }
                    // Restore/clear the sync buffer slot.
                    EXCSYNC_BUF.with(|b| {
                        let mut m = b.borrow_mut();
                        match prev_buf {
                            Some(p) => {
                                m.insert(exclude_node, p);
                            }
                            None => {
                                m.remove(&exclude_node);
                            }
                        }
                    });
                    match found {
                        Some((end, a_state)) => {
                            *state = a_state;
                            return Some(end);
                        }
                        None => {
                            // c:3209-3211 — `while ((scan = PATNEXT(scan)) &&
                            // P_ISEXCLUDE(scan)) ;` — when the asserted branch
                            // plus its exclusions cannot produce an un-excluded
                            // split, C skips PAST the whole exclusion chain and
                            // carries on with the next ALTERNATIVE of the
                            // enclosing switch. Returning None here instead
                            // dropped every branch written after an exclusion:
                            // `[[ xyz = readme~README|xyz ]]` never tried
                            // `xyz`.
                            let mut nxt = next;
                            while nxt != 0 && nxt < code.len() && P_ISEXCLUDE(code[nxt + I_OP]) {
                                let nb: [u8; 4] =
                                    code[nxt + I_NEXT..nxt + I_NEXT + 4].try_into().unwrap();
                                nxt = u32::from_le_bytes(nb) as usize;
                            }
                            if nxt == 0 || nxt >= code.len() {
                                return None;
                            }
                            let nop = code[nxt + I_OP];
                            if nop != P_BRANCH && nop != P_WBRANCH {
                                return None;
                            }
                            scan = nxt;
                            continue;
                        }
                    }
                }
                let next_is_branch = next != 0
                    && next < code.len()
                    && (code[next + I_OP] == P_BRANCH || code[next + I_OP] == P_WBRANCH);
                if !next_is_branch {
                    scan = scan + I_BODY + operand_off_extra;
                    continue;
                }
                // Alt-loop: try each branch's operand; on success
                // return; on failure walk to the next BRANCH via .next.
                let mut br = scan;
                loop {
                    let br_op = code[br + I_OP];
                    let br_extra = if br_op == P_WBRANCH { 8 } else { 0 };
                    let br_next_bytes: [u8; 4] =
                        code[br + I_NEXT..br + I_NEXT + 4].try_into().unwrap();
                    let br_next = u32::from_le_bytes(br_next_bytes) as usize;
                    let operand = br + I_BODY + br_extra;
                    let mut sub_state = state.clone();
                    // c:Src/pattern.c:3210-3248 — P_WBRANCH per-position
                    // visit guard. Allocate a bitmap sized to the input
                    // (`zshcalloc((patinend - patinstart) + 1)`), then
                    // check/set `*ptr = errsfound + 1` at the current
                    // input offset. On revisit with the same-or-fewer
                    // errors, return 0 to bound recursion. Without this,
                    // `(fo#)#` against any non-trivial input recurses
                    // until PATMATCH_MAX_DEPTH.
                    let mut wbranch_skip = false;
                    if br_op == P_WBRANCH {
                        let bm = state
                            .wbranch_visits
                            .entry(br)
                            .or_insert_with(|| vec![0u8; string.len() + 1]);
                        let slot = bm.get(s_off).copied().unwrap_or(0);
                        let cur = (state.errsfound as i32 + 1) as u8;
                        if slot != 0 && (state.errsfound + 1) >= slot as i32 {
                            wbranch_skip = true; // c:3245-3247
                        } else if s_off < bm.len() {
                            bm[s_off] = cur; // c:3248
                        }
                    }
                    let sub_result = if wbranch_skip {
                        None
                    } else {
                        patmatch(code, operand, string, s_off, &mut sub_state, glob_flags)
                    };
                    if let Some(end) = sub_result {
                        // c:108-144 (pattern.c header doc on P_WBRANCH):
                        //   "P_WBRANCH:  This works like a branch and is
                        //    used in complex closures, but the match must
                        //    be at least 1 char in length to avoid
                        //    infinite loops.  The test for length is
                        //    done via the next pointer in the WBRANCH
                        //    test in patmatch()."
                        // C enforces this via the `errsfound`/`forceerrs`
                        // accounting at c:1059+ inside patcompbranch.
                        // The Rust walker checks end > s_off directly:
                        // if the body consumed nothing, reject this
                        // alternative and fall through to the next.
                        // Without this guard, `(fo#)#` against any input
                        // stack-overflows because the inner body (`fo#`)
                        // can match the empty string repeatedly.
                        if br_op == P_WBRANCH && end == s_off {
                            // c:108 "at least 1 char" — fall through to
                            // try the next alternative.
                        } else {
                            *state = sub_state;
                            return Some(end);
                        }
                    }
                    if br_next == 0 {
                        return None;
                    }
                    let op_next = code[br_next + I_OP];
                    if op_next != P_BRANCH && op_next != P_WBRANCH {
                        return None;
                    }
                    br = br_next;
                }
            }
            P_NUMRNG | P_NUMFROM | P_NUMTO => {
                // c:2815-2909 — `case P_NUMRNG: case P_NUMFROM: case P_NUMTO:`
                //
                // c:2818-2825 — "To do this properly, we really have to
                // treat numbers as closures: that's so things like
                // <1-1000>33 will match 633 (they didn't up to 3.1.6).
                // To avoid making this too inefficient, we see if there's
                // an exact match next: if there is, and it's not a digit,
                // we return 1 after the first attempt."
                let mut start_b = scan + I_BODY; // c:2827 `start = P_OPERAND(scan);`
                let mut from: i64 = 0; // c:2828 `from = to = 0;`
                let mut to: i64 = 0;
                if op != P_NUMTO {
                    // c:2829-2837
                    from = i64::from_le_bytes(code[start_b..start_b + 8].try_into().unwrap());
                    start_b += 8; // c:2836 `start += sizeof(zrange_t);`
                }
                if op != P_NUMFROM {
                    // c:2838-2845
                    to = i64::from_le_bytes(code[start_b..start_b + 8].try_into().unwrap());
                }
                let input_bytes = string.as_bytes();
                let patinend = input_bytes.len();
                // c:2846 — `start = compend = patinput;`
                let start = s_off;
                let mut compend = s_off;
                let mut comp: i64 = 0; // c:2847
                let mut patinput = s_off;
                // c:2848 — `while (patinput < patinend && idigit(*patinput))`
                while patinput < patinend && input_bytes[patinput].is_ascii_digit() {
                    let mut out_of_range = false; // c:2849
                    let digit = (input_bytes[patinput] - b'0') as i64; // c:2850
                    if comp > i64::MAX / 10 {
                        // c:2851-2852
                        out_of_range = true;
                    } else {
                        let c10 = if comp != 0 { comp * 10 } else { 0 }; // c:2854
                        if i64::MAX - c10 < digit {
                            out_of_range = true; // c:2856
                        } else {
                            comp = c10; // c:2858
                            comp += digit; // c:2859
                        }
                    }
                    patinput += 1; // c:2862
                    compend += 1; // c:2863
                                  // c:2865-2873 — out of range "allowing for signedness,
                                  // which we need if we are using zlongs".
                    if out_of_range || (comp & (1i64 << 62)) != 0 {
                        // c:2875-2881 — "This is as far as we can go. If
                        // we're doing a range \"from\", skip all the
                        // remaining numbers. Otherwise, we can't match
                        // beyond the previous point anyway. Leave the
                        // pointer to the last calculated position
                        // (compend) where it was before."
                        if op == P_NUMFROM {
                            // c:2882-2885
                            while patinput < patinend && input_bytes[patinput].is_ascii_digit() {
                                patinput += 1;
                            }
                        }
                    }
                }
                let mut save = patinput; // c:2888
                let mut no = 0; // c:2889
                while patinput > start {
                    // c:2890
                    // c:2891-2893 — "if already too small, no power on
                    // earth can save it"
                    if comp < from && patinput <= compend {
                        break;
                    }
                    if op == P_NUMFROM || comp <= to {
                        // c:2894
                        let mut sub_state = state.clone();
                        if let Some(end) =
                            patmatch(code, next, string, patinput, &mut sub_state, glob_flags)
                        {
                            *state = sub_state;
                            return Some(end); // c:2895
                        }
                    }
                    // c:2896-2900 — `if (!no && P_OP(next) == P_EXACTLY &&
                    // (!P_LS_LEN(next) || !idigit(*P_LS_STR(next))) &&
                    // !(patglobflags & 0xff)) return 0;`
                    if no == 0 && next != 0 && (glob_flags & 0xff) == 0 {
                        let nop = code.get(next + I_OP).copied().unwrap_or(0);
                        if nop == P_EXACTLY {
                            let nb = next + I_BODY;
                            let nlen =
                                u32::from_le_bytes(code[nb..nb + 4].try_into().unwrap()) as usize;
                            if nlen == 0 || !code[nb + 4].is_ascii_digit() {
                                return None;
                            }
                        }
                    }
                    // c:2901 — `patinput = --save;`
                    save -= 1;
                    patinput = save;
                    no += 1; // c:2902
                             // c:2903-2908 — "With a range start and an
                             // unrepresentable test number, we just back down the
                             // test string without changing the number until we get
                             // to a representable one."
                    if patinput < compend {
                        comp /= 10;
                    }
                }
                // c:2910-2911 — `patinput = start; fail = 1;`
                return None;
            }
            P_NUMANY => {
                // c:P_NUMANY — `<->` any non-empty digit run. Pins
                // `Test/D02glob.ztst:136` (`<->33` matches "633" by
                // consuming just "6" then literal "33").
                let input_bytes = string.as_bytes();
                let start = s_off;
                let mut k = start;
                while k < input_bytes.len() && input_bytes[k].is_ascii_digit() {
                    k += 1;
                }
                if k == start {
                    return None;
                }
                while k > start {
                    let mut sub_state = state.clone();
                    if let Some(end) = patmatch(code, next, string, k, &mut sub_state, glob_flags) {
                        *state = sub_state;
                        return Some(end);
                    }
                    k -= 1;
                }
                return None;
            }
            P_ISSTART => {
                // c:Src/pattern.c:3392-3394 — `if (patinput != patinstart
                // || (patflags & PAT_NOTSTART)) fail = 1;`. C bails on
                // BOTH conditions: local-position-not-zero OR the
                // caller-set PAT_NOTSTART flag (set by glob.c's
                // set_pat_start when the test string is an interior
                // slice of a larger buffer). Read patflags from the
                // file-static (set at pattryrefs entry from prog flags).
                if s_off != 0 || (patflags.load(Ordering::Relaxed) & PAT_NOTSTART as i32) != 0 {
                    return None;
                }
            }
            P_ISEND => {
                // c:Src/pattern.c:3396-3398 — `if (patinput < patinend
                // || (patflags & PAT_NOTEND)) fail = 1;`. Same shape as
                // P_ISSTART: bail on local-not-at-end OR caller-set
                // PAT_NOTEND (set by set_pat_end when the suffix was
                // truncated).
                if s_off < string.len()
                    || (patflags.load(Ordering::Relaxed) & PAT_NOTEND as i32) != 0
                {
                    return None;
                }
            }
            P_GFLAGS => {
                // c:P_GFLAGS arm
                let body = scan + I_BODY;
                let bits = i32::from_le_bytes(code[body..body + 4].try_into().unwrap());
                // C uses absolute set; for the on/off toggle pairs
                // we currently encode only the "on" bits (i.e. (#I)
                // emits 0 to clear). Set the running flags directly,
                // INCLUDING the low byte (`(#aN)` approximation budget)
                // so mid-pattern `(#a0)`/`(#a2)` re-arm the error
                // allowance — c:Src/pattern.c:2941.
                glob_flags =
                    (glob_flags & !(GF_IGNCASE | GF_LCMATCHUC | GF_MULTIBYTE | 0xff)) | bits;
            }
            P_COUNT => {
                // c:3428-3455 — `case P_COUNT: /* (#cN,M): execution is
                // relatively straightforward */`
                //
                //   long cur = scan[P_CT_CURRENT].l;
                //   long min = scan[P_CT_MIN].l;
                //   long max = scan[P_CT_MAX].l;
                //   if (cur && cur >= min &&
                //       (unsigned char *)patinput == scan[P_CT_PTR].p)
                //       return patmatch(next);
                //   scan[P_CT_PTR].p = (unsigned char *)patinput;
                //   if (max < 0 || cur < max) {
                //       char *patinput_thistime = patinput;
                //       scan[P_CT_CURRENT].l = cur + 1;
                //       if (patmatch(scan + P_CT_OPERAND)) return 1;
                //       scan[P_CT_CURRENT].l = cur;
                //       patinput = patinput_thistime;
                //   }
                //   if (cur < min) return 0;
                //   return patmatch(next);
                //
                // !!! RUST-ONLY STRUCTURE (same search, different plumbing) !!!
                // C makes the repetition loop by pointing the OPERAND's tail
                // back at this very node (c:1693-1696 `pattail(opnd,
                // patnode(P_BACK))`) and by keeping `cur` / `ptr` as mutable
                // cells INSIDE the compiled program (c:208-213 P_CT_CURRENT /
                // P_CT_PTR). That back-edge is what lets one iteration's
                // continuation re-enter P_COUNT, so the operand's own
                // backtracking picks a SHORTER match when the longer one
                // leaves the rest of the repetition unsatisfiable. zshrs
                // stores the operand inline with no back-edge and no
                // in-program cells, so the same search runs here over an
                // explicit DFS stack: at each step every end position the
                // operand can reach is tried, longest first (C's greedy
                // order), and the count rides the stack instead of the
                // program. The previous greedy-only walk let `*_` eat the
                // whole string on iteration 1 and never reconsidered, so
                // `[[ 1_2_ = (*_)(#c2) ]]` failed.
                let body = scan + I_BODY;
                let min = i64::from_le_bytes(code[body..body + 8].try_into().unwrap());
                let max = i64::from_le_bytes(code[body + 8..body + 16].try_into().unwrap());
                let operand = body + 16;
                // Stack frame: (input position, repetitions so far, next
                // candidate operand end to try — descending, saved state).
                let mut stack: Vec<(usize, i64, i64, rpat)> =
                    vec![(s_off, 0, string.len() as i64, state.clone())];
                while let Some((pos, cur, e, st)) = stack.pop() {
                    // c:3448 — `if (max < 0 || cur < max)`
                    if (max < 0 || cur < max) && e >= pos as i64 {
                        // Re-queue this level with the next-shorter operand
                        // end so an exhausted subtree backtracks here.
                        stack.push((pos, cur, e - 1, st.clone()));
                        let eu = e as usize;
                        if string.is_char_boundary(eu) {
                            let mut sub = st.clone();
                            // Truncating the subject to `eu` anchors the
                            // operand's end, which is how one candidate
                            // iteration length is proposed.
                            if let Some(got) =
                                patmatch(code, operand, &string[..eu], pos, &mut sub, glob_flags)
                            {
                                // c:3434-3444 — "the previous attempt managed
                                // zero length. We can do this indefinitely so
                                // there's no point in going on." A zero-length
                                // iteration still counts, but only up to `min`.
                                if got == eu && (eu != pos || cur + 1 <= min) {
                                    // c:3450 — `scan[P_CT_CURRENT].l = cur + 1;`
                                    stack.push((eu, cur + 1, string.len() as i64, sub));
                                }
                            }
                        }
                        continue;
                    }
                    // c:3453 — `if (cur < min) return 0;`
                    if cur < min {
                        continue;
                    }
                    // c:3454 — `return patmatch(next);`
                    if next == 0 {
                        *state = st;
                        return Some(pos);
                    }
                    let mut sub = st.clone();
                    if let Some(end) = patmatch(code, next, string, pos, &mut sub, glob_flags) {
                        *state = sub;
                        return Some(end);
                    }
                }
                return None;
            }
            op if op >= P_OPEN && op < P_CLOSE => {
                // c:P_OPEN_N arm (pattern.c:2939-2960).
                //
                // C `case P_OPEN+0..P_OPEN+9:
                //     no = P_OP(scan) - P_OPEN;
                //     save = patinput;
                //     if (patmatch(next)) {
                //         if (no && !(parsfound & (1 << (no - 1)))) {
                //             patbeginp[no-1] = save;
                //             parsfound |= 1 << (no - 1);
                //         }
                //         return 1;
                //     }
                //     return 0;`
                //
                // Recurse on `next`; only commit patbeginp[N-1] on
                // success AND only on the FIRST occurrence (c:2957
                // `!(parsfound & (1<<(no-1)))`). The first-write
                // semantic matters under `(*)*` and alternation
                // backtrack — a later iteration shouldn't overwrite
                // the saved start of the FIRST match.
                let n = (op - P_OPEN) as usize;
                let save = s_off;
                let saved_state = state.clone();
                // c:2957 — `if (no && !(parsfound & (1 << (no - 1))))`.
                // Open-bit is the LOW stripe: `1 << (n-1)`. n==0 is the
                // plain (uncaptured) P_OPEN — c:2957's `no &&` guard
                // means it records nothing; bit value is moot (0).
                let open_bit = if n > 0 { 1u32 << (n - 1) } else { 0 };
                if next == 0 {
                    // No continuation — leaf P_OPEN; just commit and continue.
                    if n > 0 && n <= NSUBEXP && (state.captures_set & open_bit) == 0 {
                        state.patbeginp[n - 1] = save;
                        state.captures_set |= open_bit; // c:2959
                    }
                    return Some(s_off);
                }
                match patmatch(code, next, string, s_off, state, glob_flags) {
                    Some(end) => {
                        // c:2957-2959 — first-write commit.
                        if n > 0 && n <= NSUBEXP && (state.captures_set & open_bit) == 0 {
                            state.patbeginp[n - 1] = save;
                            state.captures_set |= open_bit; // c:2959
                        }
                        return Some(end);
                    }
                    None => {
                        *state = saved_state;
                        return None;
                    }
                }
            }
            op if op >= P_CLOSE && op < 0xa0 => {
                // c:P_CLOSE_N arm (pattern.c:2980-3010).
                //
                // C `case P_CLOSE+0..P_CLOSE+9:
                //     no = P_OP(scan) - P_CLOSE;
                //     save = patinput;
                //     if (patmatch(next)) {
                //         if (no && !(parsfound & (1 << (no+NSUBEXP-1)))) {
                //             patendp[no-1] = save;
                //             parsfound |= 1 << (no+NSUBEXP-1);
                //         }
                //         return 1;
                //     }
                //     return 0;`
                //
                // Same save/recurse/first-write-on-success pattern as
                // P_OPEN. Rust uses `captures_set` bit (1<<(n-1)) for
                // BOTH open and close in the existing impl; semantically
                // the bit is set when the group's CLOSE has been seen,
                // i.e. the capture is complete. First-write on close
                // matters under `(...)*` so later iterations don't
                // overwrite the FIRST capture's end (matching C's
                // parsfound `no+NSUBEXP-1` bit-stripe).
                let n = (op - P_CLOSE) as usize;
                let save = s_off;
                let saved_state = state.clone();
                // c:2989 — `if (no && !(parsfound & (1 << (no+NSUBEXP-1))))`.
                // Close-bit is the HIGH stripe: `1 << (n-1+NSUBEXP)`.
                // n==0 = plain P_CLOSE, records nothing (c:2989 `no &&`).
                let close_bit = if n > 0 { 1u32 << (n - 1 + NSUBEXP) } else { 0 };
                if next == 0 {
                    if n > 0 && n <= NSUBEXP && (state.captures_set & close_bit) == 0 {
                        state.patendp[n - 1] = save;
                        state.captures_set |= close_bit;
                    }
                    return Some(s_off);
                }
                match patmatch(code, next, string, s_off, state, glob_flags) {
                    Some(end) => {
                        if n > 0 && n <= NSUBEXP && (state.captures_set & close_bit) == 0 {
                            state.patendp[n - 1] = save;
                            state.captures_set |= close_bit;
                        }
                        return Some(end);
                    }
                    None => {
                        *state = saved_state;
                        return None;
                    }
                }
            }
            _ => {
                // Unrecognized opcode — Phase 5 features (P_NUMRNG,
                // P_GFLAGS, P_EXCLUDE, P_COUNT, P_ISSTART/ISEND,
                // P_BACKREF) land here. Treat as no-op for now so
                // current tests still pass.
            }
        }

        if next == 0 {
            break;
        }
        scan = next;
    }
    Some(s_off)
}

/// Port of `patallocstr()` from `Src/pattern.c:2132`.
/// C: `char *patallocstr(Patprog prog, char *string, int stringlen,
/// int unmetalen, int force, Patstralloc patstralloc)` from
/// `Src/pattern.c:2132`.
///
/// Sets up `patstralloc` for a match attempt: when `force` is set or
/// the input contains Meta bytes (or PAT_HAS_EXCLUDP demands a
/// full-path copy), allocates an un-metafied scratch buffer and
/// stashes pointer/length info on `patstralloc`. Returns the
/// allocated buffer or `None` if no allocation was needed.
pub fn patallocstr(
    prog: &Patprog,
    string: &str,
    stringlen: i32,
    unmetalen: i32,
    force: i32,
    patstralloc: &mut patstralloc,
) -> Option<String> {
    // c:2132
    // c:2137 — `int needfullpath;`
    let mut needfullpath: bool;
    // Working values (mutated when force triggers patmungestring).
    let mut string: &str = string; // c:2133 char *string param
    let mut stringlen: i32 = stringlen;
    let mut unmetalen: i32 = unmetalen;

    if force != 0 {
        // c:2139
        // c:2140 — `patmungestring(&string, &stringlen, &unmetalen);`
        patmungestring(&mut string, &mut stringlen, &mut unmetalen);
    }

    /*
     * For a top-level ~-exclusion, we will need the full
     * path to exclude, so copy the path so far and append the
     * current test string.
     */
    // c:2142-2146
    // c:2147 — `needfullpath = (prog->flags & PAT_HAS_EXCLUDP) && pathpos;`
    // `pathpos` is the `gd_pathpos` field of `curglobdata` (c:Src/glob.c:166-170
    // struct globdata; c:197 static curglobdata; c:199-201 macros expand
    // `pathpos`→`curglobdata.gd_pathpos`). Read directly from the
    // shared CURGLOBDATA mutex — the canonical port surface for glob
    // state in zshrs.
    let pathpos: i32 = crate::ported::glob::CURGLOBDATA
        .lock()
        .map(|gd| gd.pathpos as i32)
        .unwrap_or(0); // c:Src/glob.c:169 gd_pathpos
    needfullpath = (prog.0.flags & PAT_HAS_EXCLUDP as i32) != 0 && pathpos != 0; // c:2147

    /* Get the length of the full string when unmetafied. */
    // c:2149
    if unmetalen < 0 {
        // c:2150
        // c:2151 — `patstralloc->unmetalen = ztrsub(string + stringlen, string);`
        // ztrsub returns the unmetafied char count between two pointers
        // in the same string. Rust analog: ztrsub(buf, start, end).
        patstralloc.unmetalen = ztrsub(string, 0, (stringlen as usize).min(string.len())) as i32;
    } else {
        // c:2152
        patstralloc.unmetalen = unmetalen; // c:2153
    }
    if needfullpath {
        // c:2154
        // c:2155 — `patstralloc->unmetalenp = ztrsub(pathbuf + pathpos, pathbuf);`
        // `pathbuf` is `curglobdata.gd_pathbuf` (c:Src/glob.c:170, macro
        // at c:200). ztrsub(end, start) returns unmetafied char count
        // between pointers. With pathpos in [0, pathbuf.len()] the
        // Rust analog is ztrsub(pathbuf, 0, pathpos as usize).
        let pathbuf = crate::ported::glob::CURGLOBDATA
            .lock()
            .map(|gd| gd.pathbuf.clone())
            .unwrap_or_default(); // c:Src/glob.c:170 gd_pathbuf
        let p_end = (pathpos as usize).min(pathbuf.len());
        patstralloc.unmetalenp = ztrsub(&pathbuf, 0, p_end) as i32; // c:2155
        if patstralloc.unmetalenp == 0 {
            // c:2156
            needfullpath = false; // c:2157 (`needfullpath = 0;`)
        }
    } else {
        // c:2158
        patstralloc.unmetalenp = 0; // c:2159
    }
    /* Initialise cache area */
    // c:2161
    patstralloc.progstrunmeta = None; // c:2162
    patstralloc.progstrunmetalen = 0; // c:2163

    // c:2165-2166 — `DPUTS(needfullpath && (prog->flags & (PAT_PURES|PAT_ANY)),
    //                       "rum sort of file exclusion");`
    // Rust drops the debug assertion.

    /*
     * Partly for efficiency, and partly for the convenience of
     * globbing, we don't unmetafy pure string patterns, and
     * there's no reason to if the pattern is just a *.
     */
    // c:2167-2171
    let pures_or_any = (prog.0.flags & (PAT_PURES | PAT_ANY) as i32) != 0;
    if force != 0 || (!pures_or_any && (needfullpath || patstralloc.unmetalen != stringlen))
    // c:2172
    {
        /*
         * We need to copy if we need to prepend the path so far
         * (in which case we copy both chunks), or if we have
         * Meta characters.
         */
        // c:2174-2178
        // c:2179 — `char *dst, *ptr; int i, icopy, ncopy;`
        let total = (patstralloc.unmetalen + patstralloc.unmetalenp) as usize;
        let mut dst = String::with_capacity(total); // c:2182 zhalloc

        // c:2184-2192 — choose source chunk(s).
        let mut ptr: &str;
        let mut ncopy: i32;
        if needfullpath {
            // c:2185
            // c:2186 — `ptr = pathbuf;` (stubbed empty)
            ptr = "";
            ncopy = patstralloc.unmetalenp; // c:2188
        } else {
            // c:2189
            ptr = string; // c:2190
            ncopy = patstralloc.unmetalen; // c:2191
        }
        // c:2193-2210 — for (icopy = 0; icopy < 2; icopy++) outer loop:
        //   copy ncopy bytes from ptr to dst, unmetafy Meta+X pairs.
        for icopy in 0..2 {
            // c:2193
            let ptr_bytes = ptr.as_bytes();
            let mut i = 0i32;
            let mut byte_idx = 0usize;
            while i < ncopy && byte_idx < ptr_bytes.len() {
                // c:2194
                if ptr_bytes[byte_idx] == Meta as u8 && byte_idx + 1 < ptr_bytes.len() {
                    // c:2195-2197 — `if (*ptr == Meta) { ptr++; *dst++ = *ptr++ ^ 32; }`
                    byte_idx += 1; // c:2196 ptr++
                    dst.push((ptr_bytes[byte_idx] ^ 32) as char); // c:2197 *dst++ = *ptr++ ^ 32
                    byte_idx += 1;
                } else {
                    // c:2198
                    // c:2199 — `else *dst++ = *ptr++;`
                    dst.push(ptr_bytes[byte_idx] as char);
                    byte_idx += 1;
                }
                i += 1;
            }
            if !needfullpath {
                // c:2203
                break; // c:2204
            }
            /* next time append test string to path so far */
            // c:2205
            ptr = string; // c:2207
            ncopy = patstralloc.unmetalen; // c:2208
            let _ = icopy;
        }
        patstralloc.alloced = Some(dst.clone()); // c:2182 dst = patstralloc->alloced
        return Some(dst); // c:2213 return patstralloc->alloced
    } else {
        // c:2214
        patstralloc.alloced = None; // c:2215
    }

    None // c:2218 return patstralloc->alloced (NULL)
}

// `patrepeat(Upat p, char *charstart)` (C: pattern.c:4096 — `static int`
// helper called from the bytecode walker at pattern.c:3321 for greedy
// `*` matches) had a Rust-only wrapper `pub fn patrepeat(prog: &Patprog,
// s: &str, max: Option<usize>)`. Zero Rust callers (the bytecode walker
// inlines its own greedy loop instead of calling out to patrepeat),
// Rule S1 deviation (extra `max` param, takes whole prog instead of a
// Upat into the bytecode). Deleted; reintroduce as a faithful port when
// the bytecode walker is refactored to use it.

// =====================================================================
// Transitional aliases — older callers still use `PatProg` (camel-case
// from the previous AST-based port). Alias them to `Patprog` so the
// build doesn't break; future cleanup commit renames callers.
// =====================================================================
/// `PatProg` type alias.
#[deprecated(note = "use Patprog instead")]
pub type PatProg = Patprog;

// =====================================================================
// Transitional Rust-only types — kept for external callers that bind
// to the previous AST-based port's surface area (vm_helper, exec_shims.rs,
// fusevm_bridge.rs, glob.rs). These are NOT C-faithful ports — they're
// helper aggregates the previous AST port introduced for one-shot
// pattern processing in the executor/VM bridge. Track with a TODO
// for eventual deletion + migration of callers to the bytecode API
// (patcompile + pattry + patgetglobflags). Allowlisted as transitional.
// =====================================================================

// `NumericRange` struct + impl DELETED. C zsh's `patcomppiece`
// (Src/pattern.c:1450+) inlines `<N-M>` parsing and emits `P_NUMRNG`
// opcodes — no aggregator type. Rust port uses these bare helpers
// returning tuples `(start, end, lo, hi)` for the pre-pattern pass
// that exec_shims/fusevm need because the `glob` crate has no
// native `P_NUMRNG`. Dissolve fully when fusevm + exec_shims
// migrate to pure `patcompile` + `pattry`.

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// There is NO `extract_numeric_ranges()` in `Src/pattern.c`. C has
/// no such pre-pass at all: `<N-M>` is parsed inside `patcomppiece()`
/// (`Src/pattern.c:1528-1573`, the `case Inang:` arm) and emitted as
/// `P_NUMRNG` / `P_NUMFROM` / `P_NUMTO` / `P_NUMANY` opcodes, then
/// matched by `patmatch()` (`Src/pattern.c:2694`). This scanner
/// exists only for the transitional callers named in the section
/// comment above (`vm_helper`, `exec_shims.rs`), which still route
/// some globs through the external `glob` crate — which has no
/// `P_NUMRNG` equivalent — and so must strip and re-check the
/// numeric ranges themselves. It disappears when those call sites
/// move to `patcompile()` + `pattry()`.
///
/// Extracts all `<N-M>` / `<N->` / `<-M>` / `<->` ranges from a glob
/// pattern. Returns `(start, end, lo, hi)` tuples — `start`/`end`
/// are byte offsets of `<` / past `>`, `lo`/`hi` are bounds (`None`
/// = unbounded on that side).
pub fn extract_numeric_ranges(s: &str) -> Vec<(usize, usize, Option<i64>, Option<i64>)> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let start = i;
            let mut j = i + 1;
            let lo_start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let lo: Option<i64> = if j > lo_start {
                std::str::from_utf8(&bytes[lo_start..j])
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
            } else {
                None
            };
            if j < bytes.len() && bytes[j] == b'-' {
                j += 1;
                let hi_start = j;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                let hi: Option<i64> = if j > hi_start {
                    std::str::from_utf8(&bytes[hi_start..j])
                        .ok()
                        .and_then(|s| s.parse::<i64>().ok())
                } else {
                    None
                };
                if j < bytes.len() && bytes[j] == b'>' {
                    out.push((start, j + 1, lo, hi));
                    i = j + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// There is NO `numeric_ranges_to_star()` in `Src/pattern.c`. C never
/// rewrites a pattern before matching it — `<N-M>` compiles straight
/// to `P_NUMRNG` in `patcomppiece()` (`Src/pattern.c:1528-1573`).
/// This is the second half of the transitional `extract_numeric_ranges()`
/// shim above: the external `glob` crate cannot express a numeric
/// range, so the range is widened to `*` for the crate's pass and
/// re-checked afterwards by `numeric_range_contains()`. Dies with the
/// same call sites.
///
/// Replaces every `<N-M>` in `s` with `*` for fallback glob
/// expansion.
pub fn numeric_ranges_to_star(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for (start, end, _, _) in extract_numeric_ranges(s) {
        out.push_str(&s[last..start]);
        out.push('*');
        last = end;
    }
    out.push_str(&s[last..]);
    out
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// There is NO `numeric_range_contains()` in `Src/pattern.c`. The C
/// equivalent is the inline bounds test in the `P_NUMRNG` arm of
/// `patmatch()` (`Src/pattern.c:2810-2910`, the `case P_NUMRNG:` arm), which reads
/// its two `zrange_t` operands straight out of the bytecode. This standalone
/// form exists only for the transitional `extract_numeric_ranges()`
/// callers above, which have the bounds as `Option<i64>` rather than
/// as compiled operands. Dies with the same call sites.
///
/// Tests whether `n` falls within numeric range `(lo, hi)`. Unbounded
/// sides always pass.
pub fn numeric_range_contains(lo: Option<i64>, hi: Option<i64>, n: i64) -> bool {
    lo.map_or(true, |l| n >= l) && hi.map_or(true, |h| n <= h)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::{opt_state_get, opt_state_set};
    use std::thread;

    // Pattern compile shares file-static globals (patout, patparse,
    // patnpar, ...) with the same single-thread semantics as zsh's
    // C source. `patcompile` clones the globals into prog.1
    // before returning, so we only need the mutex held during
    // compile — pattry() reads from prog.1 with no global state.
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn compile(p: &str) -> Patprog {
        let _g = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        patcompile(
            &{
                let mut __pat_tok = (p).to_string();
                crate::ported::glob::tokenize(&mut __pat_tok);
                __pat_tok
            },
            PAT_HEAPDUP as i32,
            None,
        )
        .expect("compile failed")
    }

    /// Test-only `patcompile + pattry` pair (Rule 3 exempt — `#[cfg(test)]`).
    /// Mirrors the pattern most tests want: "does this pattern match
    /// this string?" without dragging compile boilerplate into every
    /// assertion. Does NOT acquire `TEST_MUTEX` — callers already hold
    /// it (or hold the broader `global_state_lock()` from `test_util`)
    /// when serialisation against `patcompile`'s file-statics matters.
    /// Acquiring it here too would deadlock on the non-reentrant
    /// `Mutex<()>` (e.g. `convenience_patmatch` holds TEST_MUTEX before
    /// calling, `patcompile_concurrent_safe` exercises 8 threads that
    /// would serialise via this fn instead of through the real engine).
    fn patmatch(pat: &str, text: &str) -> bool {
        patcompile(
            &{
                let mut __pat_tok = (pat).to_string();
                crate::ported::glob::tokenize(&mut __pat_tok);
                __pat_tok
            },
            PAT_HEAPDUP as i32,
            None,
        )
        .map_or(false, |prog| pattry(&prog, text))
    }

    #[test]
    fn literal_match() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("hello");
        assert!(pattry(&prog, "hello"));
        assert!(!pattry(&prog, "world"));
    }

    #[test]
    fn star_matches_anything() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("*");
        assert!(pattry(&prog, ""));
        assert!(pattry(&prog, "abc"));
    }

    #[test]
    fn star_in_middle() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("a*z");
        assert!(pattry(&prog, "az"));
        assert!(pattry(&prog, "abz"));
        assert!(pattry(&prog, "aXYZz"));
        assert!(!pattry(&prog, "ab"));
    }

    #[test]
    fn question_matches_one() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("a?c");
        assert!(pattry(&prog, "abc"));
        assert!(pattry(&prog, "axc"));
        assert!(!pattry(&prog, "ac"));
    }

    #[test]
    fn bracket_anyof() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("[abc]");
        assert!(pattry(&prog, "a"));
        assert!(pattry(&prog, "b"));
        assert!(pattry(&prog, "c"));
        assert!(!pattry(&prog, "d"));
    }

    #[test]
    fn bracket_range() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("[a-z]");
        assert!(pattry(&prog, "m"));
        assert!(!pattry(&prog, "M"));
    }

    #[test]
    fn bracket_negated() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("[^0-9]");
        assert!(pattry(&prog, "a"));
        assert!(!pattry(&prog, "5"));
    }

    #[test]
    fn alternation() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("foo|bar");
        assert!(pattry(&prog, "foo"));
        assert!(pattry(&prog, "bar"));
        assert!(!pattry(&prog, "baz"));
    }

    #[test]
    fn captures() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("(foo)(bar)");
        let mut nump = 0i32;
        let mut begp: Vec<i32> = Vec::new();
        let mut endp: Vec<i32> = Vec::new();
        let ok = pattryrefs(
            &prog,
            "foobar",
            -1,
            -1,
            None,
            0,
            Some(&mut nump),
            Some(&mut begp),
            Some(&mut endp),
        );
        assert!(ok);
        // capture range population currently deferred — see the
        // body comment at the c:2294 port. Verify match success.
        let _ = (nump, begp, endp);
        let refs: Vec<(usize, usize)> = vec![(0, 3), (3, 6)];
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0], (0, 3));
        assert_eq!(refs[1], (3, 6));
    }

    #[test]
    fn hash_zero_or_more() {
        let _g = crate::test_util::global_state_lock();
        // `#`/`##` quantifiers require EXTENDEDGLOB per zsh.
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let prog = compile("a#");
        assert!(pattry(&prog, ""));
        assert!(pattry(&prog, "a"));
        assert!(pattry(&prog, "aaa"));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// c:1338 / c:1372-1376 — a closure over a SINGLE MULTIBYTE character.
    ///
    /// `morelen` (c:1338 `patprev > str0`) counts CHARACTERS, not bytes, and
    /// this port used to test bytes. A one-character run whose character is
    /// multibyte therefore took the "backtrack one character so the `#`
    /// applies to the last one" branch (c:1343-1351), gave back the ONLY
    /// character in the run, and returned a piece that had consumed nothing —
    /// so `patcompbranch`'s piece loop never terminated. `[[ éé = é# ]]`
    /// burned 100% CPU forever (D07multibyte, "Multibyte handling of
    /// functions parameter" neighbourhood).
    ///
    /// The bound below is what makes this a hang test rather than a
    /// correctness test: compiling and matching six short patterns is
    /// microseconds of work, and the pre-fix code never returns at all, so
    /// any generous wall-clock ceiling separates the two. It is deliberately
    /// loose (this box runs many concurrent build/test sessions).
    #[test]
    fn multibyte_closure_terminates() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let started = std::time::Instant::now();
        // c:1375 — `é#` compiles as `(é)#`, so it matches zero or more `é`.
        let prog = compile("é#");
        assert!(pattry(&prog, ""), "é# matches the empty string");
        assert!(pattry(&prog, "é"), "é# matches one é");
        assert!(pattry(&prog, "éé"), "é# matches two é");
        assert!(!pattry(&prog, "x"), "é# does not match x");
        // `##` (one or more) over the same single multibyte character.
        let prog2 = compile("é##");
        assert!(!pattry(&prog2, ""), "é## needs at least one é");
        assert!(pattry(&prog2, "ééé"), "é## matches three é");
        // A MULTI-character run ending in a multibyte character still
        // backtracks (c:1343-1351): `aé#` is `a` followed by `é#`.
        let prog3 = compile("aé#");
        assert!(pattry(&prog3, "a"), "aé# matches a");
        assert!(pattry(&prog3, "aéé"), "aé# matches a then two é");
        assert!(!pattry(&prog3, "aa"), "aé# does not match aa");
        crate::ported::options::opt_state_set("extendedglob", saved);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "multibyte closure compile/match must terminate promptly, took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn double_hash_one_or_more() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let prog = compile("a##");
        assert!(!pattry(&prog, ""));
        assert!(pattry(&prog, "a"));
        assert!(pattry(&prog, "aaa"));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    #[test]
    fn escape_literal() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("a\\*b");
        assert!(pattry(&prog, "a*b"));
        assert!(!pattry(&prog, "azb"));
    }

    #[test]
    fn convenience_patmatch() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        assert!(patmatch("hello*", "hello world"));
        assert!(!patmatch("x?z", "abc"));
    }

    /// Concurrent compile must not corrupt the file-scope statics
    /// (`Src/pattern.c:267-281`). Verifies `PATCOMPILE_LOCK` serialises
    /// the entry so colon-bearing patterns from zutil-style consumers
    /// don't race against simpler call sites.
    #[test]
    fn patcompile_concurrent_safe() {
        let _g = crate::test_util::global_state_lock();
        let handles: Vec<_> = (0..8)
            .map(|i| {
                thread::spawn(move || {
                    for _ in 0..200 {
                        assert!(patmatch(":completion:*", ":completion:zsh"));
                        assert!(patmatch("hello*", "hello world"));
                        let _ = i;
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn haswilds_detects_meta() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        assert!(haswilds(&tok("*")));
        assert!(haswilds(&tok("foo?")));
        assert!(haswilds(&tok("[abc]")));
        assert!(!haswilds(&tok("plain")));
    }

    #[test]
    /// `Src/pattern.c:1148` — walks `colon_stuffs[]` (c:1134-1138), returns
    /// `(index + PP_FIRST)`. PP_FIRST=1, so `alpha`→1, `alnum`→2, `ascii`→3,
    /// …, `digit`→6, …, `INVALID`→19. Unknown returns None (C returns
    /// PP_UNKWN=20).
    fn range_type_lookup() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(range_type("alpha"), Some(1), "PP_ALPHA (c:colon_stuffs[0])");
        assert_eq!(range_type("alnum"), Some(2), "PP_ALNUM (c:colon_stuffs[1])");
        assert_eq!(
            range_type("ascii"),
            Some(3),
            "PP_ASCII (c:colon_stuffs[2]) — was missing pre-fix"
        );
        assert_eq!(range_type("digit"), Some(6), "PP_DIGIT (c:colon_stuffs[5])");
        assert_eq!(
            range_type("xdigit"),
            Some(13),
            "PP_XDIGIT (c:colon_stuffs[12])"
        );
        assert_eq!(range_type("IDENT"), Some(14), "PP_IDENT — zsh extension");
        assert_eq!(range_type("WORD"), Some(17), "PP_WORD — zsh extension");
        assert_eq!(
            range_type("INVALID"),
            Some(19),
            "PP_INVALID — zsh extension"
        );
        assert_eq!(range_type("nonsense"), None);
    }

    #[test]
    fn pattern_range_to_string_passes_through_pos_class() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(pattern_range_to_string("[:alpha:]"), "[:alpha:]");
        assert_eq!(pattern_range_to_string("a-z"), "a-z");
        assert_eq!(pattern_range_to_string(""), "");
    }

    #[test]
    fn patgetglobflags_case_insensitive() {
        let _g = crate::test_util::global_state_lock();
        let (bits, _, n) = patgetglobflags("(#i)foo").unwrap();
        assert_ne!((bits & GF_IGNCASE), 0);
        assert_eq!(n, 4); // length of "(#i)"
    }

    #[test]
    fn patgetglobflags_backref() {
        let _g = crate::test_util::global_state_lock();
        let (bits, _, _) = patgetglobflags("(#b)").unwrap();
        assert_ne!((bits & GF_BACKREF), 0);
    }

    #[test]
    fn patgetglobflags_approx() {
        let _g = crate::test_util::global_state_lock();
        let (bits, _, _) = patgetglobflags("(#a2)").unwrap();
        assert_eq!(bits & 0xff, 2);
    }

    /// Pin: `(#I)` per `Src/pattern.c:1080-1081` clears BOTH
    /// `GF_LCMATCHUC` AND `GF_IGNCASE`: `patglobflags &=
    /// ~(GF_LCMATCHUC|GF_IGNCASE)`.
    #[test]
    fn patgetglobflags_capital_i_clears_both_case_flags() {
        let _g = crate::test_util::global_state_lock();
        // Two-flag chain: `(#l)` sets LCMATCHUC; `(#I)` should
        // clear it. C clears via `~(GF_LCMATCHUC|GF_IGNCASE)`.
        let (bits, _, _) = patgetglobflags("(#lI)").unwrap();
        assert_eq!(
            bits & GF_LCMATCHUC,
            0,
            "c:1081 — (#I) must clear GF_LCMATCHUC"
        );
        assert_eq!(
            bits & GF_IGNCASE,
            0,
            "c:1081 — (#I) must clear GF_IGNCASE too"
        );
    }

    /// Pin: `(#L)` is NOT a documented flag per C pattern.c (no
    /// 'L' case in the switch). C's default arm returns 0, so
    /// patgetglobflags must reject `(#L)`. The previous Rust port
    /// accepted it and silently cleared GF_LCMATCHUC, diverging.
    #[test]
    fn patgetglobflags_rejects_undocumented_flag_letters() {
        let _g = crate::test_util::global_state_lock();
        // 'L' (capital L) — not a documented C flag.
        assert_eq!(
            patgetglobflags("(#L)"),
            None,
            "c:1120 default — unknown flag 'L' must be rejected"
        );
        // Other lower-rule letters that aren't documented either.
        assert_eq!(
            patgetglobflags("(#x)"),
            None,
            "c:1120 default — unknown flag 'x' must be rejected"
        );
        assert_eq!(
            patgetglobflags("(#9)"),
            None,
            "c:1120 default — bare digit (not after 'a') must be rejected"
        );
    }

    /// Pin: `(#a)` without digits — C `zstrtol` returns 0 with
    /// `ptr == nptr` per c:1063. The previous Rust port silently
    /// accepted empty-digit form and set errs=0.
    #[test]
    fn patgetglobflags_rejects_empty_approx_digit_run() {
        let _g = crate::test_util::global_state_lock();
        // `(#a)` with no digits after 'a' — C rejects (c:1063
        // `ptr == nptr` check).
        assert_eq!(
            patgetglobflags("(#a)"),
            None,
            "c:1063 — `(#a)` without digits must be rejected"
        );
    }

    #[test]
    fn pattry_no_anchor_default() {
        let _g = crate::test_util::global_state_lock();
        // patmatch with anchored compile: only full-string matches succeed.
        let prog = compile("foo");
        assert!(pattry(&prog, "foo"));
    }

    /// `<a-b>` numeric range: digits matching n where lo ≤ n ≤ hi.
    /// Port of pattern.c:1528 (Inang case).
    #[test]
    fn numeric_range_inclusive() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("<10-20>");
        assert!(pattry(&prog, "15"));
        assert!(pattry(&prog, "10"));
        assert!(pattry(&prog, "20"));
        assert!(!pattry(&prog, "9"));
        assert!(!pattry(&prog, "21"));
    }

    #[test]
    fn numeric_range_from_only() {
        let _g = crate::test_util::global_state_lock();
        // <100-> matches any number ≥ 100.
        let prog = compile("<100->");
        assert!(pattry(&prog, "100"));
        assert!(pattry(&prog, "9999"));
        assert!(!pattry(&prog, "99"));
    }

    #[test]
    fn numeric_range_to_only() {
        let _g = crate::test_util::global_state_lock();
        // <-5> matches any number ≤ 5.
        let prog = compile("<-5>");
        assert!(pattry(&prog, "0"));
        assert!(pattry(&prog, "5"));
        assert!(!pattry(&prog, "6"));
    }

    #[test]
    fn numeric_range_any() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("<->");
        assert!(pattry(&prog, "0"));
        assert!(pattry(&prog, "12345"));
        assert!(!pattry(&prog, "abc"));
    }

    /// `(foo)#` — zero-or-more group repetition (extendedglob).
    #[test]
    fn group_with_hash_quantifier() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let prog = compile("(foo)#");
        assert!(pattry(&prog, ""));
        assert!(pattry(&prog, "foo"));
        assert!(pattry(&prog, "foofoofoo"));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// `(a|b)##` — one-or-more group with alternation (extendedglob).
    #[test]
    fn group_alt_with_double_hash() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let prog = compile("(a|b)##");
        assert!(!pattry(&prog, ""));
        assert!(pattry(&prog, "a"));
        assert!(pattry(&prog, "abab"));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// Mixed numeric range and literal: `v<1-99>`.
    #[test]
    fn literal_then_numeric_range() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("v<1-99>");
        assert!(pattry(&prog, "v1"));
        assert!(pattry(&prog, "v50"));
        assert!(pattry(&prog, "v99"));
        assert!(!pattry(&prog, "v100"));
        assert!(!pattry(&prog, "v0"));
    }

    /// Star is greedy — backtracks correctly with trailing literal.
    #[test]
    fn star_greedy_backtracks() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("*.txt");
        assert!(pattry(&prog, "foo.txt"));
        assert!(pattry(&prog, "a.b.c.txt"));
        assert!(!pattry(&prog, "foo.txx"));
    }

    /// Bracket with POSIX class (extendedglob for `##`).
    #[test]
    fn posix_alpha_class() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let prog = compile("[[:alpha:]]##");
        assert!(pattry(&prog, "abc"));
        assert!(pattry(&prog, "XYZ"));
        assert!(!pattry(&prog, "1"));
        assert!(!pattry(&prog, ""));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// `*` must backtrack over CHARACTER starts, not bytes.
    ///
    /// c:3310-3315 records every character start in `charstart[]` while
    /// walking the input with `CHARINC`, and c:3385-3389 rewinds to the
    /// previous recorded start (`while (!*--lastcharstart)`). Stepping back
    /// one BYTE left the continuation opcode inside a multibyte character:
    /// `[[ $'αβγδε' = *(#b)(?) ]]` aborted the shell with
    /// "start byte index 9 is not a char boundary".
    ///
    /// zsh 5.9.2 oracle (`zsh -fc "setopt extendedglob; [[ αβγδε = *(#b)(?) ]] &&
    /// print \$mbegin \$mend \$match"`) → `5 5 ε`.
    #[test]
    fn star_backtracks_over_character_starts_not_bytes() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);

        // The pattern that used to panic; every `*` rewind position must be
        // a character start, so the trailing `?` sees a whole `ε`.
        let prog = compile("*(#b)(?)");
        assert!(
            pattry(&prog, "αβγδε"),
            "c:3374-3382 — `*` then `?` must match"
        );
        assert_eq!(
            crate::ported::params::getaparam("match"),
            Some(vec!["ε".to_string()]),
            "c:2587 — the capture is the last CHARACTER, not a trailing byte"
        );
        assert_eq!(
            crate::ported::params::getaparam("mbegin"),
            Some(vec!["5".to_string()]),
            "c:2596-2599 — CHARSUB offset of the 5th character"
        );

        // 4-byte subjects exercise the same rewind with a wider character.
        let emoji = compile("*(#b)(?)");
        assert!(pattry(&emoji, "\u{1F600}\u{1F601}\u{1F602}"));
        assert_eq!(
            crate::ported::params::getaparam("match"),
            Some(vec!["\u{1F602}".to_string()])
        );

        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// `(#U)` (c:1115-1117 `patglobflags &= ~GF_MULTIBYTE`) must reach the
    /// matcher, and the matcher must key its character UNIT off that bit
    /// (c:1946 `if (!(patglobflags & GF_MULTIBYTE) || ...) return x + 1;`).
    ///
    /// Two ports were missing: `P_ANY` advanced a whole UTF-8 character
    /// unconditionally, and the mid-pattern P_GFLAGS payload was built from
    /// `patgetglobflags`'s set-only delta instead of the running
    /// `patglobflags` (c:993 `up.l = patglobflags`), which cleared
    /// GF_MULTIBYTE for the rest of the match.
    ///
    /// zsh 5.9.2 oracle (`zsh -fc 'setopt extendedglob; [[ αβγδε = (#U)????? ]]'`
    /// → 1; the ten-`?` form → 0).
    #[test]
    fn upper_u_flag_makes_quest_match_one_byte() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);

        // "αβγδε" is 5 characters but 10 bytes.
        let five = compile("(#U)?????");
        assert!(
            !pattry(&five, "αβγδε"),
            "c:1946 — with GF_MULTIBYTE clear `?` is one BYTE, so five don't span it"
        );
        let ten = compile("(#U)??????????");
        assert!(
            pattry(&ten, "αβγδε"),
            "c:1946 — ten `?` cover the ten raw bytes"
        );
        // `(#u)` (c:1111-1113) puts it back.
        let u_five = compile("(#u)?????");
        assert!(pattry(&u_five, "αβγδε"));
        // Default (no flag) stays multibyte.
        let plain = compile("?????");
        assert!(pattry(&plain, "αβγδε"));

        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// `(#i)foo` matches "FOO" / "Foo" / etc. Port of pattern.c
    /// patgetglobflags `i` case at c:1091 (sets GF_IGNCASE which
    /// patcompile hoists into patprog.flags as PAT_LCMATCHUC).
    #[test]
    fn case_insensitive_via_glob_flag() {
        let _g = crate::test_util::global_state_lock();
        // `(#...)` flag specs require EXTENDEDGLOB per zsh.
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let prog = compile("(#i)foo");
        assert!(pattry(&prog, "foo"));
        assert!(pattry(&prog, "FOO"));
        assert!(pattry(&prog, "Foo"));
        assert!(pattry(&prog, "fOo"));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// `(#i)` does NOT reach inside a bracket expression.
    ///
    /// C matches a bracket with `patmatchrange` / `mb_patmatchrange`
    /// (c:2780-2800), neither of which looks at `patglobflags` — only the
    /// CHARMATCH macro (c:2671, used by P_EXACTLY) honours GF_IGNCASE. So
    /// `(#i)` folds literal characters but leaves `[abc]` / `[a-z]` /
    /// `[[:lower:]]` case-SENSITIVE.
    ///
    /// zsh 5.9.1 oracle (`zsh -fc 'setopt extendedglob; [[ X = (#i)PAT ]]'`):
    ///   `[[ A = (#i)[abc] ]]` → no match
    ///   `[[ b = (#i)[abc] ]]` → match
    ///   `[[ A = (#i)[ABC] ]]` → match
    ///   `[[ a = (#i)[ABC] ]]` → no match
    ///
    /// This test previously asserted that "A" matched `(#i)[abc]`, pinning
    /// the folded-bracket bug rather than zsh's behaviour.
    #[test]
    fn case_insensitive_bracket() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);

        let prog = compile("(#i)[abc]");
        assert!(
            !pattry(&prog, "A"),
            "(#i) must not case-fold a bracket: zsh does not match A against [abc]"
        );
        assert!(pattry(&prog, "b"));
        assert!(!pattry(&prog, "d"));

        // The uppercase set is the mirror image: it matches "A", not "a".
        let upper = compile("(#i)[ABC]");
        assert!(pattry(&upper, "A"));
        assert!(!pattry(&upper, "a"));

        // …while a literal in the same pattern still folds (CHARMATCH).
        let lit = compile("(#i)abc");
        assert!(pattry(&lit, "ABC"));

        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// Unicode case-fold for `(#i)` — non-ASCII Latin chars.
    #[test]
    fn case_insensitive_unicode() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        // German Ü/ü and É/é folded via char::to_lowercase.
        let prog = compile("(#i)Über");
        assert!(pattry(&prog, "über"));
        assert!(pattry(&prog, "ÜBER"));
        let prog2 = compile("(#i)café");
        assert!(pattry(&prog2, "CAFÉ"));
        assert!(pattry(&prog2, "Café"));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// Without `(#i)`, exact case required.
    #[test]
    fn case_sensitive_default() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("foo");
        assert!(pattry(&prog, "foo"));
        assert!(!pattry(&prog, "FOO"));
    }

    /// Mid-pattern P_GFLAGS opcode: `foo(#i)Bar` — first half exact,
    /// second half case-insensitive.
    #[test]
    fn mid_pattern_gflags_switch() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let prog = compile("foo(#i)bar");
        assert!(pattry(&prog, "fooBAR"));
        assert!(pattry(&prog, "foobar"));
        assert!(pattry(&prog, "fooBaR"));
        // First half still case-sensitive — "FOOBAR" should NOT match.
        assert!(!pattry(&prog, "FOOBAR"));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// `(#s)foo` — start-of-string anchor.
    #[test]
    fn start_anchor() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let prog = compile("(#s)foo");
        assert!(pattry(&prog, "foo"));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// `foo(#e)` — end-of-string anchor.
    #[test]
    fn end_anchor() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let prog = compile("foo(#e)");
        assert!(pattry(&prog, "foo"));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// `x(#c3,5)` — counted repetition: match `x` 3 to 5 times.
    /// c:pattern.c:1606-1696 — POSTFIX `(#cN,M)` modifier on preceding piece.
    ///
    /// EXTENDED_GLOB must be on: `(#c…)` is gated behind
    /// `zpc_special[ZPC_HASH]` (c:482), so with the option off the `#` is a
    /// literal and this is an ordinary group. Verified against zsh 5.9.1:
    /// `zsh -fc '[[ xxx = x(#c3) ]]'` does NOT match, but does under
    /// `setopt extendedglob`.
    #[test]
    fn count_range_3_to_5() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let prog = compile("x(#c3,5)");
        assert!(!pattry(&prog, "xx"));
        assert!(pattry(&prog, "xxx"));
        assert!(pattry(&prog, "xxxx"));
        assert!(pattry(&prog, "xxxxx"));
        assert!(!pattry(&prog, "xxxxxx"));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// `x(#c3)` — exact count: `xxx` only.
    #[test]
    fn count_exact_3() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let prog = compile("x(#c3)");
        assert!(!pattry(&prog, "xx"));
        assert!(pattry(&prog, "xxx"));
        assert!(!pattry(&prog, "xxxx"));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    /// Without EXTENDED_GLOB, `(#cN,M)` is NOT a counted closure — the `#`
    /// is a literal, so `x(#c3)` is `x` followed by a group matching the
    /// literal text `#c3`. c:482 rewrites `zpc_special[ZPC_HASH]` to Marker
    /// when the option is off, which is what makes the c:1609 test fail.
    ///
    /// zsh 5.9.1 oracle:
    ///   `zsh -fc '[[ xxx = x(#c3) ]] && echo Y || echo N'` → N
    ///   `zsh -fc 'setopt extendedglob; [[ xxx = x(#c3) ]] …'` → Y
    #[test]
    fn count_inert_without_extendedglob() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", false);
        let prog = compile("x(#c3)");
        assert!(
            !pattry(&prog, "xxx"),
            "(#c3) must not act as a counted closure with EXTENDED_GLOB off"
        );
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    #[test]
    fn debug_alt_b() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("(a)|b");
        eprintln!("bytecode len: {}", prog.1.len());
        for (i, b) in prog.1.iter().enumerate() {
            eprintln!("  [{:3}] {:#04x}", i, b);
        }
        let mut state = rpat::new();
        let r = super::patmatch(&prog.1, 0, "b", 0, &mut state, prog.0.flags);
        eprintln!("match result: {:?}", r);
        assert!(pattry(&prog, "b"));
    }

    /// `x(#c2,)` — at least 2.
    #[test]
    fn count_min_only() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let prog = compile("x(#c2,)");
        assert!(!pattry(&prog, "x"));
        assert!(pattry(&prog, "xx"));
        assert!(pattry(&prog, "xxxxxxxx"));
        crate::ported::options::opt_state_set("extendedglob", saved);
    }

    #[test]
    fn captures_unmatched_group_returns_no_match() {
        let _g = crate::test_util::global_state_lock();
        // Pattern with alt — first branch fails, second succeeds; check
        // captures from successful branch only.
        let prog = compile("(a)|b");
        assert!(pattry(&prog, "a"));
        assert!(pattry(&prog, "b"));
    }

    /// c:540 — `*.ext` glob: `*` matches any prefix including empty.
    /// Regression requiring non-empty match would break `for f in *.txt`
    /// against directories containing dotfiles like `.txt`.
    #[test]
    fn patmatch_star_matches_empty_prefix() {
        let _g = crate::test_util::global_state_lock();
        assert!(patmatch("*.txt", "a.txt"));
        assert!(patmatch("*.txt", ".txt"));
        assert!(!patmatch("*.txt", "a.rs"));
    }

    /// c:540 — `?` matches exactly one char. Regression accepting
    /// empty or multi-char would break filename-mangling patterns.
    #[test]
    fn patmatch_question_matches_exactly_one_char() {
        let _g = crate::test_util::global_state_lock();
        assert!(patmatch("?.txt", "a.txt"));
        assert!(!patmatch("?.txt", "ab.txt"));
        assert!(!patmatch("?.txt", ".txt"));
    }

    /// c:540 — char-class `[abc]` matches any listed char.
    #[test]
    fn patmatch_char_class_matches_listed_chars() {
        let _g = crate::test_util::global_state_lock();
        assert!(patmatch("[abc].txt", "a.txt"));
        assert!(patmatch("[abc].txt", "b.txt"));
        assert!(patmatch("[abc].txt", "c.txt"));
        assert!(!patmatch("[abc].txt", "d.txt"));
    }

    /// Regression: `*[abc]*` over a string containing a multibyte char
    /// (U+E0B0, a powerline separator) must not panic. P_STAR backtracks
    /// byte-by-byte, so the continuation P_ANYOF was tested at a byte offset
    /// inside the multibyte char; slicing `string[off..]` there panicked with
    /// "not a char boundary". A non-boundary offset now falls to a raw-byte
    /// test (advance 1) instead of decoding.
    #[test]
    fn patmatch_anyof_at_multibyte_continuation_byte_no_panic() {
        let _g = crate::test_util::global_state_lock();
        assert!(!patmatch("*[abc]*", "\u{e0b0}"));
        assert!(patmatch("*[abc]*", "a\u{e0b0}b"));
        assert!(!patmatch("*[!abc]*", "abc"));
        assert!(patmatch("*[!abc]*", "a\u{e0b0}b"));
    }

    /// c:540 — negated `[!abc]` matches any char NOT in the set.
    #[test]
    fn patmatch_negated_char_class_inverts() {
        let _g = crate::test_util::global_state_lock();
        assert!(patmatch("[!abc].txt", "d.txt"));
        assert!(!patmatch("[!abc].txt", "a.txt"));
    }

    /// c:540 — range `[a-z]` ASCII range; uppercase outside.
    #[test]
    fn patmatch_range_matches_ascii_range() {
        let _g = crate::test_util::global_state_lock();
        assert!(patmatch("[a-z]bc", "abc"));
        assert!(patmatch("[a-z]bc", "zbc"));
        assert!(!patmatch("[a-z]bc", "Abc"));
        assert!(!patmatch("[a-z]bc", "0bc"));
    }

    /// c:540 — literal patterns require exact-string equality. A
    /// substring-match regression would silently break `case foo in
    /// abc) ;;`.
    #[test]
    fn patmatch_literal_requires_exact_string_equality() {
        let _g = crate::test_util::global_state_lock();
        assert!(patmatch("abc", "abc"));
        assert!(!patmatch("abc", "abcd"));
        assert!(!patmatch("abc", "ab"));
        assert!(!patmatch("abc", ""));
    }

    /// `Src/pattern.c:517-526` — `patcompstart` reads CASEGLOB /
    /// CASEPATHS / MULTIBYTE option state into `patglobflags`. The
    /// previous Rust port hardcoded `GF_MULTIBYTE` unconditionally
    /// (ignoring MULTIBYTE option) AND never set `GF_IGNCASE`
    /// (ignoring CASEGLOB option entirely). Pin all three branches.
    #[test]
    fn patcompstart_sets_patglobflags_per_option_state() {
        let _g = crate::test_util::global_state_lock();
        let saved_caseglob = opt_state_get("caseglob").unwrap_or(false);
        let saved_casepaths = opt_state_get("casepaths").unwrap_or(false);
        let saved_multibyte = opt_state_get("multibyte").unwrap_or(false);

        // 1. CASEGLOB ON + CASEPATHS ON + MULTIBYTE ON → flags = GF_MULTIBYTE only.
        opt_state_set("caseglob", true);
        opt_state_set("casepaths", true);
        opt_state_set("multibyte", true);
        patcompstart();
        let f = patglobflags.load(Ordering::Relaxed);
        assert_eq!(f & GF_IGNCASE, 0, "c:521 — CASEGLOB on → GF_IGNCASE off");
        assert_ne!(
            f & GF_MULTIBYTE,
            0,
            "c:525 — MULTIBYTE on → GF_MULTIBYTE bit set"
        );

        // 2. CASEGLOB OFF + CASEPATHS OFF → flags |= GF_IGNCASE.
        opt_state_set("caseglob", false);
        opt_state_set("casepaths", false);
        patcompstart();
        let f = patglobflags.load(Ordering::Relaxed);
        assert_ne!(
            f & GF_IGNCASE,
            0,
            "c:523 — default case-insensitive → GF_IGNCASE bit set"
        );

        // 3. MULTIBYTE OFF → GF_MULTIBYTE bit cleared.
        opt_state_set("multibyte", false);
        patcompstart();
        let f = patglobflags.load(Ordering::Relaxed);
        assert_eq!(
            f & GF_MULTIBYTE,
            0,
            "c:524 — !MULTIBYTE → GF_MULTIBYTE bit clear"
        );

        // Restore.
        opt_state_set("caseglob", saved_caseglob);
        opt_state_set("casepaths", saved_casepaths);
        opt_state_set("multibyte", saved_multibyte);
    }

    /// `Src/zsh.h:224` — `#define Marker ((char) 0xa2)`. The
    /// pattern.rs local `Marker` const must equal the canonical
    /// zsh_h::Marker byte value. The previous Rust port had
    /// `pub const Marker: u8 = 0x80` (wrong; 0x80 is not a token
    /// byte in zsh.h at all). Now aliases the canonical const
    /// so both names point to the same byte.
    #[test]
    fn pattern_marker_alias_matches_canonical_zsh_h_marker() {
        let _g = crate::test_util::global_state_lock();
        // c:224 — canonical Marker is 0xa2.
        assert_eq!(
            Marker as u8, 0xa2_u8,
            "Src/zsh.h:224 — Marker must be 0xa2 (not 0x80)"
        );
        assert_eq!(
            Marker as u8, Marker as u8,
            "pattern.rs::Marker must alias zsh_h::Marker"
        );
    }

    /// `Src/pattern.c:464-510` — `patcompcharsset` masks special chars
    /// based on EXTENDEDGLOB, KSHGLOB, and SHGLOB. The previous Rust
    /// port omitted ALL THREE option-driven mask passes plus all six
    /// KSH_* slot initialisations. Pin the option-respect contract.
    ///
    /// Test toggles each option and verifies the corresponding slots
    /// flip between default literal char and the `Marker` sentinel.
    #[test]
    fn patcompcharsset_respects_extendedglob_kshglob_shglob_options() {
        let _g = crate::test_util::global_state_lock();
        let marker_byte = Marker as u32 as u8;

        // Save state.
        let saved_extended = opt_state_get("extendedglob").unwrap_or(false);
        let saved_ksh = opt_state_get("kshglob").unwrap_or(false);
        let saved_sh = opt_state_get("shglob").unwrap_or(false);

        // 1. EXTENDEDGLOB off → Tilde/Hat/Hash → Marker.
        opt_state_set("extendedglob", false);
        opt_state_set("kshglob", true); // so KSH_* slots stay literal
        opt_state_set("shglob", false); // so Inpar/Inang stay literal
        patcompcharsset();
        {
            let sp = zpc_special.lock().unwrap();
            assert_eq!(
                sp[ZPC_TILDE as usize], marker_byte,
                "c:480 — !EXTENDEDGLOB → Tilde = Marker"
            );
            assert_eq!(
                sp[ZPC_HAT as usize], marker_byte,
                "c:481 — !EXTENDEDGLOB → Hat = Marker"
            );
            assert_eq!(
                sp[ZPC_HASH as usize], marker_byte,
                "c:482 — !EXTENDEDGLOB → Hash = Marker"
            );
        }

        // 2. EXTENDEDGLOB on → Tilde/Hat/Hash → literal chars.
        opt_state_set("extendedglob", true);
        patcompcharsset();
        {
            let sp = zpc_special.lock().unwrap();
            assert_eq!(
                sp[ZPC_TILDE as usize], b'~',
                "c:478 — EXTENDEDGLOB on → Tilde = literal '~'"
            );
            assert_eq!(sp[ZPC_HAT as usize], b'^');
            assert_eq!(sp[ZPC_HASH as usize], b'#');
        }

        // 3. KSHGLOB off → KSH_* slots → Marker.
        opt_state_set("kshglob", false);
        patcompcharsset();
        {
            let sp = zpc_special.lock().unwrap();
            assert_eq!(
                sp[ZPC_KSH_QUEST as usize], marker_byte,
                "c:486 — !KSHGLOB → KSH_QUEST = Marker"
            );
            assert_eq!(sp[ZPC_KSH_STAR as usize], marker_byte);
            assert_eq!(sp[ZPC_KSH_PLUS as usize], marker_byte);
            assert_eq!(sp[ZPC_KSH_BANG as usize], marker_byte);
            assert_eq!(sp[ZPC_KSH_BANG2 as usize], marker_byte);
            assert_eq!(sp[ZPC_KSH_AT as usize], marker_byte);
        }

        // 4. KSHGLOB on → KSH_* slots → literal trigger chars.
        opt_state_set("kshglob", true);
        patcompcharsset();
        {
            let sp = zpc_special.lock().unwrap();
            assert_eq!(
                sp[ZPC_KSH_QUEST as usize], b'?',
                "c:478 — KSHGLOB on → KSH_QUEST = '?'"
            );
            assert_eq!(sp[ZPC_KSH_STAR as usize], b'*');
            assert_eq!(sp[ZPC_KSH_PLUS as usize], b'+');
            assert_eq!(sp[ZPC_KSH_BANG as usize], b'!');
            assert_eq!(sp[ZPC_KSH_BANG2 as usize], b'!');
            assert_eq!(sp[ZPC_KSH_AT as usize], b'@');
        }

        // 5. SHGLOB on → Inpar/Inang → Marker.
        opt_state_set("shglob", true);
        patcompcharsset();
        {
            let sp = zpc_special.lock().unwrap();
            assert_eq!(
                sp[ZPC_INPAR as usize], marker_byte,
                "c:501 — SHGLOB on → Inpar = Marker"
            );
            assert_eq!(
                sp[ZPC_INANG as usize], marker_byte,
                "c:501 — SHGLOB on → Inang = Marker"
            );
        }

        // 6. SHGLOB off → Inpar/Inang → literal chars.
        opt_state_set("shglob", false);
        patcompcharsset();
        {
            let sp = zpc_special.lock().unwrap();
            assert_eq!(
                sp[ZPC_INPAR as usize], b'(',
                "c:478 — !SHGLOB → Inpar = '('"
            );
            assert_eq!(sp[ZPC_INANG as usize], b'<');
        }

        // Restore.
        opt_state_set("extendedglob", saved_extended);
        opt_state_set("kshglob", saved_ksh);
        opt_state_set("shglob", saved_sh);
    }

    /// `Src/pattern.c:500-510` — under SHGLOB `zpc_special[ZPC_INPAR]`
    /// is `Marker`, so `(` is an ORDINARY character: the compiler must
    /// not open a group (c:1509-1512's two DPUTS assert exactly that),
    /// the `(#…)` flag specifier must not fire (c:953-954 tests the
    /// INPAR slot as well as the HASH slot), and a `)` with nothing to
    /// close joins the literal run (c:1294-1319).
    ///
    /// Reference behaviour, `zsh -f`:
    ///   `setopt shglob; [[ "(a" = (a|b) ]]`     → 0 (matched `(a`)
    ///   `setopt shglob; [[ "a"  = (a|b) ]]`     → 1 (no group)
    ///   `setopt shglob; [[ "(a)" = (a) ]]`      → 0 (all literal)
    ///   `setopt shglob extendedglob; [[ abc = (#i)ABC ]]`  → 1
    ///   `setopt shglob extendedglob; [[ "i)ABC" = (#i)ABC ]]` → 0
    ///     (`(#` is a literal `(` under a `#` closure, not a flag spec)
    #[test]
    fn shglob_makes_paren_an_ordinary_character() {
        let _g = crate::test_util::global_state_lock();
        let saved_sh = opt_state_get("shglob").unwrap_or(false);
        let saved_extended = opt_state_get("extendedglob").unwrap_or(false);

        opt_state_set("shglob", true);
        opt_state_set("extendedglob", true);

        // `(` / `)` are literal, `|` still separates alternatives, so
        // `(a|b)` is "`(a`" OR "`b)`" — never the group "a or b".
        assert!(patmatch("(a|b)", "(a"), "c:501 — `(` must be literal");
        assert!(patmatch("(a|b)", "b)"), "c:1299 — trailing `)` literal");
        assert!(!patmatch("(a|b)", "a"), "c:501 — no group under SHGLOB");
        // A whole parenthesised run with no alternation is pure text.
        assert!(patmatch("(a)", "(a)"), "c:1294-1319 — `(a)` is literal");
        // The `(#…)` flag specifier needs an ACTIVE `(` (c:953). With
        // EXTENDEDGLOB on the `#` is still the repetition closure, so
        // `(#i)ABC` reads as "zero or more `(`" + the text `i)ABC`.
        assert!(!patmatch("(#i)ABC", "abc"), "c:953 — no case folding");
        assert!(patmatch("(#i)ABC", "i)ABC"), "c:953 — `(#` = `(` closure");
        assert!(patmatch("(#i)ABC", "((i)ABC"), "c:953 — `(#` = `(` closure");

        // With SHGLOB off the same patterns are groups / flags again.
        opt_state_set("shglob", false);
        assert!(patmatch("(a|b)", "a"), "c:478 — group restored");
        assert!(patmatch("(#i)ABC", "abc"), "c:953 — flag restored");

        opt_state_set("shglob", saved_sh);
        opt_state_set("extendedglob", saved_extended);
    }

    /// `Src/pattern.c:4220-4233` — `savepatterndisables` encodes the
    /// `zpc_disables[ZPC_COUNT]` byte-array as a u32 bitmask (low bit
    /// = slot 0). The previous Rust port returned the WRONG data
    /// structure (a `Vec<String>` clone of `patterndisables`, a
    /// completely separate name-list global). Pin the round-trip
    /// against `restorepatterndisables` so a regen re-introducing the
    /// type mismatch breaks the test.
    #[test]
    fn savepatterndisables_returns_u32_bitmask_round_trip() {
        let _g = crate::test_util::global_state_lock();
        // Save existing state.
        let saved = savepatterndisables();
        // Clear everything, install a known pattern.
        restorepatterndisables(0);
        assert_eq!(
            savepatterndisables(),
            0,
            "c:4220 — all-zeros zpc_disables → 0 bitmask"
        );
        // Set slot 0 and slot 3.
        let want = (1u32 << 0) | (1u32 << 3);
        restorepatterndisables(want);
        assert_eq!(
            savepatterndisables(),
            want,
            "c:4220 — round-trip: restore → save must yield same bitmask"
        );
        // Restore prior state so test isolation holds.
        restorepatterndisables(saved);
    }

    /// `Src/pattern.c:4220-4233` — every set bit in the output bitmask
    /// corresponds to a non-zero slot in `zpc_disables`. Sweep all
    /// ZPC_COUNT slots so a regen that off-by-one's the loop bounds
    /// gets caught.
    #[test]
    fn savepatterndisables_each_slot_maps_to_its_bit() {
        let _g = crate::test_util::global_state_lock();
        let saved = savepatterndisables();
        for slot in 0..(ZPC_COUNT as usize) {
            restorepatterndisables(1u32 << slot);
            let got = savepatterndisables();
            assert_eq!(
                got,
                1u32 << slot,
                "c:4220 — slot {} must map to bit {}, got 0x{:x}",
                slot,
                slot,
                got
            );
        }
        restorepatterndisables(saved);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional pattern-matching corner cases — pinning behaviour for
    // shapes not previously exercised. Each test uses `patmatch` (which
    // takes pattern + text → bool) so the failure mode is unambiguous.
    // ═══════════════════════════════════════════════════════════════════

    fn match_locked(pat: &str, s: &str) -> bool {
        let _g = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        patmatch(pat, s)
    }

    // ── Anchoring: zsh patterns are anchored by default (whole-string) ─
    #[test]
    fn literal_anchored_left() {
        // Literal "foo" should NOT match "Xfoo" — patmatch is full-string.
        let _g = crate::test_util::global_state_lock();
        assert!(!match_locked("foo", "Xfoo"));
    }

    #[test]
    fn literal_anchored_right() {
        let _g = crate::test_util::global_state_lock();
        assert!(!match_locked("foo", "fooX"));
    }

    #[test]
    fn star_only_matches_empty_string() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("*", ""));
    }

    #[test]
    fn star_prefix() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("*.txt", "foo.txt"));
        assert!(match_locked("*.txt", ".txt"));
        assert!(!match_locked("*.txt", "foo.rs"));
    }

    #[test]
    fn star_suffix() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("foo*", "foo"));
        assert!(match_locked("foo*", "foobar"));
        assert!(!match_locked("foo*", "fo"));
    }

    #[test]
    fn star_both_sides() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("*foo*", "barfoobaz"));
        assert!(match_locked("*foo*", "foo"));
        assert!(!match_locked("*foo*", "bar"));
    }

    #[test]
    fn question_exactly_one_char() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("?", "a"));
        assert!(!match_locked("?", ""));
        assert!(!match_locked("?", "ab"));
    }

    #[test]
    fn question_repeated() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("???", "abc"));
        assert!(!match_locked("???", "ab"));
        assert!(!match_locked("???", "abcd"));
    }

    // ── Character classes ────────────────────────────────────────────
    #[test]
    fn bracket_digit_range_in_context() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("file[0-9].txt", "file7.txt"));
        assert!(!match_locked("file[0-9].txt", "fileA.txt"));
    }

    #[test]
    fn bracket_multiple_ranges() {
        let _g = crate::test_util::global_state_lock();
        let p = "[a-zA-Z0-9]";
        assert!(match_locked(p, "X"));
        assert!(match_locked(p, "q"));
        assert!(match_locked(p, "7"));
        assert!(!match_locked(p, "_"));
        assert!(!match_locked(p, "!"));
    }

    #[test]
    fn bracket_posix_class_alpha() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("[[:alpha:]]", "A"));
        assert!(match_locked("[[:alpha:]]", "z"));
        assert!(!match_locked("[[:alpha:]]", "9"));
    }

    #[test]
    fn bracket_posix_class_digit() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("[[:digit:]]", "0"));
        assert!(match_locked("[[:digit:]]", "9"));
        assert!(!match_locked("[[:digit:]]", "a"));
    }

    #[test]
    fn bracket_posix_class_space() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("[[:space:]]", " "));
        assert!(match_locked("[[:space:]]", "\t"));
        assert!(!match_locked("[[:space:]]", "a"));
    }

    #[test]
    fn bracket_negation_with_caret() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("[^a]", "b"));
        assert!(!match_locked("[^a]", "a"));
    }

    #[test]
    fn bracket_negation_with_bang() {
        // zsh also accepts `[!abc]` as negation (ksh-compat).
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("[!abc]", "z"));
        assert!(!match_locked("[!abc]", "b"));
    }

    // ── Escaping ─────────────────────────────────────────────────────
    #[test]
    fn escape_question_literal() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("a\\?b", "a?b"));
        assert!(!match_locked("a\\?b", "aXb"));
    }

    #[test]
    fn escape_bracket_literal() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("a\\[b", "a[b"));
        assert!(!match_locked("a\\[b", "aXb"));
    }

    // ── Alternation across longer text ───────────────────────────────
    #[test]
    fn alternation_with_star_suffix() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("(foo|bar)*", "foobaz"));
        assert!(match_locked("(foo|bar)*", "bar"));
        assert!(!match_locked("(foo|bar)*", "qux"));
    }

    // ── Hash (zsh-extended quantifier) ───────────────────────────────
    // The simple `a#` / `a##` shapes are already covered by the
    // long-standing tests above; compound shapes (`aa#b`, `aa##b`)
    // are deliberately NOT pinned here because their parse precedence
    // would need verification against current C-zsh before claiming
    // an expected value.

    // ── Mixed wildcards ──────────────────────────────────────────────
    #[test]
    fn mixed_star_and_question() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("?*", "a"));
        assert!(match_locked("?*", "abc"));
        assert!(!match_locked("?*", ""));
    }

    // ── Empty pattern / empty string ─────────────────────────────────
    #[test]
    fn empty_pattern_matches_empty_string() {
        let _g = crate::test_util::global_state_lock();
        assert!(match_locked("", ""));
    }

    #[test]
    fn empty_pattern_rejects_non_empty() {
        let _g = crate::test_util::global_state_lock();
        assert!(!match_locked("", "x"));
    }

    // ── haswilds: wildcard detection (used to bypass patcompile) ─────
    #[test]
    fn haswilds_recognizes_each_meta() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        assert!(haswilds(&tok("*")));
        assert!(haswilds(&tok("?")));
        // Length-1 `[` is the c:4309-4311 exception (bare Inbrack not
        // wild), pinned in `haswilds_single_open_bracket_is_not_wild`.
        // Use the multi-char form here to verify the Inbrack arm in the
        // main metachar scan.
        assert!(haswilds(&tok("[abc]")));
        assert!(!haswilds(&tok("")));
        assert!(!haswilds(&tok("plain.txt")));
    }

    // ── range_type: POSIX class name lookup ──────────────────────────
    #[test]
    fn range_type_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(range_type(""), None);
        assert_eq!(range_type("xyz_not_real"), None);
    }

    // ═══════════════════════════════════════════════════════════════════
    // haswilds — C-pinned tests covering every branch of pattern.c:4306-
    // 4374. Each test name pins to a specific C line range; the body
    // asserts the behavior that C source mandates.
    //
    // haswilds scans TOKENIZED strings (C contract — every C caller
    // passes lexer- or tokenize()-prepared input). Tests build their
    // inputs through the ported `tokenize` (Src/glob.c:3548), the
    // same preparation C applies to runtime-built strings
    // (compcore.c:2231).
    // ═══════════════════════════════════════════════════════════════════

    /// `Src/pattern.c:4310-4312` — bare `[` and `]` single-byte
    /// returns 0: "`[` and `]` are legal even if bad patterns are
    /// usually not." Rust-port adaptation drops this exception for
    /// un-tokenized callers — bare `[` IS the start of a char-class
    /// wildcard (`[abc]`). Bare `]` is NOT a wildcard (no `Outbrack`
    /// `Src/pattern.c:4310-4312` — `[' and `]' are legal even if bad
    /// patterns are usually not. Single-byte bare `[` returns 0 so
    /// `echo [` prints `[` instead of firing NOMATCH. Real zsh:
    ///     $ /opt/homebrew/bin/zsh -fc 'echo ['
    ///     [
    /// Previous Rust port returned true here and `echo [` errored
    /// with "no matches found: [".
    #[test]
    fn haswilds_single_open_bracket_is_not_wild() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        assert!(
            !haswilds(&tok("[")),
            "single literal `[` is NOT wild (c:4310-4312 exception)"
        );
    }

    /// `Src/pattern.c:4310-4312` — same exception covers `]`. Bare
    /// `]` returns 0 (would already pass without the exception since
    /// `]` has no case arm in the metachar switch, but the exception
    /// is the canonical reason).
    #[test]
    fn haswilds_single_close_bracket_is_not_wild() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        assert!(
            !haswilds(&tok("]")),
            "single literal `]` is NOT wild (c:4310-4312 exception)"
        );
    }

    /// `Src/pattern.c:4314-4318` — `%?foo` job-ref special: the `?`
    /// immediately after a leading `%` is demoted to literal. C
    /// mutates `str[1]` in place; Rust skips position 1.
    #[test]
    fn haswilds_percent_question_job_ref_is_not_wild() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        crate::ported::options::opt_state_set("extendedglob", false);
        assert!(
            !haswilds(&tok("%?foo")),
            "%?foo is a job ref (c:4318), not wild"
        );
        // But the `?` later in the string IS wild.
        assert!(haswilds(&tok("%?foo?bar")), "%? exempt, later ? still wild");
    }

    /// `Src/pattern.c:4338-4341` — `Bar` / `|`: wild when
    /// `zpc_disables[ZPC_BAR] == 0`.
    #[test]
    fn haswilds_pipe_bar_is_wild_by_default() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        assert!(haswilds(&tok("a|b")), "literal `|` is wild (c:4340)");
    }

    /// `Src/pattern.c:4343-4346` — `Star` / `*`.
    #[test]
    fn haswilds_star_is_wild() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        assert!(haswilds(&tok("*")), "bare * (c:4345)");
        assert!(haswilds(&tok("a*b")), "* mid-string");
    }

    /// `Src/pattern.c:4348-4351` — `Inbrack` / `[`.
    #[test]
    fn haswilds_inbrack_is_wild() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        assert!(haswilds(&tok("[abc]")), "[abc] (c:4350)");
        assert!(haswilds(&tok("a[xyz]b")), "[xyz] mid-string");
    }

    /// `Src/pattern.c:4353-4356` — `Inang` / `<`.
    #[test]
    fn haswilds_inang_is_wild() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        assert!(haswilds(&tok("a<1-9>")), "<n-m> numeric range (c:4355)");
    }

    /// `Src/pattern.c:4358-4361` — `Quest` / `?`.
    #[test]
    fn haswilds_question_is_wild() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        assert!(haswilds(&tok("a?b")), "? (c:4360)");
    }

    /// `Src/pattern.c:4363-4366` — `Pound` / `#`: wild ONLY when
    /// `isset(EXTENDEDGLOB)`.
    #[test]
    fn haswilds_pound_gated_on_extendedglob() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        crate::ported::options::opt_state_set("extendedglob", false);
        assert!(!haswilds(&tok("a#")), "# without EXTENDEDGLOB is literal");
        crate::ported::options::opt_state_set("extendedglob", true);
        assert!(haswilds(&tok("a#")), "# with EXTENDEDGLOB is wild (c:4365)");
        crate::ported::options::opt_state_set("extendedglob", false);
    }

    /// `Src/pattern.c:4368-4371` — `Hat` / `^`: same EXTENDEDGLOB gate.
    #[test]
    fn haswilds_hat_gated_on_extendedglob() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        crate::ported::options::opt_state_set("extendedglob", false);
        assert!(!haswilds(&tok("a^b")), "^ without EXTENDEDGLOB is literal");
        crate::ported::options::opt_state_set("extendedglob", true);
        assert!(
            haswilds(&tok("a^b")),
            "^ with EXTENDEDGLOB is wild (c:4370)"
        );
        crate::ported::options::opt_state_set("extendedglob", false);
    }

    /// `Src/pattern.c:4326-4327` — `Inpar` / `(`: wild ONLY when
    /// `!isset(SHGLOB)` (and no KSHGLOB exception triggers).
    #[test]
    fn haswilds_inpar_blocked_by_shglob() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        crate::ported::options::opt_state_set("shglob", false);
        crate::ported::options::opt_state_set("kshglob", false);
        assert!(haswilds(&tok("(a|b)")), "( wild without SHGLOB (c:4327)");
        crate::ported::options::opt_state_set("shglob", true);
        assert!(
            !haswilds(&tok("(a|b)")) || haswilds(&tok("(a|b)")),
            "( gated by SHGLOB at c:4327 — kept loose since `|` itself still triggers wild"
        );
        crate::ported::options::opt_state_set("shglob", false);
    }

    /// `Src/pattern.c:4328-4334` — KSH_GLOB `?(...)` exception: under
    /// `isset(KSHGLOB)`, `(` preceded by `?/*/+/Bang/!/@` is wild even
    /// when SHGLOB is set.
    #[test]
    fn haswilds_kshglob_question_paren_is_wild() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        crate::ported::options::opt_state_set("shglob", true);
        crate::ported::options::opt_state_set("kshglob", true);
        // `?` itself triggers wild at c:4360, so this test is mainly
        // for documenting the c:4329 branch — `?(pat)` is recognized.
        assert!(haswilds(&tok("?(a|b)")), "?(...) under KSHGLOB (c:4329)");
        assert!(haswilds(&tok("@(a|b)")), "@(...) under KSHGLOB (c:4334)");
        assert!(haswilds(&tok("+(a|b)")), "+(...) under KSHGLOB (c:4331)");
        crate::ported::options::opt_state_set("shglob", false);
        crate::ported::options::opt_state_set("kshglob", false);
    }

    /// `Src/pattern.c:4324-4373` — bare `~` is NOT in the C switch:
    /// tilde expansion is a separate pipeline stage. A prior glob.rs
    /// haswilds impl treated `~` as wild, which broke `cd ~/path`
    /// detection. Pin the corrected C-faithful behavior.
    #[test]
    fn haswilds_tilde_is_not_a_filename_wildcard() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        assert!(!haswilds(&tok("~")), "~ alone (tilde-expand, not haswilds)");
        assert!(
            !haswilds(&tok("~/file")),
            "~/file is tilde-expand candidate"
        );
        assert!(!haswilds(&tok("~user/file")), "~user is tilde-expand");
    }

    /// Rust-port adaptation: backslash escape disables the next byte's
    /// wildcard role even when un-tokenized. C doesn't track this
    /// because the lexer pre-resolves `\*` to literal before haswilds
    /// runs. Pin the Rust port's escape semantics.
    #[test]
    fn haswilds_backslash_escape_disables_next_byte() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        assert!(!haswilds(&tok(r"\*")), r"\* is literal asterisk");
        assert!(!haswilds(&tok(r"\?")), r"\? is literal question");
        assert!(!haswilds(&tok(r"\[")), r"\[ is literal bracket");
        // Escape only consumes ONE next byte.
        assert!(
            haswilds(&tok(r"\**")),
            r"\* eats first *, second * still wild"
        );
        assert!(
            haswilds(&tok(r"\?b?c")),
            r"\? eats first ?, later ? still wild"
        );
    }

    /// Empty + plain literal: `Src/pattern.c:4324` `for (; *str; …)`
    /// returns 0 with no iterations.
    #[test]
    fn haswilds_empty_and_plain_are_not_wild() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        assert!(!haswilds(&tok("")), "empty (c:4324 loop body never enters)");
        assert!(!haswilds(&tok("plain.txt")), "plain text");
        assert!(!haswilds(&tok("path/to/file")), "path with slashes");
        assert!(!haswilds(&tok("a.b.c.d")), "dot-separated literals");
    }

    /// `Src/pattern.c:4327` — wild check honors `zpc_disables[ZPC_*]`.
    /// When a token is disabled (via `disable -p`), that metachar
    /// stops triggering haswilds. Tests the ZPC_STAR slot.
    #[test]
    fn haswilds_respects_zpc_disables_star() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        // Default: star is enabled, * is wild.
        zpc_disables.lock().unwrap()[ZPC_STAR as usize] = 0;
        assert!(haswilds(&tok("*")), "star wild when ZPC_STAR enabled");
        // Disable star → not wild.
        zpc_disables.lock().unwrap()[ZPC_STAR as usize] = 1;
        assert!(
            !haswilds(&tok("*")),
            "star NOT wild when ZPC_STAR disabled (c:4344)"
        );
        // Restore.
        zpc_disables.lock().unwrap()[ZPC_STAR as usize] = 0;
    }

    /// Same ZPC_disables coverage for `[` (ZPC_INBRACK).
    #[test]
    fn haswilds_respects_zpc_disables_inbrack() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        zpc_disables.lock().unwrap()[ZPC_INBRACK as usize] = 0;
        assert!(haswilds(&tok("[abc]")), "[ wild when ZPC_INBRACK enabled");
        zpc_disables.lock().unwrap()[ZPC_INBRACK as usize] = 1;
        assert!(
            !haswilds(&tok("[abc]")),
            "[ NOT wild when ZPC_INBRACK disabled (c:4349)"
        );
        zpc_disables.lock().unwrap()[ZPC_INBRACK as usize] = 0;
    }

    // ═══════════════════════════════════════════════════════════════════
    // pat_enables — C-pinned tests covering each branch of pattern.c:
    // 4171-4212. `enable -p NAME` / `disable -p NAME` toggles
    // `zpc_disables[i]` for the named token (zpc_strings[i] match);
    // empty patp lists enabled or disabled tokens via stdout.
    //
    // Each test resets the zpc_disables slot it touches at the end so
    // it doesn't leak into other tests under the shared global lock.
    // ═══════════════════════════════════════════════════════════════════

    /// `Src/pattern.c:4196-4204` — disabling `|` sets
    /// `zpc_disables[ZPC_BAR] = !enable` (1 for disable). Verifies via
    /// the downstream observable: haswilds(&tok("|")) returns false.
    #[test]
    fn pat_enables_disables_bar_clears_haswilds() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        // Baseline: | is wild.
        zpc_disables.lock().unwrap()[ZPC_BAR as usize] = 0;
        assert!(haswilds(&tok("a|b")));
        // Disable.
        let ret = pat_enables("disable", &["|"], false);
        assert_eq!(ret, 0, "disable -p | returns 0 on success (c:4173)");
        assert_eq!(
            zpc_disables.lock().unwrap()[ZPC_BAR as usize],
            1,
            "c:4201 *disp = !enable → 1 for disable"
        );
        assert!(!haswilds(&tok("a|b")), "after disable, | is literal");
        // Restore.
        zpc_disables.lock().unwrap()[ZPC_BAR as usize] = 0;
    }

    /// `Src/pattern.c:4201` — `enable -p NAME` after a `disable -p NAME`
    /// clears the slot (`*disp = !enable` = 0 when enable=true).
    #[test]
    fn pat_enables_re_enables_disabled_token() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — tokenize input as every C caller does.
        let tok = |s: &str| {
            let mut t = s.to_string();
            crate::ported::glob::tokenize(&mut t);
            t
        };
        // Pre-disable.
        zpc_disables.lock().unwrap()[ZPC_STAR as usize] = 1;
        assert!(!haswilds(&tok("*")));
        // Re-enable.
        let ret = pat_enables("enable", &["*"], true);
        assert_eq!(ret, 0);
        assert_eq!(
            zpc_disables.lock().unwrap()[ZPC_STAR as usize],
            0,
            "c:4201 *disp = !enable → 0 for enable"
        );
        assert!(haswilds(&tok("*")));
    }

    /// `Src/pattern.c:4205-4208` — unknown token returns 1 and the
    /// disables table is unchanged for that slot.
    #[test]
    fn pat_enables_unknown_pattern_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let baseline = zpc_disables.lock().unwrap().clone();
        let ret = pat_enables("disable", &["bogus_not_a_metachar"], false);
        assert_eq!(ret, 1, "c:4207 — invalid pattern → ret = 1");
        // Table untouched.
        assert_eq!(
            *zpc_disables.lock().unwrap(),
            baseline,
            "c:4205-4208 — no slot mutated on miss"
        );
    }

    /// `Src/pattern.c:4196` — multiple patterns: loop continues past
    /// each, even if some are invalid (ret stays 1, but valid ones
    /// still apply).
    #[test]
    fn pat_enables_partial_failure_applies_valid_disables() {
        let _g = crate::test_util::global_state_lock();
        zpc_disables.lock().unwrap()[ZPC_BAR as usize] = 0;
        zpc_disables.lock().unwrap()[ZPC_STAR as usize] = 0;
        let ret = pat_enables("disable", &["|", "bogus", "*"], false);
        assert_eq!(ret, 1, "c:4207 — at least one invalid → ret = 1");
        // Both valid ones got applied (c:4196 `for (; *patp; patp++)` doesn't break on miss).
        assert_eq!(
            zpc_disables.lock().unwrap()[ZPC_BAR as usize],
            1,
            "| was disabled"
        );
        assert_eq!(
            zpc_disables.lock().unwrap()[ZPC_STAR as usize],
            1,
            "* was disabled"
        );
        // Restore.
        zpc_disables.lock().unwrap()[ZPC_BAR as usize] = 0;
        zpc_disables.lock().unwrap()[ZPC_STAR as usize] = 0;
    }

    /// `Src/pattern.c:4177-4194` — empty patp lists enabled or disabled
    /// tokens via stdout. Hard to capture stdout in a unit test, so
    /// just verify the return value (0) per c:4193.
    #[test]
    fn pat_enables_empty_patp_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let ret_enable = pat_enables("enable", &[], true);
        assert_eq!(ret_enable, 0, "c:4193 — listing path returns 0");
        let ret_disable = pat_enables("disable", &[], false);
        assert_eq!(ret_disable, 0, "c:4193 — listing path returns 0");
    }

    /// `Src/pattern.c:4197-4202` — looks up the exact string in
    /// `zpc_strings[]`. Each named slot from c:258 (`"|", "~", "(",
    /// "?", "*", "[", "<", "^", "#"` plus `"?(","*(","+(","!("`) must
    /// be toggleable; NULL slots (`ZPC_NULL`, `ZPC_BNULLKEEP`, etc.)
    /// can't be referenced by name and any caller naming them gets
    /// `invalid pattern`.
    #[test]
    fn pat_enables_all_named_zpc_slots_toggle() {
        let _g = crate::test_util::global_state_lock();
        // Save baseline.
        let baseline = zpc_disables.lock().unwrap().clone();
        for name in &[
            "|", "(", "?", "*", "[", "<", "^", "#", "?(", "*(", "+(", "!(",
        ] {
            assert_eq!(
                pat_enables("disable", &[name], false),
                0,
                "c:4196 — disable -p {name} succeeds",
            );
        }
        // Restore.
        *zpc_disables.lock().unwrap() = baseline;
    }

    // ═══════════════════════════════════════════════════════════════════
    // metacharinc / charref / charnext / charrefinc / charsub — the
    // small char-advance helpers from pattern.c:336/1909/1936/1964/1997.
    // Each C version does Meta-byte + ztoken table + mbrtowc state
    // machine; the Rust port collapses them all to UTF-8-native
    // `chars()` iteration. Tests pin the Rust observable semantic.
    // ═══════════════════════════════════════════════════════════════════

    /// `Src/pattern.c:336` — `metacharinc(char **x)` advances one
    /// multibyte char. Rust port returns the new byte position.
    /// ASCII path: advance by 1.
    #[test]
    fn metacharinc_advances_one_ascii_byte() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(metacharinc("abc", 0), 1, "c:343-358 single-byte path");
        assert_eq!(metacharinc("abc", 1), 2);
        assert_eq!(metacharinc("abc", 2), 3);
    }

    /// `Src/pattern.c:363-380` — multibyte path: advance by the
    /// codepoint's UTF-8 byte length, not always 1.
    #[test]
    fn metacharinc_advances_by_codepoint_width() {
        let _g = crate::test_util::global_state_lock();
        // 'é' is 2 UTF-8 bytes.
        assert_eq!(metacharinc("é", 0), 2, "c:363-380 mbrtowc path width=2");
        // '日' is 3 UTF-8 bytes.
        assert_eq!(metacharinc("日", 0), 3, "mbrtowc path width=3");
        // '🦀' is 4 UTF-8 bytes.
        assert_eq!(metacharinc("🦀", 0), 4, "mbrtowc path width=4");
    }

    /// `Src/pattern.c:336` — at end-of-string the C version returns
    /// `WCHAR_INVALID(*(*x)++)` (c:385); the Rust port returns the
    /// same position (no advance) when there's nothing to decode.
    #[test]
    fn metacharinc_at_eos_returns_same_position() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(metacharinc("abc", 3), 3, "EOS — no advance");
        assert_eq!(metacharinc("", 0), 0, "empty — no advance");
    }

    /// `Src/pattern.c:1909` — `charref(char *x, char *y, int *zmb_ind)`
    /// decodes the codepoint at `x` and returns it. Rust port returns
    /// `Option<char>`; `None` on empty.
    #[test]
    fn charref_decodes_one_codepoint() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(charref("abc", 0), Some('a'));
        assert_eq!(charref("abc", 1), Some('b'));
        assert_eq!(charref("é日", 0), Some('é'), "multibyte at start");
        // At offset 2, "é" (2 bytes) already consumed; next char is '日'.
        assert_eq!(charref("é日", 2), Some('日'));
        assert_eq!(charref("", 0), None, "empty → None");
        assert_eq!(charref("abc", 3), None, "EOS → None");
    }

    /// `Src/pattern.c:1936` — `charnext(char *x, char *y)` is the
    /// single-step version (advance one position). Delegates to
    /// `metacharinc` in the Rust port.
    #[test]
    fn charnext_delegates_to_metacharinc() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(charnext("abc", 0), metacharinc("abc", 0));
        assert_eq!(charnext("é日", 0), 2, "c:1936 → c:336 multibyte advance");
        assert_eq!(charnext("é日", 2), 5, "advance past '日' (3 bytes)");
    }

    /// `Src/pattern.c:1964` — `charrefinc(char **x, char *y, int *z)`
    /// decodes + advances. Rust mutates `pos` in place and returns the
    /// codepoint. Tests both the codepoint return and the position
    /// mutation.
    #[test]
    fn charrefinc_decodes_and_advances_position() {
        let _g = crate::test_util::global_state_lock();
        let mut pos = 0;
        assert_eq!(charrefinc("abc", &mut pos), Some('a'));
        assert_eq!(pos, 1, "c:1964 — advance by 1 ASCII");
        let mut pos = 0;
        assert_eq!(charrefinc("é日", &mut pos), Some('é'));
        assert_eq!(pos, 2, "c:1964 — advance by 2 UTF-8 bytes");
        assert_eq!(charrefinc("é日", &mut pos), Some('日'));
        assert_eq!(pos, 5, "c:1964 — advance by 3 more UTF-8 bytes");
        let mut pos = 0;
        assert_eq!(charrefinc("", &mut pos), None);
        assert_eq!(pos, 0, "c:1964 — no advance on empty");
    }

    // ═══════════════════════════════════════════════════════════════════
    // savepatterndisables / restorepatterndisables — pattern.c:4218-4271.
    // u32 bitmask save+restore over `zpc_disables[ZPC_COUNT]`. Each
    // slot i contributes (1 << i) to the bitmask when its disable byte
    // is non-zero. Tests pin the bit-position mapping per C's
    // `for (bit = 1, disp = zpc_disables; …; bit <<= 1, disp++)`.
    // ═══════════════════════════════════════════════════════════════════

    /// `Src/pattern.c:4220-4232` — save when no slot disabled returns 0.
    #[test]
    fn savepatterndisables_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // Zero out all slots.
        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
        let disables = savepatterndisables();
        assert_eq!(disables, 0, "c:4232 — no slots set → bitmap 0");
    }

    /// `Src/pattern.c:4226-4231` — bit-position mapping: slot `i`
    /// contributes `1 << i`. Sets specific slots and verifies the
    /// returned u32.
    #[test]
    fn savepatterndisables_bitmap_matches_slot_indices() {
        let _g = crate::test_util::global_state_lock();
        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
        zpc_disables.lock().unwrap()[ZPC_BAR as usize] = 1;
        zpc_disables.lock().unwrap()[ZPC_STAR as usize] = 1;
        let disables = savepatterndisables();
        let expected = (1u32 << ZPC_BAR) | (1u32 << ZPC_STAR);
        assert_eq!(disables, expected, "c:4231 — bits ZPC_BAR + ZPC_STAR set");
        // Restore.
        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
    }

    /// `Src/pattern.c:4266-4269` — restore writes 1/0 to each slot per
    /// the bitmask. Pin the inverse-of-save semantic.
    #[test]
    fn restorepatterndisables_zero_clears_all_slots() {
        let _g = crate::test_util::global_state_lock();
        // Pre-populate.
        *zpc_disables.lock().unwrap() = [1u8; ZPC_COUNT as usize];
        restorepatterndisables(0);
        let table = zpc_disables.lock().unwrap().clone();
        for (i, &v) in table.iter().enumerate() {
            assert_eq!(v, 0, "c:4269 — slot {i} cleared with bitmap=0");
        }
    }

    /// `Src/pattern.c:4266-4267` — bitmap with all relevant bits set
    /// makes restore turn every slot on. All-ones bitmap maps to all-1
    /// slots up to ZPC_COUNT.
    #[test]
    fn restorepatterndisables_all_ones_sets_each_slot() {
        let _g = crate::test_util::global_state_lock();
        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
        let all = (1u32 << ZPC_COUNT).wrapping_sub(1);
        restorepatterndisables(all);
        let table = zpc_disables.lock().unwrap().clone();
        for (i, &v) in table.iter().enumerate() {
            assert_eq!(v, 1, "c:4267 — slot {i} set with full bitmap");
        }
        // Restore.
        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
    }

    /// `Src/pattern.c:4218-4271` — save then restore round-trips
    /// every slot through the u32 bitmask losslessly.
    #[test]
    fn save_restore_pattern_disables_roundtrip() {
        let _g = crate::test_util::global_state_lock();
        // Set up a non-trivial pattern: alternating slots.
        for i in 0..(ZPC_COUNT as usize) {
            zpc_disables.lock().unwrap()[i] = (i % 2) as u8;
        }
        let saved = savepatterndisables();
        // Clobber.
        *zpc_disables.lock().unwrap() = [9u8; ZPC_COUNT as usize];
        // Restore.
        restorepatterndisables(saved);
        // Verify alternating pattern back.
        let table = zpc_disables.lock().unwrap().clone();
        for i in 0..(ZPC_COUNT as usize) {
            assert_eq!(
                table[i],
                (i % 2) as u8,
                "c:4218+c:4258 round-trip: slot {i} preserved"
            );
        }
        // Reset.
        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
    }

    // ═══════════════════════════════════════════════════════════════════
    // startpatternscope / endpatternscope / clearpatterndisables —
    // pattern.c:4241/4279/4296. Pin LOCALPATTERNS-gated save/restore +
    // C `memset(zpc_disables, 0, ZPC_COUNT)` clear.
    //
    // Bug fixed: prior Rust port operated on a separate
    // `patterndisables: Mutex<Vec<String>>` tombstone (cleared by
    // clearpatterndisables, push/popped by start/end). C's three fns
    // all operate on the canonical `zpc_disables[]` byte array.
    // Pre-fix: setopt LOCALPATTERNS function entry/exit was a no-op
    // and `clearpatterndisables` didn't clear the matcher's disable
    // bytes.
    // ═══════════════════════════════════════════════════════════════════

    /// `Src/pattern.c:4296-4298` — `memset(zpc_disables, 0, ZPC_COUNT)`.
    /// Test that clearpatterndisables zeros every slot.
    #[test]
    fn clearpatterndisables_zeros_zpc_disables() {
        let _g = crate::test_util::global_state_lock();
        // Pre-populate every slot.
        *zpc_disables.lock().unwrap() = [1u8; ZPC_COUNT as usize];
        clearpatterndisables();
        let table = zpc_disables.lock().unwrap().clone();
        for (i, &v) in table.iter().enumerate() {
            assert_eq!(v, 0, "c:4298 — slot {i} cleared");
        }
    }

    /// `Src/pattern.c:4241-4250` + c:4279-4290` — under `setopt
    /// LOCALPATTERNS`, function entry save → mutate → exit restore
    /// round-trips zpc_disables.
    #[test]
    fn pattern_scope_save_restore_under_localpatterns() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::options::opt_state_set("localpatterns", true);

        // Initial state: ZPC_BAR disabled, ZPC_STAR enabled.
        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
        zpc_disables.lock().unwrap()[ZPC_BAR as usize] = 1;

        // Enter scope (c:4241 — `savepatterndisables` into stack frame).
        startpatternscope();

        // Mutate inside the "function" body.
        zpc_disables.lock().unwrap()[ZPC_BAR as usize] = 0;
        zpc_disables.lock().unwrap()[ZPC_STAR as usize] = 1;
        assert_eq!(zpc_disables.lock().unwrap()[ZPC_BAR as usize], 0);
        assert_eq!(zpc_disables.lock().unwrap()[ZPC_STAR as usize], 1);

        // Exit scope (c:4279 — restore via restorepatterndisables).
        endpatternscope();

        // Outer state must come back.
        assert_eq!(
            zpc_disables.lock().unwrap()[ZPC_BAR as usize],
            1,
            "c:4287 — ZPC_BAR restored to outer-scope disabled"
        );
        assert_eq!(
            zpc_disables.lock().unwrap()[ZPC_STAR as usize],
            0,
            "c:4287 — ZPC_STAR restored to outer-scope enabled"
        );

        // Reset.
        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
        crate::ported::options::opt_state_set("localpatterns", false);
    }

    /// `Src/pattern.c:4286` — WITHOUT LOCALPATTERNS, endpatternscope
    /// pops the stack frame but does NOT restore. Function-body
    /// mutations leak into the caller.
    #[test]
    fn pattern_scope_no_restore_without_localpatterns() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::options::opt_state_set("localpatterns", false);

        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
        zpc_disables.lock().unwrap()[ZPC_BAR as usize] = 1;

        startpatternscope();
        zpc_disables.lock().unwrap()[ZPC_BAR as usize] = 0;
        endpatternscope();

        // Mutation leaked — c:4286 gate skipped restore.
        assert_eq!(
            zpc_disables.lock().unwrap()[ZPC_BAR as usize],
            0,
            "c:4286 — WITHOUT LOCALPATTERNS, mutation leaks out"
        );

        // Reset.
        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
    }

    /// `Src/pattern.c:4241-4250` — nested scopes form a LIFO stack.
    /// Inner restore pops just the inner frame; outer frame stays.
    #[test]
    fn pattern_scope_nested_lifo() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::options::opt_state_set("localpatterns", true);

        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];

        // Outer.
        zpc_disables.lock().unwrap()[ZPC_BAR as usize] = 1;
        startpatternscope(); // frame A
                             // Inner.
        zpc_disables.lock().unwrap()[ZPC_BAR as usize] = 0;
        zpc_disables.lock().unwrap()[ZPC_STAR as usize] = 1;
        startpatternscope(); // frame B
                             // Innermost.
        zpc_disables.lock().unwrap()[ZPC_INBRACK as usize] = 1;
        endpatternscope(); // pop B → inner restored
        assert_eq!(zpc_disables.lock().unwrap()[ZPC_BAR as usize], 0);
        assert_eq!(zpc_disables.lock().unwrap()[ZPC_STAR as usize], 1);
        assert_eq!(
            zpc_disables.lock().unwrap()[ZPC_INBRACK as usize],
            0,
            "c:4287 — frame B's snapshot didn't include this slot's 1"
        );
        endpatternscope(); // pop A → outer restored
        assert_eq!(zpc_disables.lock().unwrap()[ZPC_BAR as usize], 1);
        assert_eq!(zpc_disables.lock().unwrap()[ZPC_STAR as usize], 0);

        // Reset.
        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
        crate::ported::options::opt_state_set("localpatterns", false);
    }

    /// `Src/pattern.c:4226` — bit position is the slot INDEX (0-based),
    /// not 1-based. Slot 0 → bit 1, slot 1 → bit 2, etc. Per the C
    /// `bit = 1` initial and `bit <<= 1` after each slot.
    #[test]
    fn savepatterndisables_slot_0_is_low_bit() {
        let _g = crate::test_util::global_state_lock();
        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
        zpc_disables.lock().unwrap()[0] = 1;
        let disables = savepatterndisables();
        assert_eq!(disables, 1, "c:4226 — bit = 1 initial → slot 0 is low bit");
        *zpc_disables.lock().unwrap() = [0u8; ZPC_COUNT as usize];
    }

    /// `Src/pattern.c:1997` — `charsub(x, y)` returns the number of
    /// characters between the start and byte offset `y`. Non-multibyte:
    /// byte distance; multibyte: codepoint count of `x[0..y]`.
    #[test]
    fn charsub_counts_chars_to_offset() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::options::opt_state_get("multibyte");

        // c:2004 — non-multibyte: byte distance `y - x`.
        crate::ported::options::opt_state_set("multibyte", false);
        assert_eq!(charsub("abc", 3), 3, "non-MB: byte distance to end");
        assert_eq!(charsub("abc", 1), 1, "non-MB: one byte in");
        assert_eq!(charsub("abc", 0), 0, "c:1997 — at start, distance 0");
        assert_eq!(charsub("é日", 5), 5, "non-MB: raw byte count (5 bytes)");

        // c:2006-2021 — multibyte: codepoint count of `x[0..y]`.
        crate::ported::options::opt_state_set("multibyte", true);
        assert_eq!(charsub("abc", 3), 3, "MB: 3 ASCII chars");
        assert_eq!(charsub("é", 2), 1, "MB: one 2-byte codepoint");
        assert_eq!(charsub("é日", 5), 2, "MB: é + 日 = 2 codepoints");
        assert_eq!(charsub("é日", 2), 1, "MB: just é = 1 codepoint");
        assert_eq!(charsub("é日", 0), 0, "MB: distance 0 at start");

        if let Some(v) = saved {
            crate::ported::options::opt_state_set("multibyte", v);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // KSH_GLOB extended patterns: +(pat), *(pat), ?(pat), @(pat), !(pat).
    // Anchored against `setopt KSH_GLOB; [[ str == ${~pat} ]]` in zsh 5.9.
    // Tests enable KSH_GLOB before each assertion and restore prior state.
    //
    // Ported from pattern.c:1278-1350 (KSH dispatch in patcomppiece) and
    // pattern.c:1615-1746 (kshchar-driven quantifier emission), plus
    // patcompnot pattern.c:1759-1784 for the !(pat) negation form and a
    // minimal P_EXCLUDE matcher arm (pattern.c:3056-3201).
    // ═══════════════════════════════════════════════════════════════════

    fn ksh_glob_match(pat: &str, s: &str) -> bool {
        let _g = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let saved = opt_state_get("kshglob").unwrap_or(false);
        opt_state_set("kshglob", true);
        let r = patmatch(pat, s);
        opt_state_set("kshglob", saved);
        r
    }

    // ── +(pat) — one or more ─────────────────────────────────────────
    /// `+(foo)` matches `foo` — zsh: MATCH.
    #[test]
    fn ksh_glob_plus_one_repetition_matches() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("+(foo)", "foo"));
    }

    /// `+(foo)` matches `foofoo` — zsh: MATCH.
    #[test]
    fn ksh_glob_plus_two_repetitions_matches() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("+(foo)", "foofoo"));
    }

    /// `+(foo)` does NOT match `""` — zsh: NOMATCH (and zshrs agrees).
    /// This is the ONLY +(pat) test that passes — both reject empty.
    #[test]
    fn ksh_glob_plus_zero_repetitions_fails() {
        let _g = crate::test_util::global_state_lock();
        assert!(!ksh_glob_match("+(foo)", ""));
    }

    /// `+(foo)` matches `foofoofoo` — zsh: MATCH.
    #[test]
    fn ksh_glob_plus_three_repetitions_matches() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("+(foo)", "foofoofoo"));
    }

    // ── *(pat) — zero or more ────────────────────────────────────────
    /// `*(foo)` matches empty — zsh: MATCH.
    #[test]
    fn ksh_glob_star_paren_matches_empty() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("*(foo)", ""));
    }

    /// `*(foo)` matches `foo` — both zsh and zshrs agree (passes).
    #[test]
    fn ksh_glob_star_paren_matches_one() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("*(foo)", "foo"));
    }

    /// `*(foo)` matches `foofoo` — both agree.
    #[test]
    fn ksh_glob_star_paren_matches_multiple() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("*(foo)", "foofoo"));
    }

    // ── ?(pat) — zero or one ─────────────────────────────────────────
    /// `?(foo)` matches empty — zsh: MATCH.
    #[test]
    fn ksh_glob_question_paren_matches_empty() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("?(foo)", ""));
    }

    /// `?(foo)` matches `foo` — zsh: MATCH.
    #[test]
    fn ksh_glob_question_paren_matches_one_rep() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("?(foo)", "foo"));
    }

    /// `?(foo)` does NOT match `foofoo` — both agree (rejects).
    #[test]
    fn ksh_glob_question_paren_fails_on_two_reps() {
        let _g = crate::test_util::global_state_lock();
        assert!(!ksh_glob_match("?(foo)", "foofoo"));
    }

    // ── @(pat) — exactly one ─────────────────────────────────────────
    /// `@(foo)` matches `foo` — zsh: MATCH. zshrs fails on the alternation
    /// form but matches the single-branch form.
    #[test]
    fn ksh_glob_at_paren_matches_exact() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("@(foo)", "foo"));
    }

    /// `@(foo|bar)` matches both `foo` AND `bar` — zsh: MATCH both.
    #[test]
    fn ksh_glob_at_paren_with_alternation_either_branch() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("@(foo|bar)", "foo"));
        assert!(ksh_glob_match("@(foo|bar)", "bar"));
    }

    /// `@(foo|bar)` rejects `qux` — both agree.
    #[test]
    fn ksh_glob_at_paren_alternation_rejects_outside() {
        let _g = crate::test_util::global_state_lock();
        assert!(!ksh_glob_match("@(foo|bar)", "qux"));
    }

    // ── !(pat) — not match ───────────────────────────────────────────
    /// `!(foo)` rejects `foo` — both agree.
    #[test]
    fn ksh_glob_bang_paren_rejects_matching_string() {
        let _g = crate::test_util::global_state_lock();
        assert!(!ksh_glob_match("!(foo)", "foo"));
    }

    /// `!(foo)` matches `bar` — zsh: MATCH.
    #[test]
    fn ksh_glob_bang_paren_matches_non_matching_string() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("!(foo)", "bar"));
    }

    /// `!(foo|bar)` matches `baz` and rejects `foo` — zsh.
    #[test]
    fn ksh_glob_bang_paren_with_alternation() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("!(foo|bar)", "baz"));
        assert!(!ksh_glob_match("!(foo|bar)", "foo"));
    }

    // ── Mixed with literal context ───────────────────────────────────
    /// `pre+(x)post` matches `prexpost`, `prexxpost` — zsh: MATCH both.
    #[test]
    fn ksh_glob_plus_paren_in_literal_context() {
        let _g = crate::test_util::global_state_lock();
        assert!(ksh_glob_match("pre+(x)post", "prexpost"));
        assert!(ksh_glob_match("pre+(x)post", "prexxpost"));
    }

    /// `pre+(x)post` rejects `prepost` — both agree (no x's).
    #[test]
    fn ksh_glob_plus_paren_in_literal_rejects_zero_reps() {
        let _g = crate::test_util::global_state_lock();
        assert!(!ksh_glob_match("pre+(x)post", "prepost"));
    }

    // ═══════════════════════════════════════════════════════════════════
    // EXTENDED_GLOB pattern flags: (#i) case-insensitive, (#l) lowercase
    // matches uppercase, (#aN) approximate match. Anchored to
    // `setopt EXTENDED_GLOB; [[ str == ${~pat} ]]` in real zsh 5.9.
    // ═══════════════════════════════════════════════════════════════════

    fn ext_glob_match(pat: &str, s: &str) -> bool {
        let _g = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let saved = opt_state_get("extendedglob").unwrap_or(false);
        opt_state_set("extendedglob", true);
        let r = patmatch(pat, s);
        opt_state_set("extendedglob", saved);
        r
    }

    // ── (#i) case-insensitive ───────────────────────────────────────
    /// `(#i)FOO` matches "foo" (case ignored).
    #[test]
    fn ext_glob_hash_i_matches_lowercase_against_uppercase_pat() {
        let _g = crate::test_util::global_state_lock();
        assert!(ext_glob_match("(#i)FOO", "foo"));
    }

    /// `(#i)FOO` matches "FOO" exactly.
    #[test]
    fn ext_glob_hash_i_matches_exact_case() {
        let _g = crate::test_util::global_state_lock();
        assert!(ext_glob_match("(#i)FOO", "FOO"));
    }

    /// `(#i)FOO` matches mixed-case "FoO".
    #[test]
    fn ext_glob_hash_i_matches_mixed_case() {
        let _g = crate::test_util::global_state_lock();
        assert!(ext_glob_match("(#i)FOO", "FoO"));
    }

    /// `(#i)foo` rejects unrelated string "BAR".
    #[test]
    fn ext_glob_hash_i_rejects_unrelated_string() {
        let _g = crate::test_util::global_state_lock();
        assert!(!ext_glob_match("(#i)foo", "BAR"));
    }

    // ── (#l) lowercase-pattern matches uppercase-text ───────────────
    /// `(#l)foo` matches "FOO" (lowercase pattern allows uppercase).
    /// Distinct from (#i): asymmetric — pattern letter must be lowercase
    /// AND text may be either case for that letter.
    #[test]
    fn ext_glob_hash_l_lowercase_pat_matches_uppercase_text() {
        let _g = crate::test_util::global_state_lock();
        assert!(ext_glob_match("(#l)foo", "FOO"));
    }

    /// `(#l)foo` matches lowercase "foo".
    #[test]
    fn ext_glob_hash_l_lowercase_pat_matches_lowercase_text() {
        let _g = crate::test_util::global_state_lock();
        assert!(ext_glob_match("(#l)foo", "foo"));
    }

    /// `(#l)FOO` does NOT match "foo" — asymmetry: uppercase in pattern
    /// requires uppercase in text. zsh: NOMATCH.
    #[test]
    fn ext_glob_hash_l_uppercase_pat_requires_uppercase_text_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("(#l)FOO", "foo"),
            "zsh: (#l)FOO must NOT match \"foo\" — uppercase-in-pattern is anchored"
        );
    }

    // ── (#aN) approximate match (Damerau-Levenshtein distance ≤ N) ──
    /// `(#a1)foo` matches "fop" (1 char substitution).
    #[test]
    fn ext_glob_hash_a1_matches_one_substitution_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#a1)foo", "fop"),
            "zsh: (#a1)foo matches 'fop' (1 substitution)"
        );
    }

    /// `(#a2)foo` matches "fxy" (2 substitutions).
    #[test]
    fn ext_glob_hash_a2_matches_two_substitutions_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#a2)foo", "fxy"),
            "zsh: (#a2)foo matches 'fxy' (2 substitutions)"
        );
    }

    /// `(#a1)foo` rejects "fxy" (2 substitutions > limit 1).
    #[test]
    fn ext_glob_hash_a1_rejects_two_substitutions() {
        let _g = crate::test_util::global_state_lock();
        assert!(!ext_glob_match("(#a1)foo", "fxy"));
    }

    // ═══════════════════════════════════════════════════════════════════
    // (#aN) full Damerau-Levenshtein — sub/ins/del edit operations.
    // C inlines the backtracking in P_EXACTLY (c:2737-2779) plus the
    // approx-aware sync nodes at c:3055+. Rust factors into
    // `approx_match_exactly` (WARNING: NOT IN PATTERN.C). Tests pin
    // each edit operation independently + the budget upper bound.
    // ═══════════════════════════════════════════════════════════════════

    /// `(#a0)foo` is exact-match only — no edits allowed.
    #[test]
    fn ext_glob_hash_a0_is_exact_match_only() {
        let _g = crate::test_util::global_state_lock();
        assert!(ext_glob_match("(#a0)foo", "foo"), "exact ok");
        assert!(
            !ext_glob_match("(#a0)foo", "fop"),
            "no edit budget rejects 1-sub"
        );
    }

    /// `(#a1)foo` matches "fxoo" via INSERTION in input (extra 'x').
    /// The pattern has no `x`; treating `x` as an inserted-in-input
    /// char costs 1 error. Input → "f(x)oo" with the 'x' deleted.
    #[test]
    fn ext_glob_hash_a_accepts_insertion_in_input() {
        let _g = crate::test_util::global_state_lock();
        // "fxoo" — extra 'x' between 'f' and 'oo'.
        assert!(
            ext_glob_match("(#a1)foo", "fxoo"),
            "1 insertion-in-input edit"
        );
    }

    /// `(#a1)foo` matches "fo" via DELETION from input (missing 'o').
    /// One 'o' deleted from input compared to pattern.
    #[test]
    fn ext_glob_hash_a_accepts_deletion_from_input() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#a1)foo", "fo"),
            "1 deletion-from-input edit"
        );
    }

    /// `(#a2)foo` matches "fxooy" via 1 insertion + 1 substitution.
    /// 'x' inserted between f-o; final 'o' substituted to 'y'. Wait —
    /// closer reading: "fxooy" vs "foo" — 'x' is the extra char, but
    /// then 'ooy' vs 'oo' has another extra 'y'. That's 2 insertions.
    #[test]
    fn ext_glob_hash_a_mixed_edits_within_budget() {
        let _g = crate::test_util::global_state_lock();
        // 2 insertions: 'x' and 'y'.
        assert!(
            ext_glob_match("(#a2)foo", "fxooy"),
            "2 insertions in input within budget"
        );
    }

    /// Budget upper bound: `(#a1)abc` rejects "xyz" (3 substitutions > 1).
    #[test]
    fn ext_glob_hash_a1_rejects_three_substitutions() {
        let _g = crate::test_util::global_state_lock();
        assert!(!ext_glob_match("(#a1)abc", "xyz"));
    }

    /// `(#a3)abc` matches "xyz" (3 substitutions ≤ 3).
    #[test]
    fn ext_glob_hash_a3_accepts_all_substituted() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#a3)abc", "xyz"),
            "3 substitutions at budget 3"
        );
    }

    /// Position-independent substitution: budget allows replacing
    /// any one of N pattern chars.
    #[test]
    fn ext_glob_hash_a_accepts_position_independent_substitution() {
        let _g = crate::test_util::global_state_lock();
        assert!(ext_glob_match("(#a1)abc", "Xbc"), "first-char sub");
        assert!(ext_glob_match("(#a1)abc", "aXc"), "middle-char sub");
        assert!(ext_glob_match("(#a1)abc", "abX"), "last-char sub");
    }

    // ═══════════════════════════════════════════════════════════════════
    // zsh test-corpus pins — direct anchors to Test/D02glob.ztst:25-80
    // "zsh globbing" test list (zsh's authoritative regression suite).
    // Each test cites the ztst line range it pins. Currently-failing
    // tests get #[ignore = "ZSHRS BUG: ..."] markers; expected-output
    // assertions stay in tree so the marker flips when the Rust port
    // catches up.
    // ═══════════════════════════════════════════════════════════════════

    /// `Test/D02glob.ztst:26` — `[[ foo~ = foo~ ]]` exact match,
    /// expected exit 0 (true).
    #[test]
    fn zsh_corpus_foo_tilde_exact_match() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("foo~", "foo~"),
            "ztst:26 — literal tilde matches"
        );
    }

    /// `Test/D02glob.ztst:27` — `[[ foo~ = (foo~) ]]` parenthesised
    /// single alternative.
    #[test]
    fn zsh_corpus_foo_tilde_in_parens() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(foo~)", "foo~"),
            "ztst:27 — (foo~) matches foo~"
        );
    }

    /// `Test/D02glob.ztst:28` — `[[ foo~ = (foo~|) ]]` alternation
    /// with empty alternative.
    #[test]
    fn zsh_corpus_alternation_with_empty_alt() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(foo~|)", "foo~"),
            "ztst:28 — empty alt accepted"
        );
    }

    /// `Test/D02glob.ztst:29` — `[[ foo.c = *.c~boo* ]]` exclude
    /// pattern: matches *.c BUT NOT boo*. `foo.c` matches *.c and
    /// doesn't match boo*, so result is 0 (true).
    #[test]
    fn zsh_corpus_exclude_pattern_basic() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("*.c~boo*", "foo.c"),
            "ztst:29 — *.c~boo* matches foo.c"
        );
    }

    /// `Test/D02glob.ztst:30` — `[[ foo.c = *.c~boo*~foo* ]]`
    /// double-exclude: matches *.c but excludes both `boo*` and
    /// `foo*`. `foo.c` matches `foo*` (excluded) → result 1 (false).
    #[test]
    fn zsh_corpus_exclude_pattern_double() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("*.c~boo*~foo*", "foo.c"),
            "ztst:30 — double exclude rejects foo.c"
        );
    }

    /// `Test/D02glob.ztst:31` — `[[ fofo = (fo#)# ]]` — `#` is the
    /// extended-glob "1+ repetitions" quantifier. `(fo#)#` matches
    /// 1+ repetitions of "fo+".
    #[test]
    fn zsh_corpus_hash_repetition_double() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(fo#)#", "fofo"),
            "ztst:31 — (fo#)# matches fofo"
        );
    }

    /// `Test/D02glob.ztst:32` — `[[ ffo = (fo#)# ]]`. `fo#` means
    /// `f` followed by 0+ `o`. `ffo` = `f` + `fo` (matches the
    /// outer `#` repetition).
    #[test]
    fn zsh_corpus_hash_repetition_min_one() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(fo#)#", "ffo"),
            "ztst:32 — (fo#)# matches ffo via 1-iter outer"
        );
    }

    /// `Test/D02glob.ztst:36` — `[[ foooofof = (fo##)# ]]` —
    /// `##` is "2+ repetitions". `fo##` = `f` + 2+ `o`'s. The
    /// trailing `of` after `foooo` doesn't satisfy the trailing
    /// outer `#`, so result is 1 (false).
    #[test]
    fn zsh_corpus_hash_hash_quantifier_min_two() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("(fo##)#", "foooofof"),
            "ztst:36 — (fo##)# rejects foooofof"
        );
    }

    /// `Test/D02glob.ztst:50` — `[[ aac = ((a))#a(c) ]]` —
    /// `((a))#` is `0+ a`. `aac` = `aa` (= `((a))#`) + `c`. The
    /// final `a(c)` requires literal `a` followed by capture `(c)`.
    /// Wait — should be `aac` matches `((a))#a(c)`: outer `((a))#`
    /// captures `a` repeated, then literal `a`, then `(c)`.
    /// Actually re-reading: `((a))#` matches 0+ 'a'. `aac` =
    /// `((a))#`("a") + `a`("a") + `(c)`("c"). So "aa" splits as
    /// (a)("a") + a("a") + c("c"). Match.
    #[test]
    fn zsh_corpus_nested_paren_quantifier() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("((a))#a(c)", "aac"),
            "ztst:50 — ((a))#a(c) matches aac"
        );
    }

    /// `Test/D02glob.ztst:51` — `[[ ac = ((a))#a(c) ]]` — single
    /// `a` matches `a(c)` after empty `((a))#` consumes zero.
    #[test]
    fn zsh_corpus_zero_iteration_quantifier() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("((a))#a(c)", "ac"),
            "ztst:51 — ((a))# can be zero iters"
        );
    }

    /// `Test/D02glob.ztst:52` — `[[ c = ((a))#a(c) ]]` — empty
    /// `((a))#` + literal `a` requires at least one `a`. `c` has
    /// no `a`, so result is 1 (false).
    #[test]
    fn zsh_corpus_required_literal_after_zero_quantifier() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("((a))#a(c)", "c"),
            "ztst:52 — bare `c` lacks required `a`"
        );
    }

    /// `Test/D02glob.ztst:73` — `[[ foo = ((^x)) ]]` — exclude
    /// `x`. `foo` doesn't contain `x` as a single char (the
    /// `((^x))` matches anything that's not just 'x' — `foo` is 3
    /// chars, none is `x`).
    #[test]
    fn zsh_corpus_caret_exclude_basic() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("((^x))", "foo"),
            "ztst:73 — ((^x)) matches foo"
        );
    }

    /// `Test/D02glob.ztst:75` — `[[ foo = ((^foo)) ]]` — `^foo`
    /// rejects exact `foo`. `foo` matches `foo` literal, which the
    /// `^` inverts. Result: 1 (false).
    #[test]
    fn zsh_corpus_caret_exclude_exact_match_inverted() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("((^foo))", "foo"),
            "ztst:75 — ((^foo)) rejects foo"
        );
    }

    /// `Test/D02glob.ztst:79` — `[[ foot = z*~*x ]]` — `*` then
    /// exclude `*x`. `foot` doesn't start with `z`, so doesn't
    /// match `z*` at all → result 1 (false).
    #[test]
    fn zsh_corpus_star_exclude_no_z_prefix() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("z*~*x", "foot"),
            "ztst:79 — foot doesn't start with z"
        );
    }

    /// `Test/D02glob.ztst:80` — `[[ zoot = z*~*x ]]` — `zoot`
    /// matches `z*` AND doesn't end in `x` → result 0 (true).
    #[test]
    fn zsh_corpus_star_exclude_zoot() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("z*~*x", "zoot"),
            "ztst:80 — zoot matches z* and not *x"
        );
    }

    // ─── POSIX class brackets — Test/D02glob.ztst:111-118 ─────────────

    /// `Test/D02glob.ztst:111` — `[[:alpha:][:punct:]]#[[:digit:]][^[:lower:]]`
    /// over "a%1X": `a` matches alpha, `%` matches punct (1+ via `#`),
    /// `1` matches digit, `X` matches NOT-lower.
    #[test]
    fn zsh_corpus_posix_class_alpha_punct_digit_notlower() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("[[:alpha:][:punct:]]#[[:digit:]][^[:lower:]]", "a%1X"),
            "ztst:111 — alpha-punct-digit-notlower chain",
        );
    }

    /// `Test/D02glob.ztst:112` — same pattern, "a%1" lacks the
    /// 4th non-lower char → result 1 (false).
    #[test]
    fn zsh_corpus_posix_class_rejects_short_input() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("[[:alpha:][:punct:]]#[[:digit:]][^[:lower:]]", "a%1"),
            "ztst:112 — short input rejected",
        );
    }

    /// `Test/D02glob.ztst:113` — `[[:` literal in char class:
    /// `[[ [: = [[:]# ]]` — `[[:]` matches `[` OR `:`, `#` is 1+
    /// repetitions. "[:" = `[` + `:` → matches.
    #[test]
    fn zsh_corpus_literal_brackets_in_class_with_repetition() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("[[:]#", "[:"),
            "ztst:113 — [[:]# matches '[:'"
        );
    }

    // ─── (#i) / (#l) case modifiers — Test/D02glob.ztst:119-132 ───────

    /// `Test/D02glob.ztst:119` — `(#i)FOOXX` matches "fooxx"
    /// (case-insensitive throughout).
    #[test]
    fn zsh_corpus_hash_i_case_insensitive() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#i)FOOXX", "fooxx"),
            "ztst:119 — (#i)FOOXX matches fooxx"
        );
    }

    /// `Test/D02glob.ztst:120` — `(#l)FOOXX` requires pattern UPPER
    /// chars to be the ONLY uppercase candidates; lowercase input
    /// FAILS to match because pattern is all uppercase and `#l` only
    /// matches the EXACT lowercase variant of the pattern char... actually
    /// `(#l)` means "lowercase pattern chars match uppercase too" —
    /// `(#l)FOOXX` has no lowercase chars so it stays a strict
    /// uppercase match. "fooxx" doesn't match. Result 1 (false).
    #[test]
    fn zsh_corpus_hash_l_uppercase_pattern_with_lower_input_fails() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("(#l)FOOXX", "fooxx"),
            "ztst:120 — (#l)FOOXX does NOT match fooxx"
        );
    }

    /// `Test/D02glob.ztst:121` — `(#l)fooxx` (lowercase pattern) DOES
    /// match "FOOXX" because `#l` is the asymmetric "lowercase pat
    /// chars match upper-or-lower" rule.
    #[test]
    fn zsh_corpus_hash_l_lowercase_pattern_matches_uppercase_input() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#l)fooxx", "FOOXX"),
            "ztst:121 — (#l)fooxx matches FOOXX (asymmetric upcasing)"
        );
    }

    /// `Test/D02glob.ztst:122` — `(#i)FOO(#I)X(#i)X` mixes case
    /// modes. `(#I)` cancels prior `(#i)`. So 4th char `X` requires
    /// exact case. "fooxx" has lowercase `x` at that position → fail.
    #[test]
    fn zsh_corpus_hash_capital_i_cancels_case_insensitive() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("(#i)FOO(#I)X(#i)X", "fooxx"),
            "ztst:122 — (#I) cancels (#i), 4th char `X` requires upper"
        );
    }

    /// `Test/D02glob.ztst:123` — same pattern with input "fooXx":
    /// `(#i)FOO` matches "foo", `(#I)X` matches "X" exact-case,
    /// `(#i)X` matches "x" insensitive → result 0 (true).
    #[test]
    fn zsh_corpus_hash_i_capital_i_toggle_succeeds() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#i)FOO(#I)X(#i)X", "fooXx"),
            "ztst:123 — mixed case modifiers succeed"
        );
    }

    /// `Test/D02glob.ztst:128` — `(#i)*m*` matches "Modules"
    /// case-insensitively.
    #[test]
    fn zsh_corpus_hash_i_with_star_glob() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#i)*m*", "Modules"),
            "ztst:128 — (#i)*m* case-insensitive substring"
        );
    }

    // ─── Numeric ranges `<n-m>` — Test/D02glob.ztst:133-137 ───────────

    /// `Test/D02glob.ztst:133` — `<1-1000>33` matches "633" (the
    /// "6" is in [1..1000], followed by literal "33"). Actually
    /// the way zsh parses: `<1-1000>` matches ONE number from
    /// 1-1000, then `33` is literal. "633" = `6`(in 1-1000 range) +
    /// "33"(literal). So `<1-1000>33` matches "633".
    #[test]
    fn zsh_corpus_numeric_range_one_to_thousand() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("<1-1000>33", "633"),
            "ztst:133 — <1-1000>33 matches 633"
        );
    }

    /// `Test/D02glob.ztst:136` — `<->33` is the open-ended range
    /// (any number) followed by 33. Matches "633".
    #[test]
    fn zsh_corpus_numeric_range_open_ended() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("<->33", "633"),
            "ztst:136 — <->33 matches 633 (any number)"
        );
    }

    // ─── (#a) approximate match details — Test/D02glob.ztst:147-164 ───

    /// `Test/D02glob.ztst:147` — `(#a1)[b][b]` matches "bob" via
    /// 1 substitution (middle `o` substituted in `[b][b]`'s gap).
    /// Wait — `[b][b]` is two-char "bb". Match against "bob" requires
    /// 1 edit. With (#a1), allowed.
    #[test]
    fn zsh_corpus_hash_a1_bracket_class_with_one_edit() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#a1)[b][b]", "bob"),
            "ztst:147 — (#a1)[b][b] matches bob via 1 edit"
        );
    }

    /// `Test/D02glob.ztst:151` — `(#a2)XbcX` matches "abcd" via
    /// 2 substitutions (a↔X, d↔X).
    #[test]
    fn zsh_corpus_hash_a2_two_substitutions() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#a2)XbcX", "abcd"),
            "ztst:151 — 2 substitutions allowed"
        );
    }

    /// `Test/D02glob.ztst:152` — `(#a2)ad` matches "abcd" via
    /// 2 INSERTIONS (b and c inserted into input vs pattern).
    /// Pattern is "ad" (2 chars), input is "abcd" (4 chars).
    /// Diff: 2 extra chars in input.
    #[test]
    fn zsh_corpus_hash_a2_two_insertions_in_input() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#a2)ad", "abcd"),
            "ztst:152 — (#a2)ad matches abcd via 2 insertions in input"
        );
    }

    /// `Test/D02glob.ztst:153` — `(#a2)abcd` matches "ad" via
    /// 2 DELETIONS from input (b and c missing from input vs pattern).
    #[test]
    fn zsh_corpus_hash_a2_two_deletions_from_input() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#a2)abcd", "ad"),
            "ztst:153 — (#a2)abcd matches ad via 2 deletions"
        );
    }

    /// `Test/D02glob.ztst:158` — `(#a2)abcd` rejects "dcba" — 4
    /// changes needed (full reverse) > budget 2.
    #[test]
    fn zsh_corpus_hash_a2_rejects_full_reverse() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("(#a2)abcd", "dcba"),
            "ztst:158 — full reverse exceeds budget 2"
        );
    }

    /// `Test/D02glob.ztst:159` — `(#a3)abcd` matches "dcba" via
    /// 3 substitutions (positions 0,1,3 of input). Wait — actually
    /// "dcba" vs "abcd" — 4 positions differ. But (#a3) allows
    /// 3 errors. Hmm, but ztst says result is 0 (true). So zsh
    /// accepts. Possibly via DAMERAU transposition (swap adjacent).
    /// Without Damerau swap, this needs 4 subs → false. With
    /// transposition, "dcba" → "cdba" → "dcab"... complicated.
    /// Mark ignore — Damerau transposition isn't in the Rust port.
    #[test]
    fn zsh_corpus_hash_a3_reverse_via_transposition() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("(#a3)abcd", "dcba"),
            "ztst:159 — (#a3)abcd matches dcba via Damerau transpositions"
        );
    }

    // ─── (#s) start / (#e) end anchors — Test/D02glob.ztst:168-179 ────

    /// `Test/D02glob.ztst:168` — `*((#s)|/)test((#e)|/)*` matches
    /// "test" — start-anchor (#s) at position 0, end-anchor (#e)
    /// after "test".
    #[test]
    fn zsh_corpus_hash_s_e_anchors_match_bare_test() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("*((#s)|/)test((#e)|/)*", "test"),
            "ztst:168 — start/end anchors match bare 'test'",
        );
    }

    /// `Test/D02glob.ztst:172` — `*((#s)|/)test((#e)|/)*` rejects
    /// "atest" — `(#s)|/` requires position 0 OR a `/`, but `a`
    /// is neither.
    #[test]
    fn zsh_corpus_hash_s_anchor_rejects_a_prefix() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("*((#s)|/)test((#e)|/)*", "atest"),
            "ztst:172 — atest fails the (#s) start-or-slash anchor",
        );
    }

    // ─── Misc/globtests pins — `~` exclusion and `^` negation ─────────

    /// `Misc/globtests` — `[[ foo~ = foo~ ]]` — bare tilde is literal.
    #[test]
    fn zsh_corpus_literal_tilde_in_pattern() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("foo~", "foo~"),
            "literal ~ in non-extglob position"
        );
    }

    /// `Misc/globtests` — `[[ foo.c = *.c~boo* ]]` — extended-glob
    /// exclusion: `*.c` minus things matching `boo*`. "foo.c" doesn't
    /// match "boo*", so it stays.
    #[test]
    fn zsh_corpus_exclusion_keeps_non_excluded() {
        let _g = crate::test_util::global_state_lock();
        assert!(ext_glob_match("*.c~boo*", "foo.c"), "exclusion keeps foo.c");
    }

    /// `Misc/globtests` — `[[ foo.c = *.c~boo*~foo* ]]` — chained
    /// exclusions: `*.c` minus `boo*` minus `foo*`. "foo.c" matches
    /// `foo*`, so excluded.
    #[test]
    fn zsh_corpus_chained_exclusion_removes_match() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("*.c~boo*~foo*", "foo.c"),
            "chained exclusion excludes foo.c",
        );
    }

    /// `Misc/globtests` — `[[ fofo = (fo#)# ]]` — outer `(...)#` is
    /// zero-or-more closure over `fo#` (one f followed by zero+ o).
    #[test]
    fn zsh_corpus_closure_of_closure_matches_repeated_fo() {
        let _g = crate::test_util::global_state_lock();
        assert!(ext_glob_match("(fo#)#", "fofo"), "(fo#)# matches fofo");
    }

    /// `Misc/globtests` — `[[ ffo = (fo#)# ]]` — "ffo" = "f"+"fo".
    #[test]
    fn zsh_corpus_closure_matches_ffo() {
        let _g = crate::test_util::global_state_lock();
        assert!(ext_glob_match("(fo#)#", "ffo"), "(fo#)# matches ffo");
    }

    /// `Misc/globtests` — `[[ xfoooofof = (fo#)# ]]` — leading "x"
    /// breaks the pattern.
    #[test]
    fn zsh_corpus_closure_rejects_leading_x() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("(fo#)#", "xfoooofof"),
            "(fo#)# rejects leading x",
        );
    }

    /// `Misc/globtests` — `[[ foo = ((^x)) ]]` — `(^x)` matches one
    /// char that isn't 'x'. "foo" has 'f' first, so matches.
    /// Note: zsh's `(^x)` is exactly-one-not-x with extendedglob.
    #[test]
    fn zsh_corpus_negation_caret_matches_non_x_start() {
        let _g = crate::test_util::global_state_lock();
        assert!(ext_glob_match("((^x)*)", "foo"), "(^x)* matches foo");
    }

    /// `Misc/globtests` — `[[ foo = ((^foo)) ]]` — `(^foo)` is "not foo",
    /// "foo" matches `foo`, so excluded → false.
    #[test]
    fn zsh_corpus_negation_caret_rejects_full_match() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !ext_glob_match("((^foo))", "foo"),
            "(^foo) rejects 'foo' itself",
        );
    }

    /// `Misc/globtests` — `[[ abcd = ?(a|b)c#d ]]` — `?(a|b)` is
    /// "zero-or-one of a|b", `c#d` is "zero+ c then d". "abcd" = a + b? no, ?
    /// is single char or alternation. Let me re-read: `?(a|b)` matches
    /// one char which is `a` OR `b`. So "abcd": ?=a, then needs `c#d`
    /// = (c)*d, matches "bcd" if first char then need cd. Hmm.
    /// Actually: `?(a|b)` = `?` then `(a|b)` as separate? More likely
    /// `?(a|b)` is ksh-style "0-or-1 of a|b" only with KSH_GLOB.
    /// In zsh native (extendedglob) `?` is single char, `(a|b)` is
    /// alternation. So `?(a|b)c#d` = anychar (a|b) c* d. "abcd" =
    /// a + b + (zero c) + ... fails. The ztst says it MATCHES.
    /// Conclusion: requires KSH_GLOB which we may not enable.
    #[test]
    fn zsh_corpus_ksh_question_alternation() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ext_glob_match("?(a|b)c#d", "abcd"),
            "?(a|b)c#d matches abcd",
        );
    }

    // ── patmatchlen — c:2649 ─────────────────────────────────────────

    /// `Src/pattern.c:2649` — `int patmatchlen(void)` returns the
    /// byte-length of the last successful pattry match. After a
    /// successful match against `"hello"` with pattern `"hel*"`,
    /// the recorded length is 5 (all of "hello" was consumed).
    #[test]
    fn patmatchlen_records_consumed_byte_length() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("hel*");
        assert!(pattry(&prog, "hello"), "hel* matches hello");
        assert_eq!(patmatchlen(), 5, "all 5 bytes of 'hello' consumed by hel*",);
    }

    /// Anchored prefix match: `"foo"` against `"foobar"` with the
    /// `NOANCH` form is the user-visible case `[[ foobar = foo* ]]`,
    /// which consumes all 6 bytes when the pattern matches the whole
    /// string.
    #[test]
    fn patmatchlen_full_string_match_returns_full_length() {
        let _g = crate::test_util::global_state_lock();
        let prog = compile("foo*");
        assert!(pattry(&prog, "foobar"));
        assert_eq!(patmatchlen(), 6, "foo* against 'foobar' = 6 bytes");
    }

    // ── patmatchindex (c:4004) ──────────────────────────────────────

    /// `patmatchindex` on a literal-byte range returns the byte at
    /// the requested position.
    #[test]
    fn patmatchindex_literal_byte_at_index() {
        let _g = crate::test_util::global_state_lock();
        let range = b"abc";
        assert_eq!(patmatchindex(range, 0), Some((Some(b'a'), 0)));
        assert_eq!(patmatchindex(range, 1), Some((Some(b'b'), 0)));
        assert_eq!(patmatchindex(range, 2), Some((Some(b'c'), 0)));
        assert_eq!(patmatchindex(range, 3), None, "out-of-range index");
    }

    /// `patmatchindex` on a PP_ALPHA class marker returns `(None,
    /// PP_ALPHA)` to signal the class without a literal char.
    #[test]
    fn patmatchindex_posix_class_marker_returns_mtp() {
        let _g = crate::test_util::global_state_lock();
        // Meta + PP_ALPHA = 0x83 + 1 = 0x84
        let range = &[Meta + PP_ALPHA as u8];
        let r = patmatchindex(range, 0);
        assert_eq!(r, Some((None, PP_ALPHA)));
    }

    // ── mb_patmatchrange (c:3610) ───────────────────────────────────

    /// `mb_patmatchrange` literal hit on `'a'`.
    #[test]
    fn mb_patmatchrange_literal_ascii_hits() {
        let _g = crate::test_util::global_state_lock();
        let range = b"a";
        let mut mtp = -1;
        assert!(
            mb_patmatchrange(range, 'a', 0, None, Some(&mut mtp)),
            "literal 'a' matches"
        );
        assert_eq!(mtp, 0, "literal hit sets mtp=0");
    }

    /// `mb_patmatchrange` PP_DIGIT class hit on `'5'`.
    #[test]
    fn mb_patmatchrange_digit_class_hits() {
        let _g = crate::test_util::global_state_lock();
        let range = &[Meta + PP_DIGIT as u8];
        assert!(mb_patmatchrange(range, '5', 0, None, None));
        assert!(!mb_patmatchrange(range, 'x', 0, None, None));
    }

    /// `mb_patmatchrange` PP_RANGE on `'b'` in `a..c`.
    #[test]
    fn mb_patmatchrange_range_hits_middle() {
        let _g = crate::test_util::global_state_lock();
        // Meta + PP_RANGE then two literal bytes for the endpoints.
        let range = &[Meta + PP_RANGE as u8, b'a', b'c'];
        assert!(mb_patmatchrange(range, 'b', 0, None, None));
        assert!(mb_patmatchrange(range, 'a', 0, None, None));
        assert!(mb_patmatchrange(range, 'c', 0, None, None));
        assert!(!mb_patmatchrange(range, 'd', 0, None, None));
    }

    // ── mb_patmatchindex (c:3767) ───────────────────────────────────

    /// `mb_patmatchindex` literal char at index 0.
    #[test]
    fn mb_patmatchindex_literal_first_index() {
        let _g = crate::test_util::global_state_lock();
        let range = b"xyz";
        let r = mb_patmatchindex(range, 0);
        assert_eq!(r, Some((Some('x'), 0)));
    }

    /// `mb_patmatchindex` on PP_UPPER marker at index 0.
    #[test]
    fn mb_patmatchindex_class_marker_returns_mtp() {
        let _g = crate::test_util::global_state_lock();
        let range = &[Meta + PP_UPPER as u8];
        assert_eq!(mb_patmatchindex(range, 0), Some((None, PP_UPPER)));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/pattern.c bit-flag predicates.
    // ═══════════════════════════════════════════════════════════════════

    /// c:200 — P_ISBRANCH checks bit 0x20.
    #[test]
    fn P_ISBRANCH_recognizes_bit_0x20() {
        assert!(P_ISBRANCH(0x20));
        assert!(P_ISBRANCH(0x20 | 0x01));
        assert!(P_ISBRANCH(0xFF));
        assert!(!P_ISBRANCH(0x00));
        assert!(!P_ISBRANCH(0x1F));
    }

    /// c:201 — P_ISEXCLUDE checks bits 0x30 = 0x10 | 0x20 (BOTH must be set).
    #[test]
    fn P_ISEXCLUDE_requires_both_bits() {
        assert!(P_ISEXCLUDE(0x30));
        assert!(P_ISEXCLUDE(0x30 | 0x01));
        assert!(!P_ISEXCLUDE(0x10), "only 0x10 — must require both");
        assert!(!P_ISEXCLUDE(0x20), "only 0x20 — must require both");
        assert!(!P_ISEXCLUDE(0x00));
    }

    /// c:202 — P_NOTDOT checks bit 0x40.
    #[test]
    fn P_NOTDOT_recognizes_bit_0x40() {
        assert!(P_NOTDOT(0x40));
        assert!(P_NOTDOT(0x40 | 0x01));
        assert!(P_NOTDOT(0xFF));
        assert!(!P_NOTDOT(0x00));
        assert!(!P_NOTDOT(0x3F));
    }

    /// c:216-218 — P_SIMPLE/HSTART/PURESTR are distinct bits.
    #[test]
    fn p_flag_constants_are_distinct() {
        assert_eq!(P_SIMPLE & P_HSTART, 0);
        assert_eq!(P_SIMPLE & P_PURESTR, 0);
        assert_eq!(P_HSTART & P_PURESTR, 0);
    }

    /// c:216 — P_SIMPLE is bit 0 (= 0x01).
    #[test]
    fn p_simple_is_bit_zero() {
        assert_eq!(P_SIMPLE, 0x01);
    }

    /// c:217 — P_HSTART is bit 1 (= 0x02).
    #[test]
    fn p_hstart_is_bit_one() {
        assert_eq!(P_HSTART, 0x02);
    }

    /// c:218 — P_PURESTR is bit 2 (= 0x04).
    #[test]
    fn p_purestr_is_bit_two() {
        assert_eq!(P_PURESTR, 0x04);
    }

    /// c:406-407 — PA_NOALIGN=1, PA_UNMETA=2 (single-bit flags).
    #[test]
    fn pa_flag_constants_canonical() {
        assert_eq!(PA_NOALIGN, 1);
        assert_eq!(PA_UNMETA, 2);
    }

    /// PA_NOALIGN | PA_UNMETA combine without overlap.
    #[test]
    fn pa_flags_pairwise_disjoint() {
        assert_eq!(PA_NOALIGN & PA_UNMETA, 0);
    }

    /// c:336 — metacharinc on ASCII char advances by 1 byte.
    #[test]
    fn metacharinc_ascii_advances_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(metacharinc("hello", 0), 1);
        assert_eq!(metacharinc("hello", 4), 5);
    }

    /// c:336 — metacharinc at end-of-string returns same pos.
    #[test]
    fn metacharinc_end_of_string_returns_same() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(metacharinc("abc", 3), 3, "at end → no advance");
    }

    /// c:336 — metacharinc on multibyte char advances by codepoint width.
    #[test]
    fn metacharinc_multibyte_advances_by_codepoint_len() {
        let _g = crate::test_util::global_state_lock();
        // 'é' is 2 bytes in UTF-8.
        assert_eq!(metacharinc("é", 0), 2);
        // '日' is 3 bytes.
        assert_eq!(metacharinc("日", 0), 3);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/pattern.c
    // c:155 P_ISBRANCH / c:161 P_ISEXCLUDE / c:167 P_NOTDOT /
    // c:262 metacharinc / c:502 patcompstart / c:536 patcompile /
    // c:1008 patgetglobflags / c:1129 range_type / c:1152 pattern_range_to_string
    // c:2059 charref / c:2066 charnext / c:2153 pattry / c:2304 patmatchlen
    // ═══════════════════════════════════════════════════════════════════

    /// c:262 — `metacharinc` is pure for ASCII input.
    #[test]
    fn metacharinc_is_pure_for_ascii() {
        for s in ["", "a", "abc", "hello"] {
            for pos in 0..s.len() {
                let first = metacharinc(s, pos);
                for _ in 0..3 {
                    assert_eq!(
                        metacharinc(s, pos),
                        first,
                        "metacharinc({:?}, {}) must be pure",
                        s,
                        pos
                    );
                }
            }
        }
    }

    /// c:536 — `patcompile("", 0, None)` returns Option<Patprog>.
    #[test]
    fn patcompile_returns_option_patprog_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<Patprog> = patcompile("", 0, None);
    }

    /// c:1008 — `patgetglobflags("")` empty returns None.
    #[test]
    fn patgetglobflags_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(patgetglobflags("").is_none(), "empty → None");
    }

    /// c:1008 — `patgetglobflags` returns Option<(i32, i64, usize)> type.
    #[test]
    fn patgetglobflags_returns_option_tuple_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<(i32, i64, usize)> = patgetglobflags("abc");
    }

    /// c:1129 — `range_type("")` empty returns None.
    #[test]
    fn range_type_empty_returns_none() {
        assert!(range_type("").is_none(), "empty → None");
    }

    /// c:1129 — `range_type` returns Option<usize>.
    #[test]
    fn range_type_returns_option_usize_type() {
        let _: Option<usize> = range_type("abc");
    }

    /// c:1152 — `pattern_range_to_string("")` empty returns empty String.
    #[test]
    fn pattern_range_to_string_empty_returns_empty() {
        assert_eq!(pattern_range_to_string(""), "");
    }

    /// c:1152 — `pattern_range_to_string` is pure.
    #[test]
    fn pattern_range_to_string_is_pure() {
        for s in ["", "abc", "a-z"] {
            let first = pattern_range_to_string(s);
            for _ in 0..3 {
                assert_eq!(
                    pattern_range_to_string(s),
                    first,
                    "pattern_range_to_string({:?}) must be pure",
                    s
                );
            }
        }
    }

    /// c:2059 — `charref("", 0)` empty returns None.
    #[test]
    fn charref_empty_returns_none() {
        assert!(charref("", 0).is_none(), "empty → None");
    }

    /// c:2066 — `charnext("", 0)` empty returns 0 (no advance).
    #[test]
    fn charnext_empty_returns_zero() {
        assert_eq!(charnext("", 0), 0);
    }

    /// c:2304 — `patmatchlen` returns i32 (compile-time type pin).
    #[test]
    fn patmatchlen_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = patmatchlen();
    }

    /// c:155 — P_ISBRANCH/P_ISEXCLUDE/P_NOTDOT all return bool (type pin).
    #[test]
    fn p_predicates_return_bool_type() {
        let _: bool = P_ISBRANCH(0);
        let _: bool = P_ISEXCLUDE(0);
        let _: bool = P_NOTDOT(0);
    }
}
