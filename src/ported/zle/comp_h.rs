//! `comp.h` port — completion descriptor types + flag constants.
//!
//! Port of `Src/Zle/comp.h`. Canonical home for the new-style
//! completion machinery (compsys / compadd / addmatches / etc.) —
//! distinct from the legacy `compctl.h` machinery (which lives in
//! `compctl_h.rs`).
//!
//! C source: 10 typedefs (`Cmatcher`/`Cmlist`/`Cpattern`/`Menuinfo`/
//! `Cexpl`/`Cmgroup`/`Cmatch`/`Cline`/`Aminfo`/`Cadata`/`Cldata`/
//! `Chdata`), 13 structs (`cexpl`/`cmgroup`/`cmatch`/`cmlist`/
//! `cmatcher`/`cpattern`/`cline`/`aminfo`/`menuinfo`/`ccmakedat`/
//! `chdata`/`cadata`/`cldata`), 1 enum (`cpat`), and ~80 flag
//! constants. 0 functions.
//!
//! All UPPERCASE C constants (CGF_*, CMF_*, CLF_*, CAF_*, FC_*,
//! CP_*, CPN_*) preserved verbatim per the macro casing rule.
//! Struct names match C casing (Cexpl, Cmgroup, Cmatch, etc.) with
//! `#[allow(non_camel_case_types)]` silencing the convention warning.

// ---------------------------------------------------------------------------
// Group-flag constants (c:85-95) — flags on `cmgroup.flags`.
// ---------------------------------------------------------------------------
/// `CGF_NOSORT` constant.
pub const CGF_NOSORT: i32 = 1; // c:85
/// `CGF_LINES` constant.
pub const CGF_LINES: i32 = 2; // c:86
/// `CGF_HASDL` constant.
pub const CGF_HASDL: i32 = 4; // c:87
/// `CGF_UNIQALL` constant.
pub const CGF_UNIQALL: i32 = 8; // c:88
/// `CGF_UNIQCON` constant.
pub const CGF_UNIQCON: i32 = 16; // c:89
/// `CGF_PACKED` constant.
pub const CGF_PACKED: i32 = 32; // c:90
/// `CGF_ROWS` constant.
pub const CGF_ROWS: i32 = 64; // c:91
/// `CGF_FILES` constant.
pub const CGF_FILES: i32 = 128; // c:92
/// `CGF_MATSORT` constant.
pub const CGF_MATSORT: i32 = 256; // c:93
/// `CGF_NUMSORT` constant.
pub const CGF_NUMSORT: i32 = 512; // c:94
/// `CGF_REVSORT` constant.
pub const CGF_REVSORT: i32 = 1024; // c:95

// ---------------------------------------------------------------------------
// Match-flag constants (c:127-143) — flags on `cmatch.flags`.
// ---------------------------------------------------------------------------
/// `CMF_FILE` constant.
pub const CMF_FILE: i32 = 1 << 0; // c:127
/// `CMF_REMOVE` constant.
pub const CMF_REMOVE: i32 = 1 << 1; // c:128
/// `CMF_ISPAR` constant.
pub const CMF_ISPAR: i32 = 1 << 2; // c:129
/// `CMF_PARBR` constant.
pub const CMF_PARBR: i32 = 1 << 3; // c:130
/// `CMF_PARNEST` constant.
pub const CMF_PARNEST: i32 = 1 << 4; // c:131
/// `CMF_NOLIST` constant.
pub const CMF_NOLIST: i32 = 1 << 5; // c:132
/// `CMF_DISPLINE` constant.
pub const CMF_DISPLINE: i32 = 1 << 6; // c:133
/// `CMF_HIDE` constant.
pub const CMF_HIDE: i32 = 1 << 7; // c:134
/// `CMF_NOSPACE` constant.
pub const CMF_NOSPACE: i32 = 1 << 8; // c:135
/// `CMF_PACKED` constant.
pub const CMF_PACKED: i32 = 1 << 9; // c:136
/// `CMF_ROWS` constant.
pub const CMF_ROWS: i32 = 1 << 10; // c:137
/// `CMF_MULT` constant.
pub const CMF_MULT: i32 = 1 << 11; // c:138
/// `CMF_FMULT` constant.
pub const CMF_FMULT: i32 = 1 << 12; // c:139
/// `CMF_ALL` constant.
pub const CMF_ALL: i32 = 1 << 13; // c:140
/// `CMF_DUMMY` constant.
pub const CMF_DUMMY: i32 = 1 << 14; // c:141
/// `CMF_MORDER` constant.
pub const CMF_MORDER: i32 = 1 << 15; // c:142
/// `CMF_DELETE` constant.
pub const CMF_DELETE: i32 = 1 << 16; // c:143

// ---------------------------------------------------------------------------
// Cmatcher flag constants (c:172-178) — flags on `cmatcher.flags`.
//
// NOTE: C uses the same `CMF_*` prefix for both `cmatch.flags` and
// `cmatcher.flags`. The values are different (cmatcher uses
// 1/2/4/8 vs cmatch's bitfields) but the names overlap (CMF_LINE
// here is 1, while there's no CMF_LINE in cmatch's set). Rust
// preserves both verbatim — the per-struct dispatch context tells
// callers which set applies.
// ---------------------------------------------------------------------------
/// `CMF_LINE` constant.
pub const CMF_LINE: i32 = 1; // c:172
/// `CMF_LEFT` constant.
pub const CMF_LEFT: i32 = 2; // c:174
/// `CMF_RIGHT` constant.
pub const CMF_RIGHT: i32 = 4; // c:176
/// `CMF_INTER` constant.
pub const CMF_INTER: i32 = 8; // c:178

// ---------------------------------------------------------------------------
// Cpattern type discriminators (c:184-190).
// ---------------------------------------------------------------------------

/// Port of `enum { CPAT_CCLASS, ... }` from `Src/Zle/comp.h:184-190`.
/// C uses an anonymous int-constant enum — Rust ports as `pub const`s
/// to avoid a Rust-only enum type. The discriminator drives
/// `freecpattern()` and `cpattern.tp` dispatch (see `Cpattern` below).
pub const CPAT_CCLASS: i32 = 0; // c:185
/// `CPAT_NCLASS` constant.
pub const CPAT_NCLASS: i32 = 1; // c:186
/// `CPAT_EQUIV` constant.
pub const CPAT_EQUIV: i32 = 2; // c:187
/// `CPAT_ANY` constant.
pub const CPAT_ANY: i32 = 3; // c:188
/// `CPAT_CHAR` constant.
pub const CPAT_CHAR: i32 = 4; // c:189

// ---------------------------------------------------------------------------
// Cline flag constants (c:259-267) — flags on `cline.flags`.
// ---------------------------------------------------------------------------
/// `CLF_MISS` constant.
pub const CLF_MISS: i32 = 1; // c:259
/// `CLF_DIFF` constant.
pub const CLF_DIFF: i32 = 2; // c:260
/// `CLF_SUF` constant.
pub const CLF_SUF: i32 = 4; // c:261
/// `CLF_MID` constant.
pub const CLF_MID: i32 = 8; // c:262
/// `CLF_NEW` constant.
pub const CLF_NEW: i32 = 16; // c:263
/// `CLF_LINE` constant.
pub const CLF_LINE: i32 = 32; // c:264
/// `CLF_JOIN` constant.
pub const CLF_JOIN: i32 = 64; // c:265
/// `CLF_MATCHED` constant.
pub const CLF_MATCHED: i32 = 128; // c:266
/// `CLF_SKIP` constant.
pub const CLF_SKIP: i32 = 256; // c:267

// ---------------------------------------------------------------------------
// compadd / addmatches() flag constants (c:299-309).
// ---------------------------------------------------------------------------
/// `CAF_QUOTE` constant.
pub const CAF_QUOTE: i32 = 1; // c:299
/// `CAF_NOSORT` constant.
pub const CAF_NOSORT: i32 = 2; // c:300
/// `CAF_MATCH` constant.
pub const CAF_MATCH: i32 = 4; // c:301
/// `CAF_UNIQCON` constant.
pub const CAF_UNIQCON: i32 = 8; // c:302
/// `CAF_UNIQALL` constant.
pub const CAF_UNIQALL: i32 = 16; // c:303
/// `CAF_ARRAYS` constant.
pub const CAF_ARRAYS: i32 = 32; // c:304
/// `CAF_KEYS` constant.
pub const CAF_KEYS: i32 = 64; // c:305
/// `CAF_ALL` constant.
pub const CAF_ALL: i32 = 128; // c:306
/// `CAF_MATSORT` constant.
pub const CAF_MATSORT: i32 = 256; // c:307
/// `CAF_NUMSORT` constant.
pub const CAF_NUMSORT: i32 = 512; // c:308
/// `CAF_REVSORT` constant.
pub const CAF_REVSORT: i32 = 1024; // c:309

// ---------------------------------------------------------------------------
// Fromcomp flags (c:359-360).
// ---------------------------------------------------------------------------
/// `FC_LINE` constant.
pub const FC_LINE: i32 = 1; // c:359
/// `FC_INWORD` constant.
pub const FC_INWORD: i32 = 2; // c:360

// ---------------------------------------------------------------------------
// Special-parameter index constants — `comprpms` / "real params"
// (c:364-386). For each parameter there's a `CPN_*` index and a
// `CP_*` bitmask `(1 << CPN_*)`.
// ---------------------------------------------------------------------------
/// `CPN_WORDS` constant.
pub const CPN_WORDS: i32 = 0; // c:364
/// `CP_WORDS` constant.
pub const CP_WORDS: u32 = 1 << CPN_WORDS; // c:365
/// `CPN_REDIRS` constant.
pub const CPN_REDIRS: i32 = 1; // c:366
/// `CP_REDIRS` constant.
pub const CP_REDIRS: u32 = 1 << CPN_REDIRS; // c:367
/// `CPN_CURRENT` constant.
pub const CPN_CURRENT: i32 = 2; // c:368
/// `CP_CURRENT` constant.
pub const CP_CURRENT: u32 = 1 << CPN_CURRENT; // c:369
/// `CPN_PREFIX` constant.
pub const CPN_PREFIX: i32 = 3; // c:370
/// `CP_PREFIX` constant.
pub const CP_PREFIX: u32 = 1 << CPN_PREFIX; // c:371
/// `CPN_SUFFIX` constant.
pub const CPN_SUFFIX: i32 = 4; // c:372
/// `CP_SUFFIX` constant.
pub const CP_SUFFIX: u32 = 1 << CPN_SUFFIX; // c:373
/// `CPN_IPREFIX` constant.
pub const CPN_IPREFIX: i32 = 5; // c:374
/// `CP_IPREFIX` constant.
pub const CP_IPREFIX: u32 = 1 << CPN_IPREFIX; // c:375
/// `CPN_ISUFFIX` constant.
pub const CPN_ISUFFIX: i32 = 6; // c:376
/// `CP_ISUFFIX` constant.
pub const CP_ISUFFIX: u32 = 1 << CPN_ISUFFIX; // c:377
/// `CPN_QIPREFIX` constant.
pub const CPN_QIPREFIX: i32 = 7; // c:378
/// `CP_QIPREFIX` constant.
pub const CP_QIPREFIX: u32 = 1 << CPN_QIPREFIX; // c:379
/// `CPN_QISUFFIX` constant.
pub const CPN_QISUFFIX: i32 = 8; // c:380
/// `CP_QISUFFIX` constant.
pub const CP_QISUFFIX: u32 = 1 << CPN_QISUFFIX; // c:381
/// `CPN_COMPSTATE` constant.
pub const CPN_COMPSTATE: i32 = 9; // c:382
/// `CP_COMPSTATE` constant.
pub const CP_COMPSTATE: u32 = 1 << CPN_COMPSTATE; // c:383

/// Port of `#define CP_REALPARAMS` from `Src/Zle/comp.h:385`. Total
/// number of "real" comp parameters.
pub const CP_REALPARAMS: i32 = 10; // c:385

/// Port of `#define CP_ALLREALS` from `Src/Zle/comp.h:386`. Mask
/// covering every CP_* "real" param flag (bits 0..9 set).
pub const CP_ALLREALS: u32 = 0x3ff; // c:386

// ---------------------------------------------------------------------------
// Special-parameter index constants — `compkpms` / "key params"
// (c:389-442).
// ---------------------------------------------------------------------------
/// `CPN_NMATCHES` constant.
pub const CPN_NMATCHES: i32 = 0; // c:389
/// `CP_NMATCHES` constant.
pub const CP_NMATCHES: u32 = 1 << CPN_NMATCHES; // c:390
/// `CPN_CONTEXT` constant.
pub const CPN_CONTEXT: i32 = 1; // c:391
/// `CP_CONTEXT` constant.
pub const CP_CONTEXT: u32 = 1 << CPN_CONTEXT; // c:392
/// `CPN_PARAMETER` constant.
pub const CPN_PARAMETER: i32 = 2; // c:393
/// `CP_PARAMETER` constant.
pub const CP_PARAMETER: u32 = 1 << CPN_PARAMETER; // c:394
/// `CPN_REDIRECT` constant.
pub const CPN_REDIRECT: i32 = 3; // c:395
/// `CP_REDIRECT` constant.
pub const CP_REDIRECT: u32 = 1 << CPN_REDIRECT; // c:396
/// `CPN_QUOTE` constant.
pub const CPN_QUOTE: i32 = 4; // c:397
/// `CP_QUOTE` constant.
pub const CP_QUOTE: u32 = 1 << CPN_QUOTE; // c:398
/// `CPN_QUOTING` constant.
pub const CPN_QUOTING: i32 = 5; // c:399
/// `CP_QUOTING` constant.
pub const CP_QUOTING: u32 = 1 << CPN_QUOTING; // c:400
/// `CPN_RESTORE` constant.
pub const CPN_RESTORE: i32 = 6; // c:401
/// `CP_RESTORE` constant.
pub const CP_RESTORE: u32 = 1 << CPN_RESTORE; // c:402
/// `CPN_LIST` constant.
pub const CPN_LIST: i32 = 7; // c:403
/// `CP_LIST` constant.
pub const CP_LIST: u32 = 1 << CPN_LIST; // c:404
/// `CPN_INSERT` constant.
pub const CPN_INSERT: i32 = 8; // c:405
/// `CP_INSERT` constant.
pub const CP_INSERT: u32 = 1 << CPN_INSERT; // c:406
/// `CPN_EXACT` constant.
pub const CPN_EXACT: i32 = 9; // c:407
/// `CP_EXACT` constant.
pub const CP_EXACT: u32 = 1 << CPN_EXACT; // c:408
/// `CPN_EXACTSTR` constant.
pub const CPN_EXACTSTR: i32 = 10; // c:409
/// `CP_EXACTSTR` constant.
pub const CP_EXACTSTR: u32 = 1 << CPN_EXACTSTR; // c:410
/// `CPN_PATMATCH` constant.
pub const CPN_PATMATCH: i32 = 11; // c:411
/// `CP_PATMATCH` constant.
pub const CP_PATMATCH: u32 = 1 << CPN_PATMATCH; // c:412
/// `CPN_PATINSERT` constant.
pub const CPN_PATINSERT: i32 = 12; // c:413
/// `CP_PATINSERT` constant.
pub const CP_PATINSERT: u32 = 1 << CPN_PATINSERT; // c:414
/// `CPN_UNAMBIG` constant.
pub const CPN_UNAMBIG: i32 = 13; // c:415
/// `CP_UNAMBIG` constant.
pub const CP_UNAMBIG: u32 = 1 << CPN_UNAMBIG; // c:416
/// `CPN_UNAMBIGC` constant.
pub const CPN_UNAMBIGC: i32 = 14; // c:417
/// `CP_UNAMBIGC` constant.
pub const CP_UNAMBIGC: u32 = 1 << CPN_UNAMBIGC; // c:418
/// `CPN_UNAMBIGP` constant.
pub const CPN_UNAMBIGP: i32 = 15; // c:419
/// `CP_UNAMBIGP` constant.
pub const CP_UNAMBIGP: u32 = 1 << CPN_UNAMBIGP; // c:420
/// `CPN_INSERTP` constant.
pub const CPN_INSERTP: i32 = 16; // c:421
/// `CP_INSERTP` constant.
pub const CP_INSERTP: u32 = 1 << CPN_INSERTP; // c:422
/// `CPN_LISTMAX` constant.
pub const CPN_LISTMAX: i32 = 17; // c:423
/// `CP_LISTMAX` constant.
pub const CP_LISTMAX: u32 = 1 << CPN_LISTMAX; // c:424
/// `CPN_LASTPROMPT` constant.
pub const CPN_LASTPROMPT: i32 = 18; // c:425
/// `CP_LASTPROMPT` constant.
pub const CP_LASTPROMPT: u32 = 1 << CPN_LASTPROMPT; // c:426
/// `CPN_TOEND` constant.
pub const CPN_TOEND: i32 = 19; // c:427
/// `CP_TOEND` constant.
pub const CP_TOEND: u32 = 1 << CPN_TOEND; // c:428
/// `CPN_OLDLIST` constant.
pub const CPN_OLDLIST: i32 = 20; // c:429
/// `CP_OLDLIST` constant.
pub const CP_OLDLIST: u32 = 1 << CPN_OLDLIST; // c:430
/// `CPN_OLDINS` constant.
pub const CPN_OLDINS: i32 = 21; // c:431
/// `CP_OLDINS` constant.
pub const CP_OLDINS: u32 = 1 << CPN_OLDINS; // c:432
/// `CPN_VARED` constant.
pub const CPN_VARED: i32 = 22; // c:433
/// `CP_VARED` constant.
pub const CP_VARED: u32 = 1 << CPN_VARED; // c:434
/// `CPN_LISTLINES` constant.
pub const CPN_LISTLINES: i32 = 23; // c:435
/// `CP_LISTLINES` constant.
pub const CP_LISTLINES: u32 = 1 << CPN_LISTLINES; // c:436
/// `CPN_QUOTES` constant.
pub const CPN_QUOTES: i32 = 24; // c:437
/// `CP_QUOTES` constant.
pub const CP_QUOTES: u32 = 1 << CPN_QUOTES; // c:438
/// `CPN_IGNORED` constant.
pub const CPN_IGNORED: i32 = 25; // c:439
/// `CP_IGNORED` constant.
pub const CP_IGNORED: u32 = 1 << CPN_IGNORED; // c:440

/// Port of `#define CP_KEYPARAMS` from `Src/Zle/comp.h:442`. Total
/// number of "key" comp parameters.
pub const CP_KEYPARAMS: i32 = 26; // c:442

/// Port of `#define CP_ALLKEYS` from `Src/Zle/comp.h:443`. Mask
/// covering every CP_* "key" param flag (bits 0..25 set).
pub const CP_ALLKEYS: u32 = 0x3ffffff; // c:443

// ---------------------------------------------------------------------------
// Hook indexes (c:447-451).
// ---------------------------------------------------------------------------
/// `INSERTMATCHHOOK_OFFSET` constant.
pub const INSERTMATCHHOOK_OFFSET: usize = 0; // c:447
/// `MENUSTARTHOOK_OFFSET` constant.
pub const MENUSTARTHOOK_OFFSET: usize = 1; // c:448
/// `COMPCTLMAKEHOOK_OFFSET` constant.
pub const COMPCTLMAKEHOOK_OFFSET: usize = 2; // c:449
/// `COMPCTLCLEANUPHOOK_OFFSET` constant.
pub const COMPCTLCLEANUPHOOK_OFFSET: usize = 3; // c:450
/// `COMPLISTMATCHESHOOK_OFFSET` constant.
pub const COMPLISTMATCHESHOOK_OFFSET: usize = 4; // c:451

// ---------------------------------------------------------------------------
// Misc constants.
// ---------------------------------------------------------------------------

/// Port of `#define CM_SPACE` from `Src/Zle/comp.h:474`. Number of
/// columns to leave empty between rows of matches.
pub const CM_SPACE: i32 = 2; // c:474

// ---------------------------------------------------------------------------
// Typedef structs (c:30-470). C uses linked lists threaded through
// `next`/`prev` pointers; Rust ports as `Option<Box<T>>` for the
// owning side.
// ---------------------------------------------------------------------------

/// Port of `struct cexpl` from `Src/Zle/comp.h:40-45`. Explanation
/// string entry attached to a match group.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cexpl {
    // c:40
    /// Display even without matches.
    pub always: i32, // c:41
    /// The string itself.
    pub str: Option<String>, // c:42 (Rust keyword `str`)
    /// Number of matches.
    pub count: i32, // c:43
    /// Number of matches with fignore ignored.
    pub fcount: i32, // c:44
}

/// Port of `struct cmgroup` from `Src/Zle/comp.h:49-82`. A group of
/// completion matches (one per `compadd -J GROUP`).
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cmgroup {
    // c:49
    /// Group name.
    pub name: Option<String>, // c:50
    /// Previous group in the list.
    pub prev: Option<Box<Cmgroup>>, // c:51
    /// Next group in the list.
    pub next: Option<Box<Cmgroup>>, // c:52
    /// CGF_* flags.
    pub flags: i32, // c:53
    /// Number of matches.
    pub mcount: i32, // c:54
    /// The matches.
    pub matches: Vec<Cmatch>, // c:55
    /// Number of things to list here.
    pub lcount: i32, // c:56
    /// Number of line-displays.
    pub llcount: i32, // c:57
    /// Things to list.
    pub ylist: Vec<String>, // c:58
    /// Number of explanation strings.
    pub ecount: i32, // c:59
    /// Explanation strings.
    pub expls: Vec<Cexpl>, // c:60
    /// Number of compctls used.
    pub ccount: i32, // c:61
    /// LinkList of explanations (mid-build accumulator before `expls`).
    ///
    /// c:62 — in C the `l*` accumulators ARE the LinkLists the file-scope
    /// `expls`/`matches`/`fmatches`/`allccs` globals alias (`begcmgroup`:
    /// `matches = mgroup->lmatches`). The port makes that alias explicit with a
    /// shared `Arc<Mutex<…>>`: every clone of this group (into `amatches` AND
    /// `mgroup`) and the file-scope handle share ONE allocation, so a `compadd`
    /// append flows into all of them with no copy — killing the copy-desync bug
    /// class. See `begcmgroup` / `crate::comp_match_handles`.
    pub lexpls: std::sync::Arc<std::sync::Mutex<Vec<Cexpl>>>, // c:62
    /// LinkList of matches (mid-build accumulator before `matches`).
    pub lmatches: std::sync::Arc<std::sync::Mutex<Vec<Cmatch>>>, // c:63
    /// LinkList of matches with fignore-removed entries kept.
    pub lfmatches: std::sync::Arc<std::sync::Mutex<Vec<Cmatch>>>, // c:64
    /// LinkList of compctls used (mid-build accumulator).
    pub lallccs: std::sync::Arc<std::sync::Mutex<Vec<String>>>, // c:65
    /// Group number.
    pub num: i32, // c:66
    /// Number of opened braces.
    pub nbrbeg: i32, // c:67
    /// Number of closed braces.
    pub nbrend: i32, // c:68
    /// New matches since last permalloc().
    pub new_: i32, // c:69 (Rust keyword `new`)
    // c:71-77 — listing accumulators.
    /// Number of matches to list in columns.
    pub dcount: i32, // c:71
    /// Number of columns.
    pub cols: i32, // c:72
    /// Number of lines.
    pub lins: i32, // c:73
    /// Column width.
    pub width: i32, // c:74
    /// Per-column widths for listpacked.
    pub widths: Vec<i32>, // c:75
    /// Total length.
    pub totl: i32, // c:76
    /// Length of shortest match.
    pub shortest: i32, // c:77
    /// Permanent-alloc version of this group (the C source's
    /// shadow-copy used to survive heap resets).
    pub perm: Option<Box<Cmgroup>>, // c:78
}

/// Port of `struct brinfo` from `Src/Zle/zle.h:368-375`. Brace-info
/// node — tracks one `{` or `}` position in the pattern being
/// completed, used by `compadd -b` brace-aware matching.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Brinfo {
    // zle.h:368
    /// Next in list.
    pub next: Option<Box<Brinfo>>, // zle.h:369
    /// Previous (only for closing braces).
    pub prev: Option<Box<Brinfo>>, // zle.h:370
    /// The string to insert.
    pub str: Option<String>, // zle.h:371
    /// Original position.
    pub pos: i32, // zle.h:372
    /// Original position with quoting.
    pub qpos: i32, // zle.h:373
    /// Position for current match.
    pub curpos: i32, // zle.h:374
}

/// Port of `struct cmatch` from `Src/Zle/comp.h:99-125`. A single
/// completion match.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cmatch {
    // c:99
    /// The match itself.
    pub str: Option<String>, // c:100 (Rust keyword)
    /// The match string unquoted.
    pub orig: Option<String>, // c:101
    /// Ignored prefix, has to be re-inserted.
    pub ipre: Option<String>, // c:102
    /// Ignored prefix, unquoted.
    pub ripre: Option<String>, // c:103
    /// Ignored suffix.
    pub isuf: Option<String>, // c:104
    /// The path prefix.
    pub ppre: Option<String>, // c:105
    /// The path suffix.
    pub psuf: Option<String>, // c:106
    /// Path prefix for opendir.
    pub prpre: Option<String>, // c:107
    /// Prefix string from -P.
    pub pre: Option<String>, // c:108
    /// Suffix string from -S.
    pub suf: Option<String>, // c:109
    /// String to display (compadd -d).
    pub disp: Option<String>, // c:110
    /// Closing quote to add automatically.
    pub autoq: Option<String>, // c:111
    /// CMF_* flags (cmatch namespace).
    pub flags: i32, // c:112
    /// Places where to put the brace prefixes.
    pub brpl: Vec<i32>, // c:113
    /// ...and the suffixes.
    pub brsl: Vec<i32>, // c:114
    /// When to remove the suffix.
    pub rems: Option<String>, // c:115
    /// Shell function to call for suffix-removal.
    pub remf: Option<String>, // c:116
    /// Length of quote-prefix.
    pub qipl: i32, // c:117
    /// Length of quote-suffix.
    pub qisl: i32, // c:118
    /// Group-relative number.
    pub rnum: i32, // c:119
    /// Global number.
    pub gnum: i32, // c:120
    /// `mode` field of a stat.
    pub mode: u32, // c:121 (mode_t → u32)
    /// LIST_TYPE-character for mode or 0.
    pub modec: char, // c:122
    /// `mode` field of a stat, following symlink.
    pub fmode: u32, // c:123 (mode_t → u32)
    /// LIST_TYPE-character for fmode or 0.
    pub fmodec: char, // c:124
}

/// Port of `struct cmlist` from `Src/Zle/comp.h:147-151`. Linked
/// list of global matchers.
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct Cmlist {
    // c:147
    /// Next entry in the list.
    pub next: Option<Box<Cmlist>>, // c:148
    /// The matcher definition.
    pub matcher: Box<Cmatcher>, // c:149
    /// The string for it.
    pub str: String, // c:150
}

/// Port of `struct cmatcher` from `Src/Zle/comp.h:153-167`. Matcher
/// specification — what to match on the line vs in the trial word,
/// with optional left/right anchors.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cmatcher {
    // c:153
    /// Reference counter.
    pub refc: i32, // c:154
    /// Next matcher.
    pub next: Option<Box<Cmatcher>>, // c:155
    /// CMF_LINE/CMF_LEFT/CMF_RIGHT/CMF_INTER (cmatcher namespace).
    pub flags: i32, // c:156
    /// What matches on the line.
    pub line: Option<Box<Cpattern>>, // c:157
    /// Length of line pattern.
    pub llen: i32, // c:158
    /// What matches in the word.
    pub word: Option<Box<Cpattern>>, // c:159
    /// Length of word pattern, or:
    /// -1: word pattern is one asterisk
    /// -2: word pattern is two asterisks
    pub wlen: i32, // c:160
    /// Left anchor.
    pub left: Option<Box<Cpattern>>, // c:163
    /// Length of left anchor.
    pub lalen: i32, // c:164
    /// Right anchor.
    pub right: Option<Box<Cpattern>>, // c:165
    /// Length of right anchor.
    pub ralen: i32, // c:166
}

/// Port of `struct cpattern` from `Src/Zle/comp.h:197-210`. A
/// single pattern element in a matcher specification — represents
/// one character either in the trial completion or in the word on
/// the command line.
///
/// The C `union { char *str; convchar_t chr; } u` is dispatched by
/// `tp` (a `CPAT_*` constant). The Rust port keeps both fields with
/// `Option`s so the dispatcher reads only the live one.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cpattern {
    // c:197
    /// Next sub-pattern.
    pub next: Option<Box<Cpattern>>, // c:198
    /// Type of object — one of CPAT_*.
    pub tp: i32, // c:199
    /// If a character class (CPAT_CCLASS/CPAT_NCLASS/CPAT_EQUIV),
    /// the objects in it as a metafied byte sequence — the encoded
    /// format matches `Src/pattern.c`'s `patmatchindex` reader:
    /// `0x80 + PP_*` for POSIX classes, `0x80 + PP_RANGE` + two
    /// bytes for ranges, plain bytes for literals. Storing as
    /// `Vec<u8>` (not `String`) preserves the 0x80-0xBF marker
    /// bytes that would otherwise mangle under UTF-8 validation.
    pub str: Option<Vec<u8>>, // c:201 union.u.str
    /// If a single character (CPAT_CHAR), it.
    pub chr: u32, // c:208 union.u.chr (convchar_t)
}

/// Port of `struct cline` from `Src/Zle/comp.h:245-257`. One
/// word-part in the unambiguous-line-string list. Threaded prefix /
/// suffix sub-lists via the `prefix`/`suffix` fields.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cline {
    // c:245
    /// Next sibling word-part.
    pub next: Option<Box<Cline>>, // c:246
    /// CLF_* flags.
    pub flags: i32, // c:247
    /// Line string for this part.
    pub line: Option<String>, // c:248
    /// Length of `line`.
    pub llen: i32, // c:249
    /// Word string for this part.
    pub word: Option<String>, // c:250
    /// Length of `word`.
    pub wlen: i32, // c:251
    /// Original (unjoined) string.
    pub orig: Option<String>, // c:252
    /// Length of `orig`.
    pub olen: i32, // c:253
    /// String length (the join-up version).
    pub slen: i32, // c:254
    /// Prefix sub-list.
    pub prefix: Option<Box<Cline>>, // c:255
    /// Suffix sub-list.
    pub suffix: Option<Box<Cline>>, // c:255
    /// Min length seen for this part (joining metric).
    pub min: i32, // c:256
    /// Max length seen for this part (joining metric).
    pub max: i32, // c:256
}

/// Port of `struct aminfo` from `Src/Zle/comp.h:274-280`. Holds
/// info about ambiguous completions — there's one for fignore-
/// ignored and one for normal completion.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Aminfo {
    // c:274
    /// The first match.
    pub firstm: Option<Box<Cmatch>>, // c:275
    /// If there was an exact match.
    pub exact: i32, // c:276
    /// The exact match (if any).
    pub exactm: Option<Box<Cmatch>>, // c:277
    /// Number of matches.
    pub count: i32, // c:278
    /// Unambiguous line string.
    pub line: Option<Box<Cline>>, // c:279
}

/// Port of `struct menuinfo` from `Src/Zle/comp.h:284-295`.
/// Menu-completion state.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Menuinfo {
    // c:284
    /// Position in the group list.
    pub group: Option<Box<Cmgroup>>, // c:285
    /// Match currently inserted.
    pub cur: Option<Box<Cmatch>>, // c:286
    /// Index of `group` within the `amatches` Vec. C models `group` as
    /// a live `Cmgroup` pointer into the amatches linked list; the Rust
    /// store is a `Vec<Cmgroup>`, so the pointer is carried as this
    /// index. Meaningful whenever `group` is `Some`.
    pub group_idx: i32,
    /// Index of `cur` within `amatches[group_idx].matches`. C models
    /// `cur` as a `Cmatch *` pointing into the group's NUL-terminated
    /// `matches` array, and advances it with pointer arithmetic
    /// (`++minfo.cur`, `m--`). The Rust store is `Vec<Cmatch>`, so the
    /// pointer offset (`cur - group->matches`) is carried as this index.
    /// Meaningful whenever `cur` is `Some`.
    pub cur_idx: i32,
    /// Begin on line.
    pub pos: i32, // c:287
    /// Length of inserted string.
    pub len: i32, // c:288
    /// End on the line.
    pub end: i32, // c:289
    /// Non-zero if the cursor was at the end.
    pub we: i32, // c:290
    /// Length of suffix inserted.
    pub insc: i32, // c:291
    /// We asked if the list should be shown.
    pub asked: i32, // c:292
    /// Prefix before a brace, if any.
    pub prebr: Option<String>, // c:293
    /// Suffix after a brace.
    pub postbr: Option<String>, // c:294
}

/// Port of `struct ccmakedat` from `Src/Zle/comp.h:455-459`. Hook
/// data passed to the compctl-make path.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Ccmakedat {
    // c:455
    /// String passed to the hook.
    pub str: Option<String>, // c:456
    /// Whether we're in a command position.
    pub incmd: i32, // c:457
    /// List flag.
    pub lst: i32, // c:458
}

/// Port of `struct chdata` from `Src/Zle/comp.h:465-470`. Data
/// given to `offered` hooks.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Chdata {
    // c:465
    /// The matches generated.
    pub matches: Option<Box<Cmgroup>>, // c:466
    /// Number of matches.
    pub num: i32, // c:467
    /// Number of messages.
    pub nmesg: i32, // c:468
    /// Current match or None.
    pub cur: Option<Box<Cmatch>>, // c:469
}

/// Port of `struct cadata` from `Src/Zle/comp.h:315-337`. Data
/// passed to compadd / addmatches().
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cadata {
    // c:315
    /// Ignored prefix (-i).
    pub ipre: Option<String>, // c:316
    /// Ignored suffix (-I).
    pub isuf: Option<String>, // c:317
    /// `path` prefix (-p).
    pub ppre: Option<String>, // c:318
    /// `path` suffix (-s).
    pub psuf: Option<String>, // c:319
    /// Expanded `path` prefix (-W).
    pub prpre: Option<String>, // c:320
    /// Prefix to insert (-P).
    pub pre: Option<String>, // c:321
    /// Suffix to insert (-S).
    pub suf: Option<String>, // c:322
    /// Name of the group (`-[JV]`).
    pub group: Option<String>, // c:323
    /// Remove suffix on chars... (-r).
    pub rems: Option<String>, // c:324
    /// Function to remove suffix (-R).
    pub remf: Option<String>, // c:325
    /// Ignored suffixes (-F).
    pub ign: Option<String>, // c:326
    /// CMF_* flags (`-[fqn]`).
    pub flags: i32, // c:327
    /// CAF_* flags (`-[QUa]`).
    pub aflags: i32, // c:328
    /// Match spec (parsed from -M).
    pub match_: Option<Box<Cmatcher>>, // c:329 (Rust keyword)
    /// Explanation (-X).
    pub exp: Option<String>, // c:330
    /// Array to store matches in (-A).
    pub apar: Option<String>, // c:331
    /// Array to store originals in (-O).
    pub opar: Option<String>, // c:332
    /// Arrays to delete non-matches in (-D).
    pub dpar: Vec<String>, // c:333
    /// Array with display lists (-d).
    pub disp: Option<String>, // c:334
    /// Message to show unconditionally (-x).
    pub mesg: Option<String>, // c:335
    /// Add that many dummy matches.
    pub dummies: i32, // c:336
}

/// Port of `struct cldata` from `Src/Zle/comp.h:343-353`. List data
/// for the matches-listing path.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cldata {
    // c:343
    /// Screen width.
    pub zterm_columns: i32, // c:344
    /// Screen height.
    pub zterm_lines: i32, // c:345
    /// Value of global menuacc.
    pub menuacc: i32, // c:346
    /// No need to calculate anew.
    pub valid: i32, // c:347
    /// Number of matches to list.
    pub nlist: i32, // c:348
    /// Number of lines needed.
    pub nlines: i32, // c:349
    /// != 0 if there are hidden matches.
    pub hidden: i32, // c:350
    /// != 0 if only explanations to print.
    pub onlyexpl: i32, // c:351
    /// != 0 if hidden matches should be shown.
    pub showall: i32, // c:352
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::zle_main::zle_test_setup;

    /// Verifies CGF_* group flag values per c:85-95.
    #[test]
    fn cgf_flags_correct() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(CGF_NOSORT, 1);
        assert_eq!(CGF_LINES, 2);
        assert_eq!(CGF_HASDL, 4);
        assert_eq!(CGF_REVSORT, 1024);
    }

    /// Verifies CMF_* match flags are non-overlapping single-bits
    /// per c:127-143.
    #[test]
    fn cmf_match_flags_distinct() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let all = CMF_FILE
            | CMF_REMOVE
            | CMF_ISPAR
            | CMF_PARBR
            | CMF_PARNEST
            | CMF_NOLIST
            | CMF_DISPLINE
            | CMF_HIDE
            | CMF_NOSPACE
            | CMF_PACKED
            | CMF_ROWS
            | CMF_MULT
            | CMF_FMULT
            | CMF_ALL
            | CMF_DUMMY
            | CMF_MORDER
            | CMF_DELETE;
        assert_eq!(all.count_ones(), 17);
    }

    /// Verifies CMF_LINE/LEFT/RIGHT/INTER cmatcher flags per c:172-178.
    #[test]
    fn cmf_matcher_flags_correct() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(CMF_LINE, 1);
        assert_eq!(CMF_LEFT, 2);
        assert_eq!(CMF_RIGHT, 4);
        assert_eq!(CMF_INTER, 8);
    }

    /// Verifies CPAT_* enum values per c:184-190.
    #[test]
    fn cpat_enum_values_correct() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(CPAT_CCLASS, 0);
        assert_eq!(CPAT_NCLASS, 1);
        assert_eq!(CPAT_EQUIV, 2);
        assert_eq!(CPAT_ANY, 3);
        assert_eq!(CPAT_CHAR, 4);
    }

    /// Verifies CP_REALPARAMS / CP_ALLREALS aggregate per c:385-386.
    #[test]
    fn cp_realparams_mask_covers_10_bits() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(CP_REALPARAMS, 10);
        assert_eq!(CP_ALLREALS, 0x3ff);
        assert_eq!(CP_ALLREALS.count_ones(), 10);
        assert_eq!(
            CP_WORDS
                | CP_REDIRS
                | CP_CURRENT
                | CP_PREFIX
                | CP_SUFFIX
                | CP_IPREFIX
                | CP_ISUFFIX
                | CP_QIPREFIX
                | CP_QISUFFIX
                | CP_COMPSTATE,
            CP_ALLREALS
        );
    }

    /// Verifies CP_KEYPARAMS / CP_ALLKEYS aggregate per c:442-443.
    #[test]
    fn cp_keyparams_mask_covers_26_bits() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(CP_KEYPARAMS, 26);
        assert_eq!(CP_ALLKEYS, 0x3ffffff);
        assert_eq!(CP_ALLKEYS.count_ones(), 26);
    }

    /// Verifies CAF_* compadd flags per c:299-309.
    #[test]
    fn caf_flags_correct() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(CAF_QUOTE, 1);
        assert_eq!(CAF_NOSORT, 2);
        assert_eq!(CAF_REVSORT, 1024);
    }

    /// Verifies hook offset constants per c:447-451.
    #[test]
    fn hook_offsets_sequential() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(INSERTMATCHHOOK_OFFSET, 0);
        assert_eq!(MENUSTARTHOOK_OFFSET, 1);
        assert_eq!(COMPCTLMAKEHOOK_OFFSET, 2);
        assert_eq!(COMPCTLCLEANUPHOOK_OFFSET, 3);
        assert_eq!(COMPLISTMATCHESHOOK_OFFSET, 4);
    }

    /// Verifies CM_SPACE per c:474.
    #[test]
    fn cm_space_is_2() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(CM_SPACE, 2);
    }

    /// Verifies the structs construct cleanly with `Default`.
    #[test]
    fn structs_default_construct() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _ = Cexpl::default();
        let _ = Cmgroup::default();
        let _ = Cmatch::default();
        let _ = Cmatcher::default();
        let _ = Cpattern::default();
        let _ = Cline::default();
        let _ = Aminfo::default();
        let _ = Menuinfo::default();
        let _ = Ccmakedat::default();
        let _ = Chdata::default();
        let _ = Cadata::default();
        let _ = Cldata::default();
    }

    /// `Src/Zle/comp.h:85-95` — `CGF_*` completion group flags.
    /// Pin every bit value vs the canonical C define.
    #[test]
    fn cgf_flags_match_c_comp_h_canonical_values() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(CGF_NOSORT, 1, "c:85");
        assert_eq!(CGF_LINES, 2, "c:86");
        assert_eq!(CGF_HASDL, 4, "c:87");
        assert_eq!(CGF_UNIQALL, 8, "c:88");
        assert_eq!(CGF_UNIQCON, 16, "c:89");
        assert_eq!(CGF_PACKED, 32, "c:90");
        assert_eq!(CGF_ROWS, 64, "c:91");
        assert_eq!(CGF_FILES, 128, "c:92");
        assert_eq!(CGF_MATSORT, 256, "c:93");
        assert_eq!(CGF_NUMSORT, 512, "c:94");
        assert_eq!(CGF_REVSORT, 1024, "c:95");
    }

    /// `Src/Zle/comp.h:127-143` — `CMF_*` completion-match flags
    /// (Cmatch struct, not Cmatcher). 17 flags total.
    #[test]
    fn cmf_match_flags_match_c_comp_h_canonical_values() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(CMF_FILE, 1 << 0, "c:127");
        assert_eq!(CMF_REMOVE, 1 << 1, "c:128");
        assert_eq!(CMF_ISPAR, 1 << 2, "c:129");
        assert_eq!(CMF_PARBR, 1 << 3, "c:130");
        assert_eq!(CMF_PARNEST, 1 << 4, "c:131");
        assert_eq!(CMF_NOLIST, 1 << 5, "c:132");
        assert_eq!(CMF_DISPLINE, 1 << 6, "c:133");
        assert_eq!(CMF_HIDE, 1 << 7, "c:134");
        assert_eq!(CMF_NOSPACE, 1 << 8, "c:135");
        assert_eq!(CMF_PACKED, 1 << 9, "c:136");
        assert_eq!(CMF_ROWS, 1 << 10, "c:137");
        assert_eq!(CMF_MULT, 1 << 11, "c:138");
        assert_eq!(CMF_FMULT, 1 << 12, "c:139");
        assert_eq!(CMF_ALL, 1 << 13, "c:140");
        assert_eq!(CMF_DUMMY, 1 << 14, "c:141");
        assert_eq!(CMF_MORDER, 1 << 15, "c:142");
        assert_eq!(CMF_DELETE, 1 << 16, "c:143");
    }

    /// c:85-95 — CGF_* group flags are distinct single bits.
    /// Pin the bit-packing because the c:85 mask is OR'd into a
    /// shared `cgflags` field.
    #[test]
    fn cgf_group_flags_are_distinct_single_bits() {
        let _g = crate::test_util::global_state_lock();
        let flags = [
            CGF_NOSORT,
            CGF_LINES,
            CGF_HASDL,
            CGF_UNIQALL,
            CGF_UNIQCON,
            CGF_PACKED,
            CGF_ROWS,
            CGF_FILES,
            CGF_MATSORT,
            CGF_NUMSORT,
            CGF_REVSORT,
        ];
        for &f in &flags {
            assert_eq!(
                (f as u32).count_ones(),
                1,
                "CGF flag {} = {:#x} must be a single bit",
                f,
                f
            );
        }
        let mut all: u32 = 0;
        for &f in &flags {
            let bit = f as u32;
            assert_eq!(
                all & bit,
                0,
                "CGF flag {:#x} overlaps with existing bits",
                bit
            );
            all |= bit;
        }
    }

    /// c:127-143 — CMF_* flags are likewise distinct single bits.
    /// Pin no overlap across the 17 entries.
    #[test]
    fn cmf_match_flags_are_distinct_single_bits() {
        let _g = crate::test_util::global_state_lock();
        let flags = [
            CMF_FILE,
            CMF_REMOVE,
            CMF_ISPAR,
            CMF_PARBR,
            CMF_PARNEST,
            CMF_NOLIST,
            CMF_DISPLINE,
            CMF_HIDE,
            CMF_NOSPACE,
            CMF_PACKED,
            CMF_ROWS,
            CMF_MULT,
            CMF_FMULT,
            CMF_ALL,
            CMF_DUMMY,
            CMF_MORDER,
            CMF_DELETE,
        ];
        for &f in &flags {
            assert_eq!(
                (f as u32).count_ones(),
                1,
                "CMF flag {} = {:#x} must be a single bit",
                f,
                f
            );
        }
        let mut all: u32 = 0;
        for &f in &flags {
            let bit = f as u32;
            assert_eq!(all & bit, 0, "CMF flag {:#x} overlaps", bit);
            all |= bit;
        }
    }

    /// c:85 — CGF_NOSORT must be bit 0 (the lowest). The C source
    /// uses `cgflags & CGF_NOSORT` in many places; the bit order
    /// being load-bearing on bit 0 is the convention for the
    /// `nosort` early-exit branch.
    #[test]
    fn cgf_nosort_is_bit_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            CGF_NOSORT, 1,
            "CGF_NOSORT must be bit 0 — the early-exit `!sort` test"
        );
    }

    /// c:127 — CMF_FILE must be bit 0. Pin the convention because
    /// file-matches are the most-common dispatch case and the
    /// fast-path bit-test relies on bit 0.
    #[test]
    fn cmf_file_is_bit_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            CMF_FILE, 1,
            "CMF_FILE must be bit 0 — the file-match fast-path"
        );
    }

    /// c:85-95 — Sanity: CGF flag values match what the C source
    /// declares. Spot-check the high end of the table.
    #[test]
    fn cgf_top_flags_match_canonical_high_bits() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(CGF_PACKED, 32);
        assert_eq!(CGF_ROWS, 64);
        assert_eq!(CGF_FILES, 128);
        assert_eq!(CGF_MATSORT, 256);
        assert_eq!(CGF_NUMSORT, 512);
        assert_eq!(CGF_REVSORT, 1024);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/comp.h CMF_* + CGF_*.
    // ═══════════════════════════════════════════════════════════════════

    /// c:127-134 — first 8 CMF_* flags (1<<0 through 1<<7).
    #[test]
    fn cmf_low_byte_flags_are_single_bits() {
        assert_eq!(CMF_FILE, 1);
        assert_eq!(CMF_REMOVE, 2);
        assert_eq!(CMF_ISPAR, 4);
        assert_eq!(CMF_PARBR, 8);
        assert_eq!(CMF_PARNEST, 16);
        assert_eq!(CMF_NOLIST, 32);
        assert_eq!(CMF_DISPLINE, 64);
        assert_eq!(CMF_HIDE, 128);
    }

    /// c:135-143 — second half of CMF_* (1<<8 through 1<<16).
    #[test]
    fn cmf_high_byte_flags_are_single_bits() {
        assert_eq!(CMF_NOSPACE, 1 << 8);
        assert_eq!(CMF_PACKED, 1 << 9);
        assert_eq!(CMF_ROWS, 1 << 10);
        assert_eq!(CMF_MULT, 1 << 11);
        assert_eq!(CMF_FMULT, 1 << 12);
        assert_eq!(CMF_ALL, 1 << 13);
        assert_eq!(CMF_DUMMY, 1 << 14);
        assert_eq!(CMF_MORDER, 1 << 15);
        assert_eq!(CMF_DELETE, 1 << 16);
    }

    /// c:127-143 — Cmatch CMF_* flags pairwise disjoint single-bit
    /// values (the 17-flag bitfield).
    #[test]
    fn cmf_match_flags_pairwise_disjoint() {
        let flags = [
            CMF_FILE,
            CMF_REMOVE,
            CMF_ISPAR,
            CMF_PARBR,
            CMF_PARNEST,
            CMF_NOLIST,
            CMF_DISPLINE,
            CMF_HIDE,
            CMF_NOSPACE,
            CMF_PACKED,
            CMF_ROWS,
            CMF_MULT,
            CMF_FMULT,
            CMF_ALL,
            CMF_DUMMY,
            CMF_MORDER,
            CMF_DELETE,
        ];
        for i in 0..flags.len() {
            for j in (i + 1)..flags.len() {
                assert_eq!(
                    flags[i] & flags[j],
                    0,
                    "CMF_* flags {} and {} must not overlap",
                    flags[i],
                    flags[j]
                );
            }
        }
    }

    /// c:85-95 — CGF_* flags pairwise disjoint single bits.
    #[test]
    fn cgf_flags_pairwise_disjoint() {
        let flags = [
            CGF_NOSORT,
            CGF_LINES,
            CGF_HASDL,
            CGF_UNIQALL,
            CGF_UNIQCON,
            CGF_PACKED,
            CGF_ROWS,
            CGF_FILES,
            CGF_MATSORT,
            CGF_NUMSORT,
        ];
        for i in 0..flags.len() {
            for j in (i + 1)..flags.len() {
                assert_eq!(
                    flags[i] & flags[j],
                    0,
                    "CGF_* flags {} and {} must not overlap",
                    flags[i],
                    flags[j]
                );
            }
        }
    }

    /// c:85-89 — low CGF_* bit values (1, 2, 4, 8, 16).
    #[test]
    fn cgf_low_bits_canonical() {
        assert_eq!(CGF_NOSORT, 1);
        assert_eq!(CGF_LINES, 2);
        assert_eq!(CGF_HASDL, 4);
        assert_eq!(CGF_UNIQALL, 8);
        assert_eq!(CGF_UNIQCON, 16);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/comp.h
    // c:185-189 CPAT_* / c:259-267 CLF_* / c:299-... CAF_* / c:172-178 CMF_*
    // ═══════════════════════════════════════════════════════════════════

    /// c:185-189 — CPAT_* enum values are sequential 0..5.
    #[test]
    fn cpat_enum_sequential_0_through_4() {
        assert_eq!(CPAT_CCLASS, 0, "c:185");
        assert_eq!(CPAT_NCLASS, 1, "c:186");
        assert_eq!(CPAT_EQUIV, 2, "c:187");
        assert_eq!(CPAT_ANY, 3, "c:188");
        assert_eq!(CPAT_CHAR, 4, "c:189");
    }

    /// c:185-189 — CPAT_* are all distinct.
    #[test]
    fn cpat_enum_pairwise_distinct() {
        let codes = [CPAT_CCLASS, CPAT_NCLASS, CPAT_EQUIV, CPAT_ANY, CPAT_CHAR];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len(), "CPAT_* must be distinct");
    }

    /// c:259-267 — CLF_* flags are powers of 2 (single bits).
    #[test]
    fn clf_flags_are_single_bits() {
        for &v in &[
            CLF_MISS,
            CLF_DIFF,
            CLF_SUF,
            CLF_MID,
            CLF_NEW,
            CLF_LINE,
            CLF_JOIN,
            CLF_MATCHED,
            CLF_SKIP,
        ] {
            assert!(
                (v as u32).is_power_of_two(),
                "CLF_* flag {} must be a single bit",
                v
            );
        }
    }

    /// c:259-267 — CLF_* canonical low-bit values.
    #[test]
    fn clf_canonical_values() {
        assert_eq!(CLF_MISS, 1, "c:259");
        assert_eq!(CLF_DIFF, 2, "c:260");
        assert_eq!(CLF_SUF, 4, "c:261");
        assert_eq!(CLF_MID, 8, "c:262");
        assert_eq!(CLF_NEW, 16, "c:263");
        assert_eq!(CLF_LINE, 32, "c:264");
        assert_eq!(CLF_JOIN, 64, "c:265");
        assert_eq!(CLF_MATCHED, 128, "c:266");
        assert_eq!(CLF_SKIP, 256, "c:267");
    }

    /// c:259-267 — CLF_* flags pairwise distinct.
    #[test]
    fn clf_flags_pairwise_distinct() {
        let codes = [
            CLF_MISS,
            CLF_DIFF,
            CLF_SUF,
            CLF_MID,
            CLF_NEW,
            CLF_LINE,
            CLF_JOIN,
            CLF_MATCHED,
            CLF_SKIP,
        ];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len(), "CLF_* must be pairwise distinct");
    }

    /// c:299-302 — CAF_* canonical values.
    #[test]
    fn caf_canonical_values() {
        assert_eq!(CAF_QUOTE, 1, "c:299");
        assert_eq!(CAF_NOSORT, 2, "c:300");
        assert_eq!(CAF_MATCH, 4, "c:301");
        assert_eq!(CAF_UNIQCON, 8, "c:302");
    }

    /// c:299-302 — CAF_* are single bits.
    #[test]
    fn caf_flags_are_single_bits() {
        for &v in &[CAF_QUOTE, CAF_NOSORT, CAF_MATCH, CAF_UNIQCON] {
            assert!(
                (v as u32).is_power_of_two(),
                "CAF_* flag {} must be a single bit",
                v
            );
        }
    }

    /// c:172-178 — secondary CMF_* (LINE/LEFT/RIGHT/INTER) overlap with
    /// the main CMF_FILE/REMOVE/ISPAR/PARBR by design (different namespace).
    /// Pin canonical secondary values.
    #[test]
    fn cmf_secondary_canonical_values() {
        assert_eq!(CMF_LINE, 1, "c:172");
        assert_eq!(CMF_LEFT, 2, "c:174");
        assert_eq!(CMF_RIGHT, 4, "c:176");
        assert_eq!(CMF_INTER, 8, "c:178");
    }

    /// c:172-178 — secondary CMF_* are single bits in their namespace.
    #[test]
    fn cmf_secondary_flags_are_single_bits() {
        for &v in &[CMF_LINE, CMF_LEFT, CMF_RIGHT, CMF_INTER] {
            assert!(
                (v as u32).is_power_of_two(),
                "CMF secondary flag {} must be a single bit",
                v
            );
        }
    }

    /// c:127-143 — all CMF_* main flags are non-negative i32.
    #[test]
    fn cmf_main_flags_all_non_negative() {
        for &v in &[
            CMF_FILE,
            CMF_REMOVE,
            CMF_ISPAR,
            CMF_PARBR,
            CMF_PARNEST,
            CMF_NOLIST,
            CMF_DISPLINE,
            CMF_HIDE,
            CMF_NOSPACE,
            CMF_PACKED,
            CMF_ROWS,
            CMF_MULT,
            CMF_FMULT,
            CMF_ALL,
            CMF_DUMMY,
            CMF_MORDER,
            CMF_DELETE,
        ] {
            assert!(v >= 0, "CMF_* flag {} must be ≥ 0", v);
        }
    }

    /// c:85-95 — CGF_* are powers of two (full sweep).
    #[test]
    fn cgf_all_flags_are_single_bits() {
        for &v in &[
            CGF_NOSORT,
            CGF_LINES,
            CGF_HASDL,
            CGF_UNIQALL,
            CGF_UNIQCON,
            CGF_PACKED,
            CGF_ROWS,
            CGF_FILES,
            CGF_MATSORT,
            CGF_NUMSORT,
            CGF_REVSORT,
        ] {
            assert!(
                (v as u32).is_power_of_two(),
                "CGF_* flag {} must be a single bit",
                v
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/comp.h
    // c:364-440 CPN_*/CP_* completion-param flags + hook offsets c:447-451
    // ═══════════════════════════════════════════════════════════════════

    /// c:364-440 — every CP_* equals 1 << CPN_* (definition contract).
    #[test]
    fn cp_flags_match_cpn_shift_definition() {
        assert_eq!(CP_WORDS, 1u32 << CPN_WORDS, "c:365");
        assert_eq!(CP_REDIRS, 1u32 << CPN_REDIRS, "c:367");
        assert_eq!(CP_CURRENT, 1u32 << CPN_CURRENT, "c:369");
        assert_eq!(CP_PREFIX, 1u32 << CPN_PREFIX, "c:371");
        assert_eq!(CP_SUFFIX, 1u32 << CPN_SUFFIX, "c:373");
    }

    /// c:419-440 — high-bit CP_* still match `1 << CPN_*`.
    #[test]
    fn cp_high_bit_flags_match_cpn_shift_definition() {
        assert_eq!(CP_UNAMBIGP, 1u32 << CPN_UNAMBIGP, "c:420");
        assert_eq!(CP_INSERTP, 1u32 << CPN_INSERTP, "c:422");
        assert_eq!(CP_LISTMAX, 1u32 << CPN_LISTMAX, "c:424");
        assert_eq!(CP_LASTPROMPT, 1u32 << CPN_LASTPROMPT, "c:426");
        assert_eq!(CP_TOEND, 1u32 << CPN_TOEND, "c:428");
        assert_eq!(CP_OLDLIST, 1u32 << CPN_OLDLIST, "c:430");
        assert_eq!(CP_OLDINS, 1u32 << CPN_OLDINS, "c:432");
        assert_eq!(CP_VARED, 1u32 << CPN_VARED, "c:434");
        assert_eq!(CP_LISTLINES, 1u32 << CPN_LISTLINES, "c:436");
        assert_eq!(CP_QUOTES, 1u32 << CPN_QUOTES, "c:438");
        assert_eq!(CP_IGNORED, 1u32 << CPN_IGNORED, "c:440");
    }

    /// c:364-440 — CPN_* indices are pairwise distinct and in 0..=25.
    #[test]
    fn cpn_indices_pairwise_distinct_and_in_range() {
        let cpns: Vec<i32> = vec![
            CPN_WORDS,
            CPN_REDIRS,
            CPN_CURRENT,
            CPN_PREFIX,
            CPN_SUFFIX,
            CPN_UNAMBIGP,
            CPN_INSERTP,
            CPN_LISTMAX,
            CPN_LASTPROMPT,
            CPN_TOEND,
            CPN_OLDLIST,
            CPN_OLDINS,
            CPN_VARED,
            CPN_LISTLINES,
            CPN_QUOTES,
            CPN_IGNORED,
        ];
        let unique: std::collections::HashSet<_> = cpns.iter().copied().collect();
        assert_eq!(unique.len(), cpns.len(), "CPN_* must be pairwise distinct");
        for &v in &cpns {
            assert!(
                v >= 0 && v <= 25,
                "CPN_* index {} must be in 0..=25 range",
                v
            );
        }
    }

    /// c:443 — `CP_ALLKEYS = 0x3ffffff` (low 26 bits set).
    #[test]
    fn cp_allkeys_is_low_26_bits() {
        assert_eq!(
            CP_ALLKEYS,
            (1u32 << 26) - 1,
            "CP_ALLKEYS must be (1<<26)-1 = 0x3ffffff"
        );
    }

    /// c:442 — `CP_KEYPARAMS = 26` (matches the bit-count of CP_ALLKEYS).
    #[test]
    fn cp_keyparams_count_matches_allkeys_bit_width() {
        assert_eq!(
            CP_KEYPARAMS, 26,
            "CP_KEYPARAMS must equal CP_ALLKEYS bit count"
        );
        assert_eq!(
            (CP_ALLKEYS + 1).trailing_zeros() as i32,
            CP_KEYPARAMS,
            "CP_ALLKEYS+1 leading bit position = CP_KEYPARAMS"
        );
    }

    /// c:447-451 — hook offsets are contiguous 0..=4.
    #[test]
    fn hook_offsets_contiguous_zero_through_four() {
        assert_eq!(INSERTMATCHHOOK_OFFSET, 0, "c:447");
        assert_eq!(MENUSTARTHOOK_OFFSET, 1, "c:448");
        assert_eq!(COMPCTLMAKEHOOK_OFFSET, 2, "c:449");
        assert_eq!(COMPCTLCLEANUPHOOK_OFFSET, 3, "c:450");
        assert_eq!(COMPLISTMATCHESHOOK_OFFSET, 4, "c:451");
    }

    /// c:447-451 — hook offsets are pairwise distinct.
    #[test]
    fn hook_offsets_pairwise_distinct() {
        let offs = [
            INSERTMATCHHOOK_OFFSET,
            MENUSTARTHOOK_OFFSET,
            COMPCTLMAKEHOOK_OFFSET,
            COMPCTLCLEANUPHOOK_OFFSET,
            COMPLISTMATCHESHOOK_OFFSET,
        ];
        let unique: std::collections::HashSet<_> = offs.iter().copied().collect();
        assert_eq!(
            unique.len(),
            offs.len(),
            "hook offsets must be pairwise distinct"
        );
    }

    /// c:474 — CM_SPACE = 2 (canonical match-spec flag).
    #[test]
    fn cm_space_canonical_value() {
        assert_eq!(CM_SPACE, 2, "c:474");
    }

    /// c:364-440 — every CPN_* fits the u32 shift width (< 32).
    #[test]
    fn cpn_indices_fit_in_u32_shift() {
        for &v in &[
            CPN_WORDS,
            CPN_REDIRS,
            CPN_CURRENT,
            CPN_PREFIX,
            CPN_SUFFIX,
            CPN_UNAMBIGP,
            CPN_INSERTP,
            CPN_LISTMAX,
            CPN_LASTPROMPT,
            CPN_TOEND,
            CPN_OLDLIST,
            CPN_OLDINS,
            CPN_VARED,
            CPN_LISTLINES,
            CPN_QUOTES,
            CPN_IGNORED,
        ] {
            assert!((v as u32) < 32, "CPN_* index {} must fit u32 shift", v);
        }
    }

    /// c:447-451 — every hook offset is a usize (compile-time type pin).
    #[test]
    fn hook_offsets_are_valid_usize_indices() {
        let _: usize = INSERTMATCHHOOK_OFFSET;
        let _: usize = MENUSTARTHOOK_OFFSET;
        let _: usize = COMPCTLMAKEHOOK_OFFSET;
        let _: usize = COMPCTLCLEANUPHOOK_OFFSET;
        let _: usize = COMPLISTMATCHESHOOK_OFFSET;
    }

    /// c:443 — CP_WORDS (bit 0) and CP_IGNORED (bit 25) are both inside
    /// CP_ALLKEYS — bitwise AND yields the flag itself.
    #[test]
    fn cp_allkeys_contains_endpoints() {
        assert_eq!(
            CP_WORDS & CP_ALLKEYS,
            CP_WORDS,
            "CP_WORDS (bit 0) ⊆ CP_ALLKEYS"
        );
        assert_eq!(
            CP_IGNORED & CP_ALLKEYS,
            CP_IGNORED,
            "CP_IGNORED (bit 25) ⊆ CP_ALLKEYS"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/comp.h
    // c:85-95 CGF_* / c:127-140 CMF_*
    // ═══════════════════════════════════════════════════════════════════

    /// c:85-95 — all CGF_* flags are i32 (compile-time type pin).
    #[test]
    fn cgf_flags_all_i32_type() {
        let _: i32 = CGF_NOSORT;
        let _: i32 = CGF_LINES;
        let _: i32 = CGF_REVSORT;
    }

    /// c:85-95 — CGF_NOSORT (bit 0) through CGF_REVSORT (bit 10) cover
    /// contiguous bits 0..=10.
    #[test]
    fn cgf_flags_dense_low_bits() {
        let all = [
            CGF_NOSORT,
            CGF_LINES,
            CGF_HASDL,
            CGF_UNIQALL,
            CGF_UNIQCON,
            CGF_PACKED,
            CGF_ROWS,
            CGF_FILES,
            CGF_MATSORT,
            CGF_NUMSORT,
            CGF_REVSORT,
        ];
        let or_all: i32 = all.iter().fold(0, |acc, &v| acc | v);
        let expected = (1i32 << 11) - 1;
        assert_eq!(or_all, expected, "CGF_* must cover bits 0..=10 (no gaps)");
    }

    /// c:85-95 — all CGF_* are powers of 2 (single-bit flags).
    #[test]
    fn cgf_flags_all_powers_of_two() {
        for &v in &[
            CGF_NOSORT,
            CGF_LINES,
            CGF_HASDL,
            CGF_UNIQALL,
            CGF_UNIQCON,
            CGF_PACKED,
            CGF_ROWS,
            CGF_FILES,
            CGF_MATSORT,
            CGF_NUMSORT,
            CGF_REVSORT,
        ] {
            assert!(
                (v as u32).is_power_of_two(),
                "CGF_* {} must be a single bit",
                v
            );
        }
    }

    /// c:85-95 — CGF_* pairwise distinct.
    #[test]
    fn cgf_flags_pairwise_distinct() {
        let codes = [
            CGF_NOSORT,
            CGF_LINES,
            CGF_HASDL,
            CGF_UNIQALL,
            CGF_UNIQCON,
            CGF_PACKED,
            CGF_ROWS,
            CGF_FILES,
            CGF_MATSORT,
            CGF_NUMSORT,
            CGF_REVSORT,
        ];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len(), "CGF_* must be pairwise distinct");
    }

    /// c:85 — CGF_NOSORT is bit 0 (= 1) (alt name pin).
    #[test]
    fn cgf_nosort_is_bit_zero_alt() {
        assert_eq!(CGF_NOSORT, 1, "c:85 — NOSORT is bit 0");
    }

    /// c:127-140 — all CMF_* are i32 (compile-time type pin).
    #[test]
    fn cmf_flags_all_i32_type() {
        let _: i32 = CMF_FILE;
        let _: i32 = CMF_ALL;
    }

    /// c:127 — CMF_FILE is bit 0 (alt name pin).
    #[test]
    fn cmf_file_is_bit_zero_alt() {
        assert_eq!(CMF_FILE, 1i32 << 0, "c:127 — FILE is bit 0");
    }

    /// c:127-140 — CMF_FILE through CMF_ALL form contiguous bits 0..=13.
    #[test]
    fn cmf_flags_dense_bits_zero_through_13() {
        let all = [
            CMF_FILE,
            CMF_REMOVE,
            CMF_ISPAR,
            CMF_PARBR,
            CMF_PARNEST,
            CMF_NOLIST,
            CMF_DISPLINE,
            CMF_HIDE,
            CMF_NOSPACE,
            CMF_PACKED,
            CMF_ROWS,
            CMF_MULT,
            CMF_FMULT,
            CMF_ALL,
        ];
        let or_all: i32 = all.iter().fold(0, |acc, &v| acc | v);
        let expected = (1i32 << 14) - 1;
        assert_eq!(or_all, expected, "CMF_* must cover bits 0..=13 (no gaps)");
    }

    /// c:127-140 — CMF_* pairwise distinct.
    #[test]
    fn cmf_flags_pairwise_distinct() {
        let codes = [
            CMF_FILE,
            CMF_REMOVE,
            CMF_ISPAR,
            CMF_PARBR,
            CMF_PARNEST,
            CMF_NOLIST,
            CMF_DISPLINE,
            CMF_HIDE,
            CMF_NOSPACE,
            CMF_PACKED,
            CMF_ROWS,
            CMF_MULT,
            CMF_FMULT,
            CMF_ALL,
        ];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len(), "CMF_* must be pairwise distinct");
    }

    /// c:127-140 — all CMF_* powers of 2.
    #[test]
    fn cmf_flags_all_powers_of_two() {
        for &v in &[
            CMF_FILE,
            CMF_REMOVE,
            CMF_ISPAR,
            CMF_PARBR,
            CMF_PARNEST,
            CMF_NOLIST,
            CMF_DISPLINE,
            CMF_HIDE,
            CMF_NOSPACE,
            CMF_PACKED,
            CMF_ROWS,
            CMF_MULT,
            CMF_FMULT,
            CMF_ALL,
        ] {
            assert!(
                (v as u32).is_power_of_two(),
                "CMF_* {} must be a single bit",
                v
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Zle/comp.h
    // c:85-95 CGF_* / c:185-189 CPAT_* / c:259-261 CLF_* /
    // c:389-440 CPN_*/CP_* duality / c:443 CP_ALLKEYS /
    // c:447-451 hook offsets / c:474 CM_SPACE
    // ═══════════════════════════════════════════════════════════════════

    /// c:85-95 — CGF_* all powers of 2 (alt pin).
    #[test]
    fn cgf_flags_all_powers_of_two_alt() {
        for &v in &[
            CGF_NOSORT,
            CGF_LINES,
            CGF_HASDL,
            CGF_UNIQALL,
            CGF_UNIQCON,
            CGF_PACKED,
            CGF_ROWS,
            CGF_FILES,
            CGF_MATSORT,
            CGF_NUMSORT,
            CGF_REVSORT,
        ] {
            assert!(
                (v as u32).is_power_of_two(),
                "CGF_* {} must be a single bit",
                v
            );
        }
    }

    /// c:85-95 — CGF_* pairwise distinct (alt pin).
    #[test]
    fn cgf_flags_pairwise_distinct_alt() {
        let codes = [
            CGF_NOSORT,
            CGF_LINES,
            CGF_HASDL,
            CGF_UNIQALL,
            CGF_UNIQCON,
            CGF_PACKED,
            CGF_ROWS,
            CGF_FILES,
            CGF_MATSORT,
            CGF_NUMSORT,
            CGF_REVSORT,
        ];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len(), "CGF_* pairwise distinct");
    }

    /// c:85-95 — CGF_* OR covers bits 0..=10.
    #[test]
    fn cgf_or_covers_low_11_bits() {
        let or_all = CGF_NOSORT
            | CGF_LINES
            | CGF_HASDL
            | CGF_UNIQALL
            | CGF_UNIQCON
            | CGF_PACKED
            | CGF_ROWS
            | CGF_FILES
            | CGF_MATSORT
            | CGF_NUMSORT
            | CGF_REVSORT;
        assert_eq!(
            or_all,
            (1i32 << 11) - 1,
            "CGF_* must cover bits 0..=10 (no gaps)"
        );
    }

    /// c:185-189 — CPAT_* values 0..=4 (sequential).
    #[test]
    fn cpat_values_sequential_0_to_4() {
        assert_eq!(CPAT_CCLASS, 0);
        assert_eq!(CPAT_NCLASS, 1);
        assert_eq!(CPAT_EQUIV, 2);
        assert_eq!(CPAT_ANY, 3);
        assert_eq!(CPAT_CHAR, 4);
    }

    /// c:185-189 — CPAT_* pairwise distinct.
    #[test]
    fn cpat_pairwise_distinct() {
        let codes = [CPAT_CCLASS, CPAT_NCLASS, CPAT_EQUIV, CPAT_ANY, CPAT_CHAR];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len(), "CPAT_* pairwise distinct");
    }

    /// c:259-261 — CLF_MISS / CLF_DIFF / CLF_SUF are powers of 2.
    #[test]
    fn clf_flags_powers_of_two() {
        for &v in &[CLF_MISS, CLF_DIFF, CLF_SUF] {
            assert!(
                (v as u32).is_power_of_two(),
                "CLF_* {} must be a single bit",
                v
            );
        }
    }

    /// c:259-261 — CLF_MISS=1, CLF_DIFF=2, CLF_SUF=4 (verbatim values).
    #[test]
    fn clf_flags_exact_values() {
        assert_eq!(CLF_MISS, 1);
        assert_eq!(CLF_DIFF, 2);
        assert_eq!(CLF_SUF, 4);
    }

    /// c:389-440 — CP_X = 1 << CPN_X for every X (duality invariant).
    #[test]
    fn cp_equals_one_shift_cpn_duality() {
        // Sample 6 pairs across the range.
        assert_eq!(CP_EXACTSTR, 1u32 << CPN_EXACTSTR);
        assert_eq!(CP_PATMATCH, 1u32 << CPN_PATMATCH);
        assert_eq!(CP_PATINSERT, 1u32 << CPN_PATINSERT);
        assert_eq!(CP_UNAMBIG, 1u32 << CPN_UNAMBIG);
        assert_eq!(CP_IGNORED, 1u32 << CPN_IGNORED);
        assert_eq!(CP_TOEND, 1u32 << CPN_TOEND);
    }

    /// c:443 — `CP_ALLKEYS = 0x3ffffff` (covers bits 0..=25, all 26 CPN_*).
    #[test]
    fn cp_allkeys_covers_26_bits() {
        assert_eq!(CP_ALLKEYS, 0x3ffffffu32);
        assert_eq!(CP_ALLKEYS, (1u32 << 26) - 1, "26 bits set");
        assert_eq!(CP_ALLKEYS.count_ones(), 26);
    }

    /// c:447-451 — hook offsets sequential 0..=4.
    #[test]
    fn hook_offsets_sequential_0_to_4() {
        assert_eq!(INSERTMATCHHOOK_OFFSET, 0);
        assert_eq!(MENUSTARTHOOK_OFFSET, 1);
        assert_eq!(COMPCTLMAKEHOOK_OFFSET, 2);
        assert_eq!(COMPCTLCLEANUPHOOK_OFFSET, 3);
        assert_eq!(COMPLISTMATCHESHOOK_OFFSET, 4);
    }

    /// c:447-451 — hook offsets pairwise distinct (alt pin).
    #[test]
    fn hook_offsets_pairwise_distinct_alt() {
        let codes = [
            INSERTMATCHHOOK_OFFSET,
            MENUSTARTHOOK_OFFSET,
            COMPCTLMAKEHOOK_OFFSET,
            COMPCTLCLEANUPHOOK_OFFSET,
            COMPLISTMATCHESHOOK_OFFSET,
        ];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len(), "hook offsets pairwise distinct");
    }

    /// c:474 — `CM_SPACE = 2`.
    #[test]
    fn cm_space_is_two() {
        assert_eq!(CM_SPACE, 2);
    }

    /// c:172-178 — CMF_LINE/LEFT/RIGHT/INTER are 1,2,4,8 (powers of 2).
    #[test]
    fn cmf_line_class_flags_are_powers_of_two() {
        for &v in &[CMF_LINE, CMF_LEFT, CMF_RIGHT, CMF_INTER] {
            assert!(
                (v as u32).is_power_of_two(),
                "CMF_* line/edge {} must be single bit",
                v
            );
        }
    }
}
