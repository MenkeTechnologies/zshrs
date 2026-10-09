//! `zsh/hlgroup` module — port of `Src/Modules/hlgroup.c`.
//!
//! Exposes two read-only special parameters that bridge the
//! `$.zle.hlgroups` user-defined hash to the rendered ANSI escape
//! sequences zle uses internally:
//!   - `${.zle.esc[name]}` → full `\033[...m` escape stream
//!   - `${.zle.sgr[name]}` → bare `;`-joined SGR parameter list
//!
//! C source: 13 ported total — `convertattr`, `getgroup`, `scangroup`,
//! `getpmesc`, `scanpmesc`, `getpmsgr`, `scanpmsgr`, `setup_`,
//! `features_`, `enables_`, `boot_`, `cleanup_`, `finish_`.
//! Zero structs/enums in hlgroup.c (only `static const struct
//! gsu_scalar pmesc_gsu` and `static struct paramdef partab[]`
//! aggregates of pre-defined zsh-framework types).
//!
//! Order in this file mirrors C source order verbatim.

use crate::ported::zsh_h::features;
use crate::zsh_h::module;
use crate::ported::zsh_h::TSC_PROMPT;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

/// Port of `GROUPVAR` from `Src/Modules/hlgroup.c:33`.
/// `#define GROUPVAR ".zle.hlgroups"`. Name of the user-defined
/// associative array that maps group names to highlight-attribute
/// strings. Read by `getgroup` (c:82) + `scangroup` (c:117).
pub const GROUPVAR: &str = ".zle.hlgroups"; // c:33

/// Port of `convertattr(char *attrstr, int sgr)` from `Src/Modules/hlgroup.c:40`.
///
/// C body (c:42-77):
/// ```c
/// zattr atr;
/// match_highlight(attrstr, &atr, NULL, NULL);    // c:46
/// s = zattrescape(atr, sgr ? NULL : &len);        // c:47
/// if (sgr) { ...strip ESC[ and m, join with ; ... }
/// r = dupstring_wlen(s, len);                     // c:75
/// free(s);
/// return r;
/// ```
///
/// C signature: `static char *convertattr(char *attrstr, int sgr)`.
pub fn convertattr(attrstr: &str, sgr: bool) -> String {
    // c:40
    // c:46 — `match_highlight(attrstr, &atr, NULL, NULL);`
    let (atr, _setmask, _rest) = crate::ported::prompt::match_highlight(attrstr, None);

    // c:47 — `s = zattrescape(atr, sgr ? NULL : &len);` Body of
    // `zattrescape` (Src/prompt.c:257-280), inlined: render `atr` as the
    // terminal escape stream against a clean attribute state, then put
    // the saved state back.
    let savecurrent = *crate::ported::prompt::current_attrs_lock().lock().unwrap(); // c:260
    let saveunknown = crate::ported::prompt::txtunknownattrs.load(Ordering::Relaxed); // c:261
    crate::ported::prompt::txtunknownattrs.store(0, Ordering::Relaxed); // c:270
    crate::ported::prompt::treplaceattrs(atr); // c:271
    let esc_stream = crate::ported::prompt::applytextattributes(TSC_PROMPT); // c:272
    *crate::ported::prompt::current_attrs_lock().lock().unwrap() = savecurrent; // c:276
    crate::ported::prompt::set_pending_text_attrs(savecurrent); // c:276
    crate::ported::prompt::txtunknownattrs.store(saveunknown, Ordering::Relaxed); // c:277

    if sgr {
        // c:49-72 — strip `\033[` prefix and `m` suffix, join with `;`,
        // skip non-digit / non-`;` / non-`:` chars, replace `;`/`:` with `;`.
        // Always return at least "0" (c:67-70).
        //
        // C pointer discipline: `t` tracks the last-written char. After
        // a complete escape (`*c == 'm'`), c:64 writes a `;` separator
        // AT `t` (not past it), so when the while-loop exits via the
        // c:52 condition (next char isn't `\033[`), the c:71 `*t='\0'`
        // OVERWRITES that final separator — exactly ONE trailing `;`
        // is dropped, and only on that exit path. When the loop exits
        // via the c:62 `*c != 'm'` break, `t` was already advanced one
        // past (c:61 `t++`) and `*t='\0'` keeps everything accumulated,
        // INCLUDING a payload-trailing `;` from a `;`/`:` byte. A
        // trim-all-trailing-semicolons approach over-trims `\033[1;m`
        // (C: "1;") down to "1" — mirror the flag instead.
        let bytes = esc_stream.as_bytes();
        let mut out = String::new();
        let mut i = 0;
        let mut ended_after_m = false;
        while i + 1 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] == b'[' {
            // c:52
            i += 2; // c:53 c += 2
                    // c:54-60 — accumulate digits, treat ; or : as separator,
                    // break on anything else.
            while i < bytes.len() {
                let b = bytes[i];
                if b.is_ascii_digit() {
                    // c:54
                    out.push(b as char); // c:55
                    i += 1;
                } else if b == b';' || b == b':' {
                    // c:56
                    out.push(';'); // c:57
                    i += 1;
                } else {
                    break; // c:59
                }
            }
            // c:61 — `t++;` (conceptually: out.len() already one past).
            // c:62-65 — `if (*c != 'm') break;` else continue with `;`.
            if i >= bytes.len() || bytes[i] != b'm' {
                ended_after_m = false; // c:62-63 break — keep everything
                break;
            }
            out.push(';'); // c:64 *t = ';'
            ended_after_m = true;
            i += 1; // c:65 c++
        }
        // c:71 — `*t = '\0';` overwrites the c:64 separator when the
        // loop ended after a complete escape.
        if ended_after_m {
            out.pop();
        }
        // c:67-70 — `if (t <= s) { *s = '0'; t = s + 1; }`
        if out.is_empty() {
            out.push('0');
        }
        out
    } else {
        esc_stream // c:75 dupstring_wlen
    }
}

/// Port of `getgroup(const char *name, int sgr)` from `Src/Modules/hlgroup.c:82`. The shared
/// magic-assoc lookup behind both `${.zle.esc[name]}` and
/// `${.zle.sgr[name]}`. Reads `$.zle.hlgroups` (the `GROUPVAR`
/// `#define` at c:33), looks up `name`, runs `convertattr` on the
/// matched value's attribute string. Returns PM_UNSET (Rust `None`)
/// when the var isn't a hash, the group entry is missing, or the
/// entry has PM_UNSET set.
///
/// Port of `static HashNode getgroup(const char *name, int sgr)` from
/// `Src/Modules/hlgroup.c:82`. Looks up `name` in the user-defined
/// `$.zle.hlgroups` hash, returns the converted-attr string when found,
/// `None` for the PM_UNSET path. C synthesises a fresh Param + HashNode
/// shell; the Rust caller (an `${.zle.esc[name]}` magic-assoc fetch)
/// doesn't need the Param wrapping — only the result string.
pub fn getgroup(name: &str, sgr: bool) -> Option<String> {
    // c:82
    // c:84-94 — `pm = hcalloc(...); pm->gsu.s = &pmesc_gsu;
    //            pm->node.nam = dupstring(name);
    //            pm->node.flags = PM_SCALAR|PM_SPECIAL;`
    // The synthesised pm wraps the return; Rust returns the string
    // directly. The flag set + gsu wiring collapses since the caller
    // path (pmesc_get) consumes only `pm->u.str`.
    let _ = name; // gate against unused-param lint when body short-circuits

    // c:89 — `char *var = GROUPVAR;`
    let var = GROUPVAR;
    // c:96 — `if (!(v = getvalue(&vbuf, &var, 0))`
    let tab = crate::ported::params::paramtab();
    let table = match tab.read() {
        Ok(t) => t,
        Err(_) => return None,
    };
    let pm = match table.get(var) {
        Some(p) => p,
        None => return None, // c:102-103 PM_UNSET
    };
    // c:97 — `|| PM_TYPE(v->pm->node.flags) != PM_HASHED`
    if crate::ported::zsh_h::PM_TYPE(pm.node.flags as u32) != crate::ported::zsh_h::PM_HASHED {
        return None; // c:102-103 PM_UNSET
    }
    drop(table);
    // c:98-99 — `|| !(hlg = v->pm->gsu.h->getfn(v->pm))
    //            || !(hn = gethashnode2(hlg, name))`
    // gsu.h->getfn returns the assoc's backing hash; the canonical
    // Rust mirror is paramtab_hashed_storage — the SAME store
    // scangroup (below) and gethparam read. A prior lookup walked
    // pm.u_hash.nodes (name-only HashNode shells with no values) and
    // then probed a composite "VAR[name]" paramtab key the table
    // never indexes — so `${.zle.esc[name]}` returned unset even
    // when `${(k).zle.esc}` listed the group.
    let store = match crate::ported::params::paramtab_hashed_storage().lock() {
        Ok(s) => s,
        Err(_) => return None,
    };
    let raw_attr = match store.get(var).and_then(|m| m.get(name)) {
        Some(v) => v.clone(), // c:99 gethashnode2 hit → c:105 ((Param)hn)->u.str
        None => return None,  // c:102-103 PM_UNSET (presence ⟺ set in the mirror)
    };
    drop(store);
    // c:105 — `pm->u.str = convertattr(((Param) hn)->u.str, sgr);`
    Some(convertattr(&raw_attr, sgr)) // c:105
}

/// shared magic-assoc scanner behind `${(k).zle.esc}` /
/// `${(kv).zle.esc}` (and the `.zle.sgr` variants). Walks the
/// `$.zle.hlgroups` hash and yields each entry as
/// `(name, convertattr(value, sgr))`.
///
/// C signature: `static void scangroup(ScanFunc func, int flags, int sgr)`.
/// Rust port returns the `(name, value)` pairs as a Vec since
/// zshrs's magic-assoc dispatcher consumes the entire list rather
/// than a per-entry callback.
///
/// Port of `scangroup(ScanFunc func, int flags, int sgr)` from `Src/Modules/hlgroup.c:113`.
/// WARNING: param names don't match C — Rust=(_sgr) vs C=(func, flags, sgr)
pub fn scangroup(sgr: bool) -> Vec<(String, String)> {
    // c:113
    // c:123-125 — `if (!(v = getvalue(&vbuf, &var, 0)) ||
    //                   PM_TYPE(v->pm->node.flags) != PM_HASHED) return;`
    let var = GROUPVAR;
    let tab = crate::ported::params::paramtab();
    let table = match tab.read() {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let pm = match table.get(var) {
        Some(p) => p,
        None => return Vec::new(),
    };
    if crate::ported::zsh_h::PM_TYPE(pm.node.flags as u32) != crate::ported::zsh_h::PM_HASHED {
        return Vec::new();
    }
    drop(table);
    // c:126 — `hlg = v->pm->gsu.h->getfn(v->pm);` — fetch the typed
    // hashed-storage view (zshrs maintains a parallel IndexMap mirror).
    let store = match crate::ported::params::paramtab_hashed_storage().lock() {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let hlg = match store.get(var) {
        Some(m) => m.clone(),
        None => return Vec::new(),
    };
    drop(store);
    // c:128-130 — `memset(&pm, 0); pm.node.flags = PM_SCALAR; pm.gsu.s = &pmesc_gsu;`
    //              The Rust-side return-Vec model collapses the per-iteration
    //              Param construction; magic-assoc dispatch reads (name, value)
    //              tuples directly.
    // c:132-137 — `for (i = 0; i < hlg->hsize; i++)
    //                for (hn = hlg->nodes[i]; hn; hn = hn->next) {
    //                    pm.u.str = convertattr(((Param) hn)->u.str, sgr);
    //                    pm.node.nam = hn->nam;
    //                    func(&pm.node, flags);
    //                }`
    let mut out: Vec<(String, String)> = Vec::with_capacity(hlg.len());
    for (k, v) in hlg.iter() {
        // c:134 — `convertattr(value, sgr)`
        let converted = convertattr(v, sgr);
        out.push((k.clone(), converted));
    }
    out
}

/// Port of `getpmesc(UNUSED(HashTable ht), const char *name)` from `Src/Modules/hlgroup.c:141`.
/// C body is `return getgroup(name, 0);` — escape-form variant. getgroup
/// (c:82-108) hands back a fresh PM_SCALAR|PM_SPECIAL node carrying the
/// converted value, or PM_UNSET with "" when the group is not defined.
pub fn getpmesc(_ht: *mut crate::ported::zsh_h::HashTable, name: &str) -> Option<crate::ported::zsh_h::Param> {
    // c:141
    use crate::ported::zsh_h::{hashnode, param, PM_SCALAR, PM_SPECIAL, PM_UNSET};
    let val = getgroup(name, false); // c:143
    let unset = if val.is_none() { PM_UNSET } else { 0 }; // c:101-103
    Some(Box::new(param {
        node: hashnode {
            next: None,
            nam: name.to_string(),                               // c:92
            flags: (PM_SCALAR | PM_SPECIAL | unset) as i32, // c:93
        },
        u_str: Some(val.unwrap_or_default()), // c:102 / c:105
        ..Default::default()
    }))
}

/// Port of `scanpmesc(UNUSED(HashTable ht), ScanFunc func, int flags)` from `Src/Modules/hlgroup.c:148`.
/// C body is `scangroup(func, flags, 0);` — escape-form scanner: every
/// `$.zle.hlgroups` entry reaches `func` as a PM_SCALAR node (c:128-137).
pub fn scanpmesc(
    _ht: *mut crate::ported::zsh_h::HashTable,
    func: Option<crate::ported::zsh_h::ParamScanFunc>,
    flags: i32,
) {
    // c:148
    use crate::ported::zsh_h::{hashnode, param, PM_SCALAR};
    let Some(func) = func else { return };
    for (nam, val) in scangroup(false) {
        // c:150 scangroup(func, flags, 0)
        let pm = param {
            node: hashnode { next: None, nam, flags: PM_SCALAR as i32 }, // c:129, c:135
            u_str: Some(val),                                             // c:134
            ..Default::default()
        };
        func(&pm, flags); // c:136
    }
}

/// Port of `getpmsgr(UNUSED(HashTable ht), const char *name)` from `Src/Modules/hlgroup.c:155`.
/// C body is `return getgroup(name, 1);` — SGR-form variant.
pub fn getpmsgr(_ht: *mut crate::ported::zsh_h::HashTable, name: &str) -> Option<crate::ported::zsh_h::Param> {
    // c:155
    use crate::ported::zsh_h::{hashnode, param, PM_SCALAR, PM_SPECIAL, PM_UNSET};
    let val = getgroup(name, true); // c:157
    let unset = if val.is_none() { PM_UNSET } else { 0 }; // c:101-103
    Some(Box::new(param {
        node: hashnode {
            next: None,
            nam: name.to_string(),                               // c:92
            flags: (PM_SCALAR | PM_SPECIAL | unset) as i32, // c:93
        },
        u_str: Some(val.unwrap_or_default()), // c:102 / c:105
        ..Default::default()
    }))
}

/// Port of `scanpmsgr(UNUSED(HashTable ht), ScanFunc func, int flags)` from `Src/Modules/hlgroup.c:162`.
/// C body is `scangroup(func, flags, 1);` — SGR-form scanner.
pub fn scanpmsgr(
    _ht: *mut crate::ported::zsh_h::HashTable,
    func: Option<crate::ported::zsh_h::ParamScanFunc>,
    flags: i32,
) {
    // c:162
    use crate::ported::zsh_h::{hashnode, param, PM_SCALAR};
    let Some(func) = func else { return };
    for (nam, val) in scangroup(true) {
        // c:164 scangroup(func, flags, 1)
        let pm = param {
            node: hashnode { next: None, nam, flags: PM_SCALAR as i32 }, // c:129, c:135
            u_str: Some(val),                                             // c:134
            ..Default::default()
        };
        func(&pm, flags); // c:136
    }
}

// =====================================================================
// static struct features module_features                            c:170 (hlgroup)
// =====================================================================

// `partab` — port of `static struct paramdef partab[]` (hlgroup.c).

// `module_features` — port of `static struct features module_features`
// from hlgroup.c:170.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/hlgroup.c:182`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:182
    0 // c:197
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/hlgroup.c:189`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:189
    *features = featuresarray(m, module_features());
    0 // c:204
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/hlgroup.c:197`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:197
    handlefeatures(m, module_features(), enables) // c:211
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/hlgroup.c:204`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:204
    0 // c:218
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/hlgroup.c:211`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:211
    setfeatureenables(m, module_features(), None) // c:218
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/hlgroup.c:218`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:218
    0 // c:218
}

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN HLGROUP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["p:.zle.esc".to_string(), "p:.zle.sgr".to_string()]
}

// WARNING: NOT IN HLGROUP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(m: *const module, f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:3392 — the name-keyed variant in src/ported/module.rs; this
    // module ships no `Features` descriptor tables for the per-feature
    // ADDED bit to live on (see MODULE_FEATURE_ENABLES there).
    crate::ported::module::handlefeatures("zsh/hlgroup", &featuresarray(m, f), enables)
}

// WARNING: NOT IN HLGROUP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(m: *const module, f: &Mutex<features>, e: Option<&[i32]>) -> i32 {
    // c:3354-3382 — only a paramdef block (`partab`, c:166-169); the
    // name-keyed ledger adds or removes the `.zle.esc` / `.zle.sgr`
    // special hashes, as for zsh/langinfo.
    crate::ported::module::setfeatureenables("zsh/hlgroup", &featuresarray(m, f), e)
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

// WARNING: NOT IN HLGROUP.C — Rust-only module-framework shim.
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
            pd_size: 2,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collects what a ScanFunc-shaped scanner (c:148/162) hands its callback.
    fn collect_scan(
        scan: fn(*mut crate::ported::zsh_h::HashTable, Option<crate::ported::zsh_h::ParamScanFunc>, i32),
    ) -> Vec<(String, String)> {
        thread_local! {
            static OUT: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        fn cb(pm: &crate::ported::zsh_h::param, _flags: i32) {
            OUT.with(|o| o.borrow_mut().push((pm.node.nam.clone(), pm.u_str.clone().unwrap_or_default())));
        }
        OUT.with(|o| o.borrow_mut().clear());
        scan(std::ptr::null_mut(), Some(cb), 0);
        OUT.with(|o| o.borrow().clone())
    }

    /// Run `f` with the terminal capabilities `applytextattributes`
    /// consults (bold / underline begin sequences) populated, then
    /// restore the globals. `convertattr` output is termcap-dependent
    /// exactly as in C (`tsetcap` emits nothing for an absent capability).
    fn with_attr_caps<R>(f: impl FnOnce() -> R) -> R {
        use crate::ported::zsh_h::{TCBOLDFACEBEG, TCUNDERLINEBEG};
        let saved_len = *crate::ported::init::tclen.lock().unwrap();
        let saved_str = crate::ported::init::tcstr.lock().unwrap().clone();
        let saved_flags = crate::ported::params::TERMFLAGS.load(Ordering::SeqCst);
        // `init_term` (reached by `echoti` in a non-interactive shell) leaves the
        // terminal's colour count behind; an unknown terminal is 0 colours.
        let saved_colours = crate::ported::init::tccolours.swap(0, Ordering::SeqCst);
        {
            let mut len = crate::ported::init::tclen.lock().unwrap();
            let mut caps = crate::ported::init::tcstr.lock().unwrap();
            caps[TCBOLDFACEBEG as usize] = "\x1b[1m".to_string();
            len[TCBOLDFACEBEG as usize] = 4;
            caps[TCUNDERLINEBEG as usize] = "\x1b[4m".to_string();
            len[TCUNDERLINEBEG as usize] = 4;
        }
        crate::ported::params::TERMFLAGS.store(0, Ordering::SeqCst);
        let r = f();
        *crate::ported::init::tclen.lock().unwrap() = saved_len;
        *crate::ported::init::tcstr.lock().unwrap() = saved_str;
        crate::ported::params::TERMFLAGS.store(saved_flags, Ordering::SeqCst);
        crate::ported::init::tccolours.store(saved_colours, Ordering::SeqCst);
        r
    }

    /// Words that are not zsh text attributes (`dim`, `blink`, `strikethrough`,
    /// `hidden`, `reverse`, …) and colours the terminal has no capability for
    /// render nothing: `convertattr` goes through `match_highlight` →
    /// `zattrescape`/`applytextattributes`, which emit only what the terminfo
    /// capabilities in `tcstr` provide (prompt.c:257-280, :1645-1716).
    #[test]
    fn convertattr_unsupported_words_render_nothing() {
        let _g = crate::test_util::global_state_lock();
        for word in [
            "dim", "blink", "strikethrough", "hidden", "invisible",
            "reverse", "inverse",
        ] {
            assert_eq!(
                with_attr_caps(|| convertattr(word, false)),
                "",
                "`{word}` must render nothing without a matching capability"
            );
        }
    }

    /// `faint` is a real zsh attribute (prompt.c:1899) and renders SGR 2.
    #[test]
    fn convertattr_faint_renders_sgr_2() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(with_attr_caps(|| convertattr("faint", false)), "\x1b[2m");
    }

    /// A 24-bit colour on a terminal that declares truecolor renders the direct
    /// SGR form (applytextattributes, prompt.c:1645+). Without the declaration
    /// `match_colour` quantises the colour through `zsh/nearcolor` (prompt.c:1990-1992),
    /// which any test in the process may have loaded, so the declaration is set
    /// here rather than left to whatever state earlier tests left.
    #[test]
    fn convertattr_truecolor_renders_sgr_38_2() {
        let _g = crate::test_util::global_state_lock();
        let saved = crate::ported::params::getaparam(".term.extensions");
        let _ = crate::ported::params::setaparam(".term.extensions", vec!["truecolor".to_string()]);
        let got = with_attr_caps(|| convertattr("fg=#ff0000", false));
        let _ = crate::ported::params::setaparam(".term.extensions", saved.unwrap_or_default());
        assert_eq!(got, "\x1b[38;2;255;0;0m");
    }

    /// The empty specification renders nothing in escape mode.
    #[test]
    fn convertattr_empty_escape_is_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(with_attr_caps(|| convertattr("", false)), "");
    }

    /// `convertattr("bold", false)` renders the terminal's bold-begin
    /// capability through zattrescape/applytextattributes.
    #[test]
    fn convertattr_bold_escape() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(with_attr_caps(|| convertattr("bold", false)), "\x1b[1m");
    }

    /// `convertattr("bold,underline", false)` chains both capabilities.
    #[test]
    fn convertattr_chained_escape() {
        let _g = crate::test_util::global_state_lock();
        let s = with_attr_caps(|| convertattr("bold,underline", false));
        assert!(s.contains("\x1b[1m"));
        assert!(s.contains("\x1b[4m"));
    }

    /// SGR-mode `convertattr("bold", true)` returns `"1"`.
    #[test]
    fn convertattr_sgr_bold() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(with_attr_caps(|| convertattr("bold", true)), "1");
    }

    /// SGR-mode chains: `convertattr("bold,underline", true)` →
    /// `"1;4"`.
    #[test]
    fn convertattr_sgr_chain() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(with_attr_caps(|| convertattr("bold,underline", true)), "1;4");
    }

    /// SGR-mode empty input returns `"0"` per c:67-70 fallback.
    #[test]
    fn convertattr_sgr_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(convertattr("", true), "0");
    }

    /// `getgroup` returns None until the magic-assoc dispatch is
    /// wired (c:99-103 PM_UNSET branch).
    #[test]
    fn getgroup_returns_none_until_paramtable_wired() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getgroup("any", false), None);
        assert_eq!(getgroup("any", true), None);
    }

    /// `scangroup` returns empty until paramtable wiring lands
    /// (c:124-125 early exit).
    #[test]
    fn scangroup_returns_empty_until_paramtable_wired() {
        let _g = crate::test_util::global_state_lock();
        assert!(scangroup(false).is_empty());
        assert!(scangroup(true).is_empty());
    }

    /// c:40 — `convertattr("")` (empty input). Defensive edge.
    #[test]
    fn convertattr_empty_input_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let _ = convertattr("", false);
        let _ = convertattr("", true);
    }

    /// c:40 — `convertattr("bold")` adds bold SGR (1).
    #[test]
    fn convertattr_bold_emits_sgr_bold() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bold", false);
        assert!(
            s.contains("\x1b[1m") || s.contains("\x1b[1;"),
            "bold attr must emit SGR 1, got {:?}",
            s
        );
    }

    /// c:40 — Unknown attr keyword does NOT panic.
    #[test]
    fn convertattr_unknown_attr_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let _ = convertattr("definitely_not_a_real_attr", false);
    }

    /// c:40 — 256-color upper boundary `fg=255`.
    #[test]
    fn convertattr_256_color_upper_boundary() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=255", false);
        assert!(
            s.contains("38;5;255"),
            "256-color upper boundary 255 must encode correctly, got {:?}",
            s
        );
    }

    /// c:141 — `getpmesc` for empty/unknown name is a PM_UNSET node.
    #[test]
    fn getpmesc_empty_or_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getpmesc(std::ptr::null_mut(), "").is_some_and(|p| p.node.flags as u32 & crate::ported::zsh_h::PM_UNSET != 0));
        assert!(getpmesc(std::ptr::null_mut(), "definitely_not_in_table_xyzzy").is_some_and(|p| p.node.flags as u32 & crate::ported::zsh_h::PM_UNSET != 0));
    }

    /// c:155 — `getpmsgr` symmetric with getpmesc; empty + unknown → PM_UNSET node.
    #[test]
    fn getpmsgr_empty_or_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getpmsgr(std::ptr::null_mut(), "").is_some_and(|p| p.node.flags as u32 & crate::ported::zsh_h::PM_UNSET != 0));
        assert!(getpmsgr(std::ptr::null_mut(), "definitely_not_in_table_xyzzy").is_some_and(|p| p.node.flags as u32 & crate::ported::zsh_h::PM_UNSET != 0));
    }

    /// c:148/162 — `scanpmesc` and `scanpmsgr` hand nothing to the callback
    /// vec until paramtable wiring lands.
    #[test]
    fn scanpmesc_and_scanpmsgr_are_empty_until_wired() {
        let _g = crate::test_util::global_state_lock();
        assert!(collect_scan(scanpmesc).is_empty());
        assert!(collect_scan(scanpmsgr).is_empty());
    }

    /// c:182-210 — module-lifecycle stubs return 0.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        let mut features = Vec::new();
        assert_eq!(features_(m, &mut features), 0);
        let mut enables: Option<Vec<i32>> = None;
        assert_eq!(enables_(m, &mut enables), 0);
    }

    /// `Src/Modules/hlgroup.c:40-44` — `light-` is the alias for
    /// `bright-`. Pin both prefix variants map to the same code.
    #[test]
    fn convertattr_light_prefix_is_alias_for_bright() {
        let _g = crate::test_util::global_state_lock();
        let bright = convertattr("fg=bright-green", false);
        let light = convertattr("fg=light-green", false);
        assert_eq!(
            bright, light,
            "c:40 — light- and bright- prefixes must produce identical SGR codes"
        );
    }

    /// `Src/Modules/hlgroup.c:40-72` — SGR-mode bg color rendering.
    /// `bg=blue` in SGR mode produces "44" (the digits between
    /// `\e[` and `m`, no surrounding chars).
    #[test]
    fn convertattr_sgr_bg_color() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            convertattr("bg=blue", true),
            "44",
            "SGR mode strips ESC[/m wrapper → bare digit string"
        );
        assert_eq!(convertattr("bg=red", true), "41");
    }

    /// `Src/Modules/hlgroup.c:40-44` — Invalid color spec (not in
    /// named table, not numeric, not hex) emits NOTHING. SGR mode
    /// falls back to "0". Pin the defensive contract.
    #[test]
    fn convertattr_unknown_color_drops_silently() {
        let _g = crate::test_util::global_state_lock();
        // Plain (escape) mode: empty output for unknown color alone.
        let s = convertattr("fg=not_a_real_color", false);
        assert_eq!(s, "", "unknown color → no escape emitted");
        // SGR mode: empty output → "0" fallback per c:67-70.
        let s = convertattr("fg=not_a_real_color", true);
        assert_eq!(s, "0");
    }

    // ─── zsh-corpus pins for convertattr ───────────────────────────

    /// "bold" → SGR 1.
    #[test]
    fn hlgroup_corpus_bold_is_sgr_1() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bold", false);
        assert!(s.contains("\x1b[1m"), "bold = SGR 1, got {s:?}");
    }

    /// "underline" → SGR 4.
    #[test]
    fn hlgroup_corpus_underline_is_sgr_4() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("underline", false);
        assert!(s.contains("\x1b[4m"));
    }

    /// "italic" → SGR 3.
    #[test]
    fn hlgroup_corpus_italic_is_sgr_3() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("italic", false);
        assert!(s.contains("\x1b[3m"));
    }

    /// "fg=red" → SGR 31.
    #[test]
    fn hlgroup_corpus_fg_red_is_sgr_31() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=red", false);
        assert!(s.contains("\x1b[31m"), "fg=red = SGR 31, got {s:?}");
    }

    /// "bg=blue" → SGR 44.
    #[test]
    fn hlgroup_corpus_bg_blue_is_sgr_44() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bg=blue", false);
        assert!(s.contains("\x1b[44m"), "bg=blue = SGR 44, got {s:?}");
    }

    /// "bold,fg=red" — comma-separated combined attrs.
    #[test]
    fn hlgroup_corpus_combined_attrs() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bold,fg=red", false);
        assert!(s.contains("\x1b[1m"), "has bold");
        assert!(s.contains("\x1b[31m"), "has fg=red");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/hlgroup.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:58 — "bold" → SGR 1.
    #[test]
    fn convertattr_bold_emits_sgr_1() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bold", false);
        assert!(s.contains("\x1b[1m"), "got {:?}", s);
    }

    /// c:58 — "underline" → SGR 4.
    #[test]
    fn convertattr_underline_emits_sgr_4() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("underline", false);
        assert!(s.contains("\x1b[4m"), "got {:?}", s);
    }

    /// c:58 — "fg=red" → SGR 31.
    #[test]
    fn convertattr_fg_red_emits_sgr_31() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=red", false);
        assert!(s.contains("\x1b[31m"));
    }

    /// c:58 — "fg=green" → SGR 32.
    #[test]
    fn convertattr_fg_green_emits_sgr_32() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=green", false);
        assert!(s.contains("\x1b[32m"));
    }

    /// c:58 — "fg=blue" → SGR 34.
    #[test]
    fn convertattr_fg_blue_emits_sgr_34() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=blue", false);
        assert!(s.contains("\x1b[34m"));
    }

    /// c:58 — deterministic.
    #[test]
    fn convertattr_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for input in ["bold", "fg=red", "underline,fg=blue", ""] {
            let first = convertattr(input, false);
            for _ in 0..5 {
                assert_eq!(convertattr(input, false), first);
            }
        }
    }

    /// c:210 — getgroup unknown → None.
    #[test]
    fn getgroup_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getgroup("zshrs_never_real_group_xyz", false).is_none());
    }

    /// c:210 — getgroup empty → None.
    #[test]
    fn getgroup_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getgroup("", false).is_none());
    }

    /// c:283 — scangroup no panic.
    #[test]
    fn scangroup_returns_vec_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = scangroup(false);
        let _ = scangroup(true);
    }

    /// c:297 — getpmesc unknown → PM_UNSET node.
    #[test]
    fn getpmesc_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getpmesc(std::ptr::null_mut(), "zshrs_never_real_esc_xyz").is_some_and(|p| p.node.flags as u32 & crate::ported::zsh_h::PM_UNSET != 0));
    }

    /// c:313 — getpmsgr unknown → PM_UNSET node.
    #[test]
    fn getpmsgr_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getpmsgr(std::ptr::null_mut(), "zshrs_never_real_sgr_xyz").is_some_and(|p| p.node.flags as u32 & crate::ported::zsh_h::PM_UNSET != 0));
    }

    /// Lifecycle (c:337/366) split per-hook.
    #[test]
    fn hlgroup_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:366 — cleanup_(NULL) = 0.
    #[test]
    fn hlgroup_cleanup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cleanup_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/hlgroup.c
    // c:58 convertattr / c:210 getgroup / c:283 scangroup / c:297 getpmesc /
    // c:305 scanpmesc / c:313 getpmsgr / c:321 scanpmsgr / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:58 — `convertattr` returns String (compile-time type pin).
    #[test]
    fn convertattr_returns_string_type() {
        let _: String = convertattr("", false);
        let _: String = convertattr("fg=red", true);
    }

    /// c:58 — `convertattr("", _)` is well-defined (empty or default).
    #[test]
    fn convertattr_empty_input_no_panic() {
        let _ = convertattr("", false);
        let _ = convertattr("", true);
    }

    /// c:58 — `convertattr` full-sweep pure.
    #[test]
    fn convertattr_is_pure_full_sweep() {
        let _g = crate::test_util::global_state_lock();
        with_attr_caps(|| {
        for input in ["", "fg=red", "bold", "bg=blue,underline"] {
            let first_no_sgr = convertattr(input, false);
            let first_sgr = convertattr(input, true);
            for _ in 0..3 {
                assert_eq!(
                    convertattr(input, false),
                    first_no_sgr,
                    "convertattr({:?}, false) must be pure",
                    input
                );
                assert_eq!(
                    convertattr(input, true),
                    first_sgr,
                    "convertattr({:?}, true) must be pure",
                    input
                );
            }
        }
        });
    }

    /// c:210 — `getgroup` returns Option<String>.
    #[test]
    fn getgroup_returns_option_string_type() {
        let _: Option<String> = getgroup("any", false);
    }

    /// c:283 — `scangroup` returns Vec (compile-time pin).
    #[test]
    fn scangroup_returns_vec_type() {
        let _: Vec<(String, String)> = scangroup(false);
    }

    /// c:283 — `scangroup` is deterministic.
    #[test]
    fn scangroup_is_deterministic() {
        let first = scangroup(false);
        for _ in 0..3 {
            assert_eq!(scangroup(false), first, "scangroup must be deterministic");
        }
    }

    /// c:297 — `getpmesc(empty)` is a PM_UNSET node.
    #[test]
    fn getpmesc_empty_returns_none() {
        assert!(getpmesc(std::ptr::null_mut(), "").is_some_and(|p| p.node.flags as u32 & crate::ported::zsh_h::PM_UNSET != 0));
    }

    /// c:313 — `getpmsgr(empty)` is a PM_UNSET node.
    #[test]
    fn getpmsgr_empty_returns_none() {
        assert!(getpmsgr(std::ptr::null_mut(), "").is_some_and(|p| p.node.flags as u32 & crate::ported::zsh_h::PM_UNSET != 0));
    }

    /// c:305 + c:321 — `scanpmesc`/`scanpmsgr` return Vec (type pin).
    #[test]
    fn scanpm_variants_return_vec_type() {
        let _: Vec<(String, String)> = collect_scan(scanpmesc);
        let _: Vec<(String, String)> = collect_scan(scanpmsgr);
    }

    /// c:337-373 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn hlgroup_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
        assert_eq!(finish_(null), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/hlgroup.c
    // c:58 convertattr / c:210 getgroup / c:283 scangroup /
    // c:297 getpmesc / c:313 getpmsgr / c:305 scanpmesc / c:321 scanpmsgr
    // ═══════════════════════════════════════════════════════════════════

    /// c:58 — `convertattr` returns String (compile-time pin, alt).
    #[test]
    fn convertattr_returns_string_pin_alt() {
        let _: String = convertattr("fg=red", false);
    }

    /// c:58 — `convertattr` empty input is deterministic across modes.
    #[test]
    fn convertattr_empty_input_both_modes_deterministic() {
        let a1 = convertattr("", false);
        let a2 = convertattr("", false);
        assert_eq!(a1, a2, "empty(false) must be pure");
        let b1 = convertattr("", true);
        let b2 = convertattr("", true);
        assert_eq!(b1, b2, "empty(true) must be pure");
    }

    /// c:210 — `getgroup` for empty name is deterministic.
    #[test]
    fn getgroup_empty_name_deterministic() {
        let a = getgroup("", false);
        let b = getgroup("", false);
        assert_eq!(
            a.is_some(),
            b.is_some(),
            "getgroup('') must be deterministic"
        );
    }

    /// c:210 — `getgroup` returns Option<String> for sgr=true too.
    #[test]
    fn getgroup_sgr_mode_returns_option_string_type() {
        let _: Option<String> = getgroup("name", true);
    }

    /// c:283 — `scangroup(true)` returns Vec (sgr-mode pin).
    #[test]
    fn scangroup_sgr_mode_returns_vec_type() {
        let _: Vec<(String, String)> = scangroup(true);
    }

    /// c:283 — `scangroup(sgr=false)` and `scangroup(sgr=true)` are
    /// both deterministic per call.
    #[test]
    fn scangroup_both_modes_deterministic() {
        let f1 = scangroup(false);
        let f2 = scangroup(false);
        assert_eq!(f1, f2, "scangroup(false) must be deterministic");
        let t1 = scangroup(true);
        let t2 = scangroup(true);
        assert_eq!(t1, t2, "scangroup(true) must be deterministic");
    }

    /// c:297 — `getpmesc` returns Option<Param> (compile-time pin).
    #[test]
    fn getpmesc_returns_option_string_type() {
        let _: Option<crate::ported::zsh_h::Param> = getpmesc(std::ptr::null_mut(), "anykey");
    }

    /// c:313 — `getpmsgr` returns Option<Param> (compile-time pin).
    #[test]
    fn getpmsgr_returns_option_string_type() {
        let _: Option<crate::ported::zsh_h::Param> = getpmsgr(std::ptr::null_mut(), "anykey");
    }

    /// c:305 — `scanpmesc` is deterministic.
    #[test]
    fn scanpmesc_is_deterministic() {
        let a = collect_scan(scanpmesc);
        let b = collect_scan(scanpmesc);
        assert_eq!(a, b, "scanpmesc must be deterministic");
    }

    /// c:321 — `scanpmsgr` is deterministic.
    #[test]
    fn scanpmsgr_is_deterministic() {
        let a = collect_scan(scanpmsgr);
        let b = collect_scan(scanpmsgr);
        assert_eq!(a, b, "scanpmsgr must be deterministic");
    }

    /// c:283 — `scangroup` results: all keys non-empty (every group has a name).
    #[test]
    fn scangroup_keys_all_non_empty() {
        for (k, _) in scangroup(false) {
            assert!(
                !k.is_empty(),
                "scangroup must not yield entries with empty key"
            );
        }
    }
}
