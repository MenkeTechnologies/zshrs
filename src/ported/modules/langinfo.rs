//! Langinfo module — port of `Src/Modules/langinfo.c`.
//!
//! C source has zero `struct ...` / `enum ...` definitions. Rust
//! port matches: zero types. Three substantive functions
//! (`liitem`, `getlanginfo`, `scanlanginfo`) plus the 6 module
//! loaders.
//!
//! Provides the `${langinfo[NAME]}` magic-assoc backed by libc
//! `nl_langinfo(3)`.

/// `nl_names[]` — port of the static name-array at `langinfo.c:65`.
/// Each entry pairs with the parallel `nl_vals[]` array of `nl_item`
/// integer keys. Used by `liitem()` for name→item lookup and by
/// `scanlanginfo()` to enumerate every entry.
pub static NL_NAMES: &[&str] = &[                                         // c:65 nl_names
    "CODESET", "D_T_FMT", "D_FMT", "T_FMT",
    "RADIXCHAR", "THOUSEP", "YESEXPR", "NOEXPR", "CRNCYSTR",
    "ABDAY_1", "ABDAY_2", "ABDAY_3", "ABDAY_4",
    "ABDAY_5", "ABDAY_6", "ABDAY_7",
    "DAY_1", "DAY_2", "DAY_3", "DAY_4", "DAY_5", "DAY_6", "DAY_7",
    "ABMON_1", "ABMON_2", "ABMON_3", "ABMON_4", "ABMON_5", "ABMON_6",
    "ABMON_7", "ABMON_8", "ABMON_9", "ABMON_10", "ABMON_11", "ABMON_12",
    "MON_1", "MON_2", "MON_3", "MON_4", "MON_5", "MON_6",
    "MON_7", "MON_8", "MON_9", "MON_10", "MON_11", "MON_12",
    "T_FMT_AMPM", "AM_STR", "PM_STR",
    "ERA", "ERA_D_FMT", "ERA_D_T_FMT", "ERA_T_FMT", "ALT_DIGITS",
];

/// Port of `liitem()` from `Src/Modules/langinfo.c:379`. Walks the
/// parallel `nl_names[]` / `nl_vals[]` arrays looking for `name`;
/// returns the nl_item integer when found, None otherwise.
///
/// C signature: `static nl_item *liitem(const char *name)`.
/// Rust port collapses C's pointer return to `Option<libc::nl_item>`
/// — the C call sites only need the integer value, never write
/// through the pointer.
#[cfg(unix)]
pub fn liitem(name: &str) -> Option<libc::nl_item> {                     // c:379
    // C: walk the parallel arrays nl_names + nl_vals; nl_vals is
    // a hardcoded list of nl_item integers in name order. The
    // Rust port maps the name string directly via match.
    Some(match name {                                                    // c:386 strcmp
        "CODESET"     => libc::CODESET,
        "D_T_FMT"     => libc::D_T_FMT,
        "D_FMT"       => libc::D_FMT,
        "T_FMT"       => libc::T_FMT,
        "RADIXCHAR"   => libc::RADIXCHAR,
        "THOUSEP"     => libc::THOUSEP,
        "YESEXPR"     => libc::YESEXPR,
        "NOEXPR"      => libc::NOEXPR,
        #[cfg(target_os = "linux")]
        "CRNCYSTR"    => libc::CRNCYSTR,
        "ABDAY_1"     => libc::ABDAY_1,
        "ABDAY_2"     => libc::ABDAY_2,
        "ABDAY_3"     => libc::ABDAY_3,
        "ABDAY_4"     => libc::ABDAY_4,
        "ABDAY_5"     => libc::ABDAY_5,
        "ABDAY_6"     => libc::ABDAY_6,
        "ABDAY_7"     => libc::ABDAY_7,
        "DAY_1"       => libc::DAY_1,
        "DAY_2"       => libc::DAY_2,
        "DAY_3"       => libc::DAY_3,
        "DAY_4"       => libc::DAY_4,
        "DAY_5"       => libc::DAY_5,
        "DAY_6"       => libc::DAY_6,
        "DAY_7"       => libc::DAY_7,
        "ABMON_1"     => libc::ABMON_1,
        "ABMON_2"     => libc::ABMON_2,
        "ABMON_3"     => libc::ABMON_3,
        "ABMON_4"     => libc::ABMON_4,
        "ABMON_5"     => libc::ABMON_5,
        "ABMON_6"     => libc::ABMON_6,
        "ABMON_7"     => libc::ABMON_7,
        "ABMON_8"     => libc::ABMON_8,
        "ABMON_9"     => libc::ABMON_9,
        "ABMON_10"    => libc::ABMON_10,
        "ABMON_11"    => libc::ABMON_11,
        "ABMON_12"    => libc::ABMON_12,
        "MON_1"       => libc::MON_1,
        "MON_2"       => libc::MON_2,
        "MON_3"       => libc::MON_3,
        "MON_4"       => libc::MON_4,
        "MON_5"       => libc::MON_5,
        "MON_6"       => libc::MON_6,
        "MON_7"       => libc::MON_7,
        "MON_8"       => libc::MON_8,
        "MON_9"       => libc::MON_9,
        "MON_10"      => libc::MON_10,
        "MON_11"      => libc::MON_11,
        "MON_12"      => libc::MON_12,
        "T_FMT_AMPM"  => libc::T_FMT_AMPM,
        "AM_STR"      => libc::AM_STR,
        "PM_STR"      => libc::PM_STR,
        "ERA"         => libc::ERA,
        "ERA_D_FMT"   => libc::ERA_D_FMT,
        "ERA_D_T_FMT" => libc::ERA_D_T_FMT,
        "ERA_T_FMT"   => libc::ERA_T_FMT,
        "ALT_DIGITS"  => libc::ALT_DIGITS,
        _ => return None,                                                // c:391 return NULL
    })
}

/// Non-Unix fallback for `liitem` — `nl_item` is POSIX-only.
#[cfg(not(unix))]
pub fn liitem(_name: &str) -> Option<i32> {
    None
}

/// Port of `getlanginfo()` from `Src/Modules/langinfo.c:396`. The
/// magic-assoc lookup callback for `${langinfo[NAME]}`. Looks up
/// `name` via `liitem`, runs `nl_langinfo(*elem)`, and returns
/// the resulting locale string (or `None` for unset).
///
/// C signature: `static HashNode getlanginfo(HashTable ht,
///                                            const char *name)`.
/// Rust port returns `Option<String>` matching the observable
/// "u.str + PM_UNSET" duality C builds into the Param node.
#[cfg(unix)]
pub fn getlanginfo(name: &str) -> Option<String> {                       // c:396
    use std::ffi::CStr;
    // c:413 — `if (name) elem = liitem(name); else elem = NULL;`
    let elem = liitem(name)?;                                            // c:415
    unsafe {
        // c:418 — `listr = nl_langinfo(*elem)`.
        let ptr = libc::nl_langinfo(elem);                               // c:418
        if ptr.is_null() {
            return None;                                                 // c:425 PM_UNSET
        }
        let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        if s.is_empty() {
            // c:425 — empty result also flags PM_UNSET.
            return None;
        }
        Some(s)                                                          // c:419 dupstring
    }
}

/// Non-Unix fallback for `getlanginfo` — `nl_langinfo(3)` is
/// POSIX-only.
#[cfg(not(unix))]
pub fn getlanginfo(_name: &str) -> Option<String> {
    None
}

/// Port of `scanlanginfo()` from `Src/Modules/langinfo.c:430`. The
/// magic-assoc scan callback for `${(k)langinfo}` /
/// `${(kv)langinfo}`. Walks the `nl_names[]` array, calls
/// `nl_langinfo` for each entry, and yields every (name, value)
/// pair where the value is non-NULL.
///
/// C signature: `static void scanlanginfo(HashTable ht, ScanFunc
///                                         func, int flags)`.
/// Rust port returns the (name, value) pairs as a Vec since the
/// callback-driven C API doesn't translate cleanly.
pub fn scanlanginfo() -> Vec<(String, String)> {                         // c:430
    let mut out = Vec::new();
    for &name in NL_NAMES {                                              // c:444 walk nl_names
        if let Some(v) = getlanginfo(name) {                             // c:446 nl_langinfo
            out.push((name.to_string(), v));                             // c:451 emit
        }
    }
    out
}

// =====================================================================
// static struct paramdef partab[]                                   c:455
// static struct features module_features                            c:464
// =====================================================================

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::{features as features_t, module};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 0,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,                                                   // c:467 partab[1]
        pd_size: 1,
        n_abstract: 0,
    }))
}

/// Port of `setup_()` from `Src/Modules/langinfo.c:472`.
pub fn setup_(_m: *const module) -> i32 {                                // c:472
    0                                                                    // c:475
}

/// Port of `features_()` from `Src/Modules/langinfo.c:479`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {  // c:479
    *features = featuresarray(m, module_features());                    // c:482
    0                                                                    // c:483
}

/// Port of `enables_()` from `Src/Modules/langinfo.c:487`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 { // c:487
    handlefeatures(m, module_features(), enables)                       // c:490
}

/// Port of `boot_()` from `Src/Modules/langinfo.c:494`.
pub fn boot_(_m: *const module) -> i32 {                                 // c:494
    0                                                                    // c:497
}

/// Port of `cleanup_()` from `Src/Modules/langinfo.c:501`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {                               // c:501
    setfeatureenables(m, module_features(), None)                       // c:504
}

/// Port of `finish_()` from `Src/Modules/langinfo.c:508`.
pub fn finish_(_m: *const module) -> i32 {                               // c:508
    0                                                                    // c:511
}

// `featuresarray` — Src/module.c:3275.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["p:langinfo".to_string()]
}

// `handlefeatures` — Src/module.c:3370.
fn handlefeatures(m: *const module, f: &Mutex<features_t>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(getfeatureenables(m, f));
    } else if let Some(e) = enables.as_ref() {
        return setfeatureenables(m, f, Some(e));
    }
    0
}

fn getfeatureenables(_m: *const module, f: &Mutex<features_t>) -> Vec<i32> {
    let g = f.lock().unwrap();
    let total = g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract;
    vec![0; total as usize]
}

// `setfeatureenables` — Src/module.c:3445.
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nl_names_includes_codeset() {
        assert!(NL_NAMES.contains(&"CODESET"));
        assert!(NL_NAMES.contains(&"D_T_FMT"));
    }

    #[cfg(unix)]
    #[test]
    fn getlanginfo_codeset_is_some() {
        assert!(getlanginfo("CODESET").is_some());
    }

    #[test]
    fn getlanginfo_invalid_returns_none() {
        assert!(getlanginfo("INVALID_NAME").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn liitem_codeset_resolves() {
        assert!(liitem("CODESET").is_some());
        assert!(liitem("DOES_NOT_EXIST").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn scanlanginfo_emits_items() {
        let v = scanlanginfo();
        assert!(!v.is_empty());
        assert!(v.iter().any(|(k, _)| k == "CODESET"));
    }
}
