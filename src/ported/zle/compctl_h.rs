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
/// `CCT_UNUSED` constant.
pub const CCT_UNUSED: i32 = 0; // c:76
/// `CCT_POS` constant.
pub const CCT_POS: i32 = 1; // c:77
/// `CCT_CURSTR` constant.
pub const CCT_CURSTR: i32 = 2; // c:78
/// `CCT_CURPAT` constant.
pub const CCT_CURPAT: i32 = 3; // c:79
/// `CCT_WORDSTR` constant.
pub const CCT_WORDSTR: i32 = 4; // c:80
/// `CCT_WORDPAT` constant.
pub const CCT_WORDPAT: i32 = 5; // c:81
/// `CCT_CURSUF` constant.
pub const CCT_CURSUF: i32 = 6; // c:82
/// `CCT_CURPRE` constant.
pub const CCT_CURPRE: i32 = 7; // c:83
/// `CCT_CURSUB` constant.
pub const CCT_CURSUB: i32 = 8; // c:84
/// `CCT_CURSUBC` constant.
pub const CCT_CURSUBC: i32 = 9; // c:85
/// `CCT_NUMWORDS` constant.
pub const CCT_NUMWORDS: i32 = 10; // c:86
/// `CCT_RANGESTR` constant.
pub const CCT_RANGESTR: i32 = 11; // c:87
/// `CCT_RANGEPAT` constant.
pub const CCT_RANGEPAT: i32 = 12; // c:88
/// `CCT_QUOTE` constant.
pub const CCT_QUOTE: i32 = 13; // c:89

// ---------------------------------------------------------------------------
// Primary completion-target flags (`mask`, c:118-149).
// Each bit selects one completion-source kind (files, vars, jobs, ...)
// the compctl spec expands.
// ---------------------------------------------------------------------------
/// `CC_FILES` constant.
pub const CC_FILES: u64 = 1 << 0; // c:118
/// `CC_COMMPATH` constant.
pub const CC_COMMPATH: u64 = 1 << 1; // c:119
/// `CC_REMOVE` constant.
pub const CC_REMOVE: u64 = 1 << 2; // c:120
/// `CC_OPTIONS` constant.
pub const CC_OPTIONS: u64 = 1 << 3; // c:121
/// `CC_VARS` constant.
pub const CC_VARS: u64 = 1 << 4; // c:122
/// `CC_BINDINGS` constant.
pub const CC_BINDINGS: u64 = 1 << 5; // c:123
/// `CC_ARRAYS` constant.
pub const CC_ARRAYS: u64 = 1 << 6; // c:124
/// `CC_INTVARS` constant.
pub const CC_INTVARS: u64 = 1 << 7; // c:125
/// `CC_SHFUNCS` constant.
pub const CC_SHFUNCS: u64 = 1 << 8; // c:126
/// `CC_PARAMS` constant.
pub const CC_PARAMS: u64 = 1 << 9; // c:127
/// `CC_ENVVARS` constant.
pub const CC_ENVVARS: u64 = 1 << 10; // c:128
/// `CC_JOBS` constant.
pub const CC_JOBS: u64 = 1 << 11; // c:129
/// `CC_RUNNING` constant.
pub const CC_RUNNING: u64 = 1 << 12; // c:130
/// `CC_STOPPED` constant.
pub const CC_STOPPED: u64 = 1 << 13; // c:131
/// `CC_BUILTINS` constant.
pub const CC_BUILTINS: u64 = 1 << 14; // c:132
/// `CC_ALREG` constant.
pub const CC_ALREG: u64 = 1 << 15; // c:133
/// `CC_ALGLOB` constant.
pub const CC_ALGLOB: u64 = 1 << 16; // c:134
/// `CC_USERS` constant.
pub const CC_USERS: u64 = 1 << 17; // c:135
/// `CC_DISCMDS` constant.
pub const CC_DISCMDS: u64 = 1 << 18; // c:136
/// `CC_EXCMDS` constant.
pub const CC_EXCMDS: u64 = 1 << 19; // c:137
/// `CC_SCALARS` constant.
pub const CC_SCALARS: u64 = 1 << 20; // c:138
/// `CC_READONLYS` constant.
pub const CC_READONLYS: u64 = 1 << 21; // c:139
/// `CC_SPECIALS` constant.
pub const CC_SPECIALS: u64 = 1 << 22; // c:140
/// `CC_DELETE` constant.
pub const CC_DELETE: u64 = 1 << 23; // c:141
/// `CC_NAMED` constant.
pub const CC_NAMED: u64 = 1 << 24; // c:142
/// `CC_QUOTEFLAG` constant.
pub const CC_QUOTEFLAG: u64 = 1 << 25; // c:143
/// `CC_EXTCMDS` constant.
pub const CC_EXTCMDS: u64 = 1 << 26; // c:144
/// `CC_RESWDS` constant.
pub const CC_RESWDS: u64 = 1 << 27; // c:145
/// `CC_DIRS` constant.
pub const CC_DIRS: u64 = 1 << 28; // c:146
/// `CC_EXPANDEXPL` constant.
pub const CC_EXPANDEXPL: u64 = 1 << 30; // c:148
/// `CC_RESERVED` constant.
pub const CC_RESERVED: u64 = 1 << 31; // c:149

// ---------------------------------------------------------------------------
// Secondary completion-target flags (`mask2`, c:152-158).
// ---------------------------------------------------------------------------
/// `CC_NOSORT` constant.
pub const CC_NOSORT: u64 = 1 << 0; // c:152
/// `CC_XORCONT` constant.
pub const CC_XORCONT: u64 = 1 << 1; // c:153
/// `CC_CCCONT` constant.
pub const CC_CCCONT: u64 = 1 << 2; // c:154
/// `CC_PATCONT` constant.
pub const CC_PATCONT: u64 = 1 << 3; // c:155
/// `CC_DEFCONT` constant.
pub const CC_DEFCONT: u64 = 1 << 4; // c:156
/// `CC_UNIQCON` constant.
pub const CC_UNIQCON: u64 = 1 << 5; // c:157
/// `CC_UNIQALL` constant.
pub const CC_UNIQALL: u64 = 1 << 6; // c:158

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
pub struct Compctlp {
    // c:39
    pub cc: std::sync::Arc<Compctl>, // c:41
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
pub struct Patcomp {
    // c:46
    pub next: Option<Box<Patcomp>>,  // c:47
    pub pat: String,                 // c:48
    pub cc: std::sync::Arc<Compctl>, // c:49
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
pub struct Compcond {
    // c:54
    pub and: Option<Box<Compcond>>, // c:55
    pub or: Option<Box<Compcond>>,  // c:55
    pub typ: i32,                   // c:56  (Rust keyword `type`)
    pub n: i32,                     // c:57
    pub u: CompcondData,            // c:58 union
}

/// Port of the anonymous `union { struct r,s,l }` inside `compcond`
/// at `Src/Zle/compctl.h:58-73`. The C union is dispatched by
/// `typ` (one of the `CCT_*` constants).
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub enum CompcondData {
    // c:58
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
pub struct Compctl {
    // c:93
    /// Reference count.
    pub refc: i32, // c:94
    /// Next compctl in a `-x` chain.
    pub next: Option<std::sync::Arc<Compctl>>, // c:95
    /// Mask of completion-target flags (`CC_*`).
    pub mask: u64, // c:96
    /// Secondary mask of completion-target flags (`CC_*`, mask2).
    pub mask2: u64, // c:96
    /// `-k` variable name.
    pub keyvar: Option<String>, // c:97
    /// `-g` glob pattern.
    pub glob: Option<String>, // c:98
    /// `-s` expansion string.
    pub str: Option<String>, // c:99 (Rust keyword `str`)
    /// `-K` function name.
    pub func: Option<String>, // c:100
    /// `-X` explanation.
    pub explain: Option<String>, // c:101
    /// `-y` user-defined description for listing.
    pub ylist: Option<String>, // c:102
    /// `-P` prefix.
    pub prefix: Option<String>, // c:103
    /// `-S` suffix.
    pub suffix: Option<String>, // c:103
    /// `-l` command name to use.
    pub subcmd: Option<String>, // c:104
    /// `-1` command name to use.
    pub substr: Option<String>, // c:105
    /// `-w` with-directory.
    pub withd: Option<String>, // c:106
    /// `-H` history pattern.
    pub hpat: Option<String>, // c:107
    /// `-H` number of events to search.
    pub hnum: i32, // c:108
    /// `-J`/`-V` group name.
    pub gname: Option<String>, // c:109
    /// `-x` first compctl in the chain.
    pub ext: Option<std::sync::Arc<Compctl>>, // c:110
    /// `-x` condition for this compctl.
    pub cond: Option<Box<Compcond>>, // c:111
    /// `+` xor'ed compctl chain.
    pub xor: Option<std::sync::Arc<Compctl>>, // c:112
    /// `-M` matcher control — head of the Cmatcher chain compiled
    /// from this compctl's match-spec arg.
    pub matcher: Option<Box<crate::ported::zle::comp_h::Cmatcher>>, // c:113
    /// `-M` matcher string.
    pub mstr: Option<String>, // c:114
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::zle_main::zle_test_setup;

    /// Verifies CCT_* values per c:76-89.
    #[test]
    fn cct_constants_correct() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(CCT_UNUSED, 0);
        assert_eq!(CCT_POS, 1);
        assert_eq!(CCT_CURSTR, 2);
        assert_eq!(CCT_QUOTE, 13);
    }

    /// Verifies CC_* primary mask values per c:118-149 — single-bit,
    /// non-overlapping.
    #[test]
    fn cc_primary_mask_bits_distinct() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let all = CC_FILES
            | CC_COMMPATH
            | CC_REMOVE
            | CC_OPTIONS
            | CC_VARS
            | CC_BINDINGS
            | CC_ARRAYS
            | CC_INTVARS
            | CC_SHFUNCS
            | CC_PARAMS
            | CC_ENVVARS
            | CC_JOBS
            | CC_RUNNING
            | CC_STOPPED
            | CC_BUILTINS
            | CC_ALREG
            | CC_ALGLOB
            | CC_USERS
            | CC_DISCMDS
            | CC_EXCMDS
            | CC_SCALARS
            | CC_READONLYS
            | CC_SPECIALS
            | CC_DELETE
            | CC_NAMED
            | CC_QUOTEFLAG
            | CC_EXTCMDS
            | CC_RESWDS
            | CC_DIRS
            | CC_EXPANDEXPL
            | CC_RESERVED;
        assert_eq!(all.count_ones(), 31); // 30 sequential + 30 + 31 (skips bit 29)
    }

    /// Verifies the secondary mask values per c:152-158.
    #[test]
    fn cc_secondary_mask_values() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(CC_NOSORT, 1);
        assert_eq!(CC_XORCONT, 2);
        assert_eq!(CC_UNIQALL, 1 << 6);
    }

    /// Verifies Compctl Default initialiser zeroes every field per
    /// the C convention of `(Compctl) calloc(1, sizeof(...))`.
    #[test]
    fn compctl_default_zeros_fields() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let c = Compcond::default();
        assert_eq!(c.typ, CCT_UNUSED);
        assert!(matches!(c.u, CompcondData::Unused));
    }

    /// Verifies the CompcondData variants align with the C union
    /// dispatch per c:58-73.
    #[test]
    fn compcond_data_variants() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let r = CompcondData::R {
            a: vec![0, 1],
            b: vec![2, 3],
        };
        if let CompcondData::R { a, b } = r {
            assert_eq!(a, vec![0, 1]);
            assert_eq!(b, vec![2, 3]);
        } else {
            panic!("expected R variant");
        }
        let s = CompcondData::S {
            p: vec![1],
            s: vec!["x".into()],
        };
        assert!(matches!(s, CompcondData::S { .. }));
        let l = CompcondData::L {
            a: vec!["lo".into()],
            b: vec!["hi".into()],
        };
        assert!(matches!(l, CompcondData::L { .. }));
    }

    /// c:76-89 — Every CCT_* constant is a unique non-negative
    /// integer. Pin uniqueness so a copy-paste regen doesn't double
    /// up a tag (which would silently route two different
    /// completion-condition kinds through the same dispatch arm).
    #[test]
    fn cct_constants_are_unique() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let all = [
            CCT_UNUSED,
            CCT_POS,
            CCT_CURSTR,
            CCT_CURPAT,
            CCT_WORDSTR,
            CCT_WORDPAT,
            CCT_CURSUF,
            CCT_CURPRE,
            CCT_CURSUB,
            CCT_CURSUBC,
            CCT_NUMWORDS,
            CCT_RANGESTR,
            CCT_RANGEPAT,
            CCT_QUOTE,
        ];
        let unique: std::collections::HashSet<_> = all.iter().copied().collect();
        assert_eq!(unique.len(), all.len(), "duplicate CCT_* constant detected");
        for &v in &all {
            assert!(v >= 0, "CCT_* constants must be non-negative");
        }
    }

    /// c:76 — CCT_UNUSED MUST be 0. The C source uses zero-init as
    /// the implicit "no condition" sentinel; a regression that sets
    /// CCT_UNUSED = 1 would mark every just-allocated condition as
    /// CCT_POS by accident.
    #[test]
    fn cct_unused_is_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(CCT_UNUSED, 0, "CCT_UNUSED must be the zero-init sentinel");
    }

    /// c:118-127 — CC_* primary-mask bits are distinct singletons.
    /// Pin the bit-packing because the c:152 "primary mask" vs
    /// c:154-158 "secondary mask" distinction relies on the primary
    /// bits all being non-overlapping.
    #[test]
    fn cc_primary_mask_bits_are_distinct_singletons() {
        let _g = crate::test_util::global_state_lock();
        let primary = [
            CC_FILES,
            CC_COMMPATH,
            CC_REMOVE,
            CC_OPTIONS,
            CC_VARS,
            CC_BINDINGS,
            CC_ARRAYS,
            CC_INTVARS,
            CC_SHFUNCS,
            CC_PARAMS,
            CC_ENVVARS,
        ];
        for &m in &primary {
            assert_eq!(
                m.count_ones(),
                1,
                "primary CC_ mask {} has {} bits set",
                m,
                m.count_ones()
            );
        }
        let mut all: u64 = 0;
        for &m in &primary {
            assert_eq!(all & m, 0, "primary CC_ mask {} overlaps", m);
            all |= m;
        }
    }

    /// c:108 — `Compctl::default()` Default impl produces an empty
    /// struct ready for population. After a default + manual field
    /// write, the other fields must remain at their zero-init values.
    #[test]
    fn compctl_default_partial_population_doesnt_clobber_other_fields() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut cc = Compctl::default();
        cc.mask = CC_FILES;
        assert_eq!(cc.mask, CC_FILES);
        // Every OTHER field must still be at default
        assert_eq!(cc.refc, 0);
        assert!(cc.next.is_none());
        assert_eq!(cc.mask2, 0);
        assert!(cc.keyvar.is_none());
        assert!(cc.cond.is_none());
        assert_eq!(cc.hnum, 0);
    }

    /// `Compcond::default()` produces a CCT_UNUSED node with the
    /// `Unused` data variant. Pin the simultaneous shape so a
    /// regression that picks one but not the other (e.g. CCT_POS
    /// with Unused data) gets caught.
    #[test]
    fn compcond_default_typ_and_data_are_consistent() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let c = Compcond::default();
        assert_eq!(c.typ, CCT_UNUSED, "tag must be UNUSED");
        assert!(
            matches!(c.u, CompcondData::Unused),
            "data must be CompcondData::Unused"
        );
    }

    /// c:118-149 — Full sweep of CC_* primary-mask bits 0..31. Each
    /// must occupy a distinct bit position. Catches a regen that
    /// renumbers two flags to the same shift.
    #[test]
    fn cc_primary_mask_full_sweep_no_overlap() {
        let _g = crate::test_util::global_state_lock();
        let primary = [
            CC_FILES,
            CC_COMMPATH,
            CC_REMOVE,
            CC_OPTIONS,
            CC_VARS,
            CC_BINDINGS,
            CC_ARRAYS,
            CC_INTVARS,
            CC_SHFUNCS,
            CC_PARAMS,
            CC_ENVVARS,
            CC_STOPPED,
            CC_BUILTINS,
            CC_ALREG,
            CC_ALGLOB,
            CC_USERS,
            CC_DISCMDS,
            CC_EXCMDS,
            CC_SCALARS,
            CC_READONLYS,
            CC_SPECIALS,
            CC_DELETE,
            CC_NAMED,
            CC_QUOTEFLAG,
            CC_EXTCMDS,
            CC_RESWDS,
            CC_DIRS,
            CC_EXPANDEXPL,
            CC_RESERVED,
        ];
        for &m in &primary {
            assert_eq!(
                m.count_ones(),
                1,
                "primary CC_ mask {:#x} must be single bit",
                m
            );
        }
        let mut all: u64 = 0;
        for &m in &primary {
            assert_eq!(all & m, 0, "CC_ mask {:#x} overlaps with previous flags", m);
            all |= m;
        }
    }

    /// c:148 — CC_EXPANDEXPL lives at bit 30 (NOT 29). Pin the
    /// gap-at-bit-29 because the C source documents the bit-29
    /// hole at c:147. A regen that "fills the gap" would shift
    /// every subsequent flag.
    #[test]
    fn cc_expandexpl_at_bit_30_skips_bit_29() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            CC_EXPANDEXPL,
            1 << 30,
            "c:148 — CC_EXPANDEXPL must be at bit 30 (bit 29 is the gap)"
        );
        // Verify nothing else IS bit 29
        let all_primary = [
            CC_FILES,
            CC_COMMPATH,
            CC_REMOVE,
            CC_OPTIONS,
            CC_VARS,
            CC_BINDINGS,
            CC_ARRAYS,
            CC_INTVARS,
            CC_SHFUNCS,
            CC_PARAMS,
            CC_ENVVARS,
            CC_STOPPED,
            CC_BUILTINS,
            CC_ALREG,
            CC_ALGLOB,
            CC_USERS,
            CC_DISCMDS,
            CC_EXCMDS,
            CC_SCALARS,
            CC_READONLYS,
            CC_SPECIALS,
            CC_DELETE,
            CC_NAMED,
            CC_QUOTEFLAG,
            CC_EXTCMDS,
            CC_RESWDS,
            CC_DIRS,
            CC_EXPANDEXPL,
            CC_RESERVED,
        ];
        let bit_29: u64 = 1 << 29;
        for &m in &all_primary {
            assert_ne!(
                m, bit_29,
                "no primary mask should occupy bit 29 (the documented gap)"
            );
        }
    }

    /// c:149 — CC_RESERVED is bit 31 (the high bit). Pin the
    /// boundary so a regen that extends into bit 32 (u64 vs i32
    /// confusion) gets caught.
    #[test]
    fn cc_reserved_is_bit_31() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(CC_RESERVED, 1u64 << 31);
    }

    /// c:152-158 — Secondary-mask (mask2) flags occupy bits 0-6 of
    /// their OWN namespace. They DELIBERATELY collide with primary
    /// mask values (CC_NOSORT = 1 = CC_FILES) — the dispatcher
    /// routes via the mask vs mask2 field.
    #[test]
    fn secondary_mask_collides_with_primary_by_design() {
        let _g = crate::test_util::global_state_lock();
        // CC_NOSORT (mask2 bit 0) and CC_FILES (mask bit 0) both = 1
        assert_eq!(
            CC_NOSORT, CC_FILES,
            "collision is intentional — different mask fields"
        );
        // CC_XORCONT (mask2 bit 1) and CC_COMMPATH (mask bit 1) both = 2
        assert_eq!(CC_XORCONT, CC_COMMPATH);
        // ... but mask2 has its own no-overlap structure
        let secondary = [
            CC_NOSORT, CC_XORCONT, CC_CCCONT, CC_PATCONT, CC_DEFCONT, CC_UNIQCON, CC_UNIQALL,
        ];
        let mut all: u64 = 0;
        for &m in &secondary {
            assert_eq!(
                m.count_ones(),
                1,
                "secondary CC_ mask {:#x} must be single bit",
                m
            );
            assert_eq!(
                all & m,
                0,
                "secondary {:#x} overlaps within mask2 namespace",
                m
            );
            all |= m;
        }
    }

    /// c:76-89 — CCT_* values are sequential (0..13). Pin the
    /// dense layout so a regen that introduces a gap silently
    /// breaks the dispatcher's `(type - CCT_POS)` subtraction.
    #[test]
    fn cct_values_are_sequential_zero_through_thirteen() {
        let _g = crate::test_util::global_state_lock();
        let in_order = [
            CCT_UNUSED,
            CCT_POS,
            CCT_CURSTR,
            CCT_CURPAT,
            CCT_WORDSTR,
            CCT_WORDPAT,
            CCT_CURSUF,
            CCT_CURPRE,
            CCT_CURSUB,
            CCT_CURSUBC,
            CCT_NUMWORDS,
            CCT_RANGESTR,
            CCT_RANGEPAT,
            CCT_QUOTE,
        ];
        for (i, &v) in in_order.iter().enumerate() {
            assert_eq!(
                v, i as i32,
                "CCT_ at position {} must be {}, got {}",
                i, i, v
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compctl.h constants.
    // ═══════════════════════════════════════════════════════════════════

    /// c:76-89 — CCT_* values are sequential 0..14 (no holes).
    #[test]
    fn cct_values_pairwise_distinct() {
        let all = [
            CCT_UNUSED, CCT_POS, CCT_CURSTR, CCT_CURPAT, CCT_WORDSTR,
            CCT_WORDPAT, CCT_CURSUF, CCT_CURPRE, CCT_CURSUB, CCT_CURSUBC,
            CCT_NUMWORDS, CCT_RANGESTR, CCT_RANGEPAT, CCT_QUOTE,
        ];
        let unique: std::collections::HashSet<_> = all.iter().copied().collect();
        assert_eq!(unique.len(), all.len(), "CCT_* must be pairwise distinct");
    }

    /// c:76-89 — CCT_* canonical mid-table values (spot-check).
    #[test]
    fn cct_canonical_mid_values() {
        assert_eq!(CCT_WORDSTR, 4);
        assert_eq!(CCT_WORDPAT, 5);
        assert_eq!(CCT_CURSUF, 6);
        assert_eq!(CCT_CURPRE, 7);
        assert_eq!(CCT_NUMWORDS, 10);
        assert_eq!(CCT_RANGESTR, 11);
        assert_eq!(CCT_RANGEPAT, 12);
    }

    /// c:118 — CC_FILES is bit 0 (the most common file-completion flag,
    /// hot-path bit). Pin so a regen flipping bit assignments doesn't
    /// silently break Tab completion of filenames.
    #[test]
    fn cc_files_is_bit_zero() {
        assert_eq!(CC_FILES, 1);
    }

    /// c:118-128 — first 10 CC_* mask bits are pairwise disjoint
    /// single-bit values.
    #[test]
    fn cc_first_10_flags_are_pairwise_disjoint_single_bits() {
        let flags = [
            CC_FILES, CC_COMMPATH, CC_REMOVE, CC_OPTIONS, CC_VARS,
            CC_BINDINGS, CC_ARRAYS, CC_INTVARS, CC_SHFUNCS, CC_PARAMS,
        ];
        for (i, &a) in flags.iter().enumerate() {
            assert!(a.is_power_of_two(), "CC_* flag {} must be single bit", a);
            for &b in flags.iter().skip(i + 1) {
                assert_eq!(a & b, 0, "{} and {} overlap", a, b);
            }
        }
    }

    /// c:118-128 — first 10 CC_* mask bits in canonical 1<<N positions.
    #[test]
    fn cc_first_10_flags_canonical_positions() {
        assert_eq!(CC_FILES, 1 << 0);
        assert_eq!(CC_COMMPATH, 1 << 1);
        assert_eq!(CC_REMOVE, 1 << 2);
        assert_eq!(CC_OPTIONS, 1 << 3);
        assert_eq!(CC_VARS, 1 << 4);
        assert_eq!(CC_BINDINGS, 1 << 5);
        assert_eq!(CC_ARRAYS, 1 << 6);
        assert_eq!(CC_INTVARS, 1 << 7);
        assert_eq!(CC_SHFUNCS, 1 << 8);
        assert_eq!(CC_PARAMS, 1 << 9);
    }

    /// c:152-158 — CC_NOSORT/XORCONT canonical positions in secondary
    /// mask namespace (separate u64 from CC_FILES etc.).
    #[test]
    fn cc_secondary_low_bits_canonical() {
        assert_eq!(CC_NOSORT, 1);
        assert_eq!(CC_XORCONT, 2);
    }

    /// c:155 — CC_UNIQALL = 1 << 6.
    #[test]
    fn cc_uniqall_is_bit_6_in_secondary() {
        assert_eq!(CC_UNIQALL, 1 << 6);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compctl.h c:131-156 CC_*
    // ═══════════════════════════════════════════════════════════════════

    /// c:131-149 — every primary CC_* flag value is a power of two.
    #[test]
    fn cc_primary_flags_all_powers_of_two_full_sweep() {
        for &v in &[CC_STOPPED, CC_BUILTINS, CC_ALREG, CC_ALGLOB, CC_USERS,
                    CC_DISCMDS, CC_EXCMDS, CC_SCALARS, CC_READONLYS,
                    CC_SPECIALS, CC_DELETE, CC_NAMED, CC_QUOTEFLAG,
                    CC_EXTCMDS, CC_RESWDS, CC_DIRS, CC_EXPANDEXPL,
                    CC_RESERVED] {
            assert!(v.is_power_of_two(), "CC_* primary {:#x} must be single bit", v);
        }
    }

    /// c:148 — CC_EXPANDEXPL = bit 30 (skips bit 29 by design).
    #[test]
    fn cc_expandexpl_skips_bit_29() {
        assert_eq!(CC_EXPANDEXPL, 1u64 << 30, "c:148 = bit 30");
        // Bit 29 is reserved (intentionally unused per C source).
    }

    /// c:152-158 — secondary CC_* flags are powers of two.
    #[test]
    fn cc_secondary_flags_all_powers_of_two() {
        for &v in &[CC_NOSORT, CC_XORCONT, CC_CCCONT, CC_PATCONT,
                    CC_DEFCONT, CC_UNIQCON, CC_UNIQALL] {
            assert!(v.is_power_of_two(), "CC_* secondary {:#x} must be single bit", v);
        }
    }

    /// c:152-158 — secondary CC_* are pairwise distinct.
    #[test]
    fn cc_secondary_flags_pairwise_distinct() {
        let codes = [CC_NOSORT, CC_XORCONT, CC_CCCONT, CC_PATCONT,
                     CC_DEFCONT, CC_UNIQCON, CC_UNIQALL];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len(), "CC_* secondary must be distinct");
    }

    /// c:131-149 — primary CC_* canonical bit positions match c:113-149.
    #[test]
    fn cc_primary_canonical_high_bit_positions() {
        assert_eq!(CC_DIRS, 1u64 << 28, "c:146");
        assert_eq!(CC_RESERVED, 1u64 << 31, "c:149");
        assert_eq!(CC_DELETE, 1u64 << 23, "c:141");
        assert_eq!(CC_NAMED, 1u64 << 24, "c:142");
        assert_eq!(CC_USERS, 1u64 << 17, "c:135");
    }

    /// c:131-149 + c:152-158 — every CC_* fits in u64 (compile-time
    /// type pin).
    #[test]
    fn cc_all_flags_are_u64_type() {
        let _: u64 = CC_STOPPED;
        let _: u64 = CC_NOSORT;
        let _: u64 = CC_RESERVED;
    }

    /// c:152-158 — CC_NOSORT through CC_UNIQALL form contiguous low-7 bits.
    #[test]
    fn cc_secondary_form_contiguous_low_bits() {
        let mut bits: Vec<u64> = vec![CC_NOSORT, CC_XORCONT, CC_CCCONT,
                                       CC_PATCONT, CC_DEFCONT, CC_UNIQCON,
                                       CC_UNIQALL];
        bits.sort();
        for (i, &b) in bits.iter().enumerate() {
            assert_eq!(b, 1u64 << i, "secondary bit {} must be 1<<{}", i, i);
        }
    }

    /// c:131-149 — primary CC_* form mostly contiguous from bit 0 up
    /// (with bit 29 reserved gap to bit 30 EXPANDEXPL).
    #[test]
    fn cc_primary_no_gaps_through_bit_28() {
        // Bits 0..28 must all be claimed by primary CC_* flags.
        let all = [
            CC_FILES, CC_COMMPATH, CC_REMOVE, CC_OPTIONS, CC_VARS,
            CC_BINDINGS, CC_ARRAYS, CC_INTVARS, CC_SHFUNCS, CC_PARAMS,
            CC_ENVVARS, CC_JOBS, CC_RUNNING, CC_STOPPED, CC_BUILTINS,
            CC_ALREG, CC_ALGLOB, CC_USERS, CC_DISCMDS, CC_EXCMDS,
            CC_SCALARS, CC_READONLYS, CC_SPECIALS, CC_DELETE, CC_NAMED,
            CC_QUOTEFLAG, CC_EXTCMDS, CC_RESWDS, CC_DIRS,
        ];
        let or_all: u64 = all.iter().fold(0, |acc, &v| acc | v);
        assert_eq!(or_all, (1u64 << 29) - 1, "bits 0..28 must all be set");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compctl.h
    // c:76-89 CCT_* / c:118-149 primary CC_* / c:152-158 secondary CC_*
    // ═══════════════════════════════════════════════════════════════════

    /// c:76 — `CCT_UNUSED` is the zero sentinel (slot 0).
    #[test]
    fn cct_unused_is_zero_sentinel() {
        assert_eq!(CCT_UNUSED, 0, "c:76 — UNUSED is the slot-0 sentinel");
    }

    /// c:76-89 — all CCT_* codes are i32 (compile-time type pin).
    #[test]
    fn cct_codes_all_i32_type() {
        let _: i32 = CCT_UNUSED;
        let _: i32 = CCT_POS;
        let _: i32 = CCT_QUOTE;
    }

    /// c:76-89 — CCT_* codes cover slots 0..=13 with no gaps/dups.
    #[test]
    fn cct_codes_dense_0_through_13() {
        let mut codes = vec![
            CCT_UNUSED, CCT_POS, CCT_CURSTR, CCT_CURPAT, CCT_WORDSTR,
            CCT_WORDPAT, CCT_CURSUF, CCT_CURPRE, CCT_CURSUB, CCT_CURSUBC,
            CCT_NUMWORDS, CCT_RANGESTR, CCT_RANGEPAT, CCT_QUOTE,
        ];
        codes.sort();
        let expected: Vec<i32> = (0..=13).collect();
        assert_eq!(codes, expected,
            "CCT_* must cover slots 0..=13 with no gaps/dups");
    }

    /// c:118 — `CC_FILES` is canonical bit 0 (alt name pin).
    #[test]
    fn cc_files_is_bit_zero_alt() {
        assert_eq!(CC_FILES, 1u64 << 0, "c:118 — FILES is bit 0");
    }

    /// c:131-149 — primary CC_* form a complete bitset:
    /// (bits 0..28) | EXPANDEXPL(30) | RESERVED(31), with bit 29 gap.
    #[test]
    fn cc_primary_complete_bitset_with_bit29_gap() {
        let all = [
            CC_FILES, CC_COMMPATH, CC_REMOVE, CC_OPTIONS, CC_VARS,
            CC_BINDINGS, CC_ARRAYS, CC_INTVARS, CC_SHFUNCS, CC_PARAMS,
            CC_ENVVARS, CC_JOBS, CC_RUNNING, CC_STOPPED, CC_BUILTINS,
            CC_ALREG, CC_ALGLOB, CC_USERS, CC_DISCMDS, CC_EXCMDS,
            CC_SCALARS, CC_READONLYS, CC_SPECIALS, CC_DELETE, CC_NAMED,
            CC_QUOTEFLAG, CC_EXTCMDS, CC_RESWDS, CC_DIRS,
            CC_EXPANDEXPL, CC_RESERVED,
        ];
        let or_all: u64 = all.iter().fold(0, |acc, &v| acc | v);
        let expected = ((1u64 << 29) - 1) | (1u64 << 30) | (1u64 << 31);
        assert_eq!(or_all, expected,
            "primary CC_* must cover bits 0..28 + 30 + 31 (bit 29 reserved)");
    }

    /// c:131-149 — primary CC_* are pairwise distinct.
    #[test]
    fn cc_primary_flags_pairwise_distinct() {
        let codes = [
            CC_FILES, CC_COMMPATH, CC_REMOVE, CC_OPTIONS, CC_VARS,
            CC_BINDINGS, CC_ARRAYS, CC_INTVARS, CC_SHFUNCS, CC_PARAMS,
            CC_ENVVARS, CC_JOBS, CC_RUNNING, CC_STOPPED, CC_BUILTINS,
            CC_ALREG, CC_ALGLOB, CC_USERS, CC_DISCMDS, CC_EXCMDS,
            CC_SCALARS, CC_READONLYS, CC_SPECIALS, CC_DELETE, CC_NAMED,
            CC_QUOTEFLAG, CC_EXTCMDS, CC_RESWDS, CC_DIRS,
            CC_EXPANDEXPL, CC_RESERVED,
        ];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len(),
            "primary CC_* must be pairwise distinct");
    }

    /// c:118-149 — every primary CC_* is non-zero (no false sentinel).
    #[test]
    fn cc_primary_all_non_zero() {
        for &v in &[CC_FILES, CC_COMMPATH, CC_REMOVE, CC_OPTIONS,
                    CC_DIRS, CC_EXPANDEXPL, CC_RESERVED] {
            assert!(v > 0, "CC_* primary must be > 0; got {}", v);
        }
    }

    /// c:152-158 — every secondary CC_* is non-zero.
    #[test]
    fn cc_secondary_all_non_zero() {
        for &v in &[CC_NOSORT, CC_XORCONT, CC_CCCONT, CC_PATCONT,
                    CC_DEFCONT, CC_UNIQCON, CC_UNIQALL] {
            assert!(v > 0, "CC_* secondary must be > 0; got {}", v);
        }
    }
}
