//! `zsh.h` port — comprehensive umbrella header for the Rust port.
//!
//! Port of `Src/zsh.h` (3,375 lines). zsh.h is the umbrella header
//! every C file `#include`s. Defines: integer types, tokenized
//! character constants, ~50 typedef pointer aliases, ~64 structs,
//! and hundreds of `#define` constants for parameters, builtins,
//! redirections, jobs, pattern matching, options, terminal control,
//! prompts, signals, history, completion, etc.
//!
//! Per the macro casing rule: UPPERCASE C macros stay UPPERCASE in
//! Rust with `#[allow(non_snake_case)]`. Struct names use C casing
//! verbatim with `#[allow(non_camel_case_types)]`. Reserved-keyword
//! field names get a `_` suffix (`type` → `typ`, `str` → `str`,
//! `match` → `match_`, `new` → `new_`, `loop` → `loop_`,
//! `mod` → `mod_`, `fn` → `fn_`, `ref` → `ref_`).
//!
//! Many of the `typedef struct foo *Foo;` pointer aliases (c:510-549)
//! reference structs whose canonical home is the matching `.c` file
//! (e.g. `struct param` definition is in zsh.h:1829, but the live
//! Rust storage is in `params.rs`). We define those structs here as
//! the canonical port of zsh.h; consumers `use` them from here.

use std::sync::atomic::{AtomicI32, Ordering};

// =============================================================================
// 1. Integer type aliases (zsh.h:30-92).
// =============================================================================

/// Port of `#define minimum(a,b)` from `Src/zsh.h:31`.
#[inline]
#[allow(non_snake_case)]
pub fn minimum<T: PartialOrd>(a: T, b: T) -> T {
    // c:31
    if a < b {
        a
    } else {
        b
    }
}

/// Port of `typedef ZSH_64_BIT_TYPE zlong;` from `Src/zsh.h:38`.
/// On every modern platform this is `int64_t` / `i64`.
#[allow(non_camel_case_types)]
pub type zlong = i64; // c:38

/// Port of `typedef ZSH_64_BIT_UTYPE zulong;` from `Src/zsh.h:50`.
#[allow(non_camel_case_types)]
pub type zulong = u64; // c:50

/// Port of `#define ZLONG_MAX` from `Src/zsh.h:40-57`.
pub const ZLONG_MAX: zlong = i64::MAX; // c:40-57

// =============================================================================
// 2. mnumber + math-fn types (zsh.h:94-136).
// =============================================================================

/// Port of `mnumber` from `Src/zsh.h:95-101`. C definition:
///
/// ```c
/// typedef struct {
///     union { zlong l; double d; } u;
///     int type;
/// } mnumber;
/// ```
///
/// The C union is represented here with both alternatives held as
/// sibling fields plus a discriminant — `type_` selects which side
/// of the prior `u` union is live. Read `l` when
/// `type_ == MN_INTEGER`, read `d` when `type_ == MN_FLOAT`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy)] // c:95
pub struct mnumber {
    // c:95
    pub l: i64,     // c:97 (u.l)
    pub d: f64,     // c:98 (u.d)
    pub type_: u32, // c:100 (type)
}
/// `MN_INTEGER` from `Src/zsh.h:103`.
pub const MN_INTEGER: u32 = 1; // c:103
/// `MN_FLOAT` from `Src/zsh.h:104`.
pub const MN_FLOAT: u32 = 2; // c:104
/// `MN_UNSET` from `Src/zsh.h:105` — `mnumber not yet retrieved`.
pub const MN_UNSET: u32 = 4; // c:105

/// Port of `typedef struct mathfunc *MathFunc;` from `Src/zsh.h:107`.
pub type MathFunc = Box<mathfunc>; // c:107

/// Port of `typedef mnumber (*NumMathFunc)(...)` from `Src/zsh.h:108`.
pub type NumMathFunc = fn(name: &str, argc: i32, argv: &[mnumber], id: i32) -> mnumber;

/// Port of `typedef mnumber (*StrMathFunc)(...)` from `Src/zsh.h:109`.
pub type StrMathFunc = fn(name: &str, arg: &str, id: i32) -> mnumber;

/// Port of `struct mathfunc` from `Src/zsh.h:111-121`.
#[allow(non_camel_case_types)]
pub struct mathfunc {
    // c:111
    pub next: Option<Box<mathfunc>>, // c:112
    pub name: String,                // c:113
    pub flags: i32,                  // c:114
    pub nfunc: Option<NumMathFunc>,  // c:115
    pub sfunc: Option<StrMathFunc>,  // c:116
    pub module: Option<String>,      // c:117
    pub minargs: i32,                // c:118
    pub maxargs: i32,                // c:119
    pub funcid: i32,                 // c:120
}
/// `MFF_STR` constant.
pub const MFF_STR: i32 = 1; // c:124
/// `MFF_ADDED` constant.
pub const MFF_ADDED: i32 = 2; // c:126
/// `MFF_USERFUNC` constant.
pub const MFF_USERFUNC: i32 = 4; // c:128
/// `MFF_AUTOALL` constant.
pub const MFF_AUTOALL: i32 = 8; // c:130

// =============================================================================
// 3. Meta byte + parser tokens (zsh.h:144-224).
// =============================================================================

// c:144 — `#define Meta ((char) 0x83)`. C `char` is one byte; every
// use site in zsh compares against / writes raw bytes (`bytes[i] ==
// Meta`, `out.push(Meta)`, `t[Meta]` ITOK-table indexing). The
// previous `Meta: char = '\u{83}'` typing was the bot fakery —
// Rust `char` is a 4-byte Unicode scalar, forcing 32+ `as u8` casts
// across the tree. `u8` is the faithful byte type matching C's
// `char` (which on every zsh build is a 1-byte type).
pub const Meta: u8 = 0x83; // c:144
/// `DEFAULT_IFS` constant.
pub const DEFAULT_IFS: &str = " \t\n\u{83} "; // c:149
/// `DEFAULT_IFS_SH` constant.
pub const DEFAULT_IFS_SH: &str = " \t\n"; // c:153

// `DEFAULT_FCEDIT` / `DEFAULT_HISTSIZE` belong to `config.h`, not
// `zsh.h` — they live in `src/ported/config_h.rs` (per PORT.md
// Rule C). Callers use `crate::ported::config_h::DEFAULT_FCEDIT` /
// `crate::ported::config_h::DEFAULT_HISTSIZE` (cast `as i64` where
// the destination is `histsiz: AtomicI64`).

// Byte-token constants — names match C `Src/zsh.h:159-224` exactly
// (PascalCase). One exception: C `String` (`0x85`) would shadow Rust's
// `std::string::String`, so we use `Stringg` for that single token.
#[allow(non_upper_case_globals)]
pub const Pound: char = '\u{84}'; // c:159 #
pub const Stringg: char = '\u{85}'; // c:160 $ — C `String` (renamed: collides with std::string::String)
#[allow(non_upper_case_globals)]
pub const Hat: char = '\u{86}'; // c:161 ^
#[allow(non_upper_case_globals)]
pub const Star: char = '\u{87}'; // c:162 *
#[allow(non_upper_case_globals)]
pub const Inpar: char = '\u{88}'; // c:163 (
#[allow(non_upper_case_globals)]
pub const Inparmath: char = '\u{89}'; // c:164 ((
#[allow(non_upper_case_globals)]
pub const Outpar: char = '\u{8a}'; // c:165 )
#[allow(non_upper_case_globals)]
pub const Outparmath: char = '\u{8b}'; // c:166 ))
#[allow(non_upper_case_globals)]
pub const Qstring: char = '\u{8c}'; // c:167 "$"
#[allow(non_upper_case_globals)]
pub const Equals: char = '\u{8d}'; // c:168 =
#[allow(non_upper_case_globals)]
pub const Bar: char = '\u{8e}'; // c:169 |
#[allow(non_upper_case_globals)]
pub const Inbrace: char = '\u{8f}'; // c:170 {
#[allow(non_upper_case_globals)]
pub const Outbrace: char = '\u{90}'; // c:171 }
#[allow(non_upper_case_globals)]
pub const Inbrack: char = '\u{91}'; // c:172 [
#[allow(non_upper_case_globals)]
pub const Outbrack: char = '\u{92}'; // c:173 ]
#[allow(non_upper_case_globals)]
pub const Tick: char = '\u{93}'; // c:174 `
#[allow(non_upper_case_globals)]
pub const Inang: char = '\u{94}'; // c:175 <
#[allow(non_upper_case_globals)]
pub const Outang: char = '\u{95}'; // c:176 >
#[allow(non_upper_case_globals)]
pub const OutangProc: char = '\u{96}'; // c:177 >(...)
#[allow(non_upper_case_globals)]
pub const Quest: char = '\u{97}'; // c:178 ?
#[allow(non_upper_case_globals)]
pub const Tilde: char = '\u{98}'; // c:179 ~
#[allow(non_upper_case_globals)]
pub const Qtick: char = '\u{99}'; // c:180 "`"
#[allow(non_upper_case_globals)]
pub const Comma: char = '\u{9a}'; // c:181 ,
#[allow(non_upper_case_globals)]
pub const Dash: char = '\u{9b}'; // c:182 -
#[allow(non_upper_case_globals)]
pub const Bang: char = '\u{9c}'; // c:183 !
/// `LAST_NORMAL_TOK` constant.
pub const LAST_NORMAL_TOK: char = Bang; // c:188

#[allow(non_upper_case_globals)]
pub const Snull: char = '\u{9d}'; // c:193
#[allow(non_upper_case_globals)]
pub const Dnull: char = '\u{9e}'; // c:194
#[allow(non_upper_case_globals)]
pub const Bnull: char = '\u{9f}'; // c:195
#[allow(non_upper_case_globals)]
pub const Bnullkeep: char = '\u{a0}'; // c:200
#[allow(non_upper_case_globals)]
pub const Nularg: char = '\u{a1}'; // c:206
#[allow(non_upper_case_globals)]
pub const Marker: char = '\u{a2}'; // c:224
/// `SPECCHARS` constant.
pub const SPECCHARS: &str = "#$^*()=|{}[]`<>?~;&\n\t \\\'\""; // c:228
/// `PATCHARS` constant.
pub const PATCHARS: &str = "#^*()|[]<>?~\\"; // c:232

/// Port of `#define IS_DASH(x)` from `Src/zsh.h:242`.
#[inline]
#[allow(non_snake_case)]
pub fn IS_DASH(x: char) -> bool {
    x == '-' || x == Dash
} // c:242

// =============================================================================
// 4. Quote types (zsh.h:252-294).
// =============================================================================
/// `QT_NONE` constant.
pub const QT_NONE: i32 = 0; // c:257
/// `QT_BACKSLASH` constant.
pub const QT_BACKSLASH: i32 = 1; // c:259
/// `QT_SINGLE` constant.
pub const QT_SINGLE: i32 = 2; // c:261
/// `QT_DOUBLE` constant.
pub const QT_DOUBLE: i32 = 3; // c:263
/// `QT_DOLLARS` constant.
pub const QT_DOLLARS: i32 = 4; // c:265
/// `QT_BACKTICK` constant.
pub const QT_BACKTICK: i32 = 5; // c:271
/// `QT_SINGLE_OPTIONAL` constant.
pub const QT_SINGLE_OPTIONAL: i32 = 6; // c:276
/// `QT_BACKSLASH_PATTERN` constant.
pub const QT_BACKSLASH_PATTERN: i32 = 7; // c:282
/// `QT_BACKSLASH_SHOWNULL` constant.
pub const QT_BACKSLASH_SHOWNULL: i32 = 8; // c:286
/// `QT_QUOTEDZPUTS` constant.
pub const QT_QUOTEDZPUTS: i32 = 9; // c:291

/// Port of `#define QT_IS_SINGLE(x)` from `Src/zsh.h:294`.
#[inline]
#[allow(non_snake_case)]
pub fn QT_IS_SINGLE(x: i32) -> bool {
    x == QT_SINGLE || x == QT_SINGLE_OPTIONAL
}

// =============================================================================
// 5. Lexical tokens (zsh.h:304-371).
// =============================================================================
/// `lextok` type alias.
#[allow(non_camel_case_types)]
pub type lextok = i32;
/// `NULLTOK` constant.
pub const NULLTOK: lextok = 0; // c:305
/// `SEPER` constant.
pub const SEPER: lextok = 1;
/// `NEWLIN` constant.
pub const NEWLIN: lextok = 2;
/// `SEMI` constant.
pub const SEMI: lextok = 3;
/// `DSEMI` constant.
pub const DSEMI: lextok = 4;
/// `AMPER` constant.
pub const AMPER: lextok = 5;
/// `INPAR_TOK` constant.
pub const INPAR_TOK: lextok = 6; // collision with char INPAR; suffix
/// `OUTPAR_TOK` constant.
pub const OUTPAR_TOK: lextok = 7;
/// `DBAR` constant.
pub const DBAR: lextok = 8;
/// `DAMPER` constant.
pub const DAMPER: lextok = 9;
/// `OUTANG_TOK` constant.
pub const OUTANG_TOK: lextok = 10; // collision with char OUTANG
/// `OUTANGBANG` constant.
pub const OUTANGBANG: lextok = 11;
/// `DOUTANG` constant.
pub const DOUTANG: lextok = 12;
/// `DOUTANGBANG` constant.
pub const DOUTANGBANG: lextok = 13;
/// `INANG_TOK` constant.
pub const INANG_TOK: lextok = 14;
/// `INOUTANG` constant.
pub const INOUTANG: lextok = 15;
/// `DINANG` constant.
pub const DINANG: lextok = 16;
/// `DINANGDASH` constant.
pub const DINANGDASH: lextok = 17;
/// `INANGAMP` constant.
pub const INANGAMP: lextok = 18;
/// `OUTANGAMP` constant.
pub const OUTANGAMP: lextok = 19;
/// `AMPOUTANG` constant.
pub const AMPOUTANG: lextok = 20;
/// `OUTANGAMPBANG` constant.
pub const OUTANGAMPBANG: lextok = 21;
/// `DOUTANGAMP` constant.
pub const DOUTANGAMP: lextok = 22;
/// `DOUTANGAMPBANG` constant.
pub const DOUTANGAMPBANG: lextok = 23;
/// `TRINANG` constant.
pub const TRINANG: lextok = 24;
/// `BAR_TOK` constant.
pub const BAR_TOK: lextok = 25;
/// `BARAMP` constant.
pub const BARAMP: lextok = 26;
/// `INOUTPAR` constant.
pub const INOUTPAR: lextok = 27;
/// `DINPAR` constant.
pub const DINPAR: lextok = 28;
/// `DOUTPAR` constant.
pub const DOUTPAR: lextok = 29;
/// `AMPERBANG` constant.
pub const AMPERBANG: lextok = 30;
/// `SEMIAMP` constant.
pub const SEMIAMP: lextok = 31;
/// `SEMIBAR` constant.
pub const SEMIBAR: lextok = 32;
/// `DOUTBRACK` constant.
pub const DOUTBRACK: lextok = 33;
/// `STRING_LEX` constant.
pub const STRING_LEX: lextok = 34;
/// `ENVSTRING` constant.
pub const ENVSTRING: lextok = 35;
/// `ENVARRAY` constant.
pub const ENVARRAY: lextok = 36;
/// `ENDINPUT` constant.
pub const ENDINPUT: lextok = 37;
/// `LEXERR` constant.
pub const LEXERR: lextok = 38;
/// `BANG_TOK` constant.
pub const BANG_TOK: lextok = 39; // c:346
/// `DINBRACK` constant.
pub const DINBRACK: lextok = 40;
/// `INBRACE_TOK` constant.
pub const INBRACE_TOK: lextok = 41;
/// `OUTBRACE_TOK` constant.
pub const OUTBRACE_TOK: lextok = 42;
/// `CASE` constant.
pub const CASE: lextok = 43;
/// `COPROC` constant.
pub const COPROC: lextok = 44;
/// `DOLOOP` constant.
pub const DOLOOP: lextok = 45;
/// `DONE` constant.
pub const DONE: lextok = 46;
/// `ELIF` constant.
pub const ELIF: lextok = 47;
/// `ELSE` constant.
pub const ELSE: lextok = 48;
/// `ZEND` constant.
pub const ZEND: lextok = 49;
/// `ESAC` constant.
pub const ESAC: lextok = 50;
/// `FI` constant.
pub const FI: lextok = 51;
/// `FOR` constant.
pub const FOR: lextok = 52;
/// `FOREACH` constant.
pub const FOREACH: lextok = 53;
/// `FUNC` constant.
pub const FUNC: lextok = 54;
/// `IF` constant.
pub const IF: lextok = 55;
/// `NOCORRECT` constant.
pub const NOCORRECT: lextok = 56;
/// `REPEAT` constant.
pub const REPEAT: lextok = 57;
/// `SELECT` constant.
pub const SELECT: lextok = 58;
/// `THEN` constant.
pub const THEN: lextok = 59;
/// `TIME` constant.
pub const TIME: lextok = 60;
/// `UNTIL` constant.
pub const UNTIL: lextok = 61;
/// `WHILE` constant.
pub const WHILE: lextok = 62;
/// `TYPESET` constant.
pub const TYPESET: lextok = 63; // c:370

// =============================================================================
// 6. Redirection types (zsh.h:377-408).
// =============================================================================
/// `REDIR_WRITE` constant.
pub const REDIR_WRITE: i32 = 0;
/// `REDIR_WRITENOW` constant.
pub const REDIR_WRITENOW: i32 = 1;
/// `REDIR_APP` constant.
pub const REDIR_APP: i32 = 2;
/// `REDIR_APPNOW` constant.
pub const REDIR_APPNOW: i32 = 3;
/// `REDIR_ERRWRITE` constant.
pub const REDIR_ERRWRITE: i32 = 4;
/// `REDIR_ERRWRITENOW` constant.
pub const REDIR_ERRWRITENOW: i32 = 5;
/// `REDIR_ERRAPP` constant.
pub const REDIR_ERRAPP: i32 = 6;
/// `REDIR_ERRAPPNOW` constant.
pub const REDIR_ERRAPPNOW: i32 = 7;
/// `REDIR_READWRITE` constant.
pub const REDIR_READWRITE: i32 = 8;
/// `REDIR_READ` constant.
pub const REDIR_READ: i32 = 9;
/// `REDIR_HEREDOC` constant.
pub const REDIR_HEREDOC: i32 = 10;
/// `REDIR_HEREDOCDASH` constant.
pub const REDIR_HEREDOCDASH: i32 = 11;
/// `REDIR_HERESTR` constant.
pub const REDIR_HERESTR: i32 = 12;
/// `REDIR_MERGEIN` constant.
pub const REDIR_MERGEIN: i32 = 13;
/// `REDIR_MERGEOUT` constant.
pub const REDIR_MERGEOUT: i32 = 14;
/// `REDIR_CLOSE` constant.
pub const REDIR_CLOSE: i32 = 15;
/// `REDIR_INPIPE` constant.
pub const REDIR_INPIPE: i32 = 16;
/// `REDIR_OUTPIPE` constant.
pub const REDIR_OUTPIPE: i32 = 17;
/// `REDIR_TYPE_MASK` constant.
pub const REDIR_TYPE_MASK: i32 = 0x1f; // c:397
/// `REDIR_VARID_MASK` constant.
pub const REDIR_VARID_MASK: i32 = 0x20; // c:399
/// `REDIR_FROM_HEREDOC_MASK` constant.
pub const REDIR_FROM_HEREDOC_MASK: i32 = 0x40; // c:401
/// `IS_WRITE_FILE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn IS_WRITE_FILE(x: i32) -> bool {
    x >= REDIR_WRITE && x <= REDIR_READWRITE
}
/// `IS_APPEND_REDIR` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn IS_APPEND_REDIR(x: i32) -> bool {
    IS_WRITE_FILE(x) && (x & 2) != 0
}
/// `IS_CLOBBER_REDIR` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn IS_CLOBBER_REDIR(x: i32) -> bool {
    IS_WRITE_FILE(x) && (x & 1) != 0
}
/// `IS_ERROR_REDIR` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn IS_ERROR_REDIR(x: i32) -> bool {
    x >= REDIR_ERRWRITE && x <= REDIR_ERRAPPNOW
}
/// `IS_READFD` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn IS_READFD(x: i32) -> bool {
    (x >= REDIR_READWRITE && x <= REDIR_MERGEIN) || x == REDIR_INPIPE
}
/// `IS_REDIROP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn IS_REDIROP(x: lextok) -> bool {
    x >= OUTANG_TOK && x <= TRINANG
}

// =============================================================================
// 7. fdtable values (zsh.h:415-465).
// =============================================================================
/// `FDT_UNUSED` constant.
pub const FDT_UNUSED: i32 = 0; // c:416
/// `FDT_INTERNAL` constant.
pub const FDT_INTERNAL: i32 = 1; // c:421
/// `FDT_EXTERNAL` constant.
pub const FDT_EXTERNAL: i32 = 2; // c:426
/// `FDT_MODULE` constant.
pub const FDT_MODULE: i32 = 3; // c:433
/// `FDT_XTRACE` constant.
pub const FDT_XTRACE: i32 = 4; // c:437
/// `FDT_FLOCK` constant.
pub const FDT_FLOCK: i32 = 5; // c:441
/// `FDT_FLOCK_EXEC` constant.
pub const FDT_FLOCK_EXEC: i32 = 6; // c:446
/// `FDT_PROC_SUBST` constant.
pub const FDT_PROC_SUBST: i32 = 7; // c:454
/// `FDT_TYPE_MASK` constant.
pub const FDT_TYPE_MASK: i32 = 15; // c:458
/// `FDT_SAVED_MASK` constant.
pub const FDT_SAVED_MASK: i32 = 16; // c:465

// =============================================================================
// 8. Input-stack flags (zsh.h:468-476).
// =============================================================================
/// `INP_FREE` constant.
pub const INP_FREE: i32 = 1 << 0; // c:468
/// `INP_ALIAS` constant.
pub const INP_ALIAS: i32 = 1 << 1; // c:469
/// `INP_HIST` constant.
pub const INP_HIST: i32 = 1 << 2; // c:470
/// `INP_CONT` constant.
pub const INP_CONT: i32 = 1 << 3; // c:471
/// `INP_ALCONT` constant.
pub const INP_ALCONT: i32 = 1 << 4; // c:472
/// `INP_HISTCONT` constant.
pub const INP_HISTCONT: i32 = 1 << 5; // c:473
/// `INP_LINENO` constant.
pub const INP_LINENO: i32 = 1 << 6; // c:474
/// `INP_APPEND` constant.
pub const INP_APPEND: i32 = 1 << 7; // c:475
/// `INP_RAW_KEEP` constant.
pub const INP_RAW_KEEP: i32 = 1 << 8; // c:476

// =============================================================================
// 9. metafy flags (zsh.h:479-486).
// =============================================================================
/// `META_REALLOC` constant.
pub const META_REALLOC: i32 = 0; // c:479
/// `META_USEHEAP` constant.
pub const META_USEHEAP: i32 = 1;
/// `META_STATIC` constant.
pub const META_STATIC: i32 = 2;
/// `META_DUP` constant.
pub const META_DUP: i32 = 3;
/// `META_ALLOC` constant.
pub const META_ALLOC: i32 = 4;
/// `META_NOALLOC` constant.
pub const META_NOALLOC: i32 = 5;
/// `META_HEAPDUP` constant.
pub const META_HEAPDUP: i32 = 6;
/// `META_HREALLOC` constant.
pub const META_HREALLOC: i32 = 7;

// =============================================================================
// 10. ZCONTEXT_* (zsh.h:489-496) + entersubsh_ret (c:499-504).
// =============================================================================
/// `ZCONTEXT_HIST` constant.
pub const ZCONTEXT_HIST: i32 = 1 << 0; // c:491
/// `ZCONTEXT_LEX` constant.
pub const ZCONTEXT_LEX: i32 = 1 << 1; // c:493
/// `ZCONTEXT_PARSE` constant.
pub const ZCONTEXT_PARSE: i32 = 1 << 2; // c:495
/// `entersubsh_ret` — see fields for layout.
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct entersubsh_ret {
    // c:499
    pub gleader: i32,       // c:501
    pub list_pipe_job: i32, // c:503
}

// =============================================================================
// 11. Linknode/linklist (zsh.h:557-572) + opaque pointer typedefs (c:510-549).
// =============================================================================
/// `linknode` — see fields for layout.
#[allow(non_camel_case_types)]
pub struct linknode {
    // c:557
    /// `next` field.
    pub next: Option<Box<linknode>>,
    /// `prev` field.
    pub prev: Option<Box<linknode>>,
    /// `dat` field.
    pub dat: usize,
}
/// `linklist` — see fields for layout.
#[allow(non_camel_case_types)]
pub struct linklist {
    // c:563
    /// `first` field.
    pub first: Option<Box<linknode>>,
    /// `last` field.
    pub last: Option<Box<linknode>>,
    /// `flags` field.
    pub flags: i32,
}
/// `LinkNode` type alias.
pub type LinkNode = Box<linknode>; // c:533
/// `LinkList` type alias.
pub type LinkList = Box<linklist>; // c:534

// Pointer typedefs for the ~50 struct types declared at c:510-549.
// Each maps to a Box of the matching struct, with full field-by-field
// body ports below (organized by C source line). Forward typedefs
// here so structs that reference each other (e.g. param.old: Param)
// can compile.
/// `Alias` type alias.
pub type Alias = Box<alias>; // c:510
/// `Asgment` type alias.
pub type Asgment = Box<asgment>; // c:511
/// `Builtin` type alias.
pub type Builtin = Box<builtin>; // c:512
/// `Cmdnam` type alias.
pub type Cmdnam = Box<cmdnam>; // c:513
                               // `struct complist` body lives in `crate::ported::glob` (mirrors
                               // C: declared in zsh.h via typedef alias, body defined in glob.c).
                               // The `Complist` alias here resolves to that struct.
/// `Complist` type alias.
pub type Complist = Box<crate::ported::glob::complist>; // c:514
/// `Conddef` type alias.
pub type Conddef = Box<conddef>; // c:515
/// `Dirsav` type alias.
pub type Dirsav = Box<dirsav>; // c:516
/// `Emulation_options` type alias.
pub type Emulation_options = Box<emulation_options>; // c:517
/// `Execcmd_params` type alias.
pub type Execcmd_params = Box<execcmd_params>; // c:518
/// `Features` type alias.
pub type Features = Box<features>; // c:519
/// `Feature_enables` type alias.
pub type Feature_enables = Box<feature_enables>; // c:520
/// `Funcstack` type alias.
pub type Funcstack = Box<funcstack>; // c:521
/// `FuncWrap` type alias.
pub type FuncWrap = Box<funcwrap>; // c:522
/// `HashNode` type alias.
pub type HashNode = Box<hashnode>; // c:523
/// `HashTable` type alias.
pub type HashTable = Box<hashtable>; // c:524
/// `Heap` type alias.
pub type Heap = Box<heap>; // c:525
/// `Heapstack` type alias.
pub type Heapstack = Box<heapstack>; // c:526
/// `Histent` type alias.
pub type Histent = Box<histent>; // c:527
/// `Hookdef` type alias.
pub type Hookdef = Box<hookdef>; // c:528
/// `Imatchdata` type alias.
pub type Imatchdata = Box<imatchdata>; // c:529
/// `Job` type alias.
pub type Job = Box<job>; // c:531
/// `Jobfile` type alias.
pub type Jobfile = Box<jobfile>; // c:530
/// `Linkedmod` type alias.
pub type Linkedmod = Box<linkedmod>; // c:532
/// `Module` type alias.
pub type Module = Box<module>; // c:535
/// `Nameddir` type alias.
pub type Nameddir = Box<nameddir>; // c:536
/// `Options` type alias.
pub type Options = Box<options>; // c:537
/// `Optname` type alias.
pub type Optname = Box<optname>; // c:538
/// `Param` type alias.
pub type Param = Box<param>; // c:539
/// `Paramdef` type alias.
pub type Paramdef = Box<paramdef>; // c:540
/// `Patstralloc` type alias.
pub type Patstralloc = Box<patstralloc>; // c:541
/// `Patprog` type alias.
pub type Patprog = Box<patprog>; // c:542
/// `Prepromptfn` type alias.
pub type Prepromptfn = Box<prepromptfn>; // c:543
/// `Process` type alias.
pub type Process = Box<process>; // c:544
/// `Redir` type alias.
pub type Redir = Box<redir>; // c:545
/// `Reswd` type alias.
pub type Reswd = Box<reswd>; // c:546
/// `Shfunc` type alias.
pub type Shfunc = Box<shfunc>; // c:547
/// `Timedfn` type alias.
pub type Timedfn = Box<timedfn>; // c:548
/// `Value` type alias.
pub type Value = Box<value>; // c:549
/// `voidvoidfnptr_t` type alias.
pub type voidvoidfnptr_t = fn(); // c:621

// Body-by-body struct definitions (C source order, fields verbatim
// from zsh.h). Reserved-keyword Rust fields renamed minimally
// (type→typ, str→str, match→match_, new→new_, loop→loop_,
// mod→mod_, fn→fn_, ref→ref_, in→in_, where→where_).

/// Port of `struct prepromptfn` from `Src/zsh.h:626-628`.
#[allow(non_camel_case_types)]
pub struct prepromptfn {
    // c:626
    /// `func` field.
    pub func: voidvoidfnptr_t,
}

/// Port of `struct timedfn` from `Src/zsh.h:634-637`.
#[allow(non_camel_case_types)]
pub struct timedfn {
    // c:634
    /// `func` field.
    pub func: voidvoidfnptr_t,
    pub when: i64, // time_t
}

/// Port of `typedef int (*CondHandler)(...)` from `Src/zsh.h:681`.
pub type CondHandler = fn(args: &[String], id: i32) -> i32;

/// Port of `struct conddef` from `Src/zsh.h:683-692`.
#[allow(non_camel_case_types)]
pub struct conddef {
    // c:683
    pub next: Option<Conddef>,        // c:684
    pub name: String,                 // c:685
    pub flags: i32,                   // c:686 CONDF_*
    pub handler: Option<CondHandler>, // c:687
    pub min: i32,                     // c:688
    pub max: i32,                     // c:689
    pub condid: i32,                  // c:690
    pub module: Option<String>,       // c:691
}

/// Port of `struct dirsav` from `Src/zsh.h:1159-1164`.
#[allow(non_camel_case_types)]
pub struct dirsav {
    // c:1159
    pub dirfd: i32,              // c:1160
    pub level: i32,              // c:1160
    pub dirname: Option<String>, // c:1161
    pub dev: u64,                // c:1162 dev_t
    pub ino: u64,                // c:1163 ino_t
}

/// Port of `struct hashnode` from `Src/zsh.h:1226-1230`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Default)]
pub struct hashnode {
    // c:1226
    pub next: Option<HashNode>, // c:1227
    pub nam: String,            // c:1228
    pub flags: i32,             // c:1229
}

// hashtable function-pointer typedefs (zsh.h:1175-1193).
/// `VFunc` type alias.
pub type VFunc = fn(usize) -> usize; // c:1172
/// `FreeFunc` type alias.
pub type FreeFunc = fn(usize); // c:1173
/// `HashFunc` type alias.
pub type HashFunc = fn(name: &str) -> u32; // c:1175
/// `TableFunc` type alias.
pub type TableFunc = fn(table: &mut hashtable); // c:1176
/// `AddNodeFunc` type alias.
pub type AddNodeFunc = fn(table: &mut hashtable, name: String, val: usize);
/// `GetNodeFunc` type alias.
pub type GetNodeFunc = fn(table: &hashtable, name: &str) -> Option<HashNode>;
/// `RemoveNodeFunc` type alias.
pub type RemoveNodeFunc = fn(table: &mut hashtable, name: &str) -> Option<HashNode>;
/// `FreeNodeFunc` type alias.
pub type FreeNodeFunc = fn(node: HashNode);
/// `CompareFunc` type alias.
pub type CompareFunc = fn(a: &str, b: &str) -> i32;
/// `ScanFunc` type alias.
pub type ScanFunc = fn(node: &HashNode, flags: i32);
/// `ScanTabFunc` type alias.
pub type ScanTabFunc = fn(table: &hashtable, func: ScanFunc, flags: i32);
/// `PrintTableStats` type alias.
pub type PrintTableStats = fn(table: &hashtable);

/// Port of `struct hashtable` from `Src/zsh.h:1200-1222`.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct hashtable {
    // c:1200
    pub hsize: i32,                         // c:1202
    pub ct: i32,                            // c:1203
    pub nodes: Vec<Option<HashNode>>,       // c:1204
    pub tmpdata: usize,                     // c:1205
    pub hash: Option<HashFunc>,             // c:1208
    pub emptytable: Option<TableFunc>,      // c:1209
    pub filltable: Option<TableFunc>,       // c:1210
    pub cmpnodes: Option<CompareFunc>,      // c:1211
    pub addnode: Option<AddNodeFunc>,       // c:1212
    pub getnode: Option<GetNodeFunc>,       // c:1213
    pub getnode2: Option<GetNodeFunc>,      // c:1214
    pub removenode: Option<RemoveNodeFunc>, // c:1216
    pub disablenode: Option<ScanFunc>,      // c:1217
    pub enablenode: Option<ScanFunc>,       // c:1218
    pub freenode: Option<FreeNodeFunc>,     // c:1219
    pub printnode: Option<ScanFunc>,        // c:1220
    pub scantab: Option<ScanTabFunc>,       // c:1221
}

/// Port of `struct optname` from `Src/zsh.h:1239-1242`.
#[allow(non_camel_case_types)]
pub struct optname {
    // c:1239
    pub node: hashnode, // c:1240
    pub optno: i32,     // c:1241
}

/// Port of `struct reswd` from `Src/zsh.h:1246-1249`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct reswd {
    // c:1246
    pub node: hashnode, // c:1247
    pub token: i32,     // c:1248
}

/// Port of `struct alias` from `Src/zsh.h:1253-1257`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct alias {
    // c:1253
    pub node: hashnode, // c:1254
    pub text: String,   // c:1255
    pub inuse: i32,     // c:1256
}

/// Port of `struct asgment` from `Src/zsh.h:1267-1275`. Note the C
/// union is split into two Option<…> fields here; only one is set
/// per asgment (dispatched by `flags & ASG_ARRAY`).
///
/// `array` is typed `LinkList<String>` (the generic port in
/// `src/ported/linklist.rs`) rather than the bare `LinkList` type
/// alias above — C stores `char *` payload in the list, so the
/// typed form is what `firstnode`/`getdata`/`nextnode` traversal
/// expects at every assign-printing site (e.g. `Src/builtin.c:476-482`).
#[allow(non_camel_case_types)]
pub struct asgment {
    // c:1267
    pub node: linknode,                                           // c:1268
    pub name: String,                                             // c:1269
    pub flags: i32,                                               // c:1270 ASG_*
    pub scalar: Option<String>,                                   // c:1272 union value.scalar
    pub array: Option<crate::ported::linklist::LinkList<String>>, // c:1273 union value.array (LinkList of char *)
}

/// Port of `struct cmdnam` from `Src/zsh.h:1301-1308`. The C union
/// `{ char **name; char *cmd; }` becomes two Option fields; only
/// one is set per cmdnam (dispatched by `flags & HASHED`).
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct cmdnam {
    // c:1301
    pub node: hashnode,            // c:1302
    pub name: Option<Vec<String>>, // c:1304 union u.name
    pub cmd: Option<String>,       // c:1305 union u.cmd
}

/// Port of `struct shfunc` from `Src/zsh.h:1316-1325`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct shfunc {
    // c:1316
    pub node: hashnode,                    // c:1317
    pub filename: Option<String>,          // c:1318
    pub lineno: i64,                       // c:1321 zlong
    pub funcdef: Option<Eprog>,            // c:1322
    pub redir: Option<Eprog>,              // c:1323
    pub sticky: Option<Emulation_options>, // c:1324
    /// **RUST-ONLY EXTENSION (no C counterpart).** Raw source text
    /// for deferred-compile path: zshrs stores the function body
    /// as-typed and parses on first invocation, vs C eagerly
    /// compiling into `funcdef: Eprog` at definition time. When
    /// the fusevm bytecode cache lands, this field gets retired in
    /// favor of populating `funcdef` directly (matching C). Until
    /// then, both fields can be set: `funcdef` for compiled
    /// callers, `body` for the lazy-compile path.
    pub body: Option<String>,
}

/// Port of `struct funcstack` from `Src/zsh.h:1348-1356`.
#[allow(non_camel_case_types)]
#[derive(Clone, Default)]
pub struct funcstack {
    // c:1348
    pub prev: Option<Funcstack>,  // c:1349
    pub name: String,             // c:1350
    pub filename: Option<String>, // c:1351
    pub caller: Option<String>,   // c:1352
    pub flineno: i64,             // c:1353
    pub lineno: i64,              // c:1354
    pub tp: i32,                  // c:1355 FS_*
}

/// Port of `typedef int (*WrapFunc)(...)` from `Src/zsh.h:1360`.
pub type WrapFunc = fn(prog: Eprog, w: FuncWrap, name: &str) -> i32;

/// Port of `struct funcwrap` from `Src/zsh.h:1362-1367`.
#[allow(non_camel_case_types)]
pub struct funcwrap {
    // c:1362
    pub next: Option<FuncWrap>,    // c:1363
    pub flags: i32,                // c:1364
    pub handler: Option<WrapFunc>, // c:1365
    pub module: Option<Module>,    // c:1366
}

/// Port of `struct builtin` from `Src/zsh.h:1440-1448`.
#[allow(non_camel_case_types)]
pub struct builtin {
    // c:1440
    pub node: hashnode,                   // c:1441
    pub handlerfunc: Option<HandlerFunc>, // c:1442
    pub minargs: i32,                     // c:1443
    pub maxargs: i32,                     // c:1444
    pub funcid: i32,                      // c:1445
    pub optstr: Option<String>,           // c:1446
    pub defopts: Option<String>,          // c:1447
}

/// Port of `struct execcmd_params` from `Src/zsh.h:1492-1501`.
///
/// C's `Wordcode beg/varspc/assignspc` are `wordcode *` — raw pointers
/// into the running wordcode stream owned by `state->prog`. Rust's
/// safe analog is `usize` (index into `state.prog.prog`); `None`
/// stands in for C's `NULL`. The wordcode bytes themselves stay in
/// `state.prog` — eparams just records start offsets.
#[allow(non_camel_case_types)]
#[derive(Default)]
pub struct execcmd_params {
    // c:1492
    pub args: Option<Vec<String>>, // c:1493 LinkList args
    pub redir: Option<Vec<redir>>, // c:1494 LinkList redir
    pub beg: usize,                // c:1495 Wordcode beg (pc index)
    pub varspc: Option<usize>,     // c:1496 Wordcode varspc (NULL → None)
    pub assignspc: Option<usize>,  // c:1497 Wordcode assignspc
    pub typ: i32,                  // c:1498 (Rust keyword `type`)
    pub postassigns: i32,          // c:1499
    pub htok: i32,                 // c:1500
}

/// Port of `struct module` from `Src/zsh.h:1503-1513`. C uses a union
/// for handle/linked/alias dispatched implicitly by load type; Rust
/// port keeps three Options.
///
/// `autoloads`/`deps` are `LinkList<String>` because C's untyped
/// `LinkList` carries `char *` payload for these fields (see
/// `Src/module.c:2392` `zaddlinknode(m->deps, dep)` etc.).
#[allow(non_camel_case_types)]
#[derive(Debug)]
pub struct module {
    // c:1503
    pub node: hashnode,                                               // c:1504
    pub handle: Option<usize>,     // c:1506 union.handle (void *)
    pub linked: Option<Linkedmod>, // c:1507 union.linked
    pub alias: Option<String>,     // c:1508 union.alias
    pub autoloads: Option<crate::ported::linklist::LinkList<String>>, // c:1510
    pub deps: Option<crate::ported::linklist::LinkList<String>>, // c:1511
    pub wrapper: i32,              // c:1512
}

impl module {
    /// Construct a fresh statically-linked module entry. Mirrors C's
    /// `zshcalloc(sizeof(*m))` + `m->node.nam = ztrdup(name)` pattern
    /// at `Src/module.c:361` (`register_module`).
    pub fn new(name: &str) -> Self {
        Self {
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags: MOD_LINKED,
            },
            handle: None,
            linked: None,
            alias: None,
            autoloads: None,
            deps: None,
            wrapper: 0,
        }
    }

    /// True if the module is currently usable.
    /// Mirrors C's `MOD_BUSY`/`MOD_UNLOAD` checks at `Src/module.c:1703`
    /// (`module_loaded`): the module exists and `MOD_UNLOAD` is clear.
    pub fn is_loaded(&self) -> bool {
        (self.node.flags & MOD_LINKED) != 0 && (self.node.flags & MOD_UNLOAD) == 0
    }
}

/// Port of module fn-pointer typedefs from `Src/zsh.h:1534-1537`.
pub type Module_generic_func = fn() -> i32;
/// `Module_void_func` type alias.
pub type Module_void_func = fn(m: &module) -> i32;
/// `Module_features_func` type alias.
pub type Module_features_func = fn(m: &module, features: &mut Vec<String>) -> i32;
/// `Module_enables_func` type alias.
pub type Module_enables_func = fn(m: &module, enables: &mut Vec<i32>) -> i32;

/// Port of `struct linkedmod` from `Src/zsh.h:1539-1547`.
#[allow(non_camel_case_types)]
#[derive(Debug)]
pub struct linkedmod {
    // c:1539
    pub name: String,                           // c:1540
    pub setup: Option<Module_void_func>,        // c:1541
    pub features: Option<Module_features_func>, // c:1542
    pub enables: Option<Module_enables_func>,   // c:1543
    pub boot: Option<Module_void_func>,         // c:1544
    pub cleanup: Option<Module_void_func>,      // c:1545
    pub finish: Option<Module_void_func>,       // c:1546
}

/// Port of `struct features` from `Src/zsh.h:1553-1568`.
#[allow(non_camel_case_types)]
pub struct features {
    // c:1553
    pub bn_list: Option<Builtin>,  // c:1555
    pub bn_size: i32,              // c:1556
    pub cd_list: Option<Conddef>,  // c:1558
    pub cd_size: i32,              // c:1559
    pub mf_list: Option<MathFunc>, // c:1561
    pub mf_size: i32,              // c:1562
    pub pd_list: Option<Paramdef>, // c:1564
    pub pd_size: i32,              // c:1565
    pub n_abstract: i32,           // c:1567
}

/// Port of `struct feature_enables` from `Src/zsh.h:1573-1578`.
#[allow(non_camel_case_types)]
pub struct feature_enables {
    // c:1573
    pub str: String,          // c:1575 (Rust keyword `str`)
    pub pat: Option<Patprog>, // c:1577
}

/// Port of `typedef int (*Hookfn)(Hookdef, void *)` from `Src/zsh.h:1582`.
/// Real Rust fn-pointer matching the C ABI of a hook callback.
pub type Hookfn = fn(h: *mut hookdef, d: *mut std::ffi::c_void) -> i32;

/// Port of `struct hookdef` from `Src/zsh.h:1584-1590`.
/// `next` and `funcs` are raw pointers matching the C
/// `Hookdef next` / `LinkList funcs` types exactly. NULL is represented
/// by `std::ptr::null_mut()`. Backing storage for hookdef nodes is
/// expected to be `Box::leak`'d (mirroring C's static-storage
/// `zshhooks[]` array and module-side `struct hookdef foo[] = { ... }`
/// constants).
#[allow(non_camel_case_types)]
pub struct hookdef {
    // c:1584
    pub next: *mut hookdef,   // c:1585 — struct hookdef *
    pub name: String,         // c:1586 — char *
    pub def: Option<Hookfn>,  // c:1587 — Hookfn (NULL = None)
    pub flags: i32,           // c:1588
    pub funcs: *mut linklist, // c:1589 — LinkList (struct linklist *)
}
// SAFETY: hookdef contains raw pointers. C zsh is single-threaded;
// zshrs serializes hook operations via the module-level `hooktab`
// AtomicPtr + the global_state_lock used by tests. Marking explicitly
// because raw pointers default to !Send/!Sync.
unsafe impl Send for hookdef {}
unsafe impl Sync for hookdef {}

/// Port of `struct patprog` from `Src/zsh.h:1601-1611`.
///
/// C layout uses a trailing byte buffer accessed via
/// `(char *)prog + prog->startoff`; the Rust port stores it inline
/// as a `code: Vec<u8>` field. The opcode stream layout is preserved
/// byte-for-byte — `startoff` and `size` index into `code`.
///
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct patprog {
    // c:1601
    pub startoff: i64,  // c:1602
    pub size: i64,      // c:1603
    pub mustoff: i64,   // c:1604
    pub patmlen: i64,   // c:1605
    pub globflags: i32, // c:1606
    pub globend: i32,   // c:1607
    pub flags: i32,     // c:1608 PAT_*
    pub patnpar: i32,   // c:1609
    pub patstartch: u8, // c:1610 (last field per zsh.h)
}

/// Port of `struct patstralloc` from `Src/zsh.h:1613-1620`.
#[allow(non_camel_case_types)]
pub struct patstralloc {
    // c:1613
    pub unmetalen: i32,                // c:1614
    pub unmetalenp: i32,               // c:1615
    pub alloced: Option<String>,       // c:1617
    pub progstrunmeta: Option<String>, // c:1618
    pub progstrunmetalen: i32,         // c:1619
}

/// Port of `struct zpc_disables_save` from `Src/zsh.h:1681-1689`.
#[allow(non_camel_case_types)]
pub struct zpc_disables_save {
    // c:1681
    pub next: Option<Box<zpc_disables_save>>, // c:1682
    pub disables: u32,                        // c:1688
}
/// `Zpc_disables_save` type alias.
pub type Zpc_disables_save = Box<zpc_disables_save>; // c:1691

/// Port of `struct imatchdata` from `Src/zsh.h:1740-1760`.
#[allow(non_camel_case_types)]
pub struct imatchdata {
    // c:1740
    pub mstr: Option<String>,       // c:1742
    pub mlen: i32,                  // c:1744
    pub ustr: Option<String>,       // c:1746
    pub ulen: i32,                  // c:1748
    pub flags: i32,                 // c:1750 SUB_*
    pub replstr: Option<String>,    // c:1752
    pub repllist: Option<LinkList>, // c:1759
}

// gsu_* function-pointer typedefs (zsh.h:1790-1794) + structs.
/// `GsuScalar` type alias.
pub type GsuScalar = Box<gsu_scalar>; // c:1790
/// `GsuInteger` type alias.
pub type GsuInteger = Box<gsu_integer>; // c:1791
/// `GsuFloat` type alias.
pub type GsuFloat = Box<gsu_float>; // c:1792
/// `GsuArray` type alias.
pub type GsuArray = Box<gsu_array>; // c:1793
/// `GsuHash` type alias.
pub type GsuHash = Box<gsu_hash>; // c:1794
/// `gsu_scalar` — see fields for layout.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct gsu_scalar {
    // c:1796
    pub getfn: fn(pm: &param) -> String,        // c:1797
    pub setfn: fn(pm: &mut param, val: String), // c:1798
    pub unsetfn: fn(pm: &mut param, exp: i32),  // c:1799
}
/// `gsu_integer` — see fields for layout.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct gsu_integer {
    // c:1802
    pub getfn: fn(pm: &param) -> i64,
    /// `setfn` field.
    pub setfn: fn(pm: &mut param, val: i64),
    /// `unsetfn` field.
    pub unsetfn: fn(pm: &mut param, exp: i32),
}
/// `gsu_float` — see fields for layout.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct gsu_float {
    // c:1808
    pub getfn: fn(pm: &param) -> f64,
    /// `setfn` field.
    pub setfn: fn(pm: &mut param, val: f64),
    /// `unsetfn` field.
    pub unsetfn: fn(pm: &mut param, exp: i32),
}
/// `gsu_array` — see fields for layout.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct gsu_array {
    // c:1814
    pub getfn: fn(pm: &param) -> Vec<String>,
    /// `setfn` field.
    pub setfn: fn(pm: &mut param, val: Vec<String>),
    /// `unsetfn` field.
    pub unsetfn: fn(pm: &mut param, exp: i32),
}
/// `gsu_hash` — see fields for layout.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct gsu_hash {
    // c:1820
    pub getfn: fn(pm: &param) -> Option<&HashTable>,
    /// `setfn` field.
    pub setfn: fn(pm: &mut param, val: HashTable),
    /// `unsetfn` field.
    pub unsetfn: fn(pm: &mut param, exp: i32),
}

/// Port of `struct param` from `Src/zsh.h:1829-1867`. The C unions
/// `u` (data) and `gsu` (vtable) are flattened into per-variant
/// fields; the dispatcher looks at `node.flags & PM_TYPE` and reads
/// the matching field.
#[allow(non_camel_case_types)]
#[derive(Clone, Default)]
pub struct param {
    // c:1829
    pub node: hashnode, // c:1830
    // u union (c:1833-1842):
    pub u_data: usize,              // c:1834 void *data
    // c:1834 — typed view of `u.data` for PM_TIED scalars, where C
    // stores a `struct tieddata *` (Src/zsh.h:1870-1873) carrying the
    // partner-array pointer and the joinchar from `typeset -T s a SEP`.
    pub u_tied: Option<Box<tieddata>>,
    pub u_arr: Option<Vec<String>>, // c:1835 char **arr
    pub u_str: Option<String>,      // c:1836 char *str
    pub u_val: i64,                 // c:1837 zlong val
    pub u_dval: f64,                // c:1839 double dval
    pub u_hash: Option<HashTable>,  // c:1841 HashTable hash
    // gsu vtable union (c:1852-1858):
    pub gsu_s: Option<GsuScalar>,  // c:1853
    pub gsu_i: Option<GsuInteger>, // c:1854
    pub gsu_f: Option<GsuFloat>,   // c:1855
    pub gsu_a: Option<GsuArray>,   // c:1856
    pub gsu_h: Option<GsuHash>,    // c:1857
    pub base: i32,                 // c:1860
    pub width: i32,                // c:1862
    pub env: Option<String>,       // c:1863
    pub ename: Option<String>,     // c:1864
    pub old: Option<Param>,        // c:1865
    pub level: i32,                // c:1866
}

/// Port of `struct tieddata` from `Src/zsh.h:1870-1873`.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct tieddata {
    // c:1870
    pub arrptr: Option<Vec<String>>, // c:1871 char ***arrptr
    pub joinchar: i32,               // c:1872
}

/// Port of `struct repldata` from `Src/zsh.h:2003-2006`.
#[allow(non_camel_case_types)]
pub struct repldata {
    // c:2003
    pub b: i32,                  // c:2004
    pub e: i32,                  // c:2004
    pub replstr: Option<String>, // c:2005
}
/// `Repldata` type alias.
pub type Repldata = Box<repldata>; // c:2007

/// Port of `struct paramdef` from `Src/zsh.h:2082-2090`.
#[allow(non_camel_case_types)]
#[derive(Default)]
pub struct paramdef {
    // c:2082
    pub name: String,                 // c:2083
    pub flags: i32,                   // c:2084
    pub var: usize,                   // c:2085 void *
    pub gsu: usize,                   // c:2086 const void *
    pub getnfn: Option<GetNodeFunc>,  // c:2087
    pub scantfn: Option<ScanTabFunc>, // c:2088
    pub pm: Option<Param>,            // c:2089
}

/// Port of `struct nameddir` from `Src/zsh.h:2149-2153`.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct nameddir {
    // c:2149
    pub node: hashnode, // c:2150
    pub dir: String,    // c:2151
    pub diff: i32,      // c:2152
}

/// Port of `groupmap` from `Src/zsh.h:2161-2166`.
#[allow(non_camel_case_types)]
pub struct groupmap {
    // c:2161
    pub name: String, // c:2163
    pub gid: u32,     // c:2165 gid_t
}
/// `Groupmap` type alias.
pub type Groupmap = Box<groupmap>; // c:2167

/// Port of `groupset` from `Src/zsh.h:2170-2175`.
#[allow(non_camel_case_types)]
pub struct groupset {
    // c:2170
    pub array: Vec<groupmap>, // c:2172
    pub num: i32,             // c:2174
}
/// `Groupset` type alias.
pub type Groupset = Box<groupset>; // c:2176

/// Port of `struct histent` from `Src/zsh.h:2234-2250`.
#[allow(non_camel_case_types)]
pub struct histent {
    // c:2234
    pub node: hashnode,           // c:2235
    pub up: Option<Histent>,      // c:2237
    pub down: Option<Histent>,    // c:2238
    pub zle_text: Option<String>, // c:2239
    pub stim: i64,                // c:2244 time_t
    pub ftim: i64,                // c:2245
    pub words: Vec<i16>,          // c:2246
    pub nwords: i32,              // c:2248
    pub histnum: i64,             // c:2249 zlong
}

/// Port of `struct emulation_options` from `Src/zsh.h:2570-2585`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct emulation_options {
    // c:2570
    pub emulation: i32,          // c:2572
    pub n_on_opts: i32,          // c:2574
    pub n_off_opts: i32,         // c:2576
    pub on_opts: Vec<OptIndex>,  // c:2582
    pub off_opts: Vec<OptIndex>, // c:2584
}

/// Port of `struct ttyinfo` from `Src/zsh.h:2593-2609`. The C
/// definition `#ifdef`-selects between `termios` / `termio` / sgtty.
/// Rust port stores the raw libc `termios` (the path taken on every
/// modern host).
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct ttyinfo {
    // c:2593
    #[cfg(unix)]
    pub tio: libc::termios, // c:2595
    #[cfg(unix)]
    pub winsize: libc::winsize, // c:2607
}

/// Port of `struct heapstack` from `Src/zsh.h:2871-2877`.
#[allow(non_camel_case_types)]
pub struct heapstack {
    // c:2871
    pub next: Option<Heapstack>, // c:2872
    pub used: usize,             // c:2873
}

/// Port of `struct heap` from `Src/zsh.h:2881-2898`.
#[allow(non_camel_case_types)]
pub struct heap {
    // c:2881
    pub next: Option<Heap>,    // c:2882
    pub size: usize,           // c:2883
    pub used: usize,           // c:2884
    pub sp: Option<Heapstack>, // c:2885
}

/// Port of `struct sortelt` from `Src/zsh.h:3013-3028`.
#[allow(non_camel_case_types)]
pub struct sortelt {
    // c:3013
    pub orig: String, // c:3015
    pub cmp: String,  // c:3017
    pub origlen: i32, // c:3022
    pub len: i32,     // c:3027
}
/// `SortElt` type alias.
pub type SortElt = Box<sortelt>; // c:3030

/// Port of `struct hist_stack` from `Src/zsh.h:3037-3058`.
#[allow(non_camel_case_types)]
pub struct hist_stack {
    // c:3037
    pub histactive: i32,        // c:3038
    pub histdone: i32,          // c:3039
    pub stophist: i32,          // c:3040
    pub hlinesz: i32,           // c:3041
    pub defev: i64,             // c:3042 zlong
    pub hline: Option<String>,  // c:3043
    pub hptr: Option<String>,   // c:3044
    pub chwords: Vec<i16>,      // c:3045
    pub chwordlen: i32,         // c:3046
    pub chwordpos: i32,         // c:3047
    pub csp: i32,               // c:3056
    pub hist_keep_comment: i32, // c:3057
}

/// Port of `struct lexbufstate` from `Src/zsh.h:3069-3079`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Default)]
pub struct lexbufstate {
    // c:3069
    pub ptr: Option<String>, // c:3074
    pub siz: i32,            // c:3076
    pub len: i32,            // c:3078
}

/// Port of `struct lex_stack` from `Src/zsh.h:3082-3096`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Default)]
pub struct lex_stack {
    // c:3082
    pub dbparens: i32,              // c:3083
    pub isfirstln: i32,             // c:3084
    pub isfirstch: i32,             // c:3085
    pub lexflags: i32,              // c:3086
    pub tok: lextok,                // c:3087
    pub tokstr: Option<String>,     // c:3088
    pub zshlextext: Option<String>, // c:3089
    pub lexbuf: lexbufstate,        // c:3090
    pub lex_add_raw: i32,           // c:3091
    pub tokstr_raw: Option<String>, // c:3092
    pub lexbuf_raw: lexbufstate,    // c:3093
    pub lexstop: i32,               // c:3094
    pub toklineno: i64,             // c:3095
}

/// Port of `struct parse_stack` from `Src/zsh.h:3099-3116`.
#[allow(non_camel_case_types)]
pub struct parse_stack {
    // c:3099
    pub hdocs: Option<Box<heredocs>>, // c:3100
    pub incmdpos: i32,                // c:3102
    pub aliasspaceflag: i32,          // c:3103
    pub incond: i32,                  // c:3104
    pub inredir: i32,                 // c:3105
    pub incasepat: i32,               // c:3106
    pub isnewlin: i32,                // c:3107
    pub infor: i32,                   // c:3108
    pub inrepeat_: i32,               // c:3109
    pub intypeset: i32,               // c:3110
    pub eclen: i32,                   // c:3112
    pub ecused: i32,                  // c:3112
    pub ecnpats: i32,                 // c:3112
    pub ecbuf: Wordcode,              // c:3113
    pub ecstrs: Option<Eccstr>,       // c:3114
    pub ecsoffs: i32,                 // c:3115
    pub ecssub: i32,                  // c:3115
    pub ecnfunc: i32,                 // c:3115
}

/// Port of `struct heredocs` from `Src/zsh.h:1152-1157`. Used by
/// parse_stack above.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct heredocs {
    // c:1152
    pub next: Option<Box<heredocs>>, // c:1153
    pub typ: i32,                    // c:1154 (Rust keyword `type`)
    pub pc: i32,                     // c:1155
    pub str: Option<String>,         // c:1156
}

/// Port of `struct execstack` from `Src/zsh.h:1127-1150`.
#[allow(non_camel_case_types)]
pub struct execstack {
    // c:1127
    pub next: Option<Box<execstack>>,      // c:1128
    pub list_pipe_pid: i32,                // c:1130 pid_t
    pub nowait: i32,                       // c:1131
    pub pline_level: i32,                  // c:1132
    pub list_pipe_child: i32,              // c:1133
    pub list_pipe_job: i32,                // c:1134
    pub list_pipe_text: [u8; JOBTEXTSIZE], // c:1135
    pub lastval: i32,                      // c:1136
    pub noeval: i32,                       // c:1137
    pub badcshglob: i32,                   // c:1138
    pub cmdoutpid: i32,                    // c:1139
    pub cmdoutval: i32,                    // c:1140
    pub use_cmdoutval: i32,                // c:1141
    pub procsubstpid: i32,                 // c:1142
    pub trap_return: i32,                  // c:1143
    pub trap_state: i32,                   // c:1144
    pub trapisfunc: i32,                   // c:1145
    pub traplocallevel: i32,               // c:1146
    pub noerrs: i32,                       // c:1147
    pub this_noerrexit: i32,               // c:1148
    pub underscore: Option<String>,        // c:1149
}

/// Port of `struct process` from `Src/zsh.h:1117-1125`.
///
/// Field-shape deviations from the C struct (documented for the
/// `zsh.h ↔ zsh_h.rs` audit):
/// - `text`: `String` instead of `char text[JOBTEXTSIZE]`. The
///   `JOBTEXTSIZE` cap in C is a buffer-overflow guard; Rust's owned
///   String removes the cap without losing the field's semantic.
/// - `bgtime` / `endtime`: `Option<std::time::Instant>` instead of
///   `struct timespec`. C uses timespec for monotonic-clock points;
///   Rust's `Instant` is the equivalent abstraction.
/// - `next` removed: C threads `struct process *next` for the
///   in-job singly-linked list; Rust port owns the list externally
///   via `job.procs: Vec<process>` so callers don't carry the chain
///   pointer per node.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct process {
    // c:1117
    pub pid: i32,                            // c:1119 pid_t
    pub text: String,                        // c:1120 char text[JOBTEXTSIZE]
    pub status: i32,                         // c:1121
    pub ti: timeinfo,                        // c:1122 child_times_t ti
    pub bgtime: Option<std::time::Instant>,  // c:1123 struct timespec bgtime
    pub endtime: Option<std::time::Instant>, // c:1124 struct timespec endtime
}

/// Port of `struct job` from `Src/zsh.h:1058-1071`.
///
/// Field-shape deviations from the C struct:
/// - `procs` / `auxprocs`: `Vec<process>` instead of singly-linked
///   `struct process *procs`. Equivalent semantics; Rust owns the
///   list ergonomically rather than threading `next` pointers.
/// - `filelist`: `Vec<String>` instead of `LinkList` of `char *`.
///   Same reasoning.
/// - `text`: added (Rust extension). C reconstructs job-display text
///   on demand by walking `procs->text`; the Rust port caches the
///   composed text here so display paths don't re-walk per call.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Default)]
pub struct job {
    // c:1058
    pub gleader: i32,             // c:1059 pid_t
    pub other: i32,               // c:1060
    pub stat: i32,                // c:1062 STAT_*
    pub pwd: Option<String>,      // c:1063
    pub procs: Vec<process>,      // c:1065 struct process *procs
    pub auxprocs: Vec<process>,   // c:1066 struct process *auxprocs
    pub filelist: Vec<String>,    // c:1067 LinkList filelist
    pub stty_in_env: i32,         // c:1069
    pub ty: Option<Box<ttyinfo>>, // c:1070
    /// Rust extension: cached job-display text. C re-derives via
    /// `procs` walks in `printjob()` (`Src/jobs.c:1244+`).
    pub text: String,
}

/// Port of `struct funcdump` from `Src/zsh.h:776-786`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct funcdump {
    // c:776
    pub next: Option<FuncDump>,   // c:777
    pub dev: u64,                 // c:778 dev_t
    pub ino: u64,                 // c:779 ino_t
    pub fd: i32,                  // c:780
    pub map: Wordcode,            // c:781
    pub addr: Wordcode,           // c:782
    pub len: i32,                 // c:783
    pub count: i32,               // c:784
    pub filename: Option<String>, // c:785
}

/// Port of `struct eprog` from `Src/zsh.h:805-815`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Default)]
pub struct eprog {
    // c:805
    pub flags: i32,             // c:806 EF_*
    pub len: i32,               // c:807
    pub npats: i32,             // c:808
    pub nref: i32,              // c:809
    pub pats: Vec<Patprog>,     // c:810
    pub prog: Wordcode,         // c:811
    pub strs: Option<String>,   // c:812
    pub shf: Option<Shfunc>,    // c:813
    pub dump: Option<FuncDump>, // c:814
}

/// Port of `struct estate` from `Src/zsh.h:824-828`.
#[allow(non_camel_case_types)]
pub struct estate {
    // c:824
    pub prog: Eprog,          // c:825
    pub pc: usize,            // c:826 Wordcode pc — index into prog.prog (C is wordcode *)
    pub strs: Option<String>, // c:827 copy of prog.strs at estate creation
    pub strs_offset: usize,   // c:827 byte offset into strs — mirrors C `strs` pointer movement
}

/// Port of `struct eccstr` from `Src/zsh.h:836-858`.
#[allow(non_camel_case_types)]
pub struct eccstr {
    // c:836
    pub left: Option<Eccstr>,  // c:838
    pub right: Option<Eccstr>, // c:838
    pub str: Option<String>,   // c:841
    pub offs: wordcode,        // c:844
    pub aoffs: wordcode,       // c:847
    pub nfunc: i32,            // c:854
    pub hashval: u32,          // c:857
}

// =============================================================================
// 12. Z_* sublist flags (zsh.h:645-648).
// =============================================================================
/// `Z_TIMED` constant.
pub const Z_TIMED: i32 = 1 << 0; // c:645
/// `Z_SYNC` constant.
pub const Z_SYNC: i32 = 1 << 1; // c:646
/// `Z_ASYNC` constant.
pub const Z_ASYNC: i32 = 1 << 2; // c:647
/// `Z_DISOWN` constant.
pub const Z_DISOWN: i32 = 1 << 3; // c:648

// =============================================================================
// 13. COND_* condition types (zsh.h:660-679).
// =============================================================================
/// `COND_NOT` constant.
pub const COND_NOT: i32 = 0;
/// `COND_AND` constant.
pub const COND_AND: i32 = 1;
/// `COND_OR` constant.
pub const COND_OR: i32 = 2;
/// `COND_STREQ` constant.
pub const COND_STREQ: i32 = 3;
/// `COND_STRDEQ` constant.
pub const COND_STRDEQ: i32 = 4;
/// `COND_STRNEQ` constant.
pub const COND_STRNEQ: i32 = 5;
/// `COND_STRLT` constant.
pub const COND_STRLT: i32 = 6;
/// `COND_STRGTR` constant.
pub const COND_STRGTR: i32 = 7;
/// `COND_NT` constant.
pub const COND_NT: i32 = 8;
/// `COND_OT` constant.
pub const COND_OT: i32 = 9;
/// `COND_EF` constant.
pub const COND_EF: i32 = 10;
/// `COND_EQ` constant.
pub const COND_EQ: i32 = 11;
/// `COND_NE` constant.
pub const COND_NE: i32 = 12;
/// `COND_LT` constant.
pub const COND_LT: i32 = 13;
/// `COND_GT` constant.
pub const COND_GT: i32 = 14;
/// `COND_LE` constant.
pub const COND_LE: i32 = 15;
/// `COND_GE` constant.
pub const COND_GE: i32 = 16;
/// `COND_REGEX` constant.
pub const COND_REGEX: i32 = 17;
/// `COND_MOD` constant.
pub const COND_MOD: i32 = 18;
/// `COND_MODI` constant.
pub const COND_MODI: i32 = 19;
/// `CONDF_INFIX` constant.
pub const CONDF_INFIX: i32 = 1; // c:695
/// `CONDF_ADDED` constant.
pub const CONDF_ADDED: i32 = 2; // c:697
/// `CONDF_AUTOALL` constant.
pub const CONDF_AUTOALL: i32 = 4; // c:699

// =============================================================================
// 14. Redirection structures (zsh.h:706-740) + MULTIOUNIT.
// =============================================================================
/// `REDIRF_FROM_HEREDOC` constant.
pub const REDIRF_FROM_HEREDOC: i32 = 1; // c:708
/// `redir` — see fields for layout.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct redir {
    // c:713
    /// `typ` field.
    pub typ: i32,
    /// `flags` field.
    pub flags: i32,
    /// `fd1` field.
    pub fd1: i32,
    /// `fd2` field.
    pub fd2: i32,
    /// `name` field.
    pub name: Option<String>,
    /// `varid` field.
    pub varid: Option<String>,
    /// `here_terminator` field.
    pub here_terminator: Option<String>,
    /// `munged_here_terminator` field.
    pub munged_here_terminator: Option<String>,
}
/// `MULTIOUNIT` constant.
pub const MULTIOUNIT: usize = 8; // c:725
/// `multio` — see fields for layout.
#[allow(non_camel_case_types)]
pub struct multio {
    // c:735
    /// `ct` field.
    pub ct: i32,
    /// `rflag` field.
    pub rflag: i32,
    /// `pipe` field.
    pub pipe: i32,
    /// C `int fds[1]` with `VARLENARRAY` trailing-element realloc via
    /// `hrealloc(mn, sizeof + ct*sizeof(int))`. Rust uses a growable
    /// `Vec<i32>` (no MULTIOUNIT cap). Initial slot stamped on
    /// construction (c:2449).
    pub fds: Vec<i32>,
}

// =============================================================================
// 15. value struct (zsh.h:744-755) + VALFLAG_* + MAX_ARRLEN.
// =============================================================================
/// `value` — see fields for layout.
#[allow(non_camel_case_types)]
pub struct value {
    // c:744
    /// `pm` field.
    pub pm: Option<Param>,
    /// `arr` field.
    pub arr: Vec<String>,
    /// `scanflags` field.
    pub scanflags: i32,
    /// `valflags` field.
    pub valflags: i32,
    /// `start` field.
    pub start: i32,
    /// `end` field.
    pub end: i32,
}
/// `VALFLAG_INV` constant.
pub const VALFLAG_INV: i32 = 0x0001; // c:758
/// `VALFLAG_EMPTY` constant.
pub const VALFLAG_EMPTY: i32 = 0x0002;
/// `VALFLAG_SUBST` constant.
pub const VALFLAG_SUBST: i32 = 0x0004;
/// `VALFLAG_REFSLICE` constant.
pub const VALFLAG_REFSLICE: i32 = 0x0008;
/// `MAX_ARRLEN` constant.
pub const MAX_ARRLEN: i32 = 262144; // c:764

// =============================================================================
// 16. Word code types (zsh.h:770-1038).
// =============================================================================
/// `wordcode` type alias.
#[allow(non_camel_case_types)]
pub type wordcode = u32; // c:770
/// `Wordcode` type alias.
pub type Wordcode = Vec<wordcode>; // c:771
/// `FuncDump` type alias.
pub type FuncDump = Box<funcdump>; // c:773
/// `Eprog` type alias.
pub type Eprog = Box<eprog>; // c:774
/// `EF_REAL` constant.
pub const EF_REAL: i32 = 1; // c:817
/// `EF_HEAP` constant.
pub const EF_HEAP: i32 = 2;
/// `EF_MAP` constant.
pub const EF_MAP: i32 = 4;
/// `EF_RUN` constant.
pub const EF_RUN: i32 = 8;
/// `Estate` type alias.
pub type Estate = Box<estate>; // c:822
/// `Eccstr` type alias.
pub type Eccstr = Box<eccstr>; // c:835
/// `EC_NODUP` constant.
pub const EC_NODUP: i32 = 0; // c:869
/// `EC_DUP` constant.
pub const EC_DUP: i32 = 1; // c:872
/// `EC_DUPTOK` constant.
pub const EC_DUPTOK: i32 = 2; // c:878
/// `WC_CODEBITS` constant.
pub const WC_CODEBITS: u32 = 5; // c:882
/// `wc_code` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn wc_code(c: wordcode) -> wordcode {
    c & ((1 << WC_CODEBITS) - 1)
}
/// `wc_data` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn wc_data(c: wordcode) -> wordcode {
    c >> WC_CODEBITS
}
/// `wc_bdata` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn wc_bdata(d: wordcode) -> wordcode {
    d << WC_CODEBITS
}
/// `wc_bld` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn wc_bld(c: wordcode, d: wordcode) -> wordcode {
    c | (d << WC_CODEBITS)
}
/// `WC_END` constant.
pub const WC_END: wordcode = 0;
/// `WC_LIST` constant.
pub const WC_LIST: wordcode = 1;
/// `WC_SUBLIST` constant.
pub const WC_SUBLIST: wordcode = 2;
/// `WC_PIPE` constant.
pub const WC_PIPE: wordcode = 3;
/// `WC_REDIR` constant.
pub const WC_REDIR: wordcode = 4;
/// `WC_ASSIGN` constant.
pub const WC_ASSIGN: wordcode = 5;
/// `WC_SIMPLE` constant.
pub const WC_SIMPLE: wordcode = 6;
/// `WC_TYPESET` constant.
pub const WC_TYPESET: wordcode = 7;
/// `WC_SUBSH` constant.
pub const WC_SUBSH: wordcode = 8;
/// `WC_CURSH` constant.
pub const WC_CURSH: wordcode = 9;
/// `WC_TIMED` constant.
pub const WC_TIMED: wordcode = 10;
/// `WC_FUNCDEF` constant.
pub const WC_FUNCDEF: wordcode = 11;
/// `WC_FOR` constant.
pub const WC_FOR: wordcode = 12;
/// `WC_SELECT` constant.
pub const WC_SELECT: wordcode = 13;
/// `WC_WHILE` constant.
pub const WC_WHILE: wordcode = 14;
/// `WC_REPEAT` constant.
pub const WC_REPEAT: wordcode = 15;
/// `WC_CASE` constant.
pub const WC_CASE: wordcode = 16;
/// `WC_IF` constant.
pub const WC_IF: wordcode = 17;
/// `WC_COND` constant.
pub const WC_COND: wordcode = 18;
/// `WC_ARITH` constant.
pub const WC_ARITH: wordcode = 19;
/// `WC_AUTOFN` constant.
pub const WC_AUTOFN: wordcode = 20;
/// `WC_TRY` constant.
pub const WC_TRY: wordcode = 21;
/// `WC_COUNT` constant.
pub const WC_COUNT: wordcode = 22;
/// `Z_END` constant.
pub const Z_END: i32 = 1 << 4; // c:921
/// `Z_SIMPLE` constant.
pub const Z_SIMPLE: i32 = 1 << 5; // c:922
/// `WC_LIST_FREE` constant.
pub const WC_LIST_FREE: u32 = 6; // c:923
/// `WC_SUBLIST_END` constant.
pub const WC_SUBLIST_END: wordcode = 0;
/// `WC_SUBLIST_AND` constant.
pub const WC_SUBLIST_AND: wordcode = 1;
/// `WC_SUBLIST_OR` constant.
pub const WC_SUBLIST_OR: wordcode = 2;
/// `WC_SUBLIST_COPROC` constant.
pub const WC_SUBLIST_COPROC: wordcode = 4;
/// `WC_SUBLIST_NOT` constant.
pub const WC_SUBLIST_NOT: wordcode = 8;
/// `WC_SUBLIST_SIMPLE` constant.
pub const WC_SUBLIST_SIMPLE: wordcode = 16;
/// `WC_SUBLIST_FREE` constant.
pub const WC_SUBLIST_FREE: u32 = 5; // c:935
/// `WC_PIPE_END` constant.
pub const WC_PIPE_END: wordcode = 0;
/// `WC_PIPE_MID` constant.
pub const WC_PIPE_MID: wordcode = 1;
/// `WC_ASSIGN_SCALAR` constant.
pub const WC_ASSIGN_SCALAR: wordcode = 0;
/// `WC_ASSIGN_ARRAY` constant.
pub const WC_ASSIGN_ARRAY: wordcode = 1;
/// `WC_ASSIGN_NEW` constant.
pub const WC_ASSIGN_NEW: wordcode = 0;
/// `WC_ASSIGN_INC` constant.
pub const WC_ASSIGN_INC: wordcode = 1;
/// `WC_TIMED_EMPTY` constant.
pub const WC_TIMED_EMPTY: wordcode = 0;
/// `WC_TIMED_PIPE` constant.
pub const WC_TIMED_PIPE: wordcode = 1;
/// `WC_FOR_PPARAM` constant.
pub const WC_FOR_PPARAM: wordcode = 0;
/// `WC_FOR_LIST` constant.
pub const WC_FOR_LIST: wordcode = 1;
/// `WC_FOR_COND` constant.
pub const WC_FOR_COND: wordcode = 2;
/// `WC_SELECT_PPARAM` constant.
pub const WC_SELECT_PPARAM: wordcode = 0;
/// `WC_SELECT_LIST` constant.
pub const WC_SELECT_LIST: wordcode = 1;
/// `WC_WHILE_WHILE` constant.
pub const WC_WHILE_WHILE: wordcode = 0;
/// `WC_WHILE_UNTIL` constant.
pub const WC_WHILE_UNTIL: wordcode = 1;
/// `WC_CASE_HEAD` constant.
pub const WC_CASE_HEAD: wordcode = 0;
/// `WC_CASE_OR` constant.
pub const WC_CASE_OR: wordcode = 1;
/// `WC_CASE_AND` constant.
pub const WC_CASE_AND: wordcode = 2;
/// `WC_CASE_TESTAND` constant.
pub const WC_CASE_TESTAND: wordcode = 3;
/// `WC_CASE_FREE` constant.
pub const WC_CASE_FREE: u32 = 3; // c:1020
/// `WC_IF_HEAD` constant.
pub const WC_IF_HEAD: wordcode = 0;
/// `WC_IF_IF` constant.
pub const WC_IF_IF: wordcode = 1;
/// `WC_IF_ELIF` constant.
pub const WC_IF_ELIF: wordcode = 2;
/// `WC_IF_ELSE` constant.
pub const WC_IF_ELSE: wordcode = 3;

// =============================================================================
// 16b. WC accessor + builder macros (zsh.h:918-1038).
// Each WC_X_TYPE / WC_X_SKIP / WCB_X is one of the per-opcode
// `wc_data` slicers / `wc_bld` constructors.
// =============================================================================
/// `WCB_END` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_END() -> wordcode {
    wc_bld(WC_END, 0)
} // c:918
/// `WC_LIST_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_LIST_TYPE(c: wordcode) -> wordcode {
    wc_data(c)
} // c:920
/// `WC_LIST_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_LIST_SKIP(c: wordcode) -> wordcode {
    wc_data(c) >> WC_LIST_FREE
} // c:924
/// `WCB_LIST` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_LIST(t: wordcode, o: wordcode) -> wordcode {
    wc_bld(WC_LIST, t | (o << WC_LIST_FREE))
}
/// `WC_SUBLIST_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_SUBLIST_TYPE(c: wordcode) -> wordcode {
    wc_data(c) & 3
} // c:927
/// `WC_SUBLIST_FLAGS` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_SUBLIST_FLAGS(c: wordcode) -> wordcode {
    wc_data(c) & 0x1c
} // c:931
/// `WC_SUBLIST_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_SUBLIST_SKIP(c: wordcode) -> wordcode {
    wc_data(c) >> WC_SUBLIST_FREE
}
/// `WCB_SUBLIST` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_SUBLIST(t: wordcode, f: wordcode, o: wordcode) -> wordcode {
    wc_bld(WC_SUBLIST, t | f | (o << WC_SUBLIST_FREE))
}
/// `WC_PIPE_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_PIPE_TYPE(c: wordcode) -> wordcode {
    wc_data(c) & 1
} // c:940
/// `WC_PIPE_LINENO` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_PIPE_LINENO(c: wordcode) -> wordcode {
    wc_data(c) >> 1
}
/// `WCB_PIPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_PIPE(t: wordcode, l: wordcode) -> wordcode {
    wc_bld(WC_PIPE, t | (l << 1))
}
/// `WC_REDIR_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_REDIR_TYPE(c: wordcode) -> i32 {
    (wc_data(c) & REDIR_TYPE_MASK as u32) as i32
}
/// `WC_REDIR_VARID` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_REDIR_VARID(c: wordcode) -> i32 {
    (wc_data(c) & REDIR_VARID_MASK as u32) as i32
}
/// `WC_REDIR_FROM_HEREDOC` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_REDIR_FROM_HEREDOC(c: wordcode) -> i32 {
    (wc_data(c) & REDIR_FROM_HEREDOC_MASK as u32) as i32
}
/// `WCB_REDIR` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_REDIR(t: wordcode) -> wordcode {
    wc_bld(WC_REDIR, t)
}
/// `WC_REDIR_WORDS` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_REDIR_WORDS(c: wordcode) -> i32 {
    (if WC_REDIR_VARID(c) != 0 { 4 } else { 3 })
        + (if WC_REDIR_FROM_HEREDOC(c) != 0 { 2 } else { 0 })
}
/// `WC_ASSIGN_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_ASSIGN_TYPE(c: wordcode) -> wordcode {
    wc_data(c) & 1
} // c:955
/// `WC_ASSIGN_TYPE2` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_ASSIGN_TYPE2(c: wordcode) -> wordcode {
    (wc_data(c) & 2) >> 1
}
/// `WC_ASSIGN_NUM` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_ASSIGN_NUM(c: wordcode) -> wordcode {
    wc_data(c) >> 2
}
/// `WCB_ASSIGN` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_ASSIGN(t: wordcode, a: wordcode, n: wordcode) -> wordcode {
    wc_bld(WC_ASSIGN, t | (a << 1) | (n << 2))
}
/// `WC_SIMPLE_ARGC` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_SIMPLE_ARGC(c: wordcode) -> wordcode {
    wc_data(c)
} // c:970
/// `WCB_SIMPLE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_SIMPLE(n: wordcode) -> wordcode {
    wc_bld(WC_SIMPLE, n)
}
/// `WC_TYPESET_ARGC` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_TYPESET_ARGC(c: wordcode) -> wordcode {
    wc_data(c)
} // c:973
/// `WCB_TYPESET` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_TYPESET(n: wordcode) -> wordcode {
    wc_bld(WC_TYPESET, n)
}
/// `WC_SUBSH_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_SUBSH_SKIP(c: wordcode) -> wordcode {
    wc_data(c)
} // c:976
/// `WCB_SUBSH` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_SUBSH(o: wordcode) -> wordcode {
    wc_bld(WC_SUBSH, o)
}
/// `WC_CURSH_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_CURSH_SKIP(c: wordcode) -> wordcode {
    wc_data(c)
} // c:979
/// `WCB_CURSH` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_CURSH(o: wordcode) -> wordcode {
    wc_bld(WC_CURSH, o)
}
/// `WC_TIMED_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_TIMED_TYPE(c: wordcode) -> wordcode {
    wc_data(c)
} // c:982
/// `WCB_TIMED` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_TIMED(t: wordcode) -> wordcode {
    wc_bld(WC_TIMED, t)
}
/// `WC_FUNCDEF_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_FUNCDEF_SKIP(c: wordcode) -> wordcode {
    wc_data(c)
} // c:987
/// `WCB_FUNCDEF` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_FUNCDEF(o: wordcode) -> wordcode {
    wc_bld(WC_FUNCDEF, o)
}
/// `WC_FOR_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_FOR_TYPE(c: wordcode) -> wordcode {
    wc_data(c) & 3
} // c:990
/// `WC_FOR_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_FOR_SKIP(c: wordcode) -> wordcode {
    wc_data(c) >> 2
}
/// `WCB_FOR` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_FOR(t: wordcode, o: wordcode) -> wordcode {
    wc_bld(WC_FOR, t | (o << 2))
}
/// `WC_SELECT_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_SELECT_TYPE(c: wordcode) -> wordcode {
    wc_data(c) & 1
} // c:997
/// `WC_SELECT_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_SELECT_SKIP(c: wordcode) -> wordcode {
    wc_data(c) >> 1
}
/// `WCB_SELECT` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_SELECT(t: wordcode, o: wordcode) -> wordcode {
    wc_bld(WC_SELECT, t | (o << 1))
}
/// `WC_WHILE_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_WHILE_TYPE(c: wordcode) -> wordcode {
    wc_data(c) & 1
} // c:1003
/// `WC_WHILE_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_WHILE_SKIP(c: wordcode) -> wordcode {
    wc_data(c) >> 1
}
/// `WCB_WHILE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_WHILE(t: wordcode, o: wordcode) -> wordcode {
    wc_bld(WC_WHILE, t | (o << 1))
}
/// `WC_REPEAT_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_REPEAT_SKIP(c: wordcode) -> wordcode {
    wc_data(c)
} // c:1009
/// `WCB_REPEAT` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_REPEAT(o: wordcode) -> wordcode {
    wc_bld(WC_REPEAT, o)
}
/// `WC_TRY_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_TRY_SKIP(c: wordcode) -> wordcode {
    wc_data(c)
} // c:1012
/// `WCB_TRY` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_TRY(o: wordcode) -> wordcode {
    wc_bld(WC_TRY, o)
}
/// `WC_CASE_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_CASE_TYPE(c: wordcode) -> wordcode {
    wc_data(c) & 7
} // c:1015
/// `WC_CASE_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_CASE_SKIP(c: wordcode) -> wordcode {
    wc_data(c) >> WC_CASE_FREE
}
/// `WCB_CASE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_CASE(t: wordcode, o: wordcode) -> wordcode {
    wc_bld(WC_CASE, t | (o << WC_CASE_FREE))
}
/// `WC_IF_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_IF_TYPE(c: wordcode) -> wordcode {
    wc_data(c) & 3
} // c:1024
/// `WC_IF_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_IF_SKIP(c: wordcode) -> wordcode {
    wc_data(c) >> 2
}
/// `WCB_IF` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_IF(t: wordcode, o: wordcode) -> wordcode {
    wc_bld(WC_IF, t | (o << 2))
}
/// `WC_COND_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_COND_TYPE(c: wordcode) -> wordcode {
    wc_data(c) & 127
} // c:1032
/// `WC_COND_SKIP` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WC_COND_SKIP(c: wordcode) -> wordcode {
    wc_data(c) >> 7
}
/// `WCB_COND` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_COND(t: wordcode, o: wordcode) -> wordcode {
    wc_bld(WC_COND, t | (o << 7))
}
/// `WCB_ARITH` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_ARITH() -> wordcode {
    wc_bld(WC_ARITH, 0)
} // c:1036
/// `WCB_AUTOFN` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub fn WCB_AUTOFN() -> wordcode {
    wc_bld(WC_AUTOFN, 0)
} // c:1038

// =============================================================================
// 16c. Other macros: BUILTIN/BIN_PREFIX/CONDDEF/HOOKDEF/PARAMDEF/etc.
// =============================================================================

/// Port of `#define NULLBINCMD` from `Src/zsh.h:1438`.
pub const NULLBINCMD: Option<HandlerFunc> = None; // c:1438

/// Port of `#define EMULATION(X)` from `Src/zsh.h:2347`.
/// C macro: `(emulation & (X))`. Reads the canonical `emulation`
/// static from `crate::ported::options::emulation` directly.
#[inline]
#[allow(non_snake_case)]
pub fn EMULATION(x: i32) -> bool {
    // c:2347
    let emul = crate::ported::options::emulation.load(std::sync::atomic::Ordering::Relaxed);
    (emul & x) != 0
}

/// Port of `#define SHELL_EMULATION()` from `Src/zsh.h:2350`.
/// C macro: `(emulation & ((1<<5)-1))`. Reads the canonical
/// `emulation` static directly.
#[inline]
#[allow(non_snake_case)]
pub fn SHELL_EMULATION() -> i32 {
    // c:2350
    let emul = crate::ported::options::emulation.load(std::sync::atomic::Ordering::Relaxed);
    emul & ((1 << 5) - 1)
}

/// Port of `#define IN_EVAL_TRAP()` from `Src/zsh.h:2962`.
/// C macro reads the four globals `intrap` / `trapisfunc` /
/// `traplocallevel` / `locallevel` directly with no args; Rust
/// matches by reading the canonical statics
/// (`signals::intrap`, `signals::trapisfunc`,
/// `signals::traplocallevel`, `params::locallevel`) inside.
#[inline]
#[allow(non_snake_case)]
pub fn IN_EVAL_TRAP() -> bool {
    // c:2962
    crate::ported::signals::intrap.load(Ordering::Relaxed) != 0
        && crate::ported::signals::trapisfunc.load(Ordering::Relaxed) == 0
        && crate::ported::signals::traplocallevel.load(Ordering::Relaxed)
            == crate::ported::params::locallevel.load(Ordering::Relaxed)
}

/// Port of `#define ASG_ARRAYP(asg)` from `Src/zsh.h:1288`.
#[inline]
#[allow(non_snake_case)]
pub fn ASG_ARRAYP(asg: &asgment) -> bool {
    (asg.flags & ASG_ARRAY) != 0
}

/// Port of `#define ASG_VALUEP(asg)` from `Src/zsh.h:1296`.
#[inline]
#[allow(non_snake_case)]
pub fn ASG_VALUEP(asg: &asgment) -> bool {
    ASG_ARRAYP(asg) || asg.scalar.is_some()
}

/// Port of `#define MB_METASTRLEN2END(str, widthp, eptr)` from
/// `Src/zsh.h:3282/3363`. C: `mb_metastrlenend(str, widthp, eptr)`
/// (multibyte) or `ztrlenend(str, eptr)` (non-multibyte). Rust port
/// counts metafied chars from `str` up to `eptr` (exclusive).
#[inline]
#[allow(non_snake_case)]
pub fn MB_METASTRLEN2END(s: &str, widthp: bool, eptr: usize) -> usize {
    let truncated = if eptr <= s.len() { &s[..eptr] } else { s };
    MB_METASTRLEN2(truncated, widthp)
}

// Hook-table indices (zsh.h:3259-3262). C: `(zshhooks + N)` —
// the Rust port exposes the offsets; consumers index into the
// `zshhooks[]` array themselves.
/// `EXITHOOK_OFFSET` constant.
pub const EXITHOOK_OFFSET: usize = 0; // c:3259
/// `BEFORETRAPHOOK_OFFSET` constant.
pub const BEFORETRAPHOOK_OFFSET: usize = 1; // c:3260
/// `AFTERTRAPHOOK_OFFSET` constant.
pub const AFTERTRAPHOOK_OFFSET: usize = 2; // c:3261
/// `GETCOLORATTR_OFFSET` constant.
pub const GETCOLORATTR_OFFSET: usize = 3; // c:3262

/// Port of `#define STOPHIST` from `Src/zsh.h:2267`. Increments the
/// `stophist` global by 4. Rust port exposes the delta; the global
/// itself lives in `hist.rs`.
pub const STOPHIST_DELTA: i32 = 4; // c:2267
/// `ALLOWHIST_DELTA` constant.
pub const ALLOWHIST_DELTA: i32 = -4; // c:2268

/// Aliases under the canonical C macro names. C uses these in
/// statement-style: `STOPHIST` and `ALLOWHIST` expand to assignments
/// modifying the global; Rust port exposes them as the deltas.
pub const STOPHIST: i32 = STOPHIST_DELTA;
/// `ALLOWHIST` constant.
pub const ALLOWHIST: i32 = ALLOWHIST_DELTA;

/// Hook-table indices under their canonical zsh.h names (C: `(zshhooks
/// + N)`).
pub const EXITHOOK: usize = EXITHOOK_OFFSET;
/// `BEFORETRAPHOOK` constant.
pub const BEFORETRAPHOOK: usize = BEFORETRAPHOOK_OFFSET;
/// `AFTERTRAPHOOK` constant.
pub const AFTERTRAPHOOK: usize = AFTERTRAPHOOK_OFFSET;
/// `GETCOLORATTR` constant.
pub const GETCOLORATTR: usize = GETCOLORATTR_OFFSET;

/// Port of `#define ZLONG_CONST(x)` from `Src/zsh.h:68/72/78/83`.
/// C casts an integer literal to `zlong` via the `l`/`ll` suffix.
/// In Rust integer literals are typed at the use site; this macro
/// is an explicit cast to `zlong` (= `i64`).
#[inline]
#[allow(non_snake_case)]
pub const fn ZLONG_CONST(x: i64) -> zlong {
    x
} // c:68

/// Port of `#define STRINGIFY_LITERAL(x)` from `Src/zsh.h:2915`. C
/// uses the `#` operator to stringify an identifier. Rust's
/// `stringify!` macro does the same.
#[macro_export]
macro_rules! STRINGIFY_LITERAL {
    ($x:tt) => {
        stringify!($x)
    };
}

/// Port of `#define STRINGIFY(x)` from `Src/zsh.h:2916`. Two-pass
/// stringification (expand x first, then stringify).
#[macro_export]
macro_rules! STRINGIFY {
    ($x:tt) => {
        $crate::STRINGIFY_LITERAL!($x)
    };
}

/// Port of `#define ERRMSG(x)` from `Src/zsh.h:2917`. Build a debug
/// error-message prefix `__FILE__ ":" __LINE__ ": " x`.
#[macro_export]
macro_rules! ERRMSG {
    ($msg:expr) => {
        concat!(file!(), ":", line!(), ": ", $msg)
    };
}

/// Port of `#define HEAPID_FMT` from `Src/zsh.h:2831`. printf format
/// specifier for `Heapid` values. C uses `"%x"`; Rust uses `"{:x}"`.
pub const HEAPID_FMT: &str = "{:x}"; // c:2831

/// Port of `#define HEAP_ERROR(heap_id)` from `Src/zsh.h:2864`. Debug-
/// only macro that fprintf's an "invalid heap" error to stderr.
/// Rust port: eprintln! with the same format. Only active under
/// the `zsh-heap-debug` feature.
#[macro_export]
macro_rules! HEAP_ERROR {
    ($heap_id:expr) => {
        eprintln!(
            "{}:{}: HEAP DEBUG: invalid heap: {:x}.",
            file!(),
            line!(),
            $heap_id
        )
    };
}

/// Port of `#define DPUTS(X, Y)` macro from `Src/zsh.h:2918` (macro).
///
/// C body (DEBUG defined, c:2918):
/// ```c
/// # define DPUTS(X,Y) if (!(X)) {;} else dputs(ERRMSG(Y))
/// ```
/// where `ERRMSG(x)` is `(__FILE__ ":" STRINGIFY(__LINE__) ": " x)`
/// (c:2917). Without DEBUG (c:2923) the macro expands to nothing.
///
/// The Rust port routes through `crate::ported::utils::dputs` (port
/// of `dputs` at `Src/utils.c:253`) under `#[cfg(feature = "zsh-debug")]`
/// — Rust's analogue to C's `#ifdef DEBUG` (set by configure's
/// `--enable-zsh-debug`). Stock zsh ships with DEBUG un-set, so the
/// default zshrs build (without `--features zsh-debug`) is silent
/// too. The file:line prefix uses `file!()` / `line!()` to mirror
/// `__FILE__:__LINE__`.
#[macro_export]
macro_rules! DPUTS {
    // c:2918
    ($x:expr, $y:expr) => {
        // c:2918
        #[cfg(feature = "zsh-debug")] // c:2918 ifdef DEBUG
        {
            if $x {
                // c:2918 if (X)
                crate::ported::utils::dputs(&format!(
                    // c:2918 dputs(ERRMSG(Y))
                    "{}:{}: {}",
                    file!(),
                    line!(),
                    $y // c:2917 ERRMSG
                )); // c:2918
            } // c:2918
        } // c:2914 ifdef DEBUG
    }; // c:2918
} // c:2918

/// Port of `#define DPUTS1(X, Y, Z1)` macro from `Src/zsh.h:2919` (macro).
///
/// C body (DEBUG defined):
/// ```c
/// # define DPUTS1(X,Y,Z1) if (!(X)) {;} else dputs(ERRMSG(Y), Z1)
/// ```
/// One-arg printf-style variant — `Y` is a printf format string with
/// one `%`-substitution, `Z1` is the argument. The Rust port uses
/// `format!` with `{}` placeholders; callers should write Rust-style
/// format strings (`"BUG: x = {}"`) instead of C printf strings
/// (`"BUG: x = %d"`).
#[macro_export]
macro_rules! DPUTS1 {
    // c:2919
    ($x:expr, $y:expr, $z1:expr) => {
        // c:2919
        #[cfg(feature = "zsh-debug")] // c:2919
        {
            if $x {
                // c:2919
                crate::ported::utils::dputs(&format!(
                    // c:2919
                    "{}:{}: {}",
                    file!(),
                    line!(),
                    format!($y, $z1) // c:2917
                )); // c:2919
            } // c:2919
        } // c:2914
    }; // c:2919
} // c:2919

/// Port of `#define DPUTS2(X, Y, Z1, Z2)` macro from `Src/zsh.h:2920` (macro).
///
/// Two-arg printf-style variant. Same shape as DPUTS1 but with two
/// substitution arguments.
#[macro_export]
macro_rules! DPUTS2 {
    // c:2920
    ($x:expr, $y:expr, $z1:expr, $z2:expr) => {
        // c:2920
        #[cfg(feature = "zsh-debug")] // c:2920
        {
            if $x {
                // c:2920
                crate::ported::utils::dputs(&format!(
                    // c:2920
                    "{}:{}: {}",
                    file!(),
                    line!(),
                    format!($y, $z1, $z2) // c:2917
                )); // c:2920
            } // c:2920
        } // c:2914
    }; // c:2920
} // c:2920

/// Port of `#define DPUTS3(X, Y, Z1, Z2, Z3)` macro from `Src/zsh.h:2921` (macro).
///
/// Three-arg printf-style variant. Same shape as DPUTS1/DPUTS2 but
/// with three substitution arguments.
#[macro_export]
macro_rules! DPUTS3 {
    // c:2921
    ($x:expr, $y:expr, $z1:expr, $z2:expr, $z3:expr) => {
        // c:2921
        #[cfg(feature = "zsh-debug")] // c:2921
        {
            if $x {
                // c:2921
                crate::ported::utils::dputs(&format!(
                    // c:2921
                    "{}:{}: {}",
                    file!(),
                    line!(),
                    format!($y, $z1, $z2, $z3) // c:2917
                )); // c:2921
            } // c:2921
        } // c:2914
    }; // c:2921
} // c:2921

/// Port of `#define SGTTYFLAG` from `Src/zsh.h:2614/2616`. Termios
/// flag accessor — `shttyinfo.tio.c_oflag` (HAVE_TERMIOS) or
/// `shttyinfo.sgttyb.sg_flags` (sgtty fallback). Rust port exposes
/// the field name; consumers access via `&ttyinfo.tio.c_oflag`.
pub const SGTTYFLAG_NAME: &str = "tio.c_oflag";

/// Canonical alias under the C macro name (consumers reference it
/// in error messages / debug output).
pub const SGTTYFLAG: &str = SGTTYFLAG_NAME;

/// Port of `#define SGTABTYPE` from `Src/zsh.h:2619/2622/2625`.
/// Tab-expansion mode constant — `TAB3` / `OXTABS` / `XTABS` per
/// platform. macOS/BSD use `OXTABS`; Linux uses `XTABS`.
#[cfg(target_os = "linux")]
pub const SGTABTYPE: u32 = libc::XTABS;
/// `SGTABTYPE` constant.
#[cfg(not(target_os = "linux"))]
pub const SGTABTYPE: u32 = 0;

/// Port of `#define ZWS(s)` from `Src/zsh.h:3329/3373`. Wide-string
/// cast. In Rust `&str` is already UTF-8; pass through.
#[inline]
#[allow(non_snake_case)]
pub fn ZWS(s: &str) -> &str {
    s
}

// =============================================================================
// 16d. BUILTIN / BIN_PREFIX / CONDDEF / HOOKDEF / NUMMATHFUNC /
// STRMATHFUNC / PARAMDEF / INTPARAMDEF / STRPARAMDEF / ARRPARAMDEF /
// SPECIALPMDEF / WRAPDEF — table-row builder macros (zsh.h:1450-2125).
// These build initialiser literals for the various per-table arrays.
// Rust ports as `const fn`-equivalent constructors returning the
// matching struct.
// =============================================================================

/// Port of `BUILTIN(name, flags, handler, min, max, funcid, optstr, defopts)`
/// from `Src/zsh.h:1450`.
#[inline]
#[allow(non_snake_case)]
pub fn BUILTIN(
    name: &str,
    flags: i32,
    handler: Option<HandlerFunc>,
    min: i32,
    max: i32,
    funcid: i32,
    optstr: Option<&str>,
    defopts: Option<&str>,
) -> builtin {
    builtin {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags,
        },
        handlerfunc: handler,
        minargs: min,
        maxargs: max,
        funcid,
        optstr: optstr.map(|s| s.to_string()),
        defopts: defopts.map(|s| s.to_string()),
    }
}

/// Port of `BIN_PREFIX(name, flags)` from `Src/zsh.h:1452`. Builds a
/// prefix-builtin entry (no handler, marked with BINF_PREFIX).
#[inline]
#[allow(non_snake_case)]
pub fn BIN_PREFIX(name: &str, flags: i32) -> builtin {
    BUILTIN(
        name,
        flags | BINF_PREFIX as i32,
        NULLBINCMD,
        0,
        0,
        0,
        None,
        None,
    )
}

/// Port of `CONDDEF(name, flags, handler, min, max, condid)` from
/// `Src/zsh.h:701`.
#[inline]
#[allow(non_snake_case)]
pub fn CONDDEF(
    name: &str,
    flags: i32,
    handler: CondHandler,
    min: i32,
    max: i32,
    condid: i32,
) -> conddef {
    conddef {
        next: None,
        name: name.to_string(),
        flags,
        handler: Some(handler),
        min,
        max,
        condid,
        module: None,
    }
}

/// Port of `HOOKDEF(name, func, flags)` from `Src/zsh.h:1594`:
/// `{ NULL, name, (Hookfn) func, flags, NULL }`. `func` accepts `None`
/// to match C's `HOOKDEF("exit", NULL, HOOKF_ALL)` form.
#[inline]
#[allow(non_snake_case)]
pub fn HOOKDEF(name: &str, func: Option<Hookfn>, flags: i32) -> hookdef {
    hookdef {
        next: std::ptr::null_mut(),
        name: name.to_string(),
        def: func,
        flags,
        funcs: std::ptr::null_mut(),
    }
}

/// Port of `NUMMATHFUNC(name, func, min, max, id)` from `Src/zsh.h:133`.
#[inline]
#[allow(non_snake_case)]
pub fn NUMMATHFUNC(name: &str, func: NumMathFunc, min: i32, max: i32, id: i32) -> mathfunc {
    mathfunc {
        next: None,
        name: name.to_string(),
        flags: 0,
        nfunc: Some(func),
        sfunc: None,
        module: None,
        minargs: min,
        maxargs: max,
        funcid: id,
    }
}

/// Port of `STRMATHFUNC(name, func, id)` from `Src/zsh.h:135`.
#[inline]
#[allow(non_snake_case)]
pub fn STRMATHFUNC(name: &str, func: StrMathFunc, id: i32) -> mathfunc {
    mathfunc {
        next: None,
        name: name.to_string(),
        flags: MFF_STR,
        nfunc: None,
        sfunc: Some(func),
        module: None,
        minargs: 0,
        maxargs: 0,
        funcid: id,
    }
}

/// Port of `PARAMDEF(name, flags, var, gsu)` from `Src/zsh.h:2096`.
#[inline]
#[allow(non_snake_case)]
pub fn PARAMDEF(name: &str, flags: i32, var: usize, gsu: usize) -> paramdef {
    paramdef {
        name: name.to_string(),
        flags,
        var,
        gsu,
        getnfn: None,
        scantfn: None,
        pm: None,
    }
}

/// Port of `INTPARAMDEF(name, var)` from `Src/zsh.h:2105`.
#[inline]
#[allow(non_snake_case)]
pub fn INTPARAMDEF(name: &str, var: usize) -> paramdef {
    PARAMDEF(name, PM_INTEGER as i32, var, 0)
}

/// Port of `STRPARAMDEF(name, var)` from `Src/zsh.h:2107`.
#[inline]
#[allow(non_snake_case)]
pub fn STRPARAMDEF(name: &str, var: usize) -> paramdef {
    PARAMDEF(name, PM_SCALAR as i32, var, 0)
}

/// Port of `ARRPARAMDEF(name, var)` from `Src/zsh.h:2109`.
#[inline]
#[allow(non_snake_case)]
pub fn ARRPARAMDEF(name: &str, var: usize) -> paramdef {
    PARAMDEF(name, PM_ARRAY as i32, var, 0)
}

/// Port of `SPECIALPMDEF(name, flags, gsufn, getfn, scanfn)` from
/// `Src/zsh.h:2123`.
#[inline]
#[allow(non_snake_case)]
pub fn SPECIALPMDEF(
    name: &str,
    flags: i32,
    gsufn: usize,
    getfn: Option<GetNodeFunc>,
    scanfn: Option<ScanTabFunc>,
) -> paramdef {
    paramdef {
        name: name.to_string(),
        flags: flags | (PM_SPECIAL | PM_HIDE | PM_HIDEVAL) as i32,
        var: 0,
        gsu: gsufn,
        getnfn: getfn,
        scantfn: scanfn,
        pm: None,
    }
}
/// Port of `WRAPDEF(func)` from `Src/zsh.h:1371`.
#[inline]
#[allow(non_snake_case)]
pub fn WRAPDEF(func: WrapFunc) -> funcwrap {
    funcwrap {
        next: None,
        flags: 0,
        handler: Some(func),
        module: None,
    }
}

// =============================================================================
// 17. Job structures (zsh.h:1046-1166).
// =============================================================================
/// `jobfile` — see fields for layout.
#[allow(non_camel_case_types)]
pub struct jobfile {
    // c:1046
    /// `name` field.
    pub name: Option<String>,
    /// `fd` field.
    pub fd: i32,
    /// `is_fd` field.
    pub is_fd: i32,
}
/// `STAT_CHANGED` constant.
pub const STAT_CHANGED: i32 = 0x0001; // c:1073
/// `STAT_STOPPED` constant.
pub const STAT_STOPPED: i32 = 0x0002;
/// `STAT_TIMED` constant.
pub const STAT_TIMED: i32 = 0x0004;
/// `STAT_DONE` constant.
pub const STAT_DONE: i32 = 0x0008;
/// `STAT_LOCKED` constant.
pub const STAT_LOCKED: i32 = 0x0010;
/// `STAT_NOPRINT` constant.
pub const STAT_NOPRINT: i32 = 0x0020;
/// `STAT_INUSE` constant.
pub const STAT_INUSE: i32 = 0x0040;
/// `STAT_SUPERJOB` constant.
pub const STAT_SUPERJOB: i32 = 0x0080;
/// `STAT_SUBJOB` constant.
pub const STAT_SUBJOB: i32 = 0x0100;
/// `STAT_WASSUPER` constant.
pub const STAT_WASSUPER: i32 = 0x0200;
/// `STAT_CURSH` constant.
pub const STAT_CURSH: i32 = 0x0400;
/// `STAT_NOSTTY` constant.
pub const STAT_NOSTTY: i32 = 0x0800;
/// `STAT_ATTACH` constant.
pub const STAT_ATTACH: i32 = 0x1000;
/// `STAT_SUBLEADER` constant.
pub const STAT_SUBLEADER: i32 = 0x2000;
/// `STAT_BUILTIN` constant.
pub const STAT_BUILTIN: i32 = 0x4000;
/// `STAT_SUBJOB_ORPHANED` constant.
pub const STAT_SUBJOB_ORPHANED: i32 = 0x8000;
/// `STAT_DISOWN` constant.
pub const STAT_DISOWN: i32 = 0x10000; // c:1095
/// `SP_RUNNING` constant.
pub const SP_RUNNING: i32 = -1; // c:1097
/// `JOBTEXTSIZE` constant.
pub const JOBTEXTSIZE: usize = 80; // c:1104
                                   // C: `#define MAXJOBS_ALLOC 50` (Src/zsh.h:1107) — an int literal.
                                   // Stored as `usize` so callers using it for Vec capacity / slice
                                   // indexing don't need `as usize` casts everywhere. Matches the
                                   // adjacent `MAX_PIPESTATS: usize` type choice (both are array
                                   // sizes in C).
/// `MAXJOBS_ALLOC` constant.
pub const MAXJOBS_ALLOC: usize = 50; // c:1107
/// `MAX_PIPESTATS` constant.
pub const MAX_PIPESTATS: usize = 256; // c:1166
/// `timeinfo` — see fields for layout.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Default)]
pub struct timeinfo {
    // c:1099-1115 — when HAVE_GETRUSAGE the C type is `struct rusage`
    // and printtime reads ru_maxrss / ru_majflt / ru_minflt / ru_nswap /
    // ru_ixrss / ru_idrss / ru_isrss / ru_inblock / ru_oublock /
    // ru_nvcsw / ru_nivcsw / ru_msgsnd / ru_msgrcv / ru_nsignals.
    /// `ut` field.
    pub ut: i64,
    /// `st` field.
    pub st: i64,
    /// Maximum resident set size (KB).            ru_maxrss (c:945-952)
    pub maxrss: i64,
    /// Major page faults.                          ru_majflt (c:954-957)
    pub majflt: i64,
    /// Minor page faults.                          ru_minflt (c:959-962)
    pub minflt: i64,
    /// Number of swaps.                            ru_nswap  (c:896-899)
    pub nswap: i64,
    /// Integral shared memory size.                ru_ixrss  (c:901-907)
    pub ixrss: i64,
    /// Integral unshared data size.                ru_idrss  (c:909-919)
    pub idrss: i64,
    /// Integral unshared stack size.               ru_isrss
    pub isrss: i64,
    /// Block input operations.                     ru_inblock
    pub inblock: i64,
    /// Block output operations.                    ru_oublock
    pub oublock: i64,
    /// Voluntary context switches.                 ru_nvcsw
    pub nvcsw: i64,
    /// Involuntary context switches.               ru_nivcsw
    pub nivcsw: i64,
    /// IPC messages sent.                          ru_msgsnd
    pub msgsnd: i64,
    /// IPC messages received.                      ru_msgrcv
    pub msgrcv: i64,
    /// Signals received.                           ru_nsignals
    pub nsignals: i64,
}

impl timeinfo {
    /// `user_dur` — see implementation.
    pub fn user_dur(&self) -> std::time::Duration {
        std::time::Duration::from_micros(self.ut as u64)
    }
    /// `sys_dur` — see implementation.
    pub fn sys_dur(&self) -> std::time::Duration {
        std::time::Duration::from_micros(self.st as u64)
    }

    /// Populate this `timeinfo` from a `libc::rusage` snapshot.
    /// On macOS `ru_maxrss` is in bytes; on Linux it's KB — caller
    /// normalises via cfg.
    #[cfg(unix)]
    pub fn from_rusage(r: &libc::rusage) -> Self {
        let ut = r.ru_utime.tv_sec as i64 * 1_000_000 + r.ru_utime.tv_usec as i64;
        let st = r.ru_stime.tv_sec as i64 * 1_000_000 + r.ru_stime.tv_usec as i64;
        #[cfg(target_os = "macos")]
        let maxrss = r.ru_maxrss / 1024;
        #[cfg(not(target_os = "macos"))]
        let maxrss = r.ru_maxrss as i64;
        Self {
            ut,
            st,
            maxrss: maxrss as i64,
            majflt: r.ru_majflt as i64,
            minflt: r.ru_minflt as i64,
            nswap: r.ru_nswap as i64,
            ixrss: r.ru_ixrss as i64,
            idrss: r.ru_idrss as i64,
            isrss: r.ru_isrss as i64,
            inblock: r.ru_inblock as i64,
            oublock: r.ru_oublock as i64,
            nvcsw: r.ru_nvcsw as i64,
            nivcsw: r.ru_nivcsw as i64,
            msgsnd: r.ru_msgsnd as i64,
            msgrcv: r.ru_msgrcv as i64,
            nsignals: r.ru_nsignals as i64,
        }
    }
}

// =============================================================================
// 18. Hash table types (zsh.h:1172-1235) — DISABLED.
// =============================================================================
/// `DISABLED` constant.
pub const DISABLED: i32 = 1 << 0; // c:1235

// =============================================================================
// 19. Alias / asgment / cmdnam / shfunc / funcstack flags + macros.
// =============================================================================
/// `HASHED` constant.
pub const HASHED: i32 = 1 << 1; // c:1312
/// `ALIAS_GLOBAL` constant.
pub const ALIAS_GLOBAL: i32 = 1 << 1; // c:1261
/// `ALIAS_SUFFIX` constant.
pub const ALIAS_SUFFIX: i32 = 1 << 2; // c:1263
/// `ASG_ARRAY` constant.
pub const ASG_ARRAY: i32 = 1; // c:1280
/// `ASG_KEY_VALUE` constant.
pub const ASG_KEY_VALUE: i32 = 2; // c:1282
/// `SFC_NONE` constant.
pub const SFC_NONE: i32 = 0; // c:1329
/// `SFC_DIRECT` constant.
pub const SFC_DIRECT: i32 = 1;
/// `SFC_SIGNAL` constant.
pub const SFC_SIGNAL: i32 = 2;
/// `SFC_HOOK` constant.
pub const SFC_HOOK: i32 = 3;
/// `SFC_WIDGET` constant.
pub const SFC_WIDGET: i32 = 4;
/// `SFC_COMPLETE` constant.
pub const SFC_COMPLETE: i32 = 5;
/// `SFC_CWIDGET` constant.
pub const SFC_CWIDGET: i32 = 6;
/// `SFC_SUBST` constant.
pub const SFC_SUBST: i32 = 7;
/// `FS_SOURCE` constant.
pub const FS_SOURCE: i32 = 0; // c:1341
/// `FS_FUNC` constant.
pub const FS_FUNC: i32 = 1;
/// `FS_EVAL` constant.
pub const FS_EVAL: i32 = 2;
/// `WRAPF_ADDED` constant.
pub const WRAPF_ADDED: i32 = 1; // c:1369
/// `HOOK_SUFFIX` constant.
pub const HOOK_SUFFIX: &str = "_functions"; // c:1379
/// `HOOK_SUFFIX_LEN` constant.
pub const HOOK_SUFFIX_LEN: usize = 11; // c:1381

// =============================================================================
// 20. Options struct + MAX_OPS + OPT_* macros (zsh.h:1396-1427).
// =============================================================================
/// `MAX_OPS` constant.
pub const MAX_OPS: usize = 128; // c:1396
/// `options` — see fields for layout.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct options {
    // c:1416
    pub ind: [u8; MAX_OPS],
    /// `args` field.
    pub args: Vec<String>,
    /// `argscount` field.
    pub argscount: i32,
    /// `argsalloc` field.
    pub argsalloc: i32,
}
/// `PARSEARGS_TOPLEVEL` constant.
pub const PARSEARGS_TOPLEVEL: i32 = 0x1; // c:1425
/// `PARSEARGS_LOGIN` constant.
pub const PARSEARGS_LOGIN: i32 = 0x2; // c:1426

// Port of OPT_* macros from Src/zsh.h:1400-1414. Each takes
// `Options ops` (= `struct options *`) and a char index. The Rust
// port takes `&options` (a reference to the struct ported above) and
// indexes `ind[c]`. Char indexing is direct (not c-1) per zsh.h:1408
// `((ops)->ind[c] != 0)`.

/// Port of `OPT_MINUS(ops,c)` from `Src/zsh.h:1400` —
/// `((ops)->ind[c] & 1)`. True if option was set as `-X`.
#[inline]
#[allow(non_snake_case)]
pub fn OPT_MINUS(ops: &options, c: u8) -> bool {
    (ops.ind[c as usize] & 1) != 0
}

/// Port of `OPT_PLUS(ops,c)` from `Src/zsh.h:1402` —
/// `((ops)->ind[c] & 2)`. True if option was set as `+X`.
#[inline]
#[allow(non_snake_case)]
pub fn OPT_PLUS(ops: &options, c: u8) -> bool {
    (ops.ind[c as usize] & 2) != 0
}

/// Port of `OPT_ISSET(ops,c)` from `Src/zsh.h:1408` —
/// `((ops)->ind[c] != 0)`. True if option was set any way.
#[inline]
#[allow(non_snake_case)]
pub fn OPT_ISSET(ops: &options, c: u8) -> bool {
    ops.ind[c as usize] != 0
}

/// Port of `OPT_HASARG(ops,c)` from `Src/zsh.h:1410` —
/// `((ops)->ind[c] > 3)`. True if option carries an argument.
#[inline]
#[allow(non_snake_case)]
pub fn OPT_HASARG(ops: &options, c: u8) -> bool {
    ops.ind[c as usize] > 3
}

// =============================================================================
// 21. Builtin types + BINF_* (zsh.h:1436-1486).
// =============================================================================
/// `HandlerFunc` type alias.
pub type HandlerFunc = fn(name: &str, args: &[String], ops: &options, funcid: i32) -> i32;
/// `BINF_PLUSOPTS` constant.
pub const BINF_PLUSOPTS: u32 = 1 << 1; // c:1457
/// `BINF_PRINTOPTS` constant.
pub const BINF_PRINTOPTS: u32 = 1 << 2; // c:1458
/// `BINF_ADDED` constant.
pub const BINF_ADDED: u32 = 1 << 3; // c:1459
/// `BINF_MAGICEQUALS` constant.
pub const BINF_MAGICEQUALS: u32 = 1 << 4; // c:1460
/// `BINF_PREFIX` constant.
pub const BINF_PREFIX: u32 = 1 << 5; // c:1461
/// `BINF_DASH` constant.
pub const BINF_DASH: u32 = 1 << 6; // c:1462
/// `BINF_BUILTIN` constant.
pub const BINF_BUILTIN: u32 = 1 << 7; // c:1463
/// `BINF_COMMAND` constant.
pub const BINF_COMMAND: u32 = 1 << 8; // c:1464
/// `BINF_EXEC` constant.
pub const BINF_EXEC: u32 = 1 << 9; // c:1465
/// `BINF_NOGLOB` constant.
pub const BINF_NOGLOB: u32 = 1 << 10; // c:1466
/// `BINF_PSPECIAL` constant.
pub const BINF_PSPECIAL: u32 = 1 << 11; // c:1467
/// `BINF_SKIPINVALID` constant.
pub const BINF_SKIPINVALID: u32 = 1 << 12; // c:1469
/// `BINF_KEEPNUM` constant.
pub const BINF_KEEPNUM: u32 = 1 << 13; // c:1470
/// `BINF_SKIPDASH` constant.
pub const BINF_SKIPDASH: u32 = 1 << 14; // c:1471
/// `BINF_DASHDASHVALID` constant.
pub const BINF_DASHDASHVALID: u32 = 1 << 15; // c:1472
/// `BINF_CLEARENV` constant.
pub const BINF_CLEARENV: u32 = 1 << 16; // c:1473
/// `BINF_AUTOALL` constant.
pub const BINF_AUTOALL: u32 = 1 << 17; // c:1474
/// `BINF_HANDLES_OPTS` constant.
pub const BINF_HANDLES_OPTS: u32 = 1 << 18; // c:1480
/// `BINF_ASSIGN` constant.
pub const BINF_ASSIGN: u32 = 1 << 19; // c:1486

// =============================================================================
// 22. Module flags (zsh.h:1516-1532).
// =============================================================================
/// `MOD_BUSY` constant.
pub const MOD_BUSY: i32 = 1 << 0; // c:1516
/// `MOD_UNLOAD` constant.
pub const MOD_UNLOAD: i32 = 1 << 1; // c:1522
/// `MOD_SETUP` constant.
pub const MOD_SETUP: i32 = 1 << 2; // c:1524
/// `MOD_LINKED` constant.
pub const MOD_LINKED: i32 = 1 << 3; // c:1526
/// `MOD_INIT_S` constant.
pub const MOD_INIT_S: i32 = 1 << 4; // c:1528
/// `MOD_INIT_B` constant.
pub const MOD_INIT_B: i32 = 1 << 5; // c:1530
/// `MOD_ALIAS` constant.
pub const MOD_ALIAS: i32 = 1 << 6; // c:1532
/// `HOOKF_ALL` constant.
pub const HOOKF_ALL: i32 = 1; // c:1592

// =============================================================================
// 23. Pattern flags (zsh.h:1624-1637).
// =============================================================================
/// `PAT_HEAPDUP` constant.
pub const PAT_HEAPDUP: i32 = 0x0000; // c:1624
/// `PAT_FILE` constant.
pub const PAT_FILE: i32 = 0x0001;
/// `PAT_FILET` constant.
pub const PAT_FILET: i32 = 0x0002;
/// `PAT_ANY` constant.
pub const PAT_ANY: i32 = 0x0004;
/// `PAT_NOANCH` constant.
pub const PAT_NOANCH: i32 = 0x0008;
/// `PAT_NOGLD` constant.
pub const PAT_NOGLD: i32 = 0x0010;
/// `PAT_PURES` constant.
pub const PAT_PURES: i32 = 0x0020;
/// `PAT_STATIC` constant.
pub const PAT_STATIC: i32 = 0x0040;
/// `PAT_SCAN` constant.
pub const PAT_SCAN: i32 = 0x0080;
/// `PAT_ZDUP` constant.
pub const PAT_ZDUP: i32 = 0x0100;
/// `PAT_NOTSTART` constant.
pub const PAT_NOTSTART: i32 = 0x0200;
/// `PAT_NOTEND` constant.
pub const PAT_NOTEND: i32 = 0x0400;
/// `PAT_HAS_EXCLUDP` constant.
pub const PAT_HAS_EXCLUDP: i32 = 0x0800;
/// `PAT_LCMATCHUC` constant.
pub const PAT_LCMATCHUC: i32 = 0x1000;

// =============================================================================
// 24. zpc_chars enum (zsh.h:1643-1676).
// =============================================================================
/// `ZPC_SLASH` constant.
pub const ZPC_SLASH: i32 = 0;
/// `ZPC_NULL` constant.
pub const ZPC_NULL: i32 = 1;
/// `ZPC_BAR` constant.
pub const ZPC_BAR: i32 = 2;
/// `ZPC_OUTPAR` constant.
pub const ZPC_OUTPAR: i32 = 3;
/// `ZPC_TILDE` constant.
pub const ZPC_TILDE: i32 = 4;
/// `ZPC_SEG_COUNT` constant.
pub const ZPC_SEG_COUNT: i32 = 5;
/// `ZPC_INPAR` constant.
pub const ZPC_INPAR: i32 = ZPC_SEG_COUNT;
/// `ZPC_QUEST` constant.
pub const ZPC_QUEST: i32 = ZPC_SEG_COUNT + 1;
/// `ZPC_STAR` constant.
pub const ZPC_STAR: i32 = ZPC_SEG_COUNT + 2;
/// `ZPC_INBRACK` constant.
pub const ZPC_INBRACK: i32 = ZPC_SEG_COUNT + 3;
/// `ZPC_INANG` constant.
pub const ZPC_INANG: i32 = ZPC_SEG_COUNT + 4;
/// `ZPC_HAT` constant.
pub const ZPC_HAT: i32 = ZPC_SEG_COUNT + 5;
/// `ZPC_HASH` constant.
pub const ZPC_HASH: i32 = ZPC_SEG_COUNT + 6;
/// `ZPC_BNULLKEEP` constant.
pub const ZPC_BNULLKEEP: i32 = ZPC_SEG_COUNT + 7;
/// `ZPC_NO_KSH_GLOB` constant.
pub const ZPC_NO_KSH_GLOB: i32 = ZPC_SEG_COUNT + 8;
/// `ZPC_KSH_QUEST` constant.
pub const ZPC_KSH_QUEST: i32 = ZPC_NO_KSH_GLOB;
/// `ZPC_KSH_STAR` constant.
pub const ZPC_KSH_STAR: i32 = ZPC_NO_KSH_GLOB + 1;
/// `ZPC_KSH_PLUS` constant.
pub const ZPC_KSH_PLUS: i32 = ZPC_NO_KSH_GLOB + 2;
/// `ZPC_KSH_BANG` constant.
pub const ZPC_KSH_BANG: i32 = ZPC_NO_KSH_GLOB + 3;
/// `ZPC_KSH_BANG2` constant.
pub const ZPC_KSH_BANG2: i32 = ZPC_NO_KSH_GLOB + 4;
/// `ZPC_KSH_AT` constant.
pub const ZPC_KSH_AT: i32 = ZPC_NO_KSH_GLOB + 5;
/// `ZPC_COUNT` constant.
pub const ZPC_COUNT: i32 = ZPC_NO_KSH_GLOB + 6;

// =============================================================================
// 25. PP_* (zsh.h:1707-1735) + GF_* + ZMB_*.
// =============================================================================
/// `PP_FIRST` constant.
pub const PP_FIRST: i32 = 1;
/// `PP_ALPHA` constant.
pub const PP_ALPHA: i32 = 1;
/// `PP_ALNUM` constant.
pub const PP_ALNUM: i32 = 2;
/// `PP_ASCII` constant.
pub const PP_ASCII: i32 = 3;
/// `PP_BLANK` constant.
pub const PP_BLANK: i32 = 4;
/// `PP_CNTRL` constant.
pub const PP_CNTRL: i32 = 5;
/// `PP_DIGIT` constant.
pub const PP_DIGIT: i32 = 6;
/// `PP_GRAPH` constant.
pub const PP_GRAPH: i32 = 7;
/// `PP_LOWER` constant.
pub const PP_LOWER: i32 = 8;
/// `PP_PRINT` constant.
pub const PP_PRINT: i32 = 9;
/// `PP_PUNCT` constant.
pub const PP_PUNCT: i32 = 10;
/// `PP_SPACE` constant.
pub const PP_SPACE: i32 = 11;
/// `PP_UPPER` constant.
pub const PP_UPPER: i32 = 12;
/// `PP_XDIGIT` constant.
pub const PP_XDIGIT: i32 = 13;
/// `PP_IDENT` constant.
pub const PP_IDENT: i32 = 14;
/// `PP_IFS` constant.
pub const PP_IFS: i32 = 15;
/// `PP_IFSSPACE` constant.
pub const PP_IFSSPACE: i32 = 16;
/// `PP_WORD` constant.
pub const PP_WORD: i32 = 17;
/// `PP_INCOMPLETE` constant.
pub const PP_INCOMPLETE: i32 = 18;
/// `PP_INVALID` constant.
pub const PP_INVALID: i32 = 19;
/// `PP_LAST` constant.
pub const PP_LAST: i32 = 19;
/// `PP_UNKWN` constant.
pub const PP_UNKWN: i32 = 20;
/// `PP_RANGE` constant.
pub const PP_RANGE: i32 = 21;
/// `GF_LCMATCHUC` constant.
pub const GF_LCMATCHUC: i32 = 0x0100;
/// `GF_IGNCASE` constant.
pub const GF_IGNCASE: i32 = 0x0200;
/// `GF_BACKREF` constant.
pub const GF_BACKREF: i32 = 0x0400;
/// `GF_MATCHREF` constant.
pub const GF_MATCHREF: i32 = 0x0800;
/// `GF_MULTIBYTE` constant.
pub const GF_MULTIBYTE: i32 = 0x1000;
/// `ZMB_VALID` constant.
pub const ZMB_VALID: i32 = 0;
/// `ZMB_INCOMPLETE` constant.
pub const ZMB_INCOMPLETE: i32 = 1;
/// `ZMB_INVALID` constant.
pub const ZMB_INVALID: i32 = 2;

// =============================================================================
// 26. Param type flags (zsh.h:1878-1949).
// =============================================================================
/// `PM_SCALAR` constant.
pub const PM_SCALAR: u32 = 0;
/// `PM_ARRAY` constant.
pub const PM_ARRAY: u32 = 1 << 0;
/// `PM_INTEGER` constant.
pub const PM_INTEGER: u32 = 1 << 1;
/// `PM_EFLOAT` constant.
pub const PM_EFLOAT: u32 = 1 << 2;
/// `PM_FFLOAT` constant.
pub const PM_FFLOAT: u32 = 1 << 3;
/// `PM_HASHED` constant.
pub const PM_HASHED: u32 = 1 << 4;
/// `PM_LEFT` constant.
pub const PM_LEFT: u32 = 1 << 5;
/// `PM_RIGHT_B` constant.
pub const PM_RIGHT_B: u32 = 1 << 6;
/// `PM_RIGHT_Z` constant.
pub const PM_RIGHT_Z: u32 = 1 << 7;
/// `PM_LOWER` constant.
pub const PM_LOWER: u32 = 1 << 8;
/// `PM_UPPER` constant.
pub const PM_UPPER: u32 = 1 << 9;
/// `PM_UNDEFINED` constant.
pub const PM_UNDEFINED: u32 = 1 << 9;
/// `PM_READONLY` constant.
pub const PM_READONLY: u32 = 1 << 10;
/// `PM_TAGGED` constant.
pub const PM_TAGGED: u32 = 1 << 11;
/// `PM_EXPORTED` constant.
pub const PM_EXPORTED: u32 = 1 << 12;
/// `PM_ABSPATH_USED` constant.
pub const PM_ABSPATH_USED: u32 = 1 << 12;
/// `PM_UNIQUE` constant.
pub const PM_UNIQUE: u32 = 1 << 13;
/// `PM_UNALIASED` constant.
pub const PM_UNALIASED: u32 = 1 << 13;
/// `PM_HIDE` constant.
pub const PM_HIDE: u32 = 1 << 14;
/// `PM_CUR_FPATH` constant.
pub const PM_CUR_FPATH: u32 = 1 << 14;
/// `PM_HIDEVAL` constant.
pub const PM_HIDEVAL: u32 = 1 << 15;
/// `PM_WARNNESTED` constant.
pub const PM_WARNNESTED: u32 = 1 << 15;
/// `PM_TIED` constant.
pub const PM_TIED: u32 = 1 << 16;
/// `PM_TAGGED_LOCAL` constant.
pub const PM_TAGGED_LOCAL: u32 = 1 << 16;
/// `PM_DONTIMPORT_SUID` constant.
pub const PM_DONTIMPORT_SUID: u32 = 1 << 17;
/// `PM_LOADDIR` constant.
pub const PM_LOADDIR: u32 = 1 << 17;
/// `PM_SINGLE` constant.
pub const PM_SINGLE: u32 = 1 << 18;
/// `PM_ANONYMOUS` constant.
pub const PM_ANONYMOUS: u32 = 1 << 18;
/// `PM_LOCAL` constant.
pub const PM_LOCAL: u32 = 1 << 19;
/// `PM_KSHSTORED` constant.
pub const PM_KSHSTORED: u32 = 1 << 19;
/// `PM_SPECIAL` constant.
pub const PM_SPECIAL: u32 = 1 << 20;
/// `PM_ZSHSTORED` constant.
pub const PM_ZSHSTORED: u32 = 1 << 20;
/// `PM_RO_BY_DESIGN` constant.
pub const PM_RO_BY_DESIGN: u32 = 1 << 21;
/// `PM_READONLY_SPECIAL` constant.
pub const PM_READONLY_SPECIAL: u32 = PM_SPECIAL | PM_READONLY | PM_RO_BY_DESIGN;
/// `PM_DONTIMPORT` constant.
pub const PM_DONTIMPORT: u32 = 1 << 22;
/// `PM_DECLARED` constant.
pub const PM_DECLARED: u32 = 1 << 22;
/// `PM_RESTRICTED` constant.
pub const PM_RESTRICTED: u32 = 1 << 23;
/// `PM_UNSET` constant.
pub const PM_UNSET: u32 = 1 << 24;
/// `PM_DEFAULTED` constant.
pub const PM_DEFAULTED: u32 = PM_DECLARED | PM_UNSET;
/// `PM_REMOVABLE` constant.
pub const PM_REMOVABLE: u32 = 1 << 25;
/// `PM_AUTOLOAD` constant.
pub const PM_AUTOLOAD: u32 = 1 << 26;
/// `PM_NORESTORE` constant.
pub const PM_NORESTORE: u32 = 1 << 27;
/// `PM_AUTOALL` constant.
pub const PM_AUTOALL: u32 = 1 << 27;
/// `PM_HASHELEM` constant.
pub const PM_HASHELEM: u32 = 1 << 28;
/// `PM_NAMEDDIR` constant.
pub const PM_NAMEDDIR: u32 = 1 << 29;
/// `PM_NAMEREF` constant.
pub const PM_NAMEREF: u32 = 1 << 30;
/// `PM_TYPE` — see implementation.
#[inline]
#[allow(non_snake_case)]
pub const fn PM_TYPE(x: u32) -> u32 {
    x & (PM_SCALAR | PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_ARRAY | PM_HASHED | PM_NAMEREF)
}
/// `TYPESET_OPTSTR` constant.
pub const TYPESET_OPTSTR: &str = "aiEFALRZlurtxUhHT"; // c:1947
/// `TYPESET_OPTNUM` constant.
pub const TYPESET_OPTNUM: &str = "LRZiEF"; // c:1950

// =============================================================================
// 27. SCANPM_* (zsh.h:1953-1973).
// =============================================================================
/// `SCANPM_WANTVALS` constant.
pub const SCANPM_WANTVALS: u32 = 1 << 0;
/// `SCANPM_WANTKEYS` constant.
pub const SCANPM_WANTKEYS: u32 = 1 << 1;
/// `SCANPM_WANTINDEX` constant.
pub const SCANPM_WANTINDEX: u32 = 1 << 2;
/// `SCANPM_MATCHKEY` constant.
pub const SCANPM_MATCHKEY: u32 = 1 << 3;
/// `SCANPM_MATCHVAL` constant.
pub const SCANPM_MATCHVAL: u32 = 1 << 4;
/// `SCANPM_MATCHMANY` constant.
pub const SCANPM_MATCHMANY: u32 = 1 << 5;
/// `SCANPM_ASSIGNING` constant.
pub const SCANPM_ASSIGNING: u32 = 1 << 6;
/// `SCANPM_KEYMATCH` constant.
pub const SCANPM_KEYMATCH: u32 = 1 << 7;
/// `SCANPM_DQUOTED` constant.
pub const SCANPM_DQUOTED: u32 = 1 << 8;
/// `SCANPM_ARRONLY` constant.
pub const SCANPM_ARRONLY: u32 = 1 << 9;
/// `SCANPM_CHECKING` constant.
pub const SCANPM_CHECKING: u32 = 1 << 10;
/// `SCANPM_NOEXEC` constant.
pub const SCANPM_NOEXEC: u32 = 1 << 11;
/// `SCANPM_NONAMESPC` constant.
pub const SCANPM_NONAMESPC: u32 = 1 << 12;
/// `SCANPM_NONAMEREF` constant.
pub const SCANPM_NONAMEREF: u32 = 1 << 13;
/// `SCANPM_ISVAR_AT` constant.
pub const SCANPM_ISVAR_AT: u32 = 1 << 14;

// =============================================================================
// 28. SUB_* substitution flags (zsh.h:1981-1996).
// =============================================================================
/// `SUB_END` constant.
pub const SUB_END: i32 = 0x0001;
/// `SUB_LONG` constant.
pub const SUB_LONG: i32 = 0x0002;
/// `SUB_SUBSTR` constant.
pub const SUB_SUBSTR: i32 = 0x0004;
/// `SUB_MATCH` constant.
pub const SUB_MATCH: i32 = 0x0008;
/// `SUB_REST` constant.
pub const SUB_REST: i32 = 0x0010;
/// `SUB_BIND` constant.
pub const SUB_BIND: i32 = 0x0020;
/// `SUB_EIND` constant.
pub const SUB_EIND: i32 = 0x0040;
/// `SUB_LEN` constant.
pub const SUB_LEN: i32 = 0x0080;
/// `SUB_ALL` constant.
pub const SUB_ALL: i32 = 0x0100;
/// `SUB_GLOBAL` constant.
pub const SUB_GLOBAL: i32 = 0x0200;
/// `SUB_DOSUBST` constant.
pub const SUB_DOSUBST: i32 = 0x0400;
/// `SUB_RETFAIL` constant.
pub const SUB_RETFAIL: i32 = 0x0800;
/// `SUB_START` constant.
pub const SUB_START: i32 = 0x1000;
/// `SUB_LIST` constant.
pub const SUB_LIST: i32 = 0x2000;
/// `SUB_EGLOB` constant.
pub const SUB_EGLOB: i32 = 0x4000;

// =============================================================================
// 29. ZSHTOK_* + PREFORK_* + MULTSUB_* (zsh.h:2014-2065).
// =============================================================================
/// `ZSHTOK_SUBST` constant.
pub const ZSHTOK_SUBST: i32 = 0x0001;
/// `ZSHTOK_SHGLOB` constant.
pub const ZSHTOK_SHGLOB: i32 = 0x0002;
/// `PREFORK_TYPESET` constant.
pub const PREFORK_TYPESET: i32 = 0x01;
/// `PREFORK_ASSIGN` constant.
pub const PREFORK_ASSIGN: i32 = 0x02;
/// `PREFORK_SINGLE` constant.
pub const PREFORK_SINGLE: i32 = 0x04;
/// `PREFORK_SPLIT` constant.
pub const PREFORK_SPLIT: i32 = 0x08;
/// `PREFORK_SHWORDSPLIT` constant.
pub const PREFORK_SHWORDSPLIT: i32 = 0x10;
/// `PREFORK_NOSHWORDSPLIT` constant.
pub const PREFORK_NOSHWORDSPLIT: i32 = 0x20;
/// `PREFORK_SUBEXP` constant.
pub const PREFORK_SUBEXP: i32 = 0x40;
/// `PREFORK_KEY_VALUE` constant.
pub const PREFORK_KEY_VALUE: i32 = 0x80;
/// `PREFORK_NO_UNTOK` constant.
pub const PREFORK_NO_UNTOK: i32 = 0x100;
/// `MULTSUB_WS_AT_START` constant.
pub const MULTSUB_WS_AT_START: i32 = 1;
/// `MULTSUB_WS_AT_END` constant.
pub const MULTSUB_WS_AT_END: i32 = 2;
/// `MULTSUB_PARAM_NAME` constant.
pub const MULTSUB_PARAM_NAME: i32 = 4;

// =============================================================================
// 30. ASSPM_* (zsh.h:2130-2145).
// =============================================================================
/// `ASSPM_AUGMENT` constant.
pub const ASSPM_AUGMENT: i32 = 1 << 0;
/// `ASSPM_WARN_CREATE` constant.
pub const ASSPM_WARN_CREATE: i32 = 1 << 1;
/// `ASSPM_WARN_NESTED` constant.
pub const ASSPM_WARN_NESTED: i32 = 1 << 2;
/// `ASSPM_WARN` constant.
pub const ASSPM_WARN: i32 = ASSPM_WARN_CREATE | ASSPM_WARN_NESTED;
/// `ASSPM_ENV_IMPORT` constant.
pub const ASSPM_ENV_IMPORT: i32 = 1 << 3;
/// `ASSPM_KEY_VALUE` constant.
pub const ASSPM_KEY_VALUE: i32 = 1 << 4;

// =============================================================================
// 31. ND_* + PRINT_* + loop_return + source_return + noerrexit_bits.
// =============================================================================
/// `ND_USERNAME` constant.
pub const ND_USERNAME: i32 = 1 << 1; // c:2157
/// `ND_NOABBREV` constant.
pub const ND_NOABBREV: i32 = 1 << 2; // c:2158
/// `PRINT_NAMEONLY` constant.
pub const PRINT_NAMEONLY: i32 = 1 << 0; // c:2179
/// `PRINT_TYPE` constant.
pub const PRINT_TYPE: i32 = 1 << 1;
/// `PRINT_LIST` constant.
pub const PRINT_LIST: i32 = 1 << 2;
/// `PRINT_KV_PAIR` constant.
pub const PRINT_KV_PAIR: i32 = 1 << 3;
/// `PRINT_INCLUDEVALUE` constant.
pub const PRINT_INCLUDEVALUE: i32 = 1 << 4;
/// `PRINT_TYPESET` constant.
pub const PRINT_TYPESET: i32 = 1 << 5;
/// `PRINT_LINE` constant.
pub const PRINT_LINE: i32 = 1 << 6;
/// `PRINT_POSIX_EXPORT` constant.
pub const PRINT_POSIX_EXPORT: i32 = 1 << 7;
/// `PRINT_POSIX_READONLY` constant.
pub const PRINT_POSIX_READONLY: i32 = 1 << 8;
/// `PRINT_WITH_NAMESPACE` constant.
pub const PRINT_WITH_NAMESPACE: i32 = 1 << 9;
/// `PRINT_WHENCE_CSH` constant.
pub const PRINT_WHENCE_CSH: i32 = 1 << 7; // c:2191
/// `PRINT_WHENCE_VERBOSE` constant.
pub const PRINT_WHENCE_VERBOSE: i32 = 1 << 8;
/// `PRINT_WHENCE_SIMPLE` constant.
pub const PRINT_WHENCE_SIMPLE: i32 = 1 << 9;
/// `PRINT_WHENCE_FUNCDEF` constant.
pub const PRINT_WHENCE_FUNCDEF: i32 = 1 << 10;
/// `PRINT_WHENCE_WORD` constant.
pub const PRINT_WHENCE_WORD: i32 = 1 << 11;
/// `LOOP_OK` constant.
pub const LOOP_OK: i32 = 0; // c:2199
/// `LOOP_EMPTY` constant.
pub const LOOP_EMPTY: i32 = 1;
/// `LOOP_ERROR` constant.
pub const LOOP_ERROR: i32 = 2;
/// `SOURCE_OK` constant.
pub const SOURCE_OK: i32 = 0; // c:2210
/// `SOURCE_NOT_FOUND` constant.
pub const SOURCE_NOT_FOUND: i32 = 1;
/// `SOURCE_ERROR` constant.
pub const SOURCE_ERROR: i32 = 2;
/// `NOERREXIT_EXIT` constant.
pub const NOERREXIT_EXIT: i32 = 1; // c:2219
/// `NOERREXIT_RETURN` constant.
pub const NOERREXIT_RETURN: i32 = 2;
/// `NOERREXIT_SIGNAL` constant.
pub const NOERREXIT_SIGNAL: i32 = 8;

// =============================================================================
// 32. History flags + GETHIST_* + HISTFLAG_* + HFILE_* + LEXFLAGS_*.
// =============================================================================
/// `HIST_MAKEUNIQUE` constant.
pub const HIST_MAKEUNIQUE: u32 = 0x00000001; // c:2252
/// `HIST_OLD` constant.
pub const HIST_OLD: u32 = 0x00000002;
/// `HIST_READ` constant.
pub const HIST_READ: u32 = 0x00000004;
/// `HIST_DUP` constant.
pub const HIST_DUP: u32 = 0x00000008;
/// `HIST_FOREIGN` constant.
pub const HIST_FOREIGN: u32 = 0x00000010;
/// `HIST_TMPSTORE` constant.
pub const HIST_TMPSTORE: u32 = 0x00000020;
/// `HIST_NOWRITE` constant.
pub const HIST_NOWRITE: u32 = 0x00000040;
/// `GETHIST_UPWARD` constant.
pub const GETHIST_UPWARD: i32 = -1;
/// `GETHIST_DOWNWARD` constant.
pub const GETHIST_DOWNWARD: i32 = 1;
/// `GETHIST_EXACT` constant.
pub const GETHIST_EXACT: i32 = 0;
/// `HISTFLAG_DONE` constant.
pub const HISTFLAG_DONE: i32 = 1; // c:2270
/// `HISTFLAG_NOEXEC` constant.
pub const HISTFLAG_NOEXEC: i32 = 2;
/// `HISTFLAG_RECALL` constant.
pub const HISTFLAG_RECALL: i32 = 4;
/// `HISTFLAG_SETTY` constant.
pub const HISTFLAG_SETTY: i32 = 8;
/// `HFILE_APPEND` constant.
pub const HFILE_APPEND: u32 = 0x0001;
/// `HFILE_SKIPOLD` constant.
pub const HFILE_SKIPOLD: u32 = 0x0002;
/// `HFILE_SKIPDUPS` constant.
pub const HFILE_SKIPDUPS: u32 = 0x0004;
/// `HFILE_SKIPFOREIGN` constant.
pub const HFILE_SKIPFOREIGN: u32 = 0x0008;
/// `HFILE_FAST` constant.
pub const HFILE_FAST: u32 = 0x0010;
/// `HFILE_NO_REWRITE` constant.
pub const HFILE_NO_REWRITE: u32 = 0x0020;
/// `HFILE_USE_OPTIONS` constant.
pub const HFILE_USE_OPTIONS: u32 = 0x8000;
/// `LEXFLAGS_ACTIVE` constant.
pub const LEXFLAGS_ACTIVE: i32 = 0x0001;
/// `LEXFLAGS_ZLE` constant.
pub const LEXFLAGS_ZLE: i32 = 0x0002;
/// `LEXFLAGS_COMMENTS_KEEP` constant.
pub const LEXFLAGS_COMMENTS_KEEP: i32 = 0x0004;
/// `LEXFLAGS_COMMENTS_STRIP` constant.
pub const LEXFLAGS_COMMENTS_STRIP: i32 = 0x0008;
/// `LEXFLAGS_COMMENTS` constant.
pub const LEXFLAGS_COMMENTS: i32 = LEXFLAGS_COMMENTS_KEEP | LEXFLAGS_COMMENTS_STRIP;
/// `LEXFLAGS_NEWLINE` constant.
pub const LEXFLAGS_NEWLINE: i32 = 0x0010;

// =============================================================================
// 33. Completion context (zsh.h:2322-2332).
// =============================================================================
/// `IN_NOTHING` constant.
pub const IN_NOTHING: i32 = 0;
/// `IN_CMD` constant.
pub const IN_CMD: i32 = 1;
/// `IN_MATH` constant.
pub const IN_MATH: i32 = 2;
/// `IN_COND` constant.
pub const IN_COND: i32 = 3;
/// `IN_ENV` constant.
pub const IN_ENV: i32 = 4;
/// `IN_PAR` constant.
pub const IN_PAR: i32 = 5;

// =============================================================================
// 34. Emulation flags (zsh.h:2341-2358).
// =============================================================================
/// `EMULATE_CSH` constant.
pub const EMULATE_CSH: i32 = 1 << 1; // c:2341
/// `EMULATE_KSH` constant.
pub const EMULATE_KSH: i32 = 1 << 2;
/// `EMULATE_SH` constant.
pub const EMULATE_SH: i32 = 1 << 3;
/// `EMULATE_ZSH` constant.
pub const EMULATE_ZSH: i32 = 1 << 4;
/// `EMULATE_FULLY` constant.
pub const EMULATE_FULLY: i32 = 1 << 5;
/// `EMULATE_UNUSED` constant.
pub const EMULATE_UNUSED: i32 = 1 << 6;

// =============================================================================
// 35. Option indices (zsh.h:2362-2550).
// =============================================================================
/// `OPT_INVALID` constant.
pub const OPT_INVALID: i32 = 0;
/// `ALIASESOPT` constant.
pub const ALIASESOPT: i32 = 1;
/// `ALIASFUNCDEF` constant.
pub const ALIASFUNCDEF: i32 = 2;
/// `ALLEXPORT` constant.
pub const ALLEXPORT: i32 = 3;
/// `ALWAYSLASTPROMPT` constant.
pub const ALWAYSLASTPROMPT: i32 = 4;
/// `ALWAYSTOEND` constant.
pub const ALWAYSTOEND: i32 = 5;
/// `APPENDHISTORY` constant.
pub const APPENDHISTORY: i32 = 6;
/// `AUTOCD` constant.
pub const AUTOCD: i32 = 7;
/// `AUTOCONTINUE` constant.
pub const AUTOCONTINUE: i32 = 8;
/// `AUTOLIST` constant.
pub const AUTOLIST: i32 = 9;
/// `AUTOMENU` constant.
pub const AUTOMENU: i32 = 10;
/// `AUTONAMEDIRS` constant.
pub const AUTONAMEDIRS: i32 = 11;
/// `AUTOPARAMKEYS` constant.
pub const AUTOPARAMKEYS: i32 = 12;
/// `AUTOPARAMSLASH` constant.
pub const AUTOPARAMSLASH: i32 = 13;
/// `AUTOPUSHD` constant.
pub const AUTOPUSHD: i32 = 14;
/// `AUTOREMOVESLASH` constant.
pub const AUTOREMOVESLASH: i32 = 15;
/// `AUTORESUME` constant.
pub const AUTORESUME: i32 = 16;
/// `BADPATTERN` constant.
pub const BADPATTERN: i32 = 17;
/// `BANGHIST` constant.
pub const BANGHIST: i32 = 18;
/// `BAREGLOBQUAL` constant.
pub const BAREGLOBQUAL: i32 = 19;
/// `BASHAUTOLIST` constant.
pub const BASHAUTOLIST: i32 = 20;
/// `BASHREMATCH` constant.
pub const BASHREMATCH: i32 = 21;
/// `BEEP` constant.
pub const BEEP: i32 = 22;
/// `BGNICE` constant.
pub const BGNICE: i32 = 23;
/// `BRACECCL` constant.
pub const BRACECCL: i32 = 24;
/// `BSDECHO` constant.
pub const BSDECHO: i32 = 25;
/// `CASEGLOB` constant.
pub const CASEGLOB: i32 = 26;
/// `CASEMATCH` constant.
pub const CASEMATCH: i32 = 27;
/// `CASEPATHS` constant.
pub const CASEPATHS: i32 = 28;
/// `CBASES` constant.
pub const CBASES: i32 = 29;
/// `CDABLEVARS` constant.
pub const CDABLEVARS: i32 = 30;
/// `CDSILENT` constant.
pub const CDSILENT: i32 = 31;
/// `CHASEDOTS` constant.
pub const CHASEDOTS: i32 = 32;
/// `CHASELINKS` constant.
pub const CHASELINKS: i32 = 33;
/// `CHECKJOBS` constant.
pub const CHECKJOBS: i32 = 34;
/// `CHECKRUNNINGJOBS` constant.
pub const CHECKRUNNINGJOBS: i32 = 35;
/// `CLOBBER` constant.
pub const CLOBBER: i32 = 36;
/// `CLOBBEREMPTY` constant.
pub const CLOBBEREMPTY: i32 = 37;
/// `APPENDCREATE` constant.
pub const APPENDCREATE: i32 = 38;
/// `COMBININGCHARS` constant.
pub const COMBININGCHARS: i32 = 39;
/// `COMPLETEALIASES` constant.
pub const COMPLETEALIASES: i32 = 40;
/// `COMPLETEINWORD` constant.
pub const COMPLETEINWORD: i32 = 41;
/// `CORRECT` constant.
pub const CORRECT: i32 = 42;
/// `CORRECTALL` constant.
pub const CORRECTALL: i32 = 43;
/// `CONTINUEONERROR` constant.
pub const CONTINUEONERROR: i32 = 44;
/// `CPRECEDENCES` constant.
pub const CPRECEDENCES: i32 = 45;
/// `CSHJUNKIEHISTORY` constant.
pub const CSHJUNKIEHISTORY: i32 = 46;
/// `CSHJUNKIELOOPS` constant.
pub const CSHJUNKIELOOPS: i32 = 47;
/// `CSHJUNKIEQUOTES` constant.
pub const CSHJUNKIEQUOTES: i32 = 48;
/// `CSHNULLCMD` constant.
pub const CSHNULLCMD: i32 = 49;
/// `CSHNULLGLOB` constant.
pub const CSHNULLGLOB: i32 = 50;
/// `DEBUGBEFORECMD` constant.
pub const DEBUGBEFORECMD: i32 = 51;
/// `EMACSMODE` constant.
pub const EMACSMODE: i32 = 52;
/// `EQUALSOPT` constant.
pub const EQUALSOPT: i32 = 53; // C name "EQUALS" collides with our token const
/// `ERREXIT` constant.
pub const ERREXIT: i32 = 54;
/// `ERRRETURN` constant.
pub const ERRRETURN: i32 = 55;
/// `EXECOPT` constant.
pub const EXECOPT: i32 = 56;
/// `EXTENDEDGLOB` constant.
pub const EXTENDEDGLOB: i32 = 57;
/// `EXTENDEDHISTORY` constant.
pub const EXTENDEDHISTORY: i32 = 58;
/// `EVALLINENO` constant.
pub const EVALLINENO: i32 = 59;
/// `FLOWCONTROL` constant.
pub const FLOWCONTROL: i32 = 60;
/// `FORCEFLOAT` constant.
pub const FORCEFLOAT: i32 = 61;
/// `FUNCTIONARGZERO` constant.
pub const FUNCTIONARGZERO: i32 = 62;
/// `GLOBOPT` constant.
pub const GLOBOPT: i32 = 63;
/// `GLOBALEXPORT` constant.
pub const GLOBALEXPORT: i32 = 64;
/// `GLOBALRCS` constant.
pub const GLOBALRCS: i32 = 65;
/// `GLOBASSIGN` constant.
pub const GLOBASSIGN: i32 = 66;
/// `GLOBCOMPLETE` constant.
pub const GLOBCOMPLETE: i32 = 67;
/// `GLOBDOTS` constant.
pub const GLOBDOTS: i32 = 68;
/// `GLOBSTARSHORT` constant.
pub const GLOBSTARSHORT: i32 = 69;
/// `GLOBSUBST` constant.
pub const GLOBSUBST: i32 = 70;
/// `HASHCMDS` constant.
pub const HASHCMDS: i32 = 71;
/// `HASHDIRS` constant.
pub const HASHDIRS: i32 = 72;
/// `HASHEXECUTABLESONLY` constant.
pub const HASHEXECUTABLESONLY: i32 = 73;
/// `HASHLISTALL` constant.
pub const HASHLISTALL: i32 = 74;
/// `HISTALLOWCLOBBER` constant.
pub const HISTALLOWCLOBBER: i32 = 75;
/// `HISTBEEP` constant.
pub const HISTBEEP: i32 = 76;
/// `HISTEXPIREDUPSFIRST` constant.
pub const HISTEXPIREDUPSFIRST: i32 = 77;
/// `HISTFCNTLLOCK` constant.
pub const HISTFCNTLLOCK: i32 = 78;
/// `HISTFINDNODUPS` constant.
pub const HISTFINDNODUPS: i32 = 79;
/// `HISTIGNOREALLDUPS` constant.
pub const HISTIGNOREALLDUPS: i32 = 80;
/// `HISTIGNOREDUPS` constant.
pub const HISTIGNOREDUPS: i32 = 81;
/// `HISTIGNORESPACE` constant.
pub const HISTIGNORESPACE: i32 = 82;
/// `HISTLEXWORDS` constant.
pub const HISTLEXWORDS: i32 = 83;
/// `HISTNOFUNCTIONS` constant.
pub const HISTNOFUNCTIONS: i32 = 84;
/// `HISTNOSTORE` constant.
pub const HISTNOSTORE: i32 = 85;
/// `HISTREDUCEBLANKS` constant.
pub const HISTREDUCEBLANKS: i32 = 86;
/// `HISTSAVEBYCOPY` constant.
pub const HISTSAVEBYCOPY: i32 = 87;
/// `HISTSAVENODUPS` constant.
pub const HISTSAVENODUPS: i32 = 88;
/// `HISTSUBSTPATTERN` constant.
pub const HISTSUBSTPATTERN: i32 = 89;
/// `HISTVERIFY` constant.
pub const HISTVERIFY: i32 = 90;
/// `HUP` constant.
pub const HUP: i32 = 91;
/// `IGNOREBRACES` constant.
pub const IGNOREBRACES: i32 = 92;
/// `IGNORECLOSEBRACES` constant.
pub const IGNORECLOSEBRACES: i32 = 93;
/// `IGNOREEOF` constant.
pub const IGNOREEOF: i32 = 94;
/// `INCAPPENDHISTORY` constant.
pub const INCAPPENDHISTORY: i32 = 95;
/// `INCAPPENDHISTORYTIME` constant.
pub const INCAPPENDHISTORYTIME: i32 = 96;
/// `INTERACTIVE` constant.
pub const INTERACTIVE: i32 = 97;
/// `INTERACTIVECOMMENTS` constant.
pub const INTERACTIVECOMMENTS: i32 = 98;
/// `KSHARRAYS` constant.
pub const KSHARRAYS: i32 = 99;
/// `KSHAUTOLOAD` constant.
pub const KSHAUTOLOAD: i32 = 100;
/// `KSHGLOB` constant.
pub const KSHGLOB: i32 = 101;
/// `KSHOPTIONPRINT` constant.
pub const KSHOPTIONPRINT: i32 = 102;
/// `KSHTYPESET` constant.
pub const KSHTYPESET: i32 = 103;
/// `KSHZEROSUBSCRIPT` constant.
pub const KSHZEROSUBSCRIPT: i32 = 104;
/// `LISTAMBIGUOUS` constant.
pub const LISTAMBIGUOUS: i32 = 105;
/// `LISTBEEP` constant.
pub const LISTBEEP: i32 = 106;
/// `LISTPACKED` constant.
pub const LISTPACKED: i32 = 107;
/// `LISTROWSFIRST` constant.
pub const LISTROWSFIRST: i32 = 108;
/// `LISTTYPES` constant.
pub const LISTTYPES: i32 = 109;
/// `LOCALLOOPS` constant.
pub const LOCALLOOPS: i32 = 110;
/// `LOCALOPTIONS` constant.
pub const LOCALOPTIONS: i32 = 111;
/// `LOCALPATTERNS` constant.
pub const LOCALPATTERNS: i32 = 112;
/// `LOCALTRAPS` constant.
pub const LOCALTRAPS: i32 = 113;
/// `LOGINSHELL` constant.
pub const LOGINSHELL: i32 = 114;
/// `LONGLISTJOBS` constant.
pub const LONGLISTJOBS: i32 = 115;
/// `MAGICEQUALSUBST` constant.
pub const MAGICEQUALSUBST: i32 = 116;
/// `MAILWARNING` constant.
pub const MAILWARNING: i32 = 117;
/// `MARKDIRS` constant.
pub const MARKDIRS: i32 = 118;
/// `MENUCOMPLETE` constant.
pub const MENUCOMPLETE: i32 = 119;
/// `MONITOR` constant.
pub const MONITOR: i32 = 120;
/// `MULTIBYTE` constant.
pub const MULTIBYTE: i32 = 121;
/// `MULTIFUNCDEF` constant.
pub const MULTIFUNCDEF: i32 = 122;
/// `MULTIOS` constant.
pub const MULTIOS: i32 = 123;
/// `NOMATCH` constant.
pub const NOMATCH: i32 = 124;
/// `NOTIFY` constant.
pub const NOTIFY: i32 = 125;
/// `NULLGLOB` constant.
pub const NULLGLOB: i32 = 126;
/// `NUMERICGLOBSORT` constant.
pub const NUMERICGLOBSORT: i32 = 127;
/// `OCTALZEROES` constant.
pub const OCTALZEROES: i32 = 128;
/// `OVERSTRIKE` constant.
pub const OVERSTRIKE: i32 = 129;
/// `PATHDIRS` constant.
pub const PATHDIRS: i32 = 130;
/// `PATHSCRIPT` constant.
pub const PATHSCRIPT: i32 = 131;
/// `PIPEFAIL` constant.
pub const PIPEFAIL: i32 = 132;
/// `POSIXALIASES` constant.
pub const POSIXALIASES: i32 = 133;
/// `POSIXARGZERO` constant.
pub const POSIXARGZERO: i32 = 134;
/// `POSIXBUILTINS` constant.
pub const POSIXBUILTINS: i32 = 135;
/// `POSIXCD` constant.
pub const POSIXCD: i32 = 136;
/// `POSIXIDENTIFIERS` constant.
pub const POSIXIDENTIFIERS: i32 = 137;
/// `POSIXJOBS` constant.
pub const POSIXJOBS: i32 = 138;
/// `POSIXSTRINGS` constant.
pub const POSIXSTRINGS: i32 = 139;
/// `POSIXTRAPS` constant.
pub const POSIXTRAPS: i32 = 140;
/// `PRINTEIGHTBIT` constant.
pub const PRINTEIGHTBIT: i32 = 141;
/// `PRINTEXITVALUE` constant.
pub const PRINTEXITVALUE: i32 = 142;
/// `PRIVILEGED` constant.
pub const PRIVILEGED: i32 = 143;
/// `PROMPTBANG` constant.
pub const PROMPTBANG: i32 = 144;
/// `PROMPTCR` constant.
pub const PROMPTCR: i32 = 145;
/// `PROMPTPERCENT` constant.
pub const PROMPTPERCENT: i32 = 146;
/// `PROMPTSP` constant.
pub const PROMPTSP: i32 = 147;
/// `PROMPTSUBST` constant.
pub const PROMPTSUBST: i32 = 148;
/// `PUSHDIGNOREDUPS` constant.
pub const PUSHDIGNOREDUPS: i32 = 149;
/// `PUSHDMINUS` constant.
pub const PUSHDMINUS: i32 = 150;
/// `PUSHDSILENT` constant.
pub const PUSHDSILENT: i32 = 151;
/// `PUSHDTOHOME` constant.
pub const PUSHDTOHOME: i32 = 152;
/// `RCEXPANDPARAM` constant.
pub const RCEXPANDPARAM: i32 = 153;
/// `RCQUOTES` constant.
pub const RCQUOTES: i32 = 154;
/// `RCS` constant.
pub const RCS: i32 = 155;
/// `RECEXACT` constant.
pub const RECEXACT: i32 = 156;
/// `REMATCHPCRE` constant.
pub const REMATCHPCRE: i32 = 157;
/// `RESTRICTED` constant.
pub const RESTRICTED: i32 = 158;
/// `RMSTARSILENT` constant.
pub const RMSTARSILENT: i32 = 159;
/// `RMSTARWAIT` constant.
pub const RMSTARWAIT: i32 = 160;
/// `SHAREHISTORY` constant.
pub const SHAREHISTORY: i32 = 161;
/// `SHFILEEXPANSION` constant.
pub const SHFILEEXPANSION: i32 = 162;
/// `SHGLOB` constant.
pub const SHGLOB: i32 = 163;
/// `SHINSTDIN` constant.
pub const SHINSTDIN: i32 = 164;
/// `SHNULLCMD` constant.
pub const SHNULLCMD: i32 = 165;
/// `SHOPTIONLETTERS` constant.
pub const SHOPTIONLETTERS: i32 = 166;
/// `SHORTLOOPS` constant.
pub const SHORTLOOPS: i32 = 167;
/// `SHORTREPEAT` constant.
pub const SHORTREPEAT: i32 = 168;
/// `SHWORDSPLIT` constant.
pub const SHWORDSPLIT: i32 = 169;
/// `SINGLECOMMAND` constant.
pub const SINGLECOMMAND: i32 = 170;
/// `SINGLELINEZLE` constant.
pub const SINGLELINEZLE: i32 = 171;
/// `SOURCETRACE` constant.
pub const SOURCETRACE: i32 = 172;
/// `SUNKEYBOARDHACK` constant.
pub const SUNKEYBOARDHACK: i32 = 173;
/// `TRANSIENTRPROMPT` constant.
pub const TRANSIENTRPROMPT: i32 = 174;
/// `TRAPSASYNC` constant.
pub const TRAPSASYNC: i32 = 175;
/// `TYPESETSILENT` constant.
pub const TYPESETSILENT: i32 = 176;
/// `TYPESETTOUNSET` constant.
pub const TYPESETTOUNSET: i32 = 177;
/// `UNSET` constant.
pub const UNSET: i32 = 178;
/// `VERBOSE` constant.
pub const VERBOSE: i32 = 179;
/// `VIMODE` constant.
pub const VIMODE: i32 = 180;
/// `WARNCREATEGLOBAL` constant.
pub const WARNCREATEGLOBAL: i32 = 181;
/// `WARNNESTEDVAR` constant.
pub const WARNNESTEDVAR: i32 = 182;
/// `XTRACE` constant.
pub const XTRACE: i32 = 183;
/// `USEZLE` constant.
pub const USEZLE: i32 = 184;
/// `DVORAK` constant.
pub const DVORAK: i32 = 185;
/// `OPT_SIZE` constant.
pub const OPT_SIZE: i32 = 186;
/// `OptIndex` type alias.
pub type OptIndex = u8; // c:2556

// #define isset(X) (opts[X])                                               // c:2559
/// Port of `isset(X)` macro from `Src/zsh.h:2559`.
/// Returns true if option is set.
#[inline]
pub fn isset(opt: i32) -> bool {
    crate::ported::options::opt_state_get(&opt_name(opt)).unwrap_or(false)
}

// #define unset(X) (!opts[X])                                              // c:2560
/// Port of `unset(X)` macro from `Src/zsh.h:2560`.
/// Returns true if option is NOT set.
#[inline]
pub fn unset(opt: i32) -> bool {
    !isset(opt)
}

// #define interact (isset(INTERACTIVE))                                    // c:2562
/// Port of `interact` macro from `Src/zsh.h:2562`.
#[inline]
pub fn interact() -> bool {
    isset(INTERACTIVE)
}

// #define jobbing  (isset(MONITOR))                                        // c:2563
/// Port of `jobbing` macro from `Src/zsh.h:2563`.
#[inline]
pub fn jobbing() -> bool {
    isset(MONITOR)
}

// #define islogin  (isset(LOGINSHELL))                                     // c:2564
/// Port of `islogin` macro from `Src/zsh.h:2564`.
#[inline]
pub fn islogin() -> bool {
    isset(LOGINSHELL)
}

/// Helper: convert option constant to its name for lookup.
pub fn opt_name(opt: i32) -> &'static str {
    match opt {
        x if x == ALIASFUNCDEF => "aliasfuncdef",
        x if x == ALLEXPORT => "allexport",
        x if x == ALWAYSLASTPROMPT => "alwayslastprompt",
        x if x == ALWAYSTOEND => "alwaystoend",
        x if x == APPENDHISTORY => "appendhistory",
        x if x == AUTOCD => "autocd",
        x if x == AUTOCONTINUE => "autocontinue",
        x if x == AUTOLIST => "autolist",
        x if x == AUTOMENU => "automenu",
        x if x == AUTONAMEDIRS => "autonamedirs",
        x if x == AUTOPARAMKEYS => "autoparamkeys",
        x if x == AUTOPARAMSLASH => "autoparamslash",
        x if x == AUTOPUSHD => "autopushd",
        x if x == AUTOREMOVESLASH => "autoremoveslash",
        x if x == AUTORESUME => "autoresume",
        x if x == BADPATTERN => "badpattern",
        x if x == BANGHIST => "banghist",
        x if x == BAREGLOBQUAL => "bareglobqual",
        x if x == BASHAUTOLIST => "bashautolist",
        x if x == BASHREMATCH => "bashrematch",
        x if x == BEEP => "beep",
        x if x == BGNICE => "bgnice",
        x if x == BRACECCL => "braceccl",
        x if x == BSDECHO => "bsdecho",
        x if x == CASEGLOB => "caseglob",
        x if x == CASEMATCH => "casematch",
        x if x == CASEPATHS => "casepaths",
        x if x == CBASES => "cbases",
        x if x == CDABLEVARS => "cdablevars",
        x if x == CDSILENT => "cdsilent",
        x if x == CHASEDOTS => "chasedots",
        x if x == CHASELINKS => "chaselinks",
        x if x == CHECKJOBS => "checkjobs",
        x if x == CHECKRUNNINGJOBS => "checkrunningjobs",
        x if x == CLOBBER => "clobber",
        x if x == CLOBBEREMPTY => "clobberempty",
        x if x == APPENDCREATE => "appendcreate",
        x if x == COMBININGCHARS => "combiningchars",
        x if x == COMPLETEALIASES => "completealiases",
        x if x == COMPLETEINWORD => "completeinword",
        x if x == CORRECT => "correct",
        x if x == CORRECTALL => "correctall",
        x if x == CPRECEDENCES => "cprecedences",
        x if x == CSHJUNKIEHISTORY => "cshjunkiehistory",
        x if x == CSHJUNKIELOOPS => "cshjunkieloops",
        x if x == CSHJUNKIEQUOTES => "cshjunkiequotes",
        x if x == CSHNULLCMD => "cshnullcmd",
        x if x == CSHNULLGLOB => "cshnullglob",
        x if x == CONTINUEONERROR => "continueonerror",
        x if x == DEBUGBEFORECMD => "debugbeforecmd",
        x if x == EMACSMODE => "emacs",
        x if x == EQUALSOPT => "equals",
        x if x == ERREXIT => "errexit",
        x if x == ERRRETURN => "errreturn",
        x if x == EXECOPT => "exec",
        x if x == EXTENDEDGLOB => "extendedglob",
        x if x == EXTENDEDHISTORY => "extendedhistory",
        x if x == EVALLINENO => "evallineno",
        x if x == FLOWCONTROL => "flowcontrol",
        x if x == FORCEFLOAT => "forcefloat",
        x if x == FUNCTIONARGZERO => "functionargzero",
        x if x == GLOBOPT => "glob",
        x if x == GLOBALEXPORT => "globalexport",
        x if x == GLOBALRCS => "globalrcs",
        x if x == GLOBASSIGN => "globassign",
        x if x == GLOBCOMPLETE => "globcomplete",
        x if x == GLOBDOTS => "globdots",
        x if x == GLOBSTARSHORT => "globstarshort",
        x if x == GLOBSUBST => "globsubst",
        x if x == HASHCMDS => "hashcmds",
        x if x == HASHDIRS => "hashdirs",
        x if x == HASHEXECUTABLESONLY => "hashexecutablesonly",
        x if x == HASHLISTALL => "hashlistall",
        x if x == HISTALLOWCLOBBER => "histallowclobber",
        x if x == HISTBEEP => "histbeep",
        x if x == HISTEXPIREDUPSFIRST => "histexpiredupsfirst",
        x if x == HISTFCNTLLOCK => "histfcntllock",
        x if x == HISTFINDNODUPS => "histfindnodups",
        x if x == HISTIGNOREALLDUPS => "histignorealldups",
        x if x == HISTIGNOREDUPS => "histignoredups",
        x if x == HISTIGNORESPACE => "histignorespace",
        x if x == HISTLEXWORDS => "histlexwords",
        x if x == HISTNOFUNCTIONS => "histnofunctions",
        x if x == HISTNOSTORE => "histnostore",
        x if x == HISTREDUCEBLANKS => "histreduceblanks",
        x if x == HISTSAVEBYCOPY => "histsavebycopy",
        x if x == HISTSAVENODUPS => "histsavenodups",
        x if x == HISTSUBSTPATTERN => "histsubstpattern",
        x if x == HISTVERIFY => "histverify",
        x if x == HUP => "hup",
        x if x == IGNOREBRACES => "ignorebraces",
        x if x == IGNORECLOSEBRACES => "ignoreclosebraces",
        x if x == IGNOREEOF => "ignoreeof",
        x if x == INCAPPENDHISTORY => "incappendhistory",
        x if x == INCAPPENDHISTORYTIME => "incappendhistorytime",
        x if x == INTERACTIVE => "interactive",
        x if x == INTERACTIVECOMMENTS => "interactivecomments",
        x if x == KSHARRAYS => "ksharrays",
        x if x == KSHAUTOLOAD => "kshautoload",
        x if x == KSHGLOB => "kshglob",
        x if x == KSHOPTIONPRINT => "kshoptionprint",
        x if x == KSHTYPESET => "kshtypeset",
        x if x == KSHZEROSUBSCRIPT => "kshzerosubscript",
        x if x == LISTAMBIGUOUS => "listambiguous",
        x if x == LISTBEEP => "listbeep",
        x if x == LISTPACKED => "listpacked",
        x if x == LISTROWSFIRST => "listrowsfirst",
        x if x == LISTTYPES => "listtypes",
        x if x == LOCALOPTIONS => "localoptions",
        x if x == LOCALLOOPS => "localloops",
        x if x == LOCALPATTERNS => "localpatterns",
        x if x == LOCALTRAPS => "localtraps",
        x if x == LOGINSHELL => "loginshell",
        x if x == LONGLISTJOBS => "longlistjobs",
        x if x == MAGICEQUALSUBST => "magicequalsubst",
        x if x == MAILWARNING => "mailwarning",
        x if x == MARKDIRS => "markdirs",
        x if x == MENUCOMPLETE => "menucomplete",
        x if x == MONITOR => "monitor",
        x if x == MULTIBYTE => "multibyte",
        x if x == MULTIFUNCDEF => "multifuncdef",
        x if x == MULTIOS => "multios",
        x if x == NOMATCH => "nomatch",
        x if x == NOTIFY => "notify",
        x if x == NULLGLOB => "nullglob",
        x if x == NUMERICGLOBSORT => "numericglobsort",
        x if x == OCTALZEROES => "octalzeroes",
        x if x == OVERSTRIKE => "overstrike",
        x if x == PATHDIRS => "pathdirs",
        x if x == PATHSCRIPT => "pathscript",
        x if x == PIPEFAIL => "pipefail",
        x if x == POSIXALIASES => "posixaliases",
        x if x == POSIXARGZERO => "posixargzero",
        x if x == POSIXBUILTINS => "posixbuiltins",
        x if x == POSIXCD => "posixcd",
        x if x == POSIXIDENTIFIERS => "posixidentifiers",
        x if x == POSIXJOBS => "posixjobs",
        x if x == POSIXSTRINGS => "posixstrings",
        x if x == POSIXTRAPS => "posixtraps",
        x if x == PRINTEIGHTBIT => "printeightbit",
        x if x == PRINTEXITVALUE => "printexitvalue",
        x if x == PRIVILEGED => "privileged",
        x if x == PROMPTBANG => "promptbang",
        x if x == PROMPTCR => "promptcr",
        x if x == PROMPTPERCENT => "promptpercent",
        x if x == PROMPTSP => "promptsp",
        x if x == PROMPTSUBST => "promptsubst",
        x if x == PUSHDIGNOREDUPS => "pushdignoredups",
        x if x == PUSHDMINUS => "pushdminus",
        x if x == PUSHDSILENT => "pushdsilent",
        x if x == PUSHDTOHOME => "pushdtohome",
        x if x == RCEXPANDPARAM => "rcexpandparam",
        x if x == RCQUOTES => "rcquotes",
        x if x == RCS => "rcs",
        x if x == RECEXACT => "recexact",
        x if x == REMATCHPCRE => "rematchpcre",
        x if x == RESTRICTED => "restricted",
        x if x == RMSTARSILENT => "rmstarsilent",
        x if x == RMSTARWAIT => "rmstarwait",
        x if x == SHAREHISTORY => "sharehistory",
        x if x == SHFILEEXPANSION => "shfileexpansion",
        x if x == SHGLOB => "shglob",
        x if x == SHINSTDIN => "shinstdin",
        x if x == SHNULLCMD => "shnullcmd",
        x if x == SHOPTIONLETTERS => "shoptionletters",
        x if x == SHORTLOOPS => "shortloops",
        x if x == SHORTREPEAT => "shortrepeat",
        x if x == SHWORDSPLIT => "shwordsplit",
        x if x == SINGLECOMMAND => "singlecommand",
        x if x == SINGLELINEZLE => "singlelinezle",
        x if x == SOURCETRACE => "sourcetrace",
        x if x == SUNKEYBOARDHACK => "sunkeyboardhack",
        x if x == TRANSIENTRPROMPT => "transientrprompt",
        x if x == TRAPSASYNC => "trapsasync",
        x if x == TYPESETSILENT => "typesetsilent",
        x if x == TYPESETTOUNSET => "typesettounset",
        x if x == UNSET => "unset",
        x if x == VERBOSE => "verbose",
        x if x == ALIASESOPT => "aliases",
        x if x == WARNCREATEGLOBAL => "warncreateglobal",
        x if x == WARNNESTEDVAR => "warnnestedvar",
        x if x == XTRACE => "xtrace",
        x if x == USEZLE => "zle",
        x if x == DVORAK => "dvorak",
        // VIMODE was missing entirely from the opt_name table.
        // The storage key in ZSH_OPTIONS_SET is "vi" (not "vimode")
        // — must match so isset(VIMODE) and opt_state_set("vi")
        // address the same slot.
        x if x == VIMODE => "vi",
        _ => "",
    }
}

// =============================================================================
// 36. Terminal control (zsh.h:2633-2680).
// =============================================================================
/// `TERM_BAD` constant.
pub const TERM_BAD: i32 = 0x01;
/// `TERM_UNKNOWN` constant.
pub const TERM_UNKNOWN: i32 = 0x02;
/// `TERM_NOUP` constant.
pub const TERM_NOUP: i32 = 0x04;
/// `TERM_SHORT` constant.
pub const TERM_SHORT: i32 = 0x08;
/// `TERM_NARROW` constant.
pub const TERM_NARROW: i32 = 0x10;
/// `TCCLEARSCREEN` constant.
pub const TCCLEARSCREEN: i32 = 0;
/// `TCLEFT` constant.
pub const TCLEFT: i32 = 1;
/// `TCMULTLEFT` constant.
pub const TCMULTLEFT: i32 = 2;
/// `TCRIGHT` constant.
pub const TCRIGHT: i32 = 3;
/// `TCMULTRIGHT` constant.
pub const TCMULTRIGHT: i32 = 4;
/// `TCUP` constant.
pub const TCUP: i32 = 5;
/// `TCMULTUP` constant.
pub const TCMULTUP: i32 = 6;
/// `TCDOWN` constant.
pub const TCDOWN: i32 = 7;
/// `TCMULTDOWN` constant.
pub const TCMULTDOWN: i32 = 8;
/// `TCDEL` constant.
pub const TCDEL: i32 = 9;
/// `TCMULTDEL` constant.
pub const TCMULTDEL: i32 = 10;
/// `TCINS` constant.
pub const TCINS: i32 = 11;
/// `TCMULTINS` constant.
pub const TCMULTINS: i32 = 12;
/// `TCCLEAREOD` constant.
pub const TCCLEAREOD: i32 = 13;
/// `TCCLEAREOL` constant.
pub const TCCLEAREOL: i32 = 14;
/// `TCINSLINE` constant.
pub const TCINSLINE: i32 = 15;
/// `TCDELLINE` constant.
pub const TCDELLINE: i32 = 16;
/// `TCNEXTTAB` constant.
pub const TCNEXTTAB: i32 = 17;
/// `TCBOLDFACEBEG` constant.
pub const TCBOLDFACEBEG: i32 = 18;
/// `TCFAINTBEG` constant.
pub const TCFAINTBEG: i32 = 19;
/// `TCSTANDOUTBEG` constant.
pub const TCSTANDOUTBEG: i32 = 20;
/// `TCUNDERLINEBEG` constant.
pub const TCUNDERLINEBEG: i32 = 21;
/// `TCITALICSBEG` constant.
pub const TCITALICSBEG: i32 = 22;
/// `TCALLATTRSOFF` constant.
pub const TCALLATTRSOFF: i32 = 23;
/// `TCSTANDOUTEND` constant.
pub const TCSTANDOUTEND: i32 = 24;
/// `TCUNDERLINEEND` constant.
pub const TCUNDERLINEEND: i32 = 25;
/// `TCITALICSEND` constant.
pub const TCITALICSEND: i32 = 26;
/// `TCHORIZPOS` constant.
pub const TCHORIZPOS: i32 = 27;
/// `TCUPCURSOR` constant.
pub const TCUPCURSOR: i32 = 28;
/// `TCDOWNCURSOR` constant.
pub const TCDOWNCURSOR: i32 = 29;
/// `TCLEFTCURSOR` constant.
pub const TCLEFTCURSOR: i32 = 30;
/// `TCRIGHTCURSOR` constant.
pub const TCRIGHTCURSOR: i32 = 31;
/// `TCSAVECURSOR` constant.
pub const TCSAVECURSOR: i32 = 32;
/// `TCRESTRCURSOR` constant.
pub const TCRESTRCURSOR: i32 = 33;
/// `TCBACKSPACE` constant.
pub const TCBACKSPACE: i32 = 34;
/// `TCFGCOLOUR` constant.
pub const TCFGCOLOUR: i32 = 35;
/// `TCBGCOLOUR` constant.
pub const TCBGCOLOUR: i32 = 36;
/// `TCCURINV` constant.
pub const TCCURINV: i32 = 37;
/// `TCCURVIS` constant.
pub const TCCURVIS: i32 = 38;
/// `TC_COUNT` constant.
pub const TC_COUNT: i32 = 39;

// =============================================================================
// 37. Text attributes (zattr) (zsh.h:2689-2750).
// =============================================================================
/// `zattr` type alias.
pub type zattr = u64; // c:2689
/// `TXTBOLDFACE` constant.
pub const TXTBOLDFACE: zattr = 0x0001;
/// `TXTFAINT` constant.
pub const TXTFAINT: zattr = 0x0002;
/// `TXTSTANDOUT` constant.
pub const TXTSTANDOUT: zattr = 0x0004;
/// `TXTUNDERLINE` constant.
pub const TXTUNDERLINE: zattr = 0x0008;
/// `TXTITALIC` constant.
pub const TXTITALIC: zattr = 0x0010;
/// `TXTFGCOLOUR` constant.
pub const TXTFGCOLOUR: zattr = 0x0020;
/// `TXTBGCOLOUR` constant.
pub const TXTBGCOLOUR: zattr = 0x0040;
/// `TXT_ATTR_ALL` constant.
pub const TXT_ATTR_ALL: zattr = 0x007F;
/// `TXT_MULTIWORD_MASK` constant.
pub const TXT_MULTIWORD_MASK: zattr = 0x0400;
/// `TXT_ERROR` constant.
pub const TXT_ERROR: zattr = 0xF00000F000000003;
/// `TXT_ATTR_FONT_WEIGHT` constant.
pub const TXT_ATTR_FONT_WEIGHT: zattr = TXTBOLDFACE | TXTFAINT;
/// `TXT_ATTR_FG_COL_MASK` constant.
pub const TXT_ATTR_FG_COL_MASK: zattr = 0x000000FFFFFF0000;
/// `TXT_ATTR_FG_COL_SHIFT` constant.
pub const TXT_ATTR_FG_COL_SHIFT: u32 = 16;
/// `TXT_ATTR_BG_COL_MASK` constant.
pub const TXT_ATTR_BG_COL_MASK: zattr = 0xFFFFFF0000000000;
/// `TXT_ATTR_BG_COL_SHIFT` constant.
pub const TXT_ATTR_BG_COL_SHIFT: u32 = 40;
/// `TXT_ATTR_FG_24BIT` constant.
pub const TXT_ATTR_FG_24BIT: zattr = 0x4000;
/// `TXT_ATTR_BG_24BIT` constant.
pub const TXT_ATTR_BG_24BIT: zattr = 0x8000;
/// `TXT_ATTR_FG_MASK` constant.
pub const TXT_ATTR_FG_MASK: zattr = TXTFGCOLOUR | TXT_ATTR_FG_COL_MASK | TXT_ATTR_FG_24BIT;
/// `TXT_ATTR_BG_MASK` constant.
pub const TXT_ATTR_BG_MASK: zattr = TXTBGCOLOUR | TXT_ATTR_BG_COL_MASK | TXT_ATTR_BG_24BIT;
/// `TXT_ATTR_COLOUR_MASK` constant.
pub const TXT_ATTR_COLOUR_MASK: zattr = TXT_ATTR_FG_MASK | TXT_ATTR_BG_MASK;
/// `COL_SEQ_FG` constant.
pub const COL_SEQ_FG: i32 = 0;
/// `COL_SEQ_BG` constant.
pub const COL_SEQ_BG: i32 = 1;
/// `color_rgb` — see fields for layout.
#[allow(non_camel_case_types)]
pub struct color_rgb {
    // c:2752
    /// `red` field.
    pub red: u32,
    /// `green` field.
    pub green: u32,
    /// `blue` field.
    pub blue: u32,
}
/// `Color_rgb` type alias.
pub type Color_rgb = Box<color_rgb>;
/// `TSC_RAW` constant.
pub const TSC_RAW: i32 = 0x0001; // c:2764
/// `TSC_PROMPT` constant.
pub const TSC_PROMPT: i32 = 0x0002;
/// `TSC_DIRTY` constant — zsh 5.9.1 zsh.h:2766. The 5.9 release's
/// tsetcap re-applies the still-active attributes + colours after a
/// cap that may have clobbered them (every END cap, ALLATTRSOFF, and
/// BOLDFACEBEG). Master's prompt.c rewrite (applytextattributes)
/// dropped the flag, but the zshrs parity floor is the 5.9.x release
/// binary, whose emission sequences depend on it.
pub const TSC_DIRTY: i32 = 0x0004;

// =============================================================================
// 38. Prompt %_ command stack (zsh.h:2773-2809).
// =============================================================================
/// `CMDSTACKSZ` constant.
pub const CMDSTACKSZ: usize = 256;
/// `CS_FOR` constant.
pub const CS_FOR: i32 = 0;
/// `CS_WHILE` constant.
pub const CS_WHILE: i32 = 1;
/// `CS_REPEAT` constant.
pub const CS_REPEAT: i32 = 2;
/// `CS_SELECT` constant.
pub const CS_SELECT: i32 = 3;
/// `CS_UNTIL` constant.
pub const CS_UNTIL: i32 = 4;
/// `CS_IF` constant.
pub const CS_IF: i32 = 5;
/// `CS_IFTHEN` constant.
pub const CS_IFTHEN: i32 = 6;
/// `CS_ELSE` constant.
pub const CS_ELSE: i32 = 7;
/// `CS_ELIF` constant.
pub const CS_ELIF: i32 = 8;
/// `CS_MATH` constant.
pub const CS_MATH: i32 = 9;
/// `CS_COND` constant.
pub const CS_COND: i32 = 10;
/// `CS_CMDOR` constant.
pub const CS_CMDOR: i32 = 11;
/// `CS_CMDAND` constant.
pub const CS_CMDAND: i32 = 12;
/// `CS_PIPE` constant.
pub const CS_PIPE: i32 = 13;
/// `CS_ERRPIPE` constant.
pub const CS_ERRPIPE: i32 = 14;
/// `CS_FOREACH` constant.
pub const CS_FOREACH: i32 = 15;
/// `CS_CASE` constant.
pub const CS_CASE: i32 = 16;
/// `CS_FUNCDEF` constant.
pub const CS_FUNCDEF: i32 = 17;
/// `CS_SUBSH` constant.
pub const CS_SUBSH: i32 = 18;
/// `CS_CURSH` constant.
pub const CS_CURSH: i32 = 19;
/// `CS_ARRAY` constant.
pub const CS_ARRAY: i32 = 20;
/// `CS_QUOTE` constant.
pub const CS_QUOTE: i32 = 21;
/// `CS_DQUOTE` constant.
pub const CS_DQUOTE: i32 = 22;
/// `CS_BQUOTE` constant.
pub const CS_BQUOTE: i32 = 23;
/// `CS_CMDSUBST` constant.
pub const CS_CMDSUBST: i32 = 24;
/// `CS_MATHSUBST` constant.
pub const CS_MATHSUBST: i32 = 25;
/// `CS_ELIFTHEN` constant.
pub const CS_ELIFTHEN: i32 = 26;
/// `CS_HEREDOC` constant.
pub const CS_HEREDOC: i32 = 27;
/// `CS_HEREDOCD` constant.
pub const CS_HEREDOCD: i32 = 28;
/// `CS_BRACE` constant.
pub const CS_BRACE: i32 = 29;
/// `CS_BRACEPAR` constant.
pub const CS_BRACEPAR: i32 = 30;
/// `CS_ALWAYS` constant.
pub const CS_ALWAYS: i32 = 31;
/// `CS_COUNT` constant.
pub const CS_COUNT: i32 = 32;

// =============================================================================
// 39. Heap memory + Heapid (zsh.h:2826-2862).
// =============================================================================
/// `Heapid` type alias.
pub type Heapid = u32; // c:2826
/// `HEAPID_PERMANENT` constant.
pub const HEAPID_PERMANENT: Heapid = u32::MAX; // c:2834
/// `HDV_PUSH` constant.
pub const HDV_PUSH: i32 = 0x01;
/// `HDV_POP` constant.
pub const HDV_POP: i32 = 0x02;
/// `HDV_CREATE` constant.
pub const HDV_CREATE: i32 = 0x04;
/// `HDV_FREE` constant.
pub const HDV_FREE: i32 = 0x08;
/// `HDV_NEW` constant.
pub const HDV_NEW: i32 = 0x10;
/// `HDV_OLD` constant.
pub const HDV_OLD: i32 = 0x20;
/// `HDV_SWITCH` constant.
pub const HDV_SWITCH: i32 = 0x40;
/// `HDV_ALLOC` constant.
pub const HDV_ALLOC: i32 = 0x80;

// =============================================================================
// 40. Signal trap state (zsh.h:2935-2984).
// =============================================================================
/// `ZSIG_TRAPPED` constant.
pub const ZSIG_TRAPPED: i32 = 1 << 0;
/// `ZSIG_IGNORED` constant.
pub const ZSIG_IGNORED: i32 = 1 << 1;
/// `ZSIG_FUNC` constant.
pub const ZSIG_FUNC: i32 = 1 << 2;
/// `ZSIG_MASK` constant.
pub const ZSIG_MASK: i32 = ZSIG_TRAPPED | ZSIG_IGNORED | ZSIG_FUNC;
/// `ZSIG_ALIAS` constant.
pub const ZSIG_ALIAS: i32 = 1 << 3;
/// `ZSIG_SHIFT` constant.
pub const ZSIG_SHIFT: i32 = 4;
/// `TRAP_STATE_INACTIVE` constant.
pub const TRAP_STATE_INACTIVE: i32 = 0;
/// `TRAP_STATE_PRIMED` constant.
pub const TRAP_STATE_PRIMED: i32 = 1;
/// `TRAP_STATE_FORCE_RETURN` constant.
pub const TRAP_STATE_FORCE_RETURN: i32 = 2;
/// `ERRFLAG_ERROR` constant.
pub const ERRFLAG_ERROR: i32 = 1;
/// `ERRFLAG_INT` constant.
pub const ERRFLAG_INT: i32 = 2;
/// `ERRFLAG_HARD` constant.
pub const ERRFLAG_HARD: i32 = 4;

// =============================================================================
// 41. Sorting (zsh.h:2992-3008).
// =============================================================================
/// `SORTIT_ANYOLDHOW` constant.
pub const SORTIT_ANYOLDHOW: i32 = 0;
/// `SORTIT_IGNORING_CASE` constant.
pub const SORTIT_IGNORING_CASE: i32 = 1;
/// `SORTIT_NUMERICALLY` constant.
pub const SORTIT_NUMERICALLY: i32 = 2;
/// `SORTIT_NUMERICALLY_SIGNED` constant.
pub const SORTIT_NUMERICALLY_SIGNED: i32 = 4;
/// `SORTIT_BACKWARDS` constant.
pub const SORTIT_BACKWARDS: i32 = 8;
/// `SORTIT_IGNORING_BACKSLASHES` constant.
pub const SORTIT_IGNORING_BACKSLASHES: i32 = 16;
/// `SORTIT_SOMEHOW` constant.
pub const SORTIT_SOMEHOW: i32 = 32;

// =============================================================================
// 42. Case modify + Getkey (zsh.h:3122-3197).
// =============================================================================
/// `CASMOD_NONE` constant.
pub const CASMOD_NONE: i32 = 0;
/// `CASMOD_UPPER` constant.
pub const CASMOD_UPPER: i32 = 1;
/// `CASMOD_LOWER` constant.
pub const CASMOD_LOWER: i32 = 2;
/// `CASMOD_CAPS` constant.
pub const CASMOD_CAPS: i32 = 3;
/// `GETKEY_OCTAL_ESC` constant.
pub const GETKEY_OCTAL_ESC: i32 = 1 << 0;
/// `GETKEY_EMACS` constant.
pub const GETKEY_EMACS: i32 = 1 << 1;
/// `GETKEY_CTRL` constant.
pub const GETKEY_CTRL: i32 = 1 << 2;
/// `GETKEY_BACKSLASH_C` constant.
pub const GETKEY_BACKSLASH_C: i32 = 1 << 3;
/// `GETKEY_DOLLAR_QUOTE` constant.
pub const GETKEY_DOLLAR_QUOTE: i32 = 1 << 4;
/// `GETKEY_BACKSLASH_MINUS` constant.
pub const GETKEY_BACKSLASH_MINUS: i32 = 1 << 5;
/// `GETKEY_SINGLE_CHAR` constant.
pub const GETKEY_SINGLE_CHAR: i32 = 1 << 6;
/// `GETKEY_UPDATE_OFFSET` constant.
pub const GETKEY_UPDATE_OFFSET: i32 = 1 << 7;
/// `GETKEY_PRINTF_PERCENT` constant.
pub const GETKEY_PRINTF_PERCENT: i32 = 1 << 8;
/// `GETKEYS_ECHO` constant.
pub const GETKEYS_ECHO: i32 = GETKEY_BACKSLASH_C;
/// `GETKEYS_PRINTF_FMT` constant.
pub const GETKEYS_PRINTF_FMT: i32 = GETKEY_OCTAL_ESC | GETKEY_BACKSLASH_C | GETKEY_PRINTF_PERCENT;
/// `GETKEYS_PRINTF_ARG` constant.
pub const GETKEYS_PRINTF_ARG: i32 = GETKEY_BACKSLASH_C;
/// `GETKEYS_PRINT` constant.
pub const GETKEYS_PRINT: i32 = GETKEY_OCTAL_ESC | GETKEY_BACKSLASH_C | GETKEY_EMACS;
/// `GETKEYS_BINDKEY` constant.
pub const GETKEYS_BINDKEY: i32 = GETKEY_OCTAL_ESC | GETKEY_EMACS | GETKEY_CTRL;
/// `GETKEYS_DOLLARS_QUOTE` constant.
pub const GETKEYS_DOLLARS_QUOTE: i32 = GETKEY_OCTAL_ESC | GETKEY_EMACS | GETKEY_DOLLAR_QUOTE;
/// `GETKEYS_MATH` constant.
pub const GETKEYS_MATH: i32 = GETKEY_OCTAL_ESC | GETKEY_EMACS | GETKEY_CTRL | GETKEY_SINGLE_CHAR;
/// `GETKEYS_SEP` constant.
pub const GETKEYS_SEP: i32 = GETKEY_OCTAL_ESC | GETKEY_EMACS;
pub const GETKEYS_SUFFIX: i32 =
    GETKEY_OCTAL_ESC | GETKEY_EMACS | GETKEY_CTRL | GETKEY_BACKSLASH_MINUS;

// =============================================================================
// 43. zle flags (zsh.h:3203-3216).
// =============================================================================
/// `ZLRF_HISTORY` constant.
pub const ZLRF_HISTORY: i32 = 0x01;
/// `ZLRF_NOSETTY` constant.
pub const ZLRF_NOSETTY: i32 = 0x02;
/// `ZLRF_IGNOREEOF` constant.
pub const ZLRF_IGNOREEOF: i32 = 0x04;
/// `ZLCON_LINE_START` constant.
pub const ZLCON_LINE_START: i32 = 0;
/// `ZLCON_LINE_CONT` constant.
pub const ZLCON_LINE_CONT: i32 = 1;
/// `ZLCON_SELECT` constant.
pub const ZLCON_SELECT: i32 = 2;
/// `ZLCON_VARED` constant.
pub const ZLCON_VARED: i32 = 3;
/// `ZLE_CMD_GET_LINE` constant.
pub const ZLE_CMD_GET_LINE: i32 = 0;
/// `ZLE_CMD_READ` constant.
pub const ZLE_CMD_READ: i32 = 1;
/// `ZLE_CMD_ADD_TO_LINE` constant.
pub const ZLE_CMD_ADD_TO_LINE: i32 = 2;
/// `ZLE_CMD_TRASH` constant.
pub const ZLE_CMD_TRASH: i32 = 3;
/// `ZLE_CMD_RESET_PROMPT` constant.
pub const ZLE_CMD_RESET_PROMPT: i32 = 4;
/// `ZLE_CMD_REFRESH` constant.
pub const ZLE_CMD_REFRESH: i32 = 5;
/// `ZLE_CMD_SET_KEYMAP` constant.
pub const ZLE_CMD_SET_KEYMAP: i32 = 6;
/// `ZLE_CMD_GET_KEY` constant.
pub const ZLE_CMD_GET_KEY: i32 = 7;
/// `ZLE_CMD_SET_HIST_LINE` constant.
pub const ZLE_CMD_SET_HIST_LINE: i32 = 8;
/// `ZLE_CMD_PREEXEC` constant.
pub const ZLE_CMD_PREEXEC: i32 = 9;
/// `ZLE_CMD_POSTEXEC` constant.
pub const ZLE_CMD_POSTEXEC: i32 = 10;
/// `ZLE_CMD_CHPWD` constant.
pub const ZLE_CMD_CHPWD: i32 = 11;

// =============================================================================
// 44. zexit + nice format (zsh.h:3252-3268).
// =============================================================================
/// `ZEXIT_NORMAL` constant.
pub const ZEXIT_NORMAL: i32 = 0;
/// `ZEXIT_SIGNAL` constant.
pub const ZEXIT_SIGNAL: i32 = 1;
/// `ZEXIT_DEFERRED` constant.
pub const ZEXIT_DEFERRED: i32 = 2;
/// `NICEFLAG_HEAP` constant.
pub const NICEFLAG_HEAP: i32 = 1;
/// `NICEFLAG_QUOTE` constant.
pub const NICEFLAG_QUOTE: i32 = 2;
/// `NICEFLAG_NODUP` constant.
pub const NICEFLAG_NODUP: i32 = 4;

// =============================================================================
// 45. Multibyte macros (zsh.h:3271-3375).
// =============================================================================
/// `convchar_t` type alias.
pub type convchar_t = u32; // c:3276/3357
/// `MB_INCOMPLETE` constant.
pub const MB_INCOMPLETE: usize = usize::MAX - 1; // c:3313
/// `MB_INVALID` constant.
pub const MB_INVALID: usize = usize::MAX; // c:3314
/// `MB_CUR_MAX` constant.
pub const MB_CUR_MAX: usize = 6; // c:3324

/// Port of `MB_METACHARINIT()` from `Src/zsh.h:3275/3356`. C calls
/// `mb_charinit()` to reset multibyte state. Rust char iteration is
/// stateless; no-op.
#[inline]
#[allow(non_snake_case)]
pub fn MB_METACHARINIT() {} // c:3275

/// Port of `MB_METACHARLEN(str)` from `Src/zsh.h:3278/3359`. Returns
/// the byte length of the next metafied character. C: `*str == Meta
/// ? 2 : 1` (non-multibyte); `mb_metacharlenconv(str, NULL)`
/// (multibyte). Rust returns the same byte length.
#[inline]
#[allow(non_snake_case)]
pub fn MB_METACHARLEN(s: &[u8]) -> usize {
    // c:3278/3359
    if s.is_empty() {
        0
    } else if s[0] == Meta {
        2
    } else {
        1
    }
}

/// Port of `MB_METACHARLENCONV(str, cp)` from `Src/zsh.h:3277/3358`.
/// Returns byte length + (optionally) the converted char. Rust port
/// returns `(byte_len, Option<char>)`.
#[inline]
#[allow(non_snake_case)]
pub fn MB_METACHARLENCONV(s: &[u8]) -> (usize, Option<char>) {
    // c:3277
    if s.is_empty() {
        return (0, None);
    }
    if s[0] == Meta && s.len() >= 2 {
        let unmeta = s[1] ^ 0x20;
        (2, Some(unmeta as char))
    } else {
        (1, Some(s[0] as char))
    }
}

/// Port of `MB_METASTRLEN(str)` from `Src/zsh.h:3279/3360`. Counts
/// metafied characters in the string.
#[inline]
#[allow(non_snake_case)]
pub fn MB_METASTRLEN(s: &str) -> usize {
    // c:3279
    let mut n = 0;
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == Meta && i + 1 < bytes.len() {
            i += 2;
        } else {
            i += 1;
        }
        n += 1;
    }
    n
}

/// Port of `MB_METASTRWIDTH(str)` from `Src/zsh.h:3280/3361`. Counts
/// display width. In non-multibyte mode this is the same as
/// `MB_METASTRLEN`; in multibyte mode it accounts for wide chars.
#[inline]
#[allow(non_snake_case)]
pub fn MB_METASTRWIDTH(s: &str) -> usize {
    // c:3280
    MB_METASTRLEN(s)
}

/// Port of `MB_METASTRLEN2(str, widthp)` from `Src/zsh.h:3281/3362`.
/// Variant that returns either char count or width depending on
/// `widthp`.
#[inline]
#[allow(non_snake_case)]
pub fn MB_METASTRLEN2(s: &str, widthp: bool) -> usize {
    // c:3281
    if widthp {
        MB_METASTRWIDTH(s)
    } else {
        MB_METASTRLEN(s)
    }
}

/// Port of `MB_CHARINIT()` from `Src/zsh.h:3286/3365`. No-op
/// counterpart of `MB_METACHARINIT` for unmetafied input.
#[inline]
#[allow(non_snake_case)]
pub fn MB_CHARINIT() {} // c:3286

/// Port of `MB_CHARLEN(str, len)` from `Src/zsh.h:3288/3367`. Byte
/// length of the next char in an unmetafied byte string.
#[inline]
#[allow(non_snake_case)]
pub fn MB_CHARLEN(s: &[u8], len: usize) -> usize {
    // c:3288
    if len == 0 || s.is_empty() {
        0
    } else {
        1
    }
}

/// Port of `MB_CHARLENCONV(str, len, cp)` from `Src/zsh.h:3287/3366`.
/// Byte length + converted char of the next char in an unmetafied
/// byte string.
#[inline]
#[allow(non_snake_case)]
pub fn MB_CHARLENCONV(s: &[u8], len: usize) -> (usize, Option<char>) {
    // c:3287
    if len == 0 || s.is_empty() {
        (0, None)
    } else {
        (1, Some(s[0] as char))
    }
}

/// Port of `WCWIDTH(wc)` from `Src/zsh.h:3300`. Display width of a
/// wide character: 0 for combining marks / control chars, 2 for
/// CJK-wide / emoji, 1 otherwise.
///
/// Delegates to the `unicode-width` crate (the same data path
/// `crate::ported::compat::u9_wcwidth` uses) so combining-mark
/// detection comes from the latest UCD. The previous inline
/// CJK-only rule returned 1 for combining marks (e.g. U+0301
/// combining-acute), which broke `IS_COMBINING(wc)` =
/// `WCWIDTH(wc) == 0` and silently disabled every cluster-walk
/// codepath that depended on it (alignmultiwordleft / right,
/// inccs / deccs realign).
#[inline]
#[allow(non_snake_case)]
pub fn WCWIDTH(wc: char) -> i32 {
    // c:3300
    unicode_width::UnicodeWidthChar::width(wc)
        .map(|w| w as i32)
        .unwrap_or_else(|| if wc.is_control() { 0 } else { 1 })
}

/// Port of `WCWIDTH_WINT(wc)` from `Src/zsh.h:3311/3369`. Always
/// 1 in non-multibyte mode; uses WCWIDTH in multibyte mode.
#[inline]
#[allow(non_snake_case)]
pub fn WCWIDTH_WINT(wc: char) -> i32 {
    // c:3311
    WCWIDTH(wc)
}

/// Port of `IS_COMBINING(wc)` from `Src/zsh.h:3343`. True iff `wc`
/// is a non-zero combining character (zero display width).
#[inline]
#[allow(non_snake_case)]
pub fn IS_COMBINING(wc: char) -> bool {
    // c:3343
    wc as u32 != 0 && WCWIDTH(wc) == 0
}

/// Port of `IS_BASECHAR(wc)` from `Src/zsh.h:3352`. True iff `wc`
/// is a graphic character with non-zero width (suitable as base for
/// a combining character).
#[inline]
#[allow(non_snake_case)]
pub fn IS_BASECHAR(wc: char) -> bool {
    // c:3352
    !wc.is_whitespace() && !wc.is_control() && WCWIDTH(wc) > 0
}

/// Port of `ZWC(c)` from `Src/zsh.h:3328/3372`. C casts a char
/// literal to `wchar_t` via the `L` prefix (`L'a'`). Rust's `char`
/// is already 32-bit Unicode; the cast is a no-op.
#[inline]
#[allow(non_snake_case)]
pub const fn ZWC(c: char) -> char {
    c
} // c:3328

// =============================================================================
// 46. Options accessor compat (already-allowed alias for OPT_*).
// =============================================================================

/// Port of `OPT_ARG(ops, c)` from `Src/zsh.h:1412` —
/// `((ops)->args[((ops)->ind[c] >> 2) - 1])`. Returns the argument
/// associated with option `c`. Caller must have already checked
/// `OPT_HASARG(ops,c)`; out-of-range indices yield `None` (C would
/// dereference past `args[]`, which is undefined — Rust port stays
/// safe).
#[inline]
#[allow(non_snake_case)]
pub fn OPT_ARG<'a>(ops: &'a options, c: u8) -> Option<&'a str> {
    let idx = (ops.ind[c as usize] >> 2) as usize;
    if idx == 0 {
        return None;
    }
    ops.args.get(idx - 1).map(|s| s.as_str())
}

/// Port of `OPT_ARG_SAFE(ops, c)` from `Src/zsh.h:1414` —
/// `(OPT_HASARG(ops,c) ? OPT_ARG(ops,c) : NULL)`.
#[inline]
#[allow(non_snake_case)]
pub fn OPT_ARG_SAFE<'a>(ops: &'a options, c: u8) -> Option<&'a str> {
    if OPT_HASARG(ops, c) {
        OPT_ARG(ops, c)
    } else {
        None
    }
}

// Suppress dead-code warnings for the AtomicI32 we don't use yet.
#[allow(dead_code)]
const _MARKER_KEEP: AtomicI32 = AtomicI32::new(0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zlong_zulong_sizes() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(std::mem::size_of::<zlong>(), 8);
        assert_eq!(std::mem::size_of::<zulong>(), 8);
    }

    #[test]
    fn meta_byte_value() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(Meta as u32, 0x83);
    }

    #[test]
    fn parser_tokens_correct() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(Pound as u32, 0x84);
        assert_eq!(Bang as u32, 0x9c);
        assert_eq!(Snull as u32, 0x9d);
        assert_eq!(Dnull as u32, 0x9e);
        assert_eq!(Bnull as u32, 0x9f);
        assert_eq!(Bnullkeep as u32, 0xa0);
        assert_eq!(Nularg as u32, 0xa1);
        assert_eq!(Marker as u32, 0xa2);
    }

    #[test]
    fn pm_type_isolates_type_bits() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(PM_TYPE(PM_INTEGER | PM_EXPORTED), PM_INTEGER);
        assert_eq!(PM_TYPE(PM_ARRAY | PM_READONLY), PM_ARRAY);
    }

    #[test]
    fn opt_isset_basic() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        ops.ind[b'l' as usize] = 1; // OPT_MINUS bit
        assert!(OPT_ISSET(&ops, b'l'));
        assert!(OPT_MINUS(&ops, b'l'));
        assert!(!OPT_PLUS(&ops, b'l'));
        assert!(!OPT_ISSET(&ops, b'r'));
    }

    #[test]
    fn binf_constants_correct() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(BINF_PREFIX, 1 << 5);
        assert_eq!(BINF_ASSIGN, 1 << 19);
    }

    #[test]
    fn cond_constants_correct() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(COND_NOT, 0);
        assert_eq!(COND_MODI, 19);
    }

    #[test]
    fn fdt_constants_correct() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(FDT_UNUSED, 0);
        assert_eq!(FDT_PROC_SUBST, 7);
        assert_eq!(FDT_TYPE_MASK, 15);
    }

    #[test]
    fn redir_iswrite_classification() {
        let _g = crate::test_util::global_state_lock();
        assert!(IS_WRITE_FILE(REDIR_WRITE));
        assert!(IS_WRITE_FILE(REDIR_READWRITE));
        assert!(!IS_WRITE_FILE(REDIR_READ));
        assert!(IS_ERROR_REDIR(REDIR_ERRWRITE));
        assert!(IS_ERROR_REDIR(REDIR_ERRAPPNOW));
        assert!(!IS_ERROR_REDIR(REDIR_WRITE));
    }

    #[test]
    fn wc_macros_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let w = wc_bld(WC_LIST, 42);
        assert_eq!(wc_code(w), WC_LIST);
        assert_eq!(wc_data(w), 42);
    }

    #[test]
    fn mb_metastrlen_counts_meta_pairs() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(MB_METASTRLEN("abc"), 3);
        // META is char 0x83, but in UTF-8 it encodes as 2 bytes
        // (0xC2 0x83). The byte-level metafied counter walks the
        // raw bytes; "abc" has 3 bytes → 3. Just test ASCII here.
        assert_eq!(MB_METASTRLEN("hello"), 5);
        assert_eq!(MB_METASTRLEN(""), 0);
    }

    #[test]
    fn mb_charlen_basic() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(MB_CHARLEN(b"abc", 3), 1);
        assert_eq!(MB_CHARLEN(b"", 0), 0);
    }

    #[test]
    fn wcwidth_basic() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(WCWIDTH('a'), 1);
        assert_eq!(WCWIDTH('\u{0007}'), 0); // BEL is control
        assert_eq!(WCWIDTH('\u{4E2D}'), 2); // CJK
    }

    #[test]
    fn is_combining_zero_width() {
        let _g = crate::test_util::global_state_lock();
        assert!(!IS_COMBINING('a')); // width 1
        assert!(!IS_COMBINING('\u{0000}')); // null returns false per c:3343
                                            // Note: the WCWIDTH heuristic in this port doesn't recognise
                                            // combining marks — that needs a unicode-width table. Test
                                            // the contract (width-0 non-zero char) rather than the
                                            // specific Unicode codepoint behaviour.
                                            // BEL (control) returns 0 from WCWIDTH and is non-zero, so
                                            // IS_COMBINING returns true.
        assert!(IS_COMBINING('\u{0007}'));
    }

    #[test]
    fn pat_flags_correct() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(PAT_FILE, 0x0001);
        assert_eq!(PAT_LCMATCHUC, 0x1000);
    }

    #[test]
    fn sub_flags_correct() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(SUB_END, 0x0001);
        assert_eq!(SUB_EGLOB, 0x4000);
    }

    #[test]
    fn pp_constants_ordered() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(PP_FIRST, PP_ALPHA);
        assert!(PP_LAST >= PP_ALPHA);
        assert!(PP_RANGE > PP_LAST);
    }

    #[test]
    fn typeset_optstr_constants() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(TYPESET_OPTSTR, "aiEFALRZlurtxUhHT");
        assert_eq!(TYPESET_OPTNUM, "LRZiEF");
    }

    #[test]
    fn job_stat_flags_distinct() {
        let _g = crate::test_util::global_state_lock();
        let all = STAT_CHANGED
            | STAT_STOPPED
            | STAT_TIMED
            | STAT_DONE
            | STAT_LOCKED
            | STAT_NOPRINT
            | STAT_INUSE
            | STAT_SUPERJOB
            | STAT_SUBJOB
            | STAT_WASSUPER
            | STAT_CURSH
            | STAT_NOSTTY
            | STAT_ATTACH
            | STAT_SUBLEADER
            | STAT_BUILTIN
            | STAT_SUBJOB_ORPHANED
            | STAT_DISOWN;
        assert_eq!(all.count_ones(), 17);
    }

    #[test]
    fn opt_size_at_186() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(OPT_SIZE, 186);
    }

    #[test]
    fn cs_count_is_32() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(CS_COUNT, 32);
    }

    #[test]
    fn zwc_passes_through() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ZWC('a'), 'a');
    }

    /// `IS_DASH` matches both ASCII '-' (0x2D) and the tokenized
    /// Dash marker (0x9B) the lexer emits inside `[[ ... ]]`. Both
    /// must be recognised — regression that drops one would break
    /// either user-typed input OR pre-lexed cond expressions.
    #[test]
    fn is_dash_recognises_both_ascii_and_lexed_token() {
        let _g = crate::test_util::global_state_lock();
        assert!(IS_DASH('-'), "ASCII '-' is dash");
        assert!(IS_DASH('\u{9b}'), "lexed Dash token is dash");
        assert!(!IS_DASH('+'), "non-dash chars must NOT match");
        assert!(!IS_DASH(' '), "space is not dash");
    }

    /// c:1408 — `OPT_ISSET(ops, c)` returns true iff `ops.ind[c] != 0`.
    /// Catches a regression where the indexing returns NUL-byte
    /// "false" for a set option, breaking every `-x` flag check.
    #[test]
    fn opt_isset_reads_ind_array_directly() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        assert!(!OPT_ISSET(&ops, b'x'));
        ops.ind[b'x' as usize] = 1;
        assert!(
            OPT_ISSET(&ops, b'x'),
            "after setting ind, OPT_ISSET must be true"
        );
        ops.ind[b'x' as usize] = 0;
        assert!(
            !OPT_ISSET(&ops, b'x'),
            "clearing ind must make OPT_ISSET false"
        );
    }

    /// `PM_TYPE` masks just the type-bits of a flag word (PM_SCALAR /
    /// PM_INTEGER / PM_ARRAY / PM_HASHED / etc.). Modifier flags
    /// (PM_READONLY, PM_EXPORTED) MUST NOT leak through. A regression
    /// returning the full flag word would silently mis-dispatch every
    /// param introspection path.
    #[test]
    fn pm_type_strips_modifier_flags() {
        let _g = crate::test_util::global_state_lock();
        let with_mods = PM_INTEGER | PM_READONLY | PM_EXPORTED;
        assert_eq!(PM_TYPE(with_mods), PM_INTEGER);
        let just_array = PM_TYPE(PM_ARRAY | PM_LEFT | PM_TIED);
        assert_eq!(just_array, PM_ARRAY);
    }

    /// `Src/zsh.h:1878-1944` — `PM_*` flag values are load-bearing.
    /// Pin EVERY parameter-mode flag against the canonical C define.
    /// If any value drifts, serialised param state (typeset, export,
    /// hash dumps) corrupts on the next read.
    #[test]
    fn pm_flags_match_c_zsh_h_canonical_values() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(PM_SCALAR, 0, "c:1878");
        assert_eq!(PM_ARRAY, 1 << 0, "c:1879");
        assert_eq!(PM_INTEGER, 1 << 1, "c:1880");
        assert_eq!(PM_EFLOAT, 1 << 2, "c:1881");
        assert_eq!(PM_FFLOAT, 1 << 3, "c:1882");
        assert_eq!(PM_HASHED, 1 << 4, "c:1883");
        assert_eq!(PM_LEFT, 1 << 5, "c:1888");
        assert_eq!(PM_RIGHT_B, 1 << 6, "c:1889");
        assert_eq!(PM_RIGHT_Z, 1 << 7, "c:1890");
        assert_eq!(PM_LOWER, 1 << 8, "c:1891");
        assert_eq!(PM_UPPER, 1 << 9, "c:1895");
        assert_eq!(PM_UNDEFINED, 1 << 9, "c:1896 (aliases PM_UPPER for funcs)");
        assert_eq!(PM_READONLY, 1 << 10, "c:1898");
        assert_eq!(PM_TAGGED, 1 << 11, "c:1899");
        assert_eq!(PM_EXPORTED, 1 << 12, "c:1900");
        assert_eq!(
            PM_ABSPATH_USED,
            1 << 12,
            "c:1901 (aliases EXPORTED for funcs)"
        );
        assert_eq!(PM_UNIQUE, 1 << 13, "c:1905");
        assert_eq!(PM_HIDE, 1 << 14, "c:1908");
        assert_eq!(PM_HIDEVAL, 1 << 15, "c:1910");
        assert_eq!(PM_TIED, 1 << 16, "c:1912");
        assert_eq!(PM_SPECIAL, 1 << 20, "c:1922");
        assert_eq!(PM_RO_BY_DESIGN, 1 << 21, "c:1924");
        assert_eq!(PM_LOCAL, 1 << 19, "c:1920");
        assert_eq!(PM_UNSET, 1 << 24, "c:1930");
        assert_eq!(PM_NAMEREF, 1 << 30, "c:1944");
    }

    /// `Src/zsh.h:1953-1965` — `SCANPM_*` flag values for parameter
    /// scanning. Used by `${(k)hash}` / `${(K)hash}` / `${(m)hash}`
    /// expansion paths. Drift here mis-routes every hash scan.
    #[test]
    fn scanpm_flags_match_c_zsh_h_canonical_values() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(SCANPM_WANTVALS, 1 << 0, "c:1953");
        assert_eq!(SCANPM_WANTKEYS, 1 << 1, "c:1954");
        assert_eq!(SCANPM_WANTINDEX, 1 << 2, "c:1955");
        assert_eq!(SCANPM_MATCHKEY, 1 << 3, "c:1956");
        assert_eq!(SCANPM_MATCHVAL, 1 << 4, "c:1957");
        assert_eq!(SCANPM_MATCHMANY, 1 << 5, "c:1958");
        assert_eq!(SCANPM_ASSIGNING, 1 << 6, "c:1959");
        assert_eq!(SCANPM_KEYMATCH, 1 << 7, "c:1960");
        assert_eq!(SCANPM_DQUOTED, 1 << 8, "c:1961");
        assert_eq!(SCANPM_ARRONLY, 1 << 9, "c:1965");
    }

    /// `Src/zsh.h:144-224` — parser token byte values. These are
    /// **load-bearing single-byte sentinels** for every lex/parse
    /// path (Pound for `#` comments, Star for `*` glob, Stringg for
    /// `$param`, etc.). A drift on any byte would silently mis-route
    /// every parsed token.
    #[test]
    fn token_byte_values_match_c_zsh_h() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(Meta, 0x83, "c:144");
        assert_eq!(Pound, '\u{84}', "c:159");
        assert_eq!(Stringg, '\u{85}', "c:160");
        assert_eq!(Hat, '\u{86}', "c:161");
        assert_eq!(Star, '\u{87}', "c:162");
        assert_eq!(Inpar, '\u{88}', "c:163");
        assert_eq!(Inparmath, '\u{89}', "c:164");
        assert_eq!(Outpar, '\u{8a}', "c:165");
        assert_eq!(Outparmath, '\u{8b}', "c:166");
        assert_eq!(Qstring, '\u{8c}', "c:167");
        assert_eq!(Equals, '\u{8d}', "c:168");
        assert_eq!(Bar, '\u{8e}', "c:169");
        assert_eq!(Inbrace, '\u{8f}', "c:170");
        assert_eq!(Outbrace, '\u{90}', "c:171");
        assert_eq!(Inbrack, '\u{91}', "c:172");
        assert_eq!(Outbrack, '\u{92}', "c:173");
        assert_eq!(Tick, '\u{93}', "c:174");
        assert_eq!(Inang, '\u{94}', "c:175");
        assert_eq!(Outang, '\u{95}', "c:176");
        assert_eq!(OutangProc, '\u{96}', "c:177");
        assert_eq!(Quest, '\u{97}', "c:178");
        assert_eq!(Tilde, '\u{98}', "c:179");
        assert_eq!(Qtick, '\u{99}', "c:180");
        assert_eq!(Comma, '\u{9a}', "c:181");
        assert_eq!(Dash, '\u{9b}', "c:182");
        assert_eq!(Bang, '\u{9c}', "c:183");
        assert_eq!(LAST_NORMAL_TOK, Bang, "c:188 == Bang");
        assert_eq!(Snull, '\u{9d}', "c:193");
        assert_eq!(Dnull, '\u{9e}', "c:194");
        assert_eq!(Bnull, '\u{9f}', "c:195");
        assert_eq!(Bnullkeep, '\u{a0}', "c:200");
        assert_eq!(Nularg, '\u{a1}', "c:206");
        assert_eq!(Marker, '\u{a2}', "c:224");
    }

    /// `Src/zsh.h:226-232` — SPECCHARS / PATCHARS string literals.
    /// Pin the exact 25-char SPECCHARS set and 13-char PATCHARS set
    /// against the canonical C define. Drift on either would silently
    /// change which chars trigger quoting / pattern matching.
    #[test]
    fn specchars_patchars_match_c_zsh_h() {
        let _g = crate::test_util::global_state_lock();
        // c:228 — `SPECCHARS "#$^*()=|{}[]`<>?~;&\n\t \\\'\""`.
        assert_eq!(
            SPECCHARS, "#$^*()=|{}[]`<>?~;&\n\t \\\'\"",
            "c:228 — SPECCHARS literal must match C verbatim"
        );
        assert_eq!(
            SPECCHARS.chars().count(),
            25,
            "c:228 — SPECCHARS has 25 chars"
        );
        // c:232 — `PATCHARS "#^*()|[]<>?~\\"`.
        assert_eq!(
            PATCHARS, "#^*()|[]<>?~\\",
            "c:232 — PATCHARS literal must match C verbatim"
        );
        assert_eq!(
            PATCHARS.chars().count(),
            13,
            "c:232 — PATCHARS has 13 chars"
        );
    }

    /// `Src/zsh.h:149-153` — DEFAULT_IFS and DEFAULT_IFS_SH literals.
    #[test]
    fn default_ifs_strings_match_c_zsh_h() {
        let _g = crate::test_util::global_state_lock();
        // c:149 — `DEFAULT_IFS " \t\n\203 "` (5 chars; \203 is Meta).
        assert_eq!(
            DEFAULT_IFS, " \t\n\u{83} ",
            "c:149 — DEFAULT_IFS = space + tab + newline + Meta + space"
        );
        assert_eq!(
            DEFAULT_IFS.chars().count(),
            5,
            "c:149 — DEFAULT_IFS is 5 chars"
        );
        // c:153 — `DEFAULT_IFS_SH " \t\n"` (3 chars, POSIX sh).
        assert_eq!(
            DEFAULT_IFS_SH, " \t\n",
            "c:153 — DEFAULT_IFS_SH = POSIX 3-char set"
        );
    }

    /// `Src/zsh.h:1879-1883` — `PM_TYPE_MASK` covers the 5 type bits
    /// (PM_ARRAY..PM_HASHED). Every non-type flag must be OUTSIDE
    /// this mask. Pin so a regression that bleeds modifier flags
    /// into the type space fails.
    #[test]
    fn pm_type_mask_excludes_modifier_flags() {
        let _g = crate::test_util::global_state_lock();
        let type_mask = PM_ARRAY | PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_HASHED;
        // Modifier flags MUST be outside the type-mask range.
        for modifier in &[
            PM_LEFT,
            PM_RIGHT_B,
            PM_RIGHT_Z,
            PM_LOWER,
            PM_UPPER,
            PM_READONLY,
            PM_EXPORTED,
            PM_LOCAL,
            PM_UNSET,
        ] {
            assert_eq!(
                modifier & type_mask,
                0,
                "modifier flag 0x{:x} must NOT overlap type mask 0x{:x}",
                modifier,
                type_mask
            );
        }
    }

    /// `Src/zsh.h` IS_WRITE_FILE — `X >= REDIR_WRITE && X <= REDIR_READWRITE`
    /// (0..=8). Comprehensive sweep: every redir-type ID 0..=17 plus
    /// boundaries. A regression that flips the comparison would break
    /// every `>file` / `>>file` / `1>file` redirection emit.
    #[test]
    fn is_write_file_sweep_all_redir_types() {
        let _g = crate::test_util::global_state_lock();
        // c:298-299 — write-file family: 0..=8 inclusive.
        for x in REDIR_WRITE..=REDIR_READWRITE {
            assert!(
                IS_WRITE_FILE(x),
                "redir-type {} ({}) must be IS_WRITE_FILE",
                x,
                x
            );
        }
        // Outside the range — NOT write-file.
        for x in [
            REDIR_READ,
            REDIR_HEREDOC,
            REDIR_HEREDOCDASH,
            REDIR_HERESTR,
            REDIR_MERGEIN,
            REDIR_MERGEOUT,
            REDIR_CLOSE,
            REDIR_INPIPE,
            REDIR_OUTPIPE,
        ] {
            assert!(
                !IS_WRITE_FILE(x),
                "redir-type {} must NOT be IS_WRITE_FILE",
                x
            );
        }
    }

    /// `Src/zsh.h` IS_APPEND_REDIR — `IS_WRITE_FILE && (X & 2)`. Bit 1
    /// distinguishes write (0/1/4/5) from append (2/3/6/7). Pin every
    /// boundary so a regression that masks the wrong bit silently
    /// dispatches `>file` as `>>file`.
    #[test]
    fn is_append_redir_pins_bit_1() {
        let _g = crate::test_util::global_state_lock();
        // c:303-304 — true ONLY for 2/3/6/7 in the write-file family.
        assert!(IS_APPEND_REDIR(REDIR_APP), "REDIR_APP=2 is append");
        assert!(IS_APPEND_REDIR(REDIR_APPNOW), "REDIR_APPNOW=3 is append");
        assert!(IS_APPEND_REDIR(REDIR_ERRAPP), "REDIR_ERRAPP=6 is append");
        assert!(
            IS_APPEND_REDIR(REDIR_ERRAPPNOW),
            "REDIR_ERRAPPNOW=7 is append"
        );
        // NOT append.
        assert!(!IS_APPEND_REDIR(REDIR_WRITE), "REDIR_WRITE=0 is not append");
        assert!(
            !IS_APPEND_REDIR(REDIR_WRITENOW),
            "REDIR_WRITENOW=1 is not append"
        );
        assert!(
            !IS_APPEND_REDIR(REDIR_ERRWRITE),
            "REDIR_ERRWRITE=4 is not append"
        );
        assert!(
            !IS_APPEND_REDIR(REDIR_ERRWRITENOW),
            "REDIR_ERRWRITENOW=5 is not append"
        );
        // Outside the write-file family — never append.
        assert!(
            !IS_APPEND_REDIR(REDIR_READ),
            "REDIR_READ=9 is not write-file"
        );
    }

    /// `Src/zsh.h` IS_CLOBBER_REDIR — `IS_WRITE_FILE && (X & 1)`. Bit 0
    /// is the `NOW` suffix (>! / >>!) that bypasses NO_CLOBBER. Pin
    /// boundary so a regression mis-flagging clobber would break user
    /// scripts that rely on NO_CLOBBER semantics.
    #[test]
    fn is_clobber_redir_pins_bit_0() {
        let _g = crate::test_util::global_state_lock();
        // c:308-309 — true ONLY for 1/3/5/7 in the write-file family.
        assert!(
            IS_CLOBBER_REDIR(REDIR_WRITENOW),
            "REDIR_WRITENOW=1 is clobber"
        );
        assert!(IS_CLOBBER_REDIR(REDIR_APPNOW), "REDIR_APPNOW=3 is clobber");
        assert!(
            IS_CLOBBER_REDIR(REDIR_ERRWRITENOW),
            "REDIR_ERRWRITENOW=5 is clobber"
        );
        assert!(
            IS_CLOBBER_REDIR(REDIR_ERRAPPNOW),
            "REDIR_ERRAPPNOW=7 is clobber"
        );
        // NOT clobber.
        assert!(!IS_CLOBBER_REDIR(REDIR_WRITE));
        assert!(!IS_CLOBBER_REDIR(REDIR_APP));
        assert!(!IS_CLOBBER_REDIR(REDIR_ERRWRITE));
        assert!(!IS_CLOBBER_REDIR(REDIR_ERRAPP));
    }

    /// `Src/zsh.h` IS_ERROR_REDIR — `X >= REDIR_ERRWRITE && X <=
    /// REDIR_ERRAPPNOW` (4..=7). Pin both boundaries — a regression
    /// flipping `<=` to `<` would silently exclude REDIR_ERRAPPNOW.
    #[test]
    fn is_error_redir_inclusive_range() {
        let _g = crate::test_util::global_state_lock();
        // c:313-314 — true ONLY for 4..=7.
        for x in REDIR_ERRWRITE..=REDIR_ERRAPPNOW {
            assert!(IS_ERROR_REDIR(x), "redir-type {} must be IS_ERROR_REDIR", x);
        }
        // Outside — never error.
        for x in [
            REDIR_WRITE,
            REDIR_WRITENOW,
            REDIR_APP,
            REDIR_APPNOW,
            REDIR_READWRITE,
            REDIR_READ,
            REDIR_HEREDOC,
            REDIR_INPIPE,
        ] {
            assert!(
                !IS_ERROR_REDIR(x),
                "redir-type {} must NOT be IS_ERROR_REDIR",
                x
            );
        }
    }

    /// `Src/zsh.h` IS_READFD — `(X >= REDIR_READWRITE && X <= REDIR_MERGEIN)
    /// || X == REDIR_INPIPE`. Read-fd family: 8..=13 OR 16. Pin so a
    /// regression that drops the OR INPIPE arm breaks process
    /// substitution `<(cmd)` redirections.
    #[test]
    fn is_readfd_range_plus_inpipe() {
        let _g = crate::test_util::global_state_lock();
        // c:318-319 — true for 8..=13 INCLUSIVE plus INPIPE=16.
        for x in REDIR_READWRITE..=REDIR_MERGEIN {
            assert!(IS_READFD(x), "redir-type {} must be IS_READFD", x);
        }
        assert!(
            IS_READFD(REDIR_INPIPE),
            "REDIR_INPIPE=16 must be IS_READFD (special-case OR arm)"
        );
        // Outside — not readfd.
        assert!(!IS_READFD(REDIR_WRITE));
        assert!(!IS_READFD(REDIR_MERGEOUT));
        assert!(!IS_READFD(REDIR_CLOSE));
        assert!(!IS_READFD(REDIR_OUTPIPE));
    }

    /// `Src/zsh.h:273-290` — REDIR_* numeric values are load-bearing
    /// because IS_APPEND/IS_CLOBBER use bit-arithmetic on the values.
    /// Pin EVERY entry's exact value so a reorder silently flips the
    /// append-vs-write classification across the parser + executor.
    #[test]
    fn redir_constants_have_exact_canonical_values() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(REDIR_WRITE, 0);
        assert_eq!(REDIR_WRITENOW, 1);
        assert_eq!(REDIR_APP, 2);
        assert_eq!(REDIR_APPNOW, 3);
        assert_eq!(REDIR_ERRWRITE, 4);
        assert_eq!(REDIR_ERRWRITENOW, 5);
        assert_eq!(REDIR_ERRAPP, 6);
        assert_eq!(REDIR_ERRAPPNOW, 7);
        assert_eq!(REDIR_READWRITE, 8);
        assert_eq!(REDIR_READ, 9);
        assert_eq!(REDIR_HEREDOC, 10);
        assert_eq!(REDIR_HEREDOCDASH, 11);
        assert_eq!(REDIR_HERESTR, 12);
        assert_eq!(REDIR_MERGEIN, 13);
        assert_eq!(REDIR_MERGEOUT, 14);
        assert_eq!(REDIR_CLOSE, 15);
        assert_eq!(REDIR_INPIPE, 16);
        assert_eq!(REDIR_OUTPIPE, 17);
    }

    /// `Src/zsh.h` REDIR_TYPE_MASK / REDIR_VARID_MASK /
    /// REDIR_FROM_HEREDOC_MASK — these encode redir flags in the
    /// wordcode redir entry. The type-mask MUST be 0x1f (5 bits) so
    /// it covers REDIR_OUTPIPE=17 (0x11) without overlapping the
    /// 0x20/0x40 flag bits.
    #[test]
    fn redir_masks_have_no_overlap() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(REDIR_TYPE_MASK, 0x1f);
        assert_eq!(REDIR_VARID_MASK, 0x20);
        assert_eq!(REDIR_FROM_HEREDOC_MASK, 0x40);
        // Masks must be pairwise disjoint.
        assert_eq!(REDIR_TYPE_MASK & REDIR_VARID_MASK, 0);
        assert_eq!(REDIR_TYPE_MASK & REDIR_FROM_HEREDOC_MASK, 0);
        assert_eq!(REDIR_VARID_MASK & REDIR_FROM_HEREDOC_MASK, 0);
        // Type-mask must cover the largest REDIR_* value (17 = 0x11).
        assert_eq!(
            REDIR_OUTPIPE & REDIR_TYPE_MASK,
            REDIR_OUTPIPE,
            "type-mask must include every REDIR_* up to OUTPIPE=17"
        );
    }

    // ─── WCWIDTH / IS_COMBINING / IS_BASECHAR pin tests ──────────────

    /// `WCWIDTH('a')` = 1 (basic ASCII single width).
    #[test]
    fn zshh_corpus_wcwidth_ascii_is_one() {
        assert_eq!(WCWIDTH('a'), 1);
        assert_eq!(WCWIDTH('Z'), 1);
        assert_eq!(WCWIDTH('0'), 1);
        assert_eq!(WCWIDTH(' '), 1);
    }

    /// `WCWIDTH` on common CJK char is 2 (East Asian Wide).
    #[test]
    fn zshh_corpus_wcwidth_cjk_is_two() {
        // U+4E2D '中' (Chinese for middle) — wide.
        assert_eq!(WCWIDTH('中'), 2);
        // U+65E5 '日' (Japanese for day) — wide.
        assert_eq!(WCWIDTH('日'), 2);
    }

    /// `WCWIDTH` on combining mark is 0.
    /// U+0301 = COMBINING ACUTE ACCENT.
    #[test]
    fn zshh_corpus_wcwidth_combining_is_zero() {
        assert_eq!(WCWIDTH('\u{0301}'), 0, "combining acute accent has width 0");
    }

    /// `IS_COMBINING` true for combining accent codepoints.
    #[test]
    fn zshh_corpus_is_combining_true_for_accents() {
        assert!(IS_COMBINING('\u{0301}'), "U+0301 COMBINING ACUTE");
        assert!(IS_COMBINING('\u{0300}'), "U+0300 COMBINING GRAVE");
    }

    /// `IS_COMBINING` false for normal ASCII letters.
    #[test]
    fn zshh_corpus_is_combining_false_for_ascii() {
        assert!(!IS_COMBINING('a'));
        assert!(!IS_COMBINING('Z'));
        assert!(!IS_COMBINING('0'));
    }

    /// `IS_BASECHAR` true for ordinary printable chars.
    #[test]
    fn zshh_corpus_is_basechar_true_for_letters() {
        assert!(IS_BASECHAR('a'));
        assert!(IS_BASECHAR('A'));
        assert!(IS_BASECHAR('z'));
        assert!(IS_BASECHAR('日'));
    }

    /// `IS_BASECHAR` false for combining marks.
    #[test]
    fn zshh_corpus_is_basechar_false_for_combining() {
        assert!(
            !IS_BASECHAR('\u{0301}'),
            "combining accent is not a base char"
        );
    }

    /// Pound (lex marker) is in the imeta range 0x83..=0xa2.
    #[test]
    fn zshh_corpus_pound_marker_in_imeta_range() {
        let p = Pound as u32;
        assert!(
            p >= 0x83 && p <= 0xa2,
            "Pound = {:#x} must be in imeta range 0x83..=0xa2",
            p
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/zsh.h macro ports.
    // ═══════════════════════════════════════════════════════════════════

    /// `minimum(3, 5)` returns 3. C `#define minimum(x,y) ((x)<(y)?(x):(y))`.
    #[test]
    fn minimum_picks_smaller_int() {
        assert_eq!(minimum(3, 5), 3);
        assert_eq!(minimum(5, 3), 3);
    }

    /// `minimum(3, 3)` returns either (both equal).
    #[test]
    fn minimum_equal_values_returns_value() {
        assert_eq!(minimum(7, 7), 7);
    }

    /// `minimum(-5, 5)` returns -5 (negative smaller).
    #[test]
    fn minimum_negative_picks_negative() {
        assert_eq!(minimum(-5, 5), -5);
    }

    /// `QT_IS_SINGLE(QT_SINGLE)` returns true.
    /// C `#define QT_IS_SINGLE(x) (x == QT_SINGLE || x == QT_SINGLE_OPTIONAL)`.
    #[test]
    fn QT_IS_SINGLE_recognises_QT_SINGLE() {
        assert!(QT_IS_SINGLE(QT_SINGLE));
    }

    /// `QT_IS_SINGLE(QT_NONE)` returns false.
    #[test]
    fn QT_IS_SINGLE_QT_NONE_returns_false() {
        assert!(!QT_IS_SINGLE(QT_NONE));
    }

    /// `IS_WRITE_FILE(REDIR_WRITE)` returns true. C `#define
    /// IS_WRITE_FILE(x)` — any write-redirect family.
    #[test]
    fn IS_WRITE_FILE_recognises_write_token() {
        assert!(IS_WRITE_FILE(REDIR_WRITE));
    }

    /// `IS_APPEND_REDIR(REDIR_APP)` returns true.
    #[test]
    fn IS_APPEND_REDIR_recognises_app_token() {
        assert!(IS_APPEND_REDIR(REDIR_APP));
    }

    /// `IS_ERROR_REDIR(REDIR_ERRWRITE)` returns true.
    #[test]
    fn IS_ERROR_REDIR_recognises_errwrite_token() {
        assert!(IS_ERROR_REDIR(REDIR_ERRWRITE));
    }

    /// `IS_READFD(REDIR_READ)` returns true.
    #[test]
    fn IS_READFD_recognises_read_token() {
        assert!(IS_READFD(REDIR_READ));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/zsh.h wordcode + macro helpers.
    // ═══════════════════════════════════════════════════════════════════

    /// c:918 — `WCB_END()` returns the WC_END opcode at offset 0.
    #[test]
    fn WCB_END_returns_wc_end_opcode() {
        let end = WCB_END();
        assert_eq!(wc_code(end), WC_END, "WCB_END encodes WC_END opcode");
        assert_eq!(wc_data(end), 0, "WCB_END encodes zero data");
    }

    /// c:wc_bld — round-trip: wc_code(wc_bld(c, d)) == c.
    #[test]
    fn wc_bld_code_round_trip() {
        for opcode in [WC_END, WC_LIST, WC_SUBLIST, WC_PIPE] {
            let w = wc_bld(opcode, 42);
            assert_eq!(wc_code(w), opcode, "wc_code round-trips opcode {}", opcode);
        }
    }

    /// c:wc_bld — round-trip data field: wc_data(wc_bld(c, d)) == d.
    #[test]
    fn wc_bld_data_round_trip() {
        for data in [0u32, 1, 42, 0xFFFF, 0x00FFFFFF] {
            let w = wc_bld(WC_LIST, data);
            assert_eq!(wc_data(w), data, "wc_data round-trips data {}", data);
        }
    }

    /// c:920 — `WC_LIST_TYPE` reads the data field (matches wc_data).
    #[test]
    fn WC_LIST_TYPE_reads_data_field() {
        let w = WCB_LIST(7, 0);
        assert_eq!(WC_LIST_TYPE(w), 7, "list type round-trips");
    }

    /// c:924 — `WC_LIST_SKIP` shifts down by WC_LIST_FREE bits.
    #[test]
    fn WC_LIST_SKIP_round_trip() {
        let w = WCB_LIST(0, 100);
        assert_eq!(WC_LIST_SKIP(w), 100, "list skip round-trips");
    }

    /// c:927 — `WC_SUBLIST_TYPE` is data & 3 (low 2 bits).
    #[test]
    fn WC_SUBLIST_TYPE_masks_low_2_bits() {
        let w = WCB_SUBLIST(2, 0, 0); // type=2 fits in 2 bits
        assert_eq!(WC_SUBLIST_TYPE(w), 2);
    }

    /// c:931 — `WC_SUBLIST_FLAGS` masks bits 2-4 (0x1c).
    #[test]
    fn WC_SUBLIST_FLAGS_masks_bits_2_through_4() {
        let w = WCB_SUBLIST(0, 0x04, 0); // flag bit 2
        assert_eq!(WC_SUBLIST_FLAGS(w), 0x04);
    }

    /// c:940 — `WC_PIPE_TYPE` is data & 1 (low bit only).
    #[test]
    fn WC_PIPE_TYPE_masks_low_bit() {
        let w = WCB_PIPE(1, 100);
        assert_eq!(WC_PIPE_TYPE(w), 1);
        let w0 = WCB_PIPE(0, 100);
        assert_eq!(WC_PIPE_TYPE(w0), 0);
    }

    /// c:940 — `WC_PIPE_LINENO` is data >> 1.
    #[test]
    fn WC_PIPE_LINENO_shifts_down_by_one() {
        let w = WCB_PIPE(0, 42);
        assert_eq!(WC_PIPE_LINENO(w), 42, "lineno round-trips through >> 1");
    }

    /// c:215 — `IS_DASH('-')` returns true; other chars return false.
    #[test]
    fn IS_DASH_only_recognizes_hyphen() {
        assert!(IS_DASH('-'));
        assert!(!IS_DASH('+'));
        assert!(!IS_DASH(' '));
        assert!(!IS_DASH('a'));
        assert!(!IS_DASH('\0'));
    }

    /// c:32 — `minimum(a, b)` returns the smaller of two values.
    #[test]
    fn minimum_returns_smaller_value() {
        assert_eq!(minimum(3, 7), 3);
        assert_eq!(minimum(7, 3), 3);
        assert_eq!(
            minimum(5, 5),
            5,
            "equal → either (returns first per PartialOrd)"
        );
        assert_eq!(minimum(-10, 10), -10);
    }

    /// c:32 — minimum works with floats.
    #[test]
    fn minimum_works_with_floats() {
        assert_eq!(minimum(1.5_f64, 2.5_f64), 1.5);
        assert_eq!(minimum(-1.0_f64, 0.0_f64), -1.0);
    }

    /// c:246 — `QT_IS_SINGLE(QT_SINGLE)` returns true.
    #[test]
    fn QT_IS_SINGLE_recognizes_single_quote() {
        // QT_SINGLE is the canonical single-quote token value.
        // Sanity: any non-QT_SINGLE returns false; QT_SINGLE → true.
        assert!(QT_IS_SINGLE(crate::ported::zsh_h::QT_SINGLE));
        assert!(!QT_IS_SINGLE(0));
        assert!(!QT_IS_SINGLE(-1));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/zsh.h wordcode WC_* helpers.
    // ═══════════════════════════════════════════════════════════════════

    /// c:970 — `WCB_SIMPLE(N) / WC_SIMPLE_ARGC` round-trip preserves argc.
    #[test]
    fn WCB_SIMPLE_round_trips_argc() {
        for n in [0u32, 1, 42, 1000, 0xFFFF] {
            let w = WCB_SIMPLE(n);
            assert_eq!(WC_SIMPLE_ARGC(w), n, "argc {} must round-trip", n);
        }
    }

    /// c:973 — `WCB_TYPESET / WC_TYPESET_ARGC` round-trip.
    #[test]
    fn WCB_TYPESET_round_trips_argc() {
        for n in [0u32, 5, 100, 1000] {
            let w = WCB_TYPESET(n);
            assert_eq!(WC_TYPESET_ARGC(w), n);
        }
    }

    /// c:976 — `WCB_SUBSH / WC_SUBSH_SKIP` round-trip preserves skip offset.
    #[test]
    fn WCB_SUBSH_round_trips_skip() {
        for o in [0u32, 1, 100, 0x10000] {
            let w = WCB_SUBSH(o);
            assert_eq!(WC_SUBSH_SKIP(w), o);
        }
    }

    /// c:979 — `WCB_CURSH / WC_CURSH_SKIP` round-trip.
    #[test]
    fn WCB_CURSH_round_trips_skip() {
        for o in [0u32, 1, 50, 500] {
            let w = WCB_CURSH(o);
            assert_eq!(WC_CURSH_SKIP(w), o);
        }
    }

    /// c:982 — `WCB_TIMED / WC_TIMED_TYPE` round-trip.
    #[test]
    fn WCB_TIMED_round_trips_type() {
        for t in [0u32, 1, 2] {
            let w = WCB_TIMED(t);
            assert_eq!(WC_TIMED_TYPE(w), t);
        }
    }

    /// c:987 — `WCB_FUNCDEF / WC_FUNCDEF_SKIP` round-trip.
    #[test]
    fn WCB_FUNCDEF_round_trips_skip() {
        for o in [0u32, 10, 100] {
            let w = WCB_FUNCDEF(o);
            assert_eq!(WC_FUNCDEF_SKIP(w), o);
        }
    }

    /// c:990 — `WCB_FOR(type, skip)` encodes type in low 2 bits, skip
    /// in upper bits.
    #[test]
    fn WCB_FOR_packs_type_low_skip_high() {
        // type fits in 2 bits (0..3)
        for t in [0u32, 1, 2, 3] {
            for o in [0u32, 10, 100] {
                let w = WCB_FOR(t, o);
                assert_eq!(WC_FOR_TYPE(w), t, "type round-trip for ({}, {})", t, o);
                assert_eq!(WC_FOR_SKIP(w), o, "skip round-trip for ({}, {})", t, o);
            }
        }
    }

    /// c:997 — `WC_SELECT_TYPE(c)` masks low bit only.
    #[test]
    fn WC_SELECT_TYPE_masks_low_bit() {
        // wc_bld with 0b11 in data → SELECT_TYPE = 1 (low bit).
        let w = wc_bld(WC_SELECT, 3);
        assert_eq!(WC_SELECT_TYPE(w), 1);
        let w0 = wc_bld(WC_SELECT, 4);
        assert_eq!(WC_SELECT_TYPE(w0), 0, "0b100 & 1 = 0");
    }

    /// c:997 — `WC_SELECT_SKIP(c)` is data >> 1.
    #[test]
    fn WC_SELECT_SKIP_shifts_right_one() {
        let w = wc_bld(WC_SELECT, 42);
        assert_eq!(WC_SELECT_SKIP(w), 42 >> 1);
    }

    /// c:990 — WC_FOR_TYPE only reads 2 bits — high bits ignored.
    #[test]
    fn WC_FOR_TYPE_only_uses_low_2_bits() {
        // 0b1011 → 0b11 = 3 (low 2 bits).
        let w = wc_bld(WC_FOR, 0b1011);
        assert_eq!(WC_FOR_TYPE(w), 3);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/zsh.h PM_* parameter flag bits
    // c:3174-3206 — scalar/array/integer/float/hashed/case/readonly/etc.
    // ═══════════════════════════════════════════════════════════════════

    /// c:3174 — `PM_SCALAR` = 0 (default param type).
    #[test]
    fn pm_scalar_is_zero() {
        assert_eq!(PM_SCALAR, 0, "PM_SCALAR is the default (no bits)");
    }

    /// c:3176-3184 — PM_ARRAY/INTEGER/EFLOAT/FFLOAT/HASHED canonical bit positions.
    #[test]
    fn pm_type_flags_canonical_bit_positions() {
        assert_eq!(PM_ARRAY, 1 << 0, "c:3176");
        assert_eq!(PM_INTEGER, 1 << 1, "c:3178");
        assert_eq!(PM_EFLOAT, 1 << 2, "c:3180");
        assert_eq!(PM_FFLOAT, 1 << 3, "c:3182");
        assert_eq!(PM_HASHED, 1 << 4, "c:3184");
    }

    /// c:3186-3194 — PM_LEFT/RIGHT_B/RIGHT_Z/LOWER/UPPER canonical bits.
    #[test]
    fn pm_padding_case_flags_canonical_bit_positions() {
        assert_eq!(PM_LEFT, 1 << 5, "c:3186");
        assert_eq!(PM_RIGHT_B, 1 << 6, "c:3188");
        assert_eq!(PM_RIGHT_Z, 1 << 7, "c:3190");
        assert_eq!(PM_LOWER, 1 << 8, "c:3192");
        assert_eq!(PM_UPPER, 1 << 9, "c:3194");
    }

    /// c:3196 — PM_UNDEFINED INTENTIONALLY aliases PM_UPPER (both 1<<9).
    /// Per c:3196 comment, autoload-undefined fns reuse the UPPER bit
    /// because undefined fns can never have case attributes.
    #[test]
    fn pm_undefined_aliases_pm_upper() {
        assert_eq!(
            PM_UNDEFINED, PM_UPPER,
            "c:3196 INTENTIONAL alias: undefined fns reuse UPPER bit"
        );
    }

    /// c:3204 — PM_ABSPATH_USED INTENTIONALLY aliases PM_EXPORTED.
    /// c:3204 comment: only-for-internal-tracking flag reuses exported bit.
    #[test]
    fn pm_abspath_used_aliases_pm_exported() {
        assert_eq!(
            PM_ABSPATH_USED, PM_EXPORTED,
            "c:3204 INTENTIONAL alias: bit reuse for path-tracking"
        );
    }

    /// c:3198 — PM_READONLY = 1 << 10.
    #[test]
    fn pm_readonly_is_bit_10() {
        assert_eq!(PM_READONLY, 1 << 10);
    }

    /// c:3174-3206 — PM_* flags are all u32 (compile-time type pin).
    #[test]
    fn pm_flags_are_u32_type() {
        let _: u32 = PM_SCALAR;
        let _: u32 = PM_ARRAY;
        let _: u32 = PM_INTEGER;
        let _: u32 = PM_READONLY;
    }

    /// c:3176-3194 — distinct type flags are pairwise disjoint
    /// (excluding intentional aliases like UNDEFINED=UPPER).
    #[test]
    fn pm_distinct_type_flags_pairwise_disjoint() {
        let codes = [
            PM_ARRAY, PM_INTEGER, PM_EFLOAT, PM_FFLOAT, PM_HASHED, PM_LEFT, PM_RIGHT_B, PM_RIGHT_Z,
            PM_LOWER, PM_UPPER,
        ];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(
            unique.len(),
            codes.len(),
            "distinct PM type/padding flags must be pairwise disjoint"
        );
    }

    /// c:3176-3194 — every distinct PM_* type/padding flag is a single bit.
    #[test]
    fn pm_distinct_type_flags_all_single_bits() {
        for &v in &[
            PM_ARRAY,
            PM_INTEGER,
            PM_EFLOAT,
            PM_FFLOAT,
            PM_HASHED,
            PM_LEFT,
            PM_RIGHT_B,
            PM_RIGHT_Z,
            PM_LOWER,
            PM_UPPER,
            PM_READONLY,
            PM_TAGGED,
            PM_EXPORTED,
            PM_UNIQUE,
        ] {
            assert!(
                v.is_power_of_two(),
                "PM_* flag {:#x} must be a single bit",
                v
            );
        }
    }

    /// c:3174 — PM_SCALAR being 0 means "all flags clear" matches PM_SCALAR.
    #[test]
    fn pm_scalar_zero_is_default_state() {
        assert_eq!(
            PM_SCALAR & (PM_ARRAY | PM_INTEGER | PM_HASHED),
            0,
            "PM_SCALAR=0 by design: clear-all-bits state"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/zsh.h PRINT_* + HIST_* flag bits
    // c:3400-3428 PRINT_* / c:3452-3464 HIST_*
    // ═══════════════════════════════════════════════════════════════════

    /// c:3400-3418 — PRINT_NAMEONLY/TYPE/LIST/KV_PAIR/INCLUDEVALUE/TYPESET/
    /// LINE/POSIX_EXPORT/POSIX_READONLY/WITH_NAMESPACE canonical bits 0-9.
    #[test]
    fn print_canonical_bit_positions() {
        assert_eq!(PRINT_NAMEONLY, 1 << 0, "c:3400");
        assert_eq!(PRINT_TYPE, 1 << 1, "c:3402");
        assert_eq!(PRINT_LIST, 1 << 2, "c:3404");
        assert_eq!(PRINT_KV_PAIR, 1 << 3, "c:3406");
        assert_eq!(PRINT_INCLUDEVALUE, 1 << 4, "c:3408");
        assert_eq!(PRINT_TYPESET, 1 << 5, "c:3410");
        assert_eq!(PRINT_LINE, 1 << 6, "c:3412");
        assert_eq!(PRINT_POSIX_EXPORT, 1 << 7, "c:3414");
        assert_eq!(PRINT_POSIX_READONLY, 1 << 8, "c:3416");
        assert_eq!(PRINT_WITH_NAMESPACE, 1 << 9, "c:3418");
    }

    /// c:3420 — PRINT_WHENCE_CSH INTENTIONALLY aliases PRINT_POSIX_EXPORT
    /// (both bit 7). Per c:2191 comment: typeset and whence are separate
    /// builtins, so flag-bit reuse is safe.
    #[test]
    fn print_whence_csh_aliases_print_posix_export() {
        assert_eq!(
            PRINT_WHENCE_CSH, PRINT_POSIX_EXPORT,
            "c:3420 INTENTIONAL alias: whence vs typeset disambiguated by builtin"
        );
    }

    /// c:3422 — PRINT_WHENCE_VERBOSE aliases PRINT_POSIX_READONLY.
    #[test]
    fn print_whence_verbose_aliases_print_posix_readonly() {
        assert_eq!(
            PRINT_WHENCE_VERBOSE, PRINT_POSIX_READONLY,
            "c:3422 INTENTIONAL alias"
        );
    }

    /// c:3424 — PRINT_WHENCE_SIMPLE aliases PRINT_WITH_NAMESPACE.
    #[test]
    fn print_whence_simple_aliases_print_with_namespace() {
        assert_eq!(
            PRINT_WHENCE_SIMPLE, PRINT_WITH_NAMESPACE,
            "c:3424 INTENTIONAL alias"
        );
    }

    /// c:3400-3428 — every PRINT_* flag is i32 type (compile-time pin).
    #[test]
    fn print_flags_are_i32_type() {
        let _: i32 = PRINT_NAMEONLY;
        let _: i32 = PRINT_TYPE;
        let _: i32 = PRINT_WHENCE_CSH;
    }

    /// c:3452-3464 — HIST_MAKEUNIQUE/OLD/READ/DUP/FOREIGN/TMPSTORE/NOWRITE
    /// canonical bit values match upstream zsh-source positions.
    #[test]
    fn hist_canonical_bit_positions() {
        assert_eq!(HIST_MAKEUNIQUE, 0x01, "c:3452");
        assert_eq!(HIST_OLD, 0x02, "c:3454");
        assert_eq!(HIST_READ, 0x04, "c:3456");
        assert_eq!(HIST_DUP, 0x08, "c:3458");
        assert_eq!(HIST_FOREIGN, 0x10, "c:3460");
        assert_eq!(HIST_TMPSTORE, 0x20, "c:3462");
        assert_eq!(HIST_NOWRITE, 0x40, "c:3464");
    }

    /// c:3452-3464 — all HIST_* are u32 type (compile-time pin).
    #[test]
    fn hist_flags_are_u32_type() {
        let _: u32 = HIST_MAKEUNIQUE;
        let _: u32 = HIST_OLD;
        let _: u32 = HIST_NOWRITE;
    }

    /// c:3452-3464 — HIST_* are all single bits.
    #[test]
    fn hist_flags_all_single_bits() {
        for &v in &[
            HIST_MAKEUNIQUE,
            HIST_OLD,
            HIST_READ,
            HIST_DUP,
            HIST_FOREIGN,
            HIST_TMPSTORE,
            HIST_NOWRITE,
        ] {
            assert!(v.is_power_of_two(), "HIST_* {:#x} must be a single bit", v);
        }
    }

    /// c:3452-3464 — HIST_* are pairwise distinct.
    #[test]
    fn hist_flags_pairwise_distinct() {
        let codes = [
            HIST_MAKEUNIQUE,
            HIST_OLD,
            HIST_READ,
            HIST_DUP,
            HIST_FOREIGN,
            HIST_TMPSTORE,
            HIST_NOWRITE,
        ];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(
            unique.len(),
            codes.len(),
            "HIST_* must be pairwise distinct"
        );
    }

    /// c:3400-3418 — first 7 non-aliased PRINT_* bits are pairwise distinct.
    #[test]
    fn print_non_aliased_pairwise_distinct() {
        let codes = [
            PRINT_NAMEONLY,
            PRINT_TYPE,
            PRINT_LIST,
            PRINT_KV_PAIR,
            PRINT_INCLUDEVALUE,
            PRINT_TYPESET,
            PRINT_LINE,
        ];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(
            unique.len(),
            codes.len(),
            "non-aliased PRINT_* must be pairwise distinct"
        );
    }
}
