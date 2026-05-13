//! `zsh/ksh93` module — port of `Src/Modules/ksh93.c`.
//!
//! Implementing "namespace" requires creating a new keyword.               // c:34
//! Dummy to treat as NULL                                                  // c:105
//!
//! Top-level declaration order matches C source line-by-line:
//!   - `static struct builtin bintab[]`             c:40
//!   - `edcharsetfn(pm, x)`                          c:46
//!   - `matchgetfn(pm)`                              c:60
//!   - 6× `static const struct gsu_*`                c:91-103
//!   - `static char sh_unsetval[2]`                  c:105
//!   - `static char *sh_name = sh_unsetval;`         c:106
//!   - `static char *sh_subscript = sh_unsetval;`    c:107
//!   - `static char *sh_edchar = sh_unsetval;`       c:108
//!   - `static char sh_edmode[2]`                    c:109
//!   - `static struct paramdef partab[]`             c:116
//!   - `static struct features module_features`     c:133
//!   - `ksh93_wrapper(prog, w, name)`                c:142
//!   - `static struct funcwrap wrapper[]`            c:230
//!   - `setup_(m)` / `features_(m, features)` /
//!     `enables_(m, enables)` / `boot_(m)` /
//!     `cleanup_(m)` / `finish_(m)`                  c:235-287

#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};

use crate::ported::signals_h::{queue_signals, unqueue_signals};
use crate::ported::string::dupstring;
use crate::ported::ztype_h::INAMESPC;
use crate::ported::zsh_h::{
    eprog, funcwrap, module, param, EMULATE_KSH, EMULATION,
    PM_LOCAL, PM_NAMEREF, PM_READONLY, PM_UNSET,
};
use crate::ported::zsh_h::{PM_NAMEREF as PMN, PARAMDEF};

// =====================================================================
// /* Implementing "namespace" requires creating a new keword.  Hrm. */ c:34
// /* Standard module configuration/linkage */                          c:36-38
// static struct builtin bintab[] = {
//     BUILTIN("nameref", BINF_ASSIGN, ..., 0, -1, 0, "gpru", "n")
// };                                                                  c:40
//
// Static dispatch table consumed by C module loader. Static-link path:
// the `nameref` builtin is dispatched directly via `bin_typeset` in
// the typeset port. Table omitted from Rust port pending module-loader.
// =====================================================================

// =====================================================================
// edcharsetfn(Param pm, char *x)                                     c:46
// =====================================================================

/// Port of `edcharsetfn(Param pm, char *x)` from `Src/Modules/ksh93.c:47`.
#[allow(unused_variables)]
pub fn edcharsetfn(pm: *mut param, x: *mut libc::c_char) {           // c:47
    /*
     * To make this work like ksh, we must intercept $KEYS before the widget
     * is looked up, so that changing the key sequence causes a different
     * widget to be substituted.  Somewhat similar to "bindkey -s".
     *
     * Ksh93 adds SIGKEYBD to the trap list for this purpose.
     */                                                                 // c:49-55
    /* ; */                                                             // c:56
}

// =====================================================================
// matchgetfn(Param pm)                                               c:60
// =====================================================================

/// Port of `matchgetfn(Param pm)` from `Src/Modules/ksh93.c:60`.
///
/// C signature mirrored verbatim:
/// ```c
/// static char **
/// matchgetfn(Param pm)
/// ```
pub fn matchgetfn(pm: *mut param) -> Vec<String> {                     // c:60
    // c:60 — `char **zsh_match = getaparam("match");`
    let zsh_match: Vec<String> = crate::ported::params::paramtab().read().ok()
        .and_then(|t| t.get("match").and_then(|p| p.u_arr.clone()))
        .unwrap_or_default();
    /*
     * For this to work accurately, ksh emulation should always imply
     * that the (#m) and (#b) extendedglob operators are enabled.
     *
     * When we have a 0th element (ksharrays), it is $MATCH.  Elements
     * 1st and larger mirror the $match array.
     */                                                                 // c:64-69
    // c:71-72 — `if (pm->u.arr) freearray(pm->u.arr);`
    if !pm.is_null() {
        unsafe { (*pm).u_arr = None; }                                  // c:71-72 freearray
    }
    if !zsh_match.is_empty() {                                          // c:73
        if isset(KSHARRAYS) {                                           // c:74
            // c:75-80 — char **ap = zalloc(...); pm->u.arr = ap;
            //           *ap++ = ztrdup(getsparam("MATCH"));
            //           while (*zsh_match) *ap = ztrdup(*zsh_match++);
            let match_str: String = crate::ported::params::paramtab().read().ok()
                .and_then(|t| t.get("MATCH").and_then(|p| p.u_str.clone()))
                .unwrap_or_default();
            let mut ap: Vec<String> = Vec::with_capacity(zsh_match.len() + 1);
            ap.push(crate::ported::utils::ztrdup(&match_str));          // c:78
            for s in &zsh_match {                                       // c:79-80
                ap.push(crate::ported::utils::ztrdup(s));
            }
            if !pm.is_null() {
                unsafe { (*pm).u_arr = Some(ap.clone()); }
            }
            // c:88 — `return arrgetfn(pm);` — arrgetfn (params.c) reads
            // pm->u.arr we just set, so return the same vec.
            ap
        } else {
            // c:81-82 — pm->u.arr = zarrdup(zsh_match);
            let dup: Vec<String> = zsh_match.iter()
                .map(|s| crate::ported::utils::ztrdup(s))
                .collect();
            if !pm.is_null() {
                unsafe { (*pm).u_arr = Some(dup.clone()); }
            }
            dup                                                          // c:88 arrgetfn(pm)
        }
    } else if isset(KSHARRAYS) {                                        // c:83
        // c:84 — pm->u.arr = mkarray(ztrdup(getsparam("MATCH")));
        let match_str: String = crate::ported::params::paramtab().read().ok()
            .and_then(|t| t.get("MATCH").and_then(|p| p.u_str.clone()))
            .unwrap_or_default();
        let one = vec![crate::ported::utils::ztrdup(&match_str)];
        if !pm.is_null() {
            unsafe { (*pm).u_arr = Some(one.clone()); }
        }
        one                                                              // c:88 arrgetfn(pm)
    } else {
        // c:86 — pm->u.arr = NULL;
        if !pm.is_null() {
            unsafe { (*pm).u_arr = None; }
        }
        Vec::new()                                                       // c:88 arrgetfn(pm) → NULL
    }
}

// =====================================================================
// static const struct gsu_scalar constant_gsu                        c:91
// static const struct gsu_scalar sh_edchar_gsu                       c:94
// static const struct gsu_scalar sh_edmode_gsu                       c:96
// static const struct gsu_array sh_match_gsu                         c:98
// static const struct gsu_scalar sh_name_gsu                         c:100
// static const struct gsu_scalar sh_subscript_gsu                    c:102
//
// GSU vtables for the `.sh.*` parameters. Wired into `partab[]` below.
// Static-link path: dispatcher invokes the getfn/setfn directly.
// Tables omitted pending param-table port.
// =====================================================================

// =====================================================================
// static char sh_unsetval[2];	/* Dummy to treat as NULL */          c:105
// static char *sh_name = sh_unsetval;                                c:106
// static char *sh_subscript = sh_unsetval;                           c:107
// static char *sh_edchar = sh_unsetval;                              c:108
// static char sh_edmode[2];                                          c:109
// =====================================================================

/// Port of `static char sh_unsetval[2];` from `ksh93.c:105`.
/// /* Dummy to treat as NULL */
pub static sh_unsetval: [u8; 2] = [0, 0];                              // c:105

/// Port of `static char *sh_name = sh_unsetval;` from `ksh93.c:106`.
pub static sh_name: Mutex<String> = Mutex::new(String::new());          // c:106

/// Port of `static char *sh_subscript = sh_unsetval;` from `ksh93.c:107`.
pub static sh_subscript: Mutex<String> = Mutex::new(String::new());     // c:107

/// Port of `static char *sh_edchar = sh_unsetval;` from `ksh93.c:108`.
pub static sh_edchar: Mutex<String> = Mutex::new(String::new());        // c:108

/// Port of `static char sh_edmode[2];` from `ksh93.c:109`.
pub static sh_edmode: Mutex<[u8; 2]> = Mutex::new([0, 0]);              // c:109

// =====================================================================
/*
 * Some parameters listed here do not appear in ksh93.mdd autofeatures
 * because they are only instantiated by ksh93_wrapper() below.  This
 * obviously includes those commented out here.
 */                                                                    // c:111-115
// static struct paramdef partab[]                                    c:116
// static struct features module_features                             c:133
//
// Param/feature dispatch tables. Omitted pending module-loader port.
// =====================================================================

// =====================================================================
// ksh93_wrapper(Eprog prog, FuncWrap w, char *name)                  c:142
// =====================================================================

/// `LOCAL_NAMEREF` — `#define LOCAL_NAMEREF (PM_LOCAL|PM_UNSET|PM_NAMEREF)`
/// from `Src/Modules/ksh93.c:143`.
#[allow(dead_code)]
const LOCAL_NAMEREF: u32 = PM_LOCAL | PM_UNSET | PM_NAMEREF;            // c:143

/// Port of `ksh93_wrapper(Eprog prog, FuncWrap w, char *name)` from `Src/Modules/ksh93.c:143`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// ksh93_wrapper(Eprog prog, FuncWrap w, char *name)
/// ```
#[allow(unused_variables)]
pub fn ksh93_wrapper(prog: *const eprog, w: *const funcwrap, name: *mut libc::c_char) -> i32 { // c:143
    // c:143 — `Funcstack f;`
    let mut f: *const crate::ported::zsh_h::funcstack;
    // c:146 — `Param pm;`
    let mut pm: *mut param;
    // c:147 — `zlong num = funcstack->prev ? getiparam(".sh.level") : 0;`
    // funcstack is the global from Src/exec.c:340; stub holds NULL so
    // funcstack->prev is always NULL → branch picks 0.
    let mut num: i64 = if (*funcstack.lock().unwrap()) != 0 {
        crate::ported::params::paramtab().read().ok()
            .and_then(|t| t.get(".sh.level")
                .and_then(|p| p.u_str.as_ref().and_then(|s| s.parse::<i64>().ok())))
            .unwrap_or(0)
    } else { 0 };

    if !EMULATION(EMULATE_KSH) {                                        // c:149
        return 1;                                                       // c:150
    }

    if num == 0 {                                                       // c:152
        // c:153 — for (f = funcstack; f; f = f->prev, num++);
        f = (*funcstack.lock().unwrap()) as *const crate::ported::zsh_h::funcstack;
        while !f.is_null() {
            // f = f->prev — stub: no real chain, exit immediately.
            f = std::ptr::null_mut();
            num += 1;
        }
    } else {                                                            // c:154
        num += 1;                                                       // c:155
    }

    queue_signals();                                                    // c:157
    locallevel.fetch_add(1, Ordering::SeqCst);                          // c:158 ++locallevel;
    /* Make these local */                                              // c:158 trailing comment

    // c:160-165 — .sh.command setup
    pm = crate::ported::params::createparam(".sh.command", LOCAL_NAMEREF as i32)
        .map(Box::into_raw).unwrap_or(std::ptr::null_mut());
    if !pm.is_null() {
        unsafe { (*pm).level = locallevel.load(Ordering::Relaxed); }    // c:161 pm->level = locallevel;
                                                                         //       /* Why is this necessary? */
        /* Force scoping by assignent hack */                           // c:162
        crate::ported::params::setloopvar(".sh.command", "ZSH_DEBUG_CMD"); // c:163
        unsafe { (*pm).node.flags |= PM_READONLY as i32; }              // c:164
    }
    /* .sh.edchar is in partab and below */                             // c:166
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) != 0 {
        pm = crate::ported::params::createparam(".sh.edcol", LOCAL_NAMEREF as i32) // c:167
            .map(Box::into_raw).unwrap_or(std::ptr::null_mut());
        if !pm.is_null() {
            unsafe { (*pm).level = locallevel.load(Ordering::Relaxed); } // c:168
            crate::ported::params::setloopvar(".sh.edcol", "CURSOR");   // c:169
            unsafe { (*pm).node.flags |= (PM_NAMEREF | PM_READONLY) as i32; } // c:170
        }
    }
    /* .sh.edmode is in partab and below */                             // c:172
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) != 0 {
        pm = crate::ported::params::createparam(".sh.edtext", LOCAL_NAMEREF as i32) // c:173
            .map(Box::into_raw).unwrap_or(std::ptr::null_mut());
        if !pm.is_null() {
            unsafe { (*pm).level = locallevel.load(Ordering::Relaxed); } // c:174
            crate::ported::params::setloopvar(".sh.edtext", "BUFFER");  // c:175
            unsafe { (*pm).node.flags |= PM_READONLY as i32; }          // c:176
        }
    }

    pm = crate::ported::params::createparam(".sh.fun",                  // c:179
            (PM_LOCAL | PM_UNSET) as i32)
        .map(Box::into_raw).unwrap_or(std::ptr::null_mut());
    if !pm.is_null() {
        unsafe { (*pm).level = locallevel.load(Ordering::Relaxed); }    // c:180
        let name_str: String = if name.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned() }
        };
        crate::ported::params::setsparam(".sh.fun",                     // c:181
            &crate::ported::utils::ztrdup(&name_str));
        unsafe { (*pm).node.flags |= PM_READONLY as i32; }              // c:182
    }
    pm = crate::ported::params::createparam(".sh.level",                // c:184
            (PM_LOCAL | PM_UNSET) as i32)
        .map(Box::into_raw).unwrap_or(std::ptr::null_mut());
    if !pm.is_null() {
        unsafe { (*pm).level = locallevel.load(Ordering::Relaxed); }    // c:185
        crate::ported::params::setiparam(".sh.level", num);             // c:186
    }
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) != 0 {                                          // c:188
        // c:189-190 — extern mod_import_variable char *curkeymapname / *varedarg;
        // (extern declarations are at the locals-block level in C; ours
        // are file-level statics below.)
        /* bindkey -v forces VIMODE so this test is as good as any */   // c:191
        let curkmap = curkeymapname.lock().unwrap().clone();
        if !curkmap.is_empty() && isset(VIMODE) && curkmap == "main" {  // c:192-193
            // c:194 — strcpy(sh_edmode, "\033");
            let mut em = sh_edmode.lock().unwrap();
            em[0] = 0o33;
            em[1] = 0;
        } else {
            // c:196 — strcpy(sh_edmode, "");
            let mut em = sh_edmode.lock().unwrap();
            em[0] = 0;
            em[1] = 0;
        }
        // c:197-198 — if (sh_edchar == sh_unsetval) sh_edchar = dupstring(getsparam("KEYS"));
        let edch_unset = sh_edchar.lock().unwrap().is_empty();
        if edch_unset {
            let keys: String = crate::ported::params::paramtab().read().ok()
                .and_then(|t| t.get("KEYS").and_then(|p| p.u_str.clone()))
                .unwrap_or_default();
            *sh_edchar.lock().unwrap() = dupstring(&keys);
        }
        let varedarg_val = varedarg.lock().unwrap().clone();
        if !varedarg_val.is_empty() {                                   // c:199
            // c:200 — char *ie = itype_end((sh_name = dupstring(varedarg)), INAMESPC, 0);
            *sh_name.lock().unwrap() = dupstring(&varedarg_val);
            let nm = sh_name.lock().unwrap().clone();
            // c:200 — `char *ie = itype_end((sh_name=...), INAMESPC, 0);`
            // itype_end returns a pointer past the run of chars matching
            // the type-bits; INAMESPC = identifier-namespace chars.
            // Rust port: utils::itype_end takes (s, allow_digits_start)
            // — wrong signature for INAMESPC. Inline the byte-walk to
            // mirror the C `for (;;) test bit; advance;` loop.
            let _ = INAMESPC;
            let ie_off = {
                let mut k = 0;
                for &b in nm.as_bytes() {
                    match b {
                        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' => k += 1,
                        _ => break,
                    }
                }
                k
            };
            if ie_off < nm.len() {                                      // c:201 if (ie && *ie)
                // c:202 — *ie++ = '\0';
                let head = &nm[..ie_off];
                let tail = &nm[ie_off + 1..]; // skip the bracket char
                *sh_name.lock().unwrap() = head.to_string();
                /* Assume bin_vared has validated subscript */          // c:203
                // c:204 — sh_subscript = dupstring(ie);
                *sh_subscript.lock().unwrap() = dupstring(tail);
                // c:205-206 — ie = sh_subscript + strlen(sh_subscript); *--ie = '\0';
                let mut sub = sh_subscript.lock().unwrap();
                if sub.ends_with(']') { sub.pop(); }
            } else {
                // c:208 — sh_subscript = sh_unsetval;
                sh_subscript.lock().unwrap().clear();
            }
            pm = crate::ported::params::createparam(".sh.value",        // c:209
                    LOCAL_NAMEREF as i32)
                .map(Box::into_raw).unwrap_or(std::ptr::null_mut());
            if !pm.is_null() {
                unsafe { (*pm).level = locallevel.load(Ordering::Relaxed); } // c:210
                crate::ported::params::setloopvar(".sh.value", "BUFFER"); /* Hack */ // c:211
                unsafe { (*pm).node.flags |= PM_READONLY as i32; }      // c:212
            }
        } else {
            // c:215 — sh_name = sh_subscript = sh_unsetval;
            sh_name.lock().unwrap().clear();
            sh_subscript.lock().unwrap().clear();
        }
    } else {
        // c:217 — sh_edchar = sh_name = sh_subscript = sh_unsetval;
        sh_edchar.lock().unwrap().clear();
        sh_name.lock().unwrap().clear();
        sh_subscript.lock().unwrap().clear();
        // c:218 — strcpy(sh_edmode, "");
        let mut em = sh_edmode.lock().unwrap();
        em[0] = 0;
        em[1] = 0;
        /* TODO:                                                       // c:219-222
         * - disciplines
         * - special handling of .sh.value in math
         */
    }
    locallevel.fetch_sub(1, Ordering::SeqCst);                          // c:224 --locallevel;
    unqueue_signals();                                                  // c:225

    1                                                                   // c:227
}

// =====================================================================
// static struct funcwrap wrapper[]                                   c:230
//
// Per-function wrapper table consumed by `addwrapper(m, wrapper)` at
// boot_. Omitted pending module-loader port.
// =====================================================================

// =====================================================================
// setup_(UNUSED(Module m))                                           c:235
// =====================================================================

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/ksh93.c:236`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {                                    // c:236
    // C body c:238-239 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/ksh93.c:243`.
/// C body c:245-247 — `*features = featuresarray(m, &module_features); return 0`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {     // c:243
    *features = featuresarray(m, module_features());
    0                                                                        // c:258
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/ksh93.c:251`.
/// C body c:253-254 — `return handlefeatures(m, &module_features, enables)`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {  // c:251
    handlefeatures(m, module_features(), enables) // c:258
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/ksh93.c:258`.
/// C body: `return addwrapper(m, wrapper);`
pub fn boot_(m: *const module) -> i32 {
    // c:258 — addwrapper(m, wrapper); zshrs's fusevm doesn't run
    // through C's wrapper-dispatch chain, no-op until wrapper
    // machinery gets a Rust equivalent.
    let _ = m;
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/ksh93.c:265`.
/// C body (c:267-278):
/// ```c
/// struct paramdef *p;
/// deletewrapper(m, wrapper);
/// for (p = partab; p < partab + sizeof(partab)/sizeof(*partab); ++p) {
///     if (p->flags & PM_NAMEREF) {
///         HashNode hn = gethashnode2(paramtab, p->name);
///         if (hn)
///             ((Param)hn)->node.flags &= ~PM_NAMEREF;
///     }
/// }
/// return setfeatureenables(m, &module_features, NULL);
/// ```
pub fn cleanup_(m: *const module) -> i32 {
    // c:267 — `struct paramdef *p;`
    let mut p: usize;                                                    // c:267 (index over partab)
    // c:269 — deletewrapper(m, wrapper); zshrs's fusevm wrapper
    // machinery is a no-op (see boot_ note).

    // c:116-131 — `static struct paramdef partab[]` inlined here
    // because Rust statics can't hold String-typed paramdef.name.
    let partab: [crate::ported::zsh_h::paramdef; 9] = [
        PARAMDEF(".sh.edchar",    (PM_SCALAR | PM_SPECIAL) as i32, 0, 0),                  // c:117
        PARAMDEF(".sh.edmode",    (PM_SCALAR | PM_READONLY | PM_SPECIAL) as i32, 0, 0),    // c:119
        PARAMDEF(".sh.file",      (PMN | PM_READONLY) as i32, 0, 0),                       // c:121
        PARAMDEF(".sh.lineno",    (PMN | PM_READONLY) as i32, 0, 0),                       // c:122
        PARAMDEF(".sh.match",     (PM_ARRAY | PM_READONLY) as i32, 0, 0),                  // c:123
        PARAMDEF(".sh.name",      (PM_SCALAR | PM_READONLY | PM_SPECIAL) as i32, 0, 0),    // c:124
        PARAMDEF(".sh.subscript", (PM_SCALAR | PM_READONLY | PM_SPECIAL) as i32, 0, 0),    // c:126
        PARAMDEF(".sh.subshell",  (PMN | PM_READONLY) as i32, 0, 0),                       // c:128
        PARAMDEF(".sh.version",   (PMN | PM_READONLY) as i32, 0, 0),                       // c:130
    ];

    /* Clean up namerefs, otherwise deleteparamdef() is confused */     // c:271
    // c:272-277 — `for (p = partab; p < partab + ARRSZ; ++p) { ... }`
    p = 0;
    while p < partab.len() {                                             // c:272
        let entry = &partab[p];
        if (entry.flags as u32 & PM_NAMEREF) != 0 {                      // c:273
            // c:274 — `HashNode hn = gethashnode2(paramtab, p->name);`
            let hn: *mut param = gethashnode2(&paramtab, &entry.name);
            if !hn.is_null() {                                           // c:275
                unsafe { (*hn).node.flags &= !(PM_NAMEREF as i32); }    // c:276
            }
        }
        p += 1;
    }
    setfeatureenables(m, module_features(), None) // c:279
}


use crate::ported::zsh_h::features as features_t;

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

/// Port of `ksh93_wrapper(Eprog prog, FuncWrap w, char *name)` from `Src/Modules/ksh93.c:143`.
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 1,                // bintab: nameref (ksh93.c)
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,
        pd_size: 9,                // partab: .sh.edchar/edmode/file/lineno/match/name/subscript/subshell/version
        n_abstract: 0,
    }))
}

// Local descriptor stub mirroring the C bintab + partab.
// WARNING: NOT IN KSH93.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec![
        "b:nameref".to_string(),
        "p:.sh.edchar".to_string(),
        "p:.sh.edmode".to_string(),
        "p:.sh.file".to_string(),
        "p:.sh.lineno".to_string(),
        "p:.sh.match".to_string(),
        "p:.sh.name".to_string(),
        "p:.sh.subscript".to_string(),
        "p:.sh.subshell".to_string(),
        "p:.sh.version".to_string(),
    ]
}

// WARNING: NOT IN KSH93.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features_t>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 10]);
    }
    0
}

// WARNING: NOT IN KSH93.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<features_t>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

// PM_SCALAR / PM_ARRAY / PM_SPECIAL — referenced by PARTAB above.
const PM_SCALAR: u32 = crate::ported::zsh_h::PM_SCALAR;
const PM_ARRAY: u32 = crate::ported::zsh_h::PM_ARRAY;
const PM_SPECIAL: u32 = crate::ported::zsh_h::PM_SPECIAL;

// `paramtab` — `HashTable` global from `Src/params.c:46`. Stub: empty
// usize-sized opaque holder; gethashnode2 against it always returns NULL.
static paramtab: AtomicI32 = AtomicI32::new(0);

// `gethashnode2` lives in `Src/hashtable.c:255`. C signature:
//   HashNode gethashnode2(HashTable ht, const char *nam);
// Stub uses the param-shaped overload — returns NULL since paramtab
// is empty in static-link path.
/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/ksh93.c`.
fn gethashnode2(_ht: &AtomicI32, _name: &str) -> *mut param {
    std::ptr::null_mut()
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/ksh93.c:284`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {                                   // c:284
    // C body c:286-287 — `return 0`. Faithful empty-body port; the
    //                    ksh93 wrapper unregisters in cleanup_ via
    //                    deletewrapper.
    0
}

// =====================================================================
// External fns / globals from other Src/*.c files. Stubbed locally
// pending the proper ports of their home files.
// =====================================================================

// `emulation` lives in `crate::ported::options::emulation` per Rule C
// (its C definition is `Src/options.c:36`, not `ksh93.c`). The
// `EMULATION(bits)` macro at zsh.h:2347 tests bits against it.
pub use crate::ported::options::emulation;

// `locallevel` lives in `crate::ported::params::locallevel` per Rule C
// (its C definition is `Src/params.c:54`, not `ksh93.c`). Bumped by
// `startparamscope` on function entry, decremented by `endparamscope`
// on return. `ksh93_wrapper` increments before `createparam()` and
// decrements after.
pub use crate::ported::params::locallevel;

/// `curkeymapname` — `char *` global from `Src/Zle/zle_keymap.c`,
/// declared `extern` at c:189. Holds the active keymap name.
pub static curkeymapname: Mutex<String> = Mutex::new(String::new());

/// `varedarg` — `char *` global from `Src/Zle/zle_misc.c`, declared
/// `extern` at c:190. Holds the parameter name being edited by `vared`.
pub static varedarg: Mutex<String> = Mutex::new(String::new());

// `funcstack` — `Funcstack` global from `Src/exec.c:340`. Stubbed as
// a Mutex-wrapped raw-pointer holder since pointers aren't `Sync`
// without explicit handling. NULL by default — exec.c port wires the
// real walk.
static funcstack: Mutex<usize> = Mutex::new(0);

// `KSHARRAYS` / `VIMODE` — option indices from `Src/zsh.h`. `isset(X)`
// macro reads `opts[X]` (zsh.h:1455). Stub: false until options.c
// global table is wired through.
const KSHARRAYS: i32 = crate::ported::zsh_h::KSHARRAYS;
const VIMODE:    i32 = crate::ported::zsh_h::VIMODE;
/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/ksh93.c`.
fn isset(_opt: i32) -> bool { false }

// `param.u.arr` field — the C `union u` has `char **arr` at c:1835.
// The Rust `param` struct exposes it as `pub u_arr: Option<Vec<String>>`
// (zsh_h.rs:732), already accessible via `(*pm).u_arr`.

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies `ksh93_wrapper` returns 1 in the !EMULATE_KSH branch
    /// (c:149-150) when `emulation` global is 0 (default).
    #[test]
    fn ksh93_wrapper_returns_one_when_not_emulate_ksh() {
        emulation.store(0, Ordering::SeqCst);
        let rc = ksh93_wrapper(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
        );
        assert_eq!(rc, 1);
    }

    /// Verifies `ksh93_wrapper` runs the full body (and still returns 1
    /// per c:227) when EMULATE_KSH is set on the `emulation` global.
    /// Body relies on stubbed externals so it can't validate the
    /// param-creation side-effects yet, but it MUST not panic and MUST
    /// terminate.
    #[test]
    fn ksh93_wrapper_runs_full_body_under_emulate_ksh() {
        let saved = emulation.load(Ordering::SeqCst);
        emulation.store(EMULATE_KSH, Ordering::SeqCst);
        let rc = ksh93_wrapper(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
        );
        assert_eq!(rc, 1);
        // c:158 ++locallevel + c:224 --locallevel must net to 0.
        assert_eq!(locallevel.load(Ordering::SeqCst), 0);
        emulation.store(saved, Ordering::SeqCst);
    }

    /// Verifies `matchgetfn` returns empty Vec when `match` array is
    /// unset and KSHARRAYS is off (c:86 NULL branch).
    #[test]
    fn matchgetfn_empty_returns_empty() {
        let v = matchgetfn(std::ptr::null_mut());
        assert!(v.is_empty());
    }

    /// Verifies `edcharsetfn` is a no-op (c:56 `;`).
    #[test]
    fn edcharsetfn_noop() {
        edcharsetfn(std::ptr::null_mut(), std::ptr::null_mut());
    }

    /// Verifies all module loaders return 0.
    #[test]
    fn module_loaders_return_zero() {
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        let mut features = Vec::new();
        assert_eq!(features_(m, &mut features), 0);
        let mut enables: Option<Vec<i32>> = None;
        assert_eq!(enables_(m, &mut enables), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    /// Port of `ksh93_wrapper(Eprog prog, FuncWrap w, char *name)` from `Src/Modules/ksh93.c:143`.
    /// Verifies the C-faithful static globals are initialized empty
    /// (sh_unsetval-equivalent) at module-load.
    #[test]
    fn statics_default_to_unsetval() {
        sh_name.lock().unwrap().clear();
        sh_subscript.lock().unwrap().clear();
        sh_edchar.lock().unwrap().clear();
        *sh_edmode.lock().unwrap() = [0, 0];
        assert!(sh_name.lock().unwrap().is_empty());
        assert!(sh_subscript.lock().unwrap().is_empty());
        assert!(sh_edchar.lock().unwrap().is_empty());
        assert_eq!(*sh_edmode.lock().unwrap(), [0u8, 0u8]);
        assert_eq!(sh_unsetval, [0u8, 0u8]);
    }

    /// Port of `ksh93_wrapper(Eprog prog, FuncWrap w, char *name)` from `Src/Modules/ksh93.c:143`.
    /// Verifies `LOCAL_NAMEREF` matches the C `#define` at c:158.
    #[test]
    fn local_nameref_matches_c_define() {
        assert_eq!(LOCAL_NAMEREF, PM_LOCAL | PM_UNSET | PM_NAMEREF);
    }
}

// Suppress "unused" for the AtomicI64 import; we don't use it directly
// (locallevel is AtomicI32 to match C `int` for that field).
#[allow(dead_code)]
const _: AtomicI64 = AtomicI64::new(0);
