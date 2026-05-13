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


// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_move::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_word::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_params::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_vi::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_utils::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_refresh::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::textobjects::*;
#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;

pub const CGF_NOSORT:  i32 = 1;                                          // c:85
pub const CGF_LINES:   i32 = 2;                                          // c:86
pub const CGF_HASDL:   i32 = 4;                                          // c:87
pub const CGF_UNIQALL: i32 = 8;                                          // c:88
pub const CGF_UNIQCON: i32 = 16;                                         // c:89
pub const CGF_PACKED:  i32 = 32;                                         // c:90
pub const CGF_ROWS:    i32 = 64;                                         // c:91
pub const CGF_FILES:   i32 = 128;                                        // c:92
pub const CGF_MATSORT: i32 = 256;                                        // c:93
pub const CGF_NUMSORT: i32 = 512;                                        // c:94
pub const CGF_REVSORT: i32 = 1024;                                       // c:95

// ---------------------------------------------------------------------------
// Match-flag constants (c:127-143) — flags on `cmatch.flags`.
// ---------------------------------------------------------------------------

pub const CMF_FILE:     i32 = 1 <<  0;                                   // c:127
pub const CMF_REMOVE:   i32 = 1 <<  1;                                   // c:128
pub const CMF_ISPAR:    i32 = 1 <<  2;                                   // c:129
pub const CMF_PARBR:    i32 = 1 <<  3;                                   // c:130
pub const CMF_PARNEST:  i32 = 1 <<  4;                                   // c:131
pub const CMF_NOLIST:   i32 = 1 <<  5;                                   // c:132
pub const CMF_DISPLINE: i32 = 1 <<  6;                                   // c:133
pub const CMF_HIDE:     i32 = 1 <<  7;                                   // c:134
pub const CMF_NOSPACE:  i32 = 1 <<  8;                                   // c:135
pub const CMF_PACKED:   i32 = 1 <<  9;                                   // c:136
pub const CMF_ROWS:     i32 = 1 << 10;                                   // c:137
pub const CMF_MULT:     i32 = 1 << 11;                                   // c:138
pub const CMF_FMULT:    i32 = 1 << 12;                                   // c:139
pub const CMF_ALL:      i32 = 1 << 13;                                   // c:140
pub const CMF_DUMMY:    i32 = 1 << 14;                                   // c:141
pub const CMF_MORDER:   i32 = 1 << 15;                                   // c:142
pub const CMF_DELETE:   i32 = 1 << 16;                                   // c:143

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

pub const CMF_LINE:  i32 = 1;                                            // c:172
pub const CMF_LEFT:  i32 = 2;                                            // c:174
pub const CMF_RIGHT: i32 = 4;                                            // c:176
pub const CMF_INTER: i32 = 8;                                            // c:178

// ---------------------------------------------------------------------------
// Cpattern type discriminators (c:184-190).
// ---------------------------------------------------------------------------

/// Port of `enum { CPAT_CCLASS, ... }` from `Src/Zle/comp.h:184-190`.
/// C uses an anonymous int-constant enum — Rust ports as `pub const`s
/// to avoid a Rust-only enum type. The discriminator drives
/// `freecpattern()` and `cpattern.tp` dispatch (see `Cpattern` below).
pub const CPAT_CCLASS: i32 = 0;                                          // c:185
pub const CPAT_NCLASS: i32 = 1;                                          // c:186
pub const CPAT_EQUIV:  i32 = 2;                                          // c:187
pub const CPAT_ANY:    i32 = 3;                                          // c:188
pub const CPAT_CHAR:   i32 = 4;                                          // c:189

// ---------------------------------------------------------------------------
// Cline flag constants (c:259-267) — flags on `cline.flags`.
// ---------------------------------------------------------------------------

pub const CLF_MISS:    i32 = 1;                                          // c:259
pub const CLF_DIFF:    i32 = 2;                                          // c:260
pub const CLF_SUF:     i32 = 4;                                          // c:261
pub const CLF_MID:     i32 = 8;                                          // c:262
pub const CLF_NEW:     i32 = 16;                                         // c:263
pub const CLF_LINE:    i32 = 32;                                         // c:264
pub const CLF_JOIN:    i32 = 64;                                         // c:265
pub const CLF_MATCHED: i32 = 128;                                        // c:266
pub const CLF_SKIP:    i32 = 256;                                        // c:267

// ---------------------------------------------------------------------------
// compadd / addmatches() flag constants (c:299-309).
// ---------------------------------------------------------------------------

pub const CAF_QUOTE:   i32 = 1;                                          // c:299
pub const CAF_NOSORT:  i32 = 2;                                          // c:300
pub const CAF_MATCH:   i32 = 4;                                          // c:301
pub const CAF_UNIQCON: i32 = 8;                                          // c:302
pub const CAF_UNIQALL: i32 = 16;                                         // c:303
pub const CAF_ARRAYS:  i32 = 32;                                         // c:304
pub const CAF_KEYS:    i32 = 64;                                         // c:305
pub const CAF_ALL:     i32 = 128;                                        // c:306
pub const CAF_MATSORT: i32 = 256;                                        // c:307
pub const CAF_NUMSORT: i32 = 512;                                        // c:308
pub const CAF_REVSORT: i32 = 1024;                                       // c:309

// ---------------------------------------------------------------------------
// Fromcomp flags (c:359-360).
// ---------------------------------------------------------------------------

pub const FC_LINE:   i32 = 1;                                            // c:359
pub const FC_INWORD: i32 = 2;                                            // c:360

// ---------------------------------------------------------------------------
// Special-parameter index constants — `comprpms` / "real params"
// (c:364-386). For each parameter there's a `CPN_*` index and a
// `CP_*` bitmask `(1 << CPN_*)`.
// ---------------------------------------------------------------------------

pub const CPN_WORDS:     i32 = 0;                                        // c:364
pub const CP_WORDS:      u32 = 1 << CPN_WORDS;                           // c:365
pub const CPN_REDIRS:    i32 = 1;                                        // c:366
pub const CP_REDIRS:     u32 = 1 << CPN_REDIRS;                          // c:367
pub const CPN_CURRENT:   i32 = 2;                                        // c:368
pub const CP_CURRENT:    u32 = 1 << CPN_CURRENT;                         // c:369
pub const CPN_PREFIX:    i32 = 3;                                        // c:370
pub const CP_PREFIX:     u32 = 1 << CPN_PREFIX;                          // c:371
pub const CPN_SUFFIX:    i32 = 4;                                        // c:372
pub const CP_SUFFIX:     u32 = 1 << CPN_SUFFIX;                          // c:373
pub const CPN_IPREFIX:   i32 = 5;                                        // c:374
pub const CP_IPREFIX:    u32 = 1 << CPN_IPREFIX;                         // c:375
pub const CPN_ISUFFIX:   i32 = 6;                                        // c:376
pub const CP_ISUFFIX:    u32 = 1 << CPN_ISUFFIX;                         // c:377
pub const CPN_QIPREFIX:  i32 = 7;                                        // c:378
pub const CP_QIPREFIX:   u32 = 1 << CPN_QIPREFIX;                        // c:379
pub const CPN_QISUFFIX:  i32 = 8;                                        // c:380
pub const CP_QISUFFIX:   u32 = 1 << CPN_QISUFFIX;                        // c:381
pub const CPN_COMPSTATE: i32 = 9;                                        // c:382
pub const CP_COMPSTATE:  u32 = 1 << CPN_COMPSTATE;                       // c:383

/// Port of `#define CP_REALPARAMS` from `Src/Zle/comp.h:385`. Total
/// number of "real" comp parameters.
pub const CP_REALPARAMS: i32 = 10;                                       // c:385

/// Port of `#define CP_ALLREALS` from `Src/Zle/comp.h:386`. Mask
/// covering every CP_* "real" param flag (bits 0..9 set).
pub const CP_ALLREALS: u32 = 0x3ff;                                      // c:386

// ---------------------------------------------------------------------------
// Special-parameter index constants — `compkpms` / "key params"
// (c:389-442).
// ---------------------------------------------------------------------------

pub const CPN_NMATCHES:   i32 = 0;                                       // c:389
pub const CP_NMATCHES:    u32 = 1 << CPN_NMATCHES;                       // c:390
pub const CPN_CONTEXT:    i32 = 1;                                       // c:391
pub const CP_CONTEXT:     u32 = 1 << CPN_CONTEXT;                        // c:392
pub const CPN_PARAMETER:  i32 = 2;                                       // c:393
pub const CP_PARAMETER:   u32 = 1 << CPN_PARAMETER;                      // c:394
pub const CPN_REDIRECT:   i32 = 3;                                       // c:395
pub const CP_REDIRECT:    u32 = 1 << CPN_REDIRECT;                       // c:396
pub const CPN_QUOTE:      i32 = 4;                                       // c:397
pub const CP_QUOTE:       u32 = 1 << CPN_QUOTE;                          // c:398
pub const CPN_QUOTING:    i32 = 5;                                       // c:399
pub const CP_QUOTING:     u32 = 1 << CPN_QUOTING;                        // c:400
pub const CPN_RESTORE:    i32 = 6;                                       // c:401
pub const CP_RESTORE:     u32 = 1 << CPN_RESTORE;                        // c:402
pub const CPN_LIST:       i32 = 7;                                       // c:403
pub const CP_LIST:        u32 = 1 << CPN_LIST;                           // c:404
pub const CPN_INSERT:     i32 = 8;                                       // c:405
pub const CP_INSERT:      u32 = 1 << CPN_INSERT;                         // c:406
pub const CPN_EXACT:      i32 = 9;                                       // c:407
pub const CP_EXACT:       u32 = 1 << CPN_EXACT;                          // c:408
pub const CPN_EXACTSTR:   i32 = 10;                                      // c:409
pub const CP_EXACTSTR:    u32 = 1 << CPN_EXACTSTR;                       // c:410
pub const CPN_PATMATCH:   i32 = 11;                                      // c:411
pub const CP_PATMATCH:    u32 = 1 << CPN_PATMATCH;                       // c:412
pub const CPN_PATINSERT:  i32 = 12;                                      // c:413
pub const CP_PATINSERT:   u32 = 1 << CPN_PATINSERT;                      // c:414
pub const CPN_UNAMBIG:    i32 = 13;                                      // c:415
pub const CP_UNAMBIG:     u32 = 1 << CPN_UNAMBIG;                        // c:416
pub const CPN_UNAMBIGC:   i32 = 14;                                      // c:417
pub const CP_UNAMBIGC:    u32 = 1 << CPN_UNAMBIGC;                       // c:418
pub const CPN_UNAMBIGP:   i32 = 15;                                      // c:419
pub const CP_UNAMBIGP:    u32 = 1 << CPN_UNAMBIGP;                       // c:420
pub const CPN_INSERTP:    i32 = 16;                                      // c:421
pub const CP_INSERTP:     u32 = 1 << CPN_INSERTP;                        // c:422
pub const CPN_LISTMAX:    i32 = 17;                                      // c:423
pub const CP_LISTMAX:     u32 = 1 << CPN_LISTMAX;                        // c:424
pub const CPN_LASTPROMPT: i32 = 18;                                      // c:425
pub const CP_LASTPROMPT:  u32 = 1 << CPN_LASTPROMPT;                     // c:426
pub const CPN_TOEND:      i32 = 19;                                      // c:427
pub const CP_TOEND:       u32 = 1 << CPN_TOEND;                          // c:428
pub const CPN_OLDLIST:    i32 = 20;                                      // c:429
pub const CP_OLDLIST:     u32 = 1 << CPN_OLDLIST;                        // c:430
pub const CPN_OLDINS:     i32 = 21;                                      // c:431
pub const CP_OLDINS:      u32 = 1 << CPN_OLDINS;                         // c:432
pub const CPN_VARED:      i32 = 22;                                      // c:433
pub const CP_VARED:       u32 = 1 << CPN_VARED;                          // c:434
pub const CPN_LISTLINES:  i32 = 23;                                      // c:435
pub const CP_LISTLINES:   u32 = 1 << CPN_LISTLINES;                      // c:436
pub const CPN_QUOTES:     i32 = 24;                                      // c:437
pub const CP_QUOTES:      u32 = 1 << CPN_QUOTES;                         // c:438
pub const CPN_IGNORED:    i32 = 25;                                      // c:439
pub const CP_IGNORED:     u32 = 1 << CPN_IGNORED;                        // c:440

/// Port of `#define CP_KEYPARAMS` from `Src/Zle/comp.h:442`. Total
/// number of "key" comp parameters.
pub const CP_KEYPARAMS: i32 = 26;                                        // c:442

/// Port of `#define CP_ALLKEYS` from `Src/Zle/comp.h:443`. Mask
/// covering every CP_* "key" param flag (bits 0..25 set).
pub const CP_ALLKEYS: u32 = 0x3ffffff;                                   // c:443

// ---------------------------------------------------------------------------
// Hook indexes (c:447-451).
// ---------------------------------------------------------------------------

pub const INSERTMATCHHOOK_OFFSET:     usize = 0;                         // c:447
pub const MENUSTARTHOOK_OFFSET:       usize = 1;                         // c:448
pub const COMPCTLMAKEHOOK_OFFSET:     usize = 2;                         // c:449
pub const COMPCTLCLEANUPHOOK_OFFSET:  usize = 3;                         // c:450
pub const COMPLISTMATCHESHOOK_OFFSET: usize = 4;                         // c:451

// ---------------------------------------------------------------------------
// Misc constants.
// ---------------------------------------------------------------------------

/// Port of `#define CM_SPACE` from `Src/Zle/comp.h:474`. Number of
/// columns to leave empty between rows of matches.
pub const CM_SPACE: i32 = 2;                                             // c:474

// ---------------------------------------------------------------------------
// Typedef structs (c:30-470). C uses linked lists threaded through
// `next`/`prev` pointers; Rust ports as `Option<Box<T>>` for the
// owning side.
// ---------------------------------------------------------------------------

/// Port of `struct cexpl` from `Src/Zle/comp.h:40-45`. Explanation
/// string entry attached to a match group.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cexpl {                                                       // c:40
    /// Display even without matches.
    pub always: i32,                                                     // c:41
    /// The string itself.
    pub str: Option<String>,                                            // c:42 (Rust keyword `str`)
    /// Number of matches.
    pub count: i32,                                                      // c:43
    /// Number of matches with fignore ignored.
    pub fcount: i32,                                                     // c:44
}

/// Port of `struct cmgroup` from `Src/Zle/comp.h:49-82`. A group of
/// completion matches (one per `compadd -J GROUP`).
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cmgroup {                                                     // c:49
    /// Group name.
    pub name: Option<String>,                                            // c:50
    /// Previous group in the list.
    pub prev: Option<Box<Cmgroup>>,                                      // c:51
    /// Next group in the list.
    pub next: Option<Box<Cmgroup>>,                                      // c:52
    /// CGF_* flags.
    pub flags: i32,                                                      // c:53
    /// Number of matches.
    pub mcount: i32,                                                     // c:54
    /// The matches.
    pub matches: Vec<Cmatch>,                                            // c:55
    /// Number of things to list here.
    pub lcount: i32,                                                     // c:56
    /// Number of line-displays.
    pub llcount: i32,                                                    // c:57
    /// Things to list.
    pub ylist: Vec<String>,                                              // c:58
    /// Number of explanation strings.
    pub ecount: i32,                                                     // c:59
    /// Explanation strings.
    pub expls: Vec<Cexpl>,                                               // c:60
    /// Number of compctls used.
    pub ccount: i32,                                                     // c:61
    /// LinkList of explanations (mid-build accumulator before `expls`).
    pub lexpls: Vec<Cexpl>,                                              // c:62
    /// LinkList of matches (mid-build accumulator before `matches`).
    pub lmatches: Vec<Cmatch>,                                           // c:63
    /// LinkList of matches with fignore-removed entries kept.
    pub lfmatches: Vec<Cmatch>,                                          // c:64
    /// LinkList of compctls used (mid-build accumulator).
    pub lallccs: Vec<String>,                                            // c:65
    /// Group number.
    pub num: i32,                                                        // c:66
    /// Number of opened braces.
    pub nbrbeg: i32,                                                     // c:67
    /// Number of closed braces.
    pub nbrend: i32,                                                     // c:68
    /// New matches since last permalloc().
    pub new_: i32,                                                       // c:69 (Rust keyword `new`)
    // c:71-77 — listing accumulators.
    /// Number of matches to list in columns.
    pub dcount: i32,                                                     // c:71
    /// Number of columns.
    pub cols: i32,                                                       // c:72
    /// Number of lines.
    pub lins: i32,                                                       // c:73
    /// Column width.
    pub width: i32,                                                      // c:74
    /// Per-column widths for listpacked.
    pub widths: Vec<i32>,                                                // c:75
    /// Total length.
    pub totl: i32,                                                       // c:76
    /// Length of shortest match.
    pub shortest: i32,                                                   // c:77
    /// Permanent-alloc version of this group (the C source's
    /// shadow-copy used to survive heap resets).
    pub perm: Option<Box<Cmgroup>>,                                      // c:78
}

/// Port of `struct cmatch` from `Src/Zle/comp.h:99-125`. A single
/// completion match.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cmatch {                                                      // c:99
    /// The match itself.
    pub str: Option<String>,                                            // c:100 (Rust keyword)
    /// The match string unquoted.
    pub orig: Option<String>,                                            // c:101
    /// Ignored prefix, has to be re-inserted.
    pub ipre: Option<String>,                                            // c:102
    /// Ignored prefix, unquoted.
    pub ripre: Option<String>,                                           // c:103
    /// Ignored suffix.
    pub isuf: Option<String>,                                            // c:104
    /// The path prefix.
    pub ppre: Option<String>,                                            // c:105
    /// The path suffix.
    pub psuf: Option<String>,                                            // c:106
    /// Path prefix for opendir.
    pub prpre: Option<String>,                                           // c:107
    /// Prefix string from -P.
    pub pre: Option<String>,                                             // c:108
    /// Suffix string from -S.
    pub suf: Option<String>,                                             // c:109
    /// String to display (compadd -d).
    pub disp: Option<String>,                                            // c:110
    /// Closing quote to add automatically.
    pub autoq: Option<String>,                                           // c:111
    /// CMF_* flags (cmatch namespace).
    pub flags: i32,                                                      // c:112
    /// Places where to put the brace prefixes.
    pub brpl: Vec<i32>,                                                  // c:113
    /// ...and the suffixes.
    pub brsl: Vec<i32>,                                                  // c:114
    /// When to remove the suffix.
    pub rems: Option<String>,                                            // c:115
    /// Shell function to call for suffix-removal.
    pub remf: Option<String>,                                            // c:116
    /// Length of quote-prefix.
    pub qipl: i32,                                                       // c:117
    /// Length of quote-suffix.
    pub qisl: i32,                                                       // c:118
    /// Group-relative number.
    pub rnum: i32,                                                       // c:119
    /// Global number.
    pub gnum: i32,                                                       // c:120
    /// `mode` field of a stat.
    pub mode: u32,                                                       // c:121 (mode_t → u32)
    /// LIST_TYPE-character for mode or 0.
    pub modec: char,                                                     // c:122
    /// `mode` field of a stat, following symlink.
    pub fmode: u32,                                                      // c:123 (mode_t → u32)
    /// LIST_TYPE-character for fmode or 0.
    pub fmodec: char,                                                    // c:124
}

/// Port of `struct cmlist` from `Src/Zle/comp.h:147-151`. Linked
/// list of global matchers.
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct Cmlist {                                                      // c:147
    /// Next entry in the list.
    pub next: Option<Box<Cmlist>>,                                       // c:148
    /// The matcher definition.
    pub matcher: Box<Cmatcher>,                                          // c:149
    /// The string for it.
    pub str: String,                                                    // c:150
}

/// Port of `struct cmatcher` from `Src/Zle/comp.h:153-167`. Matcher
/// specification — what to match on the line vs in the trial word,
/// with optional left/right anchors.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cmatcher {                                                    // c:153
    /// Reference counter.
    pub refc: i32,                                                       // c:154
    /// Next matcher.
    pub next: Option<Box<Cmatcher>>,                                     // c:155
    /// CMF_LINE/CMF_LEFT/CMF_RIGHT/CMF_INTER (cmatcher namespace).
    pub flags: i32,                                                      // c:156
    /// What matches on the line.
    pub line: Option<Box<Cpattern>>,                                     // c:157
    /// Length of line pattern.
    pub llen: i32,                                                       // c:158
    /// What matches in the word.
    pub word: Option<Box<Cpattern>>,                                     // c:159
    /// Length of word pattern, or:
    /// -1: word pattern is one asterisk
    /// -2: word pattern is two asterisks
    pub wlen: i32,                                                       // c:160
    /// Left anchor.
    pub left: Option<Box<Cpattern>>,                                     // c:163
    /// Length of left anchor.
    pub lalen: i32,                                                      // c:164
    /// Right anchor.
    pub right: Option<Box<Cpattern>>,                                    // c:165
    /// Length of right anchor.
    pub ralen: i32,                                                      // c:166
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
pub struct Cpattern {                                                    // c:197
    /// Next sub-pattern.
    pub next: Option<Box<Cpattern>>,                                     // c:198
    /// Type of object — one of CPAT_*.
    pub tp: i32,                                                         // c:199
    /// If a character class (CPAT_CCLASS/CPAT_NCLASS/CPAT_EQUIV),
    /// the objects in it as a metafied byte sequence — the encoded
    /// format matches `Src/pattern.c`'s `patmatchindex` reader:
    /// `0x80 + PP_*` for POSIX classes, `0x80 + PP_RANGE` + two
    /// bytes for ranges, plain bytes for literals. Storing as
    /// `Vec<u8>` (not `String`) preserves the 0x80-0xBF marker
    /// bytes that would otherwise mangle under UTF-8 validation.
    pub str: Option<Vec<u8>>,                                            // c:201 union.u.str
    /// If a single character (CPAT_CHAR), it.
    pub chr: u32,                                                        // c:208 union.u.chr (convchar_t)
}

/// Port of `struct cline` from `Src/Zle/comp.h:245-257`. One
/// word-part in the unambiguous-line-string list. Threaded prefix /
/// suffix sub-lists via the `prefix`/`suffix` fields.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cline {                                                       // c:245
    /// Next sibling word-part.
    pub next: Option<Box<Cline>>,                                        // c:246
    /// CLF_* flags.
    pub flags: i32,                                                      // c:247
    /// Line string for this part.
    pub line: Option<String>,                                            // c:248
    /// Length of `line`.
    pub llen: i32,                                                       // c:249
    /// Word string for this part.
    pub word: Option<String>,                                            // c:250
    /// Length of `word`.
    pub wlen: i32,                                                       // c:251
    /// Original (unjoined) string.
    pub orig: Option<String>,                                            // c:252
    /// Length of `orig`.
    pub olen: i32,                                                       // c:253
    /// String length (the join-up version).
    pub slen: i32,                                                       // c:254
    /// Prefix sub-list.
    pub prefix: Option<Box<Cline>>,                                      // c:255
    /// Suffix sub-list.
    pub suffix: Option<Box<Cline>>,                                      // c:255
    /// Min length seen for this part (joining metric).
    pub min: i32,                                                        // c:256
    /// Max length seen for this part (joining metric).
    pub max: i32,                                                        // c:256
}

/// Port of `struct aminfo` from `Src/Zle/comp.h:274-280`. Holds
/// info about ambiguous completions — there's one for fignore-
/// ignored and one for normal completion.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Aminfo {                                                      // c:274
    /// The first match.
    pub firstm: Option<Box<Cmatch>>,                                     // c:275
    /// If there was an exact match.
    pub exact: i32,                                                      // c:276
    /// The exact match (if any).
    pub exactm: Option<Box<Cmatch>>,                                     // c:277
    /// Number of matches.
    pub count: i32,                                                      // c:278
    /// Unambiguous line string.
    pub line: Option<Box<Cline>>,                                        // c:279
}

/// Port of `struct menuinfo` from `Src/Zle/comp.h:284-295`.
/// Menu-completion state.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Menuinfo {                                                    // c:284
    /// Position in the group list.
    pub group: Option<Box<Cmgroup>>,                                     // c:285
    /// Match currently inserted.
    pub cur: Option<Box<Cmatch>>,                                        // c:286
    /// Begin on line.
    pub pos: i32,                                                        // c:287
    /// Length of inserted string.
    pub len: i32,                                                        // c:288
    /// End on the line.
    pub end: i32,                                                        // c:289
    /// Non-zero if the cursor was at the end.
    pub we: i32,                                                         // c:290
    /// Length of suffix inserted.
    pub insc: i32,                                                       // c:291
    /// We asked if the list should be shown.
    pub asked: i32,                                                      // c:292
    /// Prefix before a brace, if any.
    pub prebr: Option<String>,                                           // c:293
    /// Suffix after a brace.
    pub postbr: Option<String>,                                          // c:294
}

/// Port of `struct ccmakedat` from `Src/Zle/comp.h:455-459`. Hook
/// data passed to the compctl-make path.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Ccmakedat {                                                   // c:455
    /// String passed to the hook.
    pub str: Option<String>,                                            // c:456
    /// Whether we're in a command position.
    pub incmd: i32,                                                      // c:457
    /// List flag.
    pub lst: i32,                                                        // c:458
}

/// Port of `struct chdata` from `Src/Zle/comp.h:465-470`. Data
/// given to `offered` hooks.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Chdata {                                                      // c:465
    /// The matches generated.
    pub matches: Option<Box<Cmgroup>>,                                   // c:466
    /// Number of matches.
    pub num: i32,                                                        // c:467
    /// Number of messages.
    pub nmesg: i32,                                                      // c:468
    /// Current match or None.
    pub cur: Option<Box<Cmatch>>,                                        // c:469
}

/// Port of `struct cadata` from `Src/Zle/comp.h:315-337`. Data
/// passed to compadd / addmatches().
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cadata {                                                      // c:315
    /// Ignored prefix (-i).
    pub ipre: Option<String>,                                            // c:316
    /// Ignored suffix (-I).
    pub isuf: Option<String>,                                            // c:317
    /// `path` prefix (-p).
    pub ppre: Option<String>,                                            // c:318
    /// `path` suffix (-s).
    pub psuf: Option<String>,                                            // c:319
    /// Expanded `path` prefix (-W).
    pub prpre: Option<String>,                                           // c:320
    /// Prefix to insert (-P).
    pub pre: Option<String>,                                             // c:321
    /// Suffix to insert (-S).
    pub suf: Option<String>,                                             // c:322
    /// Name of the group (`-[JV]`).
    pub group: Option<String>,                                           // c:323
    /// Remove suffix on chars... (-r).
    pub rems: Option<String>,                                            // c:324
    /// Function to remove suffix (-R).
    pub remf: Option<String>,                                            // c:325
    /// Ignored suffixes (-F).
    pub ign: Option<String>,                                             // c:326
    /// CMF_* flags (`-[fqn]`).
    pub flags: i32,                                                      // c:327
    /// CAF_* flags (`-[QUa]`).
    pub aflags: i32,                                                     // c:328
    /// Match spec (parsed from -M).
    pub match_: Option<Box<Cmatcher>>,                                   // c:329 (Rust keyword)
    /// Explanation (-X).
    pub exp: Option<String>,                                             // c:330
    /// Array to store matches in (-A).
    pub apar: Option<String>,                                            // c:331
    /// Array to store originals in (-O).
    pub opar: Option<String>,                                            // c:332
    /// Arrays to delete non-matches in (-D).
    pub dpar: Vec<String>,                                               // c:333
    /// Array with display lists (-d).
    pub disp: Option<String>,                                            // c:334
    /// Message to show unconditionally (-x).
    pub mesg: Option<String>,                                            // c:335
    /// Add that many dummy matches.
    pub dummies: i32,                                                    // c:336
}

/// Port of `struct cldata` from `Src/Zle/comp.h:343-353`. List data
/// for the matches-listing path.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Cldata {                                                      // c:343
    /// Screen width.
    pub zterm_columns: i32,                                              // c:344
    /// Screen height.
    pub zterm_lines: i32,                                                // c:345
    /// Value of global menuacc.
    pub menuacc: i32,                                                    // c:346
    /// No need to calculate anew.
    pub valid: i32,                                                      // c:347
    /// Number of matches to list.
    pub nlist: i32,                                                      // c:348
    /// Number of lines needed.
    pub nlines: i32,                                                     // c:349
    /// != 0 if there are hidden matches.
    pub hidden: i32,                                                     // c:350
    /// != 0 if only explanations to print.
    pub onlyexpl: i32,                                                   // c:351
    /// != 0 if hidden matches should be shown.
    pub showall: i32,                                                    // c:352
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies CGF_* group flag values per c:85-95.
    #[test]
    fn cgf_flags_correct() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(CGF_NOSORT, 1);
        assert_eq!(CGF_LINES, 2);
        assert_eq!(CGF_HASDL, 4);
        assert_eq!(CGF_REVSORT, 1024);
    }

    /// Verifies CMF_* match flags are non-overlapping single-bits
    /// per c:127-143.
    #[test]
    fn cmf_match_flags_distinct() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let all = CMF_FILE | CMF_REMOVE | CMF_ISPAR | CMF_PARBR
                | CMF_PARNEST | CMF_NOLIST | CMF_DISPLINE | CMF_HIDE
                | CMF_NOSPACE | CMF_PACKED | CMF_ROWS | CMF_MULT
                | CMF_FMULT | CMF_ALL | CMF_DUMMY | CMF_MORDER
                | CMF_DELETE;
        assert_eq!(all.count_ones(), 17);
    }

    /// Verifies CMF_LINE/LEFT/RIGHT/INTER cmatcher flags per c:172-178.
    #[test]
    fn cmf_matcher_flags_correct() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(CMF_LINE, 1);
        assert_eq!(CMF_LEFT, 2);
        assert_eq!(CMF_RIGHT, 4);
        assert_eq!(CMF_INTER, 8);
    }

    /// Verifies CPAT_* enum values per c:184-190.
    #[test]
    fn cpat_enum_values_correct() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(CPAT_CCLASS, 0);
        assert_eq!(CPAT_NCLASS, 1);
        assert_eq!(CPAT_EQUIV, 2);
        assert_eq!(CPAT_ANY, 3);
        assert_eq!(CPAT_CHAR, 4);
    }

    /// Verifies CP_REALPARAMS / CP_ALLREALS aggregate per c:385-386.
    #[test]
    fn cp_realparams_mask_covers_10_bits() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(CP_REALPARAMS, 10);
        assert_eq!(CP_ALLREALS, 0x3ff);
        assert_eq!(CP_ALLREALS.count_ones(), 10);
        assert_eq!(CP_WORDS | CP_REDIRS | CP_CURRENT | CP_PREFIX
                  | CP_SUFFIX | CP_IPREFIX | CP_ISUFFIX
                  | CP_QIPREFIX | CP_QISUFFIX | CP_COMPSTATE,
                  CP_ALLREALS);
    }

    /// Verifies CP_KEYPARAMS / CP_ALLKEYS aggregate per c:442-443.
    #[test]
    fn cp_keyparams_mask_covers_26_bits() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(CP_KEYPARAMS, 26);
        assert_eq!(CP_ALLKEYS, 0x3ffffff);
        assert_eq!(CP_ALLKEYS.count_ones(), 26);
    }

    /// Verifies CAF_* compadd flags per c:299-309.
    #[test]
    fn caf_flags_correct() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(CAF_QUOTE, 1);
        assert_eq!(CAF_NOSORT, 2);
        assert_eq!(CAF_REVSORT, 1024);
    }

    /// Verifies hook offset constants per c:447-451.
    #[test]
    fn hook_offsets_sequential() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(INSERTMATCHHOOK_OFFSET, 0);
        assert_eq!(MENUSTARTHOOK_OFFSET, 1);
        assert_eq!(COMPCTLMAKEHOOK_OFFSET, 2);
        assert_eq!(COMPCTLCLEANUPHOOK_OFFSET, 3);
        assert_eq!(COMPLISTMATCHESHOOK_OFFSET, 4);
    }

    /// Verifies CM_SPACE per c:474.
    #[test]
    fn cm_space_is_2() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(CM_SPACE, 2);
    }

    /// Verifies the structs construct cleanly with `Default`.
    #[test]
    fn structs_default_construct() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
}
