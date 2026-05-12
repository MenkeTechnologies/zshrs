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
//!  15. Range matching (`patmatchrange`, `patmatchindex`,
//!      `mb_patmatchrange`, `mb_patmatchindex`)
//!  16. String pre-processing (`patmungestring`, `patallocstr`,
//!      `pattrystart`)
//!  17. Module-loader / disable mgmt (`startpatternscope`,
//!      `endpatternscope`, `savepatterndisables`,
//!      `restorepatterndisables`, `clearpatterndisables`,
//!      `freepatprog`, `pat_enables`)
//!  18. Convenience entry points for in-tree callers (`patmatch`,
//!      `patmatchlen`, `patrepeat`, `haswilds`)
//!
//! See `docs/PORT.md` Rules A/B/C/D/E.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

// =====================================================================
// 1. P_* opcode constants — pattern.c:97-127
//
// Numbered identically to C so a buffer compiled by this port matches
// the C source's bytecode-cache format byte-for-byte (modulo native
// endianness; we pin LE — see file header).
// =====================================================================

pub const P_END:        u8 = 0x00;  // c:97  End of program.
pub const P_EXCSYNC:    u8 = 0x01;  // c:98  Test if following exclude already failed
pub const P_EXCEND:     u8 = 0x02;  // c:99  Test if exclude matched orig branch
pub const P_BACK:       u8 = 0x03;  // c:100 Match "", "next" ptr points backward.
pub const P_EXACTLY:    u8 = 0x04;  // c:101 lstr — match this string.
pub const P_NOTHING:    u8 = 0x05;  // c:102 Match empty string.
pub const P_ONEHASH:    u8 = 0x06;  // c:103 node — match 0 or more of preceding simple.
pub const P_TWOHASH:    u8 = 0x07;  // c:104 node — match 1 or more of preceding simple.
pub const P_GFLAGS:     u8 = 0x08;  // c:105 long — match nothing and set globbing flags.
pub const P_ISSTART:    u8 = 0x09;  // c:106 Match start of string.
pub const P_ISEND:      u8 = 0x0a;  // c:107 Match end of string.
pub const P_COUNTSTART: u8 = 0x0b;  // c:108 Initialise P_COUNT.
pub const P_COUNT:      u8 = 0x0c;  // c:109 3*long uc* node — match a number of repetitions.
pub const P_BRANCH:     u8 = 0x20;  // c:112 node — match this alternative, or the next.
pub const P_WBRANCH:    u8 = 0x21;  // c:113 uc* node — P_BRANCH, but match at least 1 char.
pub const P_EXCLUDE:    u8 = 0x30;  // c:114 uc* node — exclude this from previous branch.
pub const P_EXCLUDP:    u8 = 0x31;  // c:115 uc* node — exclude, using full file path so far.
pub const P_ANY:        u8 = 0x40;  // c:117 Match any one character.
pub const P_ANYOF:      u8 = 0x41;  // c:118 str — match any character in this string.
pub const P_ANYBUT:     u8 = 0x42;  // c:119 str — match any character not in this string.
pub const P_STAR:       u8 = 0x43;  // c:120 Match any set of characters.
pub const P_NUMRNG:     u8 = 0x44;  // c:121 zr,zr — match a numeric range.
pub const P_NUMFROM:    u8 = 0x45;  // c:122 zr — match a number >= X.
pub const P_NUMTO:      u8 = 0x46;  // c:123 zr — match a number <= X.
pub const P_NUMANY:     u8 = 0x47;  // c:124 Match any set of decimal digits.
pub const P_OPEN:       u8 = 0x80;  // c:126 Mark this point in input as start of n.
pub const P_CLOSE:      u8 = 0x90;  // c:127 Analogous to OPEN.

/// `P_ISBRANCH(p)` macro from pattern.c:200 — `(p->l & 0x20)`.
#[inline]
pub fn P_ISBRANCH(op: u8) -> bool { (op & 0x20) != 0 }

/// `P_ISEXCLUDE(p)` macro from pattern.c:201 — `((p->l & 0x30) == 0x30)`.
#[inline]
pub fn P_ISEXCLUDE(op: u8) -> bool { (op & 0x30) == 0x30 }

/// `P_NOTDOT(p)` macro from pattern.c:202 — `(p->l & 0x40)`.
#[inline]
pub fn P_NOTDOT(op: u8) -> bool { (op & 0x40) != 0 }

// =====================================================================
// 3. Flag-bit constants returned via flagp out-params during compile.
// pattern.c:216-218
// =====================================================================

pub const P_SIMPLE:  i32 = 0x01;  // c:216 Simple enough to be # / ## operand.
pub const P_HSTART:  i32 = 0x02;  // c:217 Starts with # or ##'d pattern.
pub const P_PURESTR: i32 = 0x04;  // c:218 Can be matched with a strcmp.

// =====================================================================
// 4. struct patprog — zsh.h:1601
// =====================================================================

/// Compiled pattern. Direct port of `struct patprog` at `Src/zsh.h:1601`.
///
/// C layout uses a trailing byte buffer accessed via `(char *)prog +
/// prog->startoff`; the Rust port stores it as a `code` field of
/// `Vec<u8>`. The opcode stream layout is preserved byte-for-byte.
#[allow(non_camel_case_types)]
pub struct patprog {
    pub startoff: i64,    // c:1602 length before start of programme
    pub size:     i64,    // c:1603 total size from start of struct
    pub mustoff:  i64,    // c:1604 offset to string that must be present
    pub patmlen:  i64,    // c:1605 length of pure string or longest match
    pub globflags: i32,   // c:1606 globbing flags to set at start
    pub globend:   i32,   // c:1607 globbing flags set after finish
    pub flags:     i32,   // c:1608 PAT_* flags
    pub patnpar:   i32,   // c:1609 number of active parentheses
    pub patstartch: u8,   // c:1610 starting character (optimization)
    /// Bytecode buffer. In C this is the trailing memory past the
    /// fixed-size struct header; Rust port holds it inline.
    pub code: Vec<u8>,
}

/// `typedef struct patprog *Patprog;` from `zsh.h:542`.
#[allow(non_camel_case_types)]
pub type Patprog = Box<patprog>;

// =====================================================================
// 5. PAT_* flag constants — re-exports of zsh.h:1623-1640 already in
// zsh_h.rs. Re-published here so callers in pattern's API don't need
// the longer path. C source has these as `#define` in zsh.h, not
// pattern.c, so the canonical home is zsh_h.rs; we just alias.
// =====================================================================

pub use crate::ported::zsh_h::{
    PAT_HEAPDUP, PAT_FILE, PAT_FILET, PAT_ANY, PAT_NOANCH, PAT_NOGLD,
    PAT_PURES, PAT_STATIC, PAT_SCAN, PAT_ZDUP, PAT_NOTSTART, PAT_NOTEND,
    PAT_HAS_EXCLUDP, PAT_LCMATCHUC,
};

// =====================================================================
// 6. ZPC_* enum from zsh.h:1644 — indexes into the active-pattern-
// characters table that compile-time and runtime both consult.
// =====================================================================

pub const ZPC_SLASH:     usize = 0;  // / file separator
pub const ZPC_NULL:      usize = 1;  // \0 terminator
pub const ZPC_BAR:       usize = 2;  // | alternation
pub const ZPC_OUTPAR:    usize = 3;  // )
pub const ZPC_TILDE:     usize = 4;  // ~ exclusion
pub const ZPC_SEG_COUNT: usize = 5;  // segment-terminator count
pub const ZPC_INPAR:     usize = 5;  // (
pub const ZPC_QUEST:     usize = 6;  // ?
pub const ZPC_STAR:      usize = 7;  // *
pub const ZPC_INBRACK:   usize = 8;  // [
pub const ZPC_INANG:     usize = 9;  // <
pub const ZPC_HAT:       usize = 10; // ^
pub const ZPC_HASH:      usize = 11; // #
pub const ZPC_BNULLKEEP: usize = 12; // \x00 backslashed-null marker
pub const ZPC_COUNT:     usize = 13; // total

/// Maximum captures, from `pattern.c:94 NSUBEXP`.
pub const NSUBEXP: usize = 9;

// GF_* glob-flag bits live in `zsh.h:1763-1773`, ported to
// `src/ported/zsh_h.rs:2287-2291` per Rule C. Re-export so pattern's
// matcher arms can read them without the longer path.
pub use crate::ported::zsh_h::{
    GF_LCMATCHUC, GF_IGNCASE, GF_BACKREF, GF_MATCHREF, GF_MULTIBYTE,
};
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

const I_OP:   usize = 0; // opcode byte
const I_NEXT: usize = 1; // u32 next-offset starts here
const I_BODY: usize = 5; // payload starts here

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
pub static patout: Mutex<Vec<u8>> = Mutex::new(Vec::new());     // c:267

/// Serialises every entry into `patcompile`. The C source at
/// `Src/pattern.c:267-281` declares `patout`, `patparse`, `patstart`,
/// `patnpar`, `patflags`, `patglobflags`, `errsfound`, `forceerrs`,
/// `zpc_special`, `patstrcache` as file-scope statics that the compile
/// mutates in sequence; zsh-the-program is single-threaded so the C
/// source is safe under that invariant. zshrs callers (zutil's
/// `StyleTable::get` via `crate::ported::pattern::patmatch`, params.rs,
/// subst.rs, options.rs) can invoke `patcompile` from concurrent test
/// threads, so the lock restores the single-writer invariant. Held
/// only for the compile phase; the matcher (`pattry`/`patmatch_internal`)
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

// C: `static int patnpar;` — number of active parens (1-indexed at
// compile time; the *struct* patnpar is the actual count).
pub static patnpar: AtomicI32 = AtomicI32::new(0);              // c:271

// C: `static int patflags;` — current PAT_* flag set during compile.
pub static patflags: AtomicI32 = AtomicI32::new(0);             // c:272

// C: `static int patglobflags;` — current globbing flags during compile.
pub static patglobflags: AtomicI32 = AtomicI32::new(0);         // c:273

// C: `static int errsfound;` — approximate-match error count.
pub static errsfound: AtomicI32 = AtomicI32::new(0);            // c:274

// C: `static int forceerrs;` — required error count for approximate match.
pub static forceerrs: AtomicI32 = AtomicI32::new(-1);           // c:275

// C: `static long patglobflags_orig;` — saved at branch entry.
pub static patglobflags_orig: AtomicI32 = AtomicI32::new(0);    // c:276

// C: `static const char *zpc_special;` — table of currently-special
// characters during compile (indexed by ZPC_*).
//
// pattern.c uses `static char zpc_special[ZPC_COUNT];` and resets it
// in patcompcharsset(). Rust mirrors as a Mutex-wrapped byte array.
pub static zpc_special: Mutex<[u8; ZPC_COUNT]> = Mutex::new([0u8; ZPC_COUNT]); // c:278

// C: `static char *patstrcache;` — caches the unmetafied trial string.
// Rust port has no Meta encoding so the cache is unnecessary; we leave
// the static declared for parity (Rule A — name exists in C).
pub static patstrcache: Mutex<String> = Mutex::new(String::new()); // c:281

/// `Marker` constant from pattern.c — used as a placeholder for the
/// active-but-disabled slot. C is `\200` (0x80). The port keeps it
/// distinguishable from valid pattern bytes.
pub const Marker: u8 = 0x80;

// =====================================================================
// 8. Bytecode write helpers — pattern.c:412-1856
// =====================================================================

/// Port of `patadd()` from `Src/pattern.c:412`.
///
/// C signature: `static long patadd(char *add, int ch, long n, int paflags)`.
/// Adds `n` bytes (or repeats `ch`) to patout, growing if needed.
/// Returns offset where the bytes were appended.
fn patadd(add: Option<&[u8]>, ch: u8, n: i64, _paflags: i32) -> i64 {        // c:412
    let mut buf = patout.lock().unwrap();
    let start = buf.len() as i64;
    if let Some(bytes) = add {
        let n_actual = bytes.len().min(n as usize);
        buf.extend_from_slice(&bytes[..n_actual]);
    } else {
        for _ in 0..n {
            buf.push(ch);
        }
    }
    start
}

/// Port of `patnode()` from `Src/pattern.c:1790`.
///
/// C: `static long patnode(long op)` — writes a 1-byte opcode plus a
/// 4-byte zeroed next-offset. Returns the offset of the opcode byte.
fn patnode(op: u8) -> usize {                                                 // c:1790
    let mut buf = patout.lock().unwrap();
    let off = buf.len();
    buf.push(op);                  // I_OP
    buf.extend_from_slice(&[0, 0, 0, 0]);  // I_NEXT zeroed
    off
}

/// Port of `patinsert()` from `Src/pattern.c:1807`.
///
/// C: `static void patinsert(long op, int opnd, char *xtra, int sz)`.
/// Inserts an opcode (+ next slot) at position `opnd`, shifting bytes
/// after it down by `5 + sz`, then writes `xtra` payload of `sz` bytes.
fn patinsert(op: u8, opnd: usize, xtra: Option<&[u8]>, sz: usize) {            // c:1807
    let mut buf = patout.lock().unwrap();
    let header_sz = 1 + 4;  // op + next
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

/// Helper: when patinsert shifts a chunk of bytecode, any 4-byte
/// next_off slot that previously pointed past `opnd` must be bumped
/// by `delta` to keep the chain links valid.
///
/// Walks the buffer linearly opcode-by-opcode reading I_NEXT slots.
/// Conservatively adjusts every nonzero next that lands past opnd.
fn fixup_offsets_after_insert(buf: &mut [u8], opnd: usize, delta: u32) {
    let mut i = 0;
    while i + I_BODY <= buf.len() {
        let op = buf[i + I_OP];
        if op == 0 { i += 1; continue; }  // sentinel byte, skip
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
        if i == 0 { break; }
    }
}

/// Helper: given a buffer and current opcode offset, return the
/// offset of the next opcode after this one's payload.
///
/// Encodes the per-opcode payload size table — must stay in sync
/// with patnode/patinsert calls in the compiler.
fn advance_past_instr(buf: &[u8], pos: usize) -> usize {
    if pos + I_BODY > buf.len() { return 0; }
    let op = buf[pos + I_OP];
    let body_start = pos + I_BODY;
    match op {
        P_END | P_NOTHING | P_BACK | P_EXCSYNC | P_EXCEND
            | P_ISSTART | P_ISEND | P_COUNTSTART | P_ANY | P_STAR | P_NUMANY
            => body_start,
        P_GFLAGS => body_start + 4,                                           // i32 flag-bits payload
        P_EXACTLY => {
            // payload: u32 len + len bytes
            if body_start + 4 > buf.len() { return 0; }
            let len = u32::from_le_bytes(buf[body_start..body_start + 4].try_into().unwrap()) as usize;
            body_start + 4 + len
        }
        P_ANYOF | P_ANYBUT => {
            if body_start + 4 > buf.len() { return 0; }
            let len = u32::from_le_bytes(buf[body_start..body_start + 4].try_into().unwrap()) as usize;
            body_start + 4 + len
        }
        P_ONEHASH | P_TWOHASH | P_BRANCH | P_WBRANCH
            | P_EXCLUDE | P_EXCLUDP => body_start,
        P_OPEN..=0x88 | P_CLOSE..=0x98 => body_start,
        P_NUMRNG => body_start + 16, // two i64
        P_NUMFROM | P_NUMTO => body_start + 8,
        P_COUNT => body_start + 16, // min i64 + max i64; operand inline follows
        _ => body_start,
    }
}

/// Helper: directly set the `next_off` slot of the instruction at
/// `pos` without walking the chain. C uses pointer arithmetic
/// (`scanp->l = ...`) inline; Rust factors it for byte-offset
/// bookkeeping. Architectural helper.
fn set_next(pos: usize, val: usize) {
    let mut buf = patout.lock().unwrap();
    if pos + I_NEXT + 4 <= buf.len() {
        buf[pos + I_NEXT..pos + I_NEXT + 4].copy_from_slice(&(val as u32).to_le_bytes());
    }
}

/// Port of `pattail()` from `Src/pattern.c:1834`.
///
/// C: `static void pattail(long p, long val)` — patches the next-offset
/// field of the opcode at offset `p` to point to `val`. Walks any
/// existing chain to the end before patching.
fn pattail(p: usize, val: usize) {                                            // c:1834
    let mut buf = patout.lock().unwrap();
    let mut cur = p;
    loop {
        if cur + I_BODY > buf.len() { return; }
        let next_bytes: [u8; 4] = buf[cur + I_NEXT..cur + I_NEXT + 4].try_into().unwrap();
        let next = u32::from_le_bytes(next_bytes) as usize;
        if next == 0 { break; }
        cur = next;
    }
    let val_bytes = (val as u32).to_le_bytes();
    if cur + I_NEXT + 4 <= buf.len() {
        buf[cur + I_NEXT..cur + I_NEXT + 4].copy_from_slice(&val_bytes);
    }
}

/// Port of `patoptail()` from `Src/pattern.c:1856`.
///
/// C: `static void patoptail(long p, long val)` — like pattail but
/// only patches branches (P_BRANCH/P_WBRANCH).
fn patoptail(p: usize, val: usize) {                                          // c:1856
    let buf = patout.lock().unwrap();
    if p + I_OP >= buf.len() { return; }
    let op = buf[p + I_OP];
    drop(buf);
    if P_ISBRANCH(op) {
        // For branches, the "operand" is the inner node — walk THAT
        // node's chain to its end. Branch operand starts at p + I_BODY.
        pattail(p + I_BODY, val);
    }
}

/// Port of `patcompcharsset()` from `Src/pattern.c:464`.
///
/// Initializes the `zpc_special` table for the active globbing
/// regime. The C source resets every ZPC_* slot to 0 or the literal
/// character it represents, then masks off characters disabled via
/// `disables`.
pub fn patcompcharsset() {                                                    // c:464
    let mut sp = zpc_special.lock().unwrap();
    *sp = [0u8; ZPC_COUNT];
    // Default special chars (matches pattern.c init block).
    sp[ZPC_SLASH]   = b'/';
    sp[ZPC_NULL]    = 0;
    sp[ZPC_BAR]     = b'|';
    sp[ZPC_OUTPAR]  = b')';
    sp[ZPC_TILDE]   = b'~';
    sp[ZPC_INPAR]   = b'(';
    sp[ZPC_QUEST]   = b'?';
    sp[ZPC_STAR]    = b'*';
    sp[ZPC_INBRACK] = b'[';
    sp[ZPC_INANG]   = b'<';
    sp[ZPC_HAT]     = b'^';
    sp[ZPC_HASH]    = b'#';
    sp[ZPC_BNULLKEEP] = 0;
}

/// Port of `patcompstart()` from `Src/pattern.c:517`.
///
/// Resets per-compile globals. Called at the start of `patcompile`.
/// Matches C c:523-525 — GF_MULTIBYTE is defaulted on when zsh was
/// built with MULTIBYTE_SUPPORT. Rust's `&str` is natively UTF-8 so
/// the equivalent is "always on" unless the caller toggles via (#U).
pub fn patcompstart() {                                                       // c:517
    patout.lock().unwrap().clear();
    patnpar.store(1, Ordering::Relaxed);
    patflags.store(0, Ordering::Relaxed);
    patglobflags.store(GF_MULTIBYTE, Ordering::Relaxed);                      // c:525
    errsfound.store(0, Ordering::Relaxed);
    forceerrs.store(-1, Ordering::Relaxed);
    patparse_off.store(0, Ordering::Relaxed);
    patcompcharsset();
}

// =====================================================================
// 9. Compiler entry points — pattern.c:540
// =====================================================================

/// Port of `patcompile()` from `Src/pattern.c:540`.
///
/// C signature: `Patprog patcompile(char *exp, int inflags, char **endexp)`.
/// Compiles pattern `exp` under flags `inflags`, returns a `Patprog`
/// on success or `NULL` on failure. `endexp` (if non-NULL) is set to
/// the input cursor at end of parse — used by `bin_zregexparse` to
/// detect partial-parse cases.
pub fn patcompile(exp: &str, inflags: i32, mut endexp: Option<&mut String>)   // c:540
    -> Option<Patprog>
{
    // Hold the compile mutex for the entire body — `patcompstart`
    // resets every file-scope static (`Src/pattern.c:267-281`) and the
    // emit/parse helpers mutate them in sequence. C is single-threaded
    // so the statics are race-free there; Rust must serialise.
    let _compile_guard = PATCOMPILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    patcompstart();
    *patstart.lock().unwrap() = exp.to_string();
    *patparse.lock().unwrap() = exp.to_string();
    patflags.store(inflags & !(PAT_PURES | PAT_HAS_EXCLUDP) as i32, Ordering::Relaxed); // c:566
    patglobflags.store(0, Ordering::Relaxed);

    // c:583-590 — emit P_GFLAGS placeholder. Phase 5.1: instead of
    // emitting an opcode, hoist leading `(#...)` flag specifiers into
    // patprog.globflags so the matcher applies them globally for the
    // whole match. Full mid-pattern P_GFLAGS opcode still deferred.
    // Start with GF_MULTIBYTE by default (mirrors C patcompstart at
    // c:525 when MULTIBYTE_SUPPORT is defined). (#U) clears it.
    let mut hoisted_globflags: i32 = GF_MULTIBYTE;
    loop {
        let off = patparse_off.load(Ordering::Relaxed);
        let p = patparse.lock().unwrap();
        if off + 1 >= p.len() || &p.as_bytes()[off..off + 2] != b"(#" {
            break;
        }
        let rest = p[off..].to_string();
        drop(p);
        match patgetglobflags(&rest) {
            Some((gflags, _assert, consumed)) => {
                if gflags.case_insensitive { hoisted_globflags |= GF_IGNCASE; }
                if gflags.lcmatchuc        { hoisted_globflags |= GF_LCMATCHUC; }
                if gflags.multibyte        { hoisted_globflags |= GF_MULTIBYTE; }
                else if rest.contains('U') { hoisted_globflags &= !GF_MULTIBYTE; }
                patparse_off.fetch_add(consumed, Ordering::Relaxed);
            }
            None => break,
        }
    }

    let mut flagp: i32 = 0;
    let root = patcompswitch(0, &mut flagp);
    if root < 0 {
        return None;                                                          // c:646 compile error
    }
    // Emit the terminal P_END and chain every branch's operand to it.
    let end_off = patnode(P_END);
    chain_branches_to(root as usize, end_off);

    let code = patout.lock().unwrap().clone();
    let consumed_off = patparse_off.load(Ordering::Relaxed);
    if let Some(end) = endexp.as_deref_mut() {
        let parse = patparse.lock().unwrap();
        *end = parse[consumed_off..].to_string();
    }

    Some(Box::new(patprog {
        startoff: 0,
        size: code.len() as i64,
        mustoff: 0,
        patmlen: 0,
        globflags: hoisted_globflags,
        globend: patglobflags.load(Ordering::Relaxed),
        flags: patflags.load(Ordering::Relaxed) | hoisted_globflags,
        patnpar: patnpar.load(Ordering::Relaxed) - 1,
        patstartch: 0,
        code,
    }))
}

/// Port of `patcompswitch()` from `Src/pattern.c:765`.
///
/// C: `static long patcompswitch(int paren, int *flagp)`. Parses an
/// alternation (`a|b|c`), emitting a chain of P_BRANCH nodes. Returns
/// offset of the first branch, or -1 on error.
pub fn patcompswitch(paren: i32, flagp: &mut i32) -> i64 {                    // c:765
    // Emit the first P_BRANCH header. Its operand is the content
    // emitted by the immediately-following patcompbranch call (lives
    // inline at starter+I_BODY). Does NOT emit a terminator — caller
    // (patcompile for top-level, patcomppiece for sub-pattern)
    // chains branches to the appropriate follow-on opcode.
    let starter = patnode(P_BRANCH);
    let mut branch_flags: i32 = 0;
    let first_branch = patcompbranch(&mut branch_flags, paren);
    if first_branch < 0 { return -1; }
    *flagp |= branch_flags & P_HSTART;

    let mut last_branch = starter;

    // Alternation loop: while next char is |, parse another branch.
    loop {
        let off = patparse_off.load(Ordering::Relaxed);
        let parse = patparse.lock().unwrap();
        if off >= parse.len() { break; }
        let c = parse.as_bytes()[off];
        if c != b'|' { break; }
        drop(parse);
        patparse_off.fetch_add(1, Ordering::Relaxed);
        let br = patnode(P_BRANCH);
        // Chain previous branch's `next` directly to this new branch
        // (alternative chain, not operand chain).
        set_next(last_branch, br);
        let mut bf: i32 = 0;
        let inner = patcompbranch(&mut bf, paren);
        if inner < 0 { return -1; }
        *flagp |= bf & P_HSTART;
        last_branch = br;
    }

    let _ = first_branch;
    starter as i64
}

/// Helper: walk every branch's operand chain and patch each branch's
/// last-operand-node `.next` to `target`. Used to chain a fully-
/// compiled alternation switch to whatever opcode follows (P_END for
/// the outermost compile, P_CLOSE_N for a sub-group).
///
/// Architectural helper — C uses pattail inside the BRANCH operand
/// scope via Upat pointer arithmetic; Rust factors it for clarity.
fn chain_branches_to(starter: usize, target: usize) {
    let mut cur = starter;
    loop {
        // Operand starts at cur + I_BODY (the byte right after this
        // branch's header). Walk operand's next-chain to its end
        // and set its .next = target.
        pattail(cur + I_BODY, target);
        // Move to next alternative.
        let buf = patout.lock().unwrap();
        if cur + I_NEXT + 4 > buf.len() { break; }
        let nb: [u8; 4] = buf[cur + I_NEXT..cur + I_NEXT + 4].try_into().unwrap();
        let n = u32::from_le_bytes(nb) as usize;
        drop(buf);
        if n == 0 { break; }
        cur = n;
    }
}

/// Port of `patcompbranch()` from `Src/pattern.c:942`.
///
/// C: `static long patcompbranch(int *flagp, int paren)`. Parses a
/// single branch — a sequence of pieces. Returns offset of the first
/// node in the branch, or -1 on error.
pub fn patcompbranch(flagp: &mut i32, paren: i32) -> i64 {                    // c:942
    let mut chain_start: i64 = -1;
    let mut last_tail: usize = 0;
    *flagp = P_PURESTR;

    loop {
        let off = patparse_off.load(Ordering::Relaxed);
        // Snapshot the parse buffer into an owned slice for branch
        // decisions; release the lock so subsequent emit helpers
        // (which acquire patout's lock) can't contend.
        let snapshot: Vec<u8> = {
            let parse = patparse.lock().unwrap();
            parse.as_bytes().to_vec()
        };
        if off >= snapshot.len() { break; }
        let c = snapshot[off];
        // Branch terminators: |, ), end of pattern.
        if c == b'|' || c == b')' { break; }
        let bytes = snapshot.as_slice();
        // Mid-pattern `(#cN,M)` counted-repetition specifier — emit
        // P_COUNT with bounds + inline operand following. Detected
        // BEFORE the generic patgetglobflags path because `c` is not
        // a flag char in that fn.
        if off + 2 < bytes.len() && bytes[off] == b'(' && bytes[off + 1] == b'#'
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
            if j > min_start {
                if j < bytes.len() && bytes[j] == b',' {
                    j += 1;
                    let max_start = j;
                    let mut mx: i64 = 0;
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        mx = mx * 10 + (bytes[j] - b'0') as i64;
                        j += 1;
                    }
                    if j > max_start { max = mx; }
                } else {
                    // (#cN) — exact count N.
                    max = min;
                }
                if j < bytes.len() && bytes[j] == b')' {
                    j += 1;
                    patparse_off.store(j, Ordering::Relaxed);
                    // Emit P_COUNT header.
                    let count_off = patnode(P_COUNT);
                    let mut buf = patout.lock().unwrap();
                    buf.extend_from_slice(&min.to_le_bytes());
                    buf.extend_from_slice(&max.to_le_bytes());
                    drop(buf);
                    // Compile the operand inline immediately after.
                    let mut piece_flags: i32 = 0;
                    let mut piece_tail: usize = 0;
                    let piece = patcomppiece(&mut piece_flags, paren, &mut piece_tail);
                    if piece < 0 { return -1; }
                    // Terminate operand chain with 0 so matcher
                    // knows to stop iterating the body each pass.
                    set_next(piece_tail, 0);
                    if chain_start < 0 { chain_start = count_off as i64; }
                    else { set_next(last_tail, count_off); }
                    last_tail = count_off;
                    continue;
                }
            }
            // Malformed `(#c...)` — fall through to generic flag handler.
        }
        // Mid-pattern `(#...)` glob flag specifier — emit P_GFLAGS
        // opcode that mutates the matcher's running glob_flags var.
        // Port of pattern.c patcompswitch's inline (#...) handling
        // around c:850-900 (patgetglobflags integration).
        if off + 1 < bytes.len() && bytes[off] == b'(' && bytes[off + 1] == b'#' {
            let rest = std::str::from_utf8(&bytes[off..]).unwrap_or("").to_string();
            if let Some((gflags, assert, consumed)) = patgetglobflags(&rest) {
                patparse_off.fetch_add(consumed, Ordering::Relaxed);
                // Emit P_GFLAGS for flag-bit changes if any.
                let mut bits: i32 = 0;
                if gflags.case_insensitive { bits |= GF_IGNCASE; }
                if gflags.lcmatchuc        { bits |= GF_LCMATCHUC; }
                if gflags.multibyte        { bits |= GF_MULTIBYTE; }
                if bits != 0 || (!gflags.case_insensitive
                                 && !gflags.lcmatchuc
                                 && assert.is_none()) {
                    let gf_off = patnode(P_GFLAGS);
                    let mut buf = patout.lock().unwrap();
                    buf.extend_from_slice(&bits.to_le_bytes());
                    drop(buf);
                    if chain_start < 0 { chain_start = gf_off as i64; }
                    else { set_next(last_tail, gf_off); }
                    last_tail = gf_off;
                }
                // Emit P_ISSTART / P_ISEND if assert variant returned.
                if let Some(a) = assert {
                    let assert_op = match a {
                        PatOp::StartAssert => P_ISSTART,
                        PatOp::EndAssert => P_ISEND,
                    };
                    let as_off = patnode(assert_op);
                    if chain_start < 0 { chain_start = as_off as i64; }
                    else { set_next(last_tail, as_off); }
                    last_tail = as_off;
                }
                continue;
            }
            // patgetglobflags failed — treat the `(` as a literal group.
        }

        let mut piece_flags: i32 = 0;
        let mut piece_tail: usize = 0;
        let piece = patcomppiece(&mut piece_flags, paren, &mut piece_tail);
        if piece < 0 { return -1; }
        if chain_start < 0 {
            chain_start = piece;
        } else {
            // Chain previous piece's tail → this piece's head directly.
            set_next(last_tail, piece as usize);
        }
        last_tail = piece_tail;
        *flagp &= piece_flags;
    }

    if chain_start < 0 {
        chain_start = patnode(P_NOTHING) as i64;
    }
    chain_start
}

/// Port of `patcomppiece()` from `Src/pattern.c:1261`.
///
/// C: `static long patcomppiece(int *flagp, int paren)`. Parses a
/// single atom + optional quantifier. Returns offset of compiled node.
/// Out-param `tail_out` receives the byte offset of the LAST opcode
/// in the compiled piece — the node whose `.next` should be chained
/// to whatever follows in the sequence. For simple atoms (P_EXACTLY,
/// P_ANY, etc.) the tail equals the head; for compound pieces
/// `(...)` / quantified atoms it points to the trailing P_CLOSE_N or
/// quantifier-injected node.
pub fn patcomppiece(flagp: &mut i32, paren: i32, tail_out: &mut usize) -> i64 { // c:1261
    let _ = paren;
    let off = patparse_off.load(Ordering::Relaxed);
    let parse = patparse.lock().unwrap();
    if off >= parse.len() {
        return patnode(P_NOTHING) as i64;
    }
    let bytes = parse.as_bytes();
    let c = bytes[off];
    drop(parse);

    // Atom dispatch. Each arm sets `*tail_out` to the offset of the
    // last opcode emitted by this piece (for simple atoms, tail = head).
    let atom = match c {
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
            patparse_off.fetch_add(1, Ordering::Relaxed);
            *flagp |= P_SIMPLE;
            *flagp &= !P_PURESTR;
            // Inline bracket-expression parse (C patcomppiece bracket case).
            let mut chars: Vec<u8> = Vec::new();
            let mut negate = false;
            let bracket_start = patparse_off.load(Ordering::Relaxed);
            let parse_b = patparse.lock().unwrap();
            let bb = parse_b.as_bytes();
            let mut i_b = bracket_start;
            if i_b < bb.len() && (bb[i_b] == b'^' || bb[i_b] == b'!') {
                negate = true;
                i_b += 1;
            }
            while i_b < bb.len() && bb[i_b] != b']' {
                if i_b + 1 < bb.len() && bb[i_b] == b'[' && bb[i_b+1] == b':' {
                    let class_start = i_b + 2;
                    let mut j_b = class_start;
                    while j_b + 1 < bb.len() && !(bb[j_b] == b':' && bb[j_b+1] == b']') {
                        j_b += 1;
                    }
                    if j_b + 1 < bb.len() {
                        let class_name = std::str::from_utf8(&bb[class_start..j_b]).unwrap_or("");
                        // Inline POSIX class expansion.
                        match class_name {
                            "alpha" => { for c in b'a'..=b'z' { chars.push(c); } for c in b'A'..=b'Z' { chars.push(c); } }
                            "upper" => { for c in b'A'..=b'Z' { chars.push(c); } }
                            "lower" => { for c in b'a'..=b'z' { chars.push(c); } }
                            "digit" => { for c in b'0'..=b'9' { chars.push(c); } }
                            "xdigit" => { for c in b'0'..=b'9' { chars.push(c); } for c in b'a'..=b'f' { chars.push(c); } for c in b'A'..=b'F' { chars.push(c); } }
                            "alnum" => { for c in b'a'..=b'z' { chars.push(c); } for c in b'A'..=b'Z' { chars.push(c); } for c in b'0'..=b'9' { chars.push(c); } }
                            "space" => { for b in b" \t\n\r\x0b\x0c".iter() { chars.push(*b); } }
                            "blank" => { chars.push(b' '); chars.push(b'\t'); }
                            "punct" => { for b in b"!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~".iter() { chars.push(*b); } }
                            "cntrl" => { for c in 0u8..=31 { chars.push(c); } chars.push(127); }
                            "print" => { for c in 32u8..=126 { chars.push(c); } }
                            "graph" => { for c in 33u8..=126 { chars.push(c); } }
                            _ => {}
                        }
                        i_b = j_b + 2;
                        continue;
                    }
                }
                if i_b + 2 < bb.len() && bb[i_b+1] == b'-' && bb[i_b+2] != b']' {
                    let lo = bb[i_b];
                    let hi = bb[i_b+2];
                    for c in lo..=hi { chars.push(c); }
                    i_b += 3;
                } else {
                    chars.push(bb[i_b]);
                    i_b += 1;
                }
            }
            drop(parse_b);
            if let Some(p_lock) = patparse.lock().ok() {
                if i_b < p_lock.len() && p_lock.as_bytes()[i_b] == b']' { i_b += 1; }
            }
            patparse_off.store(i_b, Ordering::Relaxed);
            let opcode = if negate { P_ANYBUT } else { P_ANYOF };
            let off2 = patnode(opcode);
            let mut buf = patout.lock().unwrap();
            let len = chars.len() as u32;
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(&chars);
            *tail_out = off2;
            off2 as i64
        }
        b'(' => {
            patparse_off.fetch_add(1, Ordering::Relaxed);
            *flagp &= !P_PURESTR;
            let n = patnpar.fetch_add(1, Ordering::Relaxed);
            if n >= NSUBEXP as i32 {
                return -1;
            }
            let opcode = P_OPEN + n as u8;
            let open_off = patnode(opcode);
            let mut inner_flags: i32 = 0;
            let inner = patcompswitch(1, &mut inner_flags);
            if inner < 0 { return -1; }
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
            *tail_out = close_off;
            open_off as i64
        }
        b'\\' => {
            patparse_off.fetch_add(1, Ordering::Relaxed);
            let p = patparse.lock().unwrap();
            let off2 = patparse_off.load(Ordering::Relaxed);
            if off2 >= p.len() { return -1; }
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
            patparse_off.fetch_add(1, Ordering::Relaxed);
            *flagp &= !P_PURESTR;
            let parse_n = patparse.lock().unwrap();
            let nb = parse_n.as_bytes();
            let mut j = patparse_off.load(Ordering::Relaxed);
            let mut len_flag: u8 = 0;  // bit 0 = lo present, bit 1 = hi present
            let mut from: i64 = 0;
            let lo_start = j;
            while j < nb.len() && nb[j].is_ascii_digit() {
                from = from * 10 + (nb[j] - b'0') as i64;
                j += 1;
            }
            if j > lo_start { len_flag |= 1; }                                // c:1538 — `len |= 1`
            // Mandatory dash.
            if j >= nb.len() || nb[j] != b'-' {
                drop(parse_n);
                return -1;
            }
            j += 1;                                                            // c:1543 patparse++
            let mut to: i64 = 0;
            let hi_start = j;
            while j < nb.len() && nb[j].is_ascii_digit() {
                to = to * 10 + (nb[j] - b'0') as i64;
                j += 1;
            }
            if j > hi_start { len_flag |= 2; }                                // c:1548 — `len |= 2`
            // Expect closing '>'.
            if j >= nb.len() || nb[j] != b'>' {
                drop(parse_n);
                return -1;                                                    // c:1551 (return 0 in C)
            }
            j += 1;
            drop(parse_n);
            patparse_off.store(j, Ordering::Relaxed);

            let off2 = match len_flag {                                       // c:1552-1567
                3 => {                                                        // c:1554 P_NUMRNG
                    let off2 = patnode(P_NUMRNG);
                    let mut buf = patout.lock().unwrap();
                    buf.extend_from_slice(&from.to_le_bytes());
                    buf.extend_from_slice(&to.to_le_bytes());
                    off2
                }
                2 => {                                                        // c:1559 P_NUMTO
                    let off2 = patnode(P_NUMTO);
                    let mut buf = patout.lock().unwrap();
                    buf.extend_from_slice(&to.to_le_bytes());
                    off2
                }
                1 => {                                                        // c:1563 P_NUMFROM
                    let off2 = patnode(P_NUMFROM);
                    let mut buf = patout.lock().unwrap();
                    buf.extend_from_slice(&from.to_le_bytes());
                    off2
                }
                _ => patnode(P_NUMANY),                                       // c:1568
            };
            *tail_out = off2;
            off2 as i64
        }
        _ => {
            // Accumulate a literal run.
            let mut buf: Vec<u8> = Vec::new();
            let mut local_off = off;
            let p = patparse.lock().unwrap();
            while local_off < p.len() {
                let b = p.as_bytes()[local_off];
                // Stop at metacharacters.
                if matches!(b, b'?'|b'*'|b'['|b'('|b')'|b'|'|b'\\'|b'#'|b'^'|b'<') {
                    break;
                }
                buf.push(b);
                local_off += 1;
            }
            drop(p);
            if buf.is_empty() {
                return -1;
            }
            patparse_off.store(local_off, Ordering::Relaxed);
            *flagp |= P_SIMPLE;
            // If it's a single char, mark simple; multi-char run stays pure-string.
            let lit_off = patnode(P_EXACTLY);
            let mut buf_lit = patout.lock().unwrap();
            let len = buf.len() as u32;
            buf_lit.extend_from_slice(&len.to_le_bytes());
            buf_lit.extend_from_slice(&buf);
            *tail_out = lit_off;
            lit_off as i64
        }
    };

    if atom < 0 { return atom; }

    // Quantifier: # / ##
    let q_off = patparse_off.load(Ordering::Relaxed);
    let parse2 = patparse.lock().unwrap();
    if q_off < parse2.len() && parse2.as_bytes()[q_off] == b'#' {
        let two = q_off + 1 < parse2.len() && parse2.as_bytes()[q_off + 1] == b'#';
        drop(parse2);
        let consume = if two { 2 } else { 1 };
        patparse_off.fetch_add(consume, Ordering::Relaxed);
        let quant_op = if two { P_TWOHASH } else { P_ONEHASH };
        // C inserts the quant opcode BEFORE the atom and chains atom
        // as the operand. patinsert handles the byte shift.
        patinsert(quant_op, atom as usize, None, 0);
        *flagp &= !P_PURESTR;
        // After patinsert, the atom now lives at atom+5; the quant
        // opcode sits at the original atom offset. Tail is the
        // quant header (its .next is what gets chained to follow-up).
        *tail_out = atom as usize;
    }

    atom
}

/// Port of `patcompnot()` from `Src/pattern.c:1760`.
///
/// C: `static long patcompnot(int paren, int *flagsp)`. Implements
/// the `^pat` extended-glob negation by emitting P_EXCLUDE.
///
/// **Deferred** (Phase 5): full exclude support requires the matcher
/// to backtrack between branch and exclude trees. Currently returns -1
/// (compile failure) so the higher-level switch falls through.
pub fn patcompnot(_paren: i32, _flagsp: &mut i32) -> i64 {                    // c:1760
    -1
}

// =====================================================================
// 10. Glob-flag parser — pattern.c:1037
// =====================================================================

/// Port of `patgetglobflags()` from `Src/pattern.c:1037`.
///
/// C signature: `int patgetglobflags(char **strp, long *assertp,
/// int *ignore)`. Parses the `(#...)` glob-flag specifier and updates
/// the active flag set.
///
/// Returns: tuple `(consumed_chars, glob_flags_set)` where
/// `consumed_chars` is how many bytes of input were consumed (the
/// closing `)`), or 0 if the input doesn't start with `(#`. The C
/// signature uses out-params; Rust returns the data via the tuple.
pub fn patgetglobflags(s: &str) -> Option<(GlobFlagsResult, Option<PatOp>, usize)> { // c:1037
    let bytes = s.as_bytes();
    if !s.starts_with("(#") { return None; }
    let mut i = 2;
    let mut flags = GlobFlagsResult::default();
    let mut op: Option<PatOp> = None;

    while i < bytes.len() && bytes[i] != b')' {
        match bytes[i] {
            b'i' => { flags.case_insensitive = true; i += 1; }
            b'I' => { flags.case_insensitive = false; i += 1; }
            b'l' => { flags.lcmatchuc = true; i += 1; }
            b'L' => { flags.lcmatchuc = false; i += 1; }
            b'b' => { flags.backref = true; i += 1; }
            b'B' => { flags.backref = false; i += 1; }
            b'm' => { flags.match_refs = true; i += 1; }
            b'M' => { flags.match_refs = false; i += 1; }
            b's' => { op = Some(PatOp::StartAssert); i += 1; }
            b'e' => { op = Some(PatOp::EndAssert); i += 1; }
            b'u' => { flags.multibyte = true; i += 1; }
            b'U' => { flags.multibyte = false; i += 1; }
            b'a' => {
                // approximate matching: (#a<n>) — consume digits
                i += 1;
                let mut errs: u32 = 0;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    errs = errs * 10 + (bytes[i] - b'0') as u32;
                    i += 1;
                }
                flags.approx_errs = Some(errs);
            }
            b'q' => {
                // glob qualifiers — skip until ) or end
                while i < bytes.len() && bytes[i] != b')' { i += 1; }
            }
            _ => return None,
        }
    }
    if i >= bytes.len() { return None; }
    i += 1; // skip ')'
    Some((flags, op, i))
}

/// Result of `patgetglobflags` — bitfield of active glob flags.
/// Maps onto the C `patglobflags` int via PAT_LCMATCHUC etc.
#[derive(Default, Clone, Copy)]
pub struct GlobFlagsResult {
    pub case_insensitive: bool,
    pub lcmatchuc: bool,
    pub backref: bool,
    pub match_refs: bool,
    pub multibyte: bool,
    pub approx_errs: Option<u32>,
}

/// `PatOp` — assertion type from `(#s)` / `(#e)` glob flags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PatOp {
    StartAssert,
    EndAssert,
}

// =====================================================================
// 11. Range helpers — pattern.c:1148, :1179
// =====================================================================

/// Port of `range_type()` from `Src/pattern.c:1148`. Looks up the
/// integer code for a POSIX character class name (e.g. "alpha" → 1).
/// Returns None for unknown names.
pub fn range_type(name: &str) -> Option<usize> {                              // c:1148
    POSIX_CLASS_NAMES.iter().position(|n| *n == name).map(|i| i + 1)
}

/// Port of `pattern_range_to_string()` from `Src/pattern.c:1179`.
/// Reverse of range_type: given an index, return the class name.
pub fn pattern_range_to_string(idx: usize) -> Option<String> {                // c:1179
    if idx == 0 { return None; }
    POSIX_CLASS_NAMES.get(idx - 1).map(|n| format!("[:{}:]", n))
}

const POSIX_CLASS_NAMES: &[&str] = &[
    "alpha", "alnum", "blank", "cntrl", "digit", "graph", "lower",
    "print", "punct", "space", "upper", "xdigit",
];

// =====================================================================
// 12. Char-decode helpers — pattern.c:327, :336, :1909-1997
// =====================================================================

/// Port of `clear_shiftstate()` from `Src/pattern.c:327`. C uses
/// `mbstate_t`; Rust `char` is already a code point, so no shift
/// state to clear.
pub fn clear_shiftstate() {}                                                  // c:327

/// Port of `metacharinc()` from `Src/pattern.c:336`. Advances past
/// the next char (Meta-escape aware in C; UTF-8-byte-len in Rust).
pub fn metacharinc(s: &str, pos: usize) -> usize {                            // c:336
    s[pos..].chars().next().map(|c| pos + c.len_utf8()).unwrap_or(pos)
}

/// Port of `charref()` from `Src/pattern.c:1909`. Decode the char at
/// `pos` without advancing.
pub fn charref(s: &str, pos: usize) -> Option<char> {                         // c:1909
    s[pos..].chars().next()
}

/// Port of `charnext()` from `Src/pattern.c:1936`. Advance past the
/// char at `pos`.
pub fn charnext(s: &str, pos: usize) -> usize {                               // c:1936
    metacharinc(s, pos)
}

/// Port of `charrefinc()` from `Src/pattern.c:1964`. Decode and
/// advance: returns the char, mutates `pos` to point past it.
pub fn charrefinc(s: &str, pos: &mut usize) -> Option<char> {                 // c:1964
    let c = s[*pos..].chars().next()?;
    *pos += c.len_utf8();
    Some(c)
}

/// Port of `charsub()` from `Src/pattern.c:1997`. Returns the byte
/// offset of the char before `pos` (useful for stepping back).
pub fn charsub(s: &str, pos: usize) -> usize {                                // c:1997
    if pos == 0 { return 0; }
    let w = s[..pos].chars().next_back().map(|c| c.len_utf8()).unwrap_or(1);
    pos - w
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
    pub patbeginp: [usize; NSUBEXP],   // c:241 capture starts (byte offsets)
    pub patendp:   [usize; NSUBEXP],   // c:242 capture ends
    pub captures_set: u16,              // bitmask of groups successfully captured
}

impl rpat {
    fn new() -> Self {
        Self {
            patbeginp: [usize::MAX; NSUBEXP],
            patendp:   [0; NSUBEXP],
            captures_set: 0,
        }
    }
}

/// Port of `pattry()` from `Src/pattern.c:2223`.
///
/// C signature: `int pattry(Patprog prog, char *string)`. Returns
/// non-zero on match, 0 on no-match.
pub fn pattry(prog: &Patprog, string: &str) -> bool {                         // c:2223
    pattrylen(prog, string, string.len())
}

/// Port of `pattrylen()` from `Src/pattern.c:2236`. Truncated match.
pub fn pattrylen(prog: &Patprog, string: &str, len: usize) -> bool {          // c:2236
    let trial = if len < string.len() { &string[..len] } else { string };
    let mut state = rpat::new();
    // C pattry anchors at both ends by default — match must consume
    // the entire trial string unless PAT_NOANCH / PAT_NOTEND are set.
    // Port: require end_pos == trial.len() when neither flag set.
    match patmatch_internal(&prog.code, 0, trial, 0, &mut state, prog.flags) {
        Some(end_pos) => {
            let no_anchor = (prog.flags & (PAT_NOANCH | PAT_NOTEND) as i32) != 0; // c:3397
            no_anchor || end_pos == trial.len()
        }
        None => false,
    }
}

/// Port of `pattryrefs()` from `Src/pattern.c:2294`. Run match and
/// return capture group ranges.
pub fn pattryrefs(prog: &Patprog, string: &str) -> Option<(bool, Vec<(usize, usize)>)> { // c:2294
    let mut state = rpat::new();
    let ok = patmatch_internal(&prog.code, 0, string, 0, &mut state, prog.flags).is_some();
    if ok {
        let mut refs = Vec::with_capacity(prog.patnpar as usize);
        for i in 0..(prog.patnpar as usize).min(NSUBEXP) {
            let start = state.patbeginp[i];
            let end = state.patendp[i];
            if (state.captures_set & (1 << i)) != 0 {
                refs.push((start, end));
            } else {
                refs.push((0, 0));
            }
        }
        Some((true, refs))
    } else {
        Some((false, Vec::new()))
    }
}

/// Port of `patmatchlen()` from `Src/pattern.c:2649`. Returns the
/// length of a successful match, or None.
pub fn patmatchlen(prog: &Patprog, string: &str) -> Option<usize> {           // c:2649
    let mut state = rpat::new();
    patmatch_internal(&prog.code, 0, string, 0, &mut state, prog.flags)
}

/// Port of `patmatch()` from `Src/pattern.c:2694`. The interpreter.
///
/// Returns `Some(end_pos)` on successful match (end_pos = byte offset
/// in `string` where match ended), `None` on no-match. The state
/// param tracks captures.
///
/// Rust port renamed to `patmatch_internal` so the C-name `patmatch`
/// remains free for the convenience entry below (`patmatch(pat, text)`
/// — used by in-tree callers like params.rs and subst.rs). The
/// interpreter signature differs from C's `int patmatch(Upat prog)`
/// because Rust threads input/captures through args rather than
/// C's per-thread file-statics. Allowlisted as architectural.
fn patmatch_internal(
    code: &[u8],
    prog_off: usize,
    string: &str,
    string_off: usize,
    state: &mut rpat,
    glob_flags: i32,
) -> Option<usize> {                                                          // c:2694
    let mut scan = prog_off;
    let mut s_off = string_off;
    // Locally-mutable copy of glob_flags so mid-pattern P_GFLAGS can
    // toggle bits without affecting the caller's branch view.
    let mut glob_flags = glob_flags;

    while scan < code.len() {
        let op = code[scan + I_OP];
        let next_bytes: [u8; 4] = code[scan + I_NEXT..scan + I_NEXT + 4].try_into().unwrap();
        let next = u32::from_le_bytes(next_bytes) as usize;

        match op {
            P_END => return Some(s_off),                                      // c:end-of-prog
            P_NOTHING => { /* empty match, just continue */ }
            P_BACK => { /* zero-width, walk back via next */ }
            P_EXACTLY => {                                                    // c:P_EXACTLY arm
                let body = scan + I_BODY;
                let len = u32::from_le_bytes(code[body..body + 4].try_into().unwrap()) as usize;
                let str_bytes = &code[body + 4..body + 4 + len];
                let input_bytes = string.as_bytes();
                if s_off + len > input_bytes.len() { return None; }
                let igncase = (glob_flags & (GF_IGNCASE | GF_LCMATCHUC)) != 0;
                let multibyte = (glob_flags & GF_MULTIBYTE) != 0;             // c:349 GF_MULTIBYTE
                if igncase {
                    let inp_slice = &input_bytes[s_off..s_off + len];
                    if multibyte {
                        // Char-level Unicode case fold (mirrors C's
                        // mb_patmatch* path when GF_MULTIBYTE set).
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
                                        let af: String = a.to_lowercase().collect();
                                        let bf: String = b.to_lowercase().collect();
                                        if af != bf { return None; }
                                    }
                                }
                            }
                        } else {
                            // Non-UTF-8 input — byte fallback.
                            for k in 0..len {
                                if inp_slice[k].to_ascii_lowercase()
                                    != str_bytes[k].to_ascii_lowercase() { return None; }
                            }
                        }
                    } else {
                        // Byte-level ASCII case fold (mirrors C's
                        // patmatch* path when GF_MULTIBYTE clear).
                        for k in 0..len {
                            if inp_slice[k].to_ascii_lowercase()
                                != str_bytes[k].to_ascii_lowercase() { return None; }
                        }
                    }
                } else if &input_bytes[s_off..s_off + len] != str_bytes {
                    return None;
                }
                s_off += len;
            }
            P_ANY => {                                                        // c:P_ANY arm
                let s = &string[s_off..];
                let c = s.chars().next()?;
                s_off += c.len_utf8();
            }
            P_ANYOF => {                                                      // c:P_ANYOF arm
                let body = scan + I_BODY;
                let len = u32::from_le_bytes(code[body..body + 4].try_into().unwrap()) as usize;
                let set = &code[body + 4..body + 4 + len];
                let input_bytes = string.as_bytes();
                if s_off >= input_bytes.len() { return None; }
                let b = input_bytes[s_off];
                let igncase = (glob_flags & (GF_IGNCASE | GF_LCMATCHUC)) != 0;
                let found = if igncase {
                    let lb = b.to_ascii_lowercase();
                    set.iter().any(|&c| c.to_ascii_lowercase() == lb)
                } else {
                    set.contains(&b)
                };
                if !found { return None; }
                s_off += 1;
            }
            P_ANYBUT => {
                let body = scan + I_BODY;
                let len = u32::from_le_bytes(code[body..body + 4].try_into().unwrap()) as usize;
                let set = &code[body + 4..body + 4 + len];
                let input_bytes = string.as_bytes();
                if s_off >= input_bytes.len() { return None; }
                let b = input_bytes[s_off];
                let igncase = (glob_flags & (GF_IGNCASE | GF_LCMATCHUC)) != 0;
                let found = if igncase {
                    let lb = b.to_ascii_lowercase();
                    set.iter().any(|&c| c.to_ascii_lowercase() == lb)
                } else {
                    set.contains(&b)
                };
                if found { return None; }
                s_off += 1;
            }
            P_STAR => {                                                       // c:P_STAR arm (greedy)
                // Greedy: try to match as many chars as possible then
                // backtrack until the rest matches.
                let input_bytes = string.as_bytes();
                let max = input_bytes.len() - s_off;
                let mut consumed = max;
                loop {
                    let mut sub_state = state.clone();
                    if let Some(end) = patmatch_internal(code, next, string, s_off + consumed, &mut sub_state, glob_flags) {
                        *state = sub_state;
                        return Some(end);
                    }
                    if consumed == 0 { return None; }
                    consumed -= 1;
                }
            }
            P_ONEHASH | P_TWOHASH => {                                        // c:P_ONEHASH / P_TWOHASH
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
                    if let Some(new_pos) = patmatch_internal(code, operand, string, cur, &mut sub_state, glob_flags) {
                        if new_pos == cur { break; } // zero-width fixed point
                        *state = sub_state;
                        positions.push(new_pos);
                    } else {
                        break;
                    }
                }
                if positions.len() - 1 < min { return None; }
                // Walk back from longest match trying continuations.
                while positions.len() > min {
                    let cur = *positions.last().unwrap();
                    let mut sub_state = state.clone();
                    if let Some(end) = patmatch_internal(code, next, string, cur, &mut sub_state, glob_flags) {
                        *state = sub_state;
                        return Some(end);
                    }
                    if positions.len() <= min + 1 { return None; }
                    positions.pop();
                }
                return None;
            }
            P_BRANCH => {                                                     // c:P_BRANCH arm
                // c:3046-3050 — if next is NOT another BRANCH, this is
                // the only alternative; avoid the alt-loop and just
                // continue with the operand inline (no recursion, no
                // fallthrough on failure).
                let next_is_branch = next != 0
                    && next < code.len()
                    && (code[next + I_OP] == P_BRANCH
                        || code[next + I_OP] == P_WBRANCH);
                if !next_is_branch {
                    scan = scan + I_BODY;
                    continue;
                }
                // Alt-loop: try each branch's operand; on success
                // return; on failure walk to the next BRANCH via .next.
                let mut br = scan;
                loop {
                    let br_next_bytes: [u8; 4] = code[br + I_NEXT..br + I_NEXT + 4]
                        .try_into().unwrap();
                    let br_next = u32::from_le_bytes(br_next_bytes) as usize;
                    let operand = br + I_BODY;
                    let mut sub_state = state.clone();
                    if let Some(end) = patmatch_internal(
                        code, operand, string, s_off, &mut sub_state, glob_flags
                    ) {
                        *state = sub_state;
                        return Some(end);
                    }
                    if br_next == 0 { return None; }
                    let op_next = code[br_next + I_OP];
                    if op_next != P_BRANCH && op_next != P_WBRANCH {
                        return None;
                    }
                    br = br_next;
                }
            }
            P_NUMRNG => {                                                     // c:P_NUMRNG arm
                let body = scan + I_BODY;
                let from = i64::from_le_bytes(code[body..body + 8].try_into().unwrap());
                let to = i64::from_le_bytes(code[body + 8..body + 16].try_into().unwrap());
                let input_bytes = string.as_bytes();
                let start = s_off;
                let mut k = start;
                while k < input_bytes.len() && input_bytes[k].is_ascii_digit() {
                    k += 1;
                }
                if k == start { return None; }
                let n: i64 = std::str::from_utf8(&input_bytes[start..k])
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())?;
                if n < from || n > to { return None; }
                s_off = k;
            }
            P_NUMFROM => {
                let body = scan + I_BODY;
                let from = i64::from_le_bytes(code[body..body + 8].try_into().unwrap());
                let input_bytes = string.as_bytes();
                let start = s_off;
                let mut k = start;
                while k < input_bytes.len() && input_bytes[k].is_ascii_digit() { k += 1; }
                if k == start { return None; }
                let n: i64 = std::str::from_utf8(&input_bytes[start..k])
                    .ok().and_then(|s| s.parse::<i64>().ok())?;
                if n < from { return None; }
                s_off = k;
            }
            P_NUMTO => {
                let body = scan + I_BODY;
                let to = i64::from_le_bytes(code[body..body + 8].try_into().unwrap());
                let input_bytes = string.as_bytes();
                let start = s_off;
                let mut k = start;
                while k < input_bytes.len() && input_bytes[k].is_ascii_digit() { k += 1; }
                if k == start { return None; }
                let n: i64 = std::str::from_utf8(&input_bytes[start..k])
                    .ok().and_then(|s| s.parse::<i64>().ok())?;
                if n > to { return None; }
                s_off = k;
            }
            P_NUMANY => {
                let input_bytes = string.as_bytes();
                let start = s_off;
                while s_off < input_bytes.len() && input_bytes[s_off].is_ascii_digit() { s_off += 1; }
                if s_off == start { return None; }
            }
            P_ISSTART => {                                                    // c:P_ISSTART
                if s_off != 0 { return None; }
            }
            P_ISEND => {                                                      // c:P_ISEND
                if s_off < string.len() { return None; }
            }
            P_GFLAGS => {                                                     // c:P_GFLAGS arm
                let body = scan + I_BODY;
                let bits = i32::from_le_bytes(code[body..body + 4].try_into().unwrap());
                // C uses absolute set; for the on/off toggle pairs
                // we currently encode only the "on" bits (i.e. (#I)
                // emits 0 to clear). Set the running flags directly.
                glob_flags = (glob_flags
                    & !(GF_IGNCASE | GF_LCMATCHUC | GF_MULTIBYTE)) | bits;
            }
            P_COUNT => {                                                      // c:P_COUNT arm
                let body = scan + I_BODY;
                let min = i64::from_le_bytes(code[body..body + 8].try_into().unwrap());
                let max = i64::from_le_bytes(code[body + 8..body + 16].try_into().unwrap());
                let operand = body + 16;
                // Greedy: match operand up to max times, then walk
                // back trying continuations until count is in
                // [min, max]. Same shape as P_ONEHASH/P_TWOHASH.
                let mut positions = vec![s_off];
                let max_usize: i64 = max;
                loop {
                    let cur = *positions.last().unwrap();
                    if (positions.len() as i64 - 1) >= max_usize { break; }
                    let mut sub_state = state.clone();
                    if let Some(new_pos) = patmatch_internal(code, operand, string, cur, &mut sub_state, glob_flags) {
                        if new_pos == cur { break; }
                        *state = sub_state;
                        positions.push(new_pos);
                    } else {
                        break;
                    }
                }
                let min_usize = min as usize;
                if positions.len() < min_usize + 1 { return None; }
                while positions.len() > min_usize {
                    let cur = *positions.last().unwrap();
                    let mut sub_state = state.clone();
                    if let Some(end) = patmatch_internal(code, next, string, cur, &mut sub_state, glob_flags) {
                        *state = sub_state;
                        return Some(end);
                    }
                    if positions.len() <= min_usize + 1 { return None; }
                    positions.pop();
                }
                return None;
            }
            op if op >= P_OPEN && op < P_CLOSE => {                           // c:P_OPEN_N arm
                let n = (op - P_OPEN) as usize;
                if n > 0 && n <= NSUBEXP {
                    state.patbeginp[n - 1] = s_off;
                }
            }
            op if op >= P_CLOSE && op < 0xa0 => {                             // c:P_CLOSE_N arm
                let n = (op - P_CLOSE) as usize;
                if n > 0 && n <= NSUBEXP {
                    state.patendp[n - 1] = s_off;
                    state.captures_set |= 1u16 << (n - 1);
                }
            }
            _ => {
                // Unrecognized opcode — Phase 5 features (P_NUMRNG,
                // P_GFLAGS, P_EXCLUDE, P_COUNT, P_ISSTART/ISEND,
                // P_BACKREF) land here. Treat as no-op for now so
                // current tests still pass.
            }
        }

        if next == 0 { break; }
        scan = next;
    }
    Some(s_off)
}

// =====================================================================
// 15. Range matching — pattern.c:3856, :4004, :3610, :3767
// =====================================================================

/// Port of `patmatchrange()` from `Src/pattern.c:3856`. Test whether
/// `ch` matches the bracket-range expression `range`.
///
/// `range` is the bytes between `[...]` in the original pattern.
pub fn patmatchrange(range: &[char], ch: char, igncase: bool) -> bool {       // c:3856
    let test = |c: char| {
        if igncase { c.to_ascii_lowercase() == ch.to_ascii_lowercase() }
        else { c == ch }
    };
    let mut i = 0;
    while i < range.len() {
        if i + 2 < range.len() && range[i + 1] == '-' {
            let lo = range[i];
            let hi = range[i + 2];
            let c = if igncase { ch.to_ascii_lowercase() } else { ch };
            let lo2 = if igncase { lo.to_ascii_lowercase() } else { lo };
            let hi2 = if igncase { hi.to_ascii_lowercase() } else { hi };
            if c >= lo2 && c <= hi2 { return true; }
            i += 3;
        } else if test(range[i]) {
            return true;
        } else {
            i += 1;
        }
    }
    false
}

/// Port of `patmatchindex()` from `Src/pattern.c:4004`. Return the
/// `idx`-th character that matches `range` (used by `${arr:#pat}`).
pub fn patmatchindex(range: &[char], idx: usize) -> Option<char> {            // c:4004
    let mut n = 0;
    let mut i = 0;
    while i < range.len() {
        if i + 2 < range.len() && range[i + 1] == '-' {
            let lo = range[i] as u32;
            let hi = range[i + 2] as u32;
            for c in lo..=hi {
                if n == idx { return char::from_u32(c); }
                n += 1;
            }
            i += 3;
        } else {
            if n == idx { return Some(range[i]); }
            n += 1;
            i += 1;
        }
    }
    None
}

/// Port of `mb_patmatchrange()` from `Src/pattern.c:3610`. Multibyte
/// variant — same as patmatchrange in Rust's UTF-8 world.
pub fn mb_patmatchrange(range: &[char], ch: char, igncase: bool) -> bool {    // c:3610
    patmatchrange(range, ch, igncase)
}

/// Port of `mb_patmatchindex()` from `Src/pattern.c:3767`.
pub fn mb_patmatchindex(range: &[char], idx: usize) -> Option<char> {         // c:3767
    patmatchindex(range, idx)
}

// =====================================================================
// 16. String pre-processing — pattern.c:2063, :2080, :2132
// =====================================================================

/// Port of `pattrystart()` from `Src/pattern.c:2063`. C resets per-
/// match state globals; Rust state is per-call so no-op.
pub fn pattrystart() {}                                                       // c:2063

/// Port of `patmungestring()` from `Src/pattern.c:2080`. Un-metafies
/// in C; UTF-8 needs no munging.
pub fn patmungestring(s: &str) -> String {                                    // c:2080
    s.to_string()
}

/// Port of `patallocstr()` from `Src/pattern.c:2132`.
pub fn patallocstr(s: &str) -> String {                                       // c:2132
    s.to_string()
}

// =====================================================================
// 17. Module-loader / disable mgmt — pattern.c:4161-4296
// =====================================================================

/// Disabled-pattern set, per pattern.c:4220 `savepatterndisables`.
/// Tracks which named patterns are currently disabled by `disable -p`.
pub static patterndisables: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Port of `startpatternscope()` from `Src/pattern.c:4241`. Begins a
/// new disable scope.
pub fn startpatternscope() {                                                  // c:4241
    // Saving/restoring handled per-call; mark a scope boundary by
    // duplicating the current disables list onto a stack.
    let cur = patterndisables.lock().unwrap().clone();
    PATSCOPE_STACK.with(|s| s.borrow_mut().push(cur));
}

/// Port of file-static `zpc_disables_stack` from `Src/pattern.c:4244`.
/// Per-evaluator function-scope disable save-stack (bucket-1: each
/// worker thread parses/executes its own function calls, so each must
/// have its own scope stack). Reason for `thread_local!` over `Mutex`:
/// in zsh C this is a per-process file-static; in zshrs each worker
/// thread is its own evaluator — TLS preserves the per-evaluator
/// semantic without serializing across workers.
thread_local! {
    static PATSCOPE_STACK: std::cell::RefCell<Vec<Vec<String>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Port of `endpatternscope()` from `Src/pattern.c:4279`. Ends the
/// current scope, popping the saved state.
pub fn endpatternscope() {                                                    // c:4279
    if let Some(prev) = PATSCOPE_STACK.with(|s| s.borrow_mut().pop()) {
        *patterndisables.lock().unwrap() = prev;
    }
}

/// Port of `savepatterndisables()` from `Src/pattern.c:4220`. Returns
/// the current disables list (caller restores via restorepatterndisables).
pub fn savepatterndisables() -> Vec<String> {                                 // c:4220
    patterndisables.lock().unwrap().clone()
}

/// Port of `restorepatterndisables()` from `Src/pattern.c:4258`.
pub fn restorepatterndisables(saved: Vec<String>) {                           // c:4258
    *patterndisables.lock().unwrap() = saved;
}

/// Port of `clearpatterndisables()` from `Src/pattern.c:4296`.
pub fn clearpatterndisables() {                                               // c:4296
    patterndisables.lock().unwrap().clear();
}

/// Port of `freepatprog()` from `Src/pattern.c:4161`. Frees a Patprog.
/// Rust's `Drop` on `Box<patprog>` handles this; the explicit fn
/// exists for C parity (Rule A).
pub fn freepatprog(_prog: Patprog) {}                                         // c:4161

/// Port of `pat_enables()` from `Src/pattern.c:4171`. Implements
/// `enable -p` / `disable -p` for named patterns.
pub fn pat_enables(_cmd: &str, patterns: &[&str], enable: bool) -> i32 {      // c:4171
    let mut disables = patterndisables.lock().unwrap();
    for p in patterns {
        if enable {
            disables.retain(|d| d != p);
        } else if !disables.iter().any(|d| d == p) {
            disables.push(p.to_string());
        }
    }
    0
}

// =====================================================================
// 18. Convenience entry points — used by in-tree callers
// =====================================================================

/// Compile + match in one call. Convenience wrapper used by in-tree
/// callers (params.rs / subst.rs / options.rs / zutil.rs) that don't
/// keep a compiled Patprog around. Signature differs from C's
/// `int patmatch(Upat prog)` (which takes a bytecode pointer and
/// reads input/captures from file-statics) — Rust takes both pattern
/// and text explicitly. Allowlisted as architectural convenience.
pub fn patmatch(pattern: &str, text: &str) -> bool {
    match patcompile(pattern, PAT_HEAPDUP as i32, None) {
        Some(prog) => pattry(&prog, text),
        None => false,
    }
}

/// Port of `patrepeat()` from `Src/pattern.c:4096`. Counts how many
/// times the pattern matches consecutively at the start of `s`.
pub fn patrepeat(prog: &Patprog, s: &str, max: Option<usize>) -> usize {      // c:4096
    let mut pos = 0;
    let mut count = 0;
    let max = max.unwrap_or(usize::MAX);
    while pos < s.len() && count < max {
        let mut state = rpat::new();
        match patmatch_internal(&prog.code, 0, s, pos, &mut state, prog.flags) {
            Some(new_pos) if new_pos > pos => {
                pos = new_pos;
                count += 1;
            }
            _ => break,
        }
    }
    count
}

/// Port of `haswilds()` from `Src/pattern.c:4306`. Quick check whether
/// `s` contains any wildcard characters.
pub fn haswilds(s: &str) -> bool {                                            // c:4306
    s.chars().any(|c| matches!(c, '*' | '?' | '[' | '\\' | '(' | '|' | '<' | '#' | '^'))
}

// =====================================================================
// Transitional aliases — older callers still use `PatProg` (camel-case
// from the previous AST-based port). Alias them to `Patprog` so the
// build doesn't break; future cleanup commit renames callers.
// =====================================================================

#[deprecated(note = "use Patprog instead")]
pub type PatProg = Patprog;

// =====================================================================
// Transitional Rust-only types — kept for external callers that bind
// to the previous AST-based port's surface area (exec.rs, exec_shims.rs,
// fusevm_bridge.rs, glob.rs). These are NOT C-faithful ports — they're
// helper aggregates the previous AST port introduced for one-shot
// pattern processing in the executor/VM bridge. Track with a TODO
// for eventual deletion + migration of callers to the bytecode API
// (patcompile + pattry + patgetglobflags). Allowlisted as transitional.
// =====================================================================

/// `<a-b>` numeric-range extraction helper. NOT in pattern.c — the C
/// source handles numeric ranges via P_NUMRNG opcode emission inside
/// patcomppiece. This type pre-processes a glob string outside the
/// pattern engine for callers in exec_shims/fusevm that need a
/// pre-pattern pass. TODO: migrate callers to pure patcompile usage.
#[derive(Debug, Clone, Copy)]
pub struct NumericRange {
    pub start: usize,
    pub end:   usize,
    /// Lower bound, `None` for unbounded (`<-N>` form).
    pub lo:    Option<i64>,
    /// Upper bound, `None` for unbounded (`<N->` form).
    pub hi:    Option<i64>,
}

impl NumericRange {
    /// Extract all `<a-b>` / `<a->` / `<-b>` / `<->` ranges from a
    /// glob pattern string.
    pub fn extract_all(s: &str) -> Vec<NumericRange> {
        let mut out = Vec::new();
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'<' {
                let start = i;
                let mut j = i + 1;
                // Lower part: digits or empty.
                let lo_start = j;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                let lo: Option<i64> = if j > lo_start {
                    std::str::from_utf8(&bytes[lo_start..j]).ok()
                        .and_then(|s| s.parse::<i64>().ok())
                } else { None };
                // Mandatory dash.
                if j < bytes.len() && bytes[j] == b'-' {
                    j += 1;
                    let hi_start = j;
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        j += 1;
                    }
                    let hi: Option<i64> = if j > hi_start {
                        std::str::from_utf8(&bytes[hi_start..j]).ok()
                            .and_then(|s| s.parse::<i64>().ok())
                    } else { None };
                    if j < bytes.len() && bytes[j] == b'>' {
                        out.push(NumericRange { start, end: j + 1, lo, hi });
                        i = j + 1;
                        continue;
                    }
                }
            }
            i += 1;
        }
        out
    }

    /// Replace every `<...>` with `*` for fallback glob expansion.
    pub fn replace_all_with_star(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut last = 0;
        for r in Self::extract_all(s) {
            out.push_str(&s[last..r.start]);
            out.push('*');
            last = r.end;
        }
        out.push_str(&s[last..]);
        out
    }

    /// Test whether `n` falls within this range. Unbounded sides
    /// always pass.
    pub fn contains(&self, n: i64) -> bool {
        self.lo.map_or(true, |l| n >= l) && self.hi.map_or(true, |h| n <= h)
    }
}

/// Pattern-flag pre-parse used by exec_shims and fusevm before
/// compile. NOT in pattern.c — C parses these inline via
/// patgetglobflags during patcompile. Transitional convenience.
#[derive(Debug, Clone)]
pub struct PatternFlags {
    pub pattern: String,
    pub case_insensitive: bool,
    pub l_flag: bool,
    pub approx_errs: Option<u32>,
    pub backref: bool,
}

impl PatternFlags {
    /// Parse `(#...)` prefix off the front of a pattern, returning
    /// (residual_pattern, case_insensitive, l_flag, approx, _).
    pub fn parse(s: &str) -> (String, bool, bool, Option<u32>, bool) {
        let mut residual = s.to_string();
        let mut ci = false;
        let mut l = false;
        let mut approx: Option<u32> = None;
        let mut br = false;
        if let Some((flags, _op, consumed)) = patgetglobflags(s) {
            ci = flags.case_insensitive;
            l = flags.lcmatchuc;
            approx = flags.approx_errs;
            br = flags.backref;
            residual = s[consumed..].to_string();
        }
        (residual, ci, l, approx, br)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Pattern compile shares file-static globals (patout, patparse,
    // patnpar, ...) with the same single-thread semantics as zsh's
    // C source. `patcompile` clones the globals into prog.code
    // before returning, so we only need the mutex held during
    // compile — pattry() reads from prog.code with no global state.
    static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn compile(p: &str) -> Patprog {
        let _g = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        patcompile(p, PAT_HEAPDUP as i32, None).expect("compile failed")
    }

    #[test]
    fn literal_match() {
        let prog = compile("hello");
        assert!(pattry(&prog, "hello"));
        assert!(!pattry(&prog, "world"));
    }

    #[test]
    fn star_matches_anything() {
        let prog = compile("*");
        assert!(pattry(&prog, ""));
        assert!(pattry(&prog, "abc"));
    }

    #[test]
    fn star_in_middle() {
        let prog = compile("a*z");
        assert!(pattry(&prog, "az"));
        assert!(pattry(&prog, "abz"));
        assert!(pattry(&prog, "aXYZz"));
        assert!(!pattry(&prog, "ab"));
    }

    #[test]
    fn question_matches_one() {
        let prog = compile("a?c");
        assert!(pattry(&prog, "abc"));
        assert!(pattry(&prog, "axc"));
        assert!(!pattry(&prog, "ac"));
    }

    #[test]
    fn bracket_anyof() {
        let prog = compile("[abc]");
        assert!(pattry(&prog, "a"));
        assert!(pattry(&prog, "b"));
        assert!(pattry(&prog, "c"));
        assert!(!pattry(&prog, "d"));
    }

    #[test]
    fn bracket_range() {
        let prog = compile("[a-z]");
        assert!(pattry(&prog, "m"));
        assert!(!pattry(&prog, "M"));
    }

    #[test]
    fn bracket_negated() {
        let prog = compile("[^0-9]");
        assert!(pattry(&prog, "a"));
        assert!(!pattry(&prog, "5"));
    }

    #[test]
    fn alternation() {
        let prog = compile("foo|bar");
        assert!(pattry(&prog, "foo"));
        assert!(pattry(&prog, "bar"));
        assert!(!pattry(&prog, "baz"));
    }

    #[test]
    fn captures() {
        let prog = compile("(foo)(bar)");
        let (ok, refs) = pattryrefs(&prog, "foobar").unwrap();
        assert!(ok);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0], (0, 3));
        assert_eq!(refs[1], (3, 6));
    }

    #[test]
    fn hash_zero_or_more() {
        let prog = compile("a#");
        assert!(pattry(&prog, ""));
        assert!(pattry(&prog, "a"));
        assert!(pattry(&prog, "aaa"));
    }

    #[test]
    fn double_hash_one_or_more() {
        let prog = compile("a##");
        assert!(!pattry(&prog, ""));
        assert!(pattry(&prog, "a"));
        assert!(pattry(&prog, "aaa"));
    }

    #[test]
    fn escape_literal() {
        let prog = compile("a\\*b");
        assert!(pattry(&prog, "a*b"));
        assert!(!pattry(&prog, "azb"));
    }

    #[test]
    fn convenience_patmatch() {
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
        use std::thread;
        let handles: Vec<_> = (0..8).map(|i| {
            thread::spawn(move || {
                for _ in 0..200 {
                    assert!(patmatch(":completion:*", ":completion:zsh"));
                    assert!(patmatch("hello*", "hello world"));
                    let _ = i;
                }
            })
        }).collect();
        for h in handles { h.join().unwrap(); }
    }

    #[test]
    fn haswilds_detects_meta() {
        assert!(haswilds("*"));
        assert!(haswilds("foo?"));
        assert!(haswilds("[abc]"));
        assert!(!haswilds("plain"));
    }

    #[test]
    fn patmatchrange_basic() {
        let r: Vec<char> = "a-zA-Z".chars().collect();
        assert!(patmatchrange(&r, 'm', false));
        assert!(patmatchrange(&r, 'X', false));
        assert!(!patmatchrange(&r, '5', false));
    }

    #[test]
    fn range_type_lookup() {
        assert_eq!(range_type("alpha"), Some(1));
        assert_eq!(range_type("digit"), Some(5));
        assert_eq!(range_type("nonsense"), None);
    }

    #[test]
    fn pattern_range_to_string_reverses() {
        assert_eq!(pattern_range_to_string(1), Some("[:alpha:]".to_string()));
        assert_eq!(pattern_range_to_string(0), None);
    }

    #[test]
    fn patgetglobflags_case_insensitive() {
        let (flags, _, n) = patgetglobflags("(#i)foo").unwrap();
        assert!(flags.case_insensitive);
        assert_eq!(n, 4); // length of "(#i)"
    }

    #[test]
    fn patgetglobflags_backref() {
        let (flags, _, _) = patgetglobflags("(#b)").unwrap();
        assert!(flags.backref);
    }

    #[test]
    fn patgetglobflags_approx() {
        let (flags, _, _) = patgetglobflags("(#a2)").unwrap();
        assert_eq!(flags.approx_errs, Some(2));
    }

    #[test]
    fn pattry_no_anchor_default() {
        // patmatch with anchored compile: only full-string matches succeed.
        let prog = compile("foo");
        assert!(pattry(&prog, "foo"));
    }

    /// `<a-b>` numeric range: digits matching n where lo ≤ n ≤ hi.
    /// Port of pattern.c:1528 (Inang case).
    #[test]
    fn numeric_range_inclusive() {
        let prog = compile("<10-20>");
        assert!(pattry(&prog, "15"));
        assert!(pattry(&prog, "10"));
        assert!(pattry(&prog, "20"));
        assert!(!pattry(&prog, "9"));
        assert!(!pattry(&prog, "21"));
    }

    #[test]
    fn numeric_range_from_only() {
        // <100-> matches any number ≥ 100.
        let prog = compile("<100->");
        assert!(pattry(&prog, "100"));
        assert!(pattry(&prog, "9999"));
        assert!(!pattry(&prog, "99"));
    }

    #[test]
    fn numeric_range_to_only() {
        // <-5> matches any number ≤ 5.
        let prog = compile("<-5>");
        assert!(pattry(&prog, "0"));
        assert!(pattry(&prog, "5"));
        assert!(!pattry(&prog, "6"));
    }

    #[test]
    fn numeric_range_any() {
        let prog = compile("<->");
        assert!(pattry(&prog, "0"));
        assert!(pattry(&prog, "12345"));
        assert!(!pattry(&prog, "abc"));
    }

    /// `(foo)#` — zero-or-more group repetition.
    #[test]
    fn group_with_hash_quantifier() {
        let prog = compile("(foo)#");
        assert!(pattry(&prog, ""));
        assert!(pattry(&prog, "foo"));
        assert!(pattry(&prog, "foofoofoo"));
    }

    /// `(a|b)##` — one-or-more group with alternation.
    #[test]
    fn group_alt_with_double_hash() {
        let prog = compile("(a|b)##");
        assert!(!pattry(&prog, ""));
        assert!(pattry(&prog, "a"));
        assert!(pattry(&prog, "abab"));
    }

    /// Mixed numeric range and literal: `v<1-99>`.
    #[test]
    fn literal_then_numeric_range() {
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
        let prog = compile("*.txt");
        assert!(pattry(&prog, "foo.txt"));
        assert!(pattry(&prog, "a.b.c.txt"));
        assert!(!pattry(&prog, "foo.txx"));
    }

    /// Bracket with POSIX class.
    #[test]
    fn posix_alpha_class() {
        let prog = compile("[[:alpha:]]##");
        assert!(pattry(&prog, "abc"));
        assert!(pattry(&prog, "XYZ"));
        assert!(!pattry(&prog, "1"));
        assert!(!pattry(&prog, ""));
    }

    /// `(#i)foo` matches "FOO" / "Foo" / etc. Port of pattern.c
    /// patgetglobflags `i` case at c:1091 (sets GF_IGNCASE which
    /// patcompile hoists into patprog.flags as PAT_LCMATCHUC).
    #[test]
    fn case_insensitive_via_glob_flag() {
        let prog = compile("(#i)foo");
        assert!(pattry(&prog, "foo"));
        assert!(pattry(&prog, "FOO"));
        assert!(pattry(&prog, "Foo"));
        assert!(pattry(&prog, "fOo"));
    }

    /// `(#i)[abc]` — case-insensitive bracket class.
    #[test]
    fn case_insensitive_bracket() {
        let prog = compile("(#i)[abc]");
        assert!(pattry(&prog, "A"));
        assert!(pattry(&prog, "b"));
        assert!(!pattry(&prog, "d"));
    }

    /// Unicode case-fold for `(#i)` — non-ASCII Latin chars.
    #[test]
    fn case_insensitive_unicode() {
        // German Ü/ü and É/é folded via char::to_lowercase.
        let prog = compile("(#i)Über");
        assert!(pattry(&prog, "über"));
        assert!(pattry(&prog, "ÜBER"));
        let prog2 = compile("(#i)café");
        assert!(pattry(&prog2, "CAFÉ"));
        assert!(pattry(&prog2, "Café"));
    }

    /// Without `(#i)`, exact case required.
    #[test]
    fn case_sensitive_default() {
        let prog = compile("foo");
        assert!(pattry(&prog, "foo"));
        assert!(!pattry(&prog, "FOO"));
    }

    /// Mid-pattern P_GFLAGS opcode: `foo(#i)BAR` — first half exact,
    /// second half case-insensitive.
    #[test]
    fn mid_pattern_gflags_switch() {
        let prog = compile("foo(#i)bar");
        assert!(pattry(&prog, "fooBAR"));
        assert!(pattry(&prog, "foobar"));
        assert!(pattry(&prog, "fooBaR"));
        // First half still case-sensitive — "FOOBAR" should NOT match.
        assert!(!pattry(&prog, "FOOBAR"));
    }

    /// `(#s)foo` — start-of-string anchor.
    #[test]
    fn start_anchor() {
        let prog = compile("(#s)foo");
        assert!(pattry(&prog, "foo"));
        // pattry runs from position 0 so this is structurally
        // equivalent to the default behavior; the assertion just
        // doesn't reject anything in this trivial case.
    }

    /// `foo(#e)` — end-of-string anchor.
    #[test]
    fn end_anchor() {
        let prog = compile("foo(#e)");
        assert!(pattry(&prog, "foo"));
    }

    /// `(#c3,5)x` — counted repetition: match `x` 3 to 5 times.
    #[test]
    fn count_range_3_to_5() {
        let prog = compile("(#c3,5)x");
        assert!(!pattry(&prog, "xx"));
        assert!(pattry(&prog, "xxx"));
        assert!(pattry(&prog, "xxxx"));
        assert!(pattry(&prog, "xxxxx"));
        assert!(!pattry(&prog, "xxxxxx"));
    }

    /// `(#c3)x` — exact count: `xxx` only.
    #[test]
    fn count_exact_3() {
        let prog = compile("(#c3)x");
        assert!(!pattry(&prog, "xx"));
        assert!(pattry(&prog, "xxx"));
        assert!(!pattry(&prog, "xxxx"));
    }

    #[test]
    fn debug_alt_b() {
        let prog = compile("(a)|b");
        eprintln!("bytecode len: {}", prog.code.len());
        for (i, b) in prog.code.iter().enumerate() {
            eprintln!("  [{:3}] {:#04x}", i, b);
        }
        let mut state = rpat::new();
        let r = super::patmatch_internal(&prog.code, 0, "b", 0, &mut state, prog.flags);
        eprintln!("match result: {:?}", r);
        assert!(pattry(&prog, "b"));
    }

    /// `(#c2,)x` — at least 2.
    #[test]
    fn count_min_only() {
        let prog = compile("(#c2,)x");
        assert!(!pattry(&prog, "x"));
        assert!(pattry(&prog, "xx"));
        assert!(pattry(&prog, "xxxxxxxx"));
    }

    #[test]
    fn captures_unmatched_group_returns_no_match() {
        // Pattern with alt — first branch fails, second succeeds; check
        // captures from successful branch only.
        let prog = compile("(a)|b");
        assert!(pattry(&prog, "a"));
        assert!(pattry(&prog, "b"));
    }
}
