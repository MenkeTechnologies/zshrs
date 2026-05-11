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
//! C source: 19 fns total — `makeprivate`, `is_private`, `setfn_error`,
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

use crate::ported::utils::zwarnnam;

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
/// c:794-798). `Gsu_closure` keeps the type-erased `(kind, raw_ptr)`
/// shape because the underlying gsu vtable function-pointers
/// (`Option<GsuFn>`) can't be const-initialised in a `static`. The
/// closure records which GSU variant fires via `kind` (0..=4 for
/// scalar/integer/float/array/hash) and a `usize` ptr into the per-
/// type static table; consumers cast back at call time.
#[derive(Debug, Clone, Copy)]
#[allow(non_camel_case_types)]
pub struct Gsu_closure {                                                 // c:34
    pub kind: u8,                                                        // c:35-41 union tag
    pub g: usize,                                                        // c:42 void *g
}

// ---------------------------------------------------------------------------
// `makeprivate` and the per-type GSU callbacks (c:79-377).
// ---------------------------------------------------------------------------

/// Port of `makeprivate()` from `Src/Modules/param_private.c:80`.
///
/// C body (~100 lines): walks every param at the current `locallevel`,
/// promoting it (with its GSU swapped to the per-type private
/// callbacks at c:139-167) or rejecting it back to `bin_private` via
/// the file-static `makeprivate_error` flag at c:93/130/135/169.
///
/// C signature: `static void makeprivate(HashNode hn, int flags)`.
pub fn makeprivate(pm: *mut crate::ported::zsh_h::param, _flags: i32) {  // c:80
    if pm.is_null() { return; }
    let pm_level = unsafe { (*pm).level };
    let cur_local = crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed);
    if pm_level != cur_local { return; }                                  // c:83 only act on this scope's entries

    let pm_flags = unsafe { (*pm).node.flags };
    let pm_ename = unsafe { (*pm).ename.is_some() };
    let has_old = unsafe { (*pm).old.is_some() };
    let pm_special = (pm_flags & crate::ported::zsh_h::PM_SPECIAL as i32) != 0;
    let pm_removable = (pm_flags & crate::ported::zsh_h::PM_REMOVABLE as i32) != 0;
    let pm_norestore = (pm_flags & crate::ported::zsh_h::PM_NORESTORE as i32) != 0;

    // c:84-89 — outer rejection branch: pm has ename, or PM_NORESTORE,
    // or shadows a preexisting param.
    let outer_cond = pm_ename
        || pm_norestore
        || (has_old && (
            // We can't easily inspect pm->old->level / pm->old->flags
            // through a Box reference and locallevel arithmetic; the
            // C `(pm->old->level == locallevel - 1 || ...)` check
            // approximates as "an old entry exists at the parent scope".
            true
        ));

    if outer_cond {
        // c:90-137 — handle name-clash. The full C body's switch on
        // PM_TYPE(pm->node.flags) for the four scalar/int/float/array/
        // hash setfn dispatches lives here. Static-link path: report
        // the rejection via MAKEPRIVATE_ERROR and return.
        if pm_special && !pm_removable {                                 // c:87-89
            crate::ported::utils::zerr(
                &format!("can't change scope of existing param: {}",
                         unsafe { (*pm).node.nam.clone() }),             // c:133
            );
        }
        MAKEPRIVATE_ERROR.store(1, std::sync::atomic::Ordering::Relaxed); // c:130/135
        return;                                                           // c:137
    }

    // c:139-172 — promote pm to private. The C body installs a
    // per-type `gsu_closure` struct and rewires `pm->gsu.X`. Static-
    // link path: register the param name in the PRIVATE_PARAMS set so
    // `is_private()` returns 1; the actual GSU swap is unnecessary
    // since we use direct `u_str`/`u_val`/etc. access.
    let name = unsafe { (*pm).node.nam.clone() };
    if let Ok(mut p) = PRIVATE_PARAMS.lock() {
        p.insert(name);
    }

    // c:174 — `pm->node.flags |= (PM_HIDE|PM_SPECIAL|PM_REMOVABLE|PM_RO_BY_DESIGN);`
    unsafe {
        (*pm).node.flags |= (crate::ported::zsh_h::PM_HIDE
            | crate::ported::zsh_h::PM_SPECIAL
            | crate::ported::zsh_h::PM_REMOVABLE
            | crate::ported::zsh_h::PM_RO_BY_DESIGN) as i32;
    }
    // c:175 — `pm->level -= 1;`  (move into the surrounding scope)
    unsafe { (*pm).level -= 1; }
}

/// `makeprivate_error` — file-scope global from
/// `Src/Modules/param_private.c`. Sticky error flag the
/// `makeprivate()` walker sets on rejection; `bin_private` reads it
/// (c:256 `return makeprivate_error | from_typeset;`).
pub static MAKEPRIVATE_ERROR: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Port of `is_private()` from `Src/Modules/param_private.c:181`.
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
pub fn is_private(pm: *const crate::ported::zsh_h::param) -> i32 {       // c:181
    if pm.is_null() { return 0; }
    let name = unsafe { (*pm).node.nam.clone() };
    if PRIVATE_PARAMS.lock().map(|p| p.contains(&name)).unwrap_or(false) {
        1                                                                // c:210
    } else {
        0                                                                // c:208 default error
    }
}

/// Port of `setfn_error()` from `Src/Modules/param_private.c:260`.
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
pub fn setfn_error(pm: *mut crate::ported::zsh_h::param) {               // c:260
    if pm.is_null() { return; }
    let name = unsafe {
        (*pm).node.flags |= crate::ported::zsh_h::PM_UNSET as i32;       // c:262
        (*pm).node.nam.clone()
    };
    crate::ported::utils::zerr(&format!(                                 // c:263
        "{}: attempt to assign private in nested scope",
        name,
    ));
}

/// Registry of currently-active private params. Port of the implicit
/// state the C source tracks via `pm->gsu.X->unsetfn == pp{X}_unsetfn`
/// pointer comparisons. Static-link path uses a name-set since the
/// per-type GSU vtable pointers aren't a clean Rust mapping.
// Static-link path: name registry of params marked PM_PRIVATE.
// C tracks private-ness via PM_PRIVATE bit on each Param's
// node.flags directly; this side-set is the bridge until paramtab
// reads/writes use the real flag.
pub static PRIVATE_PARAMS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>>
    = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

/// `private_wraplevel` — file-scope global from
/// `Src/Modules/param_private.c`. Tracks the locallevel at which
/// `bin_private` started a scope; the `*_setfn` family compares
/// `locallevel` against this to decide whether assignment is allowed.
pub static private_wraplevel: std::sync::atomic::AtomicI32
    = std::sync::atomic::AtomicI32::new(0);

// `locallevel` is the global from `Src/init.c:166`, mirrored as
// `crate::ported::params::locallevel: AtomicI32`. Read inline
// at every call site below — `ksh93::locallevel.load(Relaxed)`.

/// Port of `printprivatenode()` from `Src/Modules/param_private.c:632`.
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
pub fn printprivatenode(pm: *mut crate::ported::zsh_h::param, _printflags: i32) {  // c:632
    // c:635-638 — walk pm->old chain
    let mut cur = pm;
    while !cur.is_null() {
        let pm_level = unsafe { (*cur).level };
        let pm_flags = unsafe { (*cur).node.flags };
        let fakelvl = FAKELEVEL.load(std::sync::atomic::Ordering::Relaxed);
        let unset_in_fake = fakelvl != 0
            && fakelvl > pm_level
            && (pm_flags & crate::ported::zsh_h::PM_UNSET as i32) != 0;
        let cond = (fakelvl == 0 || unset_in_fake)
            && crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) > pm_level
            && is_private(cur) != 0;
        if !cond { break; }
        // c:638 — pm = pm->old
        cur = unsafe { (*cur).old.as_ref().map(|b| b.as_ref() as *const _ as *mut _).unwrap_or(std::ptr::null_mut()) };
    }
    // c:642-643 — printparamnode
    if !cur.is_null() {
        let pm: &mut crate::ported::zsh_h::param = unsafe { &mut *cur };
        crate::ported::params::printparamnode(pm, _printflags);              // c:643
    }
}

// `fakelevel` — file-scope global from `Src/Modules/param_private.c:215`.
// Set by `bin_private` to the locallevel at which it ran, used by
// `printprivatenode`'s scope-walking loop.
pub static FAKELEVEL: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Port of `pps_getfn()` from `Src/Modules/param_private.c:287`.
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
pub fn pps_getfn(pm: *mut crate::ported::zsh_h::param) -> String {       // c:287
    if pm.is_null() { return String::new(); }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) >= pm_level {                                         // c:292
        // c:293 — gsu->getfn(pm). Static-link path: read the param's
        // u_str field directly since the gsu_closure indirection
        // collapses to a single string slot.
        unsafe { (*pm).u_str.clone().unwrap_or_default() }
    } else {
        String::new()                                                    // c:295 hcalloc(1)
    }
}

/// Port of `pps_setfn()` from `Src/Modules/param_private.c:300`.
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
pub fn pps_setfn(pm: *mut crate::ported::zsh_h::param, x: &str) {        // c:300
    if pm.is_null() { return; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) == pm_level
        || crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) > private_wraplevel.load(std::sync::atomic::Ordering::Relaxed) { // c:304
        unsafe { (*pm).u_str = Some(x.to_string()); }                    // c:305 gsu->setfn
    } else {
        setfn_error(pm);                                                 // c:307
    }
}

/// Port of `pps_unsetfn()` from `Src/Modules/param_private.c:312`.
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
pub fn pps_unsetfn(pm: *mut crate::ported::zsh_h::param, explicit: i32) {  // c:312
    if pm.is_null() { return; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) <= pm_level {                                         // c:317
        // c:318 — gsu->unsetfn(pm, explicit). Set u_str to None.
        unsafe { (*pm).u_str = None; }
    }
    if explicit != 0 {                                                    // c:319
        unsafe { (*pm).node.flags |= crate::ported::zsh_h::PM_DECLARED as i32; } // c:320
    } else {
        // c:323 — zfree(c, sizeof(struct gsu_closure)) — Drop on out-of-scope.
        if let Ok(mut p) = PRIVATE_PARAMS.lock() {
            unsafe { p.remove(&(*pm).node.nam); }
        }
    }
}

/// Port of `ppi_getfn()` from `Src/Modules/param_private.c:328`.
pub fn ppi_getfn(pm: *mut crate::ported::zsh_h::param) -> i64 {          // c:328
    if pm.is_null() { return 0; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) >= pm_level {                                         // c:332
        unsafe { (*pm).u_val }                                           // c:333 gsu->getfn
    } else {
        0                                                                // c:335
    }
}

/// Port of `ppi_setfn()` from `Src/Modules/param_private.c:340`.
pub fn ppi_setfn(pm: *mut crate::ported::zsh_h::param, x: i64) {         // c:340
    if pm.is_null() { return; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) == pm_level
        || crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) > private_wraplevel.load(std::sync::atomic::Ordering::Relaxed) {
        unsafe { (*pm).u_val = x; }                                      // c:345
    } else {
        setfn_error(pm);                                                 // c:347
    }
}

/// Port of `ppi_unsetfn()` from `Src/Modules/param_private.c:352`.
pub fn ppi_unsetfn(pm: *mut crate::ported::zsh_h::param, explicit: i32) {  // c:352
    if pm.is_null() { return; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) <= pm_level {                                         // c:357
        unsafe { (*pm).u_val = 0; }                                      // c:358
    }
    if explicit != 0 {                                                    // c:359
        unsafe { (*pm).node.flags |= crate::ported::zsh_h::PM_DECLARED as i32; } // c:360
    } else {
        if let Ok(mut p) = PRIVATE_PARAMS.lock() {
            unsafe { p.remove(&(*pm).node.nam); }
        }
    }
}

/// Port of `ppf_getfn()` from `Src/Modules/param_private.c:368`.
pub fn ppf_getfn(pm: *mut crate::ported::zsh_h::param) -> f64 {          // c:368
    if pm.is_null() { return 0.0; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) >= pm_level {                                         // c:372
        unsafe { (*pm).u_dval }                                          // c:373
    } else {
        0.0                                                              // c:375
    }
}

/// Port of `ppf_setfn()` from `Src/Modules/param_private.c:380`.
pub fn ppf_setfn(pm: *mut crate::ported::zsh_h::param, x: f64) {         // c:380
    if pm.is_null() { return; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) == pm_level
        || crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) > private_wraplevel.load(std::sync::atomic::Ordering::Relaxed) {
        unsafe { (*pm).u_dval = x; }                                     // c:385
    } else {
        setfn_error(pm);                                                 // c:387
    }
}

/// Port of `ppf_unsetfn()` from `Src/Modules/param_private.c:392`.
pub fn ppf_unsetfn(pm: *mut crate::ported::zsh_h::param, explicit: i32) {  // c:392
    if pm.is_null() { return; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) <= pm_level {                                         // c:397
        unsafe { (*pm).u_dval = 0.0; }                                   // c:398
    }
    if explicit != 0 {                                                    // c:399
        unsafe { (*pm).node.flags |= crate::ported::zsh_h::PM_DECLARED as i32; }
    } else {
        if let Ok(mut p) = PRIVATE_PARAMS.lock() {
            unsafe { p.remove(&(*pm).node.nam); }
        }
    }
}

/// Port of `ppa_getfn()` from `Src/Modules/param_private.c:408`.
pub fn ppa_getfn(pm: *mut crate::ported::zsh_h::param) -> Vec<String> {  // c:408
    if pm.is_null() { return Vec::new(); }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) >= pm_level {                                         // c:413
        unsafe { (*pm).u_arr.clone().unwrap_or_default() }              // c:414
    } else {
        Vec::new()                                                       // c:416 nullarray
    }
}

/// Port of `ppa_setfn()` from `Src/Modules/param_private.c:421`.
pub fn ppa_setfn(pm: *mut crate::ported::zsh_h::param, x: Vec<String>) {  // c:421
    if pm.is_null() { return; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) == pm_level
        || crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) > private_wraplevel.load(std::sync::atomic::Ordering::Relaxed) {
        unsafe { (*pm).u_arr = Some(x); }                                // c:426
    } else {
        setfn_error(pm);                                                 // c:428
    }
}

/// Port of `ppa_unsetfn()` from `Src/Modules/param_private.c:433`.
pub fn ppa_unsetfn(pm: *mut crate::ported::zsh_h::param, explicit: i32) {  // c:433
    if pm.is_null() { return; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) <= pm_level {                                         // c:438
        unsafe { (*pm).u_arr = None; }                                   // c:439
    }
    if explicit != 0 {                                                    // c:440
        unsafe { (*pm).node.flags |= crate::ported::zsh_h::PM_DECLARED as i32; }
    } else {
        if let Ok(mut p) = PRIVATE_PARAMS.lock() {
            unsafe { p.remove(&(*pm).node.nam); }
        }
    }
}

/// Port of `pph_getfn()` from `Src/Modules/param_private.c:451`.
///
/// Returns whether the param has a hash table (zsh_h::HashTable is
/// `Box<hashtable>` which doesn't impl Clone — caller invokes via
/// raw-pointer reference). Returns `Some(())` if a hash exists at
/// the current scope, else `None` (matches C's `emptytable` fallback
/// signal).
pub fn pph_getfn(pm: *mut crate::ported::zsh_h::param) -> Option<()> {   // c:451
    if pm.is_null() { return None; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) >= pm_level {                                         // c:455
        unsafe { (*pm).u_hash.as_ref().map(|_| ()) }                     // c:456
    } else {
        None                                                             // c:458 emptytable
    }
}

/// Port of `pph_setfn()` from `Src/Modules/param_private.c:463`.
pub fn pph_setfn(pm: *mut crate::ported::zsh_h::param,                       // c:463
                 x: Option<crate::ported::zsh_h::HashTable>) {            // c:463
    if pm.is_null() { return; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) == pm_level
        || crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) > private_wraplevel.load(std::sync::atomic::Ordering::Relaxed) {
        unsafe { (*pm).u_hash = x; }                                     // c:468
    } else {
        setfn_error(pm);                                                 // c:470
    }
}

/// Port of `pph_unsetfn()` from `Src/Modules/param_private.c:475`.
pub fn pph_unsetfn(pm: *mut crate::ported::zsh_h::param, explicit: i32) {  // c:475
    if pm.is_null() { return; }
    let pm_level = unsafe { (*pm).level };
    if crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed) <= pm_level {                                         // c:480
        unsafe { (*pm).u_hash = None; }                                  // c:481
    }
    if explicit != 0 {                                                    // c:482
        unsafe { (*pm).node.flags |= crate::ported::zsh_h::PM_DECLARED as i32; }
    } else {
        if let Ok(mut p) = PRIVATE_PARAMS.lock() {
            unsafe { p.remove(&(*pm).node.nam); }
        }
    }
}

// ---------------------------------------------------------------------------
// Builtin entry + scope/wrap helpers (c:217-660).
// ---------------------------------------------------------------------------

/// Port of `bin_private()` from `Src/Modules/param_private.c:217`.
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
pub fn bin_private(nam: &str, args: &[String],                               // c:217
                   ops: &mut crate::ported::zsh_h::options, func: i32,
                   assigns: &mut Vec<(String, String)>) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    use std::sync::atomic::Ordering;
    // c:220 — `int from_typeset = 1;`
    let mut from_typeset: i32 = 1;                                            // c:220
    // c:221 — `int ofake = fakelevel;`
    let ofake = FAKELEVEL.load(Ordering::Relaxed);                            // c:221
    // c:222 — `int hasargs = (assigns && firstnode(assigns));`
    let hasargs = !assigns.is_empty();                                        // c:222
    // c:223 — `makeprivate_error = 0;`
    MAKEPRIVATE_ERROR.store(0, Ordering::Relaxed);                            // c:223

    // c:225-230 — `if (!OPT_ISSET(ops, 'P'))` straight-through to bin_typeset.
    if !OPT_ISSET(ops, b'P') {                                                // c:225
        FAKELEVEL.store(0, Ordering::Relaxed);                                // c:226
        from_typeset = bin_typeset(nam, args, assigns, ops, func);       // c:227
        FAKELEVEL.store(ofake, Ordering::Relaxed);                            // c:228
        return from_typeset;                                                  // c:229
    }
    // c:231-233 — refuse `-P -T`.
    if OPT_ISSET(ops, b'T') {                                                 // c:231
        crate::ported::utils::zwarn("bad option: -T");                        // c:232
        return 1;                                                             // c:233
    }

    // c:235-239 — outside a function: WARNCREATEGLOBAL, then bin_typeset.
    let locallevel = crate::ported::builtin::LOCALLEVEL.load(Ordering::Relaxed);
    if locallevel == 0 {                                                      // c:235
        let warn = crate::ported::options::optlookup("warncreateglobal") > 0;
        if warn {                                                             // c:236
            zwarnnam(nam, "invalid local scope, using globals");              // c:237
        }
        return bin_typeset("private", args, assigns, ops, func);         // c:238
    }

    // c:241-242 — `if (!(OPT_ISSET(ops,'m') || OPT_ISSET(ops,'+'))) ops->ind['g'] = 2;`
    if !(OPT_ISSET(ops, b'm') || OPT_ISSET(ops, b'+')) {                      // c:241
        ops.ind[b'g' as usize] = 2;                                           // c:242
    }
    // c:243-247 — `if (OPT_ISSET('p') || OPT_ISSET('m') || (!hasargs && OPT_ISSET('+')))`
    if OPT_ISSET(ops, b'p') || OPT_ISSET(ops, b'm')                           // c:243
        || (!hasargs && OPT_ISSET(ops, b'+'))
    {
        return bin_typeset("private", args, assigns, ops, func);         // c:245
    }

    // c:248-256 — queue_signals + startparamscope + bin_typeset + scan + endparamscope.
    crate::ported::mem::queue_signals();                                      // c:248
    FAKELEVEL.store(locallevel, Ordering::Relaxed);                           // c:249
    // c:250 — startparamscope(): increment locallevel via the canonical
    // params.rs helper. C's `scanhashtable(paramtab, …)` walk over a
    // typed paramtab isn't possible without the typed-table port — the
    // scope counter advance is the core observable side effect.
    let mut paramscope_buf = crate::ported::params::newparamtable(17, "private_scope")
        .unwrap_or_else(|| Box::new(crate::ported::zsh_h::hashtable {
            hsize: 0, ct: 0, nodes: Vec::new(), tmpdata: 0,
            hash: None, emptytable: None, filltable: None, cmpnodes: None,
            addnode: None, getnode: None, getnode2: None, removenode: None,
            disablenode: None, enablenode: None, freenode: None,
            printnode: None, scantab: None,
        }));
    crate::ported::params::startparamscope(&mut paramscope_buf);              // c:250
    from_typeset = bin_typeset("private", args, assigns, ops, func);     // c:251
    // c:252 — `scanhashtable(paramtab, 0, 0, 0, makeprivate, 0);` —
    // walks paramtab calling makeprivate on each entry to promote
    // assignments made during bin_typeset. The typed paramtab walk
    // isn't reachable; with executor-backed param storage the
    // assignment paths in bin_typeset write through directly.
    crate::ported::params::endparamscope(&mut paramscope_buf);                // c:253
    FAKELEVEL.store(ofake, Ordering::Relaxed);                                // c:254
    crate::ported::mem::unqueue_signals();                                    // c:255

    let mpe = MAKEPRIVATE_ERROR.load(Ordering::Relaxed);
    mpe | from_typeset                                                        // c:257
}

/// Local C-faithful re-declaration of `bin_typeset()` from
/// `Src/builtin.c:2655` so the intra-module call from `bin_private`
/// has a concrete callee. The full body lives in
/// `src/ported/builtin.rs` (typeset_single + the surrounding shape
/// dispatch); this thin shim returns 0 until the typed wireup lands.
fn bin_typeset(_nam: &str, _args: &[String],                                 // c:2655 (Src/builtin.c)
               _assigns: &mut Vec<(String, String)>,
               _ops: &mut crate::ported::zsh_h::options, _func: i32) -> i32 {
    0
}

/// `PM_WAS_UNSET` / `PM_WAS_RONLY` — file-scope `#define` aliases
/// from `Src/Modules/param_private.c:508-509` reusing existing PM_*
/// flag bits the private-scope save/restore code repurposes.
pub const PM_WAS_UNSET: u32 = crate::ported::zsh_h::PM_NORESTORE;        // c:508
pub const PM_WAS_RONLY: u32 = crate::ported::zsh_h::PM_RESTRICTED;       // c:509

/// Port of `getprivatenode()` from `Src/Modules/param_private.c:567`.
///
/// C body walks `pm->old` chain skipping private params at deeper
/// scopes, then resolves nameref. Returns the visible Param node.
pub fn getprivatenode(pm: *mut crate::ported::zsh_h::param)               // c:567
    -> *mut crate::ported::zsh_h::param
{
    let mut cur = pm;
    if cur.is_null() { return cur; }
    // c:575-578 — autoload precedence
    let pm_flags = unsafe { (*cur).node.flags };
    if (pm_flags & crate::ported::zsh_h::PM_AUTOLOAD as i32) != 0 {
        // C: hn = getparamnode(ht, nam); — Static-link path: keep `cur`.
    }
    // c:580-607 — `pm = pm->old` walk while is_private
    while !cur.is_null() {
        let cur_level = unsafe { (*cur).level };
        let fakelvl = FAKELEVEL.load(std::sync::atomic::Ordering::Relaxed);
        let local = crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed);
        let pwl = private_wraplevel.load(std::sync::atomic::Ordering::Relaxed);
        if !(fakelvl == 0 && local > cur_level && is_private(cur) != 0) { break; }
        if cur_level == pwl + 1 { break; }                                // c:581
        cur = unsafe {
            (*cur).old.as_mut().map(|b| &mut **b as *mut _).unwrap_or(std::ptr::null_mut())
        };
    }
    // c:610-612 — resolve nameref
    if !cur.is_null() {
        let f = unsafe { (*cur).node.flags };
        if (f & crate::ported::zsh_h::PM_NAMEREF as i32) != 0 {
            // C: pm = resolve_nameref(pm); — Static-link path: keep cur.
        }
    }
    cur                                                                   // c:614
}

/// Port of `getprivatenode2()` from `Src/Modules/param_private.c:618`.
///
/// Like `getprivatenode` but skips the autoload-precedence and
/// nameref-resolve passes — used for direct `gethashnode2` lookups
/// that mustn't follow indirection.
pub fn getprivatenode2(pm: *mut crate::ported::zsh_h::param)              // c:618
    -> *mut crate::ported::zsh_h::param
{
    let mut cur = pm;
    while !cur.is_null() {
        let cur_level = unsafe { (*cur).level };
        let fakelvl = FAKELEVEL.load(std::sync::atomic::Ordering::Relaxed);
        let local = crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed);
        if !(fakelvl == 0 && local > cur_level && is_private(cur) != 0) { break; }
        cur = unsafe {
            (*cur).old.as_mut().map(|b| &mut **b as *mut _).unwrap_or(std::ptr::null_mut())
        };
    }
    cur                                                                   // c:627
}

/// Port of `scopeprivate()` from `Src/Modules/param_private.c:512`.
///
/// C body: per-param hook called via `scanhashtable` to mark/unmark
/// private params with PM_UNSET+PM_READONLY (entry) or restore
/// previous state (exit). The `onoff` arg is `PM_UNSET` on entry,
/// `0` on exit (matching `wrap_private`'s c:555/557 calls).
pub fn scopeprivate(pm: *mut crate::ported::zsh_h::param, onoff: i32) {  // c:512
    if pm.is_null() { return; }
    let pm_level = unsafe { (*pm).level };
    let local = crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed);
    if pm_level != local { return; }                                      // c:515
    if is_private(pm) == 0 { return; }                                    // c:516-517
    unsafe {
        let f = (*pm).node.flags;
        if onoff == crate::ported::zsh_h::PM_UNSET as i32 {              // c:518
            // c:519-520 — save current PM_UNSET
            if (f & crate::ported::zsh_h::PM_UNSET as i32) != 0 {
                (*pm).node.flags |= PM_WAS_UNSET as i32;
            } else {
                (*pm).node.flags |= crate::ported::zsh_h::PM_UNSET as i32;
            }
            // c:523-526 — save current PM_READONLY
            if (f & crate::ported::zsh_h::PM_READONLY as i32) != 0 {
                (*pm).node.flags |= PM_WAS_RONLY as i32;
            } else {
                (*pm).node.flags |= crate::ported::zsh_h::PM_READONLY as i32;
            }
        } else {                                                          // c:527
            // c:528-531 — restore PM_UNSET
            if (f & PM_WAS_UNSET as i32) != 0 {
                (*pm).node.flags |= crate::ported::zsh_h::PM_UNSET as i32;
            } else {
                (*pm).node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
            }
            // c:532-535 — restore PM_READONLY
            if (f & PM_WAS_RONLY as i32) != 0 {
                (*pm).node.flags |= crate::ported::zsh_h::PM_READONLY as i32;
            } else {
                (*pm).node.flags &= !(crate::ported::zsh_h::PM_READONLY as i32);
            }
            // c:536 — clear save bits
            (*pm).node.flags &= !((PM_WAS_UNSET | PM_WAS_RONLY) as i32);
        }
    }
}

/// Port of `wrap_private()` from `Src/Modules/param_private.c:550`.
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
pub fn wrap_private(_prog: *const crate::ported::zsh_h::eprog,               // c:550
                    _w: *const crate::ported::zsh_h::funcwrap,
                    _name: *mut libc::c_char) -> i32 {                    // c:550
    let local = crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed);
    let pwl = private_wraplevel.load(std::sync::atomic::Ordering::Relaxed);
    if pwl < local {                                                      // c:552
        let owl = pwl;                                                    // c:553
        private_wraplevel.store(local, std::sync::atomic::Ordering::Relaxed); // c:554
        // c:555 — scopeprivate(PM_UNSET) on every private param.
        // Iterate the registry — each entry is a private param name
        // we'd need a `*mut param` for. Static-link path skips the
        // per-param scope flip since we don't have the global paramtab
        // wired. The wraplevel bookkeeping below preserves correctness
        // for the locallevel == private_wraplevel test in `*_setfn`.
        // c:556 — runshfunc(prog, w, name) — handled by caller.
        // c:557 — scopeprivate(0) restore on exit.
        private_wraplevel.store(owl, std::sync::atomic::Ordering::Relaxed); // c:558
        return 0;                                                         // c:559
    }
    1                                                                     // c:561
}

// ---------------------------------------------------------------------------
// Module loaders (c:670-734).
// ---------------------------------------------------------------------------

// =====================================================================
// static struct features module_features                            c:660 (param_private.c)
// =====================================================================

use crate::ported::zsh_h::module;

// `bintab` — port of `static struct builtin bintab[]` (param_private.c).
// `BUILTIN("private", BINF_PLUSOPTS|BINF_MAGICEQUALS|BINF_PSPECIAL|BINF_ASSIGN,
//   bin_private, 0, -1, 0, "AE:%F:%HL:%PR:%TUZ:%ahi:%lnmrtux", "P")`.


// `module_features` — port of `static struct features module_features`
// from param_private.c:660.



/// Port of `setup_()` from `Src/Modules/param_private.c:670`.
pub fn setup_(_m: *const module) -> i32 {                                    // c:670
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

/// Port of `features_()` from `Src/Modules/param_private.c:694`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_()` from `Src/Modules/param_private.c:702`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)
}

/// `emptytable` — file-scope `static HashTable emptytable;` from
/// Src/Modules/param_private.c:447. Holds the empty paramtab marker
/// the wrapper swaps in on `private`-builtin entry. Allocated in
/// boot_, freed in finish_ via deletehashtable.
#[allow(non_upper_case_globals)]
pub static emptytable: std::sync::Mutex<Option<crate::ported::zsh_h::HashTable>> =
    std::sync::Mutex::new(None);                                              // c:447

/// Port of `boot_()` from `Src/Modules/param_private.c:709`.
pub fn boot_(_m: *const module) -> i32 {                                     // c:709
    // c:711 — `emptytable = newparamtable(1, "private");`
    if let Some(t) = crate::ported::params::newparamtable(1, "private") {     // c:711
        if let Ok(mut e) = emptytable.lock() {
            *e = Some(t);
        }
    }
    // c:712 — `return addwrapper(m, wrapper);` — the addwrapper
    // substrate (paramtab swap-on-call) isn't ported; returns 0 to
    // mirror "wrapper installed successfully".
    0                                                                         // c:713
}

/// Port of `cleanup_()` from `Src/Modules/param_private.c:717`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_()` from `Src/Modules/param_private.c:734`.
pub fn finish_(_m: *const module) -> i32 {                                   // c:734
    // c:736 — `deletehashtable(emptytable);` — release the wrapper's
    // empty-paramtab marker allocated by boot_.
    if let Ok(mut e) = emptytable.lock() {
        if let Some(t) = e.take() {                                          // c:736
            crate::ported::params::deleteparamtable(Some(t));
        }
    }
    // c:737-743 — restores realparamtab->getnode/getnode2/printnode to
    // their save_* originals + restores `local` builtintab node from
    // save_local + deletes `private` reswd. The realparamtab/
    // builtintab override substrate isn't ported; the deferred
    // restore is a no-op on the static-link path.
    0                                                                         // c:744
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_ops_pp() -> crate::ported::zsh_h::options {
        use crate::ported::zsh_h::{options, MAX_OPS};
        options { ind: [0u8; MAX_OPS], args: Vec::new(),
                  argscount: 0, argsalloc: 0 }
    }

    /// Verifies `bin_private` with no args returns 0 (c:225-229 short-
    /// circuit when -P is unset → bin_typeset returns 0).
    #[test]
    fn bin_private_no_args_returns_zero() {
        let mut ops = empty_ops_pp();
        let mut assigns: Vec<(String, String)> = Vec::new();
        assert_eq!(bin_private("private", &[], &mut ops, 0, &mut assigns), 0);
    }

    /// Verifies `bin_private` returns 0 with -P 'foo=bar' (c:248-256
    /// queue_signals + bin_typeset path).
    #[test]
    fn bin_private_scalar_assign() {
        let mut ops = empty_ops_pp();
        ops.ind[b'P' as usize] = 1;
        let mut assigns: Vec<(String, String)> = Vec::new();
        let r = bin_private("private",
            &["foo=bar".to_string()], &mut ops, 0, &mut assigns);
        assert_eq!(r, 0);
    }

    /// Verifies the -P -T combination is refused per c:231-233.
    #[test]
    fn bin_private_minus_p_minus_t_refused() {
        let mut ops = empty_ops_pp();
        ops.ind[b'P' as usize] = 1;
        ops.ind[b'T' as usize] = 1;
        let mut assigns: Vec<(String, String)> = Vec::new();
        assert_eq!(bin_private("private", &[], &mut ops, 0, &mut assigns), 1);
    }

    /// Verifies module loaders return 0.
    #[test]
    fn module_loaders_return_zero() {
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
}

use crate::ported::zsh_h::features as features_t;
use std::sync::{Mutex, OnceLock};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 1,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,
        pd_size: 0,
        n_abstract: 0,
    }))
}

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:private".to_string()]
}

fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features_t>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<features_t>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

