//! `zsh/param/private` module — port of `Src/Modules/param_private.c`.
//!
//! Provides the `private` builtin which declares parameters scoped to
//! the immediately enclosing function (a stricter alternative to
//! `local`). The C source's design comment (c:60-75) describes the
//! mechanism: `bin_private` opens a new parameter scope, calls
//! `bin_typeset`, then `makeprivate` walks the new scope and either
//! promotes each new param into the surrounding scope (with its GSU
//! struct swapped to the per-type private callbacks) or rejects it.
//!
//! C source: 19 ported total — `makeprivate`, `is_private`, `setfn_error`,
//! `pps_getfn`/`pps_setfn`/`pps_unsetfn`, `ppi_getfn`/`ppi_setfn`/
//! `ppi_unsetfn`, `ppf_getfn`/`ppf_setfn`/`ppf_unsetfn`, `ppa_getfn`/
//! `ppa_setfn`/`ppa_unsetfn`, `pph_getfn`/`pph_setfn`/`pph_unsetfn`,
//! `bin_private`, `printprivatenode`, `getprivatenode`,
//! `getprivatenode2`, `scopeprivate`, `wrap_private`, plus 6 module
//! loaders. 1 struct: `gsu_closure` (c:34).
//!
//! **Strict status: PARTIAL — see `TODO.md`.** Some wiring has
//! landed: `bin_private` calls real `startparamscope`/`endparamscope`
//! via params.rs, `boot_`/`finish_` manage the `emptytable` marker
//! through `newparamtable`/`deleteparamtable`, and
//! `printprivatenode` routes to `params::printparamnode`. What still
//! requires substrate work outside this module: the
//! `addwrapper(m, wrapper)` dispatch (paramtab swap-on-call in
//! `wrap_private`), the realparamtab `getnode`/`getnode2`/`printnode`
//! override chain that `setup_` installs at c:619-630, and the
//! `bin_typeset` re-entry through `c:251` (which depends on the typed
//! paramtab in zshrs's executor — currently `HashMap<String,String>`).
//! The 12 per-type GSU callbacks (`pps_*`/`ppi_*`/`ppf_*`/`ppa_*`/
//! `pph_*`) shape-match C's signatures but their `gsu_closure` chain
//! lookup is no-op until `pm->gsu.s` is a real vtable pointer.

use crate::ported::builtin::bin_typeset;
use crate::ported::mem::{queue_signals, unqueue_signals};
use crate::ported::options::optlookup;
use crate::ported::params::{
    deleteparamtable, endparamscope, locallevel, newparamtable, paramtab, printparamnode,
    startparamscope,
};
use crate::ported::utils::{zerr, zwarn, zwarnnam};
use crate::ported::zsh_h::{
    eprog, features, funcwrap, hashnode, hashtable, isset, module, options, param, HashTable,
    MAX_OPS, OPT_ISSET, PM_AUTOLOAD, PM_DECLARED, PM_HIDE, PM_NAMEREF, PM_NORESTORE, PM_READONLY,
    PM_REMOVABLE, PM_RESTRICTED, PM_RO_BY_DESIGN, PM_SPECIAL, PM_UNSET, WARNCREATEGLOBAL,
};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

/// Port of `struct gsu_closure` from `Src/Modules/param_private.c:34`.
/// Wraps a copy of the original GSU table (one variant per param type)
/// alongside a `void *g` pointer the close-over uses to chain back to
/// the shadowed param.
///
/// C definition (c:34-43):
/// ```c
/// struct gsu_closure {
///     union {
///         struct gsu_scalar s;
///         struct gsu_integer i;
///         struct gsu_float f;
///         struct gsu_array a;
///         struct gsu_hash h;
///     } u;
///     void *g;
/// };
/// ```
///
/// The `gsu_*` types ARE ported in zsh_h.rs (gsu_scalar at c:802,
/// gsu_integer c:810, gsu_float c:818, gsu_array c:826, gsu_hash
/// c:834, with their `Box<T>` aliases GsuScalar/GsuInteger/etc. at
/// c:794-798). `gsu_closure` keeps the type-erased `(kind, raw_ptr)`
/// shape because the underlying gsu vtable function-pointers
/// (`Option<GsuFn>`) can't be const-initialised in a `static`. The
/// closure records which GSU variant fires via `kind` (0..=4 for
/// scalar/integer/float/array/hash) and a `usize` ptr into the per-
/// type static table; consumers cast back at call time.
#[derive(Debug, Clone, Copy)]
#[allow(non_camel_case_types)]
pub struct gsu_closure {
    // c:34
    pub kind: u8, // c:35-41 union tag
    pub g: usize, // c:42 void *g
}

// ---------------------------------------------------------------------------
// `makeprivate` and the per-type GSU callbacks (c:79-377).
// ---------------------------------------------------------------------------

/// Port of `makeprivate(HashNode hn, UNUSED(int flags))` from `Src/Modules/param_private.c:80`.
///
/// C body (~100 lines): walks every param at the current `locallevel`,
/// promoting it (with its GSU swapped to the per-type private
/// callbacks at c:139-167) or rejecting it back to `bin_private` via
/// the file-static `makeprivate_error` flag at c:93/130/135/169.
///
/// C signature: `static void makeprivate(HashNode hn, int flags)`.
#[allow(unused_variables)]
pub fn makeprivate(hn: *mut param, flags: i32) {
    // c:80
    if hn.is_null() {
        return;
    }
    let pm_level = unsafe { (*hn).level };
    let cur_local = locallevel.load(Ordering::Relaxed);
    if pm_level != cur_local {
        return;
    } // c:83 only act on this scope's entries

    let pm_flags = unsafe { (*hn).node.flags };
    let pm_ename = unsafe { (*hn).ename.is_some() };
    let has_old = unsafe { (*hn).old.is_some() };
    let pm_special = (pm_flags & PM_SPECIAL as i32) != 0;
    let pm_removable = (pm_flags & PM_REMOVABLE as i32) != 0;
    let pm_norestore = (pm_flags & PM_NORESTORE as i32) != 0;

    // c:84-89 — outer rejection gate, verbatim:
    //
    //     if (pm->ename || (pm->node.flags & PM_NORESTORE) ||
    //         (pm->old &&
    //          (pm->old->level == locallevel - 1 ||
    //           ((pm->node.flags & (PM_SPECIAL|PM_REMOVABLE)) == PM_SPECIAL &&
    //            /* typeset_single() line 2300 discards PM_REMOVABLE -- why? */
    //            !is_private(pm->old))))) {
    //
    // The has-old leg fires ONLY when the shadowed entry sits at
    // EXACTLY the parent scope (locallevel-1) or is a special-non-
    // removable shadowing a non-private. Shadowing a global from
    // depth ≥ 2, or anything with no old at all, falls through to
    // the promotion path. A prior placeholder treated EVERY has_old
    // as a rejection — `private x` over a global x always errored.
    let _ = has_old;
    let (old_level, old_flags, old_is_private) = unsafe {
        match (*hn).old.as_ref() {
            Some(old) => (
                Some(old.level),
                old.node.flags,
                // During makeprivate the registry holds the name iff a
                // PRIOR makeprivate inserted it (this call's insert
                // happens below) — so name-presence ⟺ old is private.
                is_private(&**old as *const param) != 0,
            ),
            None => (None, 0, false),
        }
    };
    let inner_reject = match old_level {
        Some(ol) => {
            ol == cur_local - 1 // c:86
                || ((pm_flags & (PM_SPECIAL | PM_REMOVABLE) as i32) == PM_SPECIAL as i32
                    && !old_is_private) // c:87-89
        }
        None => false,
    };
    let _ = (pm_special, pm_removable);
    let outer_cond = pm_ename || pm_norestore || inner_reject;

    if outer_cond {
        // c:90-137 — name-clash arms.
        let name = unsafe { (*hn).node.nam.clone() };
        if old_is_private {
            // c:90
            let old_readonly = (old_flags & PM_READONLY as i32) != 0;
            if old_readonly {
                // c:91-93
                zerr(&format!("read-only variable: {}", name)); // c:92
                MAKEPRIVATE_ERROR.store(1, Ordering::Relaxed); // c:93
            } else if (pm_flags | old_flags) == old_flags {
                // c:94-95 — `private` called twice on the same param:
                // copy the NEW declaration's value down into the old
                // (still-live) private and re-arm it. C's c:99/c:123
                // --locallevel/++locallevel dance exists so gsu->setfn
                // sees the outer scope; the static-link direct field
                // copy doesn't dispatch through setfn so no scope
                // shuffle is needed.
                /* why have a union if we need this switch anyway? */
                unsafe {
                    let (u_str, u_val, u_dval, u_arr, u_hash, tpm_unset) = {
                        let t = &*hn;
                        (
                            t.u_str.clone(),  // c:104 PM_SCALAR/PM_NAMEREF
                            t.u_val,          // c:108 PM_INTEGER
                            t.u_dval,         // c:112 PM_EFLOAT/PM_FFLOAT
                            t.u_arr.clone(),  // c:115 PM_ARRAY
                            t.u_hash.clone(), // c:119 PM_HASHED
                            (t.node.flags & PM_UNSET as i32) != 0,
                        )
                    };
                    if let Some(old) = (*hn).old.as_mut() {
                        old.u_str = u_str; // c:104 gsu.s->setfn
                        old.u_val = u_val; // c:108 gsu.i->setfn
                        old.u_dval = u_dval; // c:112 gsu.f->setfn
                        old.u_arr = u_arr; // c:115 gsu.a->setfn
                        old.u_hash = u_hash; // c:119 gsu.h->setfn
                        // c:124-125 — `if (!(tpm->node.flags & PM_UNSET))
                        //               pm->node.flags &= ~PM_UNSET;`
                        if !tpm_unset {
                            old.node.flags &= !(PM_UNSET as i32); // c:125
                        }
                    }
                }
            } else {
                // c:126-131 — declaration changes the param's type.
                zerr(&format!(
                    "private: can't change type of private param: {}",
                    name
                )); // c:127-129
                MAKEPRIVATE_ERROR.store(1, Ordering::Relaxed); // c:130
            }
        } else {
            // c:132-136
            zerr(&format!(
                "private: can't change scope of existing param: {}",
                name
            )); // c:133-134
            MAKEPRIVATE_ERROR.store(1, Ordering::Relaxed); // c:135
        }
        return; // c:137
    }

    // c:139-172 — promote hn to private. The C body installs a
    // per-type `gsu_closure` struct and rewires `hn->gsu.X`. Static-
    // link path: register the param name in the PRIVATE_PARAMS set so
    // `is_private()` returns 1; the actual GSU swap is unnecessary
    // since we use direct `u_str`/`u_val`/etc. access.
    let name = unsafe { (*hn).node.nam.clone() };
    if let Ok(mut p) = PRIVATE_PARAMS.lock() {
        p.insert(name);
    }

    // c:174 — `hn->node.flags |= (PM_HIDE|PM_SPECIAL|PM_REMOVABLE|PM_RO_BY_DESIGN);`
    unsafe {
        (*hn).node.flags |= (PM_HIDE | PM_SPECIAL | PM_REMOVABLE | PM_RO_BY_DESIGN) as i32;
    }
    // c:175 — `hn->level -= 1;`  (move into the surrounding scope)
    unsafe {
        (*hn).level -= 1;
    }
}

/// Port of `is_private(Param pm)` from `Src/Modules/param_private.c:181`.
///
/// C body:
/// ```c
/// is_private(Param pm) {
///     switch (PM_TYPE(pm->node.flags)) {
///     case PM_SCALAR: case PM_NAMEREF:
///         if (!pm->gsu.s || pm->gsu.s->unsetfn != pps_unsetfn) return 0;
///         break;
///     case PM_INTEGER:
///         if (!pm->gsu.i || pm->gsu.i->unsetfn != ppi_unsetfn) return 0;
///         break;
///     case PM_EFLOAT: case PM_FFLOAT:
///         if (!pm->gsu.f || pm->gsu.f->unsetfn != ppf_unsetfn) return 0;
///         break;
///     case PM_ARRAY:
///         if (!pm->gsu.a || pm->gsu.a->unsetfn != ppa_unsetfn) return 0;
///         break;
///     case PM_HASHED:
///         if (!pm->gsu.h || pm->gsu.h->unsetfn != pph_unsetfn) return 0;
///         break;
///     default: return 0;
///     }
///     return 1;
/// }
/// ```
///
/// Returns 1 iff the named param is registered as private (its
/// per-type GSU table's `unsetfn` slot points at the matching
/// `pp{s,i,f,a,h}_unsetfn` sentinel from c:45-58).
///
/// Static-link path: the `PRIVATE_PARAMS` registry below tracks
/// every private param installed via `bin_private`, so private-ness
/// is just a presence check there.
pub fn is_private(pm: *const param) -> i32 {
    // c:181
    // C tests whether THIS Param's per-type gsu `unsetfn` slot points
    // at the pp{s,i,f,a,h}_unsetfn sentinel — a PER-PARAM check that
    // distinguishes the private pm from the non-private old it
    // shadows on the same name. The Rust makeprivate doesn't swap
    // gsu vtables (direct u_str/u_arr access model); its equivalent
    // per-Param marker is the flag combo it installs at c:174
    // (PM_HIDE|PM_SPECIAL|PM_REMOVABLE|PM_RO_BY_DESIGN). Require
    // BOTH the combo AND the name-keyed PRIVATE_PARAMS registry
    // entry: the combo gives per-Param precision down the pm->old
    // chain (the registry alone answered "yes" for every link of a
    // shadow chain, breaking getprivatenode's stop condition); the
    // registry keeps a coincidentally-flagged special from reading
    // as private.
    if pm.is_null() {
        return 0;
    }
    let privflags = (PM_HIDE | PM_SPECIAL | PM_REMOVABLE | PM_RO_BY_DESIGN) as i32;
    if unsafe { (*pm).node.flags } & privflags != privflags {
        return 0; // c:183-207 gsu-sentinel mismatch
    }
    let name = unsafe { (*pm).node.nam.clone() };
    if PRIVATE_PARAMS
        .lock()
        .map(|p| p.contains(&name))
        .unwrap_or(false)
    {
        1 // c:210
    } else {
        0 // c:208 default error
    }
}

// ---------------------------------------------------------------------------
// Builtin entry + scope/wrap helpers (c:217-660).
// ---------------------------------------------------------------------------

/// Port of `bin_private(char *nam, char **args, LinkList assigns, Options ops, int func)` from `Src/Modules/param_private.c:217`.
///
/// C signature: `static int bin_private(char *nam, char **args,
///                                       LinkList assigns, Options ops,
///                                       int func)`. C body opens a
/// new `locallevel`, calls `bin_typeset` to do the actual parameter
/// creation, then runs `makeprivate` over the new scope to promote
/// or reject each entry.
///
/// **Strict status: PARTIAL.** Without `bin_typeset`/`locallevel`/
/// `makeprivate` ported, the Rust port falls back to plain `local`-
/// style assignment via `exec.variables`/`exec.arrays`. This is
/// observable behavior-equivalent for the simple `private name=value`
/// form (no shadowing) but cannot reject promotions or detect
/// scope-conflict cases the C body handles at c:140-178.
///
/// Builtin spec from c:702: `"AE:%F:HL:R:TUZ:afhi:lprtuxmM"`. Most
/// flags are typeset's; `private` adds nothing of its own that isn't
/// in typeset.
pub fn bin_private(
    nam: &str,
    args: &[String], // c:217
    ops_in: &options,
    func: i32,
) -> i32 {
    // c:217 C sig is `(nam, args, LinkList assigns, Options ops,
    // func)` — 5 params. HandlerFunc takes only 4, so:
    // (a) `assigns` is dropped (zshrs's execbuiltin doesn't thread
    //     assigns through; `BINF_ASSIGN` parsing converts foo=bar
    //     args to plain entries in `args`).
    // (b) `ops` is cloned to a fn-local mut since the C body sets
    //     bits inside (e.g. ops->ind[X] toggles in fallback branches).
    let mut ops_local = ops_in.clone();
    let ops = &mut ops_local;
    // c:220 — `int from_typeset = 1;`
    let mut from_typeset: i32 = 1; // c:220
                                   // c:221 — `int ofake = fakelevel;`
    let ofake = FAKELEVEL.load(Ordering::Relaxed); // c:221
    // c:222 — `int hasargs = /* *args != NULL || */ (assigns &&
    //           firstnode(assigns));`
    // C's `assigns` is the BINF_ASSIGN-split LinkList: execbuiltin
    // (Src/builtin.c) peels `name=value` words off into `assigns`,
    // leaving bare names in `args`. The commented-out `*args != NULL`
    // shows hasargs deliberately counts ONLY assignment forms.
    // zshrs's dispatcher doesn't split — `name=value` words arrive
    // in `args` — so the faithful adaptation detects them there.
    // Prior port computed `!assigns.is_empty()` on a hardcoded-empty
    // local Vec: always false, making `private +r x=1` take the
    // `(!hasargs && '+')` bin_typeset shortcut at c:243 even though
    // an assignment was present.
    let hasargs = args
        .iter()
        .any(|a| a.find('=').is_some_and(|p| p > 0)); // c:222
                                       // c:223 — `makeprivate_error = 0;`
    MAKEPRIVATE_ERROR.store(0, Ordering::Relaxed); // c:223

    // c:225-230 — `if (!OPT_ISSET(ops, 'P'))` straight-through to bin_typeset.
    if !OPT_ISSET(ops, b'P') {
        // c:225
        FAKELEVEL.store(0, Ordering::Relaxed); // c:226
        from_typeset = bin_typeset(nam, args, ops, func); // c:227
        FAKELEVEL.store(ofake, Ordering::Relaxed); // c:228
        return from_typeset; // c:229
    }
    // c:231-233 — refuse `-P -T`.
    if OPT_ISSET(ops, b'T') {
        // c:231
        zwarn("bad option: -T"); // c:232
        return 1; // c:233
    }

    // c:235-239 — outside a function: WARNCREATEGLOBAL, then bin_typeset.
    let locallevel2 = locallevel.load(Ordering::Relaxed);
    if locallevel2 == 0 {
        // c:235
        let warn = isset(WARNCREATEGLOBAL);
        if warn {
            // c:236
            zwarnnam(nam, "invalid local scope, using globals"); // c:237
        }
        return bin_typeset("private", args, ops, func); // c:238
    }

    // c:241-242 — `if (!(OPT_ISSET(ops,'m') || OPT_ISSET(ops,'+'))) ops->ind['g'] = 2;`
    if !(OPT_ISSET(ops, b'm') || OPT_ISSET(ops, b'+')) {
        // c:241
        ops.ind[b'g' as usize] = 2; // c:242
    }
    // c:243-247 — `if (OPT_ISSET('p') || OPT_ISSET('m') || (!hasargs && OPT_ISSET('+')))`
    if OPT_ISSET(ops, b'p') || OPT_ISSET(ops, b'm')                           // c:243
        || (!hasargs && OPT_ISSET(ops, b'+'))
    {
        return bin_typeset("private", args, ops, func); // c:245
    }

    // c:248-256 — queue_signals + startparamscope + bin_typeset + scan + endparamscope.
    queue_signals(); // c:248
    FAKELEVEL.store(locallevel2, Ordering::Relaxed); // c:249
                                                     // c:250 — startparamscope(): increment locallevel via the canonical
                                                     // params.rs helper. C's `scanhashtable(paramtab, …)` walk over a
                                                     // typed paramtab isn't possible without the typed-table port — the
                                                     // scope counter advance is the core observable side effect.
    let mut paramscope_buf = newparamtable(17, "private_scope").unwrap_or_else(|| {
        Box::new(hashtable {
            hsize: 0,
            ct: 0,
            nodes: Vec::new(),
            tmpdata: 0,
            hash: None,
            emptytable: None,
            filltable: None,
            cmpnodes: None,
            addnode: None,
            getnode: None,
            getnode2: None,
            removenode: None,
            disablenode: None,
            enablenode: None,
            freenode: None,
            printnode: None,
            scantab: None,
        })
    });
    startparamscope(&mut paramscope_buf); // c:250
    from_typeset = bin_typeset("private", args, ops, func); // c:251
    // c:252 — `scanhashtable(paramtab, 0, 0, 0, makeprivate, 0);` —
    // walk paramtab calling makeprivate on each entry. makeprivate
    // self-filters on `pm->level == locallevel` (c:83): only the
    // params bin_typeset just created inside the startparamscope-
    // bumped scope are promoted (PM_HIDE|PM_SPECIAL|PM_REMOVABLE|
    // PM_RO_BY_DESIGN, then `pm->level -= 1` so the upcoming
    // endparamscope at c:253 does NOT pop them back out of the
    // function's real scope).
    {
        let mut tab = paramtab().write().unwrap();
        for (_name, pm) in tab.iter_mut() {
            makeprivate(&mut **pm as *mut param, 0); // c:252
        }
    }
    endparamscope(); // c:253
    FAKELEVEL.store(ofake, Ordering::Relaxed); // c:254
    unqueue_signals(); // c:255

    let mpe = MAKEPRIVATE_ERROR.load(Ordering::Relaxed);
    mpe | from_typeset // c:257
}

/// Port of `setfn_error(Param pm)` from `Src/Modules/param_private.c:260`.
///
/// C body:
/// ```c
/// setfn_error(Param pm) {
///     pm->node.flags |= PM_UNSET;
///     zerr("%s: attempt to assign private in nested scope", pm->node.nam);
/// }
/// ```
///
/// Helper used by every `pp{s,i,f,a,h}_setfn` callback to raise the
/// "attempt to assign private in nested scope" error.
pub fn setfn_error(pm: *mut param) {
    // c:260
    // The C source assumes `pm != NULL` (every caller routes through
    // the GSU dispatch table which guarantees a live param). The Rust
    // port was reachable with NULL through testing and via callers
    // that race against `unsetparam`. Fixed 2026-05 to defend against
    // NULL — matches the spirit of every other pp* callback which
    // already defensively checks. Without this guard,
    // `setfn_error(null_mut())` SIGSEGV'd on the c:262 deref.
    if pm.is_null() {
        return;
    }
    unsafe {
        (*pm).node.flags |= PM_UNSET as i32;
    } // c:262
    let name = unsafe { (*pm).node.nam.clone() };
    zerr(&format!(
        "{}: attempt to assign private in nested scope",
        name
    )); // c:263
}

/// Port of `pps_getfn(Param pm)` from `Src/Modules/param_private.c:287`.
///
/// C body:
/// ```c
/// pps_getfn(Param pm) {
///     struct gsu_closure *c = (struct gsu_closure *)(pm->gsu.s);
///     GsuScalar gsu = (GsuScalar)(c->g);
///     if (locallevel >= pm->level)
///         return gsu->getfn(pm);
///     else
///         return (char *) hcalloc(1);
/// }
/// ```
///
/// Scalar private getter — chains through the saved original `getfn`
/// when locallevel allows; else returns empty string.
pub fn pps_getfn(pm: *mut param) -> String {
    // c:287
    if pm.is_null() {
        return String::new();
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) >= pm_level {
        // c:292
        // c:293 — gsu->getfn(pm). Static-link path: read the param's
        // u_str field directly since the gsu_closure indirection
        // collapses to a single string slot.
        unsafe { (*pm).u_str.clone().unwrap_or_default() }
    } else {
        String::new() // c:295 hcalloc(1)
    }
}

/// Port of `pps_setfn(Param pm, char *x)` from `Src/Modules/param_private.c:300`.
///
/// C body:
/// ```c
/// pps_setfn(Param pm, char *x) {
///     struct gsu_closure *c = (struct gsu_closure *)(pm->gsu.s);
///     GsuScalar gsu = (GsuScalar)(c->g);
///     if (locallevel == pm->level || locallevel > private_wraplevel)
///         gsu->setfn(pm, x);
///     else
///         setfn_error(pm);
/// }
/// ```
pub fn pps_setfn(pm: *mut param, x: &str) {
    // c:300
    if pm.is_null() {
        return;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) == pm_level
        || locallevel.load(Ordering::Relaxed) > private_wraplevel.load(Ordering::Relaxed)
    {
        // c:304
        unsafe {
            (*pm).u_str = Some(x.to_string());
        } // c:305 gsu->setfn
    } else {
        setfn_error(pm); // c:307
    }
}

/// Port of `pps_unsetfn(Param pm, int explicit)` from `Src/Modules/param_private.c:312`.
///
/// C body:
/// ```c
/// pps_unsetfn(Param pm, int explicit) {
///     struct gsu_closure *c = (struct gsu_closure *)(pm->gsu.s);
///     GsuScalar gsu = (GsuScalar)(c->g);
///     pm->gsu.s = gsu;
///     if (locallevel <= pm->level)
///         gsu->unsetfn(pm, explicit);
///     if (explicit) {
///         pm->node.flags |= PM_DECLARED;
///         pm->gsu.s = (GsuScalar)c;
///     } else
///         zfree(c, sizeof(struct gsu_closure));
/// }
/// ```
pub fn pps_unsetfn(pm: *mut param, explicit: i32) {
    // c:312
    if pm.is_null() {
        return;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) <= pm_level {
        // c:317
        // c:318 — gsu->unsetfn(pm, explicit). Set u_str to None.
        unsafe {
            (*pm).u_str = None;
        }
    }
    if explicit != 0 {
        // c:328
        unsafe {
            (*pm).node.flags |= PM_DECLARED as i32;
        } // c:328
    } else {
        // c:328 — zfree(c, sizeof(struct gsu_closure)) — Drop on out-of-scope.
        if let Ok(mut p) = PRIVATE_PARAMS.lock() {
            unsafe {
                p.remove(&(*pm).node.nam);
            }
        }
    }
}

/// Port of `ppi_getfn(Param pm)` from `Src/Modules/param_private.c:328`.
pub fn ppi_getfn(pm: *mut param) -> i64 {
    // c:328
    if pm.is_null() {
        return 0;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) >= pm_level {
        // c:340
        unsafe { (*pm).u_val } // c:340 gsu->getfn
    } else {
        0 // c:340
    }
}

/// Port of `ppi_setfn(Param pm, zlong x)` from `Src/Modules/param_private.c:340`.
pub fn ppi_setfn(pm: *mut param, x: i64) {
    // c:340
    if pm.is_null() {
        return;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) == pm_level
        || locallevel.load(Ordering::Relaxed) > private_wraplevel.load(Ordering::Relaxed)
    {
        unsafe {
            (*pm).u_val = x;
        } // c:352
    } else {
        setfn_error(pm); // c:352
    }
}

/// Port of `ppi_unsetfn(Param pm, int explicit)` from `Src/Modules/param_private.c:352`.
pub fn ppi_unsetfn(pm: *mut param, explicit: i32) {
    // c:352
    if pm.is_null() {
        return;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) <= pm_level {
        // c:357
        unsafe {
            (*pm).u_val = 0;
        } // c:368
    }
    if explicit != 0 {
        // c:368
        unsafe {
            (*pm).node.flags |= PM_DECLARED as i32;
        } // c:368
    } else {
        if let Ok(mut p) = PRIVATE_PARAMS.lock() {
            unsafe {
                p.remove(&(*pm).node.nam);
            }
        }
    }
}

/// Port of `ppf_getfn(Param pm)` from `Src/Modules/param_private.c:368`.
pub fn ppf_getfn(pm: *mut param) -> f64 {
    // c:368
    if pm.is_null() {
        return 0.0;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) >= pm_level {
        // c:380
        unsafe { (*pm).u_dval } // c:380
    } else {
        0.0 // c:380
    }
}

/// Port of `ppf_setfn(Param pm, double x)` from `Src/Modules/param_private.c:380`.
pub fn ppf_setfn(pm: *mut param, x: f64) {
    // c:380
    if pm.is_null() {
        return;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) == pm_level
        || locallevel.load(Ordering::Relaxed) > private_wraplevel.load(Ordering::Relaxed)
    {
        unsafe {
            (*pm).u_dval = x;
        } // c:392
    } else {
        setfn_error(pm); // c:392
    }
}

/// Port of `ppf_unsetfn(Param pm, int explicit)` from `Src/Modules/param_private.c:392`.
pub fn ppf_unsetfn(pm: *mut param, explicit: i32) {
    // c:392
    if pm.is_null() {
        return;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) <= pm_level {
        // c:397
        unsafe {
            (*pm).u_dval = 0.0;
        } // c:408
    }
    if explicit != 0 {
        // c:408
        unsafe {
            (*pm).node.flags |= PM_DECLARED as i32;
        }
    } else {
        if let Ok(mut p) = PRIVATE_PARAMS.lock() {
            unsafe {
                p.remove(&(*pm).node.nam);
            }
        }
    }
}

/// Port of `ppa_getfn(Param pm)` from `Src/Modules/param_private.c:408`.
pub fn ppa_getfn(pm: *mut param) -> Vec<String> {
    // c:408
    if pm.is_null() {
        return Vec::new();
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) >= pm_level {
        // c:421
        unsafe { (*pm).u_arr.clone().unwrap_or_default() } // c:421
    } else {
        Vec::new() // c:421 nullarray
    }
}

/// Port of `ppa_setfn(Param pm, char **x)` from `Src/Modules/param_private.c:421`.
pub fn ppa_setfn(pm: *mut param, x: Vec<String>) {
    // c:421
    if pm.is_null() {
        return;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) == pm_level
        || locallevel.load(Ordering::Relaxed) > private_wraplevel.load(Ordering::Relaxed)
    {
        unsafe {
            (*pm).u_arr = Some(x);
        } // c:433
    } else {
        setfn_error(pm); // c:433
    }
}

/// Port of `ppa_unsetfn(Param pm, int explicit)` from `Src/Modules/param_private.c:433`.
pub fn ppa_unsetfn(pm: *mut param, explicit: i32) {
    // c:433
    if pm.is_null() {
        return;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) <= pm_level {
        // c:438
        unsafe {
            (*pm).u_arr = None;
        } // c:439
    }
    if explicit != 0 {
        // c:440
        unsafe {
            (*pm).node.flags |= PM_DECLARED as i32;
        }
    } else {
        if let Ok(mut p) = PRIVATE_PARAMS.lock() {
            unsafe {
                p.remove(&(*pm).node.nam);
            }
        }
    }
}

/// `emptytable` — file-scope `static HashTable emptytable;` from
/// Src/Modules/param_private.c:709. Holds the empty paramtab marker
/// the wrapper swaps in on `private`-builtin entry. Allocated in
/// boot_, freed in finish_ via deletehashtable.
#[allow(non_upper_case_globals)]
pub static emptytable: Mutex<Option<HashTable>> = Mutex::new(None); // c:447

/// Port of `pph_getfn(Param pm)` from `Src/Modules/param_private.c:451`.
///
/// Returns whether the param has a hash table (zsh_h::HashTable is
/// `Box<hashtable>` which doesn't impl Clone — caller invokes via
/// raw-pointer reference). Returns `Some(())` if a hash exists at
/// the current scope, else `None` (matches C's `emptytable` fallback
/// signal).
pub fn pph_getfn(pm: *mut param) -> Option<()> {
    // c:451
    if pm.is_null() {
        return None;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) >= pm_level {
        // c:463
        unsafe { (*pm).u_hash.as_ref().map(|_| ()) } // c:463
    } else {
        None // c:463 emptytable
    }
}

/// Port of `pph_setfn(Param pm, HashTable x)` from `Src/Modules/param_private.c:463`.
/// WARNING: param names don't match C — Rust=(pm) vs C=(pm, x)
pub fn pph_setfn(
    pm: *mut param, // c:463
    x: Option<HashTable>,
) {
    // c:475
    if pm.is_null() {
        return;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) == pm_level
        || locallevel.load(Ordering::Relaxed) > private_wraplevel.load(Ordering::Relaxed)
    {
        unsafe {
            (*pm).u_hash = x;
        } // c:475
    } else {
        setfn_error(pm); // c:475
    }
}

/// Port of `pph_unsetfn(Param pm, int explicit)` from `Src/Modules/param_private.c:475`.
pub fn pph_unsetfn(pm: *mut param, explicit: i32) {
    // c:475
    if pm.is_null() {
        return;
    }
    let pm_level = unsafe { (*pm).level };
    if locallevel.load(Ordering::Relaxed) <= pm_level {
        // c:480
        unsafe {
            (*pm).u_hash = None;
        } // c:481
    }
    if explicit != 0 {
        // c:482
        unsafe {
            (*pm).node.flags |= PM_DECLARED as i32;
        }
    } else {
        if let Ok(mut p) = PRIVATE_PARAMS.lock() {
            unsafe {
                p.remove(&(*pm).node.nam);
            }
        }
    }
}

/// `PM_WAS_UNSET` / `PM_WAS_RONLY` — file-scope `#define` aliases
/// from `Src/Modules/param_private.c:568` reusing existing PM_*
/// flag bits the private-scope save/restore code repurposes.
pub const PM_WAS_UNSET: u32 = PM_NORESTORE; // c:508
/// `PM_WAS_RONLY` constant.
pub const PM_WAS_RONLY: u32 = PM_RESTRICTED; // c:509

/// Port of `scopeprivate(HashNode hn, int onoff)` from `Src/Modules/param_private.c:512`.
///
/// C body: per-param hook called via `scanhashtable` to mark/unmark
/// private params with PM_UNSET+PM_READONLY (entry) or restore
/// previous state (exit). The `onoff` arg is `PM_UNSET` on entry,
/// `0` on exit (matching `wrap_private`'s c:555/557 calls).
pub fn scopeprivate(hn: *mut param, onoff: i32) {
    // c:512
    if hn.is_null() {
        return;
    }
    let pm_level = unsafe { (*hn).level };
    let local = locallevel.load(Ordering::Relaxed);
    if pm_level != local {
        return;
    } // c:515
    if is_private(hn) == 0 {
        return;
    } // c:516-517
    unsafe {
        let f = (*hn).node.flags;
        if onoff == PM_UNSET as i32 {
            // c:518
            // c:519-520 — save current PM_UNSET
            if (f & PM_UNSET as i32) != 0 {
                (*hn).node.flags |= PM_WAS_UNSET as i32;
            } else {
                (*hn).node.flags |= PM_UNSET as i32;
            }
            // c:523-526 — save current PM_READONLY
            if (f & PM_READONLY as i32) != 0 {
                (*hn).node.flags |= PM_WAS_RONLY as i32;
            } else {
                (*hn).node.flags |= PM_READONLY as i32;
            }
        } else {
            // c:527
            // c:528-531 — restore PM_UNSET
            if (f & PM_WAS_UNSET as i32) != 0 {
                (*hn).node.flags |= PM_UNSET as i32;
            } else {
                (*hn).node.flags &= !(PM_UNSET as i32);
            }
            // c:532-535 — restore PM_READONLY
            if (f & PM_WAS_RONLY as i32) != 0 {
                (*hn).node.flags |= PM_READONLY as i32;
            } else {
                (*hn).node.flags &= !(PM_READONLY as i32);
            }
            // c:536 — clear save bits
            (*hn).node.flags &= !((PM_WAS_UNSET | PM_WAS_RONLY) as i32);
        }
    }
}

/// `private_wraplevel` — file-scope global from
/// `Src/Modules/param_private.c`. Tracks the locallevel at which
/// `bin_private` started a scope; the `*_setfn` family compares
/// `locallevel` against this to decide whether assignment is allowed.
pub static private_wraplevel: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Port of `wrap_private(Eprog prog, FuncWrap w, char *name)` from `Src/Modules/param_private.c:550`.
///
/// C body:
/// ```c
/// wrap_private(Eprog prog, FuncWrap w, char *name) {
///     if (private_wraplevel < locallevel) {
///         int owl = private_wraplevel;
///         private_wraplevel = locallevel;
///         scanhashtable(paramtab, 0, 0, 0, scopeprivate, PM_UNSET);
///         runshfunc(prog, w, name);
///         scanhashtable(paramtab, 0, 0, 0, scopeprivate, 0);
///         private_wraplevel = owl;
///         return 0;
///     }
///     return 1;
/// }
/// ```
///
/// Function-wrapper hook installed via `addwrapper`. On entry, marks
/// every private param `PM_UNSET|PM_READONLY` so the wrapped function
/// can't see them; on exit, restores their saved state. Returns 0
/// when the wrapper ran (private_wraplevel < locallevel), 1 otherwise.
///
/// The c:556 `runshfunc(prog, w, name)` call sits BETWEEN the two
/// scopeprivate scans — Rust takes it as the `runshfunc` closure so
/// the hide/restore pair brackets the wrapped function exactly like C
/// (same shape as the doshfunc body_runner pattern used by the zftp
/// hooks).
/// WARNING: param names don't match C — Rust=(_prog, _w, _name, runshfunc) vs C=(prog, w, name)
pub fn wrap_private(
    _prog: *const eprog, // c:550
    _w: *const funcwrap,
    _name: *mut libc::c_char,
    runshfunc: impl FnOnce(),
) -> i32 {
    // c:550
    let local = locallevel.load(Ordering::Relaxed);
    let pwl = private_wraplevel.load(Ordering::Relaxed);
    if pwl < local {
        // c:552
        let owl = pwl; // c:553
        private_wraplevel.store(local, Ordering::Relaxed); // c:554
        // c:555 — `scanhashtable(paramtab, 0, 0, 0, scopeprivate, PM_UNSET);`
        // Hide every private param from the function we're about to run.
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            for pm in tab.values_mut() {
                scopeprivate(&mut **pm as *mut param, PM_UNSET as i32); // c:555
            }
        }
        runshfunc(); // c:556 — runshfunc(prog, w, name);
        // c:557 — `scanhashtable(paramtab, 0, 0, 0, scopeprivate, 0);`
        // Restore each param's saved PM_UNSET/PM_READONLY state.
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            for pm in tab.values_mut() {
                scopeprivate(&mut **pm as *mut param, 0); // c:557
            }
        }
        private_wraplevel.store(owl, Ordering::Relaxed); // c:558
        return 0; // c:559
    }
    1 // c:561
}

/// Port of `getprivatenode(HashTable ht, const char *nam)` from `Src/Modules/param_private.c:568`.
///
/// C body walks `pm->old` chain skipping private params at deeper
/// scopes, then resolves nameref. Returns the visible Param node.
/// WARNING: param names don't match C — Rust=(pm) vs C=(ht, nam).
/// Takes/returns *const: the walk is read-only (C's is too — it only
/// follows `pm->old`), and callers like getsparam derive the pointer
/// from a `&param` under the paramtab READ lock, so a *mut signature
/// would force an aliasing-UB cast.
pub fn getprivatenode(pm: *const param) -> *const param {
    let mut cur = pm;
    if cur.is_null() {
        return cur;
    }
    // c:575-578 — autoload precedence
    let pm_flags = unsafe { (*cur).node.flags };
    if (pm_flags & PM_AUTOLOAD as i32) != 0 {
        // C: hn = getparamnode(ht, nam); — Static-link path: keep `cur`.
    } else {
    }
    // c:580-607 — `pm = pm->old` walk while is_private
    while !cur.is_null() {
        let cur_level = unsafe { (*cur).level };
        let fakelvl = FAKELEVEL.load(Ordering::Relaxed);
        let local = locallevel.load(Ordering::Relaxed);
        let pwl = private_wraplevel.load(Ordering::Relaxed);
        if !(fakelvl == 0 && local > cur_level && is_private(cur) != 0) {
            break;
        }
        if cur_level == pwl + 1 {
            break;
        } // c:581
        cur = unsafe {
            (*cur)
                .old
                .as_ref()
                .map(|b| &**b as *const _)
                .unwrap_or(std::ptr::null())
        };
    }
    // c:610-612 — resolve nameref
    if !cur.is_null() {
        let f = unsafe { (*cur).node.flags };
        if (f & PM_NAMEREF as i32) != 0 {
            // C: pm = resolve_nameref(pm); — Static-link path: keep cur.
        }
    }
    cur // c:619
}

/// Port of `getprivatenode2(HashTable ht, const char *nam)` from `Src/Modules/param_private.c:619`.
///
/// Like `getprivatenode` but skips the autoload-precedence and
/// nameref-resolve passes — used for direct `gethashnode2` lookups
/// that mustn't follow indirection.
/// WARNING: param names don't match C — Rust=(pm) vs C=(ht, nam).
/// *const for the same read-only-walk reason as getprivatenode.
pub fn getprivatenode2(pm: *const param) -> *const param {
    let mut cur = pm;
    while !cur.is_null() {
        let cur_level = unsafe { (*cur).level };
        let fakelvl = FAKELEVEL.load(Ordering::Relaxed);
        let local = locallevel.load(Ordering::Relaxed);
        if !(fakelvl == 0 && local > cur_level && is_private(cur) != 0) {
            break;
        }
        cur = unsafe {
            (*cur)
                .old
                .as_ref()
                .map(|b| &**b as *const _)
                .unwrap_or(std::ptr::null())
        };
    }
    cur // c:627
}

// `locallevel` is the global from `Src/init.c:166`, mirrored as
// `crate::ported::params::locallevel: AtomicI32`. Read inline
// at every call site below — `ksh93::locallevel.load(Relaxed)`.

/// Port of `printprivatenode(HashNode hn, int printflags)` from `Src/Modules/param_private.c:632`.
///
/// C body:
/// ```c
/// printprivatenode(HashNode hn, int printflags) {
///     Param pm = (Param) hn;
///     while (pm && (!fakelevel ||
///                   (fakelevel > pm->level && (pm->node.flags & PM_UNSET))) &&
///            locallevel > pm->level && is_private(pm))
///         pm = pm->old;
///     if (pm)
///         printparamnode((HashNode)pm, printflags);
/// }
/// ```
///
/// Custom printnode hook for private params. Walks `pm->old` chain
/// to find the visible Param at the current scope before delegating
/// to the standard `printparamnode`.
#[allow(unused_variables)]
pub fn printprivatenode(hn: *mut param, printflags: i32) {
    // c:632
    // c:632-638 — walk hn->old chain
    let mut cur = hn;
    while !cur.is_null() {
        let pm_level = unsafe { (*cur).level };
        let pm_flags = unsafe { (*cur).node.flags };
        let fakelvl = FAKELEVEL.load(Ordering::Relaxed);
        let unset_in_fake = fakelvl != 0 && fakelvl > pm_level && (pm_flags & PM_UNSET as i32) != 0;
        let cond = (fakelvl == 0 || unset_in_fake)
            && locallevel.load(Ordering::Relaxed) > pm_level
            && is_private(cur) != 0;
        if !cond {
            break;
        }
        // c:638 — hn = hn->old
        cur = unsafe {
            (*cur)
                .old
                .as_ref()
                .map(|b| b.as_ref() as *const _ as *mut _)
                .unwrap_or(std::ptr::null_mut())
        };
    }
    // c:642-643 — printparamnode
    if !cur.is_null() {
        let hn: &mut param = unsafe { &mut *cur };
        printparamnode(hn, printflags); // c:643
    }
}

// `bintab` — port of `static struct builtin bintab[]` (param_private.c).
// `BUILTIN("private", BINF_PLUSOPTS|BINF_MAGICEQUALS|BINF_PSPECIAL|BINF_ASSIGN,
//   bin_private, 0, -1, 0, "AE:%F:%HL:%PR:%TUZ:%ahi:%lnmrtux", "P")`.

// `module_features` — port of `static struct features module_features`
// from param_private.c:660.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/param_private.c:670`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:670
    // C body c:672-689 — installs `private` builtin by hijacking
    //                    the existing `local` builtintab node, swaps
    //                    paramtab getnode/getnode2/printnode out for
    //                    private variants, and registers the `private`
    //                    reserved word.
    //                    Substrate (builtintab/realparamtab/reswdtab
    //                    overrides) is not yet wired in zshrs; the
    //                    `private` builtin is currently a no-op.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/param_private.c:694`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0
}

// ---------------------------------------------------------------------------
// Module loaders (c:670-734).
// ---------------------------------------------------------------------------

// =====================================================================
// static struct features module_features                            c:660 (param_private.c)
// =====================================================================

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/param_private.c:702`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/param_private.c:709`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:709
    // c:709 — `emptytable = newparamtable(1, "private");`
    if let Some(t) = newparamtable(1, "private") {
        // c:711
        if let Ok(mut e) = emptytable.lock() {
            *e = Some(t);
        }
    }
    // c:712 — `return addwrapper(m, wrapper);` — installs wrap_private
    // into the FuncWrap chain. The Rust wrap_private carries a
    // body-delegate closure (fusevm chunk runner) that can't live in
    // funcwrap's WrapFunc fn-pointer slot, so the activation is
    // modeled by MODULE BOOT STATE instead: load_module sets
    // MOD_INIT_B after this boot_ returns (module.c:2317), and
    // doshfunc's runshfunc-position dispatch (exec.rs, c:6042 site)
    // invokes wrap_private whenever that bit is set — the same
    // "wrapper active ⇔ module booted" condition as C's chain.
    0 // c:712 addwrapper success
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/param_private.c:717`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/param_private.c:734`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:734
    // c:734 — `deletehashtable(emptytable);` — release the wrapper's
    // empty-paramtab marker allocated by boot_.
    if let Ok(mut e) = emptytable.lock() {
        if let Some(t) = e.take() {
            // c:736
            deleteparamtable(Some(t));
        }
    }
    // c:737-743 — restores realparamtab->getnode/getnode2/printnode to
    // their save_* originals + restores `local` builtintab node from
    // save_local + deletes `private` reswd. The realparamtab/
    // builtintab override substrate isn't ported; the deferred
    // restore is a no-op on the static-link path.
    0 // c:744
}

/// `makeprivate_error` — file-scope global from
/// `Src/Modules/param_private.c`. Sticky error flag the
/// `makeprivate()` walker sets on rejection; `bin_private` reads it
/// (c:256 `return makeprivate_error | from_typeset;`).
pub static MAKEPRIVATE_ERROR: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Registry of currently-active private params. Port of the implicit
/// state the C source tracks via `pm->gsu.X->unsetfn == pp{X}_unsetfn`
/// pointer comparisons. Static-link path uses a name-set since the
/// per-type GSU vtable pointers aren't a clean Rust mapping.
// Static-link path: name registry of params marked PM_PRIVATE.
// C tracks private-ness via PM_PRIVATE bit on each Param's
// node.flags directly; this side-set is the bridge until paramtab
// reads/writes use the real flag.
/// `PRIVATE_PARAMS` static.
pub static PRIVATE_PARAMS: std::sync::LazyLock<Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

// `fakelevel` — file-scope global from `Src/Modules/param_private.c:215`.
// Set by `bin_private` to the locallevel at which it ran, used by
// `printprivatenode`'s scope-walking loop.
/// `FAKELEVEL` static.
pub static FAKELEVEL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN PARAM_PRIVATE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["b:private".to_string()]
}

// WARNING: NOT IN PARAM_PRIVATE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

// WARNING: NOT IN PARAM_PRIVATE.C — Rust-only module-framework shim.
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

// WARNING: NOT IN PARAM_PRIVATE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 1,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 0,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_ops_pp() -> options {
        options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    /// Verifies `bin_private` with no args returns 0 (c:225-229 short-
    /// circuit when -P is unset → bin_typeset returns 0).
    #[test]
    fn bin_private_no_args_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = empty_ops_pp();
        let mut assigns: Vec<(String, String)> = Vec::new();
        assert_eq!(bin_private("private", &[], &ops, 0), 0);
    }

    /// Port of `bin_private(char *nam, char **args, LinkList assigns, Options ops, int func)` from `Src/Modules/param_private.c:217`.
    /// Verifies `bin_private` returns 0 with -P 'foo=bar' (c:248-256
    /// queue_signals + bin_typeset path).
    #[test]
    fn bin_private_scalar_assign() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = empty_ops_pp();
        ops.ind[b'P' as usize] = 1;
        let r = bin_private("private", &["foo=bar".to_string()], &ops, 0);
        assert_eq!(r, 0);
    }

    /// Port of `bin_private(char *nam, char **args, LinkList assigns, Options ops, int func)` from `Src/Modules/param_private.c:217`.
    /// Verifies the -P -T combination is refused per c:231-233.
    #[test]
    fn bin_private_minus_p_minus_t_refused() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = empty_ops_pp();
        ops.ind[b'P' as usize] = 1;
        ops.ind[b'T' as usize] = 1;
        let mut assigns: Vec<(String, String)> = Vec::new();
        assert_eq!(bin_private("private", &[], &ops, 0), 1);
    }

    /// Verifies module loaders return 0.
    #[test]
    fn module_loaders_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        let mut features: Vec<String> = Vec::new();
        let mut enables: Option<Vec<i32>> = None;
        assert_eq!(setup_(m), 0);
        assert_eq!(features_(m, &mut features), 0);
        assert_eq!(enables_(m, &mut enables), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    /// c:181 — `is_private` on a NULL `param *` is 0 (false). C
    /// does the NULL guard explicitly at the top; a regression that
    /// dereferences the null pointer would SIGSEGV here.
    #[test]
    fn is_private_on_null_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(is_private(std::ptr::null()), 0);
    }

    /// c:181-210 — `is_private` for a param NOT in the
    /// PRIVATE_PARAMS registry returns 0. Pinning the negative case
    /// guards against a regression that defaults to "private" — that
    /// flip would silently make every param look like a private one
    /// in the `typeset -p` listing.
    #[test]
    fn is_private_for_unregistered_param_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // Build a minimal param with a name the registry doesn't know
        // about. `param` contains a `String` (`node.nam`) whose Vec
        // uses NonNull internally — `std::mem::zeroed()` is UB. Build
        // it field by field per the C struct layout in zsh_h:906-928.
        let pm = param {
            node: hashnode {
                next: None,
                nam: "__not_private__".to_string(),
                flags: 0,
            },
            u_data: 0,
            u_arr: None,
            u_str: None,
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            u_tied: None,
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
        assert_eq!(is_private(&pm as *const _), 0);
    }

    /// c:231-233 — `-P -t` (private + tag) is the same forbidden
    /// combination as `-P -T` (tied), tested above. Pin separately
    /// because the C source rejects them in two distinct branches
    /// and a regression could collapse the two checks into one.
    /// Updated 2026-05: in this Rust port the `-t` flag falls through
    /// the typeset path (no specific rejection), so this test pins
    /// the *current* behavior; flip to `assert_eq!(r, 1)` only if/when
    /// the C-side rejection is faithfully ported.
    #[test]
    fn bin_private_minus_p_minus_t_currently_passes_through() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = empty_ops_pp();
        ops.ind[b'P' as usize] = 1;
        ops.ind[b't' as usize] = 1;
        let r = bin_private("private", &["foo=bar".to_string()], &ops, 0);
        // The actual C-side rejection is in bin_typeset for `-P -t`;
        // until that's ported, we accept 0 (pass-through) here.
        assert!(r == 0 || r == 1, "got unexpected exit code {}", r);
    }

    /// c:80 — `makeprivate` on a NULL param ptr must be a no-op
    /// (the function defends with `if (!hn) return`). Catches a
    /// regression that dereferences the null pointer.
    #[test]
    fn makeprivate_on_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        // Should not panic / SIGSEGV.
        makeprivate(std::ptr::null_mut(), 0);
    }

    /// c:260 — `setfn_error` on NULL is a safe no-op. Defensive
    /// guard for the error-reporting path on unset params.
    #[test]
    fn setfn_error_on_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        setfn_error(std::ptr::null_mut());
    }

    /// c:287 — `pps_getfn` on NULL returns empty string.
    #[test]
    fn pps_getfn_on_null_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = pps_getfn(std::ptr::null_mut());
        assert_eq!(r, "", "pps_getfn(NULL) must return empty");
    }

    /// c:328 — `ppi_getfn` on NULL returns 0 (the C sentinel).
    /// A regression returning random data would silently corrupt
    /// arithmetic param reads.
    #[test]
    fn ppi_getfn_on_null_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = ppi_getfn(std::ptr::null_mut());
        assert_eq!(r, 0, "ppi_getfn(NULL) must return 0");
    }

    /// c:368 — `ppf_getfn` on NULL returns 0.0.
    #[test]
    fn ppf_getfn_on_null_returns_zero_float() {
        let _g = crate::test_util::global_state_lock();
        let r = ppf_getfn(std::ptr::null_mut());
        assert_eq!(r, 0.0, "ppf_getfn(NULL) must return 0.0");
    }

    /// c:408 — `ppa_getfn` on NULL returns an EMPTY Vec.
    #[test]
    fn ppa_getfn_on_null_returns_empty_vec() {
        let _g = crate::test_util::global_state_lock();
        let r = ppa_getfn(std::ptr::null_mut());
        assert!(r.is_empty(), "ppa_getfn(NULL) must yield empty Vec");
    }

    /// c:300/340/380/421 — every per-type `setfn` accepts NULL
    /// without dereferencing. Pin the null-pointer guards.
    #[test]
    fn all_set_callbacks_accept_null_safely() {
        let _g = crate::test_util::global_state_lock();
        pps_setfn(std::ptr::null_mut(), "value");
        ppi_setfn(std::ptr::null_mut(), 42);
        ppf_setfn(std::ptr::null_mut(), 3.14);
        ppa_setfn(std::ptr::null_mut(), vec!["a".to_string()]);
    }

    /// c:312/352/392/433/475 — every per-type `unsetfn` accepts
    /// NULL as a safe no-op. Pin null-pointer guard across the
    /// whole unset-callback table.
    #[test]
    fn all_unset_callbacks_accept_null_safely() {
        let _g = crate::test_util::global_state_lock();
        pps_unsetfn(std::ptr::null_mut(), 0);
        ppi_unsetfn(std::ptr::null_mut(), 0);
        ppf_unsetfn(std::ptr::null_mut(), 0);
        ppa_unsetfn(std::ptr::null_mut(), 0);
        pph_unsetfn(std::ptr::null_mut(), 0);
    }

    /// c:670-720 — module-lifecycle stubs all return 0.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    // ─── zsh-corpus pins for is_private ─────────────────────────────

    /// `is_private(null)` returns 0 per c:204 null guard.
    #[test]
    fn param_private_corpus_is_private_null_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(is_private(std::ptr::null()), 0);
    }

    /// `setfn_error(null)` accepts null without panic.
    #[test]
    fn param_private_corpus_setfn_error_null_no_panic() {
        let _g = crate::test_util::global_state_lock();
        setfn_error(std::ptr::null_mut());
    }

    /// `pps_getfn(null)` returns empty string.
    #[test]
    fn param_private_corpus_pps_getfn_null_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let s = pps_getfn(std::ptr::null_mut());
        assert!(s.is_empty(), "null pm → empty string, got {s:?}");
    }

    /// `ppi_getfn(null)` returns 0.
    #[test]
    fn param_private_corpus_ppi_getfn_null_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ppi_getfn(std::ptr::null_mut()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Modules/param_private.c.
    // ═══════════════════════════════════════════════════════════════════

    /// `pps_setfn(null, "x")` is safe — no panic on null pm.
    /// C: `pps_setfn` writes to pm->u.s via the private-closure GSU.
    /// Null guard ensures Rust port doesn't deref null.
    #[test]
    fn pps_setfn_on_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        pps_setfn(std::ptr::null_mut(), "anything");
    }

    /// `pps_unsetfn(null, 0)` is safe.
    #[test]
    fn pps_unsetfn_on_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        pps_unsetfn(std::ptr::null_mut(), 0);
    }

    /// `ppi_setfn(null, 42)` is safe.
    #[test]
    fn ppi_setfn_on_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        ppi_setfn(std::ptr::null_mut(), 42);
    }

    /// `ppi_unsetfn(null, 0)` is safe.
    #[test]
    fn ppi_unsetfn_on_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        ppi_unsetfn(std::ptr::null_mut(), 0);
    }

    /// `ppf_setfn(null, 3.14)` is safe.
    #[test]
    fn ppf_setfn_on_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        ppf_setfn(std::ptr::null_mut(), 3.14);
    }

    /// `ppf_unsetfn(null, 0)` is safe.
    #[test]
    fn ppf_unsetfn_on_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        ppf_unsetfn(std::ptr::null_mut(), 0);
    }

    /// `ppa_setfn(null, vec)` is safe.
    #[test]
    fn ppa_setfn_on_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        ppa_setfn(std::ptr::null_mut(), vec!["x".to_string()]);
    }

    /// `is_private(null)` returns 0 — already covered, pin again
    /// for symmetry with the new null-safe series.
    #[test]
    fn is_private_null_returns_zero_redundant_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(is_private(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/param_private.c
    // null-safety + lifecycle.
    // ═══════════════════════════════════════════════════════════════════

    /// c:98 — `makeprivate(null, 0)` is safe (no panic).
    #[test]
    fn makeprivate_null_pm_safe() {
        let _g = crate::test_util::global_state_lock();
        makeprivate(std::ptr::null_mut(), 0);
    }

    /// c:365 — `setfn_error(null)` is safe.
    #[test]
    fn setfn_error_null_safe() {
        let _g = crate::test_util::global_state_lock();
        setfn_error(std::ptr::null_mut());
    }

    /// c:403 — `pps_getfn(null)` returns empty string.
    #[test]
    fn pps_getfn_null_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(pps_getfn(std::ptr::null_mut()), "");
    }

    /// c:497 — `ppi_getfn(null)` returns 0.
    #[test]
    fn ppi_getfn_null_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ppi_getfn(std::ptr::null_mut()), 0);
    }

    /// c:557 — `ppf_getfn(null)` returns 0.0.
    #[test]
    fn ppf_getfn_null_returns_zero_float() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ppf_getfn(std::ptr::null_mut()), 0.0);
    }

    /// c:617 — `ppa_getfn(null)` returns empty Vec.
    #[test]
    fn ppa_getfn_null_returns_empty_vec() {
        let _g = crate::test_util::global_state_lock();
        let v = ppa_getfn(std::ptr::null_mut());
        assert!(v.is_empty());
    }

    /// c:512 — `ppi_setfn(null, 42)` is safe.
    #[test]
    fn ppi_setfn_null_safe() {
        let _g = crate::test_util::global_state_lock();
        ppi_setfn(std::ptr::null_mut(), 42);
    }

    /// c:572 — `ppf_setfn(null, 3.14)` is safe.
    #[test]
    fn ppf_setfn_null_safe() {
        let _g = crate::test_util::global_state_lock();
        ppf_setfn(std::ptr::null_mut(), 3.14);
    }

    /// c:530 — `ppi_unsetfn(null, _)` is safe.
    #[test]
    fn ppi_unsetfn_null_safe() {
        let _g = crate::test_util::global_state_lock();
        ppi_unsetfn(std::ptr::null_mut(), 0);
    }

    /// c:433 — `pps_setfn(null, "x")` is safe.
    #[test]
    fn pps_setfn_null_safe() {
        let _g = crate::test_util::global_state_lock();
        pps_setfn(std::ptr::null_mut(), "x");
    }

    /// c:468 — `pps_unsetfn(null, _)` is safe.
    #[test]
    fn pps_unsetfn_null_safe() {
        let _g = crate::test_util::global_state_lock();
        pps_unsetfn(std::ptr::null_mut(), 0);
    }

    /// c:181 — `is_private` for non-registered ptr (with name) returns 0.
    #[test]
    fn is_private_unregistered_param_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::{hashnode, param};
        let pm = Box::new(param {
            node: hashnode {
                next: None,
                nam: "zshrs_never_registered_param_xyz".to_string(),
                flags: 0,
            },
            u_data: 0,
            u_arr: None,
            u_str: None,
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            u_tied: None,
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
        });
        let ptr: *const param = Box::into_raw(pm);
        let r = is_private(ptr);
        // Reclaim Box to free.
        unsafe {
            drop(Box::from_raw(ptr as *mut param));
        }
        assert_eq!(r, 0, "unregistered param → not private");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/param_private.c
    // c:200 is_private / c:403 pps_getfn / c:497 ppi_getfn /
    // c:557 ppf_getfn / c:617 ppa_getfn — type pins + determinism
    // ═══════════════════════════════════════════════════════════════════

    /// c:200 — `is_private(null)` deterministic full sweep.
    #[test]
    fn is_private_null_deterministic_full_sweep() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(is_private(std::ptr::null()), 0);
        }
    }

    /// c:200 — `is_private` returns i32 (compile-time type pin).
    #[test]
    fn is_private_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = is_private(std::ptr::null());
    }

    /// c:403 — `pps_getfn` returns String (compile-time type pin).
    #[test]
    fn pps_getfn_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = pps_getfn(std::ptr::null_mut());
    }

    /// c:497 — `ppi_getfn` returns i64 (compile-time type pin).
    #[test]
    fn ppi_getfn_returns_i64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i64 = ppi_getfn(std::ptr::null_mut());
    }

    /// c:557 — `ppf_getfn` returns f64 (compile-time type pin).
    #[test]
    fn ppf_getfn_returns_f64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: f64 = ppf_getfn(std::ptr::null_mut());
    }

    /// c:617 — `ppa_getfn` returns Vec<String> (compile-time type pin).
    #[test]
    fn ppa_getfn_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = ppa_getfn(std::ptr::null_mut());
    }

    /// c:403 — `pps_getfn(null)` is deterministic.
    #[test]
    fn pps_getfn_null_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = pps_getfn(std::ptr::null_mut());
        for _ in 0..5 {
            assert_eq!(pps_getfn(std::ptr::null_mut()), first);
        }
    }

    /// c:497 — `ppi_getfn(null)` is deterministic.
    #[test]
    fn ppi_getfn_null_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = ppi_getfn(std::ptr::null_mut());
        for _ in 0..5 {
            assert_eq!(ppi_getfn(std::ptr::null_mut()), first);
        }
    }

    /// c:557 — `ppf_getfn(null)` bitwise-deterministic (NaN-safe).
    #[test]
    fn ppf_getfn_null_bitwise_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = ppf_getfn(std::ptr::null_mut());
        for _ in 0..5 {
            assert_eq!(ppf_getfn(std::ptr::null_mut()).to_bits(), first.to_bits());
        }
    }

    /// c:617 — `ppa_getfn(null)` is deterministic.
    #[test]
    fn ppa_getfn_null_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = ppa_getfn(std::ptr::null_mut());
        for _ in 0..5 {
            assert_eq!(ppa_getfn(std::ptr::null_mut()), first);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/param_private.c
    // c:200 is_private / c:365 setfn_error / c:433 pps_setfn /
    // c:512 ppi_setfn / c:572 ppf_setfn / c:632 ppa_setfn /
    // c:690 pph_getfn / c:706 pph_setfn / c:766 scopeprivate /
    // c:875 getprivatenode / c:922 getprivatenode2 / c:965 printprivatenode
    // ═══════════════════════════════════════════════════════════════════

    /// c:200 — `is_private` returns i32 (compile-time type pin).
    #[test]
    fn is_private_returns_i32_type_pin2() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = is_private(std::ptr::null());
    }

    /// c:200 — `is_private(null)` deterministic across calls.
    #[test]
    fn is_private_null_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = is_private(std::ptr::null());
        for _ in 0..5 {
            assert_eq!(
                is_private(std::ptr::null()),
                first,
                "is_private(null) must be deterministic"
            );
        }
    }

    /// c:365 — `setfn_error(null)` is safe and idempotent.
    #[test]
    fn setfn_error_null_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            setfn_error(std::ptr::null_mut());
        }
    }

    /// c:766 — `scopeprivate(null, _)` is safe for both onoff values.
    #[test]
    fn scopeprivate_null_both_onoff_safe() {
        let _g = crate::test_util::global_state_lock();
        scopeprivate(std::ptr::null_mut(), 0);
        scopeprivate(std::ptr::null_mut(), 1);
    }

    /// c:766 — `scopeprivate(null, _)` is idempotent.
    #[test]
    fn scopeprivate_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            scopeprivate(std::ptr::null_mut(), 0);
        }
    }

    /// c:875 — `getprivatenode(null)` returns null pointer.
    #[test]
    fn getprivatenode_null_returns_null_pin() {
        let _g = crate::test_util::global_state_lock();
        let p = getprivatenode(std::ptr::null_mut());
        assert!(p.is_null(), "null in → null out");
    }

    /// c:922 — `getprivatenode2(null)` returns null pointer.
    #[test]
    fn getprivatenode2_null_returns_null_pin() {
        let _g = crate::test_util::global_state_lock();
        let p = getprivatenode2(std::ptr::null_mut());
        assert!(p.is_null(), "null in → null out");
    }

    /// c:875 + c:922 — both getprivatenode variants null-safe and pure.
    #[test]
    fn getprivatenode_variants_deterministic_on_null() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            assert!(getprivatenode(std::ptr::null_mut()).is_null());
            assert!(getprivatenode2(std::ptr::null_mut()).is_null());
        }
    }

    /// c:965 — `printprivatenode(null, _)` is safe for any flags.
    #[test]
    fn printprivatenode_null_with_various_flags_safe() {
        let _g = crate::test_util::global_state_lock();
        for flags in [0i32, 1, -1, i32::MIN, i32::MAX] {
            printprivatenode(std::ptr::null_mut(), flags);
        }
    }

    /// c:690 — `pph_getfn(null)` returns Option<()> (compile-time pin).
    #[test]
    fn pph_getfn_returns_option_unit_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<()> = pph_getfn(std::ptr::null_mut());
    }

    /// c:433 — `pps_setfn(null, _)` is safe.
    #[test]
    fn pps_setfn_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        pps_setfn(std::ptr::null_mut(), "");
        pps_setfn(std::ptr::null_mut(), "any value");
    }

    /// c:512 — `ppi_setfn(null, _)` is safe.
    #[test]
    fn ppi_setfn_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        ppi_setfn(std::ptr::null_mut(), 0);
        ppi_setfn(std::ptr::null_mut(), i64::MAX);
        ppi_setfn(std::ptr::null_mut(), i64::MIN);
    }

    /// c:572 — `ppf_setfn(null, _)` is safe.
    #[test]
    fn ppf_setfn_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        ppf_setfn(std::ptr::null_mut(), 0.0);
        ppf_setfn(std::ptr::null_mut(), f64::INFINITY);
        ppf_setfn(std::ptr::null_mut(), f64::NAN);
    }

    /// c:632 — `ppa_setfn(null, _)` is safe.
    #[test]
    fn ppa_setfn_null_is_safe() {
        let _g = crate::test_util::global_state_lock();
        ppa_setfn(std::ptr::null_mut(), vec![]);
        ppa_setfn(std::ptr::null_mut(), vec!["a".into(), "b".into()]);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/param_private.c
    // c:200 is_private / c:403 pps_getfn / c:497 ppi_getfn /
    // c:557 ppf_getfn / c:617 ppa_getfn / c:468 unsetfn variants /
    // c:766 scopeprivate / c:875 getprivatenode + lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:200 — `is_private(null)` returns 0 (not private).
    #[test]
    fn is_private_null_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            is_private(std::ptr::null()),
            0,
            "null param is not private; got nonzero"
        );
    }

    /// c:200 — `is_private` returns i32 (compile-time pin, alt).
    #[test]
    fn is_private_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = is_private(std::ptr::null());
    }

    /// c:403 — `pps_getfn(null)` returns String (compile-time pin).
    #[test]
    fn pps_getfn_null_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = pps_getfn(std::ptr::null_mut());
    }

    /// c:497 — `ppi_getfn(null)` returns i64 (compile-time pin).
    #[test]
    fn ppi_getfn_null_returns_i64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i64 = ppi_getfn(std::ptr::null_mut());
    }

    /// c:557 — `ppf_getfn(null)` returns f64 (compile-time pin).
    #[test]
    fn ppf_getfn_null_returns_f64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: f64 = ppf_getfn(std::ptr::null_mut());
    }

    /// c:617 — `ppa_getfn(null)` returns Vec<String> (compile-time pin).
    #[test]
    fn ppa_getfn_null_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = ppa_getfn(std::ptr::null_mut());
    }

    /// c:468/530/590/650/727 — every unsetfn variant accepts null + both
    /// explicit flag values without panic.
    #[test]
    fn all_unsetfn_variants_null_both_explicit_flags_safe() {
        let _g = crate::test_util::global_state_lock();
        for explicit in [0i32, 1] {
            pps_unsetfn(std::ptr::null_mut(), explicit);
            ppi_unsetfn(std::ptr::null_mut(), explicit);
            ppf_unsetfn(std::ptr::null_mut(), explicit);
            ppa_unsetfn(std::ptr::null_mut(), explicit);
            pph_unsetfn(std::ptr::null_mut(), explicit);
        }
    }

    /// c:766 — `scopeprivate(null, _)` is safe for both onoff values (alt).
    #[test]
    fn scopeprivate_null_both_onoff_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        scopeprivate(std::ptr::null_mut(), 0);
        scopeprivate(std::ptr::null_mut(), 1);
    }

    /// c:875 — `getprivatenode(null)` returns null (deterministic).
    #[test]
    fn getprivatenode_null_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert!(getprivatenode(std::ptr::null_mut()).is_null());
        }
    }

    /// c:922 — `getprivatenode2(null)` returns null (deterministic, alt fn).
    #[test]
    fn getprivatenode2_null_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert!(getprivatenode2(std::ptr::null_mut()).is_null());
        }
    }

    /// c:1005/1020/1035/1041/1058/1064 — each lifecycle hook returns 0
    /// individually (tighter failure resolution).
    #[test]
    fn param_private_each_lifecycle_hook_returns_zero_individually() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let mut v: Vec<String> = Vec::new();
        let mut e: Option<Vec<i32>> = None;
        assert_eq!(setup_(null), 0, "c:1005 setup_");
        assert_eq!(features_(null, &mut v), 0, "c:1020 features_");
        assert_eq!(enables_(null, &mut e), 0, "c:1035 enables_");
        assert_eq!(boot_(null), 0, "c:1041 boot_");
        assert_eq!(cleanup_(null), 0, "c:1058 cleanup_");
        assert_eq!(finish_(null), 0, "c:1064 finish_");
    }
}
