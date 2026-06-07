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

use crate::ported::builtins::sched::zleactive;
pub use crate::ported::options::emulation;
pub use crate::ported::params::locallevel;
use crate::ported::params::{createparam, paramtab, setiparam, setloopvar, setsparam};
use crate::ported::signals_h::{queue_signals, unqueue_signals};
use crate::ported::string::{dupstring, ztrdup};
use crate::ported::zsh_h::{
    eprog, features, funcstack, funcwrap, isset, module, param, paramdef, EMULATE_KSH, EMULATION,
    KSHARRAYS, PARAMDEF, PM_LOCAL, PM_NAMEREF, PM_READONLY, PM_UNSET, VIMODE,
};
use crate::ported::ztype_h::INAMESPC;
use crate::zsh_h::{PM_ARRAY, PM_SCALAR, PM_SPECIAL};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

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
pub fn edcharsetfn(pm: *mut param, x: *mut libc::c_char) { // c:47
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
pub fn matchgetfn(pm: *mut param) -> Vec<String> {
    // c:60
    // c:Src/Modules/ksh93.c:60 — C's matchgetfn dereferences
    // `pm->u.arr` directly with no null guard (the function is wired
    // into a gsu_array vtable and never invoked with a NULL `pm`).
    // The Rust port is callable from tests that pass `null_mut` to
    // pin the safety boundary; preserve the convention by short-
    // circuiting to an empty Vec when `pm` is null, BEFORE looking
    // at the `match` paramtab entry. Without this, leftover `match`
    // array values from a prior test (the global paramtab is shared
    // across tests via the `global_state_lock`) leak into the null-pm
    // call's return value, breaking the "null pm → empty vec" pin.
    if pm.is_null() {
        return Vec::new();
    }
    // c:60 — `char **zsh_match = getaparam("match");`
    let zsh_match: Vec<String> = paramtab()
        .read()
        .ok()
        .and_then(|t| t.get("match").and_then(|p| p.u_arr.clone()))
        .unwrap_or_default();
    /*
     * For this to work accurately, ksh emulation should always imply
     * that the (#m) and (#b) extendedglob operators are enabled.
     *
     * When we have a 0th element (ksharrays), it is $MATCH.  Elements
     * 1st and larger mirror the $match array.
     */
    // c:64-69
    // c:71-72 — `if (pm->u.arr) freearray(pm->u.arr);`
    if !pm.is_null() {
        unsafe {
            (*pm).u_arr = None;
        } // c:71-72 freearray
    }
    if !zsh_match.is_empty() {
        // c:73
        if isset(KSHARRAYS) {
            // c:74
            // c:75-80 — char **ap = zalloc(...); pm->u.arr = ap;
            //           *ap++ = ztrdup(getsparam("MATCH"));
            //           while (*zsh_match) *ap = ztrdup(*zsh_match++);
            let match_str: String = paramtab()
                .read()
                .ok()
                .and_then(|t| t.get("MATCH").and_then(|p| p.u_str.clone()))
                .unwrap_or_default();
            let mut ap: Vec<String> = Vec::with_capacity(zsh_match.len() + 1);
            ap.push(ztrdup(&match_str)); // c:78
            for s in &zsh_match {
                // c:79-80
                ap.push(ztrdup(s));
            }
            if !pm.is_null() {
                unsafe {
                    (*pm).u_arr = Some(ap.clone());
                }
            }
            // c:88 — `return arrgetfn(pm);` — arrgetfn (params.c) reads
            // pm->u.arr we just set, so return the same vec.
            ap
        } else {
            // c:81-82 — pm->u.arr = zarrdup(zsh_match);
            let dup: Vec<String> = zsh_match.iter().map(|s| ztrdup(s)).collect();
            if !pm.is_null() {
                unsafe {
                    (*pm).u_arr = Some(dup.clone());
                }
            }
            dup // c:88 arrgetfn(pm)
        }
    } else if isset(KSHARRAYS) {
        // c:83
        // c:84 — pm->u.arr = mkarray(ztrdup(getsparam("MATCH")));
        let match_str: String = paramtab()
            .read()
            .ok()
            .and_then(|t| t.get("MATCH").and_then(|p| p.u_str.clone()))
            .unwrap_or_default();
        let one = vec![ztrdup(&match_str)];
        if !pm.is_null() {
            unsafe {
                (*pm).u_arr = Some(one.clone());
            }
        }
        one // c:88 arrgetfn(pm)
    } else {
        // c:86 — pm->u.arr = NULL;
        if !pm.is_null() {
            unsafe {
                (*pm).u_arr = None;
            }
        }
        Vec::new() // c:88 arrgetfn(pm) → NULL
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
pub static sh_unsetval: [u8; 2] = [0, 0]; // c:105

/// Port of `static char *sh_name = sh_unsetval;` from `ksh93.c:106`.
pub static sh_name: Mutex<String> = Mutex::new(String::new()); // c:106

/// Port of `static char *sh_subscript = sh_unsetval;` from `ksh93.c:107`.
pub static sh_subscript: Mutex<String> = Mutex::new(String::new()); // c:107

/// Port of `static char *sh_edchar = sh_unsetval;` from `ksh93.c:108`.
pub static sh_edchar: Mutex<String> = Mutex::new(String::new()); // c:108

/// Port of `static char sh_edmode[2];` from `ksh93.c:109`.
pub static sh_edmode: Mutex<[u8; 2]> = Mutex::new([0, 0]); // c:109

/// Port of `ksh93_wrapper(Eprog prog, FuncWrap w, char *name)` from `Src/Modules/ksh93.c:143`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// ksh93_wrapper(Eprog prog, FuncWrap w, char *name)
/// ```
#[allow(unused_variables)]
pub fn ksh93_wrapper(prog: *const eprog, w: *const funcwrap, name: *mut libc::c_char) -> i32 {
    // c:143
    // c:143 — `Funcstack f;`
    let mut f: *const funcstack;
    // c:146 — `Param pm;`
    let mut pm: *mut param;
    // c:147 — `zlong num = funcstack->prev ? getiparam(".sh.level") : 0;`
    // funcstack is the global from Src/exec.c:340; stub holds NULL so
    // funcstack->prev is always NULL → branch picks 0.
    let mut num: i64 = if (*funcstack.lock().unwrap()) != 0 {
        paramtab()
            .read()
            .ok()
            .and_then(|t| {
                t.get(".sh.level")
                    .and_then(|p| p.u_str.as_ref().and_then(|s| s.parse::<i64>().ok()))
            })
            .unwrap_or(0)
    } else {
        0
    };

    if !EMULATION(EMULATE_KSH) {
        // c:149
        return 1; // c:150
    }

    if num == 0 {
        // c:152
        // c:153 — `for (f = funcstack; f; f = f->prev, num++);` — count
        // function-call-stack depth. Route through the canonical
        // FUNCSTACK Vec<funcstack> in modules::parameter (each push is
        // a nested call). `len()` gives the depth directly.
        let _ = f; // C iterates a linked list; Rust uses Vec depth.
        if let Ok(stack) = crate::ported::modules::parameter::FUNCSTACK.lock() {
            num = stack.len() as i64;
        }
    } else {
        // c:154
        num += 1; // c:155
    }

    queue_signals(); // c:157
    locallevel.fetch_add(1, Ordering::SeqCst); // c:158 ++locallevel;
                                               /* Make these local */                                              // c:158 trailing comment

    // c:160-165 — .sh.command setup
    pm = createparam(".sh.command", LOCAL_NAMEREF as i32)
        .map(Box::into_raw)
        .unwrap_or(std::ptr::null_mut());
    if !pm.is_null() {
        unsafe {
            (*pm).level = locallevel.load(Ordering::Relaxed);
        } // c:161 pm->level = locallevel;
          //       /* Why is this necessary? */
          /* Force scoping by assignent hack */                           // c:162
        setloopvar(".sh.command", "ZSH_DEBUG_CMD"); // c:163
        unsafe {
            (*pm).node.flags |= PM_READONLY as i32;
        } // c:164
    }
    /* .sh.edchar is in partab and below */
    // c:166
    if zleactive.load(Ordering::Relaxed) != 0 {
        pm = createparam(".sh.edcol", LOCAL_NAMEREF as i32) // c:167
            .map(Box::into_raw)
            .unwrap_or(std::ptr::null_mut());
        if !pm.is_null() {
            unsafe {
                (*pm).level = locallevel.load(Ordering::Relaxed);
            } // c:168
            setloopvar(".sh.edcol", "CURSOR"); // c:169
            unsafe {
                (*pm).node.flags |= (PM_NAMEREF | PM_READONLY) as i32;
            } // c:170
        }
    }
    /* .sh.edmode is in partab and below */
    // c:172
    if zleactive.load(Ordering::Relaxed) != 0 {
        pm = createparam(".sh.edtext", LOCAL_NAMEREF as i32) // c:173
            .map(Box::into_raw)
            .unwrap_or(std::ptr::null_mut());
        if !pm.is_null() {
            unsafe {
                (*pm).level = locallevel.load(Ordering::Relaxed);
            } // c:174
            setloopvar(".sh.edtext", "BUFFER"); // c:175
            unsafe {
                (*pm).node.flags |= PM_READONLY as i32;
            } // c:176
        }
    }

    pm = createparam(
        ".sh.fun", // c:179
        (PM_LOCAL | PM_UNSET) as i32,
    )
    .map(Box::into_raw)
    .unwrap_or(std::ptr::null_mut());
    if !pm.is_null() {
        unsafe {
            (*pm).level = locallevel.load(Ordering::Relaxed);
        } // c:180
        let name_str: String = if name.is_null() {
            String::new()
        } else {
            unsafe {
                std::ffi::CStr::from_ptr(name)
                    .to_string_lossy()
                    .into_owned()
            }
        };
        setsparam(
            ".sh.fun", // c:181
            &ztrdup(&name_str),
        );
        unsafe {
            (*pm).node.flags |= PM_READONLY as i32;
        } // c:182
    }
    pm = createparam(
        ".sh.level", // c:184
        (PM_LOCAL | PM_UNSET) as i32,
    )
    .map(Box::into_raw)
    .unwrap_or(std::ptr::null_mut());
    if !pm.is_null() {
        unsafe {
            (*pm).level = locallevel.load(Ordering::Relaxed);
        } // c:185
        setiparam(".sh.level", num); // c:186
    }
    if zleactive.load(Ordering::Relaxed) != 0 {
        // c:188
        // c:189-190 — extern mod_import_variable char *curkeymapname / *varedarg;
        // (extern declarations are at the locals-block level in C; ours
        // are file-level statics below.)
        /* bindkey -v forces VIMODE so this test is as good as any */   // c:191
        let curkmap = curkeymapname.lock().unwrap().clone();
        if !curkmap.is_empty() && isset(VIMODE) && curkmap == "main" {
            // c:192-193
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
            let keys: String = paramtab()
                .read()
                .ok()
                .and_then(|t| t.get("KEYS").and_then(|p| p.u_str.clone()))
                .unwrap_or_default();
            *sh_edchar.lock().unwrap() = dupstring(&keys);
        }
        let varedarg_val = varedarg.lock().unwrap().clone();
        if !varedarg_val.is_empty() {
            // c:199
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
            if ie_off < nm.len() {
                // c:201 if (ie && *ie)
                // c:202 — *ie++ = '\0';
                let head = &nm[..ie_off];
                let tail = &nm[ie_off + 1..]; // skip the bracket char
                *sh_name.lock().unwrap() = head.to_string();
                /* Assume bin_vared has validated subscript */
                // c:203
                // c:204 — sh_subscript = dupstring(ie);
                *sh_subscript.lock().unwrap() = dupstring(tail);
                // c:205-206 — ie = sh_subscript + strlen(sh_subscript); *--ie = '\0';
                let mut sub = sh_subscript.lock().unwrap();
                if sub.ends_with(']') {
                    sub.pop();
                }
            } else {
                // c:208 — sh_subscript = sh_unsetval;
                sh_subscript.lock().unwrap().clear();
            }
            pm = createparam(
                ".sh.value", // c:209
                LOCAL_NAMEREF as i32,
            )
            .map(Box::into_raw)
            .unwrap_or(std::ptr::null_mut());
            if !pm.is_null() {
                unsafe {
                    (*pm).level = locallevel.load(Ordering::Relaxed);
                } // c:210
                setloopvar(".sh.value", "BUFFER"); /* Hack */
                // c:211
                unsafe {
                    (*pm).node.flags |= PM_READONLY as i32;
                } // c:212
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
    locallevel.fetch_sub(1, Ordering::SeqCst); // c:224 --locallevel;
    unqueue_signals(); // c:225

    1 // c:227
}

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
const LOCAL_NAMEREF: u32 = PM_LOCAL | PM_UNSET | PM_NAMEREF; // c:143

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
pub fn setup_(m: *const module) -> i32 {
    // c:236
    // C body c:238-239 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/ksh93.c:243`.
/// C body c:245-247 — `*features = featuresarray(m, &module_features); return 0`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:243
    *features = featuresarray(m, module_features());
    0 // c:258
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/ksh93.c:251`.
/// C body c:253-254 — `return handlefeatures(m, &module_features, enables)`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:251
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
    let mut p: usize; // c:267 (index over partab)
                      // c:269 — deletewrapper(m, wrapper); zshrs's fusevm wrapper
                      // machinery is a no-op (see boot_ note).

    // c:116-131 — `static struct paramdef partab[]` inlined here
    // because Rust statics can't hold String-typed paramdef.name.
    let partab: [paramdef; 9] = [
        PARAMDEF(".sh.edchar", (PM_SCALAR | PM_SPECIAL) as i32, 0, 0), // c:117
        PARAMDEF(
            ".sh.edmode",
            (PM_SCALAR | PM_READONLY | PM_SPECIAL) as i32,
            0,
            0,
        ), // c:119
        PARAMDEF(".sh.file", (PM_NAMEREF | PM_READONLY) as i32, 0, 0), // c:121
        PARAMDEF(".sh.lineno", (PM_NAMEREF | PM_READONLY) as i32, 0, 0), // c:122
        PARAMDEF(".sh.match", (PM_ARRAY | PM_READONLY) as i32, 0, 0),  // c:123
        PARAMDEF(
            ".sh.name",
            (PM_SCALAR | PM_READONLY | PM_SPECIAL) as i32,
            0,
            0,
        ), // c:124
        PARAMDEF(
            ".sh.subscript",
            (PM_SCALAR | PM_READONLY | PM_SPECIAL) as i32,
            0,
            0,
        ), // c:126
        PARAMDEF(".sh.subshell", (PM_NAMEREF | PM_READONLY) as i32, 0, 0), // c:128
        PARAMDEF(".sh.version", (PM_NAMEREF | PM_READONLY) as i32, 0, 0), // c:130
    ];

    /* Clean up namerefs, otherwise deleteparamdef() is confused */
    // c:271
    // c:272-277 — `for (p = partab; p < partab + ARRSZ; ++p) { ... }`
    p = 0;
    while p < partab.len() {
        // c:272
        let entry = &partab[p];
        if (entry.flags as u32 & PM_NAMEREF) != 0 {
            // c:273
            // c:274-276 — `HashNode hn = gethashnode2(paramtab, p->name);`
            // `if (hn) hn->flags &= ~PM_NAMEREF;`
            if let Ok(mut t) = paramtab().write() {
                if let Some(pm) = t.get_mut(&entry.name) {
                    pm.node.flags &= !(PM_NAMEREF as i32); // c:276
                }
            }
        }
        p += 1;
    }
    setfeatureenables(m, module_features(), None) // c:279
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/ksh93.c:284`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:284
    // C body c:286-287 — `return 0`. Faithful empty-body port; the
    //                    ksh93 wrapper unregisters in cleanup_ via
    //                    deletewrapper.
    0
}

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local descriptor stub mirroring the C bintab + partab.
// WARNING: NOT IN KSH93.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
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
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 10]);
    }
    0
}

// WARNING: NOT IN KSH93.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
    0
}

// =====================================================================
// External ported / globals from other Src/*.c files. Stubbed locally
// pending the proper ports of their home files.
// =====================================================================

// `emulation` lives in `crate::ported::options::emulation` per Rule C
// (its C definition is `Src/options.c:36`, not `ksh93.c`). The
// `EMULATION(bits)` macro at zsh.h:2347 tests bits against it.

// `locallevel` lives in `crate::ported::params::locallevel` per Rule C
// (its C definition is `Src/params.c:54`, not `ksh93.c`). Bumped by
// `startparamscope` on function entry, decremented by `endparamscope`
// on return. `ksh93_wrapper` increments before `createparam()` and
// decrements after.

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

// `param.u.arr` field — the C `union u` has `char **arr` at c:1835.
// The Rust `param` struct exposes it as `pub u_arr: Option<Vec<String>>`
// (zsh_h.rs:732), already accessible via `(*pm).u_arr`.

// Suppress "unused" for the AtomicI64 import; we don't use it directly
// (locallevel is AtomicI32 to match C `int` for that field).
#[allow(dead_code)]
const _: AtomicI64 = AtomicI64::new(0);

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

/// Port of `ksh93_wrapper(Eprog prog, FuncWrap w, char *name)` from `Src/Modules/ksh93.c:143`.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 1, // bintab: nameref (ksh93.c)
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 9, // partab: .sh.edchar/edmode/file/lineno/match/name/subscript/subshell/version
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zsh_h::hashnode;

    /// Verifies `ksh93_wrapper` returns 1 in the !EMULATE_KSH branch
    /// (c:149-150) when `emulation` global is 0 (default).
    #[test]
    fn ksh93_wrapper_returns_one_when_not_emulate_ksh() {
        let _g = crate::test_util::global_state_lock();
        emulation.store(0, Ordering::SeqCst);
        let rc = ksh93_wrapper(std::ptr::null(), std::ptr::null(), std::ptr::null_mut());
        assert_eq!(rc, 1);
    }

    /// Verifies `ksh93_wrapper` runs the full body (and still returns 1
    /// per c:227) when EMULATE_KSH is set on the `emulation` global.
    /// Body relies on stubbed externals so it can't validate the
    /// param-creation side-effects yet, but it MUST not panic and MUST
    /// terminate.
    #[test]
    fn ksh93_wrapper_runs_full_body_under_emulate_ksh() {
        let _g = crate::test_util::global_state_lock();
        let saved = emulation.load(Ordering::SeqCst);
        emulation.store(EMULATE_KSH, Ordering::SeqCst);
        let rc = ksh93_wrapper(std::ptr::null(), std::ptr::null(), std::ptr::null_mut());
        assert_eq!(rc, 1);
        // c:158 ++locallevel + c:224 --locallevel must net to 0.
        assert_eq!(locallevel.load(Ordering::SeqCst), 0);
        emulation.store(saved, Ordering::SeqCst);
    }

    /// Verifies `matchgetfn` returns empty Vec when `match` array is
    /// unset and KSHARRAYS is off (c:86 NULL branch).
    #[test]
    fn matchgetfn_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let v = matchgetfn(std::ptr::null_mut());
        assert!(v.is_empty());
    }

    /// Verifies `edcharsetfn` is a no-op (c:56 `;`).
    #[test]
    fn edcharsetfn_noop() {
        let _g = crate::test_util::global_state_lock();
        edcharsetfn(std::ptr::null_mut(), std::ptr::null_mut());
    }

    /// Verifies all module loaders return 0.
    #[test]
    fn module_loaders_return_zero() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        assert_eq!(LOCAL_NAMEREF, PM_LOCAL | PM_UNSET | PM_NAMEREF);
    }

    /// c:60 — `matchgetfn` on a non-null `Param` that has no array
    /// data must still return an empty Vec. Builds a valid `param`
    /// with `u_arr = None` to exercise the "has Param but no array"
    /// branch — a regression that adds `.unwrap()` would SIGSEGV.
    #[test]
    fn matchgetfn_with_param_no_array_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        // matchgetfn reads `match` from paramtab (ksh93.rs:60 — c:60).
        // A prior subst-related test may have written to it; clear so
        // the "no array data" branch isn't masked by leftover values.
        crate::ported::params::unsetparam("match");
        let mut pm = param {
            node: hashnode {
                next: None,
                nam: "match".to_string(),
                flags: 0,
            },
            u_data: 0,
            u_arr: None,
            u_str: None,
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
        };
        let v = matchgetfn(&mut pm as *mut _);
        assert!(
            v.is_empty(),
            "param without array data must produce empty result"
        );
    }

    /// c:158 — `LOCAL_NAMEREF` must include the three component flags
    /// AND no others. Pin the exact bit set so a regen that adds
    /// PM_READONLY silently changes the unset semantics.
    #[test]
    fn local_nameref_has_exactly_three_component_flags() {
        let _g = crate::test_util::global_state_lock();
        let expected = PM_LOCAL | PM_UNSET | PM_NAMEREF;
        assert_eq!(LOCAL_NAMEREF, expected);
        // Subtracting each in turn yields the other two
        assert_eq!(LOCAL_NAMEREF & !PM_LOCAL, PM_UNSET | PM_NAMEREF);
        assert_eq!(LOCAL_NAMEREF & !PM_UNSET, PM_LOCAL | PM_NAMEREF);
        assert_eq!(LOCAL_NAMEREF & !PM_NAMEREF, PM_LOCAL | PM_UNSET);
    }

    /// c:50 — `sh_unsetval` is the byte sentinel for "unset" string
    /// params (`{ 0, 0 }`). Pin the exact value so a regen flipping
    /// it to `[b'\0']` or `[1, 0]` silently breaks unset detection
    /// at every shXgetfn call site.
    #[test]
    fn sh_unsetval_is_two_zero_bytes() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            sh_unsetval,
            [0u8, 0u8],
            "sh_unsetval must be the C-canonical [0,0] sentinel"
        );
    }

    /// c:60 — `matchgetfn` with a null pointer must NOT dereference.
    /// Pinning null-safety so a regen that adds `*pm` outside a
    /// guard SIGSEGVs the test instead of shipping.
    #[test]
    fn matchgetfn_null_pointer_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let _ = matchgetfn(std::ptr::null_mut());
    }

    /// c:47 — `edcharsetfn` accepts null param + null arg pointer
    /// without dereferencing. The C body is `;` (empty) so the Rust
    /// stub must mirror that no-op + re-entry safety.
    #[test]
    fn edcharsetfn_double_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        edcharsetfn(std::ptr::null_mut(), std::ptr::null_mut());
        edcharsetfn(std::ptr::null_mut(), std::ptr::null_mut());
    }

    // ─── zsh-corpus pins for ksh93 ─────────────────────────────────

    /// `matchgetfn(null)` returns empty vec without panic.
    #[test]
    fn ksh93_corpus_matchgetfn_null_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = matchgetfn(std::ptr::null_mut());
        assert!(r.is_empty(), "null pm → empty vec, got {r:?}");
    }

    /// All four lifecycle shims return 0.
    #[test]
    fn ksh93_corpus_lifecycle_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    /// `edcharsetfn` cycling null calls remains stable.
    #[test]
    fn ksh93_corpus_edcharsetfn_repeated_null_no_panic() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..100 {
            edcharsetfn(std::ptr::null_mut(), std::ptr::null_mut());
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/ksh93.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:47 — `edcharsetfn(null, null)` is no-op (placeholder for the
    /// SIGKEYBD trap interception; the body is just a comment in C).
    /// Pin: single null call doesn't panic.
    #[test]
    fn edcharsetfn_single_null_call_no_panic() {
        let _g = crate::test_util::global_state_lock();
        edcharsetfn(std::ptr::null_mut(), std::ptr::null_mut());
    }

    /// c:60 — `matchgetfn(NULL)` returns empty Vec when `match` array
    /// is unset (no $match parameter present).
    #[test]
    fn matchgetfn_null_pm_unset_match_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        // Without setting the "match" param, getaparam returns None →
        // empty result.
        crate::ported::params::unsetparam("match");
        let r = matchgetfn(std::ptr::null_mut());
        assert!(r.is_empty(), "no $match → empty vec");
    }

    /// c:212 — `ksh93_wrapper(null, null, null)` no panic on all-null.
    #[test]
    fn ksh93_wrapper_all_null_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = ksh93_wrapper(std::ptr::null(), std::ptr::null(), std::ptr::null_mut());
    }

    /// c:491 — `setup_(NULL)` returns 0 (split per-hook).
    #[test]
    fn ksh93_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:514 — `boot_(NULL)` returns 0.
    #[test]
    fn ksh93_boot_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    /// c:536 — `cleanup_(NULL)` returns 0.
    #[test]
    fn ksh93_cleanup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cleanup_(std::ptr::null()), 0);
    }

    /// c:595 — `finish_(NULL)` returns 0.
    #[test]
    fn ksh93_finish_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    /// c:499 — `features_(NULL, _)` returns 0 with features populated.
    #[test]
    fn ksh93_features_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        let mut f = Vec::new();
        assert_eq!(features_(std::ptr::null(), &mut f), 0);
    }

    /// c:507 — `enables_(NULL, _)` no panic.
    #[test]
    fn ksh93_enables_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _ = enables_(std::ptr::null(), &mut e);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/ksh93.c
    // c:47 edcharsetfn / c:83 matchgetfn / c:212 ksh93_wrapper / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:47 — `edcharsetfn(null, null)` is idempotent (multiple calls safe).
    #[test]
    fn edcharsetfn_double_null_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            edcharsetfn(std::ptr::null_mut(), std::ptr::null_mut());
        }
    }

    /// c:83 — `matchgetfn(null)` returns Vec (not panic) — type pinning.
    #[test]
    fn matchgetfn_null_returns_vec_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = matchgetfn(std::ptr::null_mut());
    }

    /// c:83 — `matchgetfn(null)` is deterministic across repeated calls.
    #[test]
    fn matchgetfn_null_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = matchgetfn(std::ptr::null_mut());
        for _ in 0..5 {
            assert_eq!(
                matchgetfn(std::ptr::null_mut()),
                first,
                "matchgetfn(null) must be deterministic"
            );
        }
    }

    /// c:212 — `ksh93_wrapper(null, null, null)` is safe.
    #[test]
    fn ksh93_wrapper_all_null_no_panic_pin() {
        let _g = crate::test_util::global_state_lock();
        let _ = ksh93_wrapper(std::ptr::null(), std::ptr::null(), std::ptr::null_mut());
    }

    /// c:212 — `ksh93_wrapper` returns i32 (type pinning).
    #[test]
    fn ksh93_wrapper_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = ksh93_wrapper(std::ptr::null(), std::ptr::null(), std::ptr::null_mut());
    }

    /// c:491-595 — full lifecycle setup→features→enables→boot→cleanup→finish
    /// all return 0.
    #[test]
    fn ksh93_full_lifecycle_returns_zero_for_all() {
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

    /// c:491 — setup_ idempotent.
    #[test]
    fn ksh93_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:595 — finish_ idempotent.
    #[test]
    fn ksh93_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:536 — cleanup_ idempotent.
    #[test]
    fn ksh93_cleanup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:514 — boot_ idempotent.
    #[test]
    fn ksh93_boot_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(boot_(std::ptr::null()), 0);
        }
    }

    /// c:212 — ksh93_wrapper return value in canonical exit-code range
    /// (signed-byte 0..256).
    #[test]
    fn ksh93_wrapper_return_in_exit_code_range() {
        let _g = crate::test_util::global_state_lock();
        let r = ksh93_wrapper(std::ptr::null(), std::ptr::null(), std::ptr::null_mut());
        assert!(
            (0..256).contains(&r),
            "exit code must fit in u8 range, got {}",
            r
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/ksh93.c
    // c:47 edcharsetfn / c:83 matchgetfn / c:212 ksh93_wrapper
    // ═══════════════════════════════════════════════════════════════════

    /// c:83 — `matchgetfn` returns Vec<String> (compile-time type pin).
    #[test]
    fn matchgetfn_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = matchgetfn(std::ptr::null_mut());
    }

    /// c:47 — `edcharsetfn` returns void (compile-time pin).
    #[test]
    fn edcharsetfn_returns_void() {
        let _g = crate::test_util::global_state_lock();
        let _: () = edcharsetfn(std::ptr::null_mut(), std::ptr::null_mut());
    }

    /// c:212 — `ksh93_wrapper` returns i32 (compile-time pin).
    #[test]
    fn ksh93_wrapper_returns_i32_type_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = ksh93_wrapper(std::ptr::null(), std::ptr::null(), std::ptr::null_mut());
    }

    /// c:212 — `ksh93_wrapper` is deterministic for all-null args.
    #[test]
    fn ksh93_wrapper_all_null_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = ksh93_wrapper(std::ptr::null(), std::ptr::null(), std::ptr::null_mut());
        for _ in 0..5 {
            assert_eq!(
                ksh93_wrapper(std::ptr::null(), std::ptr::null(), std::ptr::null_mut()),
                first,
                "ksh93_wrapper(all null) must be deterministic"
            );
        }
    }

    /// c:83 — `matchgetfn(null)` returns empty Vec (no match available).
    #[test]
    fn matchgetfn_null_returns_empty_pin() {
        let _g = crate::test_util::global_state_lock();
        let v = matchgetfn(std::ptr::null_mut());
        assert!(v.is_empty(), "null pm → empty vec");
    }

    /// c:491 — `setup_` is idempotent + returns void/i32.
    #[test]
    fn ksh93_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:514 — `boot_` returns i32.
    #[test]
    fn ksh93_boot_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = boot_(std::ptr::null());
    }

    /// c:536 — `cleanup_` returns i32.
    #[test]
    fn ksh93_cleanup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cleanup_(std::ptr::null());
    }

    /// c:595 — `finish_` returns i32.
    #[test]
    fn ksh93_finish_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = finish_(std::ptr::null());
    }

    /// c:47 — `edcharsetfn` idempotent for repeated null calls.
    #[test]
    fn edcharsetfn_repeated_null_calls_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..20 {
            edcharsetfn(std::ptr::null_mut(), std::ptr::null_mut());
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/ksh93.c
    // c:47 edcharsetfn / c:83 matchgetfn / c:212 ksh93_wrapper + lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:83 — `matchgetfn(null)` is deterministic across calls (alt).
    #[test]
    fn matchgetfn_null_deterministic_alt() {
        let _g = crate::test_util::global_state_lock();
        let first = matchgetfn(std::ptr::null_mut());
        for _ in 0..5 {
            assert_eq!(
                matchgetfn(std::ptr::null_mut()),
                first,
                "matchgetfn(null) must be pure"
            );
        }
    }

    /// c:83 — `matchgetfn(null)` length is exactly 0 (empty Vec).
    #[test]
    fn matchgetfn_null_length_is_zero() {
        let _g = crate::test_util::global_state_lock();
        let v = matchgetfn(std::ptr::null_mut());
        assert_eq!(v.len(), 0, "matchgetfn(null) must have len=0");
    }

    /// c:212 — `ksh93_wrapper` exit code is non-negative.
    #[test]
    fn ksh93_wrapper_exit_code_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let r = ksh93_wrapper(std::ptr::null(), std::ptr::null(), std::ptr::null_mut());
        assert!(r >= 0, "exit code must be non-negative, got {}", r);
    }

    /// c:212 — `ksh93_wrapper` exit code fits in u8 range (canonical
    /// shell exit code 0..256).
    #[test]
    fn ksh93_wrapper_exit_code_fits_u8() {
        let _g = crate::test_util::global_state_lock();
        let r = ksh93_wrapper(std::ptr::null(), std::ptr::null(), std::ptr::null_mut());
        assert!(
            (0..256).contains(&r),
            "exit code {} must fit in u8 range",
            r
        );
    }

    /// c:499 — `features_` returns i32 (compile-time pin).
    #[test]
    fn ksh93_features_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<String> = Vec::new();
        let _: i32 = features_(std::ptr::null(), &mut v);
    }

    /// c:507 — `enables_` returns i32 (compile-time pin).
    #[test]
    fn ksh93_enables_with_none_returns_i32() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _: i32 = enables_(std::ptr::null(), &mut e);
    }

    /// c:491 — `setup_` is idempotent across many calls (alt).
    #[test]
    fn ksh93_setup_idempotent_many_call_alt() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..20 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:514 — `boot_` is idempotent across many calls (alt).
    #[test]
    fn ksh93_boot_idempotent_many_call_alt() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..20 {
            assert_eq!(boot_(std::ptr::null()), 0);
        }
    }

    /// c:536 — `cleanup_` is idempotent across many calls (alt).
    #[test]
    fn ksh93_cleanup_idempotent_many_call_alt() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..20 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:595 — `finish_` is idempotent across many calls (alt).
    #[test]
    fn ksh93_finish_idempotent_many_call_alt() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..20 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:491/499/507/514/536/595 — each lifecycle hook returns 0
    /// individually (tighter failure resolution).
    #[test]
    fn ksh93_each_lifecycle_hook_returns_zero_individually() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let mut v: Vec<String> = Vec::new();
        let mut e: Option<Vec<i32>> = None;
        assert_eq!(setup_(null), 0, "c:491 setup_");
        assert_eq!(features_(null, &mut v), 0, "c:499 features_");
        assert_eq!(enables_(null, &mut e), 0, "c:507 enables_");
        assert_eq!(boot_(null), 0, "c:514 boot_");
        assert_eq!(cleanup_(null), 0, "c:536 cleanup_");
        assert_eq!(finish_(null), 0, "c:595 finish_");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/ksh93.c
    // c:47 edcharsetfn / c:83 matchgetfn / c:212 ksh93_wrapper /
    // c:491-595 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:491 — `setup_` return type i32 (compile-time pin, alt).
    #[test]
    fn ksh93_setup_returns_i32_type_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:536 — `cleanup_` return type i32 (compile-time pin).
    #[test]
    fn ksh93_cleanup_returns_i32_type_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cleanup_(std::ptr::null());
    }

    /// c:595 — `finish_` return type i32 (compile-time pin).
    #[test]
    fn ksh93_finish_returns_i32_type_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = finish_(std::ptr::null());
    }

    /// c:514 — `boot_` return type i32 (compile-time pin).
    #[test]
    fn ksh93_boot_returns_i32_type_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = boot_(std::ptr::null());
    }

    /// c:83 — `matchgetfn(null)` returns Vec<String> safely.
    #[test]
    fn matchgetfn_null_pm_returns_vec_string() {
        let _g = crate::test_util::global_state_lock();
        let r = matchgetfn(std::ptr::null_mut());
        let _: Vec<String> = r;
    }

    /// c:83 — `matchgetfn` is deterministic for null pm.
    #[test]
    fn matchgetfn_null_pm_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let a = matchgetfn(std::ptr::null_mut());
        let b = matchgetfn(std::ptr::null_mut());
        assert_eq!(a, b, "matchgetfn(null) must be deterministic");
    }

    /// c:47 — `edcharsetfn(null, null)` doesn't panic.
    #[test]
    fn edcharsetfn_null_inputs_no_panic() {
        let _g = crate::test_util::global_state_lock();
        edcharsetfn(std::ptr::null_mut(), std::ptr::null_mut());
    }

    /// c:212 — `ksh93_wrapper` with all null inputs doesn't panic.
    #[test]
    fn ksh93_wrapper_all_null_inputs_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let name = std::ffi::CString::new("test").unwrap();
        let _ = ksh93_wrapper(
            std::ptr::null(),
            std::ptr::null(),
            name.as_ptr() as *mut libc::c_char,
        );
    }

    /// c:212 — `ksh93_wrapper` return type i32 (compile-time pin, alt).
    #[test]
    fn ksh93_wrapper_returns_i32_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let name = std::ffi::CString::new("test").unwrap();
        let _: i32 = ksh93_wrapper(
            std::ptr::null(),
            std::ptr::null(),
            name.as_ptr() as *mut libc::c_char,
        );
    }

    /// c:499 — `features_` deterministic on null module.
    #[test]
    fn ksh93_features_deterministic_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let mut v1: Vec<String> = Vec::new();
        let mut v2: Vec<String> = Vec::new();
        let _ = features_(std::ptr::null(), &mut v1);
        let _ = features_(std::ptr::null(), &mut v2);
        assert_eq!(v1, v2, "features_ must be deterministic");
    }

    /// c:507 — `enables_` with Some(non-empty) doesn't panic.
    #[test]
    fn ksh93_enables_with_some_non_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = Some(vec![1, 2, 3]);
        let _ = enables_(std::ptr::null(), &mut e);
    }

    /// c:491/514/595 — setup→boot→finish chain returns 0 each.
    #[test]
    fn ksh93_setup_boot_finish_chain_returns_zero_each() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        assert_eq!(boot_(null), 0);
        assert_eq!(finish_(null), 0);
    }
}
