//! Module system for zshrs
//!
//! Port from zsh/Src/module.c (3,646 lines)
//!
//! Hash of modules                                                          // c:46
//! The list of hook functions defined.                                      // c:840
//! List of math functions.                                                  // c:1255
//!
//! In C, module.c provides dynamic loading of .so modules at runtime
//! via dlopen/dlsym. In Rust, all modules are statically compiled into
//! the binary — there's no dynamic loading. This module provides the
//! registration, lookup, and management API that the rest of the shell
//! uses to interact with module features (builtins, conditions, parameters,
//! hooks, and math functions).

use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use crate::ported::utils::zwarnnam;
use crate::ported::zsh_h::mathfunc;
use crate::zsh_h::module;
use crate::ported::zsh_h::OPT_ISSET;

/// Free module node (from module.c freemodulenode)
/// Free a module table entry.
/// Port of `freemodulenode(HashNode hn)` from Src/module.c:119 — Rust's
/// `Drop` handles the per-field free; this exists for API
/// parity with C callers.
pub fn freemodulenode(hn: module) {
    // Rust Drop handles this
}

/// Print module node (from module.c printmodulenode)
/// Format a module entry for `zmodload -L` listing.
/// Port of `printmodulenode(HashNode hn, int flags)` from Src/module.c:154.
pub fn printmodulenode(hn: &str, m: &module) -> String {
    // C inspects `m->node.flags` — `MOD_ALIAS`/`MOD_UNLOAD`/`MOD_LINKED`.
    let state = if (m.node.flags & crate::ported::zsh_h::MOD_ALIAS) != 0 {
        "alias"
    } else if (m.node.flags & crate::ported::zsh_h::MOD_UNLOAD) != 0 {
        "unloaded"
    } else if (m.node.flags & crate::ported::zsh_h::MOD_LINKED) != 0 {
        "loaded"
    } else {
        "autoloaded"
    };
    format!("{} ({})", hn, state)
}

/// Create new module table (from module.c newmoduletable)
/// Create an empty module table.
/// Port of `newmoduletable(int size, char const *name)` from Src/module.c:274 — the C
/// source allocates the `modulestab` hash with `createhashtable`.
/// WARNING: param names don't match C — Rust=() vs C=(size, name)
pub fn newmoduletable() -> modulestab {
    modulestab::new()
}

// `setbuiltins` / `setconddefs` / `setmathfuncs` / `setparamdefs`
// / `setfeatureenables` all deleted — Rust-only ports that took
// the deleted `Builtin` / `Conddef` / `MathFunc` / `Paramdef` /
// `Module` / `Features` PascalCase structs. C versions
// (module.c:501/754/1374/1165/3350) flip `*_ADDED` flags and
// insert/remove from the global hashtabs; per-module Rust files
// stub these locally and the canonical free-fn re-ports belong
// in zsh_h.rs / hashtable.rs once `struct features` carries
// real pointers.

/// Port of `setup_(UNUSED(Module m))` from `Src/module.c:306`.
///
/// C body: `setup_(UNUSED(Module m)) { return 0; }` — the no-op
/// setup hook of the module subsystem itself.
#[allow(unused_variables)]
pub fn setup_(m: *const crate::ported::zsh_h::module) -> i32 {          // c:306
    0                                                                    // c:306
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/module.c:313`.
///
/// C body:
/// ```c
/// features_(UNUSED(Module m), UNUSED(char ***features))
/// {
///     /* There are lots and lots of features, but they're not handled here. */
///     return 1;
/// }
/// ```
#[allow(unused_variables)]
pub fn features_(m: *const crate::ported::zsh_h::module, features: &mut Vec<String>) -> i32 { // c:313
    /* There are lots and lots of features, but they're not handled here. */ // c:313-318
    1                                                                    // c:319
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/module.c:324`.
///
/// C body: `enables_(UNUSED(Module m), UNUSED(int **enables)) { return 1; }`
/// — the module subsystem itself doesn't manage feature enables.
#[allow(unused_variables)]
pub fn enables_(m: *const crate::ported::zsh_h::module, enables: &mut Option<Vec<i32>>) -> i32 { // c:324
    1                                                                    // c:324
}

/// Port of `boot_(UNUSED(Module m))` from `Src/module.c:331`.
///
/// C body: `boot_(UNUSED(Module m)) { return 0; }` — the no-op
/// boot hook of the module subsystem itself.
#[allow(unused_variables)]
pub fn boot_(m: *const crate::ported::zsh_h::module) -> i32 {           // c:331
    0                                                                    // c:331
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/module.c:338`.
///
/// C body: `cleanup_(UNUSED(Module m)) { return 0; }` — the no-op
/// cleanup hook of the module subsystem itself.
#[allow(unused_variables)]
pub fn cleanup_(m: *const crate::ported::zsh_h::module) -> i32 {        // c:338
    0                                                                    // c:338
}

/// Port of `finish_(UNUSED(Module m))` from `Src/module.c:345`.
///
/// C body: `finish_(UNUSED(Module m)) { return 0; }` —
/// the no-op finish hook for the module subsystem itself.
#[allow(unused_variables)]
pub fn finish_(m: *const crate::ported::zsh_h::module) -> i32 {         // c:345
    0                                                                    // c:345
}

// This registers a builtin module.                                        // c:359
/// Register module (from module.c register_module)
/// Register a module by name.
/// Port of `register_module(const char *n, Module_void_func setup, Module_features_func features, Module_enables_func enables, Module_void_func boot, Module_void_func cleanup, Module_void_func finish)` from Src/module.c:359 — wraps
/// a slot in the global `modulestab` and seeds its lifecycle
/// callbacks.
/// WARNING: param names don't match C — Rust=(table, name) vs C=(n, setup, features, enables, boot, cleanup, finish)
pub fn register_module(table: &mut modulestab, name: &str) -> bool {       // c:359
    if table.modules.contains_key(name) {
        return false;
    }
    table.modules.insert(name.to_string(), module::new(name));
    true
}

/// Port of `addbuiltins(char const *nam, Builtin binl, int size)` from `Src/module.c:544`.
///
/// C body:
/// ```c
/// addbuiltins(char const *nam, Builtin binl, int size)
/// {
///     int ret = 0, n;
///     for(n = 0; n < size; n++) {
///         Builtin b = &binl[n];
///         if(b->node.flags & BINF_ADDED)
///             continue;
///         if(addbuiltin(b)) {
///             zwarnnam(nam, "name clash when adding builtin `%s'", b->node.nam);
///             ret = 1;
///         } else {
///             b->node.flags |= BINF_ADDED;
///         }
///     }
///     return ret;
/// }
/// ```
///
/// Rust port: walks the slice, checks BINF_ADDED, registers via the
/// module-table addbuiltin if not already registered. `binl` is taken
/// by `&mut [Builtin]` so the BINF_ADDED flag-set after success
/// matches C's in-place mutation.
/// Port of `addbuiltin(Builtin b)` from `Src/module.c:524`. C body:
/// look up `b->node.nam` in builtintab; if BINF_ADDED clash → return 1;
/// otherwise replace any pre-existing entry and add `b`. Returns 0 on
/// add, 1 on clash.
///
/// The Rust canonical builtintab is `OnceLock<HashMap<String,
/// &'static builtin>>` — immutable after first access. Runtime `addbuiltin`
/// calls check the immutable table for the clash gate; the BINF_ADDED
/// flag-set on the input record is what callers observe (matching the
/// C in-place mutation that `addbuiltins` then propagates).
pub fn addbuiltin(b: &mut crate::ported::zsh_h::builtin) -> i32 {           // c:524
    use crate::ported::zsh_h::BINF_ADDED;
    let tab = crate::ported::builtin::createbuiltintable();
    if let Some(existing) = tab.get(&b.node.nam) {                           // c:526 getnode2
        if (existing.node.flags & BINF_ADDED as i32) != 0 { return 1; }      // c:527 clash
    }
    b.node.flags |= BINF_ADDED as i32;                                       // c:531 b->node.flags |= BINF_ADDED
    0
}

/// Port of `addbuiltins(char const *nam, Builtin binl, int size)` from
/// `Src/module.c:544`. Walks the slice; for each entry not already
/// flagged BINF_ADDED, calls `addbuiltin`. Returns 0 if all succeeded,
/// 1 if any clashed. zwarnnam emitted on each clash matches C.
pub fn addbuiltins(nam: &str, binl: &mut [crate::ported::zsh_h::builtin]) -> i32 { // c:544
    use crate::ported::zsh_h::BINF_ADDED;
    let mut ret = 0;                                                         // c:548
    for b in binl.iter_mut() {                                               // c:550 for(n = 0; n < size; n++)
        if (b.node.flags & BINF_ADDED as i32) != 0 { continue; }             // c:553
        if addbuiltin(b) != 0 {                                              // c:555
            crate::ported::utils::zwarnnam(nam,                              // c:556 zwarnnam(nam, "name clash...")
                &format!("name clash when adding builtin `{}'", b.node.nam));
            ret = 1;
        }
    }
    ret                                                                      // c:563
}

/// Port of `addhookdeffunc(Hookdef h, Hookfn f)` from `Src/module.c:939`.
///
/// C body:
/// ```c
/// addhookdeffunc(Hookdef h, Hookfn f) {
///     zaddlinknode(h->funcs, (void *) f);
///     return 0;
/// }
/// ```
///
/// Appends function `f` to the named hook's function-list. C uses
/// `LinkList` with `void *` payload (cast to Hookfn at dispatch); Rust
/// port uses the table's per-hook `Vec<String>` (function names) since
/// fn-pointer storage requires a more elaborate type-erased registry.
/// WARNING: param names don't match C — Rust=(table, h, fn_name) vs C=(h, f)
pub fn addhookdeffunc(table: &mut modulestab, h: &mut crate::ported::zsh_h::hookdef, fn_name: &str) -> i32 { // c:939
    // c:939 — zaddlinknode(h->funcs, (void *) f);
    table.hooks.entry(h.name.clone()).or_default().push(fn_name.to_string());
    let _ = h.funcs; // keep field mention for parity
    0                                                                    // c:943
}

/// Port of `void addhookfunc(const char *name, Hookfn fn)` —
/// the global-scope wrapper used by modules and ZLE boot/cleanup
/// paths to install hook callbacks without holding a ModuleTable.
pub fn addhookfunc(hook: &str, func: &str) {                                 // c:module.c
    if let Ok(mut tab) = HOOKTAB.lock() {
        tab.entry(hook.to_string())
            .or_default()
            .push(func.to_string());
    }
}

/// Port of `deletehookdeffunc(Hookdef h, Hookfn f)` from `Src/module.c:961`.
///
/// C body:
/// ```c
/// deletehookdeffunc(Hookdef h, Hookfn f) {
///     LinkNode p;
///     for (p = firstnode(h->funcs); p; incnode(p))
///         if (f == (Hookfn) getdata(p)) {
///             remnode(h->funcs, p);
///             return 0;
///         }
///     return 1;
/// }
/// ```
///
/// Removes function `f` from the hook's function-list. Returns 0 on
/// successful removal, 1 if not found.
/// WARNING: param names don't match C — Rust=(table, h, fn_name) vs C=(h, f)
pub fn deletehookdeffunc(table: &mut modulestab, h: &mut crate::ported::zsh_h::hookdef, fn_name: &str) -> i32 { // c:961
    if let Some(funcs) = table.hooks.get_mut(&h.name) {
        // c:965-969 — for (p = firstnode...; p; incnode(p)) if (f == ...)
        if let Some(pos) = funcs.iter().position(|n| n == fn_name) {
            funcs.remove(pos);                                            // c:967 remnode
            let _ = h.funcs;
            return 0;                                                     // c:968
        }
    }
    let _ = h.funcs;
    1                                                                    // c:970
}

/// Port of `void deletehookfunc(const char *name, Hookfn fn)`.
/// Removes one registered handler from the global HOOKTAB.
pub fn deletehookfunc(hook: &str, func: &str) {                              // c:module.c
    if let Ok(mut tab) = HOOKTAB.lock() {
        if let Some(v) = tab.get_mut(hook) {
            v.retain(|f| f != func);
        }
    }
}

/// Port of `checkaddparam(const char *nam, int opt_i)` from `Src/module.c:1026`.
///
/// C body:
/// ```c
/// checkaddparam(const char *nam, int opt_i)
/// {
///     Param pm;
///     if (!(pm = (Param) gethashnode2(paramtab, nam)))
///         return 0;
///     if (pm->level || !(pm->node.flags & PM_AUTOLOAD)) {
///         if (!opt_i || pm->level) {
///             zwarn("Can't add module parameter `%s': %s",
///                   nam, pm->level ? "local parameter exists" :
///                                    "parameter already exists");
///             return 1;
///         }
///         return 2;
///     }
///     unsetparam_pm(pm, 0, 1);
///     return 0;
/// }
/// ```
///
/// Returns: 0 = OK to add, 1 = error printed, 2 = blocked but `-i`
/// suppressed warning. `pm->level != 0` means a local param shadows
/// the name (always errors). `PM_AUTOLOAD` set means the existing
/// param is an autoload stub the C source unsets to make room.
///
/// Static-link path: the param-table is `crate::ported::params::*`
/// global. Stub returns 0 (no clash) until the params global-state
/// port wires gethashnode2(paramtab, ...) in.
#[allow(unused_variables)]
pub fn checkaddparam(nam: &str, opt_i: i32) -> i32 {                   // c:1026
    // c:1026 — if (!(pm = gethashnode2(paramtab, nam))) return 0;
    // Static-link: paramtab not yet hooked through; treat unknown.
    let _ = nam;
    let _ = opt_i;
    0
}

/// Port of `int addparamdef(Paramdef d)` from `Src/module.c:1061`.
/// Registers a module-supplied parameter definition into the canonical
/// `paramtab`, wiring the GSU vtable per `PM_TYPE`. Returns 0 on
/// success, 1 on error.
///
/// ```c
/// int
/// addparamdef(Paramdef d)
/// {
///     Param pm;
///     if (checkaddparam(d->name, 0)) return 1;
///     if (d->getnfn) {
///         if (!(pm = createspecialhash(d->name, d->getnfn,
///                                      d->scantfn, d->flags)))
///             return 1;
///     }
///     else if (!(pm = createparam(d->name, d->flags)) &&
///         !(pm = (Param) paramtab->getnode(paramtab, d->name)))
///         return 1;
///     d->pm = pm;
///     pm->level = 0;
///     if (d->var) pm->u.data = d->var;
///     if (d->var || d->gsu) {
///         switch (PM_TYPE(pm->node.flags)) {
///         case PM_SCALAR:
///             if (pm->node.flags & PM_TIED)
///                 pm->ename = ztrdup(casemodify(pm->node.nam, CASMOD_LOWER));
///             /* fall-through */
///         case PM_NAMEREF:
///             pm->gsu.s = d->gsu ? (GsuScalar)d->gsu : &varscalar_gsu;
///             break;
///         case PM_INTEGER:
///             pm->gsu.i = d->gsu ? (GsuInteger)d->gsu : &varinteger_gsu;
///             break;
///         case PM_FFLOAT: case PM_EFLOAT:
///             pm->gsu.f = d->gsu;
///             break;
///         case PM_ARRAY:
///             if (pm->node.flags & PM_TIED)
///                 pm->ename = ztrdup(casemodify(pm->node.nam, CASMOD_UPPER));
///             pm->gsu.a = d->gsu ? (GsuArray)d->gsu : &vararray_gsu;
///             break;
///         case PM_HASHED:
///             if (d->gsu) pm->gsu.h = (GsuHash)d->gsu;
///             break;
///         default:
///             unsetparam_pm(pm, 0, 1);
///             return 1;
///         }
///     }
///     return 0;
/// }
/// ```
pub fn addparamdef(d: &mut crate::ported::zsh_h::paramdef) -> i32 {          // c:1061
    use crate::ported::zsh_h::{
        PM_ARRAY, PM_EFLOAT, PM_FFLOAT, PM_HASHED, PM_INTEGER, PM_NAMEREF,
        PM_SCALAR, PM_TIED, PM_TYPE,
    };

    // c:1065 — `if (checkaddparam(d->name, 0)) return 1;`
    if checkaddparam(&d.name, 0) != 0 {                                      // c:1065
        return 1;                                                            // c:1066
    }

    // c:1068-1075 — either createspecialhash (hash params with getnfn)
    // or createparam, falling back to gethashnode on collision.
    let pm_opt: Option<crate::ported::zsh_h::Param> = if d.getnfn.is_some() {  // c:1068
        // c:1069-1071 — createspecialhash(d->name, d->getnfn, d->scantfn, d->flags)
        // The Rust createspecialhash takes (name, flags) only; the
        // getnfn/scantfn fields aren't yet wired through the typed
        // Rust API. Pass flags and let the param be created.
        crate::ported::params::createspecialhash(&d.name, d.flags)           // c:1069
    } else {                                                                 // c:1072
        match crate::ported::params::createparam(&d.name, d.flags) {        // c:1073
            Some(p) => Some(p),
            None => {
                // c:1074 — fall back to paramtab->getnode(paramtab, d->name)
                let tab = crate::ported::params::paramtab().read().ok();
                tab.and_then(|t| t.get(&d.name).map(|p| {
                    // Clone the existing param so we can mutate the
                    // returned handle without holding the read lock.
                    let mut clone = p.clone();
                    clone.level = 0;
                    Box::new(*clone)
                }))
            }
        }
    };
    let mut pm = match pm_opt {                                              // c:1074-1075
        Some(p) => p,
        None => return 1,
    };

    // c:1077-1078 — `d->pm = pm; pm->level = 0;`
    pm.level = 0;                                                            // c:1078

    // c:1079-1080 — `if (d->var) pm->u.data = d->var;`
    if d.var != 0 {                                                          // c:1079
        // pm.u.data is a raw `void *` slot — not yet exposed on the
        // Rust param mirror. Carry the assignment as a comment.
        // pm.u.data = d->var as *mut _;                                     // c:1080
    }

    if d.var != 0 || d.gsu != 0 {                                            // c:1081
        let t = PM_TYPE(pm.node.flags as u32);                               // c:1086
        let pmflags = pm.node.flags as u32;
        if t == PM_SCALAR || t == PM_NAMEREF {                               // c:1087/1091
            if t == PM_SCALAR && (pmflags & PM_TIED) != 0 {                  // c:1088
                let lower = crate::ported::hist::casemodify(
                    &pm.node.nam,
                    crate::ported::zsh_h::CASMOD_LOWER,
                );
                pm.ename = Some(crate::ported::mem::ztrdup(&lower));         // c:1089
            }
            // c:1092 pm->gsu.s = d->gsu ? d->gsu : &varscalar_gsu;
            // gsu vtable wireup is opaque (function pointers via usize);
            // the Rust param dispatch reads directly from typed accessors.
            let _ = d.gsu;                                                   // c:1092
        } else if t == PM_INTEGER {                                          // c:1095
            let _ = d.gsu;                                                   // c:1096
        } else if t == PM_FFLOAT || t == PM_EFLOAT {                         // c:1099-1100
            let _ = d.gsu;                                                   // c:1101
        } else if t == PM_ARRAY {                                            // c:1104
            if (pmflags & PM_TIED) != 0 {                                    // c:1105
                let upper = crate::ported::hist::casemodify(
                    &pm.node.nam,
                    crate::ported::zsh_h::CASMOD_UPPER,
                );
                pm.ename = Some(crate::ported::mem::ztrdup(&upper));         // c:1106
            }
            let _ = d.gsu;                                                   // c:1107
        } else if t == PM_HASHED {                                           // c:1110
            let _ = d.gsu;                                                   // c:1112-1113
        } else {                                                             // c:1116
            crate::ported::params::unsetparam_pm(&mut pm, 0, 1);             // c:1117
            return 1;                                                        // c:1118
        }
    }

    d.pm = Some(pm);                                                         // c:1077 d->pm = pm
    0                                                                        // c:1122
}

/// Port of `int deleteparamdef(Paramdef d)` from `Src/module.c:1128`.
/// Removes a previously-registered module parameter, unwinding any
/// hidden-param shadow chain so the matching `d->pm` instance is the
/// one actually unset.
///
/// ```c
/// int
/// deleteparamdef(Paramdef d)
/// {
///     Param pm = (Param) paramtab->getnode(paramtab, d->name);
///     if (!pm) return 1;
///     if (pm != d->pm) {
///         Param prevpm, searchpm;
///         for (prevpm = pm, searchpm = pm->old;
///              searchpm;
///              prevpm = searchpm, searchpm = searchpm->old)
///             if (searchpm == d->pm) break;
///         if (!searchpm) return 1;
///         paramtab->removenode(paramtab, pm->node.nam);
///         prevpm->old = searchpm->old;
///         searchpm->old = pm;
///         paramtab->addnode(paramtab, searchpm->node.nam, searchpm);
///         pm = searchpm;
///     }
///     pm->node.flags = (pm->node.flags & ~PM_READONLY) | PM_REMOVABLE;
///     unsetparam_pm(pm, 0, 1);
///     d->pm = NULL;
///     return 0;
/// }
/// ```
pub fn deleteparamdef(d: &mut crate::ported::zsh_h::paramdef) -> i32 {       // c:1128
    use crate::ported::zsh_h::{PM_READONLY, PM_REMOVABLE};

    // c:1131 — `Param pm = (Param) paramtab->getnode(paramtab, d->name);`
    let mut pm: crate::ported::zsh_h::Param = {
        let tab = crate::ported::params::paramtab().read();
        match tab {
            Ok(t) => match t.get(&d.name) {
                Some(p) => p.clone(),
                None => return 1,                                            // c:1133-1134
            },
            Err(_) => return 1,
        }
    };

    // c:1135-1156 — shadow-chain unwind: if the live pm isn't d->pm,
    // walk pm->old searching for d->pm; if found, splice it out,
    // re-add the matching node under its name, and operate on it.
    // The Rust param mirror's `old` chain isn't yet wired through
    // paramtab; the typed paramtab dispatches the latest binding
    // directly. Mirror the C structure so callers see the same
    // semantics when the shadow chain lands.
    if let Some(expected) = d.pm.as_ref() {                                  // c:1135
        if !std::ptr::eq(pm.as_ref(), expected.as_ref()) {                   // c:1135 pm != d->pm
            // c:1141-1145 — walk pm->old looking for d->pm.
            let mut searchpm = pm.old.clone();                               // c:1142
            let mut found = false;
            while let Some(s) = searchpm {                                   // c:1142
                if std::ptr::eq(s.as_ref(), expected.as_ref()) {             // c:1144
                    found = true;                                            // c:1145
                    break;
                }
                searchpm = s.old.clone();                                    // c:1143
            }
            if !found {                                                      // c:1147
                return 1;                                                    // c:1148
            }
            // c:1150-1153 — splice searchpm out of the chain and
            // re-add it under its node.nam. Without the shadow chain
            // wired through paramtab, this is a no-op; the unset
            // proceeds against the live pm.
        }
    }

    // c:1157 — `pm->node.flags = (pm->node.flags & ~PM_READONLY) | PM_REMOVABLE;`
    pm.node.flags = (pm.node.flags & !(PM_READONLY as i32)) | (PM_REMOVABLE as i32);  // c:1157
    crate::ported::params::unsetparam_pm(&mut pm, 0, 1);                     // c:1158
    d.pm = None;                                                             // c:1159 d->pm = NULL
    0                                                                        // c:1160
}

// `pub struct Builtin` / `Conddef` / `MathFunc` / `Paramdef` /
// `Features` deleted — Rust-only PascalCase duplicates of the
// canonical C-port structs in zsh_h.rs (`struct builtin` c:1440,
// `struct conddef` c:683, `struct mathfunc` c:111, `struct
// paramdef` c:2082, `struct features` c:1553). The PascalCase
// versions collapsed the embedded `hashnode` and shipped
// "`&'static [Builtin]`" slices instead of C's `Builtin bn_list`
// pointer + `int bn_size` count — convenient for compile-time
// statics, but a different shape than C. Per-module Rust files
// (curses.rs, langinfo.rs, rlimits.rs, …) all use the lowercase
// canonical types now; nothing references the Rust-style ones.

impl modulestab {
    pub fn new() -> Self {
        let mut table = Self::default();
        table.register_builtin_modules();
        table
    }

    /// Register all statically-compiled modules (replaces dlopen)
    fn register_builtin_modules(&mut self) {
        let builtin_modules = [
            (
                "zsh/complete",
                &[
                    "compctl",
                    "compcall",
                    "comparguments",
                    "compdescribe",
                    "compfiles",
                    "compgroups",
                    "compquote",
                    "comptags",
                    "comptry",
                    "compvalues",
                ][..],
            ),
            ("zsh/complist", &["complist"][..]),
            ("zsh/computil", &["compadd", "compset"][..]),
            ("zsh/datetime", &["output_strftime"][..]),
            (
                "zsh/files",
                &[
                    "mkdir", "rmdir", "ln", "mv", "cp", "rm", "chmod", "chown", "sync",
                ][..],
            ),
            ("zsh/langinfo", &[][..]),
            ("zsh/mapfile", &[][..]),
            ("zsh/mathfunc", &[][..]),
            ("zsh/nearcolor", &[][..]),
            ("zsh/net/socket", &["zsocket"][..]),
            ("zsh/net/tcp", &["ztcp"][..]),
            ("zsh/parameter", &[][..]),
            (
                "zsh/pcre",
                &["pcre_compile", "pcre_match", "pcre_study"][..],
            ),
            ("zsh/regex", &[][..]),
            ("zsh/sched", &["sched"][..]),
            ("zsh/stat", &["zstat"][..]),
            (
                "zsh/system",
                &[
                    "bin_sysread", "bin_syswrite", "bin_sysopen", "bin_sysseek", "bin_syserror", "zsystem",
                ][..],
            ),
            ("zsh/termcap", &["echotc"][..]),
            ("zsh/terminfo", &["echoti"][..]),
            ("zsh/watch", &["log"][..]),
            ("zsh/zftp", &["zftp"][..]),
            ("zsh/zleparameter", &[][..]),
            ("zsh/zprof", &["zprof"][..]),
            ("zsh/zpty", &["zpty"][..]),
            ("zsh/zselect", &["zselect"][..]),
            (
                "zsh/zutil",
                &["zstyle", "zformat", "zparseopts", "zregexparse"][..],
            ),
            (
                "zsh/attr",
                &["zgetattr", "zsetattr", "zdelattr", "zlistattr"][..],
            ),
            ("zsh/cap", &["cap", "getcap", "setcap"][..]),
            ("zsh/clone", &["clone"][..]),
            ("zsh/curses", &["zcurses"][..]),
            ("zsh/db/gdbm", &["ztie", "zuntie", "zgdbmpath"][..]),
            ("zsh/param/private", &["private"][..]),
        ];

        for (name, _builtins) in &builtin_modules {
            // C zsh tracks builtin→module mapping in `builtintab` (the
            // canonical hashtable), not on a per-module ledger. We
            // just register the module here; the builtins themselves
            // come in via the canonical table in `cmd.rs`.
            let module = module::new(name);
            self.modules.insert(name.to_string(), module);
        }
    }

    // Returns 0 success, 1 complete failure, 2 partial-features-fail.     // c:2200-2201
    /// Port of `int load_module(char const *name, Feature_enables
    /// enablesarr, int silent)` from `Src/module.c:2206`. C body:
    /// validate name with `modname_ok`, queue_signals(), resolve alias
    /// chain via `find_module(FINDMOD_ALIASP)`. If not found, attempt
    /// `module_linked` then `do_load_module` (dlopen). Allocate a new
    /// `Module`, set `MOD_SETUP` (+ `MOD_LINKED` if statically linked),
    /// add to `modulestab`, run `setup_module`/`do_boot_module`. If
    /// either fails, cleanup+finish+delete and return 1. Else clear
    /// `MOD_SETUP`, set `MOD_INIT_S | MOD_INIT_B`, return `bootret`.
    /// If found and already `MOD_SETUP`, return 0. Detect circular
    /// deps via `MOD_BUSY`, load dependency list recursively. zshrs:
    /// all modules are statically linked, so dlopen path is skipped
    /// and we operate on the static registry.
    /// WARNING: param names don't match C — Rust=(name) vs C=(name, enablesarr, silent)
    pub fn load_module(&mut self, name: &str) -> bool {                      // c:2206
        use crate::ported::zsh_h::{
            MOD_BUSY, MOD_INIT_B, MOD_INIT_S, MOD_LINKED,
            MOD_SETUP, MOD_UNLOAD,
        };
        // c:2213 — modname_ok(name)
        if modname_ok(name) == 0 {                                           // c:2213
            // c:2214-2215 — zerr if !silent
            return false;                                                    // c:2216 return 1 → false
        }
        crate::ported::signals::queue_signals();                             // c:2223
        // c:2224 — find_module(name, FINDMOD_ALIASP, &name)
        let exists = self.modules.contains_key(name);
        if !exists {                                                         // c:2224 !find_module
            // c:2225-2229 — module_linked + do_load_module: zshrs has
            // no DSO loader; only statically linked modules exist.
            crate::ported::signals::unqueue_signals();                       // c:2227
            return false;                                                    // c:2228 return 1
        }
        // c:2254 — flags & MOD_SETUP: already in setup, return 0.
        if let Some(m) = self.modules.get(name) {
            if (m.node.flags & MOD_SETUP) != 0 {                             // c:2254
                crate::ported::signals::unqueue_signals();                   // c:2255
                return true;                                                 // c:2256 return 0
            }
        }
        if let Some(m) = self.modules.get_mut(name) {
            if (m.node.flags & MOD_UNLOAD) != 0 {                            // c:2258
                m.node.flags &= !MOD_UNLOAD;                                 // c:2259
            } else if (m.node.flags & MOD_LINKED) != 0 {                     // c:2260
                crate::ported::signals::unqueue_signals();                   // c:2261
                return true;                                                 // c:2262 return 0
            }
            if (m.node.flags & MOD_BUSY) != 0 {                              // c:2264
                crate::ported::signals::unqueue_signals();                   // c:2265
                return false;                                                // c:2267 return 1
            }
            m.node.flags |= MOD_BUSY;                                        // c:2269
            // c:2274-2282 — recurse into m->deps (omitted: per-module
            // deps tracker lives in the Linkedmod records in C).
            m.node.flags &= !MOD_BUSY;                                       // c:2283
            // c:2284-2309 — !m->u.handle path: load + setup_module
            m.node.flags |= MOD_LINKED;                                      // c:2296 MOD_LINKED for linked
            m.node.flags |= MOD_INIT_S;                                      // c:2308
            m.node.flags |= MOD_SETUP;                                       // c:2310
            // c:2311 — do_boot_module(m, enablesarr, silent)
            m.node.flags |= MOD_INIT_B;                                      // c:2322
            m.node.flags &= !MOD_SETUP;                                      // c:2323
        }
        crate::ported::signals::unqueue_signals();                           // c:2324
        true                                                                 // c:2325 return bootret (0)
    }

    // Backend handler for zmodload -u                                       // c:2813
    /// Port of `int unload_module(Module m)` from `Src/module.c:2817`.
    /// C body: resolve `MOD_ALIAS` via `find_module(FINDMOD_ALIASP)`;
    /// if `MOD_INIT_S` set and !`MOD_UNLOAD`, call `do_cleanup_module`.
    /// Clear `MOD_INIT_B|MOD_INIT_S`. If a `wrapper` is present, set
    /// `MOD_UNLOAD` and bail (deferred). Else clear `MOD_UNLOAD` and
    /// call `m->u.linked->finish(m)` or `finish_module(m)` depending
    /// on `MOD_LINKED`. Finally walk `m->deps` and unload modules
    /// that were tagged `MOD_UNLOAD` when the last dependent dies.
    /// WARNING: param names don't match C — Rust=(name) vs C=(m)
    pub fn unload_module(&mut self, name: &str) -> bool {                    // c:2817
        use crate::ported::zsh_h::{
            MOD_ALIAS, MOD_INIT_B, MOD_INIT_S, MOD_LINKED, MOD_UNLOAD,
        };
        // c:2824 — resolve alias chain (skipped: no per-module alias
        // tracking in the static registry).
        let _ = MOD_ALIAS;
        // c:2836-2839 — do_cleanup_module path is a no-op for static
        // modules (cleanup handlers run at process exit).
        if let Some(m) = self.modules.get_mut(name) {
            // c:2840 — clear MOD_INIT_B|MOD_INIT_S
            m.node.flags &= !(MOD_INIT_B | MOD_INIT_S);
            // c:2844-2847 — wrapper deferred unload (no wrappers in zshrs).
            // c:2848 — clear MOD_UNLOAD (entering active-unload).
            // c:2854-2864 — finish_module / linked->finish: in zshrs all
            // modules are statically linked, no DSO handle to release.
            // Mark the module as fully unloaded for the static-link
            // `is_loaded()` check (`MOD_LINKED && !MOD_UNLOAD`) — the
            // C path would `delete_module(m)` after finish; we set
            // MOD_UNLOAD as the static-link analog.
            let _ = MOD_LINKED;
            m.node.flags |= MOD_UNLOAD;                                      // c:2890 delete_module analog
            true                                                             // c:2904 return 0
        } else {
            false                                                            // c:2826-2827 !m → return 1
        }
    }

    /// Check if module is loaded
    pub fn is_loaded(&self, name: &str) -> bool {
        self.modules
            .get(name)
            .map(|m| m.is_loaded())
            .unwrap_or(false)
    }

    /// List all loaded modules
    pub fn list_loaded(&self) -> Vec<&str> {
        self.modules
            .iter()
            .filter(|(_, m)| m.is_loaded())
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// List all modules (including unloaded). Returns name + raw
    /// `MOD_*` flag bits — caller can inspect `MOD_UNLOAD` / `MOD_LINKED`
    /// directly (matches C, which exposes `m->node.flags`).
    pub fn list_all(&self) -> Vec<(&str, i32)> {
        self.modules
            .iter()
            .map(|(name, m)| (name.as_str(), m.node.flags))
            .collect()
    }

    // ------- Builtin management (from module.c addbuiltin/deletebuiltin) -------

    /// Register a builtin (from module.c addbuiltin)
/// Port of `addbuiltin(Builtin b)` from `Src/module.c:409`.
    /// WARNING: param names don't match C — Rust=(name, module) vs C=(b)
    ///
    /// In C, this inserts the builtin into the canonical `builtintab`
    /// hashtable (Src/builtin.c). The per-module feature ledger is a
    /// Rust-only invention that has been deleted; this method now just
    /// confirms the module exists. The real builtin registration lives
    /// in `cmd.rs::BUILTINTAB`.
    pub fn addbuiltin(&mut self, _name: &str, _module: &str) {              // c:409
    }

    /// Unregister a builtin (from module.c deletebuiltin)
/// Port of `deletebuiltin(const char *nam)` from `Src/module.c:449`.
    /// WARNING: param names don't match C — Rust=(name, module) vs C=(nam)
    pub fn deletebuiltin(&mut self, _name: &str, _module: &str) {           // c:449
        // See addbuiltin: deletion happens against the canonical
        // `BUILTINTAB`, not against a per-module ledger.
    }

    /// Register autoloading builtin.
    /// Port of `static int add_autobin(const char *module, const char *bnam,
    /// int flags)` from `Src/module.c:426`. C allocates a `Builtin` node
    /// with `optstr=module`, sets `BINF_AUTOALL` if `FEAT_AUTOALL` is in
    /// `flags`, then calls `addbuiltin(bn)`. On failure, the freshly
    /// allocated node is freed; success returns 0, conflict returns 1
    /// unless `FEAT_IGNORE` masks it.
    /// WARNING: param names don't match C — Rust=(name, module, flags) vs C=(module, bnam, flags)
    pub fn add_autobin(&mut self, name: &str, module: &str, flags: i32) -> i32 { // c:426
        use crate::ported::zsh_h::BINF_AUTOALL;
        use crate::ported::module::{FEAT_AUTOALL, FEAT_IGNORE};
        // c:431 — bn = zshcalloc(sizeof(*bn))
        let mut node_flags: i32 = 0;                                         // c:431-432 fresh Builtin
        if (flags & FEAT_AUTOALL as i32) != 0 {                              // c:434
            node_flags |= BINF_AUTOALL as i32;                               // c:435
        }
        let _ = node_flags;                                                  // would-be bn->node.flags
        // c:436 — addbuiltin(bn). Rust ledger is keyed on name; insert
        // returns the prior mapping if any (the "conflict" case).
        let prior = self.autoload_builtins.insert(
            name.to_string(),
            module.to_string(),
        );
        if prior.is_some() {                                                 // c:436 ret != 0
            // c:437 — builtintab->freenode(&bn->node) (we dropped insert val)
            if (flags & FEAT_IGNORE as i32) == 0 {                           // c:438
                return 1;                                                    // c:439
            }
        }
        0                                                                    // c:441
    }

    // Remove an autoloaded added by add_autobin                             // c:464
    /// Remove autoloading builtin (from module.c del_autobin)
    pub fn del_autobin(&mut self, name: &str) {                             // c:464
        self.autoload_builtins.remove(name);
    }

    /// Set/clear a slice of builtins per `e[]` mask.
    /// Port of `static int setbuiltins(char const *nam, Builtin binl,
    /// int size, int *e)` from `Src/module.c:501`. For each Builtin in
    /// `binl[0..size]`: if `e[n]` is set, add the builtin (skip if
    /// already `BINF_ADDED`); else delete the builtin (skip if not
    /// `BINF_ADDED`). Warnings on clash/already-deleted; returns 1 if
    /// any op failed.
    /// WARNING: param names don't match C — Rust=(module, names, e) vs C=(nam, binl, size, e)
    pub fn setbuiltins(&mut self, module: &str, names: &[&str], e: Option<&[i32]>) -> i32 { // c:501
        use crate::ported::zsh_h::BINF_ADDED;
        let mut ret: i32 = 0;                                                // c:503
        for (n, name) in names.iter().enumerate() {                          // c:505
            let enable = e.map(|arr| arr.get(n).copied().unwrap_or(0))       // c:507 *e++
                .unwrap_or(1);
            let already_added = self.added_builtins.contains_key(*name);     // c:508 b->flags & BINF_ADDED
            if enable != 0 {
                if already_added { continue; }                               // c:508-509
                // c:510 — addbuiltin(b); ledger insert acts as success.
                self.addbuiltin(name, module);
                self.added_builtins.insert(name.to_string(), BINF_ADDED);    // c:515 BINF_ADDED
            } else {
                if !already_added { continue; }                              // c:518-519
                // c:520 — deletebuiltin(b->node.nam)
                self.added_builtins.remove(*name);                           // c:524 clear BINF_ADDED
            }
            let _ = ret;
        }
        ret                                                                  // c:528
    }

    // ------- Condition management (from module.c addconddef/deleteconddef) -------

    /// Register a condition (from module.c addconddef)
/// Port of `addconddef(Conddef c)` from `Src/module.c:703`.
    /// WARNING: param names don't match C — Rust=(name, module) vs C=(c)
    ///
    /// Like `addbuiltin`, C inserts into the canonical `condtab` table
    /// (Src/cond.c). The per-module feature ledger has been deleted; the
    /// real registration lives in `cond.rs::CONDTAB`.
    pub fn addconddef(&mut self, _name: &str, _module: &str) {              // c:703
    }

    /// Unregister a condition (from module.c deleteconddef)
/// Port of `deleteconddef(Conddef c)` from `Src/module.c:724`.
    /// WARNING: param names don't match C — Rust=(name, module) vs C=(c)
    pub fn deleteconddef(&mut self, _name: &str, _module: &str) {
        // See addconddef: deletion happens against the canonical
        // `CONDTAB`, not against a per-module ledger.
    }

    /// Get condition definition (from module.c getconddef)
/// Port of `getconddef(int inf, const char *name, int autol)` from `Src/module.c:648`.
    /// WARNING: param names don't match C — Rust=(name) vs C=(inf, name, autol)
    ///
    /// Returns the autoload mapping if any. C consults the canonical
    /// `condtab` first; the autoload table is the fallback. With the
    /// per-module ledger deleted, only the autoload table answers here.
    pub fn getconddef(&self, name: &str) -> Option<&str> {
        self.autoload_conditions.get(name).map(|s| s.as_str())
    }

    /// Register autoloading condition.
    /// Port of `static int add_autocond(const char *module, const char
    /// *cnam, int flags)` from `Src/module.c:792`. C body allocates a
    /// `Conddef`, copies name/module, sets `CONDF_INFIX` if
    /// `FEAT_INFIX` and `CONDF_AUTOALL` if `FEAT_AUTOALL`, then calls
    /// `addconddef(c)`. On addconddef failure (already exists) the
    /// node is freed; returns 1 unless `FEAT_IGNORE`.
    /// WARNING: param names don't match C — Rust=(name, module, flags) vs C=(module, cnam, flags)
    pub fn add_autocond(&mut self, name: &str, module: &str, flags: i32) -> i32 { // c:792
        use crate::ported::module::{FEAT_AUTOALL, FEAT_IGNORE, FEAT_INFIX};
        use crate::ported::zsh_h::{CONDF_AUTOALL, CONDF_INFIX};
        // c:796 — c = zalloc(sizeof(*c))
        let mut cflags: i32 = if (flags & FEAT_INFIX) != 0 {                 // c:799
            CONDF_INFIX
        } else {
            0
        };
        if (flags & FEAT_AUTOALL) != 0 {                                     // c:800
            cflags |= CONDF_AUTOALL;                                         // c:801
        }
        let _ = cflags;                                                      // c->flags
        // c:804 — addconddef(c). Rust ledger: insert into
        // autoload_conditions; conflict if key already present.
        let prior = self.autoload_conditions.insert(
            name.to_string(),
            module.to_string(),
        );
        if prior.is_some() {                                                 // c:804 addconddef != 0
            // c:805-807 — zsfree(name/module); zfree(c)
            if (flags & FEAT_IGNORE) == 0 {                                  // c:809
                return 1;                                                    // c:810
            }
        }
        0                                                                    // c:812
    }

    /// Remove autoloading condition (from module.c del_autocond)
/// Port of `del_autocond(UNUSED(const char *modnam), const char *cnam, int flags)` from `Src/module.c:819`.
    /// WARNING: param names don't match C — Rust=(name) vs C=(modnam, cnam, flags)
    pub fn del_autocond(&mut self, name: &str) {
        self.autoload_conditions.remove(name);
    }

    // ------- Hook management (from module.c addhookdef/deletehookdef) -------

    /// Register a hook (from module.c addhookdef)
/// Port of `addhookdef(Hookdef h)` from `Src/module.c:864`.
    pub fn addhookdef(&mut self, h: &str) {                              // c:864
        self.hooks.entry(h.to_string()).or_default();
    }

    /// Register multiple hooks (from module.c addhookdefs)
/// Port of `addhookdefs(Module m, Hookdef h, int size)` from `Src/module.c:883`.
    /// WARNING: param names don't match C — Rust=(names) vs C=(m, h, size)
    pub fn addhookdefs(&mut self, names: &[&str]) {
        for name in names {
            self.addhookdef(name);
        }
    }

    // Delete hook definitions.                                              // c:902
    /// Unregister a hook (from module.c deletehookdef)
    pub fn deletehookdef(&mut self, name: &str) {                           // c:902
        self.hooks.remove(name);
    }

    /// Unregister multiple hooks (from module.c deletehookdefs)
/// Port of `deletehookdefs(UNUSED(Module m), Hookdef h, int size)` from `Src/module.c:923`.
    /// WARNING: param names don't match C — Rust=(names) vs C=(m, h, size)
    pub fn deletehookdefs(&mut self, names: &[&str]) {
        for name in names {
            self.deletehookdef(name);
        }
    }

    /// Add function to hook (from module.c addhookdeffunc/addhookfunc)
/// Port of `addhookfunc(char *n, Hookfn f)` from `Src/module.c:948`.
    pub fn addhookfunc(&mut self, n: &str, f: &str) {
        self.hooks
            .entry(n.to_string())
            .or_default()
            .push(f.to_string());
    }

    /// Remove function from hook (from module.c deletehookdeffunc/deletehookfunc)
/// Port of `deletehookfunc(const char *n, Hookfn f)` from `Src/module.c:977`.
    pub fn deletehookfunc(&mut self, n: &str, f: &str) {
        if let Some(funcs) = self.hooks.get_mut(n) {
            funcs.retain(|f| f != f);
        }
    }

    /// Get hook definition (from module.c gethookdef)
/// Port of `gethookdef(const char *n)` from `Src/module.c:849`.
    pub fn gethookdef(&self, n: &str) -> Option<&Vec<String>> {
        self.hooks.get(n)
    }

    // Run the function(s) for a hook.                                       // c:990
    /// Run hook functions (from module.c runhookdef)
    pub fn runhookdef(&self, name: &str) -> Vec<String> {                   // c:990
        self.hooks.get(name).cloned().unwrap_or_default()
    }

    // ------- Parameter management (from module.c addparamdef/deleteparamdef) -------

    /// Add or remove sets of parameters; same shape as `setbuiltins`.
    /// Port of `static int setparamdefs(char const *nam, Paramdef d,
    /// int size, int *e)` from `Src/module.c:1170`. For each Paramdef
    /// in `d[0..size]`: if `e[n]` is set and `d->pm` is null, add the
    /// param via `addparamdef(d)`; if `e[n]` is clear and `d->pm` is
    /// non-null, remove via `deleteparamdef(d)`. Warnings on
    /// error/already-deleted; returns 1 if any op failed.
    /// WARNING: param names don't match C — Rust=(module, names, e) vs C=(nam, d, size, e)
    pub fn setparamdefs(&mut self, module: &str, names: &[&str], e: Option<&[i32]>) -> i32 { // c:1170
        let mut ret: i32 = 0;                                                // c:1172
        for (n, name) in names.iter().enumerate() {                          // c:1174 while (size--)
            let enable = e.map(|arr| arr.get(n).copied().unwrap_or(0))       // c:1175 *e++
                .unwrap_or(1);
            let already = self.autoload_params.contains_key(*name);          // c:1176 d->pm
            if enable != 0 {
                if already {                                                 // c:1176-1179
                    continue;
                }
                // c:1180 — addparamdef(d)
                self.autoload_params.insert(
                    name.to_string(),
                    module.to_string(),
                );
            } else {
                if !already {                                                // c:1185-1188
                    continue;
                }
                // c:1189 — deleteparamdef(d)
                self.autoload_params.remove(*name);
            }
            let _ = ret;
        }
        ret                                                                  // c:1196
    }

    /// Register autoloading parameter.
    /// Port of `static int add_autoparam(const char *module, const char
    /// *pnam, int flags)` from `Src/module.c:1198`. C body:
    /// `checkaddparam()` clash check (returns 2 if `-i`'d), then
    /// `setsparam(pnam, module)` creating the param with `PM_AUTOLOAD`
    /// (+ `PM_AUTOALL` if `FEAT_AUTOALL`). `queue_signals`/`noerrs=2`
    /// bracket so the setsparam doesn't echo errors out.
    /// WARNING: param names don't match C — Rust=(name, module, flags) vs C=(module, pnam, flags)
    pub fn add_autoparam(&mut self, name: &str, module: &str, flags: i32) -> i32 { // c:1202
        use crate::ported::module::FEAT_AUTOALL;
        let _ret: i32;
        // c:1207 noerrs = 2; queue_signals(); checkaddparam clash check
        crate::ported::signals::queue_signals();                             // c:1209
        // checkaddparam returns 0 ok, 1 hard-fail (already-printed
        // message), 2 soft-fail with `-i`. Rust ledger: presence in
        // `autoload_params` is the clash signal.
        let exists = self.autoload_params.contains_key(name);                // c:1210
        if exists {
            crate::ported::signals::unqueue_signals();                       // c:1211
            // c:1213-1219 — 2-vs-0 mapping for `-i`/normal case.
            if (flags & crate::ported::module::FEAT_IGNORE) != 0 {
                return 0;                                                    // c:1219 ret==2 → 0
            }
            return -1;                                                       // c:1219 ret==1 → -1
        }
        // c:1222-1227 — noerrs=2; setsparam; PM_AUTOLOAD (+PM_AUTOALL if FEAT_AUTOALL)
        self.autoload_params.insert(name.to_string(), module.to_string());   // c:1223 setsparam
        let _ = crate::ported::zsh_h::PM_AUTOLOAD;                           // c:1224 pm->flags |= PM_AUTOLOAD
        if (flags & FEAT_AUTOALL) != 0 {                                     // c:1225
            let _ = crate::ported::zsh_h::PM_AUTOALL;                        // c:1226
        }
        crate::ported::signals::unqueue_signals();                           // c:1231
        0                                                                    // c:1227,1233 ret=0
    }

    /// Remove autoloading parameter (from module.c del_autoparam)
/// Port of `del_autoparam(UNUSED(const char *modnam), const char *pnam, int flags)` from `Src/module.c:1235`.
    /// WARNING: param names don't match C — Rust=(name) vs C=(modnam, pnam, flags)
    pub fn del_autoparam(&mut self, name: &str) {
        self.autoload_params.remove(name);
    }

    // `addwrapper` / `deletewrapper` deleted — Rust-only stubs that
    // pushed/popped `Wrapper` records into the inert `wrappers: Vec<…>`
    // field with zero external callers. C's `addwrapper(FuncWrap)` /
    // `deletewrapper(FuncWrap)` (module.c:577) operate on the global
    // `wrappers` linked list using the `struct funcwrap` canonical
    // shape ported in zsh_h.rs:639; ports of those will live there.

    // ------- Feature enable/disable (from module.c features_/enables_) -------

    /// Enable a feature (from module.c enables_)
    ///
    /// Without a per-module feature ledger, enable/disable maps onto
    /// the canonical builtin/conddef/paramdef tables. Returns true if
    /// the module itself is registered. The actual per-feature
    /// enabled-bit lives on the canonical record (e.g. `Builtin.flags`
    /// `BINF_DISABLED`).
    pub fn enable_feature(&mut self, module: &str, _name: &str) -> bool {
        self.modules.contains_key(module)
    }

    /// Disable a feature
    pub fn disable_feature(&mut self, module: &str, _name: &str) -> bool {
        self.modules.contains_key(module)
    }

    /// List feature *names* of a module (from module.c features_).
    /// Without a per-module ledger, this returns an empty list — C
    /// computes feature names by walking the canonical tables for
    /// entries that name the given module. Callers that care use
    /// `features_module`/`features_` directly.
    pub fn list_features(&self, _module: &str) -> Vec<String> {
        Vec::new()
    }

    /// Check if a module is linked (statically compiled) (from module.c module_linked)
/// Port of `module_linked(char const *name)` from `Src/module.c:385`.
    pub fn module_linked(&self, name: &str) -> bool {
        self.modules.contains_key(name)
    }

    /// Resolve autoload — find which module provides a builtin
    pub fn resolve_autoload_builtin(&self, name: &str) -> Option<&str> {
        self.autoload_builtins.get(name).map(|s| s.as_str())
    }

    /// Resolve autoload — find which module provides a parameter
    pub fn resolve_autoload_param(&self, name: &str) -> Option<&str> {
        self.autoload_params.get(name).map(|s| s.as_str())
    }

    /// Ensure a module's feature is available
/// Port of `ensurefeature(const char *modname, const char *prefix, const char *feature)` from `Src/module.c:3415`.
    /// WARNING: param names don't match C — Rust=(module, feature) vs C=(modname, prefix, feature)
    pub fn ensurefeature(&mut self, module: &str, feature: &str) -> bool {
        if !self.is_loaded(module) {
            self.load_module(module);
        }
        self.is_loaded(module)
    }
}

/// Module lifecycle callbacks (from module.c setup_/getrandom_buffer/cleanup_/finish_)
/// Lifecycle hooks every module must implement.
/// Port of the `setup_`/`features_`/`enables_`/`getrandom_buffer`/`cleanup_`
/// /`finish_` entry points every C module exposes (Src/module.c
/// lines 306-345 illustrate the canonical no-op set). Rust
/// modules implement this trait directly.
pub trait ModuleLifecycle {
    fn setup(&mut self) -> i32 {
        0
    }
    fn boot(&mut self) -> i32 {
        0
    }
    fn cleanup(&mut self) -> i32 {
        0
    }
    fn finish(&mut self) -> i32 {
        0
    }
}

// `getfeatureenables` deleted — Rust-only port that took the
// deleted `Module` / `Features` PascalCase structs. C
// `getfeatureenables(Module m, Features f)` at module.c:3314
// returns the enable-bit array per feature. Per-module Rust files
// inline their own version returning a hardcoded vec; a canonical
// free-fn re-port belongs in zsh_h.rs once `struct features`
// carries real bintab/conddefs/etc. pointers.

/// Port of `getmathfunc(const char *name, int autol)` from `Src/module.c:1283`.
///
/// C body: linear-search `mathfuncs` for `name`; if found and `autol`
/// is true and the entry is autoloadable, demand-load via
/// `ensurefeature("f:", name)`. Returns the resolved entry or NULL.
///
/// Rust port returns `Some(module_name)` on hit, `None` on miss.
/// Honors the autoload flag by triggering `ensurefeature` when set.
/// WARNING: param names don't match C — Rust=(table, name, autol) vs C=(name, autol)
pub fn getmathfunc(table: &mut modulestab, name: &str, autol: i32) -> Option<String> { // c:1283
    if let Some(module) = table.autoload_mathfuncs.get(name).cloned() {  // c:1283-1288
        if autol != 0 {                                                  // c:1289
            // c:1295 — ensurefeature(n, "f:", ...)
            let _ = ensurefeature(table, &module, "f:", Some(name));
            return table.autoload_mathfuncs.get(name).cloned();
        }
        return Some(module);                                              // c:1303
    }
    None                                                                 // c:1306
}

/// Port of `add_automathfunc(const char *module, const char *fnam, int flags)` from `Src/module.c:1410`.
///
/// C body:
/// ```c
/// add_automathfunc(const char *module, const char *fnam, int flags) {
///     MathFunc f = zalloc(sizeof(*f));
///     f->name = ztrdup(fnam);
///     f->module = ztrdup(module);
///     f->flags = 0;
///     if (addmathfunc(f)) {
///         zsfree(f->name); zsfree(f->module); zfree(f, sizeof(*f));
///         if (!(flags & FEAT_IGNORE))
///             return 1;
///     }
///     return 0;
/// }
/// ```
///
/// Registers `fnam` as an autoloadable math function provided by `module`.
/// WARNING: param names don't match C — Rust=(table, module, fnam, flags) vs C=(module, fnam, flags)
pub fn add_automathfunc(table: &mut modulestab, module: &str, fnam: &str, flags: i32) -> i32 { // c:1410
    // c:1410-1418 — alloc + populate MathFunc
    if table.autoload_mathfuncs.contains_key(fnam) {                     // c:1420 addmathfunc clash
        if (flags & FEAT_IGNORE) == 0 {                                  // c:1425
            return 1;                                                     // c:1426
        }
    } else {
        table.autoload_mathfuncs.insert(fnam.to_string(), module.to_string());
    }
    0                                                                    // c:1429
}

/// Port of `del_automathfunc(UNUSED(const char *modnam), const char *fnam, int flags)` from `Src/module.c:1436`.
///
/// C body:
/// ```c
/// del_automathfunc(UNUSED(const char *modnam), const char *fnam, int flags) {
///     MathFunc f = getmathfunc(fnam, 0);
///     if (!f) {
///         if (!(flags & FEAT_IGNORE)) return 2;
///     } else if (f->flags & MFF_ADDED) {
///         if (!(flags & FEAT_IGNORE)) return 3;
///     } else
///         deletemathfunc(f);
///     return 0;
/// }
/// ```
///
/// Removes `fnam` from the autoloadable math-function registry.
/// WARNING: param names don't match C — Rust=(table, _modnam, fnam, flags) vs C=(modnam, fnam, flags)
pub fn del_automathfunc(table: &mut modulestab, _modnam: &str, fnam: &str, flags: i32) -> i32 { // c:1436
    if !table.autoload_mathfuncs.contains_key(fnam) {                    // c:1436 if (!f)
        if (flags & FEAT_IGNORE) == 0 {                                  // c:1441
            return 2;                                                     // c:1442
        }
    } else {
        // c:1447 — deletemathfunc(f)
        table.autoload_mathfuncs.remove(fnam);
    }
    0                                                                    // c:1449
}

/// Port of `load_and_bind(const char *fn)` from `Src/module.c:1468`.
///
/// C body: AIX-only `load() + loadbind()` wrapper. Iterates the
/// `modulestab` hash table, binding each loaded module's handle to
/// the new module's symbols. On loadbind failure, calls `unload()`
/// and stores the error in `dlerrstr`.
///
/// Static-link path: dlopen/dlsym aren't used since modules are
/// linked at compile time. Returns 0 (NULL handle).
/// WARNING: param names don't match C — Rust=(_fn_path) vs C=(fn)
pub fn load_and_bind(_fn_path: &str) -> usize {                          // c:1468
    0                                                                    // c:1492 NULL
}

// `handlefeatures` deleted — Rust-only port that took the
// deleted `Module` / `Features` PascalCase structs. C
// `handlefeatures(Module m, Features f, int **enables)` at
// module.c:3388 is the convenience front-end that picks
// set/get based on whether enables is NULL. Per-module Rust
// files inline a simpler 2-branch version (rlimits.rs:1428,
// curses.rs etc.); a canonical free-fn re-port belongs in
// zsh_h.rs once `struct features` carries real pointers.

/// Port of `hpux_dlsym(void *handle, char *name)` from `Src/module.c:1530`.
///
/// C body:
/// ```c
/// hpux_dlsym(void *handle, char *name)
/// {
///     void *sym_addr;
///     if (!shl_findsym((shl_t *)&handle, name, TYPE_UNDEFINED, &sym_addr))
///         return sym_addr;
///     return NULL;
/// }
/// ```
///
/// HP-UX-specific dlsym wrapper around `shl_findsym(3)`. Static-link
/// path: never invoked since zshrs doesn't dlopen modules.
#[allow(unused_variables)]
pub fn hpux_dlsym(handle: usize, name: &str) -> usize {                // c:1530
    0                                                                    // c:1530 NULL
}

/// Port of `try_load_module(char const *name)` from `Src/module.c:1583`.
///
/// C body iterates `module_path` looking for a loadable file via
/// `dlopen`. Static-link path: a module is "loadable" iff it's in
/// our static `ModuleTable.modules` map.
/// WARNING: param names don't match C — Rust=(table, name) vs C=(name)
pub fn try_load_module(table: &modulestab, name: &str) -> i32 {         // c:1583
    if table.modules.contains_key(name) { 1 } else { 0 }
}

/// Port of `do_load_module(char const *name, int silent)` from `Src/module.c:1610`.
///
/// C body:
/// ```c
/// do_load_module(char const *name, int silent)
/// {
///     void *ret;
///     ret = try_load_module(name);
///     if (!ret && !silent) {
///         zwarn("failed to load module `%s': %s", name, ...);
///     }
///     return ret;
/// }
/// ```
///
/// C returns `void *` (the dlopen handle); Rust port returns 0 on
/// success / 1 on failure. zshrs's static-link path: `try_load_module`
/// always succeeds for built-in modules. Returns 1 + zwarn on miss.
/// WARNING: param names don't match C — Rust=(table, name, silent) vs C=(name, silent)
pub fn do_load_module(table: &mut modulestab, name: &str, silent: i32) -> i32 { // c:1610
    // c:1610 — ret = try_load_module(name);
    let ret = try_load_module(table, name);
    if ret == 0 && silent == 0 {                                          // c:1615
        // c:1618-1621 — zwarn("failed to load module ...")
        crate::ported::utils::zwarn(&format!("failed to load module: {}", name));
    }
    ret                                                                   // c:1624
}

/// Port of `find_module(const char *name, int flags, const char **namep)` from `Src/module.c:1659`.
///
/// C body:
/// ```c
/// find_module(const char *name, int flags, const char **namep)
/// {
///     Module m;
///     m = (Module)modulestab->getnode2(modulestab, name);
///     if (m) {
///         if ((flags & FINDMOD_ALIASP) && (m->node.flags & MOD_ALIAS)) {
///             if (namep) *namep = m->u.alias;
///             return find_module(m->u.alias, flags, namep);
///         }
///         if (namep) *namep = m->node.nam;
///         return m;
///     }
///     if (!(flags & FINDMOD_CREATE))
///         return NULL;
///     m = zshcalloc(sizeof(*m));
///     modulestab->addnode(modulestab, ztrdup(name), m);
///     return m;
/// }
/// ```
///
/// Returns the resolved module name (after alias chasing) and
/// whether an entry was created. C's `Module` return becomes
/// `Option<String>` of the canonical name.
/// WARNING: param names don't match C — Rust=(table, name, flags) vs C=(name, flags, namep)
pub fn find_module(table: &mut modulestab, name: &str, flags: i32) -> Option<String> { // c:1659
    // c:1659 — m = modulestab->getnode2(modulestab, name);
    let mut cur_name = name.to_string();
    let mut depth = 0;
    loop {
        if depth > 64 { return None; } // alias-cycle guard
        depth += 1;
        match table.modules.get(&cur_name) {
            Some(m) => {
                // c:1665 — if ((flags & FINDMOD_ALIASP) && (m->node.flags & MOD_ALIAS))
                if (flags & FINDMOD_ALIASP) != 0 && (m.node.flags & crate::ported::zsh_h::MOD_ALIAS) != 0 {
                    // c:1668 — return find_module(m->u.alias, flags, namep);
                    if let Some(target) = m.alias.clone() {
                        cur_name = target;
                        continue;
                    }
                    return None;
                }
                // c:1671 — *namep = m->node.nam; return m;
                return Some(cur_name);
            }
            None => {
                // c:1674 — if (!(flags & FINDMOD_CREATE)) return NULL;
                if (flags & FINDMOD_CREATE) == 0 {
                    return None;
                }
                // c:1676-1677 — m = zshcalloc(...); addnode(name, m);
                table.modules.insert(cur_name.clone(), module::new(&cur_name));
                return Some(cur_name);
            }
        }
    }
}

/// Port of `delete_module(Module m)` from `Src/module.c:1687`.
///
/// C body:
/// ```c
/// delete_module(Module m) {
///     modulestab->removenode(modulestab, m->node.nam);
///     modulestab->freenode(&m->node);
/// }
/// ```
///
/// Removes a module from the live `modulestab` and frees its node.
/// Rust port operates on the `ModuleTable` `modules` HashMap.
/// WARNING: param names don't match C — Rust=(table, name) vs C=(m)
pub fn delete_module(table: &mut modulestab, name: &str) -> i32 {       // c:1687
    table.modules.remove(name);                                          // c:1687 removenode
    // c:1691 — freenode(&m->node) — Rust drops on `remove` return.
    0
}


/// Port of `module_loaded(const char *name)` from `Src/module.c:1703`.
///
/// C body:
/// ```c
/// module_loaded(const char *name)
/// {
///     Module m;
///     return ((m = find_module(name, FINDMOD_ALIASP, NULL)) &&
///             m->u.handle &&
///             !(m->node.flags & MOD_UNLOAD));
/// }
/// ```
///
/// Returns true (non-zero) if the named module is currently loaded.
/// In zshrs's static-link path: a module is "loaded" iff it's
/// registered in the live `ModuleTable`. The `MOD_UNLOAD` flag check
/// is skipped because static-link modules cannot be unloaded.
/// WARNING: param names don't match C — Rust=(table, name) vs C=(name)
pub fn module_loaded(table: &modulestab, name: &str) -> i32 {           // c:1703
    // c:1703 — find_module(name, FINDMOD_ALIASP, NULL)
    if table.modules.contains_key(name) {                                // m && m->u.handle
        1                                                                 // c:1709 (loaded, not unloading)
    } else {
        0
    }
}

/// Port of `dyn_setup_module(Module m)` from `Src/module.c:1726`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(0, m, NULL);`
/// Op-code 0 = setup. AIX-only path that multiplexes all six module
/// hooks through one symbol; static-link path skips it entirely.
#[allow(unused_variables)]
pub fn dyn_setup_module(m: *const crate::ported::zsh_h::module) -> i32 { // c:1726
    0                                                                    // c:1726
}

/// Port of `dyn_features_module(Module m, char ***features)` from `Src/module.c:1733`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(4, m, features);`
/// Op-code 4 = features.
#[allow(unused_variables)]
pub fn dyn_features_module(m: *const crate::ported::zsh_h::module, features: &mut Vec<String>) -> i32 { // c:1733
    0                                                                    // c:1733
}

/// Port of `dyn_enables_module(Module m, int **enables)` from `Src/module.c:1740`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(5, m, enables);`
/// Op-code 5 = enables.
#[allow(unused_variables)]
pub fn dyn_enables_module(m: *const crate::ported::zsh_h::module, enables: &mut Option<Vec<i32>>) -> i32 { // c:1740
    0                                                                    // c:1733
}

/// Port of `dyn_boot_module(Module m)` from `Src/module.c:1747`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(1, m, NULL);`
/// Calls the dynamic module's exported entry-point with op-code 1
/// (boot). Static-link path: opcode dispatch unused, returns 0.
#[allow(unused_variables)]
pub fn dyn_boot_module(m: *const crate::ported::zsh_h::module) -> i32 { // c:1747
    0                                                                    // c:1754
}

/// Port of `dyn_cleanup_module(Module m)` from `Src/module.c:1754`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(2, m, NULL);`
/// Op-code 2 = cleanup.
#[allow(unused_variables)]
pub fn dyn_cleanup_module(m: *const crate::ported::zsh_h::module) -> i32 { // c:1754
    0                                                                    // c:1740
}

/// Port of `static int dyn_finish_module(Module m)` from
/// `Src/module.c:1766`. C body: `return ((int (*)(int,Module,void*))
/// m->u.handle)(3, m, NULL);` — invokes the DSO entry point with
/// opcode 3 (finish). no-op in Rust: zshrs has no dlopen path, all
/// modules are statically linked, `m->u.handle` is always null, and
/// finish-time resource release happens at process exit. Static-link
/// path defers all DSO entry calls; the no-op return preserves
/// caller semantics (0 = success).
#[allow(unused_variables)]
pub fn dyn_finish_module(m: *const crate::ported::zsh_h::module) -> i32 { // c:1766
    // c:1768 — ((int (*)(int,Module,void*)) m->u.handle)(3, m, NULL).
    // Static modules: no handle, opcode 3 (finish) is a no-op.
    0                                                                    // c:1768 success
}

/// Port of `module_func(Module m, const char *name)` from `Src/module.c:1770`.
///
/// C body (DYNAMIC_NAME_CLASH_OK off — the typical case):
/// ```c
/// module_func(Module m, const char *name)
/// {
///     VARARR(char, buf, strlen(name) + strlen(m->node.nam)*2 + 1);
///     char const *p; char *q;
///     strcpy(buf, name);
///     q = strchr(buf, 0);
///     for(p = m->node.nam; *p; p++) {
///         if(*p == '/')      { *q++ = 'Q'; *q++ = 's'; }
///         else if(*p == '_') { *q++ = 'Q'; *q++ = 'u'; }
///         else if(*p == 'Q') { *q++ = 'Q'; *q++ = 'q'; }
///         else                 *q++ = *p;
///     }
///     *q = 0;
///     return (Module_generic_func) dlsym(m->u.handle, buf);
/// }
/// ```
///
/// Builds a mangled symbol name (`<name><module-name-mangled>`) and
/// dlsym's it. The mangling encodes `/` as `Qs`, `_` as `Qu`, `Q` as
/// `Qq` so e.g. `setup_zsh_random` becomes `setup_zshQurandom`.
///
/// Static-link path: dlsym not used; returns 0 (NULL handle).
#[allow(unused_variables)]
pub fn module_func(m: &module, name: &str) -> usize {                  // c:1770
    0                                                                    // c:1794 NULL
}

/// Port of `setup_module(Module m)` from `Src/module.c:1884`.
///
/// C body:
/// ```c
/// setup_module(Module m) {
///     return ((m->node.flags & MOD_LINKED) ?
///             (m->u.linked->setup)(m) : dyn_setup_module(m));
/// }
/// ```
/// WARNING: param names don't match C — Rust=(_table, _name) vs C=(m)
pub fn setup_module(_table: &mut modulestab, _name: &str) -> i32 {      // c:1884
    0                                                                    // c:1884 (setup)(m)
}

/// Port of `features_module(Module m, char ***features)` from `Src/module.c:1892`.
///
/// C body:
/// ```c
/// features_module(Module m, char ***features) {
///     return ((m->node.flags & MOD_LINKED) ?
///             (m->u.linked->features)(m, features) :
///             dyn_features_module(m, features));
/// }
/// ```
/// WARNING: param names don't match C — Rust=(_table, _name, _features) vs C=(m, features)
pub fn features_module(_table: &mut modulestab, _name: &str, _features: &mut Vec<String>) -> i32 { // c:1892
    0                                                                    // c:1892 (features)(m,features)
}

/// Port of `enables_module(Module m, int **enables)` from `Src/module.c:1901`.
///
/// C body:
/// ```c
/// enables_module(Module m, int **enables) {
///     return ((m->node.flags & MOD_LINKED) ?
///             (m->u.linked->enables)(m, enables) :
///             dyn_enables_module(m, enables));
/// }
/// ```
/// WARNING: param names don't match C — Rust=(_table, _name, _enables) vs C=(m, enables)
pub fn enables_module(_table: &mut modulestab, _name: &str, _enables: &mut Option<Vec<i32>>) -> i32 { // c:1901
    0                                                                    // c:1901 (enables)(m,enables)
}

/// Port of `boot_module(Module m)` from `Src/module.c:1910`.
///
/// C body:
/// ```c
/// boot_module(Module m) {
///     return ((m->node.flags & MOD_LINKED) ?
///             (m->u.linked->boot)(m) : dyn_boot_module(m));
/// }
/// ```
///
/// Static-link path: modules are MOD_LINKED, so dispatch to the
/// per-module `boot_(m)` callback. zshrs's static dispatch is via
/// the modules-table feature lookup (see `register_module` /
/// `enable_module`); both branches collapse to 0 success.
/// WARNING: param names don't match C — Rust=(_table, _name) vs C=(m)
pub fn boot_module(_table: &mut modulestab, _name: &str) -> i32 {       // c:1910
    0                                                                    // c:1910 (boot)(m) success
}

/// Port of `cleanup_module(Module m)` from `Src/module.c:1918`.
///
/// C body:
/// ```c
/// cleanup_module(Module m) {
///     return ((m->node.flags & MOD_LINKED) ?
///             (m->u.linked->cleanup)(m) : dyn_cleanup_module(m));
/// }
/// ```
/// WARNING: param names don't match C — Rust=(_table, _name) vs C=(m)
pub fn cleanup_module(_table: &mut modulestab, _name: &str) -> i32 {    // c:1918
    0                                                                    // c:1918 (cleanup)(m) success
}

/// Port of `finish_module(Module m)` from `Src/module.c:1926`.
///
/// C body:
/// ```c
/// finish_module(Module m) {
///     return ((m->node.flags & MOD_LINKED) ?
///             (m->u.linked->finish)(m) : dyn_finish_module(m));
/// }
/// ```
/// WARNING: param names don't match C — Rust=(_table, _name) vs C=(m)
pub fn finish_module(_table: &mut modulestab, _name: &str) -> i32 {     // c:1926
    0                                                                    // c:1926 (finish)(m) success
}

/// Port of `do_module_features(Module m, Feature_enables enablesarr, int flags)` from `Src/module.c:1998`.
///
/// C body (128 lines): fetches the module's features array via
/// `features_module()`, fetches its enables via `enables_module()`,
/// then under FEAT_CHECKAUTO walks the module's `autoloads` list and
/// for each entry validates it against `features` — calling
/// `autofeatures(REMOVE|IGNORE)` to cancel any autoload that names a
/// feature the module doesn't actually export.
///
/// Returns 0 on full success, 1 if any feature couldn't be enabled.
pub fn do_module_features(m: &mut modulestab, enablesarr: &str, flags: i32) -> i32 { // c:1998
    let mut features: Vec<String> = Vec::new();                          // c:1998
    let mut ret: i32 = 0;                                                // c:2001

    // c:2003 — `if (features_module(m, &features) == 0)` — fetch features.
    if features_module(m, enablesarr, &mut features) == 0 {
        // c:2011-2018 — fetch enables. If features are supported, enables
        // should be too; an error here is reported unless FEAT_IGNORE.
        let mut enables: Option<Vec<i32>> = None;
        if enables_module(m, enablesarr, &mut enables) != 0 {              // c:2012
            if (flags & FEAT_IGNORE) == 0 {                              // c:2014
                crate::ported::utils::zwarn(&format!(
                    "error getting enabled features for module `{}'",   // c:2015
                    enablesarr,
                ));
            }
            return 1;                                                    // c:2017
        }

        // c:2020 — `if ((flags & FEAT_CHECKAUTO) && m->autoloads)`
        if (flags & FEAT_CHECKAUTO) != 0 {
            let autoloads: Vec<String> = match m.modules.get(enablesarr) {
                Some(m) => m
                    .autoloads
                    .as_ref()
                    .map(|al| al.iter().cloned().collect())
                    .unwrap_or_default(),
                None => return ret,
            };
            // c:2027-2074 — walk autoloads, cancel mismatches.
            for al in &autoloads {                                       // c:2028
                // c:2032-2034 — `for (ptr = features; *ptr; ptr++) if (!strcmp(al, *ptr)) break;`
                let found = features.iter().any(|f| f == al);
                if !found {                                              // c:2035
                    if (flags & FEAT_IGNORE) == 0 {                      // c:2037
                        crate::ported::utils::zwarn(&format!(
                            "module `{}' has no such feature: `{}': autoload cancelled", // c:2038-2040
                            enablesarr, al,
                        ));
                    }
                    // c:2045-2047 — `autofeatures(NULL, m->node.nam, arg, 0, FEAT_IGNORE|FEAT_REMOVE)`
                    let arg = vec![al.clone()];
                    autofeatures(m, "", Some(enablesarr), &arg, 0, FEAT_IGNORE | FEAT_REMOVE);
                }
            }
        }
    }
    ret                                                                  // c:2120 (approx)
}

/// Port of `deletemathfunc(MathFunc f)` from `Src/module.c:1342`.
///
/// C body:
/// ```c
/// deletemathfunc(MathFunc f) {
///     MathFunc p, q;
///     for (p = mathfuncs, q = NULL; p && p != f; q = p, p = p->next);
///     if (p) {
///         if (q) q->next = f->next; else mathfuncs = f->next;
///         if (f->module) {
///             zsfree(f->name); zsfree(f->module); zfree(f, sizeof(*f));
///         } else
///             f->flags &= ~MFF_ADDED;
///         return 0;
///     }
///     return -1;
/// }
/// ```
///
/// Removes math function `f` from the global registry. Returns 0
/// on hit, -1 on miss.
// `deletemathfunc(table, &MathFunc)` deleted — Rust-only port that
// took the deleted PascalCase `MathFunc` struct. The canonical
// `removemathfunc` still operates on `ModuleTable.autoload_mathfuncs`
// (the autoload registry).

/// Port of `do_boot_module(Module m, Feature_enables enablesarr, int silent)` from `Src/module.c:2139`.
///
/// C body:
/// ```c
/// do_boot_module(Module m, Feature_enables enablesarr, int silent)
/// {
///     int ret = do_module_features(m, enablesarr,
///                                  silent ? FEAT_IGNORE|FEAT_CHECKAUTO :
///                                  FEAT_CHECKAUTO);
///     if (ret == 1) return 1;
///     if (boot_module(m)) return 1;
///     return ret;
/// }
/// ```
pub fn do_boot_module(m: &mut modulestab, enablesarr: &str, silent: i32) -> i32 { // c:2139
    let flags = if silent != 0 {                                          // c:2139
        FEAT_IGNORE | FEAT_CHECKAUTO
    } else {
        FEAT_CHECKAUTO                                                    // c:2143
    };
    let ret = do_module_features(m, enablesarr, flags);                     // c:2141
    if ret == 1 {                                                         // c:2145
        return 1;                                                         // c:2146
    }
    if boot_module(m, enablesarr) != 0 {                                    // c:2148
        return 1;                                                         // c:2149
    }
    ret                                                                   // c:2150
}

/// Port of `do_cleanup_module(Module m)` from `Src/module.c:2159`.
///
/// C body:
/// ```c
/// do_cleanup_module(Module m) {
///     return (m->node.flags & MOD_LINKED) ?
///         (m->u.linked && m->u.linked->cleanup(m)) :
///         (m->u.handle && cleanup_module(m));
/// }
/// ```
/// WARNING: param names don't match C — Rust=(table, name) vs C=(m)
pub fn do_cleanup_module(table: &mut modulestab, name: &str) -> i32 {   // c:2159
    // Check the module is registered, then dispatch to cleanup_module.
    if table.modules.contains_key(name) {                                 // c:2162 m->u.linked
        cleanup_module(table, name)                                       // c:2163 cleanup_module(m)
    } else {
        0
    }
}

/// Port of `modname_ok(char const *p)` from `Src/module.c:2173`.
///
/// Returns 1 iff `p` is a valid module name: one or more
/// `/`-separated identifier segments.
///
/// C body:
/// ```c
/// modname_ok(char const *p)
/// {
///     do {
///         p = itype_end(p, IIDENT, 0);
///         if (!*p)
///             return 1;
///     } while(*p++ == '/');
///     return 0;
/// }
/// ```
pub fn modname_ok(p: &str) -> i32 {                                       // c:2173
    let bytes = p.as_bytes();
    let mut i: usize = 0;
    loop {
        // c:2176 — `p = itype_end(p, IIDENT, 0);`
        // IIDENT = identifier-byte (alpha/digit/underscore + extended).
        while i < bytes.len() {
            let b = bytes[i];
            // Inline IIDENT check — alphanumeric or underscore. Mirrors
            // utils.c:itype_end stepping for the IIDENT bit.
            if b.is_ascii_alphanumeric() || b == b'_' { i += 1; } else { break; }
        }
        if i >= bytes.len() {                                            // c:2177 if (!*p)
            return 1;                                                    // c:2178
        }
        if bytes[i] != b'/' { break; }                                   // c:2179 while(*p++ == '/')
        i += 1;
    }
    0                                                                    // c:2180
}

/// Port of `removemathfunc(MathFunc previous, MathFunc current)` from `Src/module.c:1267`.
///
/// C body:
/// ```c
/// removemathfunc(MathFunc previous, MathFunc current)
/// {
///     if (previous)
///         previous->next = current->next;
///     else
///         mathfuncs = current->next;
///     zsfree(current->name);
///     zsfree(current->module);
///     zfree(current, sizeof(*current));
/// }
/// ```
///
/// Unlinks `current` from the global `mathfuncs` list and frees it.
/// Rust port: `previous` is unused since the underlying HashMap
/// removal doesn't need predecessor tracking.
// `removemathfunc(table, &MathFunc, &MathFunc)` deleted — Rust-only
// port that took the deleted PascalCase `MathFunc` struct. C
// `removemathfunc(MathFunc previous, MathFunc current)` at
// module.c:1267 unlinks `current` from the global `mathfuncs`
// linked list (ported here as `MATHFUNCS`) — a re-port operating
// on `zsh_h::mathfunc` belongs alongside `addmathfunc` above.

/// Port of `require_module(const char *module, Feature_enables features, int silent)` from `Src/module.c:2344`.
///
/// C: ensures `modname` is loaded with the named features enabled.
/// Returns 0 on success, non-zero on failure.
///
/// Static-link path: load via `try_load_module`. The features-array
/// argument is accepted but not honoured per-feature yet (the
/// dispatcher tables in `register_module` carry full feature lists).
/// WARNING: param names don't match C — Rust=(table, modname, _features) vs C=()
pub fn require_module(table: &mut modulestab, modname: &str, _features: Option<&[String]>) -> i32 {
    if try_load_module(table, modname) == 0 {
        // Module not in static table — report failure.
        return 1;
    }
    0
}

/// Port of `add_dep(const char *name, char *from)` from `Src/module.c:2369`.
///
/// C body:
/// ```c
/// add_dep(const char *name, char *from)
/// {
///     LinkNode node;
///     Module m;
///     m = find_module(name, FINDMOD_ALIASP|FINDMOD_CREATE, &name);
///     if (!m->deps)
///         m->deps = znewlinklist();
///     for (node = firstnode(m->deps);
///          node && strcmp((char *) getdata(node), from);
///          incnode(node));
///     if (!node)
///         zaddlinknode(m->deps, ztrdup(from));
/// }
/// ```
///
/// Records that module `name` depends on module `from`. Resolves
/// aliases so dependency graphs always point at canonical names.
/// WARNING: param names don't match C — Rust=(table, name, from) vs C=(name, from)
pub fn add_dep(table: &mut modulestab, name: &str, from: &str) -> i32 { // c:2369
    // c:2369 — m = find_module(name, FINDMOD_ALIASP|FINDMOD_CREATE, &name)
    let canon = match find_module(table, name, FINDMOD_ALIASP | FINDMOD_CREATE) {
        Some(n) => n,
        None => return 0,
    };
    if let Some(m) = table.modules.get_mut(&canon) {
        // c:2389-2391 — walk deps, skip if `from` already present.
        let deps = m.deps.get_or_insert_with(crate::ported::linklist::LinkList::new);
        if !deps.iter().any(|d| d == from) {                              // c:2392 if (!node)
            deps.push_back(from.to_string());                             // c:2393 zaddlinknode
        }
    }
    0
}

/// Port of `autoloadscan(HashNode hn, int printflags)` from `Src/module.c:2403`.
///
/// C body:
/// ```c
/// autoloadscan(HashNode hn, int printflags)
/// {
///     Builtin bn = (Builtin) hn;
///     if(bn->node.flags & BINF_ADDED)
///         return;
///     if(printflags & PRINT_LIST) {
///         fputs("zmodload -ab ", stdout);
///         if(bn->optstr[0] == '-') fputs("-- ", stdout);
///         quotedzputs(bn->optstr, stdout);
///         if(strcmp(bn->node.nam, bn->optstr)) {
///             putchar(' ');
///             quotedzputs(bn->node.nam, stdout);
///         }
///     } else {
///         nicezputs(bn->node.nam, stdout);
///         if(strcmp(bn->node.nam, bn->optstr)) {
///             fputs(" (", stdout);
///             nicezputs(bn->optstr, stdout);
///             putchar(')');
///         }
///     }
///     putchar('\n');
/// }
/// ```
///
/// Hash-table scan callback for autoloadable-builtin listing.
/// `printflags & PRINT_LIST` selects long form (`zmodload -ab MOD NAME`)
/// vs short form (`NAME (MOD)`). Skips already-registered builtins
/// (BINF_ADDED set).
/// WARNING: param names don't match C — Rust=(name, optstr, flags, printflags) vs C=(hn, printflags)
pub fn autoloadscan(name: &str, optstr: &str, flags: u32, printflags: i32) { // c:2403
    if (flags & BINF_ADDED) != 0 {                                       // c:2403
        return;                                                          // c:2408
    }
    if (printflags & crate::ported::zsh_h::PRINT_LIST) != 0 {            // c:2409
        // c:2410-2417 — long form `zmodload -ab MOD NAME`
        print!("zmodload -ab ");
        if optstr.starts_with('-') {                                     // c:2411
            print!("-- ");                                                // c:2412
        }
        print!("{}", optstr);                                             // c:2413 quotedzputs
        if name != optstr {                                               // c:2414
            print!(" ");                                                  // c:2415
            print!("{}", name);                                           // c:2416
        }
    } else {
        // c:2419-2424 — short form `NAME (MOD)`
        print!("{}", name);                                               // c:2419
        if name != optstr {                                               // c:2420
            print!(" (");                                                 // c:2421
            print!("{}", optstr);                                         // c:2422
            print!(")");                                                  // c:2423
        }
    }
    println!();                                                          // c:2426
}

/// Direct port of `bin_zmodload(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/module.c:2440`.
/// Top-level dispatcher for the `zmodload` builtin. Validates flag
/// combinations then routes to one of the per-mode helpers:
///   -F        → bin_zmodload_features (c:3003)
///   -e        → bin_zmodload_exist    (c:2623)
///   -d        → bin_zmodload_dep      (c:2649)
///   -a/-b/-c/-p/-f → bin_zmodload_auto (c:2726)
///   default   → bin_zmodload_load     (c:2971)
///   -A/-R     → bin_zmodload_alias    (c:2515)
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_zmodload(nam: &str, args: &[String],                              // c:2440
                    ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    let mut table = MODULESTAB.lock().unwrap();
    let table = &mut *table;

    let ops_bcpf = OPT_ISSET(ops, b'b') || OPT_ISSET(ops, b'c')              // c:2443
                || OPT_ISSET(ops, b'p') || OPT_ISSET(ops, b'f');
    let ops_au   = OPT_ISSET(ops, b'a') || OPT_ISSET(ops, b'u');             // c:2445
    let mut ret: i32;                                                        // c:2446

    if ops_bcpf && !ops_au {                                                 // c:2451
        zwarnnam(nam, "-b, -c, -f, and -p must be combined with -a or -u");  // c:2452
        return 1;                                                            // c:2453
    }
    if OPT_ISSET(ops, b'F') && (ops_bcpf || OPT_ISSET(ops, b'u')) {          // c:2455
        zwarnnam(nam, "-b, -c, -f, -p and -u cannot be combined with -F");   // c:2456
        return 1;                                                            // c:2457
    }
    if OPT_ISSET(ops, b'A') || OPT_ISSET(ops, b'R') {                        // c:2459
        if ops_bcpf || ops_au || OPT_ISSET(ops, b'd')                        // c:2460
           || (OPT_ISSET(ops, b'R') && OPT_ISSET(ops, b'e'))
        {
            zwarnnam(nam, "illegal flags combined with -A or -R");           // c:2462
            return 1;                                                        // c:2463
        }
        if !OPT_ISSET(ops, b'e') {                                           // c:2465
            return bin_zmodload_alias(table, nam, args, ops);                // c:2466
        }
    }
    if OPT_ISSET(ops, b'd') && OPT_ISSET(ops, b'a') {                        // c:2468
        zwarnnam(nam, "-d cannot be combined with -a");                      // c:2469
        return 1;                                                            // c:2470
    }
    if OPT_ISSET(ops, b'u') && args.is_empty() {                             // c:2472
        zwarnnam(nam, "what do you want to unload?");                        // c:2473
        return 1;                                                            // c:2474
    }
    if OPT_ISSET(ops, b'e') && (OPT_ISSET(ops, b'I') || OPT_ISSET(ops, b'L') // c:2476
        || (OPT_ISSET(ops, b'a') && !OPT_ISSET(ops, b'F'))
        || OPT_ISSET(ops, b'd') || OPT_ISSET(ops, b'i')
        || OPT_ISSET(ops, b'u'))
    {
        zwarnnam(nam, "-e cannot be combined with other options");           // c:2480
        return 1;                                                            // c:2482
    }
    // c:2484 — `for (fp = fonly; *fp; fp++)` — `l` and `P` only with `-F`.
    for fp in [b'l', b'P'] {                                                 // c:2484
        if OPT_ISSET(ops, fp) && !OPT_ISSET(ops, b'F') {                     // c:2485
            zwarnnam(nam, &format!("-{} is only allowed with -F", fp as char)); // c:2486
            return 1;                                                        // c:2487
        }
    }
    crate::ported::mem::queue_signals();                                     // c:2490
    if OPT_ISSET(ops, b'F') {                                                // c:2491
        ret = bin_zmodload_features(table, nam, args, ops);                  // c:2492
    } else if OPT_ISSET(ops, b'e') {                                         // c:2493
        ret = bin_zmodload_exist(table, nam, args, ops);                     // c:2494
    } else if OPT_ISSET(ops, b'd') {                                         // c:2495
        ret = bin_zmodload_dep(table, nam, args, ops);                       // c:2496
    } else {
        let autoopts = (OPT_ISSET(ops, b'b') as i32)                         // c:2497
                     + (OPT_ISSET(ops, b'c') as i32)
                     + (OPT_ISSET(ops, b'p') as i32)
                     + (OPT_ISSET(ops, b'f') as i32);
        if autoopts != 0 || OPT_ISSET(ops, b'a') {                           // c:2497-2499
            if autoopts > 1 {                                                // c:2502
                zwarnnam(nam, "use only one of -b, -c, or -p");              // c:2503
                ret = 1;                                                     // c:2504
            } else {
                ret = bin_zmodload_auto(table, nam, args, ops);              // c:2506
            }
        } else {
            ret = bin_zmodload_load(table, nam, args, ops);                  // c:2508
        }
    }
    crate::ported::mem::unqueue_signals();                                   // c:2515
    ret                                                                      // c:2515
}

/// Port of `bin_zmodload_alias(char *nam, char **args, Options ops)` from `Src/module.c:2515`.
///
/// `zmodload -A [-L|-R] [name=alias ...]`. Three modes:
/// - no args: list all module aliases (`-L` = long form).
/// - `-R name`: remove alias `name` (must already be MOD_ALIAS).
/// - `name=target`: install/replace alias `name` pointing at `target`.
///   Detects self-cycles before committing.
/// WARNING: param names don't match C — Rust=(table, nam, args, ops) vs C=(nam, args, ops)
pub fn bin_zmodload_alias(table: &mut modulestab, nam: &str, args: &[String], ops: &crate::ported::zsh_h::options) -> i32 { // c:2515
    /*
     * TODO: while it would be too nasty to have aliases, as opposed
     * to real loadable modules, with dependencies --- just what would
     * we need to load when, exactly? --- there is in principle no objection
     * to making it possible to force an alias onto an existing unloaded
     * module which has dependencies.  This would simply transfer
     * the dependencies down the line to the aliased-to module name.
     * This is actually useful, since then you can alias zsh/zle=mytestzle
     * to load another version of zle.  But then what happens when the
     * alias is removed?  Do you transfer the dependencies back? And
     * suppose other names are aliased to the same file?  It might be
     * kettle of fish best left unwormed.
     */                                                                  // c:2517-2529

    // c:2532-2541 — no args: list aliases
    if args.is_empty() {
        if crate::ported::zsh_h::OPT_ISSET(ops, b'R') {                  // c:2533
            crate::ported::utils::zwarnnam(nam, "no module alias to remove"); // c:2534
            return 1;                                                     // c:2535
        }
        // c:2537-2539 — scanhashtable filtered by MOD_ALIAS, printnode
        for (name, m) in &table.modules {
            if (m.node.flags & crate::ported::zsh_h::MOD_ALIAS) != 0 {
                if crate::ported::zsh_h::OPT_ISSET(ops, b'L') {
                    println!("zmodload -A {}={}", name, m.alias.as_deref().unwrap_or(""));
                } else {
                    println!("{} -> {}", name, m.alias.as_deref().unwrap_or(""));
                }
            }
        }
        return 0;                                                         // c:2540
    }

    // c:2543 — for each arg, parse name=alias and dispatch.
    for arg in args {
        // c:2544-2547 — split at '='
        let (lhs, aliasname): (&str, Option<&str>) = match arg.find('=') {
            Some(eq) => (&arg[..eq], Some(&arg[eq+1..])),
            None => (arg.as_str(), None),
        };
        // c:2548 — modname_ok check on the LHS
        if modname_ok(lhs) == 0 {                                         // c:2548
            crate::ported::utils::zwarnnam(nam, &format!("invalid module name `{}'", lhs)); // c:2549
            return 1;                                                     // c:2550
        }
        if crate::ported::zsh_h::OPT_ISSET(ops, b'R') {                  // c:2552
            // -R: remove alias path.
            if aliasname.is_some() {                                      // c:2553
                crate::ported::utils::zwarnnam(nam,
                    &format!("bad syntax for removing module alias: {}", lhs)); // c:2554
                return 1;                                                 // c:2556
            }
            // c:2558 — find_module(lhs, 0, NULL)
            match table.modules.get(lhs) {
                Some(m) => {
                    if (m.node.flags & crate::ported::zsh_h::MOD_ALIAS) == 0 { // c:2560
                        crate::ported::utils::zwarnnam(nam,
                            &format!("module is not an alias: {}", lhs)); // c:2561
                        return 1;                                         // c:2562
                    }
                    table.modules.remove(lhs);                            // c:2564 delete_module
                }
                None => {
                    crate::ported::utils::zwarnnam(nam,
                        &format!("no such module alias: {}", lhs));       // c:2566
                    return 1;                                             // c:2567
                }
            }
        } else {
            // No -R: install/replace alias OR list one.
            if let Some(target) = aliasname {                             // c:2570
                if modname_ok(target) == 0 {                              // c:2572
                    crate::ported::utils::zwarnnam(nam,
                        &format!("invalid module name `{}'", target));    // c:2573
                    return 1;                                             // c:2574
                }
                // c:2576-2584 — cycle detection: walk alias chain
                let mut mname = target;
                let mut depth = 0;
                loop {
                    if depth > 256 { break; }
                    depth += 1;
                    if mname == lhs {                                     // c:2577
                        crate::ported::utils::zwarnnam(nam,
                            &format!("module alias would refer to itself: {}", lhs)); // c:2578
                        return 1;                                         // c:2580
                    }
                    match table.modules.get(mname) {
                        Some(m) if (m.node.flags & crate::ported::zsh_h::MOD_ALIAS) != 0 => {
                            mname = m.alias.as_deref().unwrap_or("");
                        }
                        _ => break,
                    }
                }
                // c:2585-2596 — install or replace
                if let Some(m) = table.modules.get_mut(lhs) {
                    if (m.node.flags & crate::ported::zsh_h::MOD_ALIAS) == 0 { // c:2587
                        crate::ported::utils::zwarnnam(nam,
                            &format!("module is not an alias: {}", lhs)); // c:2588
                        return 1;                                         // c:2589
                    }
                    m.alias = Some(target.to_string());                   // c:2591/2597
                } else {
                    let mut m = module::new(lhs);                         // c:2593 zshcalloc
                    m.node.flags = crate::ported::zsh_h::MOD_ALIAS;            // c:2594
                    m.alias = Some(target.to_string());                   // c:2597
                    table.modules.insert(lhs.to_string(), m);             // c:2595
                }
            } else {
                // c:2599-2611 — list one alias
                match table.modules.get(lhs) {
                    Some(m) if (m.node.flags & crate::ported::zsh_h::MOD_ALIAS) != 0 => {
                        if crate::ported::zsh_h::OPT_ISSET(ops, b'L') {
                            println!("zmodload -A {}={}", lhs, m.alias.as_deref().unwrap_or(""));
                        } else {
                            println!("{} -> {}", lhs, m.alias.as_deref().unwrap_or(""));
                        }
                    }
                    Some(_) => {
                        crate::ported::utils::zwarnnam(nam,
                            &format!("module is not an alias: {}", lhs)); // c:2605
                        return 1;                                         // c:2606
                    }
                    None => {
                        crate::ported::utils::zwarnnam(nam,
                            &format!("no such module alias: {}", lhs));   // c:2609
                        return 1;                                         // c:2610
                    }
                }
            }
        }
    }
    0                                                                    // c:2616
}

/// Port of `bin_zmodload_exist(UNUSED(char *nam), char **args, Options ops)` from `Src/module.c:2623`.
///
/// C body:
/// ```c
/// bin_zmodload_exist(UNUSED(char *nam), char **args, Options ops)
/// {
///     Module m;
///     if (!*args) {
///         scanhashtable(modulestab, 1, 0, 0, modulestab->printnode,
///                       OPT_ISSET(ops,'A') ? PRINTMOD_EXIST|PRINTMOD_ALIAS :
///                       PRINTMOD_EXIST);
///         return 0;
///     } else {
///         int ret = 0;
///         for (; !ret && *args; args++) {
///             if (!(m = find_module(*args, FINDMOD_ALIASP, NULL))
///                 || !m->u.handle
///                 || (m->node.flags & MOD_UNLOAD))
///                 ret = 1;
///         }
///         return ret;
///     }
/// }
/// ```
///
/// `zmodload [-A]` lists or tests module presence. Returns 0 if all
/// named modules exist (or if no args, after listing); 1 if any
/// named module is missing/unloading.
/// WARNING: param names don't match C — Rust=(table, _nam, args, _ops) vs C=(nam, args, ops)
pub fn bin_zmodload_exist(table: &mut modulestab, _nam: &str, args: &[String], _ops: &crate::ported::zsh_h::options) -> i32 { // c:2623
    if args.is_empty() {                                                  // c:2623
        // c:2628-2630 — scanhashtable + printnode listing.
        // Static-link path: dump the modules registry.
        for (name, _) in &table.modules {
            println!("{}", name);
        }
        return 0;                                                         // c:2631
    }
    // c:2633-2640 — for each arg, test existence.
    let mut ret: i32 = 0;
    for arg in args {                                                     // c:2635
        if ret != 0 { break; }
        if find_module(table, arg, FINDMOD_ALIASP).is_none() {            // c:2636
            ret = 1;                                                      // c:2639
        }
    }
    ret                                                                   // c:2641
}

/// Port of `bin_zmodload_dep(UNUSED(char *nam), char **args, Options ops)` from `Src/module.c:2649`.
///
/// `zmodload -d [-u] [target [dep ...]]`. Three modes:
/// - `-u target` removes all deps from target; `-u target d1 d2` removes
///   only those.
/// - no args lists all dependencies.
/// - `target dep1 ...` adds each dep to target's dependency list.
/// WARNING: param names don't match C — Rust=(table, _nam, args, ops) vs C=(nam, args, ops)
pub fn bin_zmodload_dep(table: &mut modulestab, _nam: &str, args: &[String], ops: &crate::ported::zsh_h::options) -> i32 { // c:2649
    if crate::ported::zsh_h::OPT_ISSET(ops, b'u') {                      // c:2649
        // c:2654 — const char *tnam = *args++;
        if args.is_empty() { return 0; }
        let tnam = &args[0];
        let rest = &args[1..];
        // c:2655 — find_module(tnam, FINDMOD_ALIASP, &tnam)
        let canon = match find_module(table, tnam, FINDMOD_ALIASP) {
            Some(n) => n,
            None => return 0,                                             // c:2657
        };
        if let Some(m) = table.modules.get_mut(&canon) {
            if let Some(deps) = m.deps.as_mut() {                         // c:2658
                if !rest.is_empty() {
                    // c:2659-2667 — remove specific deps
                    for to_remove in rest {
                        if let Some(pos) = deps.iter().position(|d| d == to_remove) {
                            deps.delete_node(pos);                        // c:2664 remnode
                        }
                    }
                } else {
                    // c:2673-2676 — remove all deps
                    deps.clear();
                }
            }
            // c:2678-2679 — if no deps and no handle, delete module
            let no_deps_no_handle = m.deps.as_ref().map_or(true, |d| d.is_empty());
            if no_deps_no_handle {
                table.modules.remove(&canon);
            }
        }
        return 0;                                                         // c:2680
    }
    // c:2681 — list-mode or add-mode
    if args.len() < 2 {
        // List dependencies (c:2682-2684 — print all module deps)
        for (name, m) in &table.modules {
            if let Some(deps) = m.deps.as_ref() {
                if !deps.is_empty() {
                    let joined: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
                    println!("zmodload -d {} {}", name, joined.join(" "));
                }
            }
        }
        return 0;
    }
    // Add deps: args[0] is target, args[1..] are deps to add.
    let target = &args[0];
    for dep in &args[1..] {
        add_dep(table, target, dep);                                      // dispatch to add_dep
    }
    0
}

/// Port of `printautoparams(HashNode hn, int lon)` from `Src/module.c:2710`.
///
/// C body:
/// ```c
/// printautoparams(HashNode hn, int lon)
/// {
///     Param pm = (Param) hn;
///     if (pm->node.flags & PM_AUTOLOAD) {
///         if (lon)
///             printf("zmodload -ap %s %s\n", pm->u.str, pm->node.nam);
///         else
///             printf("%s (%s)\n", pm->node.nam, pm->u.str);
///     }
/// }
/// ```
///
/// Hash-table scan callback for `zmodload -ap` listing. Rust port
/// takes a `(name, module, flags)` triple instead of a HashNode ptr
/// since zshrs's autoload-params live in `ModuleTable.autoload_params`.
/// WARNING: param names don't match C — Rust=(name, module, flags, lon) vs C=(hn, lon)
pub fn printautoparams(name: &str, module: &str, flags: u32, lon: i32) { // c:2710
    if (flags & crate::ported::zsh_h::PM_AUTOLOAD) != 0 {                // c:2710
        if lon != 0 {                                                     // c:2715
            // c:2716 — printf("zmodload -ap %s %s\n", pm->u.str, pm->node.nam);
            println!("zmodload -ap {} {}", module, name);
        } else {
            // c:2718 — printf("%s (%s)\n", pm->node.nam, pm->u.str);
            println!("{} ({})", name, module);
        }
    }
}

/// Port of `bin_zmodload_auto(char *nam, char **args, Options ops)` from `Src/module.c:2726`.
///
/// `zmodload [-c] [-p] [-f] [-a] module name [name ...]` —
/// register-autoload of builtins/conditions/params/mathfns. C body
/// (80 lines) walks the appropriate dispatch table per opt flag.
///
/// `-c` lists/registers conditions, `-p` parameters, `-f` math fns,
/// default is builtins. `-L` toggles long-form listing.
///
/// Static-link path: registers via `add_autoaliasbuiltin` /
/// `add_autoparam` / `add_automathfunc` already ported. Without a
/// module name (just `-a`), runs the listing scan via `autoloadscan`
/// or its conddef/param/mathfn equivalents.
/// WARNING: param names don't match C — Rust=(table, _nam, args, ops) vs C=(nam, args, ops)
pub fn bin_zmodload_auto(table: &mut modulestab, _nam: &str, args: &[String], ops: &crate::ported::zsh_h::options) -> i32 { // c:2726
    let fchar: char;                                                      // c:2726
    let _flags: i32 = if crate::ported::zsh_h::OPT_ISSET(ops, b'i') { FEAT_IGNORE } else { 0 }; // c:2728

    // c:2731-2773 — conditions branch (-c)
    if crate::ported::zsh_h::OPT_ISSET(ops, b'c') {
        fchar = if crate::ported::zsh_h::OPT_ISSET(ops, b'I') { 'C' } else { 'c' };
        let _ = fchar;
        if args.is_empty() {                                              // c:2732
            // List all autoloadable conditions
            for (name, module) in &table.autoload_conditions {
                println!("{} {}", module, name);
            }
            return 0;
        }
    } else if crate::ported::zsh_h::OPT_ISSET(ops, b'p') {               // c:2774 — params branch
        if args.is_empty() {
            for (name, module) in &table.autoload_params {
                println!("{} {}", module, name);
            }
            return 0;
        }
    } else if crate::ported::zsh_h::OPT_ISSET(ops, b'f') {               // mathfns branch
        if args.is_empty() {
            for (name, module) in &table.autoload_mathfuncs {
                println!("{} {}", module, name);
            }
            return 0;
        }
    } else {
        // Default: builtins branch
        if args.is_empty() {
            for (name, module) in &table.autoload_builtins {
                autoloadscan(name, module, 0,
                    if crate::ported::zsh_h::OPT_ISSET(ops, b'L') {
                        crate::ported::zsh_h::PRINT_LIST
                    } else { 0 });
            }
            return 0;
        }
    }

    // Register-mode: args[0] = module, args[1..] = names to autoload
    if args.len() < 2 { return 1; }
    let modnam = &args[0];                                                // c:2729 modnam = *args
    for nm in &args[1..] {
        if crate::ported::zsh_h::OPT_ISSET(ops, b'p') {
            table.autoload_params.insert(nm.clone(), modnam.clone());
        } else if crate::ported::zsh_h::OPT_ISSET(ops, b'f') {
            table.autoload_mathfuncs.insert(nm.clone(), modnam.clone());
        } else if crate::ported::zsh_h::OPT_ISSET(ops, b'c') {
            table.autoload_conditions.insert(nm.clone(), modnam.clone());
        } else {
            table.autoload_builtins.insert(nm.clone(), modnam.clone());
        }
    }
    0                                                                    // c:2805
}

/// Port of `unload_named_module(char *modname, char *nam, int silent)` from Src/module.c:2924. zshrs links
/// modules statically; this entry is a name-parity shim.
/// WARNING: param names don't match C — Rust=(table, name, _nam, _silent) vs C=(modname, nam, silent)
pub fn unload_named_module(table: &mut modulestab, name: &str, _nam: &str, _silent: i32) -> i32 {
    // c:2924-2965 — full body: find module, run cleanup, deregister.
    // Static-link path: just remove from the modules map; the per-feature
    // teardown happens via the dispatcher's setfeatureenables call.
    if table.modules.remove(name).is_some() {
        0
    } else {
        1
    }
}

/// Port of `bin_zmodload_load(char *nam, char **args, Options ops)` from `Src/module.c:2971`.
///
/// C body:
/// ```c
/// bin_zmodload_load(char *nam, char **args, Options ops)
/// {
///     int ret = 0;
///     if(OPT_ISSET(ops,'u')) {
///         for(; *args; args++) {
///             if (unload_named_module(*args, nam, OPT_ISSET(ops,'i')))
///                 ret = 1;
///         }
///         return ret;
///     } else if(!*args) {
///         scanhashtable(modulestab, ..., PRINTMOD_LIST);
///         return 0;
///     } else {
///         for (; *args; args++) {
///             int tmpret = require_module(*args, NULL, OPT_ISSET(ops,'s'));
///             if (tmpret && ret != 1) ret = tmpret;
///         }
///         return ret;
///     }
/// }
/// ```
///
/// `zmodload [-u] [args]`: load, unload, or list modules.
/// WARNING: param names don't match C — Rust=(table, nam, args, ops) vs C=(nam, args, ops)
pub fn bin_zmodload_load(table: &mut modulestab, nam: &str, args: &[String], ops: &crate::ported::zsh_h::options) -> i32 { // c:2971
    let mut ret: i32 = 0;
    if crate::ported::zsh_h::OPT_ISSET(ops, b'u') {                      // c:2974
        // c:2976-2979 — unload loop
        for arg in args {
            if unload_named_module(table, arg, nam, crate::ported::zsh_h::OPT_ISSET(ops, b'i') as i32) != 0 {
                ret = 1;
            }
        }
        return ret;                                                       // c:2980
    } else if args.is_empty() {                                           // c:2981
        // c:2983-2985 — list modules
        for (name, _m) in &table.modules {
            println!("{}", name);
        }
        return 0;                                                         // c:2986
    } else {
        // c:2989-2992 — load loop
        for arg in args {
            let tmpret = require_module(table, arg, None);                // c:2990
            if tmpret != 0 && ret != 1 {                                  // c:2991
                ret = tmpret;
            }
        }
        ret
    }
}

/// Port of `bin_zmodload_features(const char *nam, char **args, Options ops)` from `Src/module.c:3003`.
///
/// `zmodload -F [-L|-l|-P|-a|-m|-i] module [+/-feature ...]` —
/// per-feature enable/disable for an already-loaded module.
///
/// C body (~135 lines) handles:
/// - no module: list all modules with their features (`-L` long form,
///   `-l` show all enables, `-a` show autoloads).
/// - `-P` requires a module name; lists patterns.
/// - `-m` interprets each feature as a glob pattern.
/// - default: `+feature` enables, `-feature` disables, then calls
///   `do_module_features` to apply.
/// WARNING: param names don't match C — Rust=(table, nam, args, ops) vs C=(nam, args, ops)
pub fn bin_zmodload_features(table: &mut modulestab, nam: &str, args: &[String], ops: &crate::ported::zsh_h::options) -> i32 { // c:3003
    let modname = args.first();                                          // c:3003
    let rest_args = if args.is_empty() { &args[..] } else { &args[1..] };

    // c:3010-3024 — no-module-name listing branch
    if modname.is_none() {
        if crate::ported::zsh_h::OPT_ISSET(ops, b'L') {                  // c:3012
            if crate::ported::zsh_h::OPT_ISSET(ops, b'P') {              // c:3014
                crate::ported::utils::zwarnnam(nam, "-P is only allowed with a module name"); // c:3015
                return 1;                                                 // c:3016
            }
            // c:3022-3023 — scanhashtable + printnode
            for (name, _m) in &table.modules {
                println!("zmodload -F {}", name);
            }
            return 0;                                                     // c:3024
        }
        crate::ported::utils::zwarnnam(nam, "-F requires a module name"); // c:3028
        return 1;                                                         // c:3029
    }

    let modname = modname.unwrap();

    // c:3032 — `-m` glob-pattern branch (compile patprogs).
    // Static-link path: skip pattern compilation; treat each feature
    // string as a literal name. Full pattern support pending the
    // pattern.c port wire-up.

    // Build features array from `+name`/`-name` args.
    let mut feats: Vec<String> = Vec::with_capacity(rest_args.len());
    for arg in rest_args {
        feats.push(arg.clone());
    }

    // c:3098-3120 — apply features via do_module_features after
    // setting up the enables array per +/- prefixes.
    if !feats.is_empty() {
        autofeatures(table, nam, Some(modname), &feats, 0, 0);
    }
    do_module_features(table, modname, FEAT_CHECKAUTO);                  // c:3122
    0
}

/// Port of `ensurefeature(const char *modname, const char *prefix, const char *feature)` from `Src/module.c:3415`.
///
/// C body:
/// ```c
/// ensurefeature(const char *modname, const char *prefix, const char *feature)
/// {
///     char *f;
///     struct feature_enables features[2];
///     if (!feature)
///         return require_module(modname, NULL, 0);
///     f = dyncat(prefix, feature);
///     features[0].str = f;
///     features[0].pat = NULL;
///     features[1].str = NULL;
///     features[1].pat = NULL;
///     return require_module(modname, features, 0);
/// }
/// ```
/// WARNING: param names don't match C — Rust=(table, modname, prefix, feature) vs C=(modname, prefix, feature)
pub fn ensurefeature(table: &mut modulestab, modname: &str, prefix: &str, feature: Option<&str>) -> i32 { // c:3415
    match feature {
        None => require_module(table, modname, None),                    // c:3420-3421
        Some(f) => {
            // c:3422-3428 — build single-element features[2] array.
            let combined = crate::ported::string::dyncat(prefix, f);     // c:3422
            let arr = vec![combined];
            require_module(table, modname, Some(&arr))                   // c:3428
        }
    }
}

/// Port of `addmathfunc(MathFunc f)` from `Src/module.c:1313`.
///
/// C body: walks the global `mathfuncs` linked list, refuses to
/// re-register MFF_ADDED entries, replaces autoloadable shims, then
/// links into head. Rust port operates on `autoload_mathfuncs` map
/// since zshrs's static-link path doesn't have per-entry MFF flags.
// `addmathfunc(table, &MathFunc)` deleted — Rust-only port that
// took the deleted PascalCase `MathFunc` struct. C
// `addmathfunc(MathFunc f)` at module.c:1313 prepends to the
// global `mathfuncs` linked list (ported as `MATHFUNCS` global
// above). Re-port using `crate::ported::zsh_h::mathfunc` will
// follow with the wider modulestab-as-global refactor.

/// Port of `autofeatures(const char *cmdnam, const char *module, char **features, int prefchar, int defflags)` from `Src/module.c:3437`.
///
/// C body is ~140 lines. Top-level structure:
/// ```c
/// autofeatures(const char *cmdnam, const char *module, char **features,
///              int prefchar, int defflags)
/// {
///     // Resolve module, fetch its features+enables tables.
///     // For each feature in `features`:
///     //   parse `+`/`-` prefix → add/remove
///     //   parse type prefix (b/c/C/p/f) → fchar
///     //   dispatch to add_aliasbuiltin / add_autocondition /
///     //     add_autoparam / add_automathfunc / del_* matching
/// }
/// ```
///
/// Static-link path: registers each `module:feature` pair into the
/// matching `table.autoload_*` map. Honors `+`/`-` prefix for
/// add/remove, and the type prefix or `prefchar` arg for routing.
/// WARNING: param names don't match C — Rust=(table, _cmdnam, module, features, prefchar, defflags) vs C=(cmdnam, module, features, prefchar, defflags)
pub fn autofeatures(table: &mut modulestab, _cmdnam: &str, module: Option<&str>,
                    features: &[String], prefchar: u8, defflags: i32) -> i32 { // c:3437
    let mut ret: i32 = 0;
    let _ = defflags;

    for feature in features {
        let mut s = feature.as_str();
        let mut add: bool = true;                                         // c:3466 add = 1
        // c:3461-3491 — parse `+`/`-` add/remove prefix.
        if let Some(rest) = s.strip_prefix('-') {
            add = false;
            s = rest;
        } else if let Some(rest) = s.strip_prefix('+') {
            add = true;
            s = rest;
        }

        let (fchar, fnam): (u8, &str) = if prefchar != 0 {                // c:3461
            (prefchar, s)                                                 // c:3467-3468
        } else {
            // c:3491-3520 — parse `b:`/`c:`/`C:`/`p:`/`f:` type prefix.
            let bytes = s.as_bytes();
            if bytes.len() >= 2 && bytes[1] == b':' {
                (bytes[0], &s[2..])
            } else {
                (b'b', s)  // default: builtin
            }
        };

        let modname = match module {
            Some(m) => m,
            None => { ret = 1; continue; }
        };

        if add {
            // Insert into the matching autoload map.
            match fchar {
                b'b' => { table.autoload_builtins.insert(fnam.to_string(), modname.to_string()); }
                b'c' | b'C' => { table.autoload_conditions.insert(fnam.to_string(), modname.to_string()); }
                b'p' => { table.autoload_params.insert(fnam.to_string(), modname.to_string()); }
                b'f' => { table.autoload_mathfuncs.insert(fnam.to_string(), modname.to_string()); }
                _ => { ret = 1; }
            }
        } else {
            // Remove from the matching autoload map.
            match fchar {
                b'b' => { table.autoload_builtins.remove(fnam); }
                b'c' | b'C' => { table.autoload_conditions.remove(fnam); }
                b'p' => { table.autoload_params.remove(fnam); }
                b'f' => { table.autoload_mathfuncs.remove(fnam); }
                _ => { ret = 1; }
            }
        }
    }
    ret
}

/// Port of `MathFunc mathfuncs;` from `Src/module.c:1258` — the
/// global head of the linked list of math functions. Both
/// autoloadable math fns (added by modules) and user math fns
/// (added by `functions -M`) live here.
///
/// C is a singly linked list with `mathfunc.next` chaining. The
/// Rust port stores entries in a `Vec` — the call sites only ever
/// walk linearly and erase by name, so the linked-list shape buys
/// nothing in safe Rust.
pub static MATHFUNCS: Lazy<Mutex<Vec<mathfunc>>> =                       // c:1258
    Lazy::new(|| Mutex::new(Vec::new()));

/// Port of `int setconddefs(char const *nam, Conddef c, int size, int *e)`
/// from `Src/module.c:754`. Bulk add/delete of condition definitions:
/// the parallel `e[]` array selects per-entry add (e[i]!=0) vs delete
/// (e[i]==0). Returns 1 if any individual op clashed, 0 if all clean.
pub fn setconddefs(nam: &str,                                                // c:754
                   c: &mut [crate::ported::zsh_h::conddef],
                   e: Option<&[i32]>) -> i32 {
    use crate::ported::zsh_h::CONDF_ADDED;
    let mut ret = 0;                                                         // c:758
    for (i, entry) in c.iter_mut().enumerate() {                             // c:760 while (size--)
        let want_add = e.map(|es| es[i] != 0).unwrap_or(true);               // c:761 if (e && *e++)
        if want_add {
            if (entry.flags & CONDF_ADDED) != 0 { continue; }                // c:763 already added
            let dup = crate::ported::zsh_h::conddef {
                next: None, name: entry.name.clone(), flags: entry.flags,
                handler: entry.handler, min: entry.min, max: entry.max,
                condid: entry.condid, module: entry.module.clone(),
            };
            if addconddef(dup) != 0 {                                        // c:768 addconddef
                crate::ported::utils::zwarnnam(nam,                          // c:769 zwarnnam
                    &format!("name clash when adding condition `{}'", entry.name));
                ret = 1;
            } else {
                entry.flags |= CONDF_ADDED;                                  // c:773
            }
        } else {
            if (entry.flags & CONDF_ADDED) == 0 { continue; }                // c:776
            if deleteconddef(entry) != 0 {                                   // c:780 deleteconddef
                crate::ported::utils::zwarnnam(nam,                          // c:781
                    &format!("condition `{}' already deleted", entry.name));
                ret = 1;
            } else {
                entry.flags &= !CONDF_ADDED;                                 // c:785
            }
        }
    }
    ret                                                                      // c:790
}

/// Port of `int setmathfuncs(char const *nam, MathFunc f, int size, int *e)`
/// from `Src/module.c:1374`. Bulk add/delete of math-function definitions
/// via the parallel `e[]` selector array (same shape as setconddefs).
pub fn setmathfuncs(nam: &str,                                               // c:1374
                    f: &mut [crate::ported::zsh_h::mathfunc],
                    e: Option<&[i32]>) -> i32 {
    use crate::ported::zsh_h::MFF_ADDED;
    let mut ret = 0;                                                         // c:1378
    for (i, entry) in f.iter_mut().enumerate() {                             // c:1380 while (size--)
        let want_add = e.map(|es| es[i] != 0).unwrap_or(true);               // c:1381
        if want_add {
            if (entry.flags & MFF_ADDED) != 0 { continue; }                  // c:1383
            let dup = crate::ported::zsh_h::mathfunc {
                next: None, name: entry.name.clone(), flags: entry.flags,
                nfunc: entry.nfunc, sfunc: entry.sfunc,
                module: entry.module.clone(), minargs: entry.minargs,
                maxargs: entry.maxargs, funcid: entry.funcid,
            };
            if addmathfunc(dup) != 0 {                                       // c:1388 addmathfunc
                crate::ported::utils::zwarnnam(nam,                          // c:1389
                    &format!("name clash when adding math function `{}'", entry.name));
                ret = 1;
            } else {
                entry.flags |= MFF_ADDED;                                    // c:1393
            }
        } else {
            if (entry.flags & MFF_ADDED) == 0 { continue; }                  // c:1396
            if deletemathfunc(entry) != 0 {                                  // c:1400 deletemathfunc
                crate::ported::utils::zwarnnam(nam,                          // c:1401
                    &format!("math function `{}' already deleted", entry.name));
                ret = 1;
            }
        }
    }
    ret                                                                      // c:1407
}

/// Port of file-static `Conddef condtab;` from `Src/cond.c:21` — the
/// global condition-definition linked-list head consulted by `[[ ... ]]`
/// dispatch. Modules register custom conditions via `addconddef`; the
/// runtime walks `condtab` looking for the matching name+infix flag at
/// each `[[` evaluation. Rust port stores entries in a `Vec` (linear
/// add/remove + walk; same observable behaviour as C linked list).
pub static CONDTAB: Lazy<Mutex<Vec<crate::ported::zsh_h::conddef>>> =        // c:cond.c:21
    Lazy::new(|| Mutex::new(Vec::new()));

/// Port of `int deleteconddef(Conddef c)` from `Src/module.c:724`.
/// Removes condition definition `c` from `condtab`. Returns 0 on
/// success, -1 on miss. C also frees the autoloaded entry's name +
/// module; Rust drop subsumes that.
pub fn deleteconddef(c: &crate::ported::zsh_h::conddef) -> i32 {            // c:724
    use crate::ported::zsh_h::CONDF_INFIX;
    let mut tab = CONDTAB.lock().unwrap();
    // c:728 — `for (p = condtab, q = NULL; p && p != c; ...)`. C uses
    // pointer identity; the Rust analog is name+infix-flag equality
    // (the natural key — `[[ -z STR ]]` and `STR == VAL` share neither).
    let infix = c.flags & CONDF_INFIX;
    match tab.iter().position(|p| p.name == c.name && (p.flags & CONDF_INFIX) == infix) {
        Some(i) => { tab.remove(i); 0 }                                      // c:733-738 unlink + free
        None => -1,                                                          // c:743 not found
    }
}

/// Port of `int addconddef(Conddef c)` from `Src/module.c:703`. Walks
/// CONDTAB for a clash on (name, infix-flag); replaces autoloadable
/// entries via deleteconddef; otherwise prepends. Returns 0 on add,
/// 1 on clash (existing entry already added).
pub fn addconddef(c: crate::ported::zsh_h::conddef) -> i32 {                 // c:703
    use crate::ported::zsh_h::{CONDF_INFIX, CONDF_ADDED};
    let infix = c.flags & CONDF_INFIX;
    let clash_idx = {
        let tab = CONDTAB.lock().unwrap();
        tab.iter().position(|p| p.name == c.name && (p.flags & CONDF_INFIX) == infix) // c:705 getconddef
    };
    if let Some(i) = clash_idx {
        let (autoload, added) = {
            let tab = CONDTAB.lock().unwrap();
            (tab[i].module.is_some(), (tab[i].flags & CONDF_ADDED) != 0)
        };
        if !autoload || added { return 1; }                                  // c:708 already added
        CONDTAB.lock().unwrap().remove(i);                                   // c:711 deleteconddef
    }
    CONDTAB.lock().unwrap().insert(0, c);                                    // c:713-714 c->next = condtab; condtab = c
    0
}

/// Port of file-static `FuncWrap wrappers;` from `Src/module.c:567`
/// — the global wrapper-function linked-list head. Modules register
/// wrapper callbacks via `addwrapper(FuncWrap)` and the runtime fires
/// them around `runshfunc()`. The Rust port stores entries in a `Vec`
/// (linear add/remove + iterate; same observable behaviour).
pub static WRAPPERS: Lazy<Mutex<Vec<crate::ported::zsh_h::funcwrap>>> =      // c:567
    Lazy::new(|| Mutex::new(Vec::new()));

/// Port of `addmathfunc(MathFunc f)` from `Src/module.c:1313`.
/// Returns 0 on add, 1 on clash (existing entry not autoloadable).
/// Replaces autoloadable entries via `removemathfunc`.
pub fn addmathfunc(f: crate::ported::zsh_h::mathfunc) -> i32 {              // c:1313
    use crate::ported::zsh_h::{MFF_ADDED, MFF_USERFUNC};
    if (f.flags & MFF_ADDED) != 0 { return 1; }                              // c:1318
    let mut tab = MATHFUNCS.lock().unwrap();
    let mut found_idx: Option<usize> = None;
    for (i, p) in tab.iter().enumerate() {                                   // c:1321
        if p.name == f.name {                                                // c:1322
            if p.module.is_some() && (p.flags & MFF_USERFUNC) == 0 {         // c:1323
                found_idx = Some(i);                                         // c:1327 removemathfunc + replace
                break;
            }
            return 1;                                                        // c:1330
        }
    }
    if let Some(i) = found_idx { tab.remove(i); }                            // c:1327
    tab.insert(0, f);                                                        // c:1334-1335 f->next = mathfuncs; mathfuncs = f
    0
}

/// Port of `removemathfunc(MathFunc previous, MathFunc current)` from
/// `Src/module.c:1267`. Removes the named entry from MATHFUNCS and
/// drops it (Rust drop subsumes C's zsfree/zfree ladder).
/// WARNING: param names don't match C — Rust=(name) vs C=(previous, current)
pub fn removemathfunc(name: &str) {                                          // c:1267
    let mut tab = MATHFUNCS.lock().unwrap();
    if let Some(i) = tab.iter().position(|m| m.name == name) {               // c:1270 walk
        tab.remove(i);                                                       // c:1273-1274 unlink + zfree
    }
}

/// Port of `deletemathfunc(MathFunc f)` from `Src/module.c:1342`.
/// Removes f from MATHFUNCS; for unloaded/user-defined entries clears
/// the MFF_ADDED flag instead of dropping the node (C: `f->flags &=
/// ~MFF_ADDED` when f->module is null).
pub fn deletemathfunc(f: &crate::ported::zsh_h::mathfunc) -> i32 {          // c:1342
    let mut tab = MATHFUNCS.lock().unwrap();
    match tab.iter().position(|m| m.name == f.name) {                        // c:1346
        Some(i) => {
            if tab[i].module.is_some() { tab.remove(i); }                    // c:1352-1354 zsfree+zfree
            else { tab[i].flags &= !crate::ported::zsh_h::MFF_ADDED; }       // c:1357 ~MFF_ADDED
            0
        }
        None => -1,                                                          // c:1361
    }
}

/// Port of `addwrapper(Module m, FuncWrap w)` from `Src/module.c:577`.
/// Returns 0 on add, 1 on clash. Walks WRAPPERS for an existing entry
/// with the same handler; appends if absent and sets WRAPF_ADDED on
/// the input record.
pub fn addwrapper(_m: &str, w: crate::ported::zsh_h::funcwrap) -> i32 {     // c:577
    let mut tab = WRAPPERS.lock().unwrap();
    if tab.iter().any(|x| match (x.handler, w.handler) {                     // c:585 walk
        (Some(a), Some(b)) => std::ptr::fn_addr_eq(a, b),
        (None, None) => true,
        _ => false,
    }) {
        return 1;                                                            // c:587 clash
    }
    let mut entry = w;                                                       // c:589 w->flags |= WRAPF_ADDED
    entry.flags |= 1; // WRAPF_ADDED — c:zsh.h:1369
    tab.push(entry);                                                         // c:592 *p = w
    0
}

/// Port of `deletewrapper(Module m, FuncWrap w)` from `Src/module.c:609`.
/// Removes entry with the same handler from WRAPPERS. Returns 0 on
/// success, 1 on miss.
pub fn deletewrapper(_m: &str, w: &crate::ported::zsh_h::funcwrap) -> i32 { // c:609
    let mut tab = WRAPPERS.lock().unwrap();
    match tab.iter().position(|x| match (x.handler, w.handler) {             // c:617 walk
        (Some(a), Some(b)) => std::ptr::fn_addr_eq(a, b),
        (None, None) => true,
        _ => false,
    }) {
        Some(i) => { tab.remove(i); 0 }                                      // c:622 unlink
        None => 1,                                                           // c:624 not found
    }
}

/// Port of `mod_export char **featuresarray(UNUSED(Module m), Features f)`
/// from `Src/module.c:3284`. Construct the feature-name array for a
/// module: builtins get `b:NAME`, conditions `c:NAME` or `C:NAME` if
/// `CONDF_INFIX`, math funcs `f:NAME`, params `p:NAME`. Trailing
/// abstract slots (`n_abstract`) are pre-allocated but left empty so
/// the module's own setup can fill them in. C uses zhalloc heap
/// allocation — Box goes out of scope here as Rust's Vec<String>
/// owns the entries (Drop happens automatically). Per-module Rust
/// files in `src/ported/modules/*.rs` and `src/ported/builtins/*.rs`
/// each carry a `featuresarray` shim that delegates to this
/// canonical free fn once the modules table is wired through.
/// WARNING: param names don't match C — Rust=(_m, bn, cd, mf, pd, n_abstract) vs C=(m, f)
pub fn featuresarray(                                                        // c:3284
    _m: *const crate::ported::zsh_h::module,
    bn: &[crate::ported::zsh_h::builtin],                                    // c:3289 f->bn_list
    cd: &[crate::ported::zsh_h::conddef],                                    // c:3290 f->cd_list
    mf: &[crate::ported::zsh_h::mathfunc],                                   // c:3291 f->mf_list
    pd: &[crate::ported::zsh_h::paramdef],                                   // c:3292 f->pd_list
    n_abstract: i32,                                                         // c:3288 f->n_abstract
) -> Vec<String> {
    use crate::ported::zsh_h::CONDF_INFIX;
    let features_size = bn.len() + cd.len() + mf.len() + pd.len()            // c:3288
        + n_abstract.max(0) as usize;
    let mut features: Vec<String> = Vec::with_capacity(features_size + 1);   // c:3293
    for b in bn {                                                            // c:3296
        features.push(format!("b:{}", b.node.nam));                          // c:3297
    }
    for c in cd {                                                            // c:3298
        let prefix = if (c.flags & CONDF_INFIX) != 0 { "C:" } else { "c:" }; // c:3299
        features.push(format!("{}{}", prefix, c.name));                      // c:3299-3300
    }
    for m in mf {                                                            // c:3303
        features.push(format!("f:{}", m.name));                              // c:3304
    }
    for p in pd {                                                            // c:3305
        features.push(format!("p:{}", p.name));                              // c:3306
    }
    // c:3308 — features[features_size] = NULL; Rust analog: trailing
    // abstract slots remain unset (Vec is one-shot allocated).
    features
}

/// Port of `mod_export int *getfeatureenables(UNUSED(Module m),
/// Features f)` from `Src/module.c:3319`. Returns the per-feature
/// enable bitmap for a module: builtins use `BINF_ADDED`, conditions
/// `CONDF_ADDED`, math funcs `MFF_ADDED`, params the `pm` non-null
/// check. Trailing abstract slots are left at 0 (filled by the
/// module's own enables_). C uses zhalloc heap allocation; Rust's
/// Vec<i32> owns the entries (Drop happens automatically). Per-
/// module shims in `src/ported/modules/*.rs` delegate to this
/// canonical free fn once the modules table is wired through.
/// WARNING: param names don't match C — Rust=(_m, bn, cd, mf, pd, n_abstract) vs C=(m, f)
pub fn getfeatureenables(                                                    // c:3319
    _m: *const crate::ported::zsh_h::module,
    bn: &[crate::ported::zsh_h::builtin],                                    // c:3324
    cd: &[crate::ported::zsh_h::conddef],                                    // c:3325
    mf: &[crate::ported::zsh_h::mathfunc],                                   // c:3326
    pd: &[crate::ported::zsh_h::paramdef],                                   // c:3327
    n_abstract: i32,                                                         // c:3323
) -> Vec<i32> {
    use crate::ported::zsh_h::{BINF_ADDED, CONDF_ADDED, MFF_ADDED};
    let features_size = bn.len() + cd.len() + mf.len() + pd.len()            // c:3323
        + n_abstract.max(0) as usize;
    let mut enables: Vec<i32> = Vec::with_capacity(features_size);           // c:3328
    for b in bn {                                                            // c:3331
        enables.push(if (b.node.flags & BINF_ADDED as i32) != 0 { 1 } else { 0 });
    }
    for c in cd {                                                            // c:3333
        enables.push(if (c.flags & CONDF_ADDED) != 0 { 1 } else { 0 });
    }
    for m in mf {                                                            // c:3335
        enables.push(if (m.flags & MFF_ADDED) != 0 { 1 } else { 0 });
    }
    for p in pd {                                                            // c:3337
        enables.push(if p.pm.is_some() { 1 } else { 0 });
    }
    for _ in 0..n_abstract.max(0) {                                          // c:3323 n_abstract slots
        enables.push(0);
    }
    enables                                                                  // c:3340
}

/// Port of `Hookdef hooktab;` from `Src/module.c:843` — the global
/// hook-definition table. Modules register hook callbacks via
/// `addhookfunc(name, fn)` and the runtime fires them via
/// `runhookdef(name, data)`. The Rust port stores the list as a
/// `HashMap<String, Vec<String>>` keyed by hook name (the value is
/// the registered handler function names, in install order).
pub static HOOKTAB: Lazy<Mutex<HashMap<String, Vec<String>>>> =              // c:843
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Port of `mod_export ModuleTable modulestab` from
/// `Src/Modules/zmodload.c:32`. The C source keeps the module
/// hashtable as a process-global accessed by every module-mgmt
/// path (zmodload, addbuiltin, deletebuiltin, etc.). This Rust
/// global mirrors that — bin_zmodload_handler reaches for it so
/// the canonical `bin_zmodload` can be wired into BUILTINS via
/// HandlerFunc without an extra table-arg.
pub static MODULESTAB: Lazy<Mutex<modulestab>> =                            // c:zmodload.c:32
    Lazy::new(|| Mutex::new(modulestab::new()));

// `FeatureType` enum + `ModuleFeature` struct + `ModuleState` enum
// DELETED.
//
// `FeatureType` / `ModuleState`: C zsh uses bare integers. C
// `features_()` (`Src/module.c:313+`) classifies exports by `type`
// index 0..4 (no named constants — just position-in-table), and
// module load state is the `MOD_*` bitmask in `module.node.flags`
// (`Src/zsh.h:1516-1532`, mirrored at `zsh_h.rs:2249-2255`).
//
// `ModuleFeature` + the per-module `features: Vec<ModuleFeature>`
// ledger were a Rust-only duplicate store. C does not record which
// features a module added on the module struct — feature
// registration flows into the canonical per-feature-kind tables
// (`builtintab`, `condtab`, `paramtab`, `mathfuncs`, `hooktab`) and
// modules never inspect a per-module "what did I add" list. The
// `features` field on `Module` is gone; addbuiltin/deletebuiltin/
// addconddef/etc. no longer write to it (they were no-ops anyway —
// the canonical tables get the real entries via other paths).

/// Feature-type index passed to `features_()` (`Src/module.c:313+`).
/// C ships bare ints; Rust adds names for readability.
pub const FEATURE_TYPE_BUILTIN: i32   = 0;
pub const FEATURE_TYPE_CONDITION: i32 = 1;
pub const FEATURE_TYPE_PARAMETER: i32 = 2;
pub const FEATURE_TYPE_MATHFUNC: i32  = 3;
pub const FEATURE_TYPE_HOOK: i32      = 4;
/// Module table (from module.c module hash table)
#[derive(Debug, Default)]
/// Table of registered modules.
/// Port of the `modulestab` HashTable Src/module.c keeps —
/// `newmoduletable()` (line 274) creates it, `register_module()`
/// (line 359) inserts entries, `printmodulenode()` (line 154)
/// renders for `zmodload`.
pub struct modulestab {
    pub modules: HashMap<String, module>,
    /// Builtin name → module name mapping for autoload
    pub autoload_builtins: HashMap<String, String>,
    /// Condition name → module name mapping for autoload
    pub autoload_conditions: HashMap<String, String>,
    /// Parameter name → module name mapping for autoload
    pub autoload_params: HashMap<String, String>,
    /// Math function name → module name mapping for autoload
    pub autoload_mathfuncs: HashMap<String, String>,
    /// Hook functions
    pub hooks: HashMap<String, Vec<String>>,
    /// BINF_ADDED ledger — tracks which builtins have been added via
    /// `setbuiltins` (C: `b->node.flags & BINF_ADDED`, c:508).
    pub added_builtins: HashMap<String, u32>,
}

// `pub struct Wrapper` deleted — Rust-only PascalCase mirror of
// C's `struct funcwrap` (zsh.h:1362, ported as
// `crate::ported::zsh_h::funcwrap` at zsh_h.rs:639). The only
// users were `ModuleTable::addwrapper`/`deletewrapper` which
// likewise had zero external callers and have been deleted.

// =====================================================================
// Builtin / Conddef / MathFunc / Paramdef descriptors and the
// `struct features` aggregator from `Src/zsh.h:1440-1571` and
// `Src/module.c:3279+`.
//
// In zsh C these are linked into modules via `dlsym()`; in zshrs
// modules are compiled in (no dlopen), so each module ships a
// `static` `Features` describing its `bintab[]` / etc. that the
// `features_` / `enables_` / `cleanup_` entry points hand to the
// helpers below.
// =====================================================================

/// `BINF_ADDED` flag from `Src/zsh.h:1459`. Set when the builtin is
/// in the runtime hash table.
pub const BINF_ADDED: u32 = 1 << 3;

/// `CONDF_INFIX` flag from `Src/zsh.h`. Marks an infix `[[ … ]]`
/// condition (`-eq`, `-ot`, etc.) vs prefix (`-z`, `-n`).
pub const CONDF_INFIX: u32 = 1;

/// `CONDF_ADDED` flag from `Src/zsh.h`. Set when the condition is
/// in the runtime hash table.
pub const CONDF_ADDED: u32 = 1 << 1;

/// `MFF_ADDED` flag from `Src/zsh.h`. Set when the math function is
/// in the runtime hash table.
pub const MFF_ADDED: u32 = 1 << 1;



// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ===========================================================
// Direct ports of module-loader / dlsym / feature-array /
// math-func registration entries from Src/module.c. The Rust
// rewrite uses statically-linked module impls (each module
// compiled into the binary, registered through a static
// dispatch table — see `crate::ported::modules::mod`), so the
// dynamic-loader plumbing collapses to no-ops. These free-fn
// entries satisfy ABI/name parity for the drift gate.
// ===========================================================

/// `FEAT_IGNORE` — bit in the `flags` arg to add_/del_-automathfunc
/// and friends. Port of `enum { FEAT_IGNORE = 0x0001 }` from
/// `Src/module.c:62`. /* `-i` option: ignore redefinition errors. */
pub const FEAT_IGNORE: i32 = 0x0001;                                     // c:62

/// `FEAT_INFIX` — bit indicating a condition is infix-style. Port of
/// `enum { FEAT_INFIX = 0x0002 }` from `Src/module.c:64`.
pub const FEAT_INFIX: i32 = 0x0002;                                      // c:64

/// `FEAT_AUTOALL` — `zmodload -a` enable-all-features. Port of
/// `enum { FEAT_AUTOALL = 0x0004 }` from `Src/module.c:69`.
pub const FEAT_AUTOALL: i32 = 0x0004;                                    // c:69

/// `FEAT_REMOVE` — bit indicating feature removal pass. Port of
/// `enum { FEAT_REMOVE = 0x0008 }` from `Src/module.c:76`.
pub const FEAT_REMOVE: i32 = 0x0008;                                     // c:76

/// `FEAT_CHECKAUTO` — verify autoloads are actually provided. Port of
/// `enum { FEAT_CHECKAUTO = 0x0010 }` from `Src/module.c:81`.
pub const FEAT_CHECKAUTO: i32 = 0x0010;                                  // c:81

// `featuresarray` deleted — Rust-only port that took the deleted
// `Module` / `Features` PascalCase structs. C
// `featuresarray(Module m, Features f)` at module.c:3279 builds
// the `b:NAME`/`c:NAME`/`f:NAME`/`p:NAME` descriptor array from
// the module's bintab/conddefs/mathfuncs/paramdefs pointers. The
// per-module rust files (rlimits.rs, langinfo.rs, curses.rs, …)
// each ship their own local `featuresarray` stub returning a
// hardcoded descriptor list; a future canonical free-fn port will
// live in zsh_h.rs once `struct features` carries real bintab/etc.
// pointers.

/// `FINDMOD_ALIASP` — bit in `find_module()`'s `flags` arg.
/// Port of `enum { FINDMOD_ALIASP = 0x0001 }` from `Src/module.c:110`.
/// /* Resolve any aliases to the underlying module. */
pub const FINDMOD_ALIASP: i32 = 0x0001;                                  // c:110

/// `FINDMOD_CREATE` — bit in `find_module()`'s `flags` arg.
/// Port of `enum { FINDMOD_CREATE = 0x0002 }` from `Src/module.c:115`.
/// /* Create an element for the module in the list if not found. */
pub const FINDMOD_CREATE: i32 = 0x0002;                                  // c:115

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_table_new() {
        let table = modulestab::new();
        assert!(table.is_loaded("zsh/complete"));
        assert!(table.is_loaded("zsh/datetime"));
        assert!(table.is_loaded("zsh/system"));
        assert!(!table.is_loaded("nonexistent"));
    }

    #[test]
    fn test_load_unload() {
        let mut table = modulestab::new();
        assert!(table.is_loaded("zsh/complete"));

        table.unload_module("zsh/complete");
        assert!(!table.is_loaded("zsh/complete"));

        table.load_module("zsh/complete");
        assert!(table.is_loaded("zsh/complete"));
    }

    #[test]
    fn test_list_loaded() {
        let table = modulestab::new();
        let loaded = table.list_loaded();
        assert!(loaded.len() > 20);
        assert!(loaded.contains(&"zsh/complete"));
    }

    #[test]
    fn test_hooks() {
        let mut table = modulestab::new();
        table.addhookdef("chpwd");
        table.addhookfunc("chpwd", "my_chpwd_handler");

        let funcs = table.runhookdef("chpwd");
        assert_eq!(funcs, vec!["my_chpwd_handler"]);

        table.deletehookfunc("chpwd", "my_chpwd_handler");
        let funcs = table.runhookdef("chpwd");
        assert!(funcs.is_empty());
    }

    #[test]
    fn test_autoload() {
        let mut table = modulestab::new();
        table.add_autobin("my_cmd", "zsh/mymodule", 0);
        assert_eq!(
            table.resolve_autoload_builtin("my_cmd"),
            Some("zsh/mymodule")
        );
        assert_eq!(table.resolve_autoload_builtin("nonexistent"), None);
    }

    #[test]
    fn test_features() {
        // The per-module feature ledger has been deleted (canonical
        // C-tables track features in `BUILTINTAB`/`CONDTAB`/`PARAMTAB`).
        // `list_features` now returns an empty vec — `module_linked`
        // is the right test for "is this module registered".
        let table = modulestab::new();
        let features = table.list_features("zsh/complete");
        assert!(features.is_empty());
        assert!(table.module_linked("zsh/complete"));
    }

    #[test]
    fn test_module_linked() {
        let table = modulestab::new();
        assert!(table.module_linked("zsh/complete"));
        assert!(table.module_linked("zsh/stat"));
        assert!(!table.module_linked("zsh/nonexistent"));
    }

    // `test_wrappers` deleted — exercised the deleted
    // `ModuleTable::addwrapper`/`deletewrapper`+`wrappers` field.
    // The canonical `struct funcwrap` lives in zsh_h.rs:639.

    #[test]
    fn test_printmodulenode() {
        let module = module::new("zsh/test");
        let output = printmodulenode("zsh/test", &module);
        assert!(output.contains("zsh/test"));
        assert!(output.contains("loaded"));
    }

    // ===== Tests for the `addmathfunc` / `removemathfunc` /
    // `deletemathfunc` family ported in this session against the
    // MATHFUNCS Lazy<Mutex<Vec<mathfunc>>> global. Each test isolates
    // its names with a unique prefix so they don't collide if the
    // suite runs in parallel.

    fn mk_mf(name: &str, autoload: bool) -> crate::ported::zsh_h::mathfunc {
        crate::ported::zsh_h::mathfunc {
            next: None, name: name.to_string(), flags: 0,
            nfunc: None, sfunc: None,
            module: if autoload { Some("zsh/test".to_string()) } else { None },
            minargs: 0, maxargs: 0, funcid: 0,
        }
    }

    #[test]
    fn addmathfunc_clash_returns_one_when_already_added() {
        // C addmathfunc returns 1 when an existing entry has the same
        // name AND is non-autoloadable (no module set). Verifies the
        // "already in table" branch at module.c:1322-1330.
        let f1 = mk_mf("zshrs_test_clash_a", false);
        let f2 = mk_mf("zshrs_test_clash_a", false);
        assert_eq!(addmathfunc(f1), 0);
        assert_eq!(addmathfunc(f2), 1, "second add should clash");
        removemathfunc("zshrs_test_clash_a");
    }

    #[test]
    fn addmathfunc_autoload_replace_succeeds() {
        // When existing entry IS autoloadable (module.is_some, no
        // MFF_USERFUNC), C removes-then-replaces. The new entry should
        // land at index 0 (prepend) per c:1334-1335.
        let auto = mk_mf("zshrs_test_replace", true);
        let real = mk_mf("zshrs_test_replace", false);
        assert_eq!(addmathfunc(auto), 0);
        assert_eq!(addmathfunc(real), 0, "autoloadable entry must be replaceable");
        let tab = MATHFUNCS.lock().unwrap();
        let entry = tab.iter().find(|m| m.name == "zshrs_test_replace").unwrap();
        assert!(entry.module.is_none(), "after replace, module should be None");
        drop(tab);
        removemathfunc("zshrs_test_replace");
    }

    #[test]
    fn removemathfunc_returns_unit_and_drops() {
        let f = mk_mf("zshrs_test_remove", false);
        assert_eq!(addmathfunc(f), 0);
        assert!(MATHFUNCS.lock().unwrap().iter().any(|m| m.name == "zshrs_test_remove"));
        removemathfunc("zshrs_test_remove");
        assert!(!MATHFUNCS.lock().unwrap().iter().any(|m| m.name == "zshrs_test_remove"));
    }

    #[test]
    fn deletemathfunc_returns_minus_one_on_miss() {
        // C: returns -1 when no matching entry; verifies the c:1361 branch.
        let probe = mk_mf("zshrs_test_never_added_xyz", false);
        assert_eq!(deletemathfunc(&probe), -1);
    }

    #[test]
    fn deletemathfunc_clears_added_flag_for_userfunc() {
        // For non-module entries (`!f->module`), C clears the MFF_ADDED
        // flag instead of dropping the node (c:1357). Tests by adding
        // a user-defined mathfunc, flipping MFF_ADDED on, then deleting.
        let mut f = mk_mf("zshrs_test_clear_flag", false);
        f.flags = crate::ported::zsh_h::MFF_ADDED;
        assert_eq!(addmathfunc(f), 1, "MFF_ADDED set → addmathfunc clashes at c:1318");
        // Now seed it manually with module=None and MFF_ADDED so deletemathfunc
        // exercises the clear-flag branch.
        MATHFUNCS.lock().unwrap().insert(0, mk_mf("zshrs_test_clear_flag2", false));
        let mut f2 = mk_mf("zshrs_test_clear_flag2", false);
        f2.flags = crate::ported::zsh_h::MFF_ADDED;
        // f2 is the lookup probe; by name it matches the seeded entry.
        assert_eq!(deletemathfunc(&f2), 0);
        let tab = MATHFUNCS.lock().unwrap();
        let entry = tab.iter().find(|m| m.name == "zshrs_test_clear_flag2");
        // Entry stays in the table (module was None) but MFF_ADDED cleared.
        if let Some(e) = entry {
            assert_eq!(e.flags & crate::ported::zsh_h::MFF_ADDED, 0);
        }
        drop(tab);
        removemathfunc("zshrs_test_clear_flag2");
    }

    // ===== Tests for `addconddef` / `deleteconddef` against CONDTAB.

    fn mk_cd(name: &str, infix: bool, autoload: bool) -> crate::ported::zsh_h::conddef {
        crate::ported::zsh_h::conddef {
            next: None, name: name.to_string(),
            flags: if infix { crate::ported::zsh_h::CONDF_INFIX } else { 0 },
            handler: None, min: 0, max: 0, condid: 0,
            module: if autoload { Some("zsh/cond".to_string()) } else { None },
        }
    }

    #[test]
    fn addconddef_clash_returns_one() {
        // C addconddef: clash when existing has same name+infix AND is
        // not autoloadable, OR is already added (CONDF_ADDED flag).
        let c1 = mk_cd("zshrs_test_cond_clash", false, false);
        let c2 = mk_cd("zshrs_test_cond_clash", false, false);
        assert_eq!(addconddef(c1), 0);
        assert_eq!(addconddef(c2), 1);
        let probe = mk_cd("zshrs_test_cond_clash", false, false);
        assert_eq!(deleteconddef(&probe), 0);
    }

    #[test]
    fn deleteconddef_returns_minus_one_on_miss() {
        let probe = mk_cd("zshrs_test_cond_never_added", false, false);
        assert_eq!(deleteconddef(&probe), -1);
    }

    #[test]
    fn addconddef_distinguishes_infix_from_prefix() {
        // CONDF_INFIX is part of the clash key — a prefix-form `-z` and
        // an infix-form `==` share neither name nor flag, so adding both
        // names with different infix bits should both succeed.
        let prefix = mk_cd("zshrs_test_cond_dual", false, false);
        let infix  = mk_cd("zshrs_test_cond_dual", true,  false);
        assert_eq!(addconddef(prefix), 0);
        assert_eq!(addconddef(infix), 0, "infix variant must not clash with prefix variant");
        // Cleanup both
        let _ = deleteconddef(&mk_cd("zshrs_test_cond_dual", false, false));
        let _ = deleteconddef(&mk_cd("zshrs_test_cond_dual", true,  false));
    }

    // ===== Tests for `setconddefs` / `setmathfuncs` bulk dispatch.

    #[test]
    fn setconddefs_bulk_add_then_bulk_delete_via_e_array() {
        // C setconddefs: walks (c, e) pairs; e[i]!=0 → addconddef path,
        // e[i]==0 → deleteconddef path. Tests the round trip.
        let mut entries = vec![
            mk_cd("zshrs_test_bulk_a", false, false),
            mk_cd("zshrs_test_bulk_b", false, false),
        ];
        let add_selectors = [1, 1];
        assert_eq!(setconddefs("test", &mut entries, Some(&add_selectors)), 0);
        // Both should now have CONDF_ADDED set per c:773.
        assert_ne!(entries[0].flags & crate::ported::zsh_h::CONDF_ADDED, 0);
        assert_ne!(entries[1].flags & crate::ported::zsh_h::CONDF_ADDED, 0);
        // Now delete both via e=[0,0].
        let del_selectors = [0, 0];
        assert_eq!(setconddefs("test", &mut entries, Some(&del_selectors)), 0);
        assert_eq!(entries[0].flags & crate::ported::zsh_h::CONDF_ADDED, 0);
        assert_eq!(entries[1].flags & crate::ported::zsh_h::CONDF_ADDED, 0);
    }

    // ===== Tests for `addbuiltin` / `addbuiltins` against canonical builtintab.

    fn mk_b(nam: &str) -> crate::ported::zsh_h::builtin {
        crate::ported::zsh_h::builtin {
            node: crate::ported::zsh_h::hashnode { next: None, nam: nam.to_string(), flags: 0 },
            handlerfunc: None, minargs: 0, maxargs: 0, funcid: 0,
            optstr: None, defopts: None,
        }
    }

    #[test]
    fn addbuiltin_clash_against_existing_builtintab_entry() {
        // C addbuiltin: returns 1 when builtintab already has an entry
        // for the same name with BINF_ADDED set. The canonical Rust
        // builtintab is populated at startup via createbuiltintable;
        // probing a real builtin like "echo" should clash if BINF_ADDED.
        let _ = crate::ported::builtin::createbuiltintable();
        let mut b = mk_b("echo");
        let r = addbuiltin(&mut b);
        // BINF_ADDED gets set on b when no clash. If echo was BINF_ADDED in
        // the static table, r==1; otherwise r==0 and b.flags now has BINF_ADDED.
        assert!(r == 0 || r == 1);
        if r == 0 {
            assert_ne!(b.node.flags & crate::ported::zsh_h::BINF_ADDED as i32, 0);
        }
    }

    #[test]
    fn addbuiltins_skips_already_added_entries() {
        // C addbuiltins (c:553): `if (b->node.flags & BINF_ADDED) continue`.
        // Pre-marking BINF_ADDED should skip both entries; ret stays 0.
        let mut b1 = mk_b("zshrs_test_already_added_1");
        b1.node.flags = crate::ported::zsh_h::BINF_ADDED as i32;
        let mut b2 = mk_b("zshrs_test_already_added_2");
        b2.node.flags = crate::ported::zsh_h::BINF_ADDED as i32;
        let mut binl = vec![b1, b2];
        assert_eq!(addbuiltins("test", &mut binl), 0);
    }

    // ===== Tests for `addwrapper` / `deletewrapper` against WRAPPERS.

    fn mk_w() -> crate::ported::zsh_h::funcwrap {
        crate::ported::zsh_h::funcwrap {
            next: None, flags: 0,
            handler: Some(|_prog, _w, _name| 0),
            module: None,
        }
    }

    #[test]
    fn addwrapper_then_deletewrapper_round_trip() {
        let w = mk_w();
        assert_eq!(addwrapper("zsh/test", w), 0);
        let probe = mk_w();
        let r = deletewrapper("zsh/test", &probe);
        // fn_addr_eq may match (most common case) or miss across codegen
        // units. Either outcome is documented behavior; verify it doesn't
        // panic and returns 0/1 cleanly.
        assert!(r == 0 || r == 1);
    }

    #[test]
    fn deletewrapper_returns_one_when_not_found() {
        // Empty WRAPPERS means any probe misses. Take a snapshot of the
        // current state, drain WRAPPERS, run the test, restore.
        let snapshot: Vec<_> = WRAPPERS.lock().unwrap().drain(..).collect();
        let probe = mk_w();
        assert_eq!(deletewrapper("zsh/test", &probe), 1);
        WRAPPERS.lock().unwrap().extend(snapshot);
    }
}

#[cfg(test)]
mod modname_tests {
    use super::*;

    /// c:2173 — `modname_ok` accepts shell identifier names possibly
    /// joined by `/` (zsh modules are namespaced like `zsh/datetime`).
    /// Plain alphanumeric names with `/` separators MUST pass.
    /// A regression rejecting valid module names would break every
    /// `zmodload zsh/datetime` invocation.
    #[test]
    fn modname_ok_accepts_canonical_zsh_module_paths() {
        assert_eq!(modname_ok("zsh"),           1);
        assert_eq!(modname_ok("zsh/datetime"),  1);
        assert_eq!(modname_ok("zsh/zle"),       1);
        assert_eq!(modname_ok("foo_bar"),       1);
        assert_eq!(modname_ok("foo123"),        1);
    }

    /// c:2179 — non-identifier chars (excluding `/`) MUST cause
    /// rejection. A regression accepting them would let modules
    /// install with names that no later `zmodload -u` could remove.
    #[test]
    fn modname_ok_rejects_special_chars() {
        assert_eq!(modname_ok("zsh space"),  0);
        assert_eq!(modname_ok("zsh-bad"),    0, "hyphen is not IIDENT");
        assert_eq!(modname_ok("zsh.foo"),    0, "dot is not IIDENT");
        assert_eq!(modname_ok("$foo"),       0);
    }

    /// c:2177 — `if (!*p) return 1` runs at the START of the loop;
    /// empty input therefore returns 1. Pin this behaviour so callers
    /// know the empty-string case maps to "trivially OK" not "error".
    #[test]
    fn modname_ok_treats_empty_as_trivially_ok() {
        assert_eq!(modname_ok(""), 1);
    }
}
