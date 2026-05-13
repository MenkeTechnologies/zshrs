//! `compctl.h` port — completion descriptor types + `CC_*` / `CCT_*`
//! flag constants used by the legacy `compctl` builtin.
//!
//! Port of `Src/Zle/compctl.h`. Canonical home for the four typedefs
//! (`Compctlp`/`Compctl`/`Compcond`/`Patcomp`) and the two flag-bit
//! families: 30 `CC_*` primary completion-target flags (mask) and 7
//! `CC_*` secondary flags (mask2), plus 14 `CCT_*` `-x` condition
//! types.
//!
//! C source: 4 typedefs + 4 structs (`compctlp`, `patcomp`,
//! `compcond`, `compctl`), 14 `CCT_*` constants (c:76-89), 30
//! primary `CC_*` constants (c:118-149), 7 secondary `CC_*` constants
//! (c:152-158). 0 functions.
//!
//! `compctl.rs` (the .c port) re-exports these via `pub use
//! super::compctl_h::*;` so existing `cc_flags::FILES` / `cct::POS`
//! call sites keep compiling alongside the C-canonical
//! `CC_FILES` / `CCT_POS` names.

// ---------------------------------------------------------------------------
// `-x` condition type constants (c:76-89).
// ---------------------------------------------------------------------------


// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
use crate::extensions::widget::*;
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

pub const CCT_UNUSED:   i32 = 0;                                         // c:76
pub const CCT_POS:      i32 = 1;                                         // c:77
pub const CCT_CURSTR:   i32 = 2;                                         // c:78
pub const CCT_CURPAT:   i32 = 3;                                         // c:79
pub const CCT_WORDSTR:  i32 = 4;                                         // c:80
pub const CCT_WORDPAT:  i32 = 5;                                         // c:81
pub const CCT_CURSUF:   i32 = 6;                                         // c:82
pub const CCT_CURPRE:   i32 = 7;                                         // c:83
pub const CCT_CURSUB:   i32 = 8;                                         // c:84
pub const CCT_CURSUBC:  i32 = 9;                                         // c:85
pub const CCT_NUMWORDS: i32 = 10;                                        // c:86
pub const CCT_RANGESTR: i32 = 11;                                        // c:87
pub const CCT_RANGEPAT: i32 = 12;                                        // c:88
pub const CCT_QUOTE:    i32 = 13;                                        // c:89

// ---------------------------------------------------------------------------
// Primary completion-target flags (`mask`, c:118-149).
// Each bit selects one completion-source kind (files, vars, jobs, ...)
// the compctl spec expands.
// ---------------------------------------------------------------------------

pub const CC_FILES:      u64 = 1 <<  0;                                  // c:118
pub const CC_COMMPATH:   u64 = 1 <<  1;                                  // c:119
pub const CC_REMOVE:     u64 = 1 <<  2;                                  // c:120
pub const CC_OPTIONS:    u64 = 1 <<  3;                                  // c:121
pub const CC_VARS:       u64 = 1 <<  4;                                  // c:122
pub const CC_BINDINGS:   u64 = 1 <<  5;                                  // c:123
pub const CC_ARRAYS:     u64 = 1 <<  6;                                  // c:124
pub const CC_INTVARS:    u64 = 1 <<  7;                                  // c:125
pub const CC_SHFUNCS:    u64 = 1 <<  8;                                  // c:126
pub const CC_PARAMS:     u64 = 1 <<  9;                                  // c:127
pub const CC_ENVVARS:    u64 = 1 << 10;                                  // c:128
pub const CC_JOBS:       u64 = 1 << 11;                                  // c:129
pub const CC_RUNNING:    u64 = 1 << 12;                                  // c:130
pub const CC_STOPPED:    u64 = 1 << 13;                                  // c:131
pub const CC_BUILTINS:   u64 = 1 << 14;                                  // c:132
pub const CC_ALREG:      u64 = 1 << 15;                                  // c:133
pub const CC_ALGLOB:     u64 = 1 << 16;                                  // c:134
pub const CC_USERS:      u64 = 1 << 17;                                  // c:135
pub const CC_DISCMDS:    u64 = 1 << 18;                                  // c:136
pub const CC_EXCMDS:     u64 = 1 << 19;                                  // c:137
pub const CC_SCALARS:    u64 = 1 << 20;                                  // c:138
pub const CC_READONLYS:  u64 = 1 << 21;                                  // c:139
pub const CC_SPECIALS:   u64 = 1 << 22;                                  // c:140
pub const CC_DELETE:     u64 = 1 << 23;                                  // c:141
pub const CC_NAMED:      u64 = 1 << 24;                                  // c:142
pub const CC_QUOTEFLAG:  u64 = 1 << 25;                                  // c:143
pub const CC_EXTCMDS:    u64 = 1 << 26;                                  // c:144
pub const CC_RESWDS:     u64 = 1 << 27;                                  // c:145
pub const CC_DIRS:       u64 = 1 << 28;                                  // c:146
pub const CC_EXPANDEXPL: u64 = 1 << 30;                                  // c:148
pub const CC_RESERVED:   u64 = 1 << 31;                                  // c:149

// ---------------------------------------------------------------------------
// Secondary completion-target flags (`mask2`, c:152-158).
// ---------------------------------------------------------------------------

pub const CC_NOSORT:  u64 = 1 << 0;                                      // c:152
pub const CC_XORCONT: u64 = 1 << 1;                                      // c:153
pub const CC_CCCONT:  u64 = 1 << 2;                                      // c:154
pub const CC_PATCONT: u64 = 1 << 3;                                      // c:155
pub const CC_DEFCONT: u64 = 1 << 4;                                      // c:156
pub const CC_UNIQCON: u64 = 1 << 5;                                      // c:157
pub const CC_UNIQALL: u64 = 1 << 6;                                      // c:158

// ---------------------------------------------------------------------------
// Typedef structs (c:32-115).
//
// C uses linked lists threaded through `next` pointers. The Rust
// port substitutes `Option<Box<...>>` for the same self-referential
// chain; the linked-list semantics are preserved.
// ---------------------------------------------------------------------------

/// Port of `struct compctlp` from `Src/Zle/compctl.h:39-42`. Hash
/// table node entry holding a pointer to the compctl descriptor.
///
/// C definition (c:39-42):
/// ```c
/// struct compctlp {
///     struct hashnode node;
///     Compctl cc;
/// };
/// ```
///
/// The Rust port omits the `hashnode` head (zshrs's hashtable
/// machinery threads name+next through a separate scaffold) and
/// keeps the semantic payload — a pointer to the compctl descriptor.
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct Compctlp {                                                    // c:39
    pub cc: std::sync::Arc<Compctl>,                                                // c:41
}

/// Port of `struct patcomp` from `Src/Zle/compctl.h:46-50`. Linked-
/// list node for the pattern-compctl registry (entries created by
/// `compctl -p PATTERN ...`).
///
/// C definition (c:46-50):
/// ```c
/// struct patcomp {
///     Patcomp next;
///     char *pat;
///     Compctl cc;
/// };
/// ```
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct Patcomp {                                                     // c:46
    pub next: Option<Box<Patcomp>>,                                      // c:47
    pub pat: String,                                                     // c:48
    pub cc: std::sync::Arc<Compctl>,                                                // c:49
}

/// Port of `struct compcond` from `Src/Zle/compctl.h:54-74`. The
/// per-condition descriptor for `compctl -x`.
///
/// C definition (c:54-74):
/// ```c
/// struct compcond {
///     Compcond and, or;
///     int type;            /* one of CCT_* */
///     int n;               /* array length */
///     union {
///         struct { int *a, *b; } r;       /* CCT_POS, CCT_NUMWORDS */
///         struct { int *p; char **s; } s;  /* CCT_CURSTR, CCT_CURPAT, ... */
///         struct { char **a, **b; } l;     /* CCT_RANGESTR, ... */
///     } u;
/// };
/// ```
///
/// The Rust port collapses C's `union` into an explicit enum
/// (`CompcondData`) since Rust unions require unsafe; the dispatch
/// is by `typ` per the C convention.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Compcond {                                                    // c:54
    pub and: Option<Box<Compcond>>,                                      // c:55
    pub or:  Option<Box<Compcond>>,                                      // c:55
    pub typ: i32,                                                        // c:56  (Rust keyword `type`)
    pub n:   i32,                                                        // c:57
    pub u:   CompcondData,                                               // c:58 union
}

/// Port of the anonymous `union { struct r,s,l }` inside `compcond`
/// at `Src/Zle/compctl.h:58-73`. The C union is dispatched by
/// `typ` (one of the `CCT_*` constants).
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub enum CompcondData {                                                  // c:58
    /// Port of `struct { int *a, *b; } r` (c:59-62) — used by
    /// `CCT_POS`, `CCT_NUMWORDS`.
    R { a: Vec<i32>, b: Vec<i32> },
    /// Port of `struct { int *p; char **s; } s` (c:63-66) — used by
    /// `CCT_CURSTR`, `CCT_CURPAT`, `CCT_CURSUF`, `CCT_CURPRE`,
    /// `CCT_CURSUB`, `CCT_CURSUBC`, `CCT_WORDSTR`, `CCT_WORDPAT`,
    /// `CCT_QUOTE`.
    S { p: Vec<i32>, s: Vec<String> },
    /// Port of `struct { char **a, **b; } l` (c:68-71) — used by
    /// `CCT_RANGESTR`, `CCT_RANGEPAT`.
    L { a: Vec<String>, b: Vec<String> },
    /// Empty (CCT_UNUSED).
    #[default]
    Unused,
}

/// Port of `struct compctl` from `Src/Zle/compctl.h:93-115`. The
/// real per-command compctl descriptor — what `compctl name args`
/// allocates and registers in the `compctltab` hashtable.
///
/// C definition (c:93-115) — 22 fields. Field names + types
/// preserved verbatim; pointer types collapse to `Option<String>` /
/// `Option<std::sync::Arc<Compctl>>` etc. as appropriate.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct Compctl {                                                     // c:93
    /// Reference count.
    pub refc: i32,                                                       // c:94
    /// Next compctl in a `-x` chain.
    pub next: Option<std::sync::Arc<Compctl>>,                                      // c:95
    /// Mask of completion-target flags (`CC_*`).
    pub mask: u64,                                                       // c:96
    /// Secondary mask of completion-target flags (`CC_*`, mask2).
    pub mask2: u64,                                                      // c:96
    /// `-k` variable name.
    pub keyvar: Option<String>,                                          // c:97
    /// `-g` glob pattern.
    pub glob: Option<String>,                                            // c:98
    /// `-s` expansion string.
    pub str: Option<String>,                                            // c:99 (Rust keyword `str`)
    /// `-K` function name.
    pub func: Option<String>,                                            // c:100
    /// `-X` explanation.
    pub explain: Option<String>,                                         // c:101
    /// `-y` user-defined description for listing.
    pub ylist: Option<String>,                                           // c:102
    /// `-P` prefix.
    pub prefix: Option<String>,                                          // c:103
    /// `-S` suffix.
    pub suffix: Option<String>,                                          // c:103
    /// `-l` command name to use.
    pub subcmd: Option<String>,                                          // c:104
    /// `-1` command name to use.
    pub substr: Option<String>,                                          // c:105
    /// `-w` with-directory.
    pub withd: Option<String>,                                           // c:106
    /// `-H` history pattern.
    pub hpat: Option<String>,                                            // c:107
    /// `-H` number of events to search.
    pub hnum: i32,                                                       // c:108
    /// `-J`/`-V` group name.
    pub gname: Option<String>,                                           // c:109
    /// `-x` first compctl in the chain.
    pub ext: Option<std::sync::Arc<Compctl>>,                                       // c:110
    /// `-x` condition for this compctl.
    pub cond: Option<Box<Compcond>>,                                     // c:111
    /// `+` xor'ed compctl chain.
    pub xor: Option<std::sync::Arc<Compctl>>,                                       // c:112
    /// `-M` matcher control — head of the Cmatcher chain compiled
    /// from this compctl's match-spec arg.
    pub matcher: Option<Box<crate::ported::zle::comp_h::Cmatcher>>,      // c:113
    /// `-M` matcher string.
    pub mstr: Option<String>,                                            // c:114
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies CCT_* values per c:76-89.
    #[test]
    fn cct_constants_correct() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(CCT_UNUSED, 0);
        assert_eq!(CCT_POS, 1);
        assert_eq!(CCT_CURSTR, 2);
        assert_eq!(CCT_QUOTE, 13);
    }

    /// Verifies CC_* primary mask values per c:118-149 — single-bit,
    /// non-overlapping.
    #[test]
    fn cc_primary_mask_bits_distinct() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let all = CC_FILES | CC_COMMPATH | CC_REMOVE | CC_OPTIONS
                | CC_VARS | CC_BINDINGS | CC_ARRAYS | CC_INTVARS
                | CC_SHFUNCS | CC_PARAMS | CC_ENVVARS | CC_JOBS
                | CC_RUNNING | CC_STOPPED | CC_BUILTINS | CC_ALREG
                | CC_ALGLOB | CC_USERS | CC_DISCMDS | CC_EXCMDS
                | CC_SCALARS | CC_READONLYS | CC_SPECIALS | CC_DELETE
                | CC_NAMED | CC_QUOTEFLAG | CC_EXTCMDS | CC_RESWDS
                | CC_DIRS | CC_EXPANDEXPL | CC_RESERVED;
        assert_eq!(all.count_ones(), 31);  // 30 sequential + 30 + 31 (skips bit 29)
    }

    /// Verifies the secondary mask values per c:152-158.
    #[test]
    fn cc_secondary_mask_values() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(CC_NOSORT, 1);
        assert_eq!(CC_XORCONT, 2);
        assert_eq!(CC_UNIQALL, 1 << 6);
    }

    /// Verifies Compctl Default initialiser zeroes every field per
    /// the C convention of `(Compctl) calloc(1, sizeof(...))`.
    #[test]
    fn compctl_default_zeros_fields() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let cc = Compctl::default();
        assert_eq!(cc.refc, 0);
        assert!(cc.next.is_none());
        assert_eq!(cc.mask, 0);
        assert_eq!(cc.mask2, 0);
        assert!(cc.keyvar.is_none());
        assert!(cc.cond.is_none());
        assert!(cc.xor.is_none());
        assert_eq!(cc.hnum, 0);
    }

    /// Verifies Compcond Default starts in CCT_UNUSED state.
    #[test]
    fn compcond_default_is_unused() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let c = Compcond::default();
        assert_eq!(c.typ, CCT_UNUSED);
        assert!(matches!(c.u, CompcondData::Unused));
    }

    /// Verifies the CompcondData variants align with the C union
    /// dispatch per c:58-73.
    #[test]
    fn compcond_data_variants() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let r = CompcondData::R { a: vec![0, 1], b: vec![2, 3] };
        if let CompcondData::R { a, b } = r {
            assert_eq!(a, vec![0, 1]);
            assert_eq!(b, vec![2, 3]);
        } else {
            panic!("expected R variant");
        }
        let s = CompcondData::S { p: vec![1], s: vec!["x".into()] };
        assert!(matches!(s, CompcondData::S { .. }));
        let l = CompcondData::L { a: vec!["lo".into()], b: vec!["hi".into()] };
        assert!(matches!(l, CompcondData::L { .. }));
    }
}
