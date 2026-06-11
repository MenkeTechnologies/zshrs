//! Langinfo module — port of `Src/Modules/langinfo.c`.
//!
//! C source has zero `struct ...` / `enum ...` definitions. Rust
//! port matches: zero types. Three substantive functions
//! (`liitem`, `getlanginfo`, `scanlanginfo`) plus the 6 module
//! loaders.
//!
//! Provides the `${langinfo[NAME]}` magic-assoc backed by libc
//! `nl_langinfo(3)`.

use crate::ported::zsh_h::features;
use crate::ported::zsh_h::{
    hashnode, param, HashTable, Param, ScanFunc, PM_READONLY, PM_SCALAR,
};
use crate::utils::unmetafy;
use crate::zsh_h::module;
/// `nl_names[]` — port of the static name-array at `langinfo.c:65`.
/// Each entry pairs with the parallel `nl_vals[]` array of `nl_item`
/// integer keys. Used by `liitem()` for name→item lookup and by
/// `scanlanginfo()` to enumerate every entry.
use std::ffi::CStr;
use std::sync::{Mutex, OnceLock};

/// Port of `liitem(const char *name)` from `Src/Modules/langinfo.c:379`. Walks the
/// parallel `nl_names[]` / `nl_vals[]` arrays looking for `name`;
/// returns the nl_item integer when found, None otherwise.
///
/// C signature: `static nl_item *liitem(const char *name)`.
/// Rust port collapses C's pointer return to `Option<libc::nl_item>`
/// — the C call sites only need the integer value, never write
/// through the pointer.
/// Parallel `(nl_names[], nl_vals[])` arrays from
/// `langinfo.c:65,235` — paired here to keep liitem's body a
/// faithful loop-over-arrays match for C.
#[cfg(unix)]
static NL_TABLE: &[(&str, libc::nl_item)] = &[
    // c:65,235
    ("CODESET", libc::CODESET),
    ("D_T_FMT", libc::D_T_FMT),
    ("D_FMT", libc::D_FMT),
    ("T_FMT", libc::T_FMT),
    ("RADIXCHAR", libc::RADIXCHAR),
    ("THOUSEP", libc::THOUSEP),
    ("YESEXPR", libc::YESEXPR),
    ("NOEXPR", libc::NOEXPR),
    #[cfg(target_os = "linux")]
    ("CRNCYSTR", libc::CRNCYSTR),
    ("ABDAY_1", libc::ABDAY_1),
    ("ABDAY_2", libc::ABDAY_2),
    ("ABDAY_3", libc::ABDAY_3),
    ("ABDAY_4", libc::ABDAY_4),
    ("ABDAY_5", libc::ABDAY_5),
    ("ABDAY_6", libc::ABDAY_6),
    ("ABDAY_7", libc::ABDAY_7),
    ("DAY_1", libc::DAY_1),
    ("DAY_2", libc::DAY_2),
    ("DAY_3", libc::DAY_3),
    ("DAY_4", libc::DAY_4),
    ("DAY_5", libc::DAY_5),
    ("DAY_6", libc::DAY_6),
    ("DAY_7", libc::DAY_7),
    ("ABMON_1", libc::ABMON_1),
    ("ABMON_2", libc::ABMON_2),
    ("ABMON_3", libc::ABMON_3),
    ("ABMON_4", libc::ABMON_4),
    ("ABMON_5", libc::ABMON_5),
    ("ABMON_6", libc::ABMON_6),
    ("ABMON_7", libc::ABMON_7),
    ("ABMON_8", libc::ABMON_8),
    ("ABMON_9", libc::ABMON_9),
    ("ABMON_10", libc::ABMON_10),
    ("ABMON_11", libc::ABMON_11),
    ("ABMON_12", libc::ABMON_12),
    ("MON_1", libc::MON_1),
    ("MON_2", libc::MON_2),
    ("MON_3", libc::MON_3),
    ("MON_4", libc::MON_4),
    ("MON_5", libc::MON_5),
    ("MON_6", libc::MON_6),
    ("MON_7", libc::MON_7),
    ("MON_8", libc::MON_8),
    ("MON_9", libc::MON_9),
    ("MON_10", libc::MON_10),
    ("MON_11", libc::MON_11),
    ("MON_12", libc::MON_12),
    ("T_FMT_AMPM", libc::T_FMT_AMPM),
    ("AM_STR", libc::AM_STR),
    ("PM_STR", libc::PM_STR),
    ("ERA", libc::ERA),
    ("ERA_D_FMT", libc::ERA_D_FMT),
    ("ERA_D_T_FMT", libc::ERA_D_T_FMT),
    ("ERA_T_FMT", libc::ERA_T_FMT),
    ("ALT_DIGITS", libc::ALT_DIGITS),
];
/// `liitem` — see implementation.
#[cfg(unix)]
pub fn liitem(name: &str) -> Option<libc::nl_item> {
    // c:379
    NL_TABLE.iter().find(|(n, _)| *n == name).map(|(_, v)| *v) // c:386 strcmp
}

/// Port of `liitem(const char *name)` from `Src/Modules/langinfo.c:379`.
/// Non-Unix fallback for `liitem` — `nl_item` is POSIX-only.
#[cfg(not(unix))]
#[allow(unused_variables)]
pub fn liitem(name: &str) -> Option<i32> {
    // c:379
    None
}

/// Port of `getlanginfo(UNUSED(HashTable ht), const char *name)` from `Src/Modules/langinfo.c:396`. The
/// magic-assoc lookup callback for `${langinfo[NAME]}`. Looks up
/// `name` via `liitem`, runs `nl_langinfo(*elem)`, and returns a
/// Param carrying the locale string (or `None` for unset) —
/// PARTAB-dispatch shape matching `getpmsysparams`.
#[cfg(unix)]
pub fn getlanginfo(_ht: *mut HashTable, name: &str) -> Option<Param> {
    // c:396
    // c:403-404 — `nameu = dupstring(name); unmetafy(nameu, &len);`
    let mut buf = name.as_bytes().to_vec(); // c:403
    unmetafy(&mut buf); // c:404
    let nameu = std::str::from_utf8(&buf).ok()?;
    // c:411-415 — `if (name) elem = liitem(name); else elem = NULL;`
    let elem = liitem(nameu)?; // c:412
    let listr = unsafe {
        // c:416 — `listr = nl_langinfo(*elem)`. C only sets PM_UNSET
        // when `elem` is NULL or `listr` is NULL — an empty result
        // string is treated as a valid (present) value, NOT unset:
        //
        //   if (elem && (listr = nl_langinfo(*elem))) {
        //       pm->u.str = dupstring(listr);
        //   } else {
        //       pm->u.str = dupstring("");
        //       pm->node.flags |= PM_UNSET;
        //   }
        //
        // Prior Rust port conflated `""` with PM_UNSET by mapping
        // empty results to None, which (a) made `${+langinfo[X]}`
        // return 0 for legitimately-empty fields, and (b) caused
        // scanlanginfo to silently drop them from `${(kv)langinfo}`.
        let ptr = libc::nl_langinfo(elem); // c:416
        if ptr.is_null() {
            return None; // c:421-423 PM_UNSET
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned() // c:417 dupstring
    };
    // c:406-409 — pm = hcalloc(...); PM_SCALAR | PM_READONLY.
    Some(Box::new(param {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: PM_SCALAR as i32 | PM_READONLY as i32, // c:408
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(listr), // c:417 pm->u.str
        u_val: 0,
        u_dval: 0.0,
        u_hash: None,
        gsu_s: None,
        gsu_i: None,
        gsu_f: None,
        gsu_a: None,
        gsu_h: None,
        base: 0,
        width: 0,
        env: None,
        ename: None,
        old: None,
        level: 0,
    }))
}

/// Port of `getlanginfo(UNUSED(HashTable ht), const char *name)` from `Src/Modules/langinfo.c:396`.
/// Non-Unix fallback for `getlanginfo` — `nl_langinfo(3)` is
/// POSIX-only.
#[cfg(not(unix))]
#[allow(unused_variables)]
pub fn getlanginfo(_ht: *mut HashTable, _name: &str) -> Option<Param> {
    // c:396
    None
}

/// Port of `scanlanginfo(UNUSED(HashTable ht), ScanFunc func, int flags)` from `Src/Modules/langinfo.c:430`. The
/// magic-assoc scan callback for `${(k)langinfo}` /
/// `${(kv)langinfo}`. Walks the `nl_names[]` array, calls
/// `nl_langinfo` for each entry, and dispatches every present
/// (name, value) pair through `func` — PARTAB-dispatch shape
/// matching `scanpmsysparams`.
pub fn scanlanginfo(_ht: *mut HashTable, func: Option<ScanFunc>, flags: i32) {
    // c:430
    let f = match func {
        Some(f) => f,
        None => return,
    };
    for &name in NL_NAMES {
        // c:444 walk nl_names
        if let Some(pm) = getlanginfo(std::ptr::null_mut(), name) {
            // c:446 nl_langinfo
            let node_box = Box::new(pm.node.clone());
            f(&node_box, flags); // c:451 func(&pm->node, flags)
        }
    }
}

// `partab` — port of `static struct paramdef partab[]` (langinfo.c:455).

// `module_features` — port of `static struct features module_features`
// from langinfo.c:464.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/langinfo.c:472`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:472
    0 // c:487
}

// =====================================================================
// static struct paramdef partab[]                                   c:455
// static struct features module_features                            c:464
// =====================================================================

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/langinfo.c:479`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:479
    *features = featuresarray(m, module_features());
    0 // c:494
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/langinfo.c:487`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:487
    handlefeatures(m, module_features(), enables) // c:501
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/langinfo.c:494`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:494
    0 // c:508
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/langinfo.c:501`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:501
    setfeatureenables(m, module_features(), None) // c:508
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/langinfo.c:508`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:508
    0 // c:508
}
/// `NL_NAMES` static.
pub static NL_NAMES: &[&str] = &[
    // c:65 nl_names
    "CODESET",
    "D_T_FMT",
    "D_FMT",
    "T_FMT",
    "RADIXCHAR",
    "THOUSEP",
    "YESEXPR",
    "NOEXPR",
    "CRNCYSTR",
    "ABDAY_1",
    "ABDAY_2",
    "ABDAY_3",
    "ABDAY_4",
    "ABDAY_5",
    "ABDAY_6",
    "ABDAY_7",
    "DAY_1",
    "DAY_2",
    "DAY_3",
    "DAY_4",
    "DAY_5",
    "DAY_6",
    "DAY_7",
    "ABMON_1",
    "ABMON_2",
    "ABMON_3",
    "ABMON_4",
    "ABMON_5",
    "ABMON_6",
    "ABMON_7",
    "ABMON_8",
    "ABMON_9",
    "ABMON_10",
    "ABMON_11",
    "ABMON_12",
    "MON_1",
    "MON_2",
    "MON_3",
    "MON_4",
    "MON_5",
    "MON_6",
    "MON_7",
    "MON_8",
    "MON_9",
    "MON_10",
    "MON_11",
    "MON_12",
    "T_FMT_AMPM",
    "AM_STR",
    "PM_STR",
    "ERA",
    "ERA_D_FMT",
    "ERA_D_T_FMT",
    "ERA_T_FMT",
    "ALT_DIGITS",
];

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN LANGINFO.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["p:langinfo".to_string()]
}

// WARNING: NOT IN LANGINFO.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

// WARNING: NOT IN LANGINFO.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN LANGINFO.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 0,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 1,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test fixture — unwraps the canonical `getlanginfo` Param to
    /// the value string, like `vm_helper::partab_get` does.
    fn getli(name: &str) -> Option<String> {
        getlanginfo(std::ptr::null_mut(), name).and_then(|p| p.u_str)
    }

    /// Test fixture — collects the canonical `scanlanginfo` callback
    /// stream into (key, value) pairs, composing scan + per-key get
    /// the same way `vm_helper::partab_scan_keys` + `partab_get` do.
    fn scanli() -> Vec<(String, String)> {
        use std::cell::RefCell;
        thread_local! {
            static KEYS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        }
        KEYS.with(|k| k.borrow_mut().clear());
        fn cb(node: &crate::ported::zsh_h::HashNode, _flags: i32) {
            KEYS.with(|k| k.borrow_mut().push(node.nam.clone()));
        }
        scanlanginfo(std::ptr::null_mut(), Some(cb), 0);
        KEYS.with(|k| {
            k.borrow()
                .iter()
                .map(|name| (name.clone(), getli(name).unwrap_or_default()))
                .collect()
        })
    }

    #[test]
    fn nl_names_includes_codeset() {
        let _g = crate::test_util::global_state_lock();
        assert!(NL_NAMES.contains(&"CODESET"));
        assert!(NL_NAMES.contains(&"D_T_FMT"));
    }

    #[cfg(unix)]
    #[test]
    fn getlanginfo_codeset_is_some() {
        let _g = crate::test_util::global_state_lock();
        assert!(getli("CODESET").is_some());
    }

    #[test]
    fn getlanginfo_invalid_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getli("INVALID_NAME").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn liitem_codeset_resolves() {
        let _g = crate::test_util::global_state_lock();
        assert!(liitem("CODESET").is_some());
        assert!(liitem("DOES_NOT_EXIST").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn scanlanginfo_emits_items() {
        let _g = crate::test_util::global_state_lock();
        let v = scanli();
        assert!(!v.is_empty());
        assert!(v.iter().any(|(k, _)| k == "CODESET"));
    }

    /// c:65 — `nl_names` is the authoritative table of every POSIX
    /// nl_item name. Verify the set includes every category the
    /// langinfo(3) spec lists, not just CODESET. A regression that
    /// truncates the table would silently break `$(getlanginfo D_FMT)`
    /// and similar lookups in user scripts.
    #[test]
    fn nl_names_covers_canonical_locale_items() {
        let _g = crate::test_util::global_state_lock();
        for required in [
            "CODESET",
            "D_T_FMT",
            "D_FMT",
            "T_FMT",
            "T_FMT_AMPM",
            "AM_STR",
            "PM_STR",
            "DAY_1",
            "DAY_7",
            "ABDAY_1",
            "MON_1",
            "MON_12",
            "RADIXCHAR",
            "THOUSEP",
            "YESEXPR",
            "NOEXPR",
        ] {
            assert!(
                NL_NAMES.contains(&required),
                "NL_NAMES missing {} — port table truncated?",
                required
            );
        }
    }

    /// c:65 — every entry in NL_NAMES must be a valid nl_item name
    /// per langinfo.h: ALL_CAPS_WITH_UNDERSCORES, no leading digit.
    /// Pinning the shape catches a regression that adds spaces or
    /// lowercase entries (which would silently fail `getlanginfo` on
    /// the user-facing builtin path).
    #[test]
    fn nl_names_entries_are_uppercase_identifiers() {
        let _g = crate::test_util::global_state_lock();
        for &n in NL_NAMES {
            assert!(!n.is_empty(), "empty entry in NL_NAMES");
            assert!(
                n.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "NL_NAMES entry {:?} contains non-uppercase chars",
                n
            );
            assert!(
                !n.starts_with(|c: char| c.is_ascii_digit()),
                "NL_NAMES entry {:?} starts with a digit",
                n
            );
        }
    }

    /// c:65 — NL_NAMES must not have duplicate entries. The C source
    /// preserves the LC_* category groupings; an accidental duplicate
    /// would silently double-emit in `scanlanginfo`.
    #[test]
    fn nl_names_has_no_duplicates() {
        let _g = crate::test_util::global_state_lock();
        let unique: std::collections::HashSet<_> = NL_NAMES.iter().copied().collect();
        assert_eq!(unique.len(), NL_NAMES.len(), "duplicate entry in NL_NAMES");
    }

    /// c:430 — `scanlanginfo` keys must be a subset of NL_NAMES.
    /// (The C source walks `nl_names` in order.) Anything extra
    /// would indicate a parallel hardcoded list drifted out of sync
    /// with the canonical table.
    #[cfg(unix)]
    #[test]
    fn scanlanginfo_keys_are_subset_of_nl_names() {
        let _g = crate::test_util::global_state_lock();
        for (k, _) in scanli() {
            assert!(
                NL_NAMES.contains(&k.as_str()),
                "scanlanginfo emitted {:?} which is not in NL_NAMES",
                k
            );
        }
    }

    /// c:396 — `getlanginfo` is case-sensitive: lowercase input must
    /// not match the uppercase canonical name. Catches a regression
    /// that adds a `.to_uppercase()` for "convenience".
    #[cfg(unix)]
    #[test]
    fn getlanginfo_is_case_sensitive() {
        let _g = crate::test_util::global_state_lock();
        assert!(getli("CODESET").is_some());
        assert!(
            getli("codeset").is_none(),
            "getlanginfo must be case-sensitive per the C source's strcmp lookup"
        );
    }

    /// c:472-510 — module-lifecycle stubs all return 0 in C.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const crate::ported::zsh_h::module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    // ─── zsh-corpus pins ────────────────────────────────────────────

    /// `liitem("CODESET")` returns Some (POSIX `nl_item` standard key).
    #[test]
    fn langinfo_corpus_codeset_is_known() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            liitem("CODESET").is_some(),
            "CODESET is a POSIX-standard nl_item key"
        );
    }

    /// `liitem("DAY_1")` returns Some (POSIX day name).
    #[test]
    fn langinfo_corpus_day_1_is_known() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            liitem("DAY_1").is_some(),
            "DAY_1 is a POSIX-standard nl_item key"
        );
    }

    /// `liitem("RADIXCHAR")` is known.
    #[test]
    fn langinfo_corpus_radixchar_is_known() {
        let _g = crate::test_util::global_state_lock();
        assert!(liitem("RADIXCHAR").is_some());
    }

    /// `liitem("BOGUS_NOT_REAL_KEY")` returns None.
    #[test]
    fn langinfo_corpus_unknown_key_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(liitem("BOGUS_NOT_REAL_KEY").is_none());
        assert!(liitem("").is_none());
    }

    /// `getli("CODESET")` returns a non-empty string (system locale).
    #[test]
    fn langinfo_corpus_getlanginfo_codeset_nonempty() {
        let _g = crate::test_util::global_state_lock();
        let r = getli("CODESET");
        assert!(r.is_some(), "CODESET resolves to a string");
        let s = r.unwrap();
        assert!(!s.is_empty(), "CODESET non-empty (e.g. 'UTF-8'), got {s:?}");
    }

    /// `scanlanginfo` returns non-empty list (system has known keys).
    #[test]
    fn langinfo_corpus_scanlanginfo_returns_entries() {
        let _g = crate::test_util::global_state_lock();
        let entries = scanli();
        assert!(
            !entries.is_empty(),
            "scanlanginfo should return some entries"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/langinfo.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:379 — `liitem` for canonical POSIX names returns Some.
    #[test]
    #[cfg(unix)]
    fn liitem_canonical_posix_names_resolve() {
        let _g = crate::test_util::global_state_lock();
        assert!(liitem("CODESET").is_some(), "CODESET is POSIX-required");
        assert!(liitem("D_FMT").is_some(), "D_FMT is POSIX-required");
        assert!(liitem("T_FMT").is_some(), "T_FMT is POSIX-required");
    }

    /// c:379 — `liitem` deterministic for same input.
    #[test]
    #[cfg(unix)]
    fn liitem_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for name in &["CODESET", "BOGUS_XYZ", "", "AM_STR"] {
            let first = liitem(name);
            for _ in 0..5 {
                assert_eq!(liitem(name), first, "{:?} must be pure", name);
            }
        }
    }

    /// c:396 — `getli("")` returns None (empty name not in table).
    #[test]
    #[cfg(unix)]
    fn getlanginfo_empty_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getli("").is_none());
    }

    /// c:396 — `getlanginfo` of unknown name returns None.
    #[test]
    #[cfg(unix)]
    fn getlanginfo_unknown_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getli("zzz_never_a_real_key").is_none());
    }

    /// c:430 — `scanlanginfo` output contains CODESET on every system
    /// (POSIX-required cap).
    #[test]
    #[cfg(unix)]
    fn scanlanginfo_includes_codeset() {
        let _g = crate::test_util::global_state_lock();
        let entries = scanli();
        let has_codeset = entries.iter().any(|(k, _)| k == "CODESET");
        assert!(has_codeset, "POSIX CODESET must appear in scan output");
    }

    /// c:430 — `scanlanginfo` deterministic (same locale → same output).
    #[test]
    #[cfg(unix)]
    fn scanlanginfo_is_deterministic_for_static_locale() {
        let _g = crate::test_util::global_state_lock();
        let a = scanli();
        let b = scanli();
        assert_eq!(a, b, "two consecutive scans must agree");
    }

    /// Lifecycle (c:183/210/217) split per-hook.
    #[test]
    fn langinfo_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:210 — boot_(NULL) = 0.
    #[test]
    fn langinfo_boot_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/langinfo.c
    // c:94 liitem / c:119 getlanginfo / c:163 scanlanginfo / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:94 — `liitem("")` empty returns None.
    #[cfg(unix)]
    #[test]
    fn liitem_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(liitem("").is_none(), "empty name → None");
    }

    /// c:94 — `liitem` is deterministic.
    #[cfg(unix)]
    #[test]
    fn liitem_is_deterministic_full_sweep() {
        let _g = crate::test_util::global_state_lock();
        for s in ["CODESET", "DAY_1", "MON_1", "RADIXCHAR", "__unknown_xyz__"] {
            let first = liitem(s);
            for _ in 0..3 {
                assert_eq!(liitem(s), first, "liitem({:?}) must be deterministic", s);
            }
        }
    }

    /// c:119 — `getlanginfo` returns Option<String>.
    #[cfg(unix)]
    #[test]
    fn getlanginfo_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = getli("CODESET");
    }

    /// c:119 — `getlanginfo` is deterministic for same locale.
    #[cfg(unix)]
    #[test]
    fn getlanginfo_deterministic_for_codeset() {
        let _g = crate::test_util::global_state_lock();
        let first = getli("CODESET");
        for _ in 0..3 {
            assert_eq!(
                getli("CODESET"),
                first,
                "getli('CODESET') must be deterministic"
            );
        }
    }

    /// c:163 — `scanlanginfo` returns Vec<(String, String)>.
    #[test]
    fn scanlanginfo_returns_vec_tuple_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<(String, String)> = scanli();
    }

    /// c:163 — `scanlanginfo` is deterministic full-sweep.
    #[cfg(unix)]
    #[test]
    fn scanlanginfo_full_sweep_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = scanli();
        for _ in 0..3 {
            assert_eq!(
                scanli(),
                first,
                "scanlanginfo must be fully deterministic"
            );
        }
    }

    /// c:217 — cleanup_(NULL) = 0.
    #[test]
    fn langinfo_cleanup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cleanup_(std::ptr::null()), 0);
    }

    /// c:224 — finish_(NULL) = 0.
    #[test]
    fn langinfo_finish_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    /// c:163 — `scanlanginfo` entries: name+value are both ASCII-safe.
    #[cfg(unix)]
    #[test]
    fn scanlanginfo_entries_are_ascii_keys() {
        let _g = crate::test_util::global_state_lock();
        let entries = scanli();
        for (k, _v) in &entries {
            assert!(k.is_ascii(), "key {:?} must be ASCII", k);
        }
    }

    /// c:183 + c:217 — setup_/cleanup_ round-trip safe.
    #[test]
    fn langinfo_setup_cleanup_round_trip_safe() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
        assert_eq!(cleanup_(std::ptr::null()), 0);
        assert_eq!(setup_(std::ptr::null()), 0);
        assert_eq!(cleanup_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/langinfo.c
    // c:94 liitem / c:119 getlanginfo / c:163 scanlanginfo + lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:94 — `liitem("")` empty input returns None (not a known item).
    #[cfg(unix)]
    #[test]
    fn liitem_empty_string_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(liitem("").is_none(), "empty langinfo item must be None");
    }

    /// c:94 — `liitem` returns None for nonsense names.
    #[cfg(unix)]
    #[test]
    fn liitem_nonsense_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            liitem("___definitely_not_a_langinfo_item_xyz___").is_none(),
            "unknown langinfo name must be None"
        );
    }

    /// c:94 — `liitem` returns Some for known POSIX items.
    /// CODESET is the most universally-supported langinfo item per POSIX.
    #[cfg(unix)]
    #[test]
    fn liitem_codeset_returns_some() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            liitem("CODESET").is_some(),
            "CODESET must resolve on every POSIX libc"
        );
    }

    /// c:119 — `getli("")` for empty name returns None (alt pin).
    #[cfg(unix)]
    #[test]
    fn getlanginfo_empty_name_returns_none_alt() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            getli("").is_none(),
            "empty langinfo name must yield None"
        );
    }

    /// c:119 — `getlanginfo` for nonsense name returns None.
    #[cfg(unix)]
    #[test]
    fn getlanginfo_nonsense_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getli("___bogus_langinfo_xyz___").is_none());
    }

    /// c:163 — `scanlanginfo` no-args is fast (deterministic snapshot).
    /// Pin: result must not contain empty keys (every entry has a name).
    #[cfg(unix)]
    #[test]
    fn scanlanginfo_no_empty_keys() {
        let _g = crate::test_util::global_state_lock();
        for (k, _v) in scanli() {
            assert!(!k.is_empty(), "no scanlanginfo entry may have empty key");
        }
    }

    /// c:163 — `scanlanginfo` entries have no duplicate keys.
    #[cfg(unix)]
    #[test]
    fn scanlanginfo_no_duplicate_keys() {
        let _g = crate::test_util::global_state_lock();
        let entries = scanli();
        let mut seen = std::collections::HashSet::new();
        for (k, _) in &entries {
            assert!(
                seen.insert(k.clone()),
                "duplicate langinfo key {:?} in scanlanginfo output",
                k
            );
        }
    }

    /// c:163 — `scanlanginfo` keys are uppercase ASCII (POSIX item names).
    #[cfg(unix)]
    #[test]
    fn scanlanginfo_keys_are_uppercase_ascii() {
        let _g = crate::test_util::global_state_lock();
        for (k, _) in scanli() {
            assert!(
                k.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "langinfo key {:?} must be uppercase ASCII + digits + underscore",
                k
            );
        }
    }

    /// c:183 — `setup_` returns i32 (compile-time pin).
    #[test]
    fn langinfo_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:195 — `features_` returns i32 (compile-time pin).
    #[test]
    fn langinfo_features_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<String> = Vec::new();
        let _: i32 = features_(std::ptr::null(), &mut v);
    }

    /// c:210 — `boot_` returns i32 (compile-time pin).
    #[test]
    fn langinfo_boot_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = boot_(std::ptr::null());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/langinfo.c
    // c:94 liitem / c:119 getlanginfo / c:163 scanlanginfo /
    // c:183-224 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:183 — `setup_` is idempotent.
    #[test]
    fn langinfo_setup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:217 — `cleanup_` is idempotent.
    #[test]
    fn langinfo_cleanup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:224 — `finish_` is idempotent.
    #[test]
    fn langinfo_finish_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:217 — `cleanup_` return type i32 (compile-time pin).
    #[test]
    fn langinfo_cleanup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cleanup_(std::ptr::null());
    }

    /// c:224 — `finish_` return type i32 (compile-time pin).
    #[test]
    fn langinfo_finish_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = finish_(std::ptr::null());
    }

    /// c:94 — `liitem("")` empty name returns None.
    #[test]
    fn liitem_empty_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(liitem("").is_none(), "empty name must yield None");
    }

    /// c:94 — `liitem("__never_real_xyz__")` unknown returns None.
    #[test]
    fn liitem_unknown_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            liitem("__never_real_li_item_xyz__").is_none(),
            "unknown name must yield None"
        );
    }

    /// c:119 — `getli("")` empty name returns None (alt 2).
    #[test]
    fn getlanginfo_empty_name_returns_none_alt_2() {
        let _g = crate::test_util::global_state_lock();
        assert!(getli("").is_none(), "empty name must yield None");
    }

    /// c:119 — `getlanginfo` deterministic for same input.
    #[test]
    fn getlanginfo_deterministic_for_same_input() {
        let _g = crate::test_util::global_state_lock();
        let a = getli("CODESET");
        let b = getli("CODESET");
        assert_eq!(a, b, "getli(CODESET) must be deterministic");
    }

    /// c:163 — `scanlanginfo` returns Vec<(String, String)>.
    #[test]
    fn scanlanginfo_returns_vec_tuple_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<(String, String)> = scanli();
    }

    /// c:163 — `scanlanginfo` is deterministic across calls.
    #[test]
    fn scanlanginfo_deterministic_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        let a = scanli();
        let b = scanli();
        assert_eq!(a, b, "scanlanginfo must be deterministic");
    }

    /// c:203 — `enables_` with Some(non-empty) doesn't panic.
    #[test]
    fn langinfo_enables_with_some_non_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = Some(vec![1, 2, 3]);
        let _ = enables_(std::ptr::null(), &mut e);
    }

    /// c:183/210/224 — setup→boot→finish chain returns 0 each.
    #[test]
    fn langinfo_setup_boot_finish_chain_returns_zero_each() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        assert_eq!(boot_(null), 0);
        assert_eq!(finish_(null), 0);
    }
}
