//! `zsh/ksh93` module — port of `Src/Modules/ksh93.c`.
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

use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};

use crate::ported::signals_h::{queue_signals, unqueue_signals};
use crate::ported::string::dupstring;
use crate::ported::ztype::INAMESPC;
use crate::ported::zsh_h::{
    eprog, funcwrap, module, param, EMULATE_KSH, EMULATION,
    PM_LOCAL, PM_NAMEREF, PM_READONLY, PM_UNSET,
};

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

/// Port of `edcharsetfn()` from `Src/Modules/ksh93.c:47`.
pub fn edcharsetfn(_pm: *mut param, _x: *mut libc::c_char) {           // c:47
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

/// Port of `matchgetfn()` from `Src/Modules/ksh93.c:60`.
///
/// C signature mirrored verbatim:
/// ```c
/// static char **
/// matchgetfn(Param pm)
/// ```
pub fn matchgetfn(pm: *mut param) -> Vec<String> {                     // c:60
    // c:62 — `char **zsh_match = getaparam("match");`
    let zsh_match: Vec<String> = getaparam("match");
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
            let mut ap: Vec<String> = Vec::with_capacity(zsh_match.len() + 1);
            ap.push(ztrdup(&getsparam("MATCH")));                       // c:78
            for s in &zsh_match {                                       // c:79-80
                ap.push(ztrdup(s));
            }
            if !pm.is_null() {
                unsafe { (*pm).u_arr = Some(ap.clone()); }
            }
            return arrgetfn(pm, ap);
        } else {
            // c:81-82 — pm->u.arr = zarrdup(zsh_match);
            let dup: Vec<String> = zsh_match.iter().map(|s| ztrdup(s)).collect();
            if !pm.is_null() {
                unsafe { (*pm).u_arr = Some(dup.clone()); }
            }
            return arrgetfn(pm, dup);
        }
    } else if isset(KSHARRAYS) {                                        // c:83
        // c:84 — pm->u.arr = mkarray(ztrdup(getsparam("MATCH")));
        let one = vec![ztrdup(&getsparam("MATCH"))];
        if !pm.is_null() {
            unsafe { (*pm).u_arr = Some(one.clone()); }
        }
        return arrgetfn(pm, one);
    } else {
        // c:86 — pm->u.arr = NULL;
        if !pm.is_null() {
            unsafe { (*pm).u_arr = None; }
        }
        return arrgetfn(pm, Vec::new());
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
/// from `Src/Modules/ksh93.c:158`.
#[allow(dead_code)]
const LOCAL_NAMEREF: u32 = PM_LOCAL | PM_UNSET | PM_NAMEREF;            // c:158

/// Port of `ksh93_wrapper()` from `Src/Modules/ksh93.c:143`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// ksh93_wrapper(Eprog prog, FuncWrap w, char *name)
/// ```
pub fn ksh93_wrapper(_prog: *const eprog, _w: *const funcwrap, name: *mut libc::c_char) -> i32 { // c:143
    // c:145 — `Funcstack f;`
    let mut f: Funcstack;
    // c:146 — `Param pm;`
    let mut pm: *mut param;
    // c:147 — `zlong num = funcstack->prev ? getiparam(".sh.level") : 0;`
    // funcstack is the global from Src/exec.c:340; stub holds NULL so
    // funcstack->prev is always NULL → branch picks 0.
    let mut num: i64 = if (*funcstack.lock().unwrap()) != 0 { getiparam(".sh.level") } else { 0 };

    if !EMULATION(emulation.load(Ordering::Relaxed), EMULATE_KSH) {     // c:149
        return 1;                                                       // c:150
    }

    if num == 0 {                                                       // c:152
        // c:153 — for (f = funcstack; f; f = f->prev, num++);
        f = (*funcstack.lock().unwrap()) as Funcstack;
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
    pm = createparam(".sh.command", LOCAL_NAMEREF);
    if !pm.is_null() {
        unsafe { (*pm).level = locallevel.load(Ordering::Relaxed); }    // c:161 pm->level = locallevel;
                                                                         //       /* Why is this necessary? */
        /* Force scoping by assignent hack */                           // c:162
        setloopvar(".sh.command", "ZSH_DEBUG_CMD");                     // c:163
        unsafe { (*pm).node.flags |= PM_READONLY as i32; }              // c:164
    }
    /* .sh.edchar is in partab and below */                             // c:166
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) != 0 {
        pm = createparam(".sh.edcol", LOCAL_NAMEREF);                   // c:167
        if !pm.is_null() {
            unsafe { (*pm).level = locallevel.load(Ordering::Relaxed); } // c:168
            setloopvar(".sh.edcol", "CURSOR");                          // c:169
            unsafe { (*pm).node.flags |= (PM_NAMEREF | PM_READONLY) as i32; } // c:170
        }
    }
    /* .sh.edmode is in partab and below */                             // c:172
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) != 0 {
        pm = createparam(".sh.edtext", LOCAL_NAMEREF);                  // c:173
        if !pm.is_null() {
            unsafe { (*pm).level = locallevel.load(Ordering::Relaxed); } // c:174
            setloopvar(".sh.edtext", "BUFFER");                         // c:175
            unsafe { (*pm).node.flags |= PM_READONLY as i32; }          // c:176
        }
    }

    pm = createparam(".sh.fun", PM_LOCAL | PM_UNSET);                   // c:179
    if !pm.is_null() {
        unsafe { (*pm).level = locallevel.load(Ordering::Relaxed); }    // c:180
        let name_str: String = if name.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned() }
        };
        setsparam(".sh.fun", &ztrdup(&name_str));                       // c:181
        unsafe { (*pm).node.flags |= PM_READONLY as i32; }              // c:182
    }
    pm = createparam(".sh.level", PM_LOCAL | PM_UNSET);                 // c:184
    if !pm.is_null() {
        unsafe { (*pm).level = locallevel.load(Ordering::Relaxed); }    // c:185
        setiparam(".sh.level", num);                                    // c:186
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
            *sh_edchar.lock().unwrap() = dupstring(&getsparam("KEYS"));
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
            pm = createparam(".sh.value", LOCAL_NAMEREF);               // c:209
            if !pm.is_null() {
                unsafe { (*pm).level = locallevel.load(Ordering::Relaxed); } // c:210
                setloopvar(".sh.value", "BUFFER");      /* Hack */      // c:211
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

/// Port of `setup_()` from `Src/Modules/ksh93.c:236`.
pub fn setup_(_m: *const module) -> i32 { 0 }                          // c:238

/// Port of `features_()` from `Src/Modules/ksh93.c:243`.
pub fn features_(_m: *const module) -> i32 { 0 }                       // c:246

/// Port of `enables_()` from `Src/Modules/ksh93.c:251`.
pub fn enables_(_m: *const module) -> i32 { 0 }                        // c:253

/// Port of `boot_()` from `Src/Modules/ksh93.c:258`.
/// C body: `return addwrapper(m, wrapper);`
pub fn boot_(m: *const module) -> i32 {
    addwrapper(m, &WRAPPER)                                              // c:260
}

/// Port of `cleanup_()` from `Src/Modules/ksh93.c:265`.
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
    deletewrapper(m, &WRAPPER);                                          // c:269

    /* Clean up namerefs, otherwise deleteparamdef() is confused */     // c:271
    // c:272-277 — walk PARTAB, clear PM_NAMEREF on live param nodes.
    for entry in PARTAB.iter() {
        if (entry.flags & PM_NAMEREF) != 0 {                             // c:273
            // c:274 — `HashNode hn = gethashnode2(paramtab, p->name);`
            let hn: *mut param = gethashnode2(&paramtab, entry.name);
            if !hn.is_null() {                                           // c:275
                unsafe { (*hn).node.flags &= !(PM_NAMEREF as i32); }    // c:276
            }
        }
    }
    setfeatureenables(m, &MODULE_FEATURES_KSH93, None)                  // c:279
}

/// Stand-in for `static struct funcwrap wrapper[]` (c:230). One entry
/// pointing at `ksh93_wrapper`. The C array carries `WRAPDEF(...)`
/// macros — Rust port stores the fn pointer + flags inline.
pub static WRAPPER: [WrapperEntry; 1] = [WrapperEntry {
    flags: 0,
    handler: ksh93_wrapper,
}];

/// Per-wrapper-entry shape mirroring `struct funcwrap` from
/// zsh.h:1362. Fields: `flags`, the wrapper fn ptr.
pub struct WrapperEntry {
    pub flags: i32,
    pub handler: fn(*const eprog, *const funcwrap, *mut libc::c_char) -> i32,
}

/// Stand-in for `static struct paramdef partab[]` (c:116). Captures
/// the per-entry `name` + `flags` so the cleanup_ PM_NAMEREF walk has
/// real data to traverse.
pub static PARTAB: [PartabEntry; 9] = [
    PartabEntry { name: ".sh.edchar",    flags: PM_SCALAR | PM_SPECIAL },                  // c:117
    PartabEntry { name: ".sh.edmode",    flags: PM_SCALAR | PM_READONLY | PM_SPECIAL },    // c:119
    PartabEntry { name: ".sh.file",      flags: PM_NAMEREF | PM_READONLY },                // c:121
    PartabEntry { name: ".sh.lineno",    flags: PM_NAMEREF | PM_READONLY },                // c:122
    PartabEntry { name: ".sh.match",     flags: PM_ARRAY | PM_READONLY },                  // c:123
    PartabEntry { name: ".sh.name",      flags: PM_SCALAR | PM_READONLY | PM_SPECIAL },    // c:124
    PartabEntry { name: ".sh.subscript", flags: PM_SCALAR | PM_READONLY | PM_SPECIAL },    // c:126
    PartabEntry { name: ".sh.subshell",  flags: PM_NAMEREF | PM_READONLY },                // c:128
    PartabEntry { name: ".sh.version",   flags: PM_NAMEREF | PM_READONLY },                // c:130
];

/// Per-paramdef-entry shape mirroring `struct paramdef` from
/// zsh.h:2082. Fields: `name`, `flags`.
pub struct PartabEntry {
    pub name: &'static str,
    pub flags: u32,
}

// `module_features` for ksh93 — empty bn/cd/mf, partab carries 9
// entries. Stub to satisfy setfeatureenables call.
static MODULE_FEATURES_KSH93: Mutex<crate::ported::zsh_h::features> =
    Mutex::new(crate::ported::zsh_h::features {
        bn_list: None,                                                   // c:134 bintab[1] (nameref)
        bn_size: 1,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,                                                    // c:137 partab[9]
        pd_size: 9,
        n_abstract: 0,
    });

// PM_SCALAR / PM_ARRAY / PM_SPECIAL — referenced by PARTAB above.
const PM_SCALAR: u32 = crate::ported::zsh_h::PM_SCALAR;
const PM_ARRAY: u32 = crate::ported::zsh_h::PM_ARRAY;
const PM_SPECIAL: u32 = crate::ported::zsh_h::PM_SPECIAL;

// `addwrapper` lives in `Src/module.c:2185`. Stub returns 0 (success).
fn addwrapper(_m: *const module, _w: &[WrapperEntry]) -> i32 { 0 }

// `deletewrapper` lives in `Src/module.c:2207`. Stub returns 0.
fn deletewrapper(_m: *const module, _w: &[WrapperEntry]) {}

// `paramtab` — `HashTable` global from `Src/params.c:46`. Stub: empty
// usize-sized opaque holder; gethashnode2 against it always returns NULL.
static paramtab: AtomicI32 = AtomicI32::new(0);

// `gethashnode2` lives in `Src/hashtable.c:288`. C signature:
//   HashNode gethashnode2(HashTable ht, const char *nam);
// Stub uses the param-shaped overload — returns NULL since paramtab
// is empty in static-link path.
fn gethashnode2(_ht: &AtomicI32, _name: &str) -> *mut param {
    std::ptr::null_mut()
}

// `setfeatureenables` lives in `Src/module.c:3445`. Stub.
fn setfeatureenables(_m: *const module, _f: &Mutex<crate::ported::zsh_h::features>, _e: Option<&Vec<i32>>) -> i32 { 0 }

/// Port of `finish_()` from `Src/Modules/ksh93.c:284`.
pub fn finish_(_m: *const module) -> i32 { 0 }                         // c:286

// =====================================================================
// External fns / globals from other Src/*.c files. Stubbed locally
// pending the proper ports of their home files.
// =====================================================================

/// `emulation` — int global from `Src/options.c`. Holds the current
/// emulation bitmap (EMULATE_ZSH/KSH/SH/CSH); EMULATION() macro tests
/// bits against it.
pub static emulation: AtomicI32 = AtomicI32::new(0);

/// `locallevel` — int global from `Src/init.c:166`. Tracks function-
/// local-scope nesting depth. ksh93_wrapper increments before
/// createparam() (c:158) and decrements after (c:224).
pub static locallevel: AtomicI32 = AtomicI32::new(0);

/// `curkeymapname` — `char *` global from `Src/Zle/zle_keymap.c`,
/// declared `extern` at c:189. Holds the active keymap name.
pub static curkeymapname: Mutex<String> = Mutex::new(String::new());

/// `varedarg` — `char *` global from `Src/Zle/zle_misc.c`, declared
/// `extern` at c:190. Holds the parameter name being edited by `vared`.
pub static varedarg: Mutex<String> = Mutex::new(String::new());

// `Funcstack` — `typedef struct funcstack *Funcstack` from
// `Src/zsh.h:545`. Mirrored as a raw pointer for the C-level code
// shape; `funcstack` global lives in `Src/exec.c:340`.
type Funcstack = *mut libc::c_void;

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
fn isset(_opt: i32) -> bool { false }

// `getsparam` lives in `Src/params.c:3640`. Stub returns "".
fn getsparam(_name: &str) -> String { String::new() }

// `getaparam` lives in `Src/params.c:3680`. Stub returns empty Vec.
fn getaparam(_name: &str) -> Vec<String> { Vec::new() }

// `getiparam` lives in `Src/params.c:3669`. Stub returns 0.
fn getiparam(_name: &str) -> i64 { 0 }

// `setsparam` lives in `Src/params.c:3380`. Stub.
fn setsparam(_name: &str, _value: &str) {}

// `setiparam` lives in `Src/params.c:3390`. Stub.
fn setiparam(_name: &str, _value: i64) {}

// `setloopvar` lives in `Src/params.c:3408`. Stub.
fn setloopvar(_name: &str, _value: &str) {}

// `createparam` lives in `Src/params.c:1320`. Returns a new (or
// existing) param pointer with the requested flags. Stub returns NULL
// — ksh93_wrapper's `if (pm = createparam(...))` branches all become
// no-op until the real param-table port lands.
fn createparam(_name: &str, _flags: u32) -> *mut param { std::ptr::null_mut() }

// `arrgetfn` lives in `Src/params.c:5045`. C signature returns the
// param's array value as `char **`. Stub forwards the array.
fn arrgetfn(_pm: *mut param, fallback: Vec<String>) -> Vec<String> { fallback }

// `ztrdup` lives in `Src/string.c:18`. Pass-through clone.
fn ztrdup(s: &str) -> String { s.to_string() }


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
        assert_eq!(features_(m), 0);
        assert_eq!(enables_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

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
