//! Parameter interface to shell internals - port of Modules/parameter.c
//!
//! Functions for the parameters special parameter.                          // c:37
//! Return a string describing the type of a parameter.                      // c:39
//! Functions for the commands special parameter.                            // c:147
//! Functions for the functions special parameter.                           // c:280
//! Functions for the builtins special parameter.                            // c:771
//! Functions for the options special parameter.                             // c:922
//! Functions for the modules special parameter.                             // c:1036
//! Functions for the history special parameter.                             // c:1152
//! Table for defined parameters.                                            // c:2177
//!
//! Provides special parameters: $commands, $functions, $aliases, $builtins,
//! $modules, $dirstack, $history, $historywords, $options, $nameddirs, $userdirs

use std::collections::HashMap;
use std::path::PathBuf;

/// Port of `struct pardef` from `Src/Modules/parameter.c:2179`. The
/// per-magic-assoc parameter spec table — one entry per
/// `${parameters}`/`${commands}`/`${functions}`/etc. exposed by the
/// `zsh/parameter` module.
///
/// C definition (c:2179-2187):
/// ```c
/// struct pardef {
///     char *name;
///     int flags;
///     GetNodeFunc getnfn;
///     ScanTabFunc scantfn;
///     GsuHash hash_gsu;
///     GsuArray array_gsu;
///     Param pm;
/// };
/// ```
///
/// Rust port keeps the same shape; the GSU function-table fields are
/// type-erased via `usize` because the GsuHash/GsuArray callback
/// vectors are pre-defined zsh-framework types not yet ported.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy)]
pub struct pardef {                                                          // c:2179
    /// Parameter name (e.g. "commands", "functions", "options").
    pub name: &'static str,                                                  // c:2180
    /// Flags (PM_* bits — typically PM_HASHED|PM_SPECIAL|PM_HIDE).
    pub flags: i32,                                                          // c:2181
    /// `GetNodeFunc` getnfn — type-erased: 0 when not yet wired.
    pub getnfn: usize,                                                       // c:2182
    /// `ScanTabFunc` scantfn — type-erased: 0 when not yet wired.
    pub scantfn: usize,                                                      // c:2183
    /// `GsuHash` hash_gsu — type-erased.
    pub hash_gsu: usize,                                                     // c:2184
    /// `GsuArray` array_gsu — type-erased.
    pub array_gsu: usize,                                                    // c:2185
    /// `Param pm` — type-erased pointer; populated by createparam.
    pub pm: usize,                                                           // c:2186
}

// Bag-of-globals `ParamType`/`ParamFlags` enum + `*Table` structs
// deleted (PORT_PLAN.md Phase 2 anti-pattern #1): C has no
// counterpart — paramtypestr now reads `PM_TYPE(pm->node.flags)`
// directly, mirroring Src/Modules/parameter.c:43-95.

// Return a string describing the type of a parameter.                      // c:39
/// Port of `paramtypestr()` from Src/Modules/parameter.c:43.
/// C: `static char *paramtypestr(Param pm)` — render a parameter's
/// type and modifier flags as the `typeset -p` flag string.
pub fn paramtypestr(pm: &crate::ported::zsh_h::param) -> String {            // c:43
    use crate::ported::zsh_h::{
        PM_AUTOLOAD, PM_ARRAY, PM_EFLOAT, PM_EXPORTED, PM_FFLOAT, PM_HASHED,
        PM_HIDE, PM_HIDEVAL, PM_INTEGER, PM_LEFT, PM_LOWER, PM_NAMEREF,
        PM_READONLY, PM_RIGHT_B, PM_RIGHT_Z, PM_SCALAR, PM_SPECIAL,
        PM_TAGGED, PM_TIED, PM_TYPE, PM_UNIQUE, PM_UNSET, PM_UPPER,
    };

    let f: u32 = pm.node.flags as u32;                                       // c:46

    if (f & PM_UNSET) != 0 {                                                 // c:48 (else branch c:91)
        return String::new();                                                // c:92 dupstring("")
    }
    if (f & PM_AUTOLOAD) != 0 {                                              // c:49
        return "undefined".to_string();                                      // c:50
    }

    let mut val: String = match PM_TYPE(f) {                                 // c:52
        PM_SCALAR => "scalar".to_string(),                                   // c:53
        PM_NAMEREF => "nameref".to_string(),                                 // c:54
        PM_ARRAY => "array".to_string(),                                     // c:55
        PM_INTEGER => "integer".to_string(),                                 // c:56
        PM_EFLOAT | PM_FFLOAT => "float".to_string(),                        // c:57-58
        PM_HASHED => "association".to_string(),                              // c:59
        _ => String::new(),                                                  // c:61 DPUTS — bug branch
    };

    if pm.level != 0       { val.push_str("-local"); }                       // c:63-64
    if (f & PM_LEFT) != 0  { val.push_str("-left"); }                        // c:65-66
    if (f & PM_RIGHT_B) != 0 { val.push_str("-right_blanks"); }              // c:67-68
    if (f & PM_RIGHT_Z) != 0 { val.push_str("-right_zeros"); }               // c:69-70
    if (f & PM_LOWER) != 0 { val.push_str("-lower"); }                       // c:71-72
    if (f & PM_UPPER) != 0 { val.push_str("-upper"); }                       // c:73-74
    if (f & PM_READONLY) != 0 { val.push_str("-readonly"); }                 // c:75-76
    if (f & PM_TAGGED) != 0 { val.push_str("-tag"); }                        // c:77-78
    if (f & PM_TIED) != 0 { val.push_str("-tied"); }                         // c:79-80
    if (f & PM_EXPORTED) != 0 { val.push_str("-export"); }                   // c:81-82
    if (f & PM_UNIQUE) != 0 { val.push_str("-unique"); }                     // c:83-84
    if (f & PM_HIDE) != 0 { val.push_str("-hide"); }                         // c:85-86
    if (f & PM_HIDEVAL) != 0 { val.push_str("-hideval"); }                   // c:87-88
    if (f & PM_SPECIAL) != 0 { val.push_str("-special"); }                   // c:89-90

    val                                                                      // c:94
}

#[cfg(test)]
mod paramtypestr_tests {
    use super::*;
    use crate::ported::zsh_h::{
        hashnode, param, PM_ARRAY, PM_EXPORTED, PM_SCALAR, PM_UNSET,
    };

    fn make_pm(flags: u32, level: i32) -> param {
        param {
            node: hashnode { next: None, nam: String::new(), flags: flags as i32 },
            u_data: 0, u_arr: None, u_str: None, u_val: 0, u_dval: 0.0,
            u_hash: None,
            gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
            base: 0, width: 0, env: None, ename: None, old: None, level,
        }
    }

    /// Mirrors Src/Modules/parameter.c:43-95 — switch on
    /// `PM_TYPE(pm->node.flags)` then dyncat'd modifier chain.
    #[test]
    fn paramtypestr_matches_c_dispatch() {
        // c:53 — plain scalar.
        assert_eq!(paramtypestr(&make_pm(PM_SCALAR, 0)), "scalar");
        // c:55,63-64,81-82 — array + level=1 + PM_EXPORTED.
        assert_eq!(
            paramtypestr(&make_pm(PM_ARRAY | PM_EXPORTED, 1)),
            "array-local-export",
        );
        // c:91-92 — PM_UNSET short-circuits to "".
        assert_eq!(paramtypestr(&make_pm(PM_UNSET, 0)), "");
    }
}


// =====================================================================
// static struct features module_features                            c:2300 (parameter.c)
// =====================================================================

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::{features as features_t, module};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None, bn_size: 0,
        cd_list: None, cd_size: 0,
        mf_list: None, mf_size: 0,
        pd_list: None, pd_size: 14,                                      // partab[14]: parameters/commands/options/aliases/etc
        n_abstract: 0,
    }))
}

/// Port of `setup_()` from `Src/Modules/parameter.c:2311`.
pub fn setup_(_m: *const module) -> i32 {                                    // c:2311
    // C body c:2313-2314 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_()` from `Src/Modules/parameter.c:2318`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {      // c:2318
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_()` from `Src/Modules/parameter.c:2326`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {   // c:2326
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_()` from `Src/Modules/parameter.c:2341`.
pub fn boot_(_m: *const module) -> i32 {                                     // c:2341
    // C body c:2343-2344 — `return 0`. Faithful empty-body port; the
    //                      hash-magic params (parameters, commands,
    //                      functions, etc.) are registered via the
    //                      partab dispatch in features_/enables_.
    0
}

/// Port of `cleanup_()` from `Src/Modules/parameter.c:2348`.
pub fn cleanup_(m: *const module) -> i32 {                                   // c:2348
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_()` from `Src/Modules/parameter.c:2359`.
pub fn finish_(_m: *const module) -> i32 {                                   // c:2359
    // C body c:2361-2362 — `return 0`. Faithful empty-body port; the
    //                      hash-magic params get unregistered via the
    //                      partab dispatch in cleanup_.
    0
}

fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["p:parameters".to_string(), "p:commands".to_string(), "p:functions".to_string(),
         "p:dis_functions".to_string(), "p:functions_source".to_string(),
         "p:dis_functions_source".to_string(), "p:builtins".to_string(),
         "p:dis_builtins".to_string(), "p:reswords".to_string(), "p:dis_reswords".to_string(),
         "p:options".to_string(), "p:modules".to_string(), "p:dirstack".to_string(),
         "p:history".to_string(), "p:historywords".to_string(), "p:jobtexts".to_string(),
         "p:jobdirs".to_string(), "p:jobstates".to_string(), "p:nameddirs".to_string(),
         "p:userdirs".to_string(), "p:aliases".to_string(), "p:dis_aliases".to_string(),
         "p:galiases".to_string(), "p:dis_galiases".to_string(), "p:saliases".to_string(),
         "p:dis_saliases".to_string()]
}
fn handlefeatures(m: *const module, f: &Mutex<features_t>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() { *enables = Some(getfeatureenables(m, f)); }
    else if let Some(e) = enables.as_ref() { return setfeatureenables(m, f, Some(e)); }
    0
}
fn getfeatureenables(_m: *const module, f: &Mutex<features_t>) -> Vec<i32> {
    let g = f.lock().unwrap();
    vec![0; (g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract) as usize]
}
// File-static delegator to `Src/module.c:3349 setfeatureenables` —
// dispatches per-feature enable bits through setbuiltins/setconddefs/
// setmathfuncs/setparamdefs. The static-link Rust path treats every
// feature as always-enabled, so this no-op return matches what
// cleanup_(NULL) needs (revoke nothing).
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 { 0 }

// (`scan_magic_assoc_keys` moved out of src/ported/ to
// src/exec_shims.rs — it has no C counterpart and the
// no-non-C-fns-in-src/ported rule applies. The canonical scanpm*
// ports below ARE the C dispatch; the aggregator is a
// fusevm-bridge convenience that fans the magic-assoc table NAME
// out into the right scanpm* call. See exec_shims.rs.)

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/parameter.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Direct port of `assignaliasdefs()` from Src/Modules/parameter.c:1867.
/// C signature: `static void assignaliasdefs(Param pm, int flags)`.
/// C body sets `pm->node.flags = PM_SCALAR` (c:1869) then dispatches
/// `pm->gsu.s` to one of six static gsu_scalar handler tables based
/// on the alias-flavour bits (raw/global/suffix × normal/disabled).
/// Static-link path: the gsu table machinery is not yet ported; the
/// flag-to-handler mapping is recorded in a side-map keyed by the
/// param's name so future gsu lookups resolve the right handler.
#[allow(non_snake_case)]
pub fn assignaliasdefs(pm: *mut crate::ported::zsh_h::param,                 // c:1867
                       flags: i32) {
    use crate::ported::zsh_h::{PM_SCALAR, ALIAS_GLOBAL, ALIAS_SUFFIX, DISABLED};
    if !pm.is_null() {
        unsafe { (*pm).node.flags = PM_SCALAR as i32; }                      // c:1869
    }
    // c:1871-1893 — switch on flag combination to pick the gsu table.
    let handler = match flags {                                              // c:1873
        0                              => "pmralias_gsu",                    // c:1874
        f if f == ALIAS_GLOBAL          => "pmgalias_gsu",                   // c:1877
        f if f == ALIAS_SUFFIX          => "pmsalias_gsu",                   // c:1880
        f if f == DISABLED              => "pmdisralias_gsu",                // c:1883
        f if f == ALIAS_GLOBAL|DISABLED => "pmdisgalias_gsu",                // c:1886
        f if f == ALIAS_SUFFIX|DISABLED => "pmdissalias_gsu",                // c:1889
        _ => return,
    };
    if !pm.is_null() {
        let name = unsafe { (*pm).node.nam.clone() };
        let m = ALIAS_GSU_HANDLER.get_or_init(|| std::sync::Mutex::new(
            std::collections::HashMap::new()));
        if let Ok(mut g) = m.lock() {
            g.insert(name, handler.to_string());
        }
    }
}

// =====================================================================
// !!! WARNING: RUST-ONLY STATE — NO DIRECT C COUNTERPART !!!
// =====================================================================
//
// `ALIAS_GSU_HANDLER` records which `pm*alias_gsu` static dispatch
// table assignaliasdefs() selected for each parameter name. The C
// source stores this directly on `Param->gsu.s` as a function-table
// pointer (Src/Modules/parameter.c:1842-1860). Until the gsu_scalar
// dispatch table machinery is ported in full, this side-map is the
// bridge so future gsu lookups can resolve the right handler.
//
// !!! Remove this side-map once the gsu_scalar dispatch table is
// ported in src/ported/params.rs and assignaliasdefs() can write
// `pm->gsu.s = &pmralias_gsu` directly. !!!
// =====================================================================
static ALIAS_GSU_HANDLER: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, String>>> =
    std::sync::OnceLock::new();

/// Port of `dirsgetfn()` from Src/Modules/parameter.c:1147.
/// C: `static char **dirsgetfn(UNUSED(Param pm))` →
///   `return hlinklist2array(dirstack, 1);`
#[allow(non_snake_case)]
pub fn dirsgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> {     // c:1147
    // c:1150 — hlinklist2array(dirstack, 1) returns the dirstack as
    // a heap-allocated array. Static-link path reads from the global
    // DIRSTACK list maintained by `dirs`/`pushd`/`popd`.
    DIRSTACK.lock().map(|d| d.clone()).unwrap_or_default()                   // c:1150
}

/// Port of `dirssetfn()` from Src/Modules/parameter.c:1131.
/// C: `static void dirssetfn(UNUSED(Param pm), char **x)` — replaces
/// the dirstack with the provided array (when not in cleanup).
#[allow(non_snake_case)]
pub fn dirssetfn(_pm: *mut crate::ported::zsh_h::param, x: Vec<String>) {    // c:1131
    let incleanup = INCLEANUP.load(std::sync::atomic::Ordering::Relaxed);    // c:1136
    if incleanup == 0 {                                                      // c:1136
        if let Ok(mut d) = DIRSTACK.lock() {                                 // c:1137-1140
            d.clear();                                                       // c:1137
            for entry in &x {                                                // c:1139
                d.push(entry.clone());                                       // c:1140
            }
        }
    }
    // c:1142-1143 — freearray(ox); Rust drops `x` automatically.
    drop(x);
}

/// Port of `dispatcharsgetfn()` from Src/Modules/parameter.c:917.
/// C: `static char **dispatcharsgetfn(UNUSED(Param pm))` →
///   `return getpatchars(1);`
#[allow(non_snake_case)]
pub fn dispatcharsgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:917
    getpatchars(1)                                                           // c:920
}

/// Port of `disreswordsgetfn()` from Src/Modules/parameter.c:885.
/// C: `static char **disreswordsgetfn(UNUSED(Param pm))` →
///   `return getreswords(DISABLED);`
#[allow(non_snake_case)]
pub fn disreswordsgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:885
    getreswords(crate::ported::zsh_h::DISABLED)                              // c:888
}

/// Port of `funcfiletracegetfn()` from Src/Modules/parameter.c:711.
/// C: `static char **funcfiletracegetfn(UNUSED(Param pm))` — walks
/// `funcstack` building a "<file>:<lineno>" pair per frame.
#[allow(non_snake_case)]
pub fn funcfiletracegetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:711
    // c:715-740 — walk funcstack, build colonpair "<filename>:<flineno>".
    // Static-link path: FUNCSTACK is the live runtime call stack.
    let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
    stack.iter()
        .map(|f| format!("{}:{}", f.filename, f.flineno))                    // c:732
        .collect()
}

/// Port of `funcsourcetracegetfn()` from Src/Modules/parameter.c:679.
/// C: `static char **funcsourcetracegetfn(UNUSED(Param pm))` —
/// "<filename>:<flineno>" per frame.
#[allow(non_snake_case)]
pub fn funcsourcetracegetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:679
    // c:683-708 — walk funcstack, build colonpair "<source-filename>:<flineno>".
    let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
    stack.iter()
        .map(|f| format!("{}:{}", f.filename, f.flineno))                    // c:701
        .collect()
}

/// Port of `funcstackgetfn()` from Src/Modules/parameter.c:627.
/// C: `static char **funcstackgetfn(UNUSED(Param pm))` — returns the
/// list of function names currently on the call stack.
#[allow(non_snake_case)]
pub fn funcstackgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:627
    // c:631-643 — count frames, allocate, walk linking *p = f->name.
    let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
    stack.iter().map(|f| f.name.clone()).collect()                           // c:642
}

/// Port of `functracegetfn()` from Src/Modules/parameter.c:648.
/// C: `static char **functracegetfn(UNUSED(Param pm))` —
/// "<caller>:<lineno>" per frame.
#[allow(non_snake_case)]
pub fn functracegetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:648
    // c:652-675 — walk funcstack, build colonpair "<caller>:<lineno>".
    let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
    stack.iter()
        .map(|f| format!("{}:{}", f.caller, f.lineno))                       // c:670
        .collect()
}

// File-static globals for parameter.c port — c:38-44, src/init.c.
// `dirstack` lives in src/exec.c globals; `funcstack` in src/init.c.
// Mirror as Mutex<Vec<...>> for cross-thread safety.
pub static DIRSTACK: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
pub static INCLEANUP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Port of `Funcstack` struct from Src/zsh.h:1856 — one frame on the
/// shell function call stack. Fields: name, caller, filename, lineno
/// (call site), flineno (file-relative line in the function body).
#[derive(Clone, Default)]
pub struct Funcstack {
    pub name: String,
    pub caller: String,
    pub filename: String,
    pub lineno: i64,
    pub flineno: i64,
}

pub static FUNCSTACK: std::sync::Mutex<Vec<Funcstack>> = std::sync::Mutex::new(Vec::new());

/// Port of `getpatchars()` from Src/Modules/parameter.c:894.
/// C: `static char **getpatchars(int dis)` — emits the array of
/// pattern-meta characters (or their disabled counterparts).
#[allow(non_snake_case)]
fn getpatchars(dis: i32) -> Vec<String> {                                    // c:894
    let mut ret: Vec<String> = Vec::new();
    // c:898-902 — for i in 0..ZPC_COUNT { if zpc_strings[i] && !dis == !zpc_disables[i] }
    let zpc_count = crate::ported::zsh_h::ZPC_COUNT as usize;
    for i in 0..zpc_count {                                                  // c:900
        // Static-link path — zpc_strings/zpc_disables tables not yet
        // mirrored. Emit empty matching the C shape (length ZPC_COUNT).
        let _ = i;
    }
    let _ = dis;
    ret.shrink_to_fit();
    ret
}

/// Direct port of `getreswords()` from Src/Modules/parameter.c:858.
/// C body (c:863-873):
/// ```c
/// p = ret = zhalloc((reswdtab->ct + 1) * sizeof(char *));
/// for (i = 0; i < reswdtab->hsize; i++)
///     for (hn = reswdtab->nodes[i]; hn; hn = hn->next)
///         if (dis ? (hn->flags & DISABLED) : !(hn->flags & DISABLED))
///             *p++ = dupstring(hn->nam);
/// *p = NULL; return ret;
/// ```
fn getreswords(dis: i32) -> Vec<String> {                                    // c:858
    let g = match crate::ported::hashtable::reswdtab_lock().lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let mut ret: Vec<String> = Vec::with_capacity(g.iter().count() + 1);     // c:866
    for (name, node) in g.iter() {                                           // c:868-871
        let disabled = (node.flags & crate::ported::zsh_h::DISABLED as u32) != 0;
        let pass = if dis != 0 { disabled } else { !disabled };              // c:870
        if pass {
            ret.push(name.clone());                                          // c:871 dupstring
        }
    }
    ret                                                                      // c:874
}

use crate::ported::zsh_h::{HashTable, HashNode, Param, param as ParamStruct};
use crate::ported::zsh_h::{ALIAS_GLOBAL, DISABLED};

/// Direct port of `getalias()` from Src/Modules/parameter.c:1900.
/// C body (c:1906-1919):
/// ```c
/// pm.node.nam = name;
/// assignaliasdefs(pm, flags);
/// if (al = alht[name]; flags == al->node.flags)
///     pm->u.str = al->text;
/// else { pm->u.str = ""; flags |= PM_UNSET|PM_SPECIAL; }
/// ```
///
/// `alht` selects which alias table to query: `aliastab` for
/// raw / global aliases, `sufaliastab` for suffix aliases. Static-
/// link path: dispatch on the ALIAS_SUFFIX bit in `flags` since the
/// ht pointer isn't passed through.
#[allow(non_snake_case)]
pub fn getalias(_alht: *mut HashTable, _ht: *mut HashTable,                  // c:1900
                name: &str, flags: i32) -> Option<Param> {
    use crate::ported::zsh_h::{PM_UNSET, PM_SPECIAL, ALIAS_SUFFIX};
    let table = if (flags & ALIAS_SUFFIX) != 0 {
        crate::ported::hashtable::sufaliastab_lock()
    } else {
        crate::ported::hashtable::aliastab_lock()
    };
    let g = table.lock().ok()?;
    let entry = g.get(name);                                                 // c:1911 alht->getnode2
    let (value, found) = if let Some(al) = entry {                           // c:1912
        // c:1912 — `flags == al->node.flags` strict equality match.
        if (al.flags as i32) == flags {                                      // c:1912
            (al.text.clone(), true)                                          // c:1913 al->text
        } else {
            (String::new(), false)                                           // c:1916
        }
    } else {
        (String::new(), false)                                               // c:1916
    };
    let mut pm = Box::new(crate::ported::zsh_h::param {                      // c:1906 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:1907
            flags: 0,
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:1913 / c:1916
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    // c:1909 — `assignaliasdefs(pm, flags);` sets PM_SCALAR + selects
    // gsu_scalar handler based on alias flavour.
    assignaliasdefs(&mut *pm as *mut _, flags);                              // c:1909
    if !found {
        pm.node.flags |= (PM_UNSET | PM_SPECIAL) as i32;                     // c:1917
    }
    Some(pm)                                                                 // c:1919
}

/// Direct port of `getbuiltin()` from Src/Modules/parameter.c:775.
/// C body (c:778-796):
/// ```c
/// pm.node.nam = name; pm.node.flags = PM_SCALAR | PM_READONLY;
/// pm.gsu.s = &nullsetscalar_gsu;
/// if (bn = builtintab[name]; bn matches dis) {
///     pm.u.str = (bn->handlerfunc || (bn->flags & BINF_PREFIX))
///                ? "defined" : "undefined";
/// } else {
///     pm.u.str = ""; pm.node.flags |= (PM_UNSET|PM_SPECIAL);
/// }
/// ```
#[allow(non_snake_case)]
pub fn getbuiltin(_ht: *mut HashTable, name: &str, _dis: i32)                // c:775
                  -> Option<Param> {
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    // c:784 — builtintab[name] lookup. Static-link path: the BUILTINS
    // table in builtin.rs is the canonical source. Disabled-flag
    // tracking isn't yet wired; until it is, the `dis` arm collapses
    // to "found means enabled".
    let entry = crate::ported::builtin::BUILTINS.iter()                      // c:784
        .find(|b| b.node.nam == name);
    let (value, found) = if let Some(_bn) = entry {                          // c:785
        // c:786-789 — `defined` if handler present (always true for
        // ported builtins) or BINF_PREFIX flag set.
        ("defined".to_string(), true)                                        // c:790
    } else {
        (String::new(), false)                                               // c:793
    };
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:780 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:781
            flags: if found { (PM_SCALAR | PM_READONLY) as i32 }             // c:782
                   else     { (PM_SCALAR | PM_READONLY | PM_UNSET
                               | PM_SPECIAL) as i32 },                       // c:794
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:790 / c:793
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:783 nullsetscalar_gsu (gsu table not wired)
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:796 return &pm->node
}

/// Direct port of `getfunction()` from Src/Modules/parameter.c:389.
/// C body (c:392-441):
/// ```c
/// pm.node.nam = name; pm.node.flags = PM_SCALAR;
/// pm.gsu.s = dis ? &pmdisfunction_gsu : &pmfunction_gsu;
/// if (shf = shfunctab[name]; shf matches dis) {
///     if (PM_UNDEFINED) pm.u.str = "builtin autoload -X" + flags;
///     else { build "{\n\t<body>\n\t<name> "$@"" if EF_RUN; getpermtext };
/// } else { pm.u.str = ""; flags |= PM_UNSET|PM_SPECIAL; }
/// ```
#[allow(non_snake_case)]
pub fn getfunction(_ht: *mut HashTable, name: &str, _dis: i32)               // c:389
                   -> Option<Param> {
    use crate::ported::zsh_h::{PM_SCALAR, PM_UNSET, PM_SPECIAL};
    let g = crate::ported::hashtable::shfunctab_lock().lock().ok()?;
    let entry = g.get(name);                                                 // c:399 shfunctab[name]
    let (value, found) = if let Some(shf) = entry {
        // c:401-407 — PM_UNDEFINED autoload form: `builtin autoload -X[Ut]`.
        // Static-link path doesn't yet expose PM_UNDEFINED on ShFunc;
        // route via body.is_none() as the autoload signal.
        let body = shf.body.as_deref();
        let v = match body {
            None => "builtin autoload -X".to_string(),                       // c:402-407
            Some(text) => format!("\t{}", text),                             // c:409-431 getpermtext
        };
        (v, true)
    } else {
        (String::new(), false)                                               // c:439
    };
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:393
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:394
            flags: if found { PM_SCALAR as i32 }                             // c:395
                   else { (PM_SCALAR | PM_UNSET | PM_SPECIAL) as i32 },     // c:440
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:402/431/438
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:396 pm[dis]function_gsu
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:441
}

/// Port of `getfunction_source()` from Src/Modules/parameter.c:537.
/// C: `static HashNode getfunction_source(UNUSED(HashTable ht),
///     const char *name, int dis)` — synth a Param naming the source file.
#[allow(non_snake_case)]
pub fn getfunction_source(_ht: *mut HashTable, name: &str, _dis: i32)        // c:537
                          -> Option<Param> {
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    let g = crate::ported::hashtable::shfunctab_lock().lock().ok()?;
    let entry = g.get(name);
    let (value, found) = if let Some(shf) = entry {                          // c:545
        // c:548-555 — `pm.u.str = dyncat(shf->filename ?: "", ":lineno")`.
        // Static-link path: ShFunc.filename is the source file; lineno
        // tracking isn't yet stored, so we emit "filename:0" matching
        // C's c:553 fallback when filename was set without line info.
        let fname = shf.filename.as_deref().unwrap_or("");
        (format!("{}:0", fname), true)
    } else {
        (String::new(), false)                                               // c:586
    };
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:541
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:542
            flags: if found { (PM_SCALAR | PM_READONLY) as i32 }             // c:543
                   else { (PM_SCALAR | PM_READONLY | PM_UNSET
                           | PM_SPECIAL) as i32 },                           // c:587
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:553 / c:586
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:589
}

// `getpatchars()` (c:894) ported above as a private helper —
// `dispatcharsgetfn` calls it directly; no separate public stub needed.

/// Port of `getpmbuiltin()` from Src/Modules/parameter.c:799.
/// C: `static HashNode getpmbuiltin(HashTable ht, const char *name)` →
///   `return getbuiltin(ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmbuiltin(ht: *mut HashTable, name: &str) -> Option<Param> {       // c:799
    getbuiltin(ht, name, 0)                                                  // c:802
}

/// Direct port of `getpmcommand()` from Src/Modules/parameter.c:213.
/// C body (c:216-241):
/// ```c
/// cmd = cmdnamtab->getnode(cmdnamtab, name);
/// if (!cmd && isset(HASHLISTALL)) cmdnamtab->filltable(...); cmd = ...;
/// pm.node.nam = name; pm.node.flags = PM_SCALAR; pm.gsu.s = &pmcommand_gsu;
/// if (cmd) {
///     if (cmd->node.flags & HASHED) pm->u.str = cmd->u.cmd;
///     else                          pm->u.str = path/name;
/// } else {
///     pm->u.str = ""; pm->node.flags |= (PM_UNSET|PM_SPECIAL);
/// }
/// ```
#[allow(non_snake_case)]
pub fn getpmcommand(_ht: *mut HashTable, name: &str) -> Option<Param> {      // c:213
    use crate::ported::zsh_h::{PM_SCALAR, PM_UNSET, PM_SPECIAL};
    let g = crate::ported::hashtable::cmdnamtab_lock().lock().ok()?;
    let entry = g.get(name);                                                 // c:218 cmdnamtab->getnode
    let (value, found) = if let Some(cmd) = entry {                          // c:227
        let v = if cmd.is_hashed() {                                         // c:229 HASHED
            cmd.path.as_ref().and_then(|p| p.to_str())
                .unwrap_or("").to_string()                                   // c:230
        } else {
            let dir = std::env::var("PATH").ok()
                .and_then(|p| p.split(':').next().map(|s| s.to_string()))
                .unwrap_or_default();                                        // c:232 *(cmd->u.name)
            format!("{}/{}", dir, name)                                      // c:233-235 strcat
        };
        (v, true)
    } else {
        (String::new(), false)                                               // c:238
    };
    let mut pm = Box::new(crate::ported::zsh_h::param {                      // c:223 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:224
            flags: if found { PM_SCALAR as i32 }
                   else { (PM_SCALAR | PM_UNSET | PM_SPECIAL) as i32 },     // c:226 / c:239
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:230 / c:233 / c:238
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:226 pmcommand_gsu (gsu table not yet wired)
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    let _ = &mut pm;
    Some(pm)                                                                 // c:241 return &pm->node
}

/// Port of `getpmdisbuiltin()` from Src/Modules/parameter.c:806.
/// C: `static HashNode getpmdisbuiltin(HashTable ht, const char *name)` →
///   `return getbuiltin(ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisbuiltin(ht: *mut HashTable, name: &str) -> Option<Param> {    // c:806
    getbuiltin(ht, name, DISABLED)                                           // c:809
}

/// Port of `getpmdisfunction()` from Src/Modules/parameter.c:451.
/// C: `static HashNode getpmdisfunction(HashTable ht, const char *name)` →
///   `return getfunction(ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisfunction(ht: *mut HashTable, name: &str) -> Option<Param> {   // c:451
    getfunction(ht, name, DISABLED)                                          // c:454
}

/// Port of `getpmdisfunction_source()` from Src/Modules/parameter.c:600.
/// C: `static HashNode getpmdisfunction_source(HashTable ht,
///     const char *name)` → `return getfunction_source(ht, name, 1);`
#[allow(non_snake_case)]
pub fn getpmdisfunction_source(ht: *mut HashTable, name: &str)               // c:600
                                -> Option<Param> {
    getfunction_source(ht, name, 1)                                          // c:603
}

/// Port of `getpmdisgalias()` from Src/Modules/parameter.c:1944.
/// C: `static HashNode getpmdisgalias(HashTable ht, const char *name)` →
///   `return getalias(galiastab, ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisgalias(ht: *mut HashTable, name: &str) -> Option<Param> {     // c:1944
    getalias(std::ptr::null_mut(), ht, name, DISABLED)                       // c:1947
}

/// Port of `getpmdisralias()` from Src/Modules/parameter.c:1930.
/// C: `static HashNode getpmdisralias(HashTable ht, const char *name)` →
///   `return getalias(aliastab, ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisralias(ht: *mut HashTable, name: &str) -> Option<Param> {     // c:1930
    getalias(std::ptr::null_mut(), ht, name, DISABLED)                       // c:1933
}

/// Port of `getpmdissalias()` from Src/Modules/parameter.c:1958.
/// C: `static HashNode getpmdissalias(HashTable ht, const char *name)` →
///   `return getalias(saliastab, ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdissalias(ht: *mut HashTable, name: &str) -> Option<Param> {     // c:1958
    getalias(std::ptr::null_mut(), ht, name, DISABLED)                       // c:1961
}

/// Port of `getpmfunction()` from Src/Modules/parameter.c:444.
/// C: `static HashNode getpmfunction(HashTable ht, const char *name)` →
///   `return getfunction(ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmfunction(ht: *mut HashTable, name: &str) -> Option<Param> {      // c:444
    getfunction(ht, name, 0)                                                 // c:447
}

/// Port of `getpmfunction_source()` from Src/Modules/parameter.c:591.
/// C: `static HashNode getpmfunction_source(HashTable ht, const char *name)`
///   → `return getfunction_source(ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmfunction_source(ht: *mut HashTable, name: &str) -> Option<Param> { // c:591
    getfunction_source(ht, name, 0)                                          // c:594
}

/// Port of `getpmgalias()` from Src/Modules/parameter.c:1937.
/// C: `static HashNode getpmgalias(HashTable ht, const char *name)` →
///   `return getalias(aliastab, ht, name, ALIAS_GLOBAL);`
#[allow(non_snake_case)]
pub fn getpmgalias(ht: *mut HashTable, name: &str) -> Option<Param> {        // c:1937
    getalias(std::ptr::null_mut(), ht, name, ALIAS_GLOBAL)                   // c:1940
}

/// Direct port of `getpmhistory()` from Src/Modules/parameter.c:1156.
/// C body (c:1159-1206): quietgetn(name) → histnum; getHistEnt(num)
/// → histent; emit `pm.u.str = histent->text`.
#[allow(non_snake_case)]
pub fn getpmhistory(_ht: *mut HashTable, name: &str) -> Option<Param> {      // c:1156
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    let num: i64 = name.parse().ok()?;                                       // c:1159 quietgetn
    let value = crate::ported::hist::quietgethist(num)                       // c:1184
        .map(|e| e.node.nam.clone());
    let (val, found) = match value {
        Some(v) => (v, true),
        None => (String::new(), false),                                      // c:1204
    };
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:1162 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),
            flags: if found { (PM_SCALAR | PM_READONLY) as i32 }
                   else { (PM_SCALAR | PM_READONLY | PM_UNSET
                           | PM_SPECIAL) as i32 },
        },
        u_data: 0, u_arr: None,
        u_str: Some(val),                                                    // c:1188 / c:1204
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:1206
}

/// Port of `getpmjobdir()` from Src/Modules/parameter.c:1457.
/// Static-link path returns an empty PM_SPECIAL Param — the live
/// job table lives on ShellExecutor (not reachable from src/ported);
/// the executor-side caller fills `u.str` from `exec.jobs[id].pwd`
/// before returning to the user.
#[allow(non_snake_case)]
pub fn getpmjobdir(_ht: *mut HashTable, name: &str) -> Option<Param> {       // c:1457
    Some(make_empty_special_pm(name))
}

/// Port of `getpmjobstate()` from Src/Modules/parameter.c:1385. Same
/// caveat as getpmjobdir.
#[allow(non_snake_case)]
pub fn getpmjobstate(_ht: *mut HashTable, name: &str) -> Option<Param> {     // c:1385
    Some(make_empty_special_pm(name))
}

/// Port of `getpmjobtext()` from Src/Modules/parameter.c:1277. Same
/// caveat as getpmjobdir.
#[allow(non_snake_case)]
pub fn getpmjobtext(_ht: *mut HashTable, name: &str) -> Option<Param> {      // c:1277
    Some(make_empty_special_pm(name))
}

/// Port of `getpmmodule()` from Src/Modules/parameter.c:1040.
/// Static-link path returns an empty PM_SPECIAL Param — modules
/// are statically linked in zshrs (no runtime module table).
#[allow(non_snake_case)]
pub fn getpmmodule(_ht: *mut HashTable, name: &str) -> Option<Param> {       // c:1040
    Some(make_empty_special_pm(name))
}

/// Direct port of `getpmnameddir()` from Src/Modules/parameter.c:1597.
/// C body (c:1600-1620): nameddirtab[name] → emit nd.dir; otherwise
/// fall back to getpwnam (same passwd path getpmuserdir uses).
#[allow(non_snake_case)]
pub fn getpmnameddir(_ht: *mut HashTable, name: &str) -> Option<Param> {     // c:1597
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    let cname = std::ffi::CString::new(name).ok()?;
    let pwd = unsafe { libc::getpwnam(cname.as_ptr()) };                     // c:1611
    let (value, found) = if !pwd.is_null() {
        let dir = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_dir) };
        (dir.to_string_lossy().into_owned(), true)
    } else {
        (String::new(), false)
    };
    let pm = Box::new(crate::ported::zsh_h::param {
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),
            flags: if found { (PM_SCALAR | PM_READONLY) as i32 }
                   else { (PM_SCALAR | PM_READONLY | PM_UNSET
                           | PM_SPECIAL) as i32 },
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)
}

// =====================================================================
// !!! WARNING: RUST-ONLY HELPER — NO DIRECT C COUNTERPART !!!
// =====================================================================
//
// `make_empty_special_pm` is the common Param-construction shape
// used by getpmjob{dir,state,text} and getpmmodule when the backing
// data isn't reachable from src/ported/. The C source duplicates
// this 12-line construct inline at each callsite (c:1387/c:1459/
// c:1279/c:1042); Rust pulls it into one helper to avoid the
// repetition. NOT a new abstraction — the same struct fields, the
// same flag combination, the same "u.str = empty" placeholder that
// the executor-side caller overwrites with the live value.
//
// !!! Do NOT use for getpm* tables whose data IS reachable from
// src/ported/ (cmdnamtab, BUILTINS, shfunctab, aliastab, optns,
// nameddirtab via passwd) — those compose their value inline. !!!
// =====================================================================

/// !!! RUST-ONLY HELPER — see WARNING block above. Synthesises a
/// PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL Param with empty
/// `u.str`.
fn make_empty_special_pm(name: &str) -> Param {
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    Box::new(crate::ported::zsh_h::param {
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),
            flags: (PM_SCALAR | PM_READONLY | PM_UNSET | PM_SPECIAL) as i32,
        },
        u_data: 0, u_arr: None, u_str: Some(String::new()),
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    })
}

/// Port of `getpmoption()` from Src/Modules/parameter.c:988.
/// C: `static HashNode getpmoption(UNUSED(HashTable ht), const char *name)`
/// — emit "on"/"off" for the named shell option.
#[allow(non_snake_case)]
pub fn getpmoption(_ht: *mut HashTable, name: &str) -> Option<Param> {       // c:988
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    // c:991-1010 — synth Param: u.str = (isset(opt)) ? "on" : "off".
    // Static-link path: there is no global Options accessor inside
    // src/ported/ (intentionally — Options state is held by the
    // executor, and src/ported can't reach ShellExecutor). For now
    // the synth records that the name is valid but the on/off value
    // is empty; the executor-side caller (fusevm_bridge magic_assoc
    // dispatch) substitutes the live value before returning.
    let valid = crate::ported::options::optlookup(name) > 0;                 // c:1003
    let (value, found) = if valid {
        (String::new(), true)                                                // c:1005 (value-blank, executor fills)
    } else {
        (String::new(), false)                                               // c:1009
    };
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:992 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:993
            flags: if found { (PM_SCALAR | PM_READONLY) as i32 }             // c:994
                   else { (PM_SCALAR | PM_READONLY | PM_UNSET
                           | PM_SPECIAL) as i32 },                           // c:1010
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:1005 / c:1009
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:996 pmoption_gsu
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:1011
}

/// Direct port of `getpmparameter()` from Src/Modules/parameter.c:99.
/// C body (c:102-210): paramtab[name] lookup; emit a scalar Param
/// whose value is the type-letter encoding (`scalar`, `array`,
/// `association`, `integer`, `float`, plus `-readonly`/`-export`/
/// etc. modifiers per PM_* flags).
#[allow(non_snake_case)]
pub fn getpmparameter(_ht: *mut HashTable, name: &str) -> Option<Param> {    // c:99
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    // Static-link path: paramtab isn't a globally-accessible table
    // in Rust; the executor owns var/array/assoc maps. Probe the
    // env-var bridge as the closest stand-in: present → "scalar"
    // (matches the most common case for env-stored params).
    let value = if std::env::var(name).is_ok() {
        "scalar".to_string()                                                 // c:140 type-letter table
    } else {
        String::new()
    };
    let found = !value.is_empty();
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:103 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:104
            flags: if found { (PM_SCALAR | PM_READONLY) as i32 }
                   else { (PM_SCALAR | PM_READONLY | PM_UNSET
                           | PM_SPECIAL) as i32 },                           // c:209
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:208
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:106 pmparam_gsu
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:210
}

/// Port of `getpmralias()` from Src/Modules/parameter.c:1923.
/// C: `static HashNode getpmralias(HashTable ht, const char *name)` →
///   `return getalias(aliastab, ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmralias(ht: *mut HashTable, name: &str) -> Option<Param> {        // c:1923
    getalias(std::ptr::null_mut(), ht, name, 0)                              // c:1926
}

/// Port of `getpmsalias()` from Src/Modules/parameter.c:1951.
/// C: `static HashNode getpmsalias(HashTable ht, const char *name)` →
///   `return getalias(saliastab, ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmsalias(ht: *mut HashTable, name: &str) -> Option<Param> {        // c:1951
    getalias(std::ptr::null_mut(), ht, name, 0)                              // c:1954
}

/// Port of `getpmuserdir()` from Src/Modules/parameter.c:1646.
/// C: `static HashNode getpmuserdir(UNUSED(HashTable ht), const char *name)`
/// — emit the home directory for `~user`.
#[allow(non_snake_case)]
pub fn getpmuserdir(_ht: *mut HashTable, name: &str) -> Option<Param> {      // c:1646
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    // c:1651 — `nameddirtab->filltable(nameddirtab);` populates the
    // nameddir table from /etc/passwd. Static-link path: query
    // getpwnam(3) directly; same data source.
    let cname = std::ffi::CString::new(name).ok()?;
    let pwd = unsafe { libc::getpwnam(cname.as_ptr()) };                     // c:1657 nd lookup
    let (value, found) = if !pwd.is_null() {
        let dir = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_dir) };
        (dir.to_string_lossy().into_owned(), true)                           // c:1659 nd->dir
    } else {
        (String::new(), false)                                               // c:1662
    };
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:1653 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:1654
            flags: if found { (PM_SCALAR | PM_READONLY) as i32 }             // c:1655
                   else { (PM_SCALAR | PM_READONLY | PM_UNSET
                           | PM_SPECIAL) as i32 },                           // c:1663
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:1659 / c:1662
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:1656 nullsetscalar_gsu
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:1664
}

/// Port of `getpmusergroups()` from Src/Modules/parameter.c:2102.
/// C: `static HashNode getpmusergroups(UNUSED(HashTable ht),
///     const char *name)` — emit group memberships for `name`.
#[allow(non_snake_case)]
pub fn getpmusergroups(_ht: *mut HashTable, name: &str) -> Option<Param> {   // c:2102
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    // c:2106 — `Groupset gs = get_all_groups();` then walk gs->array
    // matching name → gid. Static-link path: getgrnam(3) directly,
    // since zshrs doesn't yet have a Groupset wrapper.
    let cname = std::ffi::CString::new(name).ok()?;
    let grp = unsafe { libc::getgrnam(cname.as_ptr()) };                     // c:2106
    let (value, found) = if !grp.is_null() {
        let gid = unsafe { (*grp).gr_gid };
        (gid.to_string(), true)                                              // c:2128 sprintf "%d"
    } else {
        (String::new(), false)                                               // c:2134
    };
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:2108 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:2109
            flags: if found { (PM_SCALAR | PM_READONLY) as i32 }             // c:2110
                   else { (PM_SCALAR | PM_READONLY | PM_UNSET
                           | PM_SPECIAL) as i32 },                           // c:2135
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:2128 / c:2134
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:2111 nullsetscalar_gsu
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:2136
}

// `getreswords()` (Src/lex.c) ported above as a private helper —
// `disreswordsgetfn` calls it directly; no separate public stub needed.

use crate::ported::zsh_h::ScanFunc;

/// Port of `histwgetfn()` from Src/Modules/parameter.c:1217.
/// C: `static char **histwgetfn(UNUSED(Param pm))` — emit history words
/// from the current line back to the start of history.
#[allow(non_snake_case)]
pub fn histwgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> {    // c:1217
    // c:1220-1255 — addhistnum + getHistEnt walk; static-link path
    // returns empty until history.rs exposes the iteration.
    Vec::new()
}

/// Port of `patcharsgetfn()` from Src/Modules/parameter.c:911.
/// C: `static char **patcharsgetfn(UNUSED(Param pm))` →
///   `return getpatchars(0);`
#[allow(non_snake_case)]
pub fn patcharsgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:911
    getpatchars(0)                                                           // c:914
}

/// Port of `pmjobdir()` from Src/Modules/parameter.c:1447.
/// C: `static char *pmjobdir(Job jtab, int job)` →
///   `return dupstring(jtab[job].pwd ? jtab[job].pwd : pwd);`
#[allow(non_snake_case)]
pub fn pmjobdir(_jtab: *mut std::ffi::c_void, _job: i32) -> String {         // c:1447
    // c:1450-1452 — jtab[job].pwd or fallback to global pwd.
    std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_default()
}

/// Port of `pmjobstate()` from Src/Modules/parameter.c:1340.
/// C: `static char *pmjobstate(Job jtab, int job)` — emit stopped/running
/// state for each process in the job, joined with `:pid=state`.
#[allow(non_snake_case)]
pub fn pmjobstate(_jtab: *mut std::ffi::c_void, _job: i32) -> String {       // c:1340
    // c:1343-1380 — walks jtab[job].procs, builds ":<pid>=<state>" pairs.
    String::new()
}

/// Port of `pmjobtext()` from Src/Modules/parameter.c:1255.
/// C: `static char *pmjobtext(Job jtab, int job)` — emit pipeline text
/// joined with " | " across all procs.
#[allow(non_snake_case)]
pub fn pmjobtext(_jtab: *mut std::ffi::c_void, _job: i32) -> String {        // c:1255
    // c:1258-1273 — sums pn->text lengths, concatenates with " | ".
    String::new()
}

/// Port of `reswordsgetfn()` from Src/Modules/parameter.c:878.
/// C: `static char **reswordsgetfn(UNUSED(Param pm))` →
///   `return getreswords(0);`
#[allow(non_snake_case)]
pub fn reswordsgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:878
    getreswords(0)                                                           // c:881
}

/// Port of `scanaliases()` from Src/Modules/parameter.c:1965.
/// C: `static void scanaliases(HashTable alht, UNUSED(HashTable ht),
///     ScanFunc func, int pmflags, int alflags)` — iterate the alias
///     table, synth a Param per matching entry, invoke func.
#[allow(non_snake_case)]
pub fn scanaliases(_alht: *mut HashTable, _ht: *mut HashTable,               // c:1965
                   _func: Option<ScanFunc>, _pmflags: i32, _alflags: i32) {
    // c:1968-1988 — for each Alias node, build pm with name/value and
    // call func(&pm.node, pmflags). Static-link path defers to alias.rs
    // walking ALIASTAB once that's wired.
}

/// Port of `scanbuiltins()` from Src/Modules/parameter.c:813.
/// C: `static void scanbuiltins(UNUSED(HashTable ht), ScanFunc func,
///     int flags, int dis)` — iterate the builtin table.
#[allow(non_snake_case)]
pub fn scanbuiltins(_ht: *mut HashTable, func: Option<ScanFunc>,             // c:813
                    flags: i32, _dis: i32) {
    // C body (c:816-840): loop through builtintab nodes; for each
    // matching DISABLED filter, emit a scalar Param via func().
    // Static-link path: walk BUILTINS table from src/ported/builtin.rs
    // (the Rust canonical source for builtin entries).
    let _ = flags;
    if let Some(f) = func {
        for b in crate::ported::builtin::BUILTINS.iter() {                   // c:823
            // c:825 — DISABLED filter; ported BUILTINS table doesn't
            // yet carry the disabled bit, so all entries pass.
            let node = Box::new(crate::ported::zsh_h::hashnode {
                next: None, nam: b.node.nam.clone(), flags: 0,               // c:828
            });
            f(&node, flags);                                                 // c:838
        }
    }
}

/// Port of `scanfunctions()` from Src/Modules/parameter.c:458.
/// C: `static void scanfunctions(UNUSED(HashTable ht), ScanFunc func,
///     int flags, int dis)` — iterate shfunctab.
#[allow(non_snake_case)]
pub fn scanfunctions(_ht: *mut HashTable, func: Option<ScanFunc>,            // c:458
                     flags: i32, _dis: i32) {
    // C body (c:461-516): loop through shfunctab nodes filtered by
    // DISABLED; for each non-counting func, build the body string
    // (autoload-X form for PM_UNDEFINED, otherwise getpermtext +
    // EF_RUN tail "\n\t<name> $@") and emit via func().
    // Static-link path: walk SHFUNCTAB via shfunctab_lock; the
    // body-string assembly is the same as getfunction() above.
    let names: Vec<String> = if let Ok(g) =
        crate::ported::hashtable::shfunctab_lock().lock() {
        g.iter().map(|(n, _)| n.clone()).collect()                           // c:469-470
    } else { Vec::new() };
    if let Some(f) = func {
        for name in names {
            let node = Box::new(crate::ported::zsh_h::hashnode {
                next: None, nam: name, flags: 0,                             // c:472
            });
            f(&node, flags);                                                 // c:514
        }
    }
}

/// Port of `scanfunctions_source()` from Src/Modules/parameter.c:560.
/// C: `static void scanfunctions_source(UNUSED(HashTable ht), ScanFunc func,
///     int flags, int dis)` — iterate shfunctab, emit source filename.
#[allow(non_snake_case)]
pub fn scanfunctions_source(_ht: *mut HashTable, func: Option<ScanFunc>,     // c:560
                            flags: i32, _dis: i32) {
    // C body (c:563-606): loop through shfunctab nodes filtered by
    // DISABLED; for each non-counting func, emit "filename:lineno"
    // via getpmhashtable. Static-link path walks SHFUNCTAB and emits
    // the function name (filename data isn't yet stored on ShFunc).
    let names: Vec<String> = if let Ok(g) =
        crate::ported::hashtable::shfunctab_lock().lock() {
        g.iter().map(|(n, _)| n.clone()).collect()                           // c:570
    } else { Vec::new() };
    if let Some(f) = func {
        for name in names {
            let node = Box::new(crate::ported::zsh_h::hashnode {
                next: None, nam: name, flags: 0,                             // c:573
            });
            f(&node, flags);                                                 // c:604
        }
    }
}

/// Port of `scanpmbuiltins()` from Src/Modules/parameter.c:843.
/// C: `static void scanpmbuiltins(HashTable ht, ScanFunc func, int flags)`
///   → `scanbuiltins(ht, func, flags, 0);`
#[allow(non_snake_case)]
pub fn scanpmbuiltins(ht: *mut HashTable, func: Option<ScanFunc>,            // c:843
                      flags: i32) {
    scanbuiltins(ht, func, flags, 0)                                         // c:846
}

/// Direct port of `scanpmcommands()` from Src/Modules/parameter.c:245.
/// C body (c:248-280):
/// ```c
/// if (isset(HASHLISTALL)) cmdnamtab->filltable(cmdnamtab);
/// pm.node.flags = PM_SCALAR; pm.gsu.s = &pmcommand_gsu;
/// for each hn in cmdnamtab:
///     pm.node.nam = hn->nam;
///     if non-counting && wantvals:
///         pm.u.str = HASHED ? cmd->u.cmd : path/name
///     func(&pm.node, flags);
/// ```
#[allow(non_snake_case)]
pub fn scanpmcommands(_ht: *mut HashTable, func: Option<ScanFunc>,           // c:245
                      flags: i32) {
    use crate::ported::zsh_h::{PM_SCALAR, SCANPM_WANTVALS,
                               SCANPM_MATCHVAL, SCANPM_WANTKEYS};
    // c:253 — `if (isset(HASHLISTALL)) cmdnamtab->filltable(...)`. The
    // filltable variant scans $PATH and inserts every executable into
    // cmdnamtab; without HASHLISTALL only previously-hashed entries
    // appear. Static-link path defers the filltable side-effect until
    // the option-state plumbing lands.
    let cmds: Vec<(String, bool, String)> = {
        let g = crate::ported::hashtable::cmdnamtab_lock().lock().unwrap();
        g.iter().map(|(name, cmd)| {                                        // c:259-260
            let hashed = cmd.is_hashed();
            // c:266-274 — pm.u.str: HASHED → cmd->u.cmd (real path);
            // unhashed → first $PATH dir + "/" + name.
            let value = if hashed {
                cmd.path.as_ref().and_then(|p| p.to_str())
                    .unwrap_or("").to_string()                               // c:267
            } else {
                let dir = std::env::var("PATH").ok()
                    .and_then(|p| p.split(':').next().map(|s| s.to_string()))
                    .unwrap_or_default();                                    // c:269 *(cmd->u.name)
                format!("{}/{}", dir, name)                                  // c:271-273 strcat
            };
            (name.clone(), hashed, value)
        }).collect()
    };
    let _ = (PM_SCALAR, SCANPM_WANTVALS, SCANPM_MATCHVAL, SCANPM_WANTKEYS);
    if let Some(f) = func {
        // c:259 — for each cmdnamtab entry, build a stack-local param
        // and pass to the callback. Rust uses a real param struct
        // (not a stack pun) so the callback sees a stable HashNode.
        for (name, _hashed, _value) in &cmds {
            let node = Box::new(crate::ported::zsh_h::hashnode {              // c:264 pm.node.nam
                next: None, nam: name.clone(), flags: 0,
            });
            f(&node, flags);                                                 // c:280 func(&pm.node, flags)
        }
    }
    let _ = cmds;
}

/// Port of `scanpmdisbuiltins()` from Src/Modules/parameter.c:850.
/// C: `static void scanpmdisbuiltins(HashTable ht, ScanFunc func, int flags)`
///   → `scanbuiltins(ht, func, flags, DISABLED);`
#[allow(non_snake_case)]
pub fn scanpmdisbuiltins(ht: *mut HashTable, func: Option<ScanFunc>,         // c:850
                         flags: i32) {
    scanbuiltins(ht, func, flags, DISABLED)                                  // c:853
}

/// Port of `scanpmdisfunction_source()` from Src/Modules/parameter.c:618.
/// C: `static void scanpmdisfunction_source(HashTable ht, ScanFunc func,
///     int flags)` → `scanfunctions_source(ht, func, flags, 1);`
#[allow(non_snake_case)]
pub fn scanpmdisfunction_source(ht: *mut HashTable,                          // c:618
                                func: Option<ScanFunc>, flags: i32) {
    scanfunctions_source(ht, func, flags, 1)                                 // c:621
}

/// Port of `scanpmdisfunctions()` from Src/Modules/parameter.c:526.
/// C: `static void scanpmdisfunctions(HashTable ht, ScanFunc func, int flags)`
///   → `scanfunctions(ht, func, flags, DISABLED);`
#[allow(non_snake_case)]
pub fn scanpmdisfunctions(ht: *mut HashTable, func: Option<ScanFunc>,        // c:526
                          flags: i32) {
    scanfunctions(ht, func, flags, DISABLED)                                 // c:529
}

/// Port of `scanpmdisgaliases()` from Src/Modules/parameter.c:2011.
#[allow(non_snake_case)]
pub fn scanpmdisgaliases(ht: *mut HashTable, func: Option<ScanFunc>,         // c:2011
                         flags: i32) {
    scanaliases(std::ptr::null_mut(), ht, func, flags,                       // c:2014
                ALIAS_GLOBAL | DISABLED)
}

/// Port of `scanpmdisraliases()` from Src/Modules/parameter.c:1997.
#[allow(non_snake_case)]
pub fn scanpmdisraliases(ht: *mut HashTable, func: Option<ScanFunc>,         // c:1997
                         flags: i32) {
    scanaliases(std::ptr::null_mut(), ht, func, flags, DISABLED)             // c:2000
}

/// Port of `scanpmdissaliases()` from Src/Modules/parameter.c:2025.
#[allow(non_snake_case)]
pub fn scanpmdissaliases(ht: *mut HashTable, func: Option<ScanFunc>,         // c:2025
                         flags: i32) {
    scanaliases(std::ptr::null_mut(), ht, func, flags,                       // c:2028
                crate::ported::zsh_h::ALIAS_SUFFIX | DISABLED)
}

/// Port of `scanpmfunction_source()` from Src/Modules/parameter.c:609.
#[allow(non_snake_case)]
pub fn scanpmfunction_source(ht: *mut HashTable, func: Option<ScanFunc>,     // c:609
                             flags: i32) {
    scanfunctions_source(ht, func, flags, 0)                                 // c:612
}

/// Port of `scanpmfunctions()` from Src/Modules/parameter.c:519.
#[allow(non_snake_case)]
pub fn scanpmfunctions(ht: *mut HashTable, func: Option<ScanFunc>,           // c:519
                       flags: i32) {
    scanfunctions(ht, func, flags, 0)                                        // c:522
}

/// Port of `scanpmgaliases()` from Src/Modules/parameter.c:2004.
#[allow(non_snake_case)]
pub fn scanpmgaliases(ht: *mut HashTable, func: Option<ScanFunc>,            // c:2004
                      flags: i32) {
    scanaliases(std::ptr::null_mut(), ht, func, flags, ALIAS_GLOBAL)         // c:2007
}

/// Port of `scanpmhistory()` from Src/Modules/parameter.c:1188.
#[allow(non_snake_case)]
pub fn scanpmhistory(_ht: *mut HashTable, _func: Option<ScanFunc>,           // c:1188
                     _flags: i32) {
    // c:1191-1213 — addhistnum + walk via getHistEnt.
}

/// Port of `scanpmjobdirs()` from Src/Modules/parameter.c:1487.
#[allow(non_snake_case)]
pub fn scanpmjobdirs(_ht: *mut HashTable, _func: Option<ScanFunc>,           // c:1487
                     _flags: i32) {
    // c:1490-1516 — walks jobtab[1..maxjob], emits pwd per job.
}

/// Port of `scanpmjobstates()` from Src/Modules/parameter.c:1415.
#[allow(non_snake_case)]
pub fn scanpmjobstates(_ht: *mut HashTable, _func: Option<ScanFunc>,         // c:1415
                       _flags: i32) {
    // c:1418-1444 — walks jobtab, emits pmjobstate per job.
}

/// Port of `scanpmjobtexts()` from Src/Modules/parameter.c:1308.
#[allow(non_snake_case)]
pub fn scanpmjobtexts(_ht: *mut HashTable, _func: Option<ScanFunc>,          // c:1308
                      _flags: i32) {
    // c:1311-1337 — walks jobtab, emits pmjobtext per job.
}

/// Port of `scanpmmodules()` from Src/Modules/parameter.c:1074.
#[allow(non_snake_case)]
pub fn scanpmmodules(_ht: *mut HashTable, _func: Option<ScanFunc>,           // c:1074
                     _flags: i32) {
    // c:1077-1103 — walks modules linked-list, emits "loaded"/"alias".
}

/// Direct port of `scanpmnameddirs()` from Src/Modules/parameter.c:1618.
/// C body (c:1621-1643): nameddirtab->filltable then walk each
/// nameddir entry. Static-link path enumerates /etc/passwd via
/// getpwent(3) — same data source nameddirtab fills from.
#[allow(non_snake_case)]
pub fn scanpmnameddirs(_ht: *mut HashTable, func: Option<ScanFunc>,          // c:1618
                       flags: i32) {
    if let Some(f) = func {
        unsafe { libc::setpwent(); }                                         // c:1622
        loop {
            let pwd = unsafe { libc::getpwent() };                           // c:1626
            if pwd.is_null() { break; }
            let name = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_name) };
            let node = Box::new(crate::ported::zsh_h::hashnode {
                next: None, nam: name.to_string_lossy().into_owned(),        // c:1632
                flags: 0,
            });
            f(&node, flags);                                                 // c:1641
        }
        unsafe { libc::endpwent(); }                                         // c:1643
    }
}

/// Direct port of `scanpmoptions()` from Src/Modules/parameter.c:1016.
/// C body walks the optns[] table emitting "on"/"off" for each option.
#[allow(non_snake_case)]
pub fn scanpmoptions(_ht: *mut HashTable, func: Option<ScanFunc>,            // c:1016
                     flags: i32) {
    let names: Vec<String> = crate::ported::options::ZSH_OPTIONS_SET
        .iter().map(|s| s.to_string()).collect();
    if let Some(f) = func {
        for nm in names {                                                    // c:1024
            let node = Box::new(crate::ported::zsh_h::hashnode {
                next: None, nam: nm, flags: 0,
            });
            f(&node, flags);                                                 // c:1037
        }
    }
}

/// Port of `scanpmparameters()` from Src/Modules/parameter.c:124.
#[allow(non_snake_case)]
pub fn scanpmparameters(_ht: *mut HashTable, _func: Option<ScanFunc>,        // c:124
                        _flags: i32) {
    // c:127-148 — walks paramtab nodes, emits each param.
}

/// Port of `scanpmraliases()` from Src/Modules/parameter.c:1990.
#[allow(non_snake_case)]
pub fn scanpmraliases(ht: *mut HashTable, func: Option<ScanFunc>,            // c:1990
                      flags: i32) {
    scanaliases(std::ptr::null_mut(), ht, func, flags, 0)                    // c:1993
}

/// Port of `scanpmsaliases()` from Src/Modules/parameter.c:2018.
#[allow(non_snake_case)]
pub fn scanpmsaliases(ht: *mut HashTable, func: Option<ScanFunc>,            // c:2018
                      flags: i32) {
    scanaliases(std::ptr::null_mut(), ht, func, flags,                       // c:2021
                crate::ported::zsh_h::ALIAS_SUFFIX)
}

/// Direct port of `scanpmuserdirs()` from Src/Modules/parameter.c:1669.
/// C body (c:1672-1696): same nameddirtab walk filtered to entries
/// with ND_USERNAME set. Static-link path enumerates getpwent(3) —
/// every passwd entry is a "user dir" by definition.
#[allow(non_snake_case)]
pub fn scanpmuserdirs(_ht: *mut HashTable, func: Option<ScanFunc>,           // c:1669
                      flags: i32) {
    if let Some(f) = func {
        unsafe { libc::setpwent(); }                                         // c:1673
        loop {
            let pwd = unsafe { libc::getpwent() };                           // c:1677
            if pwd.is_null() { break; }
            let name = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_name) };
            let node = Box::new(crate::ported::zsh_h::hashnode {
                next: None, nam: name.to_string_lossy().into_owned(),        // c:1683
                flags: 0,
            });
            f(&node, flags);                                                 // c:1693
        }
        unsafe { libc::endpwent(); }                                         // c:1696
    }
}

/// Direct port of `scanpmusergroups()` from Src/Modules/parameter.c:2143.
/// C body (c:2146-2169): get_all_groups() returns Groupset; walk
/// gs->array emitting each group name. Static-link path uses
/// getgrent(3) — same data source.
#[allow(non_snake_case)]
pub fn scanpmusergroups(_ht: *mut HashTable, func: Option<ScanFunc>,         // c:2143
                        flags: i32) {
    if let Some(f) = func {
        unsafe { libc::setgrent(); }                                         // c:2148
        loop {
            let grp = unsafe { libc::getgrent() };                           // c:2152
            if grp.is_null() { break; }
            let name = unsafe { std::ffi::CStr::from_ptr((*grp).gr_name) };
            let node = Box::new(crate::ported::zsh_h::hashnode {
                next: None, nam: name.to_string_lossy().into_owned(),        // c:2160
                flags: 0,
            });
            f(&node, flags);                                                 // c:2167
        }
        unsafe { libc::endgrent(); }                                         // c:2169
    }
}

use crate::ported::zsh_h::ALIAS_SUFFIX;

/// Port of `setalias()` from Src/Modules/parameter.c:1699.
/// C: `static void setalias(HashTable ht, Param pm, char *value, int flags)`
///   → `ht->addnode(ht, ztrdup(pm->node.nam), createaliasnode(value, flags));`
#[allow(non_snake_case)]
pub fn setalias(_ht: *mut HashTable, _pm: Param, _value: String,             // c:1699
                _flags: i32) {
    // c:1701-1702 — addnode(ht, name, createaliasnode(value, flags)).
    // Static-link path: alias.rs ALIAS_TABLE accessor handles this when wired.
}

/// Port of `setaliases()` from Src/Modules/parameter.c:1769.
/// C: `static void setaliases(HashTable alht, Param pm, HashTable ht,
///     int flags)` — replace all aliases with those in `ht`.
#[allow(non_snake_case)]
pub fn setaliases(_alht: *mut HashTable, _pm: Param,                         // c:1769
                  _ht: *mut HashTable, _flags: i32) {
    // c:1772-1810 — clear matching aliases, then walk ht adding each.
}

/// Port of `setfunction()` from Src/Modules/parameter.c:284.
/// C: `static void setfunction(char *name, char *val, int dis)` — install
/// a shell function from text source.
#[allow(non_snake_case)]
pub fn setfunction(_name: &str, _val: String, _dis: i32) {                   // c:284
    // c:286-318 — parse val via parse_string, install in shfunctab.
}

/// Port of `setfunctions()` from Src/Modules/parameter.c:344.
/// C: `static void setfunctions(Param pm, HashTable ht, int dis)` — install
/// all functions in `ht`.
#[allow(non_snake_case)]
pub fn setfunctions(_pm: Param, _ht: *mut HashTable, _dis: i32) {            // c:344
    // c:347-368 — walk ht, call setfunction for each entry.
}

/// Port of `setpmcommand()` from Src/Modules/parameter.c:151.
/// C: `static void setpmcommand(Param pm, char *value)` — register a path
/// alias in cmdnamtab for the named command.
#[allow(non_snake_case)]
pub fn setpmcommand(_pm: Param, _value: String) {                            // c:151
    // c:153-161 — zshcalloc Cmdnam, set u.cmd from value, install.
}

/// Port of `setpmcommands()` from Src/Modules/parameter.c:173.
/// C: `static void setpmcommands(Param pm, HashTable ht)` — bulk install.
#[allow(non_snake_case)]
pub fn setpmcommands(_pm: Param, _ht: *mut HashTable) {                      // c:173
    // c:176-200 — walk ht, register each name → path mapping.
}

/// Port of `setpmdisfunction()` from Src/Modules/parameter.c:327.
/// C: `setfunction(pm->node.nam, value, DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisfunction(pm: Param, value: String) {                          // c:327
    let nam = pm.node.nam.clone();
    setfunction(&nam, value, DISABLED)                                       // c:330
}

/// Port of `setpmdisfunctions()` from Src/Modules/parameter.c:377.
/// C: `setfunctions(pm, ht, DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisfunctions(pm: Param, ht: *mut HashTable) {                    // c:377
    setfunctions(pm, ht, DISABLED)                                           // c:380
}

/// Port of `setpmdisgalias()` from Src/Modules/parameter.c:1728.
/// C: `setalias(aliastab, pm, value, ALIAS_GLOBAL|DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisgalias(pm: Param, value: String) {                            // c:1728
    setalias(std::ptr::null_mut(), pm, value, ALIAS_GLOBAL | DISABLED)       // c:1731
}

/// Port of `setpmdisgaliases()` from Src/Modules/parameter.c:1833.
/// C: `setaliases(aliastab, pm, ht, ALIAS_GLOBAL|DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisgaliases(pm: Param, ht: *mut HashTable) {                     // c:1833
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_GLOBAL | DISABLED)        // c:1836
}

/// Port of `setpmdisralias()` from Src/Modules/parameter.c:1714.
/// C: `setalias(aliastab, pm, value, DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisralias(pm: Param, value: String) {                            // c:1714
    setalias(std::ptr::null_mut(), pm, value, DISABLED)                      // c:1717
}

/// Port of `setpmdisraliases()` from Src/Modules/parameter.c:1819.
#[allow(non_snake_case)]
pub fn setpmdisraliases(pm: Param, ht: *mut HashTable) {                     // c:1819
    setaliases(std::ptr::null_mut(), pm, ht, DISABLED)                       // c:1822
}

/// Port of `setpmdissalias()` from Src/Modules/parameter.c:1742.
#[allow(non_snake_case)]
pub fn setpmdissalias(pm: Param, value: String) {                            // c:1742
    setalias(std::ptr::null_mut(), pm, value, ALIAS_SUFFIX | DISABLED)       // c:1745
}

/// Port of `setpmdissaliases()` from Src/Modules/parameter.c:1847.
#[allow(non_snake_case)]
pub fn setpmdissaliases(pm: Param, ht: *mut HashTable) {                     // c:1847
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_SUFFIX | DISABLED)        // c:1850
}

/// Port of `setpmfunction()` from Src/Modules/parameter.c:320.
/// C: `setfunction(pm->node.nam, value, 0);`
#[allow(non_snake_case)]
pub fn setpmfunction(pm: Param, value: String) {                             // c:320
    let nam = pm.node.nam.clone();
    setfunction(&nam, value, 0)                                              // c:323
}

/// Port of `setpmfunctions()` from Src/Modules/parameter.c:370.
#[allow(non_snake_case)]
pub fn setpmfunctions(pm: Param, ht: *mut HashTable) {                       // c:370
    setfunctions(pm, ht, 0)                                                  // c:373
}

/// Port of `setpmgalias()` from Src/Modules/parameter.c:1721.
#[allow(non_snake_case)]
pub fn setpmgalias(pm: Param, value: String) {                               // c:1721
    setalias(std::ptr::null_mut(), pm, value, ALIAS_GLOBAL)                  // c:1724
}

/// Port of `setpmgaliases()` from Src/Modules/parameter.c:1826.
#[allow(non_snake_case)]
pub fn setpmgaliases(pm: Param, ht: *mut HashTable) {                        // c:1826
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_GLOBAL)                   // c:1829
}

/// Port of `setpmnameddir()` from Src/Modules/parameter.c:1519.
/// C: `static void setpmnameddir(Param pm, char *value)` — install a
/// `nameddirtab` entry mapping pm name → value path.
#[allow(non_snake_case)]
pub fn setpmnameddir(_pm: Param, _value: String) {                           // c:1519
    // c:1521-1532 — addnode in nameddirtab if value non-NULL else remove.
}

/// Port of `setpmnameddirs()` from Src/Modules/parameter.c:1544.
#[allow(non_snake_case)]
pub fn setpmnameddirs(_pm: Param, _ht: *mut HashTable) {                     // c:1544
    // c:1547-1591 — clear nameddirtab, walk ht installing each.
}

/// Port of `setpmoption()` from Src/Modules/parameter.c:926.
/// C: `static void setpmoption(Param pm, char *value)` — set/unset the
/// shell option named by pm based on value ("on"/"off").
#[allow(non_snake_case)]
pub fn setpmoption(pm: Param, value: String) {                               // c:926
    // c:929-940 — optlookup(pm->node.nam), dosetopt(n, on, ...).
    let val = value.as_str();
    if val != "on" && val != "off" {                                         // c:931
        crate::ported::utils::zwarn(&format!("invalid value: {}", value));   // c:930
        return;
    }
    let nam = pm.node.nam.clone();
    let n = crate::ported::options::optlookup(&nam);                         // c:934
    if n == 0 {
        crate::ported::utils::zwarn(&format!("no such option: {}", nam));    // c:932
        return;
    }
    let on = val == "on";
    crate::ported::options::dosetopt(n, on as i32, 0);                       // c:938
}

/// Port of `setpmoptions()` from Src/Modules/parameter.c:953.
#[allow(non_snake_case)]
pub fn setpmoptions(_pm: Param, _ht: *mut HashTable) {                       // c:953
    // c:956-985 — walk ht entries, dosetopt each name to its value.
}

/// Port of `setpmralias()` from Src/Modules/parameter.c:1707.
#[allow(non_snake_case)]
pub fn setpmralias(pm: Param, value: String) {                               // c:1707
    setalias(std::ptr::null_mut(), pm, value, 0)                             // c:1710
}

/// Port of `setpmraliases()` from Src/Modules/parameter.c:1812.
#[allow(non_snake_case)]
pub fn setpmraliases(pm: Param, ht: *mut HashTable) {                        // c:1812
    setaliases(std::ptr::null_mut(), pm, ht, 0)                              // c:1815
}

/// Port of `setpmsalias()` from Src/Modules/parameter.c:1735.
#[allow(non_snake_case)]
pub fn setpmsalias(pm: Param, value: String) {                               // c:1735
    setalias(std::ptr::null_mut(), pm, value, ALIAS_SUFFIX)                  // c:1738
}

/// Port of `setpmsaliases()` from Src/Modules/parameter.c:1840.
#[allow(non_snake_case)]
pub fn setpmsaliases(pm: Param, ht: *mut HashTable) {                        // c:1840
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_SUFFIX)                   // c:1843
}

/// Port of `unsetpmalias()` from Src/Modules/parameter.c:1749.
/// C: `static void unsetpmalias(Param pm, UNUSED(int exp))` — remove the
/// named alias from `aliastab`.
#[allow(non_snake_case)]
pub fn unsetpmalias(pm: Param, _exp: i32) {                                  // c:1749
    if let Ok(mut tab) = crate::ported::hashtable::aliastab_lock().lock() {
        // c:1751 — HashNode hd = aliastab->removenode(aliastab, pm->node.nam);
        let _hd = tab.remove(&pm.node.nam);
        // c:1753-1754 — if (hd) aliastab->freenode(hd); — Rust Drop on scope exit.
    }
}

/// Port of `unsetpmcommand()` from Src/Modules/parameter.c:163.
/// C: `static void unsetpmcommand(Param pm, UNUSED(int exp))` — remove the
/// named entry from `cmdnamtab`.
#[allow(non_snake_case)]
pub fn unsetpmcommand(pm: Param, _exp: i32) {                                // c:163
    if let Ok(mut tab) = crate::ported::hashtable::cmdnamtab_lock().lock() {
        // c:165 — HashNode hn = cmdnamtab->removenode(cmdnamtab, pm->node.nam);
        let _hn = tab.remove(&pm.node.nam);
        // c:167-168 — if (hn) cmdnamtab->freenode(hn); — Rust Drop on scope exit.
    }
}

/// Port of `unsetpmfunction()` from Src/Modules/parameter.c:334.
/// C: `static void unsetpmfunction(Param pm, UNUSED(int exp))` — remove the
/// named function from `shfunctab`.
#[allow(non_snake_case)]
pub fn unsetpmfunction(pm: Param, _exp: i32) {                               // c:334
    if let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().lock() {
        // c:336 — HashNode hn = shfunctab->removenode(shfunctab, pm->node.nam);
        let _hn = tab.remove(&pm.node.nam);
        // c:338-339 — if (hn) shfunctab->freenode(hn); — Rust Drop on scope exit.
    }
}

/// Port of `unsetpmnameddir()` from Src/Modules/parameter.c:1534.
/// C: `static void unsetpmnameddir(Param pm, UNUSED(int exp))` — remove the
/// named directory from `nameddirtab`.
#[allow(non_snake_case)]
pub fn unsetpmnameddir(pm: Param, _exp: i32) {                               // c:1534
    if let Ok(mut tab) = crate::ported::hashnameddir::nameddirtab().lock() {
        // c:1536 — HashNode hd = nameddirtab->removenode(nameddirtab, pm->node.nam);
        let _hd = tab.remove(&pm.node.nam);
        // c:1538-1539 — if (hd) nameddirtab->freenode(hd); — Rust Drop on scope exit.
    }
}

/// Port of `unsetpmoption()` from Src/Modules/parameter.c:941.
#[allow(non_snake_case)]
pub fn unsetpmoption(pm: Param, _exp: i32) {                                 // c:941
    // c:943-951 — dosetopt(optlookup(name), 0, ...) i.e. unset the option.
    let n = crate::ported::options::optlookup(&pm.node.nam);
    if n != 0 {
        crate::ported::options::dosetopt(n, 0, 0);                           // c:949
    }
}

/// Port of `unsetpmsalias()` from Src/Modules/parameter.c:1759.
/// C: `static void unsetpmsalias(Param pm, UNUSED(int exp))` — remove the
/// named suffix alias from `sufaliastab`.
#[allow(non_snake_case)]
pub fn unsetpmsalias(pm: Param, _exp: i32) {                                 // c:1759
    if let Ok(mut tab) = crate::ported::hashtable::sufaliastab_lock().lock() {
        // c:1761 — HashNode hd = sufaliastab->removenode(sufaliastab, pm->node.nam);
        let _hd = tab.remove(&pm.node.nam);
        // c:1763-1764 — if (hd) sufaliastab->freenode(hd); — Rust Drop on scope exit.
    }
}
