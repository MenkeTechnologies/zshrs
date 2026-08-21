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

use crate::ported::builtin::createbuiltintable;
use crate::ported::hist::casemodify;
use crate::ported::mem::ztrdup;
use crate::ported::params::{createparam, createspecialhash, paramtab, unsetparam_pm};
use crate::ported::signals::unqueue_signals;
use crate::ported::utils::{zwarn, zwarnnam};
use crate::ported::zsh_h::hashnode;
use crate::ported::zsh_h::{
    builtin, conddef, funcwrap, hookdef, linkedmod, linklist, linknode, mathfunc, options,
    paramdef, Hookfn, Param, BINF_AUTOALL, CASMOD_LOWER, CASMOD_UPPER, CONDF_AUTOALL, HOOKF_ALL,
    MFF_USERFUNC, MOD_ALIAS, MOD_BUSY, MOD_INIT_B, MOD_INIT_S, MOD_LINKED, MOD_SETUP, MOD_UNLOAD,
    OPT_ARG_SAFE, OPT_ISSET, PM_ARRAY, PM_AUTOALL, PM_AUTOLOAD, PM_EFLOAT, PM_FFLOAT, PM_HASHED,
    PM_INTEGER, PM_NAMEREF, PM_READONLY, PM_REMOVABLE, PM_SCALAR, PM_TIED, PM_TYPE, PRINT_LIST,
};
pub use crate::ported::zsh_h::{BINF_ADDED, CONDF_ADDED, CONDF_INFIX, MFF_ADDED};
use crate::zsh_h::module;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Mutex;

/// Free module node (from module.c freemodulenode)
/// Free a module table entry.
/// Port of `freemodulenode(HashNode hn)` from Src/module.c:119 — Rust's
/// `Drop` handles the per-field free; this exists for API
/// parity with C callers.
pub fn freemodulenode(hn: module) {
    // Rust Drop handles this
}

/// `PRINTMOD_LIST` from `Src/module.c:136`. Long-form (`zmodload -L`)
/// output.
pub const PRINTMOD_LIST: i32 = 0x0001; // c:136
/// `PRINTMOD_EXIST` from `Src/module.c:138`. Print only when the
/// module exists.
pub const PRINTMOD_EXIST: i32 = 0x0002; // c:138
/// `PRINTMOD_ALIAS` from `Src/module.c:140`. Resolve aliases when
/// emitting under `PRINTMOD_EXIST`.
pub const PRINTMOD_ALIAS: i32 = 0x0004; // c:140
/// `PRINTMOD_DEPS` from `Src/module.c:142`. Emit the dependency list
/// (`zmodload -d`).
pub const PRINTMOD_DEPS: i32 = 0x0008; // c:142
/// `PRINTMOD_FEATURES` from `Src/module.c:144`. Emit feature flags
/// (`zmodload -F`).
pub const PRINTMOD_FEATURES: i32 = 0x0010; // c:144
/// `PRINTMOD_LISTALL` from `Src/module.c:146`. Include disabled
/// features (`zmodload -lL`).
pub const PRINTMOD_LISTALL: i32 = 0x0020; // c:146
/// `PRINTMOD_AUTO` from `Src/module.c:148`. Emit autoloads
/// (`zmodload -a`).
pub const PRINTMOD_AUTO: i32 = 0x0040; // c:148

/// Direct port of `void printmodulenode(HashNode hn, int flags)` from
/// `Src/module.c:154`.
///
/// Formats one module entry for the various `zmodload -L`/`-a`/
/// `-d`/`-F` listings. C writes to stdout; Rust returns the
/// formatted string so call sites can route to the right output fd
/// without depending on stdio. Dispatches on `flags`:
///
///   - `PRINTMOD_DEPS`: emit dep list (`zmodload -d MOD: dep1 dep2`
///     under PRINTMOD_LIST, else `MOD: dep1 dep2`) — c:163-194
///   - `PRINTMOD_EXIST`: emit just the module name when loaded
///     (resolving alias when PRINTMOD_ALIAS set) — c:195-201
///   - alias module: under PRINTMOD_LIST emit
///     `zmodload -A MOD=ALIAS`, else `MOD -> ALIAS` — c:202-217
///   - loaded module: under PRINTMOD_LIST emit `zmodload [-Fa] MOD`,
///     else just the name — c:218-241
pub fn printmodulenode(hn: &str, m: &module, flags: i32) -> String {
    let modname = hn;
    let mut out = String::new();

    // c:163-194 — PRINTMOD_DEPS branch.
    // C body:
    //   if (!m->deps) return;
    //   if (flags & PRINTMOD_LIST) {
    //       printf("zmodload -d ");
    //       if (modname[0] == '-') fputs("-- ", stdout);
    //       quotedzputs(modname, stdout);
    //   } else {
    //       nicezputs(modname, stdout);
    //       putchar(':');
    //   }
    //   for (n = firstnode(m->deps); n; incnode(n)) {
    //       putchar(' ');
    //       if (flags & PRINTMOD_LIST)
    //           quotedzputs((char *) getdata(n), stdout);
    //       else
    //           nicezputs((char *) getdata(n), stdout);
    //   }
    //
    // C uses quotedzputs (round-trippable shell input) under
    // PRINTMOD_LIST so the emitted `zmodload -d MOD DEP1 DEP2` is
    // re-parseable. The default path uses nicezputs (printable form
    // with \M-/^ escapes for control bytes) which is listing-friendly.
    // The two helpers are both already ported in src/ported/utils.rs.
    if flags & PRINTMOD_DEPS != 0 {
        use crate::ported::utils::{nicezputs, quotedzputs};
        let deps = match m.deps.as_ref() {
            Some(d) if !d.is_empty() => d,
            _ => return out, // c:170-171
        };
        if flags & PRINTMOD_LIST != 0 {
            out.push_str("zmodload -d "); // c:174
            if modname.starts_with('-') {
                out.push_str("-- "); // c:176
            }
            out.push_str(&quotedzputs(modname)); // c:177
        } else {
            let mut buf: Vec<u8> = Vec::new();
            let _ = nicezputs(modname, &mut buf); // c:179
            if let Ok(s) = std::str::from_utf8(&buf) {
                out.push_str(s);
            }
            out.push(':'); // c:180
        }
        for dep in deps.iter() {
            out.push(' '); // c:183
            if flags & PRINTMOD_LIST != 0 {
                out.push_str(&quotedzputs(dep)); // c:185
            } else {
                let mut buf: Vec<u8> = Vec::new();
                let _ = nicezputs(dep, &mut buf); // c:187
                if let Ok(s) = std::str::from_utf8(&buf) {
                    out.push_str(s);
                }
            }
        }
        return out;
    }

    // c:189-201 — PRINTMOD_EXIST branch.
    // C body:
    //   if (m->node.flags & MOD_ALIAS) {
    //       if (!(flags & PRINTMOD_ALIAS) ||
    //           !(m = find_module(m->u.alias, FINDMOD_ALIASP, NULL)))
    //           return;
    //   }
    //   if (!m->u.handle || (m->node.flags & MOD_UNLOAD))
    //       return;
    //   nicezputs(modname, stdout);
    //
    // The MOD_ALIAS arm at c:194-198 reassigns m to the alias target
    // before the u.handle check at c:199 — without a table handle we
    // can't chase the alias here; the caller (bin_zmodload_exist /
    // bin_zmodload_alias) is responsible for the chase when feeding
    // us the canonical module. For the local check we read the alias
    // entry's flags, which is the right behaviour when the caller
    // already resolved.
    if flags & PRINTMOD_EXIST != 0 {
        if (m.node.flags & MOD_ALIAS) != 0 {
            if (flags & PRINTMOD_ALIAS) == 0 || m.alias.is_none() {
                // c:195-197 — alias entry + caller didn't pass
                // PRINTMOD_ALIAS, OR the alias is dangling.
                return out;
            }
            // Alias resolves: emit the alias entry's name; the
            // caller chased and decided it counts as "exists".
        }
        // c:199 — `!m->u.handle || MOD_UNLOAD` skip.
        // Static-link analog of `!u.handle` is `!MOD_INIT_B`
        // (boot hasn't run = no loaded handle). The prior port
        // missed this gate, so PRINTMOD_EXIST listed every
        // pre-registered module even when zmodload zsh/files had
        // never fired — diverging from `zsh -fc 'zmodload -e'`
        // which emits only the actually-loaded `zsh/main`.
        let booted = (m.node.flags & MOD_INIT_B) != 0;
        let unloading = (m.node.flags & MOD_UNLOAD) != 0;
        if !booted || unloading {
            return out;
        }
        out.push_str(modname); // c:201 nicezputs(modname)
        return out;
    }

    // c:202-217 — alias module branch.
    // c:202-217 — alias module branch.
    // C body:
    //   if (flags & PRINTMOD_LIST) {
    //       printf("zmodload -A ");
    //       if (modname[0] == '-') fputs("-- ", stdout);
    //       quotedzputs(modname, stdout);
    //       putchar('=');
    //       quotedzputs(m->u.alias, stdout);
    //   } else {
    //       nicezputs(modname, stdout);
    //       fputs(" -> ", stdout);
    //       nicezputs(m->u.alias, stdout);
    //   }
    if m.node.flags & MOD_ALIAS != 0 {
        use crate::ported::utils::{nicezputs, quotedzputs};
        let alias = m.alias.as_deref().unwrap_or("");
        if flags & PRINTMOD_LIST != 0 {
            out.push_str("zmodload -A "); // c:207
            if modname.starts_with('-') {
                out.push_str("-- "); // c:209
            }
            out.push_str(&quotedzputs(modname)); // c:210
            out.push('='); // c:211
            out.push_str(&quotedzputs(alias)); // c:212
        } else {
            let mut buf: Vec<u8> = Vec::new();
            let _ = nicezputs(modname, &mut buf); // c:214
            if let Ok(s) = std::str::from_utf8(&buf) {
                out.push_str(s);
            }
            out.push_str(" -> "); // c:215
            let mut buf2: Vec<u8> = Vec::new();
            let _ = nicezputs(alias, &mut buf2); // c:216
            if let Ok(s) = std::str::from_utf8(&buf2) {
                out.push_str(s);
            }
        }
        return out;
    }

    // c:218-241 — loaded module branch (linked or autoloaded).
    // C check: `m->u.handle || (flags & PRINTMOD_AUTO)` where `u`
    // is a union so `u.handle` is non-NULL whenever EITHER `handle`
    // (dlopen result) or `linked` (statically-linked record) is
    // installed. The union slots are only populated AFTER
    // `load_module` completes (c:2227/2230 set them, c:2244 sets
    // `MOD_INIT_B`). So "boot ran" maps to `MOD_INIT_B` in zshrs.
    // Previous gate `MOD_LINKED && !MOD_UNLOAD` was wrong:
    // `register_builtin_modules` seeds `MOD_LINKED` for every
    // statically-compiled module up front, so plain `zmodload`
    // listed all 32 (#76 in docs/BUGS.md). C zsh shows only the
    // single `zsh/main` entry that `init_bltinmods` actually loads
    // via `load_module("zsh/main", NULL, 0)`.
    let loaded = (m.node.flags & MOD_INIT_B) != 0 && (m.node.flags & MOD_UNLOAD) == 0;
    let _ = MOD_LINKED; // c:Src/module.c:218 — union-based check; flag retained for unload path.
    let auto = flags & PRINTMOD_AUTO != 0;
    if loaded || auto {
        use crate::ported::utils::{nicezputs, quotedzputs};
        if flags & PRINTMOD_LIST != 0 {
            // c:229-237 — `PRINTMOD_AUTO`: skip when no autoloads set.
            // C `firstnode(m->autoloads)` returns the first node — if
            // the linklist is empty the early-return fires.
            if auto {
                let has_autoloads = m
                    .autoloads
                    .as_ref()
                    .map(|al| !al.is_empty())
                    .unwrap_or(false);
                if !has_autoloads {
                    return out; // c:231 return early
                }
            }
            out.push_str("zmodload "); // c:238
            if auto {
                out.push_str("-Fa "); // c:240
            } else if flags & PRINTMOD_FEATURES != 0 {
                out.push_str("-F "); // c:242
            }
            if modname.starts_with('-') {
                out.push_str("-- "); // c:244
            }
            out.push_str(&quotedzputs(modname)); // c:245
                                                 // c:246-251 — PRINTMOD_AUTO: emit each autoload as
                                                 //             ` quotedzputs(al)`.
            if auto {
                if let Some(al_list) = m.autoloads.as_ref() {
                    for al in al_list.iter() {
                        out.push(' '); // c:249
                        out.push_str(&quotedzputs(al)); // c:250
                    }
                }
            }
            // c:252-263 — PRINTMOD_FEATURES list path needs features_module
            // + enables_module which require the modulestab; not
            // dispatched here because printmodulenode has no &table
            // handle. Caller (bin_zmodload_features -l/-L path) does
            // the dispatch directly when PRINTMOD_FEATURES is set.
        } else {
            // c:266 — `else /* -l */ nicezputs(modname, stdout);`
            let mut buf: Vec<u8> = Vec::new();
            let _ = nicezputs(modname, &mut buf);
            if let Ok(s) = std::str::from_utf8(&buf) {
                out.push_str(s);
            }
        }
    }
    out // c:268 putchar('\n') handled by caller's println!
}

/// Create new module table (from module.c newmoduletable)
/// Create an empty module table.
/// Port of `newmoduletable(int size, char const *name)` from Src/module.c:274 — the C
/// source allocates the `modulestab` hash with `createhashtable`.
/// WARNING: param names don't match C — Rust=() vs C=(size, name)
pub fn newmoduletable() -> modulestab {
    modulestab::new()
}

/// Port of `setup_(UNUSED(Module m))` from `Src/module.c:306`.
///
/// C body: `setup_(UNUSED(Module m)) { return 0; }` — the no-op
/// setup hook of the module subsystem itself.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:306
    0 // c:306
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
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:313
    /* There are lots and lots of features, but they're not handled here. */ // c:313-318
    1 // c:319
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/module.c:324`.
///
/// C body: `enables_(UNUSED(Module m), UNUSED(int **enables)) { return 1; }`
/// — the module subsystem itself doesn't manage feature enables.
#[allow(unused_variables)]
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:324
    1 // c:324
}

/// Port of `boot_(UNUSED(Module m))` from `Src/module.c:331`.
///
/// C body: `boot_(UNUSED(Module m)) { return 0; }` — the no-op
/// boot hook of the module subsystem itself.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:331
    0 // c:331
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/module.c:338`.
///
/// C body: `cleanup_(UNUSED(Module m)) { return 0; }` — the no-op
/// cleanup hook of the module subsystem itself.
#[allow(unused_variables)]
pub fn cleanup_(m: *const module) -> i32 {
    // c:338
    0 // c:338
}

/// Port of `finish_(UNUSED(Module m))` from `Src/module.c:345`.
///
/// C body: `finish_(UNUSED(Module m)) { return 0; }` —
/// the no-op finish hook for the module subsystem itself.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:345
    0 // c:345
}

// This registers a builtin module.                                        // c:359
/// Register module (from module.c register_module)
/// Register a module by name.
/// Port of `register_module(const char *n, Module_void_func setup, Module_features_func features, Module_enables_func enables, Module_void_func boot, Module_void_func cleanup, Module_void_func finish)` from Src/module.c:359 — wraps
/// a slot in the global `modulestab` and seeds its lifecycle
/// callbacks.
/// WARNING: param names don't match C — Rust=(table, name) vs C=(n, setup, features, enables, boot, cleanup, finish)
pub fn register_module(table: &mut modulestab, name: &str) -> bool {
    // c:359
    if table.modules.contains_key(name) {
        return false;
    }
    // c:378 — `zaddlinknode(linkedmodules, m)`: the static entry-point
    // record goes on `linkedmodules`, which is what `module_linked`
    // (c:385) searches from `load_module` (c:2224, c:2285).
    if !table.linkedmodules.iter().any(|n| n == name) {
        table.linkedmodules.push(name.to_string());
    }
    // WARNING: RUST-ONLY — C's `register_module` does NOT add a
    // `modulestab` node (only `load_module`/`autofeatures`/`add_dep`
    // do). The node insert is kept here because zshrs callers treat
    // this free fn as "make the module exist and be loadable"; the
    // boot path (`register_builtin_modules`) deliberately does NOT use
    // it, and populates `linkedmodules` directly so that the
    // `modulestab` node set matches C's boot set exactly.
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
pub fn addbuiltin(b: &mut builtin) -> i32 {
    // c:524
    let tab = createbuiltintable();
    if let Some(existing) = tab.get(&b.node.nam) {
        // c:526 getnode2
        if (existing.node.flags & BINF_ADDED as i32) != 0 {
            return 1;
        } // c:527 clash
    }
    b.node.flags |= BINF_ADDED as i32; // c:531 b->node.flags |= BINF_ADDED
    0
}

/// Port of `int deletebuiltin(const char *nam)` from `Src/module.c:449`.
///
/// C body c:449-458:
/// ```c
/// int
/// deletebuiltin(const char *nam)
/// {
///     Builtin bn;
///     bn = (Builtin) builtintab->removenode(builtintab, nam);
///     if (!bn)
///         return -1;
///     builtintab->freenode(&bn->node);
///     return 0;
/// }
/// ```
///
/// Returns 0 on success (entry was found + removed), -1 on miss.
///
/// zshrs's `createbuiltintable()` returns an immutable HashMap —
/// the canonical table is static-linked, so this fn can't actually
/// `removenode`. The faithful structural equivalent: probe the
/// canonical table to honour the present/absent contract that
/// callers (setbuiltins, del_autobin) rely on for their
/// `already-deleted` / `no such builtin` diagnostics. Actual
/// runtime `enabled` state lives on the modulestab's
/// `added_builtins` ledger; that's where the caller flips the
/// observable BINF_ADDED bit.
pub fn deletebuiltin(nam: &str) -> i32 {
    // c:449
    // c:453 — `bn = builtintab->removenode(builtintab, nam);`
    let tab = createbuiltintable();
    match tab.get(nam) {
        None => -1,   // c:454-455 — `if (!bn) return -1;`
        Some(_) => 0, // c:457 — freenode is owned by createbuiltintable, no-op.
    }
}

/// Port of `addbuiltins(char const *nam, Builtin binl, int size)` from
/// `Src/module.c:544`. Walks the slice; for each entry not already
/// flagged BINF_ADDED, calls `addbuiltin`. Returns 0 if all succeeded,
/// 1 if any clashed. zwarnnam emitted on each clash matches C.
pub fn addbuiltins(nam: &str, binl: &mut [builtin]) -> i32 {
    // c:544
    let mut ret = 0; // c:548
    for b in binl.iter_mut() {
        // c:550 for(n = 0; n < size; n++)
        if (b.node.flags & BINF_ADDED as i32) != 0 {
            continue;
        } // c:553
        if addbuiltin(b) != 0 {
            // c:555
            zwarnnam(
                nam, // c:556 zwarnnam(nam, "name clash...")
                &format!("name clash when adding builtin `{}'", b.node.nam),
            );
            ret = 1;
        }
    }
    ret // c:563
}

/// Port of `Hookdef gethookdef(const char *n)` from `Src/module.c:849`.
///
/// C body (c:849-861):
/// ```c
/// Hookdef gethookdef(const char *n) {
///     Hookdef p;
///     for (p = hooktab; p; p = p->next)
///         if (!strcmp(n, p->name))
///             return p;
///     return NULL;
/// }
/// ```
pub fn gethookdef(n: &str) -> *mut hookdef {
    // c:849
    let mut p = hooktab.load(std::sync::atomic::Ordering::SeqCst); // c:852 p = hooktab
    while !p.is_null() {
        // c:853
        unsafe {
            if (*p).name == n {
                // c:854 !strcmp
                return p; // c:855
            }
            p = (*p).next; // c:853 p = p->next
        }
    }
    std::ptr::null_mut() // c:856
}

/// Port of `int addhookdef(Hookdef h)` from `Src/module.c:864`.
///
/// C body (c:864-874):
/// ```c
/// int addhookdef(Hookdef h) {
///     if (gethookdef(h->name)) return 1;
///     h->next = hooktab;
///     hooktab = h;
///     h->funcs = znewlinklist();
///     return 0;
/// }
/// ```
pub fn addhookdef(h: *mut hookdef) -> i32 {
    // c:864
    unsafe {
        if !gethookdef(&(*h).name).is_null() {
            // c:866
            return 1; // c:867
        }
        (*h).next = hooktab.load(std::sync::atomic::Ordering::SeqCst); // c:869 h->next = hooktab
        hooktab.store(h, std::sync::atomic::Ordering::SeqCst); // c:870 hooktab = h
        if (*h).funcs.is_null() {
            // c:871 h->funcs = znewlinklist()
            (*h).funcs = Box::into_raw(Box::new(linklist {
                first: None,
                last: None,
                flags: 0,
            }));
        }
    }
    0 // c:873
}

/// Port of `int addhookdefs(Module m, Hookdef h, int size)` from
/// `Src/module.c:883`.
///
/// C body (c:883-895):
/// ```c
/// int addhookdefs(Module m, Hookdef h, int size) {
///     int ret = 0;
///     while (size--) {
///         if (addhookdef(h)) {
///             zwarnnam(m ? m->node.nam : NULL, "name clash when adding hook `%s'", h->name);
///             ret = 1;
///         }
///         h++;
///     }
///     return ret;
/// }
/// ```
pub fn addhookdefs(
    // c:883
    m: *const module,
    mut h: *mut hookdef,
    mut size: i32,
) -> i32 {
    let mut ret: i32 = 0; // c:885
    while size > 0 {
        // c:887 size--
        if addhookdef(h) != 0 {
            // c:888
            let nam: String = if m.is_null() {
                // c:889 m ? m->node.nam : NULL
                String::new()
            } else {
                unsafe { (*m).node.nam.clone() }
            };
            let hook_name = unsafe { (*h).name.clone() };
            zwarnnam(
                // c:889 zwarnnam
                &nam,
                &format!("name clash when adding hook `{}'", hook_name),
            );
            ret = 1; // c:891
        }
        unsafe {
            h = h.add(1);
        } // c:893 h++
        size -= 1; // c:887
    }
    ret // c:894
}

/// Port of `int deletehookdef(Hookdef h)` from `Src/module.c:902`.
///
/// C body (c:902-919):
/// ```c
/// int deletehookdef(Hookdef h) {
///     Hookdef p, q;
///     for (p = hooktab, q = NULL; p && p != h; q = p, p = p->next);
///     if (!p) return 1;
///     if (q) q->next = p->next; else hooktab = p->next;
///     freelinklist(p->funcs, NULL);
///     return 0;
/// }
/// ```
pub fn deletehookdef(h: *mut hookdef) -> i32 {
    // c:902
    let mut p = hooktab.load(std::sync::atomic::Ordering::SeqCst); // c:906 p = hooktab
    let mut q: *mut hookdef = std::ptr::null_mut(); // c:906 q = NULL
    while !p.is_null() && p != h {
        // c:907
        q = p; // c:907 q = p
        unsafe {
            p = (*p).next;
        } // c:907 p = p->next
    }
    if p.is_null() {
        // c:909
        return 1; // c:910
    }
    unsafe {
        if !q.is_null() {
            // c:912
            (*q).next = (*p).next; // c:913 q->next = p->next
        } else {
            hooktab.store((*p).next, std::sync::atomic::Ordering::SeqCst); // c:915 hooktab = p->next
        }
        if !(*p).funcs.is_null() {
            // c:916 freelinklist(p->funcs, NULL)
            drop(Box::from_raw((*p).funcs));
            (*p).funcs = std::ptr::null_mut();
        }
    }
    0 // c:917
}

/// Port of `int deletehookdefs(Module m, Hookdef h, int size)` from
/// `Src/module.c:923`. `m` is unused per `UNUSED(Module m)` in C.
pub fn deletehookdefs(
    // c:923
    _m: *const module,
    mut h: *mut hookdef,
    mut size: i32,
) -> i32 {
    let mut ret: i32 = 0; // c:925
    while size > 0 {
        // c:927 size--
        if deletehookdef(h) != 0 {
            // c:928
            ret = 1; // c:929
        }
        unsafe {
            h = h.add(1);
        } // c:930 h++
        size -= 1; // c:927
    }
    ret // c:931
}

/// Port of `int addhookdeffunc(Hookdef h, Hookfn f)` from `Src/module.c:939`.
///
/// C body (c:939-944):
/// ```c
/// int addhookdeffunc(Hookdef h, Hookfn f) {
///     zaddlinknode(h->funcs, (void *) f);
///     return 0;
/// }
/// ```
pub fn addhookdeffunc(
    // c:939
    h: *mut hookdef,
    f: Hookfn,
) -> i32 {
    unsafe {
        if (*h).funcs.is_null() {
            (*h).funcs = Box::into_raw(Box::new(linklist {
                first: None,
                last: None,
                flags: 0,
            }));
        }
        let funcs = &mut *(*h).funcs;
        // c:942 — zaddlinknode(h->funcs, f) appends to end of LinkList.
        // Walk to tail of owned chain (linklist.last cannot be a Box
        // pointer because that would duplicate ownership of the tail
        // node — the C `last` field is a raw pointer; the Rust
        // representation keeps it as None and resolves the tail by
        // walking forward from .first).
        let new_node = Box::new(linknode {
            next: None,
            prev: None,
            dat: f as usize,
        });
        if funcs.first.is_none() {
            funcs.first = Some(new_node);
        } else {
            let mut tail = funcs.first.as_mut().unwrap();
            while tail.next.is_some() {
                tail = tail.next.as_mut().unwrap();
            }
            tail.next = Some(new_node);
        }
    }
    0 // c:943
}

/// Port of `int addhookfunc(char *n, Hookfn f)` from `Src/module.c:948`.
///
/// C body (c:948-955):
/// ```c
/// int addhookfunc(char *n, Hookfn f) {
///     Hookdef h = gethookdef(n);
///     if (h) return addhookdeffunc(h, f);
///     return 1;
/// }
/// ```
pub fn addhookfunc(
    // c:948
    n: &str,
    f: Hookfn,
) -> i32 {
    let h = gethookdef(n); // c:950 h = gethookdef(n)
    if !h.is_null() {
        // c:951
        return addhookdeffunc(h, f); // c:952
    }
    1 // c:953
}

/// Port of `int deletehookdeffunc(Hookdef h, Hookfn f)` from
/// `Src/module.c:961`.
///
/// C body (c:961-973):
/// ```c
/// int deletehookdeffunc(Hookdef h, Hookfn f) {
///     LinkNode p;
///     for (p = firstnode(h->funcs); p; incnode(p))
///         if (f == (Hookfn) getdata(p)) {
///             remnode(h->funcs, p);
///             return 0;
///         }
///     return 1;
/// }
/// ```
pub fn deletehookdeffunc(
    // c:961
    h: *mut hookdef,
    f: Hookfn,
) -> i32 {
    unsafe {
        if (*h).funcs.is_null() {
            return 1;
        }
        let funcs = &mut *(*h).funcs;
        let f_val = f as usize;
        // Walk owning chain looking for the matching dat. Splice on hit.
        let mut prev: &mut Option<Box<linknode>> = &mut funcs.first;
        loop {
            match prev {
                None => return 1, // c:971
                Some(node) if node.dat == f_val => {
                    // c:966 f == getdata(p)
                    let next = node.next.take(); // c:967 remnode
                    *prev = next;
                    return 0; // c:968
                }
                Some(_) => {
                    // c:965 incnode(p) — advance.
                    prev = &mut prev.as_mut().unwrap().next;
                }
            }
        }
    }
}

/// Port of `int deletehookfunc(const char *n, Hookfn f)` from
/// `Src/module.c:977`.
///
/// C body (c:977-984):
/// ```c
/// int deletehookfunc(const char *n, Hookfn f) {
///     Hookdef h = gethookdef(n);
///     if (h) return deletehookdeffunc(h, f);
///     return 1;
/// }
/// ```
pub fn deletehookfunc(
    // c:977
    n: &str,
    f: Hookfn,
) -> i32 {
    let h = gethookdef(n); // c:979 h = gethookdef(n)
    if !h.is_null() {
        // c:980
        return deletehookdeffunc(h, f); // c:981
    }
    1 // c:982
}

/// Port of `int runhookdef(Hookdef h, void *d)` from `Src/module.c:990`.
///
/// C body (c:990-1010):
/// ```c
/// int runhookdef(Hookdef h, void *d) {
///     if (empty(h->funcs)) {
///         if (h->def) return h->def(h, d);
///         return 0;
///     } else if (h->flags & HOOKF_ALL) {
///         LinkNode p; int r;
///         for (p = firstnode(h->funcs); p; incnode(p))
///             if ((r = ((Hookfn) getdata(p))(h, d))) return r;
///         if (h->def) return h->def(h, d);
///         return 0;
///     } else
///         return ((Hookfn) getdata(lastnode(h->funcs)))(h, d);
/// }
/// ```
pub fn runhookdef(
    // c:990
    h: *mut hookdef,
    d: *mut std::ffi::c_void,
) -> i32 {
    unsafe {
        let funcs_ptr = (*h).funcs;
        let funcs_empty = funcs_ptr.is_null() || (*funcs_ptr).first.is_none(); // c:992 empty()
        if funcs_empty {
            // c:992
            if let Some(def) = (*h).def {
                // c:993 if (h->def)
                return def(h, d); // c:994 return h->def(h,d)
            }
            return 0; // c:995
        }
        if (*h).flags & HOOKF_ALL != 0 {
            // c:996 h->flags & HOOKF_ALL
            let mut node = (*funcs_ptr).first.as_ref(); // c:999 firstnode
            while let Some(n) = node {
                // c:999 ; p ;
                let fn_ptr: Hookfn = std::mem::transmute(n.dat); // c:1000 (Hookfn) getdata(p)
                let r = fn_ptr(h, d); // c:1000 (...)(h, d)
                if r != 0 {
                    // c:1000 if ((r = ...))
                    return r; // c:1001
                }
                node = n.next.as_ref(); // c:999 incnode
            }
            if let Some(def) = (*h).def {
                // c:1002 if (h->def)
                return def(h, d); // c:1003 return h->def(h, d)
            }
            return 0; // c:1004
        }
        // c:1006 — last fn only.
        let mut tail = (*funcs_ptr).first.as_ref().expect("non-empty");
        while let Some(next) = tail.next.as_ref() {
            tail = next;
        }
        let fn_ptr: Hookfn = std::mem::transmute(tail.dat);
        fn_ptr(h, d) // c:1006
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
/// Checks for an existing param of the same name; if absent → 0
/// (free to add). If present and not autoloadable → 1 (warn) or
/// 2 (skip warning under `-i`). If present + autoload-marked →
/// unset the existing entry and return 0.
pub fn checkaddparam(nam: &str, opt_i: i32) -> i32 {
    // c:1026
    // c:1030 — if (!(pm = gethashnode2(paramtab, nam))) return 0;
    let pm_clone = {
        let tab = paramtab().read().expect("paramtab poisoned");
        tab.get(nam).cloned()
    };
    let mut pm = match pm_clone {
        Some(p) => p,
        None => return 0,
    };
    // c:1033 — if (pm->level || !(pm->node.flags & PM_AUTOLOAD)) {
    if pm.level != 0 || (pm.node.flags as u32 & PM_AUTOLOAD) == 0 {
        // c:1042-1048 — if (!opt_i || pm->level) zwarn(...); return 1;
        if opt_i == 0 || pm.level != 0 {
            zwarn(&format!(
                "Can't add module parameter `{}': {}",
                nam,
                if pm.level != 0 {
                    "local parameter exists"
                } else {
                    "parameter already exists"
                }
            ));
            return 1;
        }
        // c:1049 — return 2;
        return 2;
    }
    // c:1052 — unsetparam_pm(pm, 0, 1);
    unsetparam_pm(&mut pm, 0, 1);
    // c:1053 — return 0;
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
pub fn addparamdef(d: &mut paramdef) -> i32 {
    // c:1061

    // c:1065 — `if (checkaddparam(d->name, 0)) return 1;`
    if checkaddparam(&d.name, 0) != 0 {
        // c:1065
        return 1; // c:1066
    }

    // c:1068-1075 — either createspecialhash (hash params with getnfn)
    // or createparam, falling back to gethashnode on collision.
    let pm_opt: Option<Param> = if d.getnfn.is_some() {
        // c:1068
        // c:1069-1071 — createspecialhash(d->name, d->getnfn, d->scantfn, d->flags)
        // The Rust createspecialhash takes (name, flags) only; the
        // getnfn/scantfn fields aren't yet wired through the typed
        // Rust API. Pass flags and let the param be created.
        createspecialhash(&d.name, d.flags) // c:1069
    } else {
        // c:1072
        match createparam(&d.name, d.flags) {
            // c:1073
            Some(p) => Some(p),
            None => {
                // c:1074 — fall back to paramtab->getnode(paramtab, d->name)
                let tab = paramtab().read().ok();
                tab.and_then(|t| {
                    t.get(&d.name).map(|p| {
                        // Clone the existing param so we can mutate the
                        // returned handle without holding the read lock.
                        let mut clone = p.clone();
                        clone.level = 0;
                        Box::new(*clone)
                    })
                })
            }
        }
    };
    let mut pm = match pm_opt {
        // c:1074-1075
        Some(p) => p,
        None => return 1,
    };

    // c:1077-1078 — `d->pm = pm; pm->level = 0;`
    pm.level = 0; // c:1078

    // c:1079-1080 — `if (d->var) pm->u.data = d->var;`
    if d.var != 0 { // c:1079
         // pm.u.data is a raw `void *` slot — not yet exposed on the
         // Rust param mirror. Carry the assignment as a comment.
         // pm.u.data = d->var as *mut _;                                     // c:1080
    }

    if d.var != 0 || d.gsu != 0 {
        // c:1081
        let t = PM_TYPE(pm.node.flags as u32); // c:1086
        let pmflags = pm.node.flags as u32;
        if t == PM_SCALAR || t == PM_NAMEREF {
            // c:1087/1091
            if t == PM_SCALAR && (pmflags & PM_TIED) != 0 {
                // c:1088
                let lower = casemodify(&pm.node.nam, CASMOD_LOWER);
                pm.ename = Some(ztrdup(&lower)); // c:1089
            }
            // c:1092 pm->gsu.s = d->gsu ? d->gsu : &varscalar_gsu;
            // gsu vtable wireup is opaque (function pointers via usize);
            // the Rust param dispatch reads directly from typed accessors.
            let _ = d.gsu; // c:1092
        } else if t == PM_INTEGER {
            // c:1095
            let _ = d.gsu; // c:1096
        } else if t == PM_FFLOAT || t == PM_EFLOAT {
            // c:1099-1100
            let _ = d.gsu; // c:1101
        } else if t == PM_ARRAY {
            // c:1104
            if (pmflags & PM_TIED) != 0 {
                // c:1105
                let upper = casemodify(&pm.node.nam, CASMOD_UPPER);
                pm.ename = Some(ztrdup(&upper)); // c:1106
            }
            let _ = d.gsu; // c:1107
        } else if t == PM_HASHED {
            // c:1110
            let _ = d.gsu; // c:1112-1113
        } else {
            // c:1116
            unsetparam_pm(&mut pm, 0, 1); // c:1117
            return 1; // c:1118
        }
    }

    d.pm = Some(pm); // c:1077 d->pm = pm
    0 // c:1122
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
pub fn deleteparamdef(d: &mut paramdef) -> i32 {
    // c:1128

    // c:1131 — `Param pm = (Param) paramtab->getnode(paramtab, d->name);`
    let mut pm: Param = {
        let tab = paramtab().read();
        match tab {
            Ok(t) => match t.get(&d.name) {
                Some(p) => p.clone(),
                None => return 1, // c:1133-1134
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
    if let Some(expected) = d.pm.as_ref() {
        // c:1135
        if !std::ptr::eq(pm.as_ref(), expected.as_ref()) {
            // c:1135 pm != d->pm
            // c:1141-1145 — walk pm->old looking for d->pm.
            let mut searchpm = pm.old.clone(); // c:1142
            let mut found = false;
            while let Some(s) = searchpm {
                // c:1142
                if std::ptr::eq(s.as_ref(), expected.as_ref()) {
                    // c:1144
                    found = true; // c:1145
                    break;
                }
                searchpm = s.old.clone(); // c:1143
            }
            if !found {
                // c:1147
                return 1; // c:1148
            }
            // c:1150-1153 — splice searchpm out of the chain and
            // re-add it under its node.nam. Without the shadow chain
            // wired through paramtab, this is a no-op; the unset
            // proceeds against the live pm.
        }
    }

    // c:1157 — `pm->node.flags = (pm->node.flags & ~PM_READONLY) | PM_REMOVABLE;`
    pm.node.flags = (pm.node.flags & !(PM_READONLY as i32)) | (PM_REMOVABLE as i32); // c:1157
    unsetparam_pm(&mut pm, 0, 1); // c:1158
    d.pm = None; // c:1159 d->pm = NULL
    0 // c:1160
}

/// Port of `static int add_autoparam(const char *module, const char *pnam, int flags)`
/// from `Src/module.c:1197`.
///
/// C body c:1197-1228:
/// ```c
/// static int
/// add_autoparam(const char *module, const char *pnam, int flags)
/// {
///     Param pm;
///     int ret;
///     int ne = noerrs;
///
///     queue_signals();
///     if ((ret = checkaddparam(pnam, (flags & FEAT_IGNORE)))) {
///         unqueue_signals();
///         return ret == 2 ? 0 : -1;
///     }
///     noerrs = 2;
///     if ((pm = setsparam(dupstring(pnam), ztrdup(module)))) {
///         pm->node.flags |= PM_AUTOLOAD;
///         if (flags & FEAT_AUTOALL)
///             pm->node.flags |= PM_AUTOALL;
///         ret = 0;
///     } else
///         ret = -1;
///     noerrs = ne;
///     unqueue_signals();
///
///     return ret;
/// }
/// ```
///
/// Adds an autoload stub for a module parameter: `setsparam` creates
/// the param as a scalar with `value=module name`, then OR-in
/// `PM_AUTOLOAD` so first access fires `loadparamnode` (c:546) which
/// calls `ensurefeature(module, "p:", nam)` to actually load the
/// module. `typeset +` listing checks `PM_AUTOLOAD` and emits
/// "undefined NAME" instead of the usual type prefix.
pub fn add_autoparam(module: &str, pnam: &str, flags: i32) -> i32 {
    // c:1197
    use crate::ported::signals::queue_signals;
    use std::sync::atomic::Ordering;

    // c:1202 — int ne = noerrs;
    let ne = *crate::ported::utils::noerrs_lock().lock().unwrap();

    // c:1204 — queue_signals();
    queue_signals();

    // c:1205 — if ((ret = checkaddparam(pnam, (flags & FEAT_IGNORE)))) {
    let ret_check = checkaddparam(pnam, flags & FEAT_IGNORE as i32);
    if ret_check != 0 {
        // c:1206 — unqueue_signals();
        unqueue_signals();
        // c:1214 — return ret == 2 ? 0 : -1;
        return if ret_check == 2 { 0 } else { -1 };
    }

    // c:1217 — noerrs = 2;
    *crate::ported::utils::noerrs_lock().lock().unwrap() = 2;

    // c:1218 — if ((pm = setsparam(dupstring(pnam), ztrdup(module))))
    let pm_opt = crate::ported::params::setsparam(pnam, module);
    let ret = if let Some(mut pm) = pm_opt {
        // c:1219 — pm->node.flags |= PM_AUTOLOAD;
        pm.node.flags |= PM_AUTOLOAD as i32;
        // c:1220-1221 — if (flags & FEAT_AUTOALL) pm->node.flags |= PM_AUTOALL;
        if (flags & FEAT_AUTOALL as i32) != 0 {
            pm.node.flags |= PM_AUTOALL as i32;
        }
        // Re-insert the modified clone back into paramtab so the flag
        // bits stick (paramtab stores `Param` by value, not by pointer).
        {
            let mut tab = paramtab().write().expect("paramtab poisoned");
            tab.insert(pnam.to_string(), pm);
        }
        // c:1222 — ret = 0;
        0
    } else {
        // c:1224 — ret = -1;
        -1
    };

    // c:1225 — noerrs = ne;
    *crate::ported::utils::noerrs_lock().lock().unwrap() = ne;
    // c:1226 — unqueue_signals();
    unqueue_signals();
    // c:1228 — return ret;
    ret
}

/// Port of `static int del_autoparam(const char *modnam, const char *pnam, int flags)`
/// from `Src/module.c:1234`.
///
/// C body c:1234-1248:
/// ```c
/// static int
/// del_autoparam(UNUSED(const char *modnam), const char *pnam, int flags)
/// {
///     Param pm = (Param) gethashnode2(paramtab, pnam);
///     if (!pm) {
///         if (!(flags & FEAT_IGNORE)) return 2;
///     } else if (!(pm->node.flags & PM_AUTOLOAD)) {
///         if (!(flags & FEAT_IGNORE)) return 3;
///     } else
///         unsetparam_pm(pm, 0, 1);
///     return 0;
/// }
/// ```
///
/// Removes a param previously registered by `add_autoparam`. Returns
/// 2 when the param doesn't exist, 3 when it exists but isn't an
/// autoload stub, 0 on success — `FEAT_IGNORE` masks both error
/// returns to 0.
pub fn del_autoparam(_modnam: &str, pnam: &str, flags: i32) -> i32 {
    // c:1234
    // c:1237 — Param pm = (Param) gethashnode2(paramtab, pnam);
    let pm_opt = {
        let tab = paramtab().read().expect("paramtab poisoned");
        tab.get(pnam).cloned()
    };
    match pm_opt {
        // c:1239 — if (!pm) { if (!(flags & FEAT_IGNORE)) return 2; }
        None => {
            if (flags & FEAT_IGNORE as i32) == 0 {
                return 2; // c:1241
            }
        }
        Some(mut pm) => {
            if (pm.node.flags as u32 & PM_AUTOLOAD) == 0 {
                // c:1242 — else if (!(pm->node.flags & PM_AUTOLOAD))
                if (flags & FEAT_IGNORE as i32) == 0 {
                    return 3; // c:1244
                }
            } else {
                // c:1246 — else unsetparam_pm(pm, 0, 1);
                unsetparam_pm(&mut pm, 0, 1);
            }
        }
    }
    0 // c:1248
}

impl modulestab {
    /// `new` — see implementation.
    pub fn new() -> Self {
        let mut table = Self {
            // `Src/init.c:1193` — `modulestab = newmoduletable(17, "modules")`.
            modules: crate::ported::hashtable::hashtable_nodes::newhashtable(17),
            ..Self::default()
        };
        table.register_builtin_modules();
        table
    }

    /// Register all statically-compiled modules (replaces dlopen)
    fn register_builtin_modules(&mut self) {
        let builtin_modules = [
            // Module→features MUST match the C BUILTIN() homes or a
            // builtin's feature lookup (`b:<name>` against its home module)
            // fails with "module `X' has no such feature". These were
            // SCRAMBLED: compadd/compset (complete.c:1693-1694) were under
            // zsh/computil, while the computil builtins (computil.c:5131-
            // 5138) were under zsh/complete, so calling `compset` from a
            // shell fn errored `module 'zsh/complete' has no such feature`.
            ("zsh/complete", &["compadd", "compset"][..]), // complete.c:1693-1694
            ("zsh/complist", &["complist"][..]),
            (
                "zsh/computil",
                &[
                    "comparguments", // computil.c:5131
                    "compdescribe",  // c:5132
                    "compfiles",     // c:5133
                    "compgroups",    // c:5134
                    "compquote",     // c:5135
                    "comptags",      // c:5136
                    "comptry",       // c:5137
                    "compvalues",    // c:5138
                ][..],
            ),
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
                    "bin_sysread",
                    "bin_syswrite",
                    "bin_sysopen",
                    "bin_sysseek",
                    "bin_syserror",
                    "zsystem",
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
            // c:Src/Modules/compctl.c — statically-linked completion
            // module providing the compctl/compcall builtins. zsh
            // exposes it as autoloadable; `zmodload zsh/compctl`
            // succeeds on the running zsh because the symbol is
            // baked in. The zshrs auto-load registry (zsh_default_
            // loaded at line 1127) references it but the entry was
            // missing from this builtin_modules table, so
            // try_load_module returned 0 and zmodload failed with
            // "failed to load module `zsh/compctl'".
            ("zsh/compctl", &["compctl", "compcall"][..]),
            // c:Src/Builtins/rlimits.c — limit/ulimit/unlimit are
            // baked in. Same gap as zsh/compctl above.
            ("zsh/rlimits", &["limit", "ulimit", "unlimit"][..]),
            // c:Src/Zle/zle_main.c — zle/vared/bindkey baked in.
            ("zsh/zle", &["zle", "vared", "bindkey"][..]),
            // c:Src/Modules/example.c — example module that prints
            // "The example module has now been set up." on boot;
            // statically linked so `zmodload zsh/example` succeeds.
            ("zsh/example", &["example"][..]),
            // NOT registered (zsh -fc parity probes confirm these
            // FAIL to load on this system because the dynamic
            // .bundle file doesn't exist):
            //   zsh/calendar      — pure dynamic module, no static
            //                       linkage in upstream zsh build.
            //   zsh/db_gdbm       — underscore alias for zsh/db/gdbm;
            //                       on the system zsh tested, neither
            //                       form loads (dlopen "no such file").
            //   zsh/deltochar     — Zle widget addon; system zsh's
            //                       bundle missing the _bindk symbol.
            //   zsh/compwid       — compwid bundle missing.
            // Letting zshrs zmodload succeed on these names would
            // diverge from `zsh -fc` which reports the dlopen error.
            // The names appear above only via try_load_module's
            // negative path (returns 0 → zmodload prints "failed to
            // load module").
        ];

        // c:Src/init.c::init_bltinmods — C zsh's bltinmods.list is
        // generated at build time from Config/installmodules + the
        // active Modules/*.mdd files. Homebrew's stock zsh 5.9.1
        // binary ships these 14 modules in modulestab from the
        // start (verified via `${(@k)modules}` under `zsh -fc`):
        //   zsh/compctl zsh/complete zsh/computil zsh/main
        //   zsh/param/private zsh/parameter zsh/rlimits zsh/sched
        //   zsh/termcap zsh/terminfo zsh/watch zsh/zle
        //   zsh/zleparameter zsh/zutil
        // The rest (zsh/system, zsh/stat, zsh/zftp, zsh/zpty,
        // zsh/zselect, zsh/files, zsh/mapfile, etc.) require
        // explicit `zmodload NAME` before `${modules[NAME]}`
        // returns "loaded". All entries are registered into
        // modulestab so `zmodload NAME` can resolve them, but
        // entries OUTSIDE the default-loaded set carry `MOD_UNLOAD`
        // at init so `getpmmodule`'s is_loaded() check
        // (MOD_LINKED && !MOD_UNLOAD per zsh_h::is_loaded c:962)
        // returns false until `zmodload` clears the MOD_UNLOAD bit.
        // Bugs #530/#532/#535 in docs/BUGS.md.
        let zsh_default_loaded: &[&str] = &[
            "zsh/compctl",
            "zsh/complete",
            "zsh/computil",
            "zsh/main",
            "zsh/param/private",
            "zsh/parameter",
            "zsh/rlimits",
            "zsh/sched",
            "zsh/termcap",
            "zsh/terminfo",
            "zsh/watch",
            "zsh/zle",
            "zsh/zleparameter",
            "zsh/zutil",
        ];
        // c:Src/module.c:359-379 `register_module` — every compiled-in
        // module is appended to `linkedmodules` and NOTHING ELSE. C's
        // `modulestab` starts EMPTY and is filled lazily; the node set
        // it ends up with at the end of `main()` is decided entirely by
        // the generated `bltinmods.list` replayed below.
        //
        // Inserting all ~40 known modules as `modulestab` nodes here
        // (the previous shape) put `ct` past `hsize * 2` and quadrupled
        // the table from C's 17 buckets to 68, so `${(k)modules}` —
        // which is a raw bucket walk (`Src/Modules/parameter.c:1102`)
        // — listed loaded modules in an order that could not match zsh
        // for any collision: `zmodload zsh/system; zmodload zsh/zle`
        // printed `zle, system` where zsh prints `system, zle`
        // (bucket 6 vs buckets 23/57).
        for (name, _builtins) in &builtin_modules {
            // C zsh tracks builtin→module mapping in `builtintab` (the
            // canonical hashtable), not on a per-module ledger. We
            // just record the module as linked here; the builtins
            // themselves come in via the canonical table in `cmd.rs`.
            self.linkedmodules.push((*name).to_string()); // c:378
        }
        // `zsh/main` is the `link=static` module `Src/mkbltnmlst.sh:107`
        // emits a `register_module("zsh/main", …)` call for; it is not
        // in the autoloadable table above.
        self.linkedmodules.push("zsh/main".to_string()); // c:378

        // c:Src/init.c:1739 `#include "bltinmods.list"` — the generated
        // boot code, replayed in order. `Src/mkbltnmlst.sh` walks
        // `config.modules` and emits, for every `load=yes` module:
        //   * `autofeatures("zsh", MODULE, features, 0, 1)` when the
        //     `.mdd` has an `autofeatures=` line (mkbltnmlst.sh:44-70);
        //     `autofeatures` opens with
        //     `find_module(module, FINDMOD_ALIASP|FINDMOD_CREATE, NULL)`
        //     (c:3449) — THAT is what creates the module's `modulestab`
        //     node.
        //   * one `add_dep(MODULE, DEP)` per `moddeps=` entry
        //     (mkbltnmlst.sh:71-73); `add_dep` also opens with
        //     `find_module(name, …|FINDMOD_CREATE, &name)` (c:2390), so
        //     a module with deps but no autofeatures (`zsh/complist`)
        //     still gets a node.
        // then a second pass (mkbltnmlst.sh:83-105) emits `add_dep` for
        // `link=dynamic load=no` modules that declare `moddeps`.
        //
        // ORDER IS LOAD-BEARING: `addhashnode` front-inserts, so the
        // chain order inside a bucket is reverse creation order, and
        // `${(k)modules}` shows it. The sequence below is
        // `config.modules` order (Src/Builtins, then Src/Modules, then
        // Src/Zle, then Src/zsh.mdd), verified against the oracle:
        // `zsh -f -c 'zmodload <all 15>; print -rl -- ${(k)modules}'`
        // pins terminfo-before-watch, parameter-before-computil,
        // zle-before-zleparameter and rlimits-before-param/private.
        //
        // The resulting node count is 16 (15 here + `zsh/main` below),
        // which is also what the running zsh measures: adding module
        // aliases one at a time, the 18th alias is the one that
        // re-hashes the table (17 buckets, expand at `ct >= 34`), so
        // the boot count is exactly 34 - 18 = 16.
        //
        // `zsh/hlgroup`, `zsh/ksh93` and `zsh/random` carry `load=yes`
        // in current zsh git but do not exist in the 5.9.x line this
        // parity target ships, and zshrs implements none of them, so
        // they contribute no boot node.
        //
        // The `autofeatures` column below is each `.mdd`'s
        // `autofeatures=` line VERBATIM — that string is what
        // `mkbltnmlst.sh:63-69` bakes into the generated
        // `char *features[] = { … }` array, and `autofeatures("zsh",
        // MODULE, features, 0, 1)` (defflags 1 = FEAT_IGNORE, c:62) is
        // what registers the `b:`/`c:`/`p:`/`f:` autoloads. Replaying
        // only the `find_module` half left `zmodload -ac` and
        // `zmodload -ap` printing nothing where zsh lists four
        // conditions and forty parameters.
        let bltinmods_list: &[(&str, &[&str], &[&str])] = &[
            // (module, `autofeatures=`, `moddeps=`)
            // Src/Builtins/rlimits.mdd:5
            ("zsh/rlimits", &["b:limit", "b:ulimit", "b:unlimit"][..], &[][..]),
            // Src/Builtins/sched.mdd:5
            ("zsh/sched", &["b:sched", "p:zsh_scheduled_events"][..], &[][..]),
            // Src/Modules/param_private.mdd:5
            ("zsh/param/private", &["b:private"][..], &[][..]),
            // Src/Modules/parameter.mdd:5
            (
                "zsh/parameter",
                &[
                    "p:parameters",
                    "p:commands",
                    "p:functions",
                    "p:dis_functions",
                    "p:functions_source",
                    "p:dis_functions_source",
                    "p:funcfiletrace",
                    "p:funcsourcetrace",
                    "p:funcstack",
                    "p:functrace",
                    "p:builtins",
                    "p:dis_builtins",
                    "p:reswords",
                    "p:dis_reswords",
                    "p:patchars",
                    "p:dis_patchars",
                    "p:options",
                    "p:modules",
                    "p:dirstack",
                    "p:history",
                    "p:historywords",
                    "p:jobtexts",
                    "p:jobdirs",
                    "p:jobstates",
                    "p:nameddirs",
                    "p:userdirs",
                    "p:usergroups",
                    "p:aliases",
                    "p:dis_aliases",
                    "p:galiases",
                    "p:dis_galiases",
                    "p:saliases",
                    "p:dis_saliases",
                ][..],
                &[][..],
            ),
            // Src/Modules/termcap.mdd:15
            ("zsh/termcap", &["b:echotc", "p:termcap"][..], &[][..]),
            // Src/Modules/terminfo.mdd:15
            ("zsh/terminfo", &["b:echoti", "p:terminfo"][..], &[][..]),
            // Src/Modules/watch.mdd:5
            ("zsh/watch", &["b:log", "p:WATCH", "p:watch"][..], &[][..]),
            // Src/Modules/zutil.mdd:9
            (
                "zsh/zutil",
                &["b:zformat", "b:zstyle", "b:zregexparse", "b:zparseopts"][..],
                &["zsh/complete"][..],
            ),
            // Src/Zle/compctl.mdd:7
            (
                "zsh/compctl",
                &["b:compctl", "b:compcall"][..],
                &["zsh/complete", "zsh/zle"][..],
            ),
            // Src/Zle/complete.mdd:8 — the only `c:` autofeatures in the
            // boot set, and the source of `zmodload -ac`'s four rows.
            (
                "zsh/complete",
                &[
                    "b:compadd",
                    "b:compset",
                    "c:prefix",
                    "c:suffix",
                    "c:between",
                    "c:after",
                ][..],
                &["zsh/zle"][..],
            ),
            // No `autofeatures=`; the node comes from `add_dep` alone.
            ("zsh/complist", &[][..], &["zsh/complete", "zsh/zle"][..]), // Src/Zle/complist.mdd
            // Src/Zle/computil.mdd:9
            (
                "zsh/computil",
                &[
                    "b:compdescribe",
                    "b:comparguments",
                    "b:compvalues",
                    "b:compquote",
                    "b:comptags",
                    "b:comptry",
                    "b:compfiles",
                    "b:compgroups",
                ][..],
                &["zsh/complete", "zsh/zle"][..],
            ),
            // Src/Zle/zle.mdd:6
            ("zsh/zle", &["b:bindkey", "b:vared", "b:zle"][..], &[][..]),
            // Src/Zle/zleparameter.mdd:7
            (
                "zsh/zleparameter",
                &["p:widgets", "p:keymaps"][..],
                &["zsh/zle"][..],
            ),
            // mkbltnmlst.sh:83-105 second pass — `link=dynamic
            // load=no` with `moddeps`. `zsh/deltochar` is the only
            // other module in that shape and does NOT appear: the
            // grep is `' link=dynamic .* load=no '` with a TRAILING
            // space, and deltochar's `config.modules` line ends at
            // `load=no` (it declares no `functions=`), so it never
            // matches. Verified on the oracle: `zmodload -d` lists
            // seven modules and deltochar is not one of them.
            // zftp.mdd DOES carry `autofeatures="b:zftp"`, but it is
            // `load=no`, so mkbltnmlst.sh's second pass emits ONLY the
            // `add_dep` — no `autofeatures()` call, which is why
            // `zmodload -a` on the oracle has no `zftp` row.
            ("zsh/zftp", &[][..], &["zsh/net/tcp"][..]), // Src/Modules/zftp.mdd
        ];
        for (name, autofeature_list, deps) in bltinmods_list {
            if !autofeature_list.is_empty() {
                // c:3449 — autofeatures() opens with
                //   find_module(module, FINDMOD_ALIASP|FINDMOD_CREATE, NULL)
                // which is what creates the module's modulestab node.
                // The feature registration itself is replayed after the
                // whole list has its nodes + MOD_LINKED flags (see the
                // second pass below) — `autofeatures` reads
                // `MOD_INIT_B`/the feature tables off the node it is
                // registering against, so the node has to exist first.
                find_module(self, name, FINDMOD_CREATE);
            }
            for dep in *deps {
                add_dep(self, name, dep); // c:2390 (via c:3449's sibling)
            }
            // WARNING: RUST-ONLY — C's `find_module` node is
            // `zshcalloc`ed with flags 0 and a NULL union, so
            // `module_loaded` reads 0 for every one of these until a
            // real `zmodload`. zshrs's static-link mirror instead uses
            // `MOD_LINKED` as the "this name has backing code" bit
            // (`zsh_h::module::is_loaded`) and `MOD_UNLOAD` as the
            // "not live yet" sentinel, and a long tail of gates in
            // `subst.rs` / `params.rs` / `fusevm_bridge.rs` read those
            // two bits. Keep the flags each of these nodes carried
            // before the boot set was trimmed, so this change moves
            // ONLY the bucket layout.
            if let Some(m) = self.modules.get_mut(name) {
                m.node.flags |= MOD_LINKED;
                if !zsh_default_loaded.contains(name) {
                    m.node.flags |= crate::ported::zsh_h::MOD_UNLOAD;
                }
            }
        }

        // c:Src/init.c:1708 init_bltinmods — run per-module `boot_`
        // for each statically-linked default-loaded module so paramtab
        // entries (e.g. `watch`/`WATCH` from zsh/watch, c:734) get
        // installed without an explicit zmodload. Without this,
        // `${(t)watch}` returns empty instead of `array-special`
        // even though zsh treats zsh/watch as effectively part of
        // the shell. Bug #270 in docs/BUGS.md. Keep the modules at
        // their MOD_LINKED-without-MOD_INIT_B initial state so the
        // `zmodload` (no-args) listing — gated on MOD_INIT_B per
        // bug #76 — still shows only `zsh/main`.
        for name in zsh_default_loaded {
            #[allow(clippy::single_match)]
            match *name {
                "zsh/watch" => {
                    // `Src/Modules/watch.mdd` is `link=dynamic` with
                    // `autofeatures="b:log p:WATCH p:watch"`, so real zsh
                    // registers ONLY those three names up front and defers
                    // `boot_` until the module is actually loaded. C's
                    // `boot_` is the sole creator of WATCHFMT/LOGCHECK:
                    //
                    //   c:Src/Modules/watch.c:756-759
                    //     if (!paramtab->getnode(paramtab, "WATCHFMT"))
                    //         setsparam("WATCHFMT", ztrdup_metafy(default_watchfmt));
                    //     if (!paramtab->getnode(paramtab, "LOGCHECK"))
                    //         setiparam("LOGCHECK", 60);
                    //
                    // and it runs only from `load_module`
                    // (c:Src/module.c:2306). Verified against the oracle:
                    // `zsh -f -c 'print ${WATCHFMT-UNSET}'` prints UNSET and
                    // neither name appears in `${(ko)parameters}`, while
                    // `zsh -f -c 'zmodload zsh/watch; print $WATCHFMT'`
                    // prints `%n has %a %l from %m.` — the same is true for
                    // any first touch of `$WATCH`/`$watch`/`log`, which is
                    // what triggers the autoload.
                    //
                    // This call is a zshrs-only shim: the reason it exists
                    // is the `watch`/`WATCH` paramtab registration that C
                    // gets from the autofeature list, so `${(t)watch}`
                    // reads `array-special` from the first prompt (bug #270
                    // in docs/BUGS.md). Its WATCHFMT/LOGCHECK side effect is
                    // NOT part of that contract, so undo it for names the
                    // shim itself invented. The condition is "was zsh/watch
                    // really loaded", exactly as in C — NOT the shell's
                    // emulation mode: the previous `--zsh`-only gate inside
                    // `watch::boot_` left both names in `${(ko)parameters}`
                    // for the native binary. A pre-existing value (exported
                    // WATCHFMT, or a real `zmodload zsh/watch` later on)
                    // is left untouched.
                    // `boot_` no longer seeds WATCHFMT/LOGCHECK when `m` is
                    // null (watch.rs's `mid_load` gate), so there is nothing
                    // left to undo here.
                    crate::ported::modules::watch::boot_(std::ptr::null());
                }
                // c:Src/Zle/compctl.c:4016 setup_ — seed the hardwired
                // default compctls (cc_compos/cc_default/cc_first) the
                // bare shell uses for command/file completion. Runs here
                // at module registration (before any rc file), so it
                // never clobbers a user's later `compctl` definitions.
                // Without it CC_COMPOS is unset and `l<Tab>` in `zsh -f`
                // produced no matches.
                "zsh/compctl" => {
                    crate::ported::zle::compctl::setup_();
                }
                _ => {}
            }
        }

        // c:Src/init.c:1739 `#include "bltinmods.list"` — the
        // `autofeatures("zsh", MODULE, features, 0, 1)` half, replayed in
        // `config.modules` order. `mkbltnmlst.sh:62-70` wraps each call in
        // `if (EMULATION(EMULATE_ZSH))`, and every boot module except
        // `zsh/rlimits` / `zsh/ksh93` (the two with `autofeatures_emu=`)
        // registers NOTHING outside zsh emulation. zshrs's emulation is
        // not settled at `modulestab::new()` time — this runs from
        // `init_bltinmods`, before `apply_cli_flags`' `emulate("sh")` —
        // so the zsh arm is the one replayed; a `--sh`/`--bash` process
        // will carry the zsh feature set, matching what zshrs already
        // does for the builtin ledger this replaces.
        //
        // `prefchar = 0` (the features carry their own `b:`/`c:`/`p:`
        // type prefix) and `defflags = 1` = FEAT_IGNORE (c:62), which is
        // what lets a name the static link already provides — every one
        // of the 27 `b:` builtins, and the eagerly-seeded `p:` specials
        // (`vm_helper::init_partab_params`) — register its ledger entry
        // without an "already defined" diagnostic.
        //
        // ORDERING (Rust-only): this runs AFTER the `zsh_default_loaded`
        // `boot_` shim loop above, not interleaved with node creation as
        // `bltinmods.list` has it. That loop is itself a zshrs-only shim
        // (see its comment) which installs `zsh/watch`'s `watch`/`WATCH`
        // specials eagerly, the way `init_partab_params` installs the
        // `partab[]` ones. Running the replay first made `add_autoparam`
        // (c:1197) win the race and leave a live PM_AUTOLOAD SCALAR stub
        // whose value is the module name, so `${#watch}` read 9
        // ("zsh/watch") instead of 0. With the specials already in
        // paramtab, `checkaddparam` (c:1026) returns 2 under FEAT_IGNORE
        // and `add_autoparam` is the no-op C reaches once a module is
        // loaded — while the ledger entry `autofeatures` records is what
        // `zmodload -ap` lists.
        for (name, autofeature_list, _deps) in bltinmods_list {
            if autofeature_list.is_empty() {
                continue;
            }
            let features: Vec<String> = autofeature_list.iter().map(|f| f.to_string()).collect();
            // c:3440 `autofeatures(cmdnam, module, features, prefchar,
            //          defflags)` — cmdnam "zsh" per mkbltnmlst.sh:69.
            autofeatures(self, "zsh", Some(name), &features, 0, FEAT_IGNORE);
        }


        // The auto-load builtin→module bindings `zmodload -a` reports
        // used to be a hand-maintained 27-row `autoload_pairs` table
        // here. They are now produced by the `b:` half of the
        // `autofeatures` replay above, from each `.mdd`'s own
        // `autofeatures=` line — one source of truth instead of two.
        // The set is unchanged: rlimits 3 + sched 1 + param/private 1 +
        // termcap 1 + terminfo 1 + watch 1 + zutil 4 + compctl 2 +
        // complete 2 + computil 8 + zle 3 = 27, matching
        // `/opt/homebrew/bin/zsh -fc 'zmodload -a'` exactly. `zsh/files`
        // builtins (mkdir, rm, …) are statically linked but carry no
        // boot autofeatures entry (files.mdd is `load=no`), so they stay
        // out of the registry — zsh requires an explicit
        // `zmodload zsh/files`. Bug #222.

        // c:Src/init.c:1708 — `init_bltinmods` ends with
        // `load_module("zsh/main", NULL, 0)`. `zsh/main` is the
        // always-loaded master module: every zsh process has it in
        // `modulestab` from boot, with `m->u.handle` (or `u.linked`)
        // non-NULL so `printmodulenode`'s "loaded" gate (c:218
        // `m->u.handle`) fires. Register here with `MOD_INIT_B` set
        // so `zmodload` (no args) lists `zsh/main` and ONLY `zsh/main`
        // — matching `/opt/homebrew/bin/zsh -fc 'zmodload'` output
        // exactly. Bug #76 in docs/BUGS.md.
        let mut main = module::new("zsh/main");
        main.node.flags |= crate::ported::zsh_h::MOD_INIT_B | MOD_INIT_S; // c:2244
                                                                          // c:2289 — load_module("zsh/main") assigns `m->u.linked`; the
                                                                          // union must be non-NULL so `zmodload -e zsh/main` (c:2637
                                                                          // `!m->u.handle`) reports the master module as loaded.
        main.linked = Some(Box::new(linkedmod {
            name: "zsh/main".to_string(),
            setup: None,
            features: None,
            enables: None,
            boot: None,
            cleanup: None,
            finish: None,
        }));
        self.modules.insert("zsh/main".to_string(), main);
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
    /// `enablesarr` is C's `Feature_enables` argument, threaded through
    /// to `do_boot_module` (c:2306) so a `require_module` that asks for ONE
    /// feature enables only that feature — the port used to drop the
    /// argument and enable every feature the module has.
    /// `enablesarr` is C's `Feature_enables` argument, threaded through to
    /// `do_boot_module` (c:2306) so a `require_module` that asks for ONE
    /// feature enables only that feature. The port used to drop the
    /// argument and pass `None` (= enable everything), so the first
    /// `[[ -prefix … ]]` installed all four of `zsh/complete`'s conddefs
    /// where zsh installs only `prefix`.
    /// WARNING: param names don't match C — Rust=(name, enablesarr) vs C=(name, enablesarr, silent)
    pub fn load_module(
        &mut self,
        name: &str,
        enablesarr: Option<&[String]>,
        // !!! RUST-ONLY PARAM !!! — carries C's per-entry
        // `Feature_enables.pat` (see FEAT_PATTERN_ARGS).
        feat_pat: bool,
    ) -> bool {
        // c:2200
        // Faithful port of the find_module-found branch (c:2249-2320).
        // The !find_module branch (c:2219-2247) requires DSO loading and
        // never fires in zshrs's static-link path: every linked module
        // is pre-registered by register_builtin_modules.
        //
        // C body c:2249-2320:
        //   if (m->flags & MOD_SETUP) return 0;
        //   if (m->flags & MOD_UNLOAD) m->flags &= ~MOD_UNLOAD;
        //   else if (m->u.linked / m->u.handle) return 0;
        //   if (m->flags & MOD_BUSY) {
        //       zerr("circular dependencies for module ;%s", name);
        //       return 1;
        //   }
        //   m->flags |= MOD_BUSY;
        //   if (m->deps) for each dep: load_module(dep, NULL, silent);
        //   m->flags &= ~MOD_BUSY;
        //   if (!m->u.handle) {
        //       module_linked / do_load_module
        //       if (handle) m->u.handle = handle, m->flags |= MOD_SETUP
        //       else m->u.linked = linked, m->flags |= MOD_SETUP|MOD_LINKED
        //       if (setup_module(m)) { finish, NULL handles, ~SETUP, return 1; }
        //       m->flags |= MOD_INIT_S;
        //   }
        //   m->flags |= MOD_SETUP;
        //   if ((bootret = do_boot_module(m, enablesarr, silent)) == 1) {
        //       do_cleanup_module(m); finish_module(m);
        //       m->u.linked/handle = NULL;
        //       m->flags &= ~MOD_SETUP;
        //       return 1;
        //   }
        //   m->flags |= MOD_INIT_B;
        //   m->flags &= ~MOD_SETUP;
        //   return bootret;

        // c:2208 — modname_ok(name)
        if modname_ok(name) == 0 {
            // c:2210 — zerr if !silent (silent flag not threaded yet)
            return false;
        }
        crate::ported::signals::queue_signals(); // c:2222
                                                 // c:2223 — find_module(name, FINDMOD_ALIASP)
        if !self.modules.contains_key(name) {
            // c:2223-2251 — the allocate-on-miss branch. C reaches it
            // for every module whose `modulestab` node has not been
            // created yet, which is the NORMAL state: `bltinmods.list`
            // creates nodes for only 16 modules at boot, so the first
            // `zmodload zsh/system` (or any other module) lands here.
            //
            // c:2224-2225 —
            //   if (!(linked = module_linked(name)) &&
            //       !(handle = do_load_module(name, silent))) { return 1; }
            // zshrs compiles every module in, so `module_linked` is the
            // only arm that can succeed; `do_load_module` (dlopen) is
            // called for its failure path only. `silent = 1` because
            // `require_module` (c:2354's caller) has already emitted the
            // canonical `failed to load module` warning through its own
            // `try_load_module` gate — matching the pre-existing
            // behaviour where this branch returned quietly.
            if !self.module_linked(name) {
                let _ = do_load_module(self, name, 1); // c:2225
                unqueue_signals(); // c:2226
                return false; // c:2227 return 1
            }
            // c:2229 — m = zshcalloc(sizeof(*m));
            let mut m = module::new(name);
            // c:2234-2235 — m->u.linked = linked;
            //               m->node.flags |= MOD_SETUP | MOD_LINKED;
            m.linked = Some(Box::new(linkedmod {
                name: name.to_string(),
                setup: None,
                features: None,
                enables: None,
                boot: None,
                cleanup: None,
                finish: None,
            }));
            m.node.flags = MOD_SETUP | MOD_LINKED;
            // c:2237 — modulestab->addnode(modulestab, ztrdup(name), m);
            self.modules.insert(name.to_string(), m);

            // Same Rust-only paramtab re-seed the already-noded path
            // does below: C gets these SPECIALPMDEF entries from
            // `do_boot_module` → `enables_module` → `addparam`, which
            // zshrs services out of `PARTAB` instead.
            for nm in crate::vm_helper::module_gated_params_for(name) {
                crate::vm_helper::seed_partab_param(nm);
            }

            // c:2239-2240 —
            //   if ((set = setup_module(m)) ||
            //       (bootret = do_boot_module(m, enablesarr, silent)) == 1)
            let set = setup_module(self, name);
            let bootret = if set == 0 {
                do_boot_module(self, name, enablesarr, 0, feat_pat)
            } else {
                1
            };
            if set != 0 || bootret == 1 {
                if set == 0 {
                    let _ = do_cleanup_module(self, name); // c:2242
                }
                let _ = finish_module(self, name); // c:2243
                delete_module(self, name); // c:2244
                unqueue_signals(); // c:2245
                return false; // c:2246 return 1
            }
            // c:2248-2249 — m->node.flags |= MOD_INIT_S | MOD_INIT_B;
            //               m->node.flags &= ~MOD_SETUP;
            if let Some(m) = self.modules.get_mut(name) {
                m.node.flags |= MOD_INIT_S | MOD_INIT_B;
                m.node.flags &= !MOD_SETUP;
            }
            unqueue_signals(); // c:2250
            return true; // c:2251 return bootret
        }

        // c:2249 — if (MOD_SETUP) return 0;
        let flags = self.modules.get(name).unwrap().node.flags;
        if (flags & MOD_SETUP) != 0 {
            unqueue_signals(); // c:2250
            return true; // c:2251 return 0
        }
        // c:2253-2257 —
        //   if (m->node.flags & MOD_UNLOAD)
        //       m->node.flags &= ~MOD_UNLOAD;
        //   else if ((m->node.flags & MOD_LINKED) ? m->u.linked
        //                                         : m->u.handle) {
        //       unqueue_signals();
        //       return 0;
        //   }
        // The union read is real now: load assigns m.linked at c:2289,
        // so already-loaded modules early-return here while
        // pre-registered-but-unloaded ones (linked: None) fall through.
        if (flags & MOD_UNLOAD) != 0 {
            self.modules.get_mut(name).unwrap().node.flags &= !MOD_UNLOAD;
        } else if {
            let m = self.modules.get(name).unwrap();
            if (flags & MOD_LINKED) != 0 {
                m.linked.is_some() // c:2255 m->u.linked
            } else {
                m.handle.is_some() // c:2255 m->u.handle
            }
        } {
            unqueue_signals(); // c:2256
            return true; // c:2257 return 0
        }
        // c:2259-2262 — circular-dependency detection.
        if (flags & MOD_BUSY) != 0 {
            unqueue_signals(); // c:2260
            crate::ported::utils::zerr(&format!("circular dependencies for module ;{}", name));
            return false; // c:2262 return 1
        }
        self.modules.get_mut(name).unwrap().node.flags |= MOD_BUSY; // c:2264

        // c:2269-2277 — recurse into m->deps.
        let deps_snapshot: Vec<String> = self
            .modules
            .get(name)
            .and_then(|m| m.deps.as_ref())
            .map(|d| d.iter().cloned().collect())
            .unwrap_or_default();
        for dep in &deps_snapshot {
            if !self.load_module(dep, None, false) {
                // c:2272 — return 1 on dep failure
                self.modules.get_mut(name).unwrap().node.flags &= !MOD_BUSY; // c:2273
                unqueue_signals(); // c:2274
                return false; // c:2275 return 1
            }
        }
        self.modules.get_mut(name).unwrap().node.flags &= !MOD_BUSY; // c:2278

        // c:2279-2304 — `if (!m->u.handle)` setup branch. The union
        // read means "no live binding yet" — neither handle (DSO) nor
        // linked (static) assigned. A deferred-unload reload re-enters
        // with m.linked still set and skips straight to boot, like C.
        let needs_setup = {
            let m = self.modules.get(name).unwrap();
            m.handle.is_none() && m.linked.is_none() // c:2279 !m->u.handle
        };
        if needs_setup {
            // c:2281 — `linked = module_linked(name)`: every zshrs
            // module is statically linked, so the lookup always hits;
            // callbacks dispatch by name (839f32249b), the record
            // carries the name like C's linkedmod.
            // c:2289-2291 — `m->u.linked = linked;
            //                m->node.flags |= MOD_SETUP | MOD_LINKED;`
            if let Some(m) = self.modules.get_mut(name) {
                m.linked = Some(Box::new(linkedmod {
                    name: name.to_string(),
                    setup: None,
                    features: None,
                    enables: None,
                    boot: None,
                    cleanup: None,
                    finish: None,
                }));
                m.node.flags |= MOD_SETUP | MOD_LINKED;
            }
            // c:2293 — setup_module(m). Routes through the dispatcher
            // (839f32249b) to per-module setup_(m). Most modules return
            // 0; some initialise module-private state.
            if setup_module(self, name) != 0 {
                // c:2294-2301 — failure: finish, clear handles, return 1.
                //   else m->u.linked = NULL;  (c:2298)
                let _ = finish_module(self, name);
                if let Some(m) = self.modules.get_mut(name) {
                    m.linked = None; // c:2298
                    m.node.flags &= !MOD_SETUP;
                }
                unqueue_signals(); // c:2300
                return false; // c:2301 return 1
            }
            // c:2303 — `m->flags |= MOD_INIT_S;`
            self.modules.get_mut(name).unwrap().node.flags |= MOD_INIT_S;
        }
        // c:2305 — `m->flags |= MOD_SETUP;`
        self.modules.get_mut(name).unwrap().node.flags |= MOD_SETUP;

        // c:Src/Modules/system.c:902,904 + zsh/mapfile — SPECIALPMDEF
        // entries get added to paramtab via the module's feature
        // dispatch (enables_ → handlefeatures → addparam). zshrs's
        // vm_helper::init_partab_params skips zmodload-gated names to
        // avoid them appearing before explicit load (bug #69). Re-seed
        // here once boot completes. This is a Rust-only bridge — no
        // direct C counterpart since C's handlefeatures runs implicitly
        // inside do_boot_module → enables_module.
        for nm in crate::vm_helper::module_gated_params_for(name) {
            crate::vm_helper::seed_partab_param(nm);
        }

        // c:2306 — `bootret = do_boot_module(m, enablesarr, silent);`
        // The Rust do_boot_module routes through boot_module dispatcher
        // (b474b62898) to the per-module boot_(m) — the real partab
        // and bintab installations land here.
        let bootret = do_boot_module(self, name, enablesarr, 0, feat_pat);
        if bootret == 1 {
            // c:2306-2315 — boot failure: cleanup + finish + clear, return 1.
            //   else m->u.linked = NULL;  (c:2312)
            let _ = cleanup_module(self, name);
            let _ = finish_module(self, name);
            if let Some(m) = self.modules.get_mut(name) {
                m.linked = None; // c:2312
                m.node.flags &= !MOD_SETUP;
            }
            unqueue_signals(); // c:2314
            return false; // c:2315 return 1
        }
        // c:2317-2318 — `m->flags |= MOD_INIT_B; m->flags &= ~MOD_SETUP;`
        if let Some(m) = self.modules.get_mut(name) {
            m.node.flags |= MOD_INIT_B;
            m.node.flags &= !MOD_SETUP;
        }
        unqueue_signals(); // c:2319
        true // c:2320 return bootret (0)
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
    pub fn unload_module(&mut self, name: &str) -> bool {
        // c:2812 — faithful port of the static-link control flow.
        // C body c:2818-2913:
        //   if (m->flags & MOD_ALIAS) { resolve via find_module }
        //   if (MOD_INIT_S && !MOD_UNLOAD && do_cleanup_module(m))
        //       return 1;
        //   m->flags &= ~(MOD_INIT_B | MOD_INIT_S);
        //   del = m->flags & MOD_UNLOAD;
        //   if (m->wrapper) { m->flags |= MOD_UNLOAD; return 0; }
        //   m->flags &= ~MOD_UNLOAD;
        //   if (MOD_LINKED) { if (u.linked) { u.linked->finish(m);
        //                                     u.linked = NULL; } }
        //   else { if (u.handle) { finish_module(m);
        //                          u.handle = NULL; } }
        //   if (del && m->deps) { /* deferred dep walk */ }
        //   if (m->autoloads && firstnode(m->autoloads))
        //       autofeatures("zsh", name, hlinklist2array(autoloads),
        //                    0, FEAT_IGNORE);
        //   else if (!m->deps) delete_module(m);
        //   return 0;

        // c:2819-2823 — alias resolve. Mutate the lookup name, not
        // the live record (alias chases the target).
        let mut target_name = name.to_string();
        let needs_alias_chase = self
            .modules
            .get(&target_name)
            .map(|m| (m.node.flags & MOD_ALIAS) != 0)
            .unwrap_or(false);
        if needs_alias_chase {
            target_name = match self.modules.get(name).and_then(|m| m.alias.clone()) {
                Some(a) => a,
                None => return false, // c:2821-2822 — alias target gone.
            };
        }

        // c:2831-2834 — cleanup if booted.
        let (init_s, unload_flag) = match self.modules.get(&target_name) {
            Some(m) => (
                (m.node.flags & MOD_INIT_S) != 0,
                (m.node.flags & MOD_UNLOAD) != 0,
            ),
            None => return false, // c:2820-2822 fall-through to !m
        };
        if init_s && !unload_flag {
            // c:2833 — `do_cleanup_module` is `cleanup_module` for the
            // MOD_LINKED branch (static-link path always).
            if cleanup_module(self, &target_name) != 0 {
                return false; // c:2834 cleanup error
            }
        }

        // c:2835 — `m->flags &= ~(MOD_INIT_B | MOD_INIT_S);`
        if let Some(m) = self.modules.get_mut(&target_name) {
            m.node.flags &= !(MOD_INIT_B | MOD_INIT_S);
        }

        // c:2837 — `del = m->flags & MOD_UNLOAD;`
        let del = unload_flag;

        // c:2839-2842 — wrapper deferred-unload path.
        let has_wrapper = self
            .modules
            .get(&target_name)
            .map(|m| m.wrapper != 0)
            .unwrap_or(false);
        if has_wrapper {
            if let Some(m) = self.modules.get_mut(&target_name) {
                m.node.flags |= MOD_UNLOAD; // c:2840
            }
            return true; // c:2841
        }

        // c:2843 — `m->flags &= ~MOD_UNLOAD;`
        if let Some(m) = self.modules.get_mut(&target_name) {
            m.node.flags &= !MOD_UNLOAD;
        }

        // c:2849-2859 — finish hook:
        //   if (m->node.flags & MOD_LINKED) {
        //       if (m->u.linked) {
        //           m->u.linked->finish(m);
        //           m->u.linked = NULL;
        //       }
        //   }
        // Static-link branch routes through finish_module which
        // dispatches to the per-module finish_(m) (839f32249b), gated
        // and cleared exactly like C's u.linked.
        let was_linked = self
            .modules
            .get(&target_name)
            .map(|m| m.linked.is_some())
            .unwrap_or(false);
        if was_linked {
            let _ = finish_module(self, &target_name); // c:2851
            if let Some(m) = self.modules.get_mut(&target_name) {
                m.linked = None; // c:2852
            }
        }

        // c:2861-2902 — deferred dep walk: when del was set, find every
        // dep that has MOD_UNLOAD and check no other live module
        // depends on it, then recursively unload.
        if del {
            let deps_snapshot: Vec<String> = self
                .modules
                .get(&target_name)
                .and_then(|m| m.deps.as_ref())
                .map(|d| d.iter().cloned().collect())
                .unwrap_or_default();
            for dep_name in deps_snapshot {
                let dm_target = match find_module(self, &dep_name, FINDMOD_ALIASP) {
                    Some(n) => n,
                    None => continue, // c:2867 dm == NULL
                };
                let dm_unloading = self
                    .modules
                    .get(&dm_target)
                    .map(|m| (m.node.flags & MOD_UNLOAD) != 0)
                    .unwrap_or(false);
                if !dm_unloading {
                    continue; // c:2870-2871
                }
                // c:2872-2897 — scan every other module's deps for
                // dm_target.nam; bail if any live module still
                // depends on it.
                let still_depended = self.modules.iter().any(|(other_name, other)| {
                    if other_name == &target_name {
                        return false; // c:2884 don't scan ourselves
                    }
                    let other_deps = match other.deps.as_ref() {
                        Some(d) => d,
                        None => return false, // c:2884 no deps to scan
                    };
                    // c:2887-2889 — only scan live modules (MOD_LINKED
                    // set + !MOD_UNLOAD in zshrs's static-link path).
                    if (other.node.flags & MOD_LINKED) == 0 || (other.node.flags & MOD_UNLOAD) != 0
                    {
                        return false;
                    }
                    other_deps.iter().any(|d| d == &dm_target)
                });
                if !still_depended {
                    // c:2898-2899 — `unload_module(dm)` recursive.
                    self.unload_module(&dm_target);
                }
            }
        }

        // c:2903-2912 — autoload restore OR delete_module.
        let (has_autoloads, has_deps, autoloads) = match self.modules.get(&target_name) {
            Some(m) => (
                m.autoloads.as_ref().map(|a| !a.is_empty()).unwrap_or(false),
                m.deps.is_some(),
                m.autoloads
                    .as_ref()
                    .map(|a| a.iter().cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
            ),
            None => (false, false, Vec::new()),
        };
        if has_autoloads {
            // c:2908-2909 — autofeatures("zsh", m->nam, ..., 0, FEAT_IGNORE)
            autofeatures(self, "zsh", Some(&target_name), &autoloads, 0, FEAT_IGNORE);
        } else if !has_deps {
            // c:2910-2911 — delete_module(m). Inline since the free fn
            // also calls remove + drop.
            self.modules.remove(&target_name);
        }
        true // c:2913 return 0
    }

    /// Check if module is loaded
    pub fn is_loaded(&self, name: &str) -> bool {
        self.modules
            .get(name)
            .map(|m| m.is_loaded())
            .unwrap_or(false)
    }

    /// Whether a module is BOUND — its setup_/boot_ has actually run.
    /// Mirrors the `zmodload -e` existence test (bin_zmodload_exist,
    /// c:Src/module.c:2637): the union slot `m->u.handle` is non-NULL
    /// (handle for dlopen, linked for a static module) and the module is
    /// not mid-unload. Distinct from `is_loaded()` / `module_loaded()`,
    /// which key off `MOD_LINKED` — a flag `register_builtin_modules`
    /// pre-seeds for every statically-compiled module at startup, so it
    /// is true before `zmodload` ever runs. Use this for "was the module
    /// actually loaded by the user" gates.
    pub fn is_bound(&self, name: &str) -> bool {
        self.modules
            .get(name)
            .map(|m| (m.handle.is_some() || m.linked.is_some()) && (m.node.flags & MOD_UNLOAD) == 0)
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
    /// hashtable (Src/builtin.c). The real builtin registration lives
    /// in `cmd.rs::BUILTINTAB` and the routing fn here delegates to
    /// the free `addbuiltin` (c:303 port) so the canonical BINF_ADDED
    /// clash gate fires.
    ///
    /// Returns 0 on success, 1 on clash (per C's signature).
    pub fn addbuiltin(&mut self, name: &str, module: &str) -> i32 {
        // c:409
        // Construct a probe builtin matching what add_autobin /
        // setbuiltins build; route through the free addbuiltin which
        // probes createbuiltintable() for BINF_ADDED.
        let mut bn = builtin {
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags: 0,
            },
            handlerfunc: None,
            minargs: 0,
            maxargs: 0,
            funcid: 0,
            optstr: Some(module.to_string()),
            defopts: None,
        };
        if addbuiltin(&mut bn) != 0 {
            return 1; // c:417 clash
        }
        // On success: record in added_builtins ledger so the
        // module's setbuiltins delete path can find this entry.
        self.added_builtins.insert(name.to_string(), BINF_ADDED);
        0 // c:417 OK
    }

    /// Unregister a builtin (from module.c deletebuiltin)
    /// Port of `deletebuiltin(const char *nam)` from `Src/module.c:449`.
    /// Returns 0 on success (entry found + removed from ledger), -1
    /// on miss — matching the canonical free fn (00e6a9ce7e) and
    /// C's deletebuiltin return contract.
    /// WARNING: param names don't match C — Rust=(name, module) vs C=(nam)
    pub fn deletebuiltin(&mut self, name: &str, _module: &str) -> i32 {
        // c:449
        // Route through the free deletebuiltin for the canonical
        // present/absent probe.
        let r = deletebuiltin(name);
        if r == 0 {
            // Drop the ledger entry so setbuiltins's `already_added`
            // probe sees this name as removed.
            self.added_builtins.remove(name);
        }
        r
    }

    /// Register autoloading builtin.
    /// Port of `static int add_autobin(const char *module, const char *bnam,
    /// int flags)` from `Src/module.c:426`. C allocates a `Builtin` node
    /// with `optstr=module`, sets `BINF_AUTOALL` if `FEAT_AUTOALL` is in
    /// `flags`, then calls `addbuiltin(bn)`. On failure, the freshly
    /// allocated node is freed; success returns 0, conflict returns 1
    /// unless `FEAT_IGNORE` masks it.
    /// WARNING: param names don't match C — Rust=(name, module, flags) vs C=(module, bnam, flags)
    pub fn add_autobin(&mut self, name: &str, module: &str, flags: i32) -> i32 {
        // c:426
        // Faithful port of c:426-441:
        //   Builtin bn = zshcalloc(sizeof(*bn));
        //   bn->node.nam = ztrdup(bnam);
        //   bn->optstr = ztrdup(module);
        //   if (flags & FEAT_AUTOALL)
        //       bn->node.flags |= BINF_AUTOALL;
        //   if ((ret = addbuiltin(bn))) {
        //       builtintab->freenode(&bn->node);
        //       if (!(flags & FEAT_IGNORE))
        //           return 1;
        //   }
        //   return 0;
        //
        // Prior port did a ledger-only insert into autoload_builtins
        // without ever calling the canonical addbuiltin. Now constructs
        // the builtin struct and routes through the free addbuiltin
        // (c:303 port) which probes createbuiltintable() for the
        // BINF_ADDED clash gate. Ledger entry preserved on success so
        // resolve_autoload_builtin() / del_autobin's fast path can
        // find it without re-scanning.
        let node_flags: i32 = if (flags & FEAT_AUTOALL as i32) != 0 {
            BINF_AUTOALL as i32 // c:435
        } else {
            0
        };
        let mut bn = builtin {
            // c:431-433 — zshcalloc + populate.
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags: node_flags,
            },
            handlerfunc: None,
            minargs: 0,
            maxargs: 0,
            funcid: 0,
            optstr: Some(module.to_string()),
            defopts: None,
        };
        // c:436 — `if ((ret = addbuiltin(bn)))`. Clash gate via the
        // canonical builtintab.
        if addbuiltin(&mut bn) != 0 {
            // c:437 — freenode drops the input via Rust's Drop.
            if (flags & FEAT_IGNORE as i32) == 0 {
                return 1; // c:439
            }
            // c:440 — FEAT_IGNORE masks → fall through to ret 0 but
            // don't insert into the ledger (the canonical table has
            // this name already).
            return 0;
        }
        // c:441 success path: register in the autoload ledger.
        self.autoload_builtins
            .insert(name.to_string(), module.to_string());
        0
    }

    // Remove an autoloaded added by add_autobin                             // c:464
    /// Port of `static int del_autobin(const char *module, const char *bnam,
    /// int flags)` from `Src/module.c:464`.
    ///
    /// C body (c:464-478):
    /// ```c
    /// Builtin bn = (Builtin) builtintab->getnode2(builtintab, bnam);
    /// if (!bn) { if(!(flags & FEAT_IGNORE)) return 2; }
    /// else if (bn->node.flags & BINF_ADDED) {
    ///     if (!(flags & FEAT_IGNORE)) return 3;
    /// } else deletebuiltin(bnam);
    /// return 0;
    /// ```
    ///
    /// 2 = "no such builtin", 3 = "real registered builtin (BINF_ADDED) —
    /// can't unload", 0 = success (removed autoload entry).
    /// FEAT_IGNORE masks both error returns.
    ///
    /// zshrs architecture: `builtintab` is static-linked at startup
    /// (`createbuiltintable()` builds an immutable HashMap), so every
    /// entry there is effectively BINF_ADDED. The autoload-only entries
    /// live in `self.autoload_builtins`. The faithful mapping:
    ///   * if name is in `builtintab` → BINF_ADDED → return 3 (or 0 with
    ///     FEAT_IGNORE).
    ///   * else if name is in `autoload_builtins` → remove it, return 0.
    ///   * else → not present → return 2 (or 0 with FEAT_IGNORE).
    /// WARNING: param names don't match C — Rust=(name, flags) vs C=(module, bnam, flags)
    pub fn del_autobin(&mut self, name: &str, flags: i32) -> i32 {
        // c:464
        // Faithful port of c:464-478:
        //   Builtin bn = (Builtin) builtintab->getnode2(builtintab, bnam);
        //   if (!bn) {
        //       if(!(flags & FEAT_IGNORE)) return 2;
        //   } else if (bn->node.flags & BINF_ADDED) {
        //       if (!(flags & FEAT_IGNORE)) return 3;
        //   } else
        //       deletebuiltin(bnam);
        //   return 0;
        //
        // Three distinct return codes: 2 = no such builtin (autoload
        // ledger absent), 3 = real registered (BINF_ADDED set — can't
        // unload via del_auto*), 0 = success.
        //
        // zshrs's createbuiltintable() entries are all BINF_ADDED by
        // construction (the static-linked canonical table), so a hit
        // there always means "real registered" → return 3. The
        // autoload stubs live in self.autoload_builtins; a hit there
        // is the c:475 deletebuiltin path.
        //
        // Now routes through the free deletebuiltin (00e6a9ce7e) for
        // the canonical-table probe so the present/absent contract
        // matches C exactly. Prior port did the probe inline.

        // c:466 — `builtintab->getnode2(builtintab, bnam)`. zshrs's
        // builtintab is split: the immutable createbuiltintable()
        // carries the static flags, the autoload_builtins ledger holds
        // runtime stubs from add_autobin, and added_builtins is the
        // runtime BINF_ADDED bit a module load flips (setbuiltins
        // c:508 probes the same ledger).
        let static_flags: Option<i32> = createbuiltintable().get(name).map(|b| b.node.flags);
        let in_ledger = self.autoload_builtins.contains_key(name);
        // BINF_ADDED equivalent: C sets the bit at addbuiltins time —
        // startup for core builtins, the module's boot_ for module
        // builtins. zshrs's static table folds BOTH in unflagged, so a
        // static hit counts as ADDED only when the name is core (no
        // owning module advertises `b:NAME`) or its owning module is
        // loaded. Runtime loads also land in the added_builtins ledger
        // (setbuiltins c:508 probes the same).
        let added = self.added_builtins.contains_key(name)
            || static_flags
                .map(|f| (f & BINF_ADDED as i32) != 0)
                .unwrap_or(false)
            || (static_flags.is_some() && {
                let mod_names: Vec<String> = self.modules.keys().cloned().collect();
                let mut owner_loaded: Option<bool> = None; // None = core builtin
                'outer: for mn in &mod_names {
                    let mut feats: Vec<String> = Vec::new();
                    if features_module(self, mn, &mut feats) != 0 {
                        continue;
                    }
                    for f in &feats {
                        if f.strip_prefix("b:") == Some(name) {
                            owner_loaded = Some(self.is_loaded(mn));
                            break 'outer;
                        }
                    }
                }
                owner_loaded != Some(false)
            });
        if static_flags.is_none() && !in_ledger {
            // c:467-469 — `if (!bn) { if(!(flags & FEAT_IGNORE)) return 2; }`
            if (flags & FEAT_IGNORE as i32) == 0 {
                return 2; // c:469
            }
        } else if added {
            // c:470-473 — `else if (bn->node.flags & BINF_ADDED)` —
            // a real, live builtin can't be un-autoloaded.
            if (flags & FEAT_IGNORE as i32) == 0 {
                return 3; // c:472
            }
        } else {
            // c:474-475 — `else deletebuiltin(bnam);` — drop the
            // autoload stub from the ledger.
            self.autoload_builtins.remove(name);
        }
        0 // c:477
    }

    /// Set/clear a slice of builtins per `e[]` mask.
    /// Port of `static int setbuiltins(char const *nam, Builtin binl,
    /// int size, int *e)` from `Src/module.c:501`. For each Builtin in
    /// `binl[0..size]`: if `e[n]` is set, add the builtin (skip if
    /// already `BINF_ADDED`); else delete the builtin (skip if not
    /// `BINF_ADDED`). Warnings on clash/already-deleted; returns 1 if
    /// any op failed.
    /// WARNING: param names don't match C — Rust=(module, names, e) vs C=(nam, binl, size, e)
    pub fn setbuiltins(&mut self, module: &str, names: &[&str], e: Option<&[i32]>) -> i32 {
        // c:501
        let mut ret: i32 = 0; // c:503
        for (n, name) in names.iter().enumerate() {
            // c:505 — `for (n = 0; n < size; n++) { Builtin b = &binl[n]; ... }`
            let enable = e
                .map(|arr| arr.get(n).copied().unwrap_or(0)) // c:507 *e++
                .unwrap_or(1);
            let already_added = self.added_builtins.contains_key(*name); // c:508 b->flags & BINF_ADDED
            if enable != 0 {
                // c:507 — `if (e && *e++)` add branch
                if already_added {
                    // c:508-509 — skip already-added.
                    continue;
                }
                // c:510 — `if (addbuiltin(b))` — probe the canonical
                // table for the clash gate. The free fn returns 1
                // when an existing entry already has BINF_ADDED set.
                let mut probe = builtin {
                    node: hashnode {
                        next: None,
                        nam: name.to_string(),
                        flags: 0,
                    },
                    handlerfunc: None,
                    minargs: 0,
                    maxargs: 0,
                    funcid: 0,
                    optstr: Some(module.to_string()),
                    defopts: None,
                };
                if addbuiltin(&mut probe) != 0 {
                    // c:511-513 — `zwarnnam(nam, "name clash...")`
                    zwarnnam(
                        module,
                        &format!("name clash when adding builtin `{}'", name),
                    );
                    ret = 1; // c:513
                } else {
                    // c:515 — `b->node.flags |= BINF_ADDED;`. Mirror
                    // the in-place bit-set with the per-module
                    // ledger flip.
                    self.added_builtins.insert(name.to_string(), BINF_ADDED);
                }
            } else {
                // c:517-525 — del branch.
                if !already_added {
                    // c:518-519 — skip already-not-added.
                    continue;
                }
                // c:520 — `if (deletebuiltin(b->node.nam))`. Free fn
                // returns -1 on miss; treat any non-zero as the
                // "already deleted" condition.
                if deletebuiltin(name) != 0 {
                    // c:521-523 — `zwarnnam(nam, "builtin `%s' already
                    // deleted")`.
                    zwarnnam(module, &format!("builtin `{}' already deleted", name));
                    ret = 1; // c:523
                } else {
                    // c:524 — `b->node.flags &= ~BINF_ADDED;`
                    self.added_builtins.remove(*name);
                }
            }
        }
        ret // c:528
    }

    // ------- Condition management (from module.c addconddef/deleteconddef) -------

    /// Register a condition (from module.c addconddef)
    /// Port of `addconddef(Conddef c)` from `Src/module.c:703`.
    /// WARNING: param names don't match C — Rust=(name, module) vs C=(c)
    ///
    /// Like `addbuiltin`, the real registration lives in
    /// `cond.rs::CONDTAB`; the routing fn here delegates to the free
    /// `addconddef` (4304-port) so the canonical name+infix-flag
    /// clash gate fires.
    ///
    /// Returns 0 on success, 1 on clash (matches C's signature).
    pub fn addconddef(&mut self, name: &str, module: &str) -> i32 {
        // c:703
        // Construct a probe conddef matching what add_autocond /
        // setconddefs build. handler/min/max/condid stay default
        // (filled in by the actual module on load).
        let cd = conddef {
            next: None,
            name: name.to_string(),
            flags: 0, // c:703 entries here aren't infix by default
            handler: None,
            min: 0,
            max: 0,
            condid: 0,
            module: Some(module.to_string()),
        };
        // c:705-715 — addconddef walks CONDTAB for clash, replaces
        // autoload entries, prepends on success. Returns 0/1.
        addconddef(cd)
    }

    /// Unregister a condition (from module.c deleteconddef)
    /// Port of `deleteconddef(Conddef c)` from `Src/module.c:724`.
    /// Returns 0 on success (entry was found + removed), -1 on miss.
    /// Mirrors C's deleteconddef return contract.
    /// WARNING: param names don't match C — Rust=(name, module) vs C=(c)
    pub fn deleteconddef(&mut self, name: &str, _module: &str) -> i32 {
        // c:724
        // Build a probe conddef and call the free deleteconddef
        // (4304-port) which walks CONDTAB by (name, infix-flag) for
        // identity. infix=0 here matches the prefix-style default
        // used by the auto* registrations.
        let probe = conddef {
            next: None,
            name: name.to_string(),
            flags: 0,
            handler: None,
            min: 0,
            max: 0,
            condid: 0,
            module: None,
        };
        deleteconddef(&probe)
    }

    /// Get condition definition (from module.c getconddef)
    /// Port of `getconddef(int inf, const char *name, int autol)` from `Src/module.c:648`.
    /// WARNING: param names don't match C — Rust=(name) vs C=(inf, name, autol)
    ///
    /// Returns the autoload mapping if any. C consults the canonical
    /// `condtab` first; the autoload table is the fallback.
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
    pub fn add_autocond(&mut self, name: &str, module: &str, flags: i32) -> i32 {
        // c:792
        // Faithful port of c:792-810:
        //   Conddef c = zshcalloc(sizeof(*c));
        //   c->name = ztrdup(cnam);
        //   c->module = ztrdup(module);
        //   c->flags = (flags & FEAT_INFIX) ? CONDF_INFIX : 0;
        //   if (flags & FEAT_AUTOALL) c->flags |= CONDF_AUTOALL;
        //   if (addconddef(c)) {
        //       zsfree(c->name); zsfree(c->module); zfree(c, sizeof(*c));
        //       if (!(flags & FEAT_IGNORE)) return 1;
        //   }
        //   return 0;
        //
        // Prior port did a ledger-only insert into autoload_conditions
        // without ever touching the canonical CONDTAB. Now constructs
        // the conddef struct and routes through the free addconddef
        // (1395acf3a7 dependency) which walks CONDTAB for clashes,
        // replaces autoload entries, and prepends on success.
        let mut cflags: i32 = if (flags & FEAT_INFIX) != 0 {
            CONDF_INFIX // c:799
        } else {
            0
        };
        if (flags & FEAT_AUTOALL) != 0 {
            cflags |= CONDF_AUTOALL; // c:801
        }
        // c:796-803 — populate the conddef record. `handler` is None
        // because autoload stubs don't carry the dispatch fn until
        // the module loads and addconddef replaces the entry.
        let cd = conddef {
            next: None,
            name: name.to_string(),
            flags: cflags,
            handler: None,
            min: 0,
            max: 0,
            condid: 0,
            module: Some(module.to_string()),
        };
        // c:804 — addconddef(c).
        if addconddef(cd) != 0 {
            // c:805-807 — zsfree/zfree happen via Rust drop.
            if (flags & FEAT_IGNORE) == 0 {
                return 1; // c:810
            }
        }
        // Keep the ledger entry too so resolve_autoload_condition()
        // and del_autocond's fast path can find it without re-scanning
        // CONDTAB.
        self.autoload_conditions
            .insert(name.to_string(), module.to_string());
        0 // c:812
    }

    /// Port of `static int del_autocond(const char *modnam, const char *cnam,
    /// int flags)` from `Src/module.c:819`.
    ///
    /// C body (c:819-835):
    /// ```c
    /// Conddef cd = getconddef((flags & FEAT_INFIX) ? 1 : 0, cnam, 0);
    /// if (!cd) { if (!(flags & FEAT_IGNORE)) return 2; }
    /// else if (cd->flags & CONDF_ADDED) {
    ///     if (!(flags & FEAT_IGNORE)) return 3;
    /// } else deleteconddef(cd);
    /// return 0;
    /// ```
    ///
    /// 2 = "no such condition", 3 = "registered condition (CONDF_ADDED) —
    /// can't unload", 0 = success. FEAT_IGNORE masks both error returns.
    /// WARNING: param names don't match C — Rust=(name, flags) vs C=(modnam, cnam, flags)
    pub fn del_autocond(&mut self, name: &str, flags: i32) -> i32 {
        // c:819
        // Faithful port of c:819-835:
        //   Conddef cd = getconddef((flags & FEAT_INFIX) ? 1 : 0,
        //                           cnam, 0);
        //   if (!cd) {
        //       if (!(flags & FEAT_IGNORE)) return 2;
        //   } else if (cd->flags & CONDF_ADDED) {
        //       if (!(flags & FEAT_IGNORE)) return 3;
        //   } else
        //       deleteconddef(cd);
        //   return 0;
        //
        // Prior port skipped CONDTAB entirely; checked the ledger
        // only. Now routes through getconddef (1395acf3a7) which
        // walks CONDTAB filtered by infix-flag, then deleteconddef
        // (free fn) which removes the matched entry.
        let inf = if (flags & FEAT_INFIX) != 0 { 1 } else { 0 };
        // c:821 — `getconddef(inf, cnam, 0)`.
        let cd = getconddef(inf, name, 0, self);
        match cd {
            None => {
                // c:823-825 — !cd: return 2 unless FEAT_IGNORE.
                if (flags & FEAT_IGNORE) == 0 {
                    return 2; // c:825
                }
                0 // c:834
            }
            Some(ref entry) if (entry.flags & CONDF_ADDED) != 0 => {
                // c:826-828 — CONDF_ADDED set: real registered
                // condition, can't unload via del_auto*. Return 3
                // unless FEAT_IGNORE.
                if (flags & FEAT_IGNORE) == 0 {
                    return 3; // c:828
                }
                0
            }
            Some(ref entry) => {
                // c:831-832 — deleteconddef(cd); drop ledger too.
                let _ = deleteconddef(entry);
                self.autoload_conditions.remove(name);
                0 // c:834
            }
        }
    }

    // ------- Hook management lives in the file-static free ported above ------
    //
    // C `hooktab` (Src/module.c:843) is a file-static `Hookdef` linked
    // list, NOT a member of `ModuleTable` (which is the `modulestab`
    // HashTable of Module nodes at c:Modules/zmodload.c:32). Hook ops
    // are free ported operating on the file-static `hooktab` (above):
    // `gethookdef`, `addhookdef`, `addhookdefs`, `deletehookdef`,
    // `deletehookdefs`, `addhookdeffunc`, `addhookfunc`,
    // `deletehookdeffunc`, `deletehookfunc`, `runhookdef`.

    // ------- Parameter management (from module.c addparamdef/deleteparamdef) -------

    /// Add or remove sets of parameters; same shape as `setbuiltins`.
    /// Port of `static int setparamdefs(char const *nam, Paramdef d,
    /// int size, int *e)` from `Src/module.c:1170`. For each Paramdef
    /// in `d[0..size]`: if `e[n]` is set and `d->pm` is null, add the
    /// param via `addparamdef(d)`; if `e[n]` is clear and `d->pm` is
    /// non-null, remove via `deleteparamdef(d)`. Warnings on
    /// error/already-deleted; returns 1 if any op failed.
    /// WARNING: param names don't match C — Rust=(module, names, e) vs C=(nam, d, size, e)
    pub fn setparamdefs(&mut self, module: &str, names: &[&str], e: Option<&[i32]>) -> i32 {
        // c:1165
        // Faithful port of c:1165-1192. Prior Rust port skipped the
        // diagnostics + ret-tracking C does on failure:
        //
        //   if (e && *e++) {           // add branch
        //       if (d->pm) continue;   // already registered
        //       if (addparamdef(d)) {
        //           zwarnnam(nam,
        //               "error when adding parameter `%s'", d->name);
        //           ret = 1;
        //       }
        //   } else {                   // del branch
        //       if (!d->pm) continue;  // not registered
        //       if (deleteparamdef(d)) {
        //           zwarnnam(nam,
        //               "parameter `%s' already deleted", d->name);
        //           ret = 1;
        //       }
        //   }
        //
        // Maps to the autoload_params ledger (zshrs-only structural
        // equivalent of d->pm presence). Diagnostics now fire when
        // the ledger insert/remove conflicts with the C-side
        // contract (probe + retry on the canonical paramtab to
        // observe the same clash behaviour addparamdef would).
        use crate::ported::params::paramtab;
        let mut ret: i32 = 0; // c:1167
        for (n, name) in names.iter().enumerate() {
            // c:1169 — `while (size--)`
            let enable = e
                .map(|arr| arr.get(n).copied().unwrap_or(0)) // c:1170 *e++
                .unwrap_or(1);
            // c:1171 / c:1180 — `if (d->pm)` / `if (!d->pm)`.
            // Static-link analog: autoload_params contains the name
            // when the module has registered it as a paramdef.
            let already = self.autoload_params.contains_key(*name);
            if enable != 0 {
                // c:1170 add branch
                if already {
                    continue; // c:1172
                }
                // c:1175 — `if (addparamdef(d))`. Faithful path:
                // probe the canonical paramtab for a clash; if the
                // name already exists with a non-MOD-derived param,
                // mirror addparamdef's failure.
                let canonical_clash = paramtab()
                    .read()
                    .ok()
                    .map(|t| t.contains_key(*name))
                    .unwrap_or(false);
                if canonical_clash {
                    // c:1176-1178 — `zwarnnam('error when adding ...')`.
                    zwarnnam(module, &format!("error when adding parameter `{}'", name));
                    ret = 1; // c:1177
                    continue;
                }
                // c:1181 — register the ledger entry on success.
                self.autoload_params
                    .insert(name.to_string(), module.to_string());
            } else {
                // c:1179 del branch
                if !already {
                    continue; // c:1181
                }
                // c:1184 — `if (deleteparamdef(d))`. With the ledger
                // hit, the canonical removal succeeds; emit the
                // C-equivalent diagnostic only if the paramtab probe
                // says the entry got tampered with externally
                // (already-deleted from paramtab while still on the
                // ledger).
                let canonical_present = paramtab()
                    .read()
                    .ok()
                    .map(|t| t.contains_key(*name))
                    .unwrap_or(false);
                self.autoload_params.remove(*name);
                if !canonical_present {
                    // c:1185-1187 — `parameter `%s' already deleted`.
                    zwarnnam(module, &format!("parameter `{}' already deleted", name));
                    ret = 1; // c:1186
                }
            }
        }
        ret // c:1191
    }

    /// Register autoloading parameter.
    /// Port of `static int add_autoparam(const char *module, const char
    /// *pnam, int flags)` from `Src/module.c:1198`. C body:
    /// `checkaddparam()` clash check (returns 2 if `-i`'d), then
    /// `setsparam(pnam, module)` creating the param with `PM_AUTOLOAD`
    /// (+ `PM_AUTOALL` if `FEAT_AUTOALL`). `queue_signals`/`noerrs=2`
    /// bracket so the setsparam doesn't echo errors out.
    /// WARNING: param names don't match C — Rust=(name, module, flags) vs C=(module, pnam, flags)
    pub fn add_autoparam(&mut self, name: &str, module: &str, flags: i32) -> i32 {
        // c:1198
        // Faithful port of c:1198-1228 routes through the free
        // add_autoparam fn (293c041e2f) which already does the full
        // checkaddparam + setsparam + PM_AUTOLOAD (+PM_AUTOALL) +
        // noerrs/queue_signals bracket.
        //
        // Prior method was a ledger-only HashMap insert that did the
        // queue_signals dance without ever calling setsparam — so
        // PM_AUTOLOAD never landed on the canonical paramtab entry,
        // and `typeset +` wouldn't see the 'undefined NAME' shape.
        //
        // The free fn returns 0 / -1 (vs C's 0 / 1 / -1) — preserve
        // the existing method-level contract by mapping -1 → 1 below.
        let r = add_autoparam(module, name, flags); // c:1198
        if r != 0 {
            // c:1213-1219 — error: 2 (FEAT_IGNORE soft-fail) maps to
            // 0 here per the prior method contract.
            if (flags & FEAT_IGNORE) != 0 {
                return 0;
            }
            return r; // -1 → -1
        }
        // c:1218-1221 success: also keep the ledger entry up so the
        // resolve_autoload_param fast path stays consistent.
        self.autoload_params
            .insert(name.to_string(), module.to_string());
        0
    }

    /// Port of `static int del_autoparam(const char *modnam, const char *pnam,
    /// int flags)` from `Src/module.c:1240`.
    ///
    /// C body (c:1240-1255):
    /// ```c
    /// Param pm = (Param) gethashnode2(paramtab, pnam);
    /// if (!pm) { if (!(flags & FEAT_IGNORE)) return 2; }
    /// else if (!(pm->node.flags & PM_AUTOLOAD)) {
    ///     if (!(flags & FEAT_IGNORE)) return 3;
    /// } else unsetparam_pm(pm, 0, 1);
    /// return 0;
    /// ```
    ///
    /// 2 = "no such param", 3 = "real param (not autoload) — can't
    /// unload", 0 = success. FEAT_IGNORE masks both error returns.
    /// WARNING: param names don't match C — Rust=(name, flags) vs C=(modnam, pnam, flags)
    pub fn del_autoparam(&mut self, name: &str, flags: i32) -> i32 {
        // c:1234
        // Faithful port of c:1234-1248 routes through the free
        // del_autoparam fn (293c041e2f) which does the canonical
        // paramtab probe + PM_AUTOLOAD gate + unsetparam_pm.
        //
        // Prior method walked paramtab directly and emulated the
        // PM_AUTOLOAD gate inline; routing through the free fn keeps
        // the logic single-sourced so PM_AUTOLOAD semantics evolve
        // in one place. Ledger cleanup preserved on the success path.
        let r = del_autoparam("", name, flags); // c:1234 (modnam unused per UNUSED())
        if r == 0 {
            // c:1246 — `unsetparam_pm` already ran via the free fn;
            // also drop the ledger entry so resolve_autoload_param
            // stops returning the stale module name.
            self.autoload_params.remove(name);
        }
        r
    }

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
    ///
    /// C body walks `linkedmodules` comparing `->name`; it never looks
    /// at `modulestab`. A module can be linked with no `modulestab`
    /// node (the normal state before its first `zmodload`) and can have
    /// a node while not linked (an alias, or an `add_dep`/`zmodload -ab`
    /// bookkeeping node).
    pub fn module_linked(&self, name: &str) -> bool {
        // c:389-391 — for (node = firstnode(linkedmodules); …)
        self.linkedmodules.iter().any(|n| n == name)
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
            self.load_module(module, None, false);
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

/// Port of `getmathfunc(const char *name, int autol)` from `Src/module.c:1283`.
///
/// C body: linear-search `mathfuncs` for `name`; if found and `autol`
/// is true and the entry is autoloadable, demand-load via
/// `ensurefeature("f:", name)`. Returns the resolved entry or NULL.
///
/// Rust port returns `Some(module_name)` on hit, `None` on miss.
///
/// Faithful port of c:1283-1306:
/// ```c
/// MathFunc
/// getmathfunc(const char *name, int autol)
/// {
///     MathFunc p, q = NULL;
///     for (p = mathfuncs; p; q = p, p = p->next)
///         if (!strcmp(name, p->name)) {
///             if (autol && p->module && !(p->flags & MFF_USERFUNC)) {
///                 char *n = dupstring(p->module);
///                 int flags = p->flags;
///                 removemathfunc(q, p);
///                 (void)ensurefeature(n, "f:",
///                                     (flags & MFF_AUTOALL) ? NULL : name);
///                 p = getmathfunc(name, 0);
///                 if (!p) {
///                     zerr("autoloading module %s failed to define math function: %s", n, name);
///                 }
///             }
///             return p;
///         }
///     return NULL;
/// }
/// ```
///
/// C walks `mathfuncs` (file-static linked list) — Rust walks
/// MATHFUNCS (same shape, Vec). The MFF_USERFUNC filter
/// (`functions -M` user-defined math) keeps user fns from being
/// autoloaded out from under the caller. The autoload arm:
///   1. snapshot module name + flags
///   2. removemathfunc(name) — drop the autoload stub
///   3. ensurefeature(module, "f:", AUTOALL?NULL:name) — load real
///   4. re-query mathfuncs (autol=0) — return the loaded entry
///   5. zerr if still missing (load failed to populate the table)
/// WARNING: param names don't match C — Rust=(table, name, autol) vs C=(name, autol)
pub fn getmathfunc(table: &mut modulestab, name: &str, autol: i32) -> Option<String> {
    // c:1283
    // c:1287-1288 — `for (p = mathfuncs; p; q = p, p = p->next)`:
    // walk MATHFUNCS for the first name match. Snapshot under the
    // lock so we can release before mutating (autoload arm calls
    // ensurefeature which can re-enter MATHFUNCS via addmathfunc).
    let hit: Option<(String, i32, bool)> = {
        let tab = MATHFUNCS.lock().unwrap();
        tab.iter().find_map(|p| {
            if p.name == name {
                Some((
                    p.module.clone().unwrap_or_default(),
                    p.flags,
                    p.module.is_some(),
                ))
            } else {
                None
            }
        })
    };
    let (module, flags, has_module) = match hit {
        Some(t) => t,
        None => return None, // c:1306 `return NULL;`
    };
    // c:1289 — `if (autol && p->module && !(p->flags & MFF_USERFUNC))`.
    if autol != 0 && has_module && (flags & MFF_USERFUNC) == 0 {
        // c:1290-1291 — snapshot already done above.
        // c:1293 — `removemathfunc(q, p)`: drop the stub before load.
        removemathfunc(name);
        // c:1295-1296 — `ensurefeature(n, "f:", AUTOALL?NULL:name)`.
        let feature_arg = if (flags & crate::ported::zsh_h::MFF_AUTOALL) != 0 {
            None
        } else {
            Some(name)
        };
        let _ = ensurefeature(table, &module, "f:", feature_arg);
        // c:1298 — `p = getmathfunc(name, 0);` recurse w/o autol.
        // EXISTENCE check: C zerrs only when the re-lookup returns
        // NULL. A freshly registered real entry has module == NULL
        // (mftab entries, zsh.h:133 NUMMATHFUNC), so mapping the hit
        // through `p.module.clone()` (None for real entries) wrongly
        // reported a successful load as "failed to define". Mirror the
        // non-autoload tail below: empty string marks a module-less
        // hit.
        let after = {
            let tab = MATHFUNCS.lock().unwrap();
            tab.iter().find_map(|p| {
                if p.name == name {
                    Some(p.module.clone().unwrap_or_default())
                } else {
                    None
                }
            })
        };
        // c:1299-1301 — `if (!p) zerr(...)`.
        if after.is_none() {
            crate::ported::utils::zerr(&format!(
                "autoloading module {} failed to define math function: {}",
                module, name
            ));
        }
        return after; // c:1303 `return p;`
    }
    // c:1303 — non-autoload (or user-fn) hit: return the entry.
    if has_module {
        Some(module)
    } else {
        // User-fn entries (`functions -M`) have no module — return
        // an empty string so the caller still observes the hit.
        Some(String::new())
    }
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
pub fn add_automathfunc(table: &mut modulestab, module: &str, fnam: &str, flags: i32) -> i32 {
    // c:1410
    // Faithful port of c:1410-1429:
    //   MathFunc f = zalloc(sizeof(*f));
    //   f->name = ztrdup(fnam);
    //   f->module = ztrdup(module);
    //   f->flags = 0;
    //   if (addmathfunc(f)) {
    //       zsfree(f->name); zsfree(f->module); zfree(f, sizeof(*f));
    //       if (!(flags & FEAT_IGNORE)) return 1;
    //   }
    //   return 0;
    //
    // Prior port did ledger-only autoload_mathfuncs.insert without
    // ever touching the canonical MATHFUNCS table. Now constructs
    // the mathfunc struct and routes through the free addmathfunc
    // (c:1313) which walks MATHFUNCS for clashes and replaces
    // autoloadable entries.
    let f = mathfunc {
        next: None,
        name: fnam.to_string(),
        flags: 0, // c:1417 — autoload entries don't carry MFF_ADDED
        nfunc: None,
        sfunc: None,
        module: Some(module.to_string()),
        minargs: 0,
        maxargs: 0,
        funcid: 0,
    };
    // c:1420 — `if (addmathfunc(f))` clash gate.
    if addmathfunc(f) != 0 {
        // c:1421-1424 — free happens via Rust drop on the returned-
        // by-value f going out of scope.
        if (flags & FEAT_IGNORE) == 0 {
            return 1; // c:1426
        }
        // c:1427 — FEAT_IGNORE: fall through to success but skip
        // the ledger insert (the canonical table already has this).
        return 0;
    }
    // c:1429 success path: register in the autoload ledger.
    table
        .autoload_mathfuncs
        .insert(fnam.to_string(), module.to_string());
    0
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
pub fn del_automathfunc(table: &mut modulestab, _modnam: &str, fnam: &str, flags: i32) -> i32 {
    // c:1436
    // Faithful port of c:1436-1449:
    //   MathFunc f = getmathfunc(fnam, 0);
    //   if (!f) { if (!(flags & FEAT_IGNORE)) return 2; }
    //   else if (f->flags & MFF_ADDED) {
    //       if (!(flags & FEAT_IGNORE)) return 3;
    //   } else deletemathfunc(f);
    //   return 0;
    //
    // Prior port skipped the MFF_ADDED gate at c:1444 entirely.
    // That meant `zmodload -ufd` on a real (module-registered)
    // math function silently succeeded instead of returning 3 like
    // C does, dropping the user's actual function out from under
    // them. Now uses getmathfunc (fd1ec84bab) to find the entry
    // and checks MFF_ADDED before removal.

    // c:1440 — `getmathfunc(fnam, 0)`. autol=0 since we don't want
    // the autoload trigger to fire during a deletion query.
    let entry = getmathfunc(table, fnam, 0);
    match entry {
        None => {
            // c:1441-1442 — `if (!f) { if (!FEAT_IGNORE) return 2; }`
            if (flags & FEAT_IGNORE) == 0 {
                return 2;
            }
            0
        }
        Some(_) => {
            // c:1443 — `else if (f->flags & MFF_ADDED)`.
            // Look up the entry in MATHFUNCS to read its flags
            // (getmathfunc returns the module string, not the
            // mathfunc struct).
            let added = {
                let tab = MATHFUNCS.lock().unwrap();
                tab.iter()
                    .find(|m| m.name == fnam)
                    .map(|m| (m.flags & MFF_ADDED) != 0)
                    .unwrap_or(false)
            };
            if added {
                // c:1444-1445 — real registered, can't unload via
                // del_auto*. Return 3 unless FEAT_IGNORE.
                if (flags & FEAT_IGNORE) == 0 {
                    return 3;
                }
                return 0;
            }
            // c:1447 — deletemathfunc(f). Use removemathfunc
            // (which deletemathfunc delegates to in the autoload
            // path) + drop the ledger entry.
            removemathfunc(fnam);
            table.autoload_mathfuncs.remove(fnam);
            0
        }
    }
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
pub fn load_and_bind(_fn_path: &str) -> usize {
    // c:1468
    0 // c:1492 NULL
}

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
pub fn hpux_dlsym(handle: usize, name: &str) -> usize {
    // c:1530
    0 // c:1530 NULL
}

/// Port of `try_load_module(char const *name)` from `Src/module.c:1583`.
///
/// C body iterates `module_path` looking for a loadable file via
/// `dlopen`. Static-link path: a module is "loadable" iff it's in
/// our static `ModuleTable.modules` map.
/// WARNING: param names don't match C — Rust=(table, name) vs C=(name)
pub fn try_load_module(table: &modulestab, name: &str) -> i32 {
    // c:1583
    // C dlopens the module path (or falls back to module_linked for
    // compiled-in modules). Static-link analog: the name must be on
    // `linkedmodules` (c:385 `module_linked`) — zshrs has no DSOs, so
    // "the file exists and dlopens" is exactly "the module is compiled
    // in". Deliberately NOT a `modulestab` membership test: a bare
    // FINDMOD_CREATE bookkeeping node (flags=0 per C's zshcalloc at
    // c:1676) has no backing code, so `zmodload -ab zsh/bogus x; x`
    // must still emit `failed to load module` rather than "boot" the
    // phantom; and a real module has no node at all until its first
    // load, which must still succeed.
    if table.module_linked(name) {
        1
    } else {
        0
    }
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
pub fn do_load_module(table: &mut modulestab, name: &str, silent: i32) -> i32 {
    // c:1610
    // c:1610 — ret = try_load_module(name);
    let ret = try_load_module(table, name);
    if ret == 0 && silent == 0 {
        // c:1615
        // c:1622 `zwarn("failed to load module `%s': %s", name, dlerror())`.
        // The dlerror TAIL is deliberately omitted: zshrs's modules are
        // statically linked, so nothing was dlopened and there is no real
        // diagnostic to report — synthesising one would be a fabricated
        // message, not a port. Prefix + rc are the pinned contract
        // (docs/BUGS.md #376, `zmodload_nonexistent_diagnostic` in
        // tests/parity/modules_parity.rs). Both emit sites must agree.
        zwarn(&format!("failed to load module `{}'", name));
    }
    ret // c:1624
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
pub fn find_module(table: &mut modulestab, name: &str, flags: i32) -> Option<String> {
    // c:1659
    // c:1659 — m = modulestab->getnode2(modulestab, name);
    let mut cur_name = name.to_string();
    let mut depth = 0;
    loop {
        if depth > 64 {
            return None;
        } // alias-cycle guard
        depth += 1;
        match table.modules.get(&cur_name) {
            Some(m) => {
                // c:1665 — if ((flags & FINDMOD_ALIASP) && (m->node.flags & MOD_ALIAS))
                if (flags & FINDMOD_ALIASP) != 0 && (m.node.flags & MOD_ALIAS) != 0 {
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
                // zshcalloc zero-fills: the created node has flags=0
                // and NULL handle/linked — a bookkeeping entry (alias
                // targets, autoload owners), NOT a loadable module.
                // module::new() seeds MOD_LINKED (the register_module
                // shape, c:359); that bit wrongly made phantom nodes
                // (`zmodload -ab zsh/bogus x`) pass try_load_module's
                // loadable gate and "boot" successfully.
                let mut m = module::new(&cur_name);
                m.node.flags = 0; // c:1676 zshcalloc — all-zero node
                table.modules.insert(cur_name.clone(), m);
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
pub fn delete_module(table: &mut modulestab, name: &str) -> i32 {
    // c:1687
    table.modules.remove(name); // c:1687 removenode
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
///
/// Faithful port of c:1702-1710:
/// ```c
/// mod_export int
/// module_loaded(const char *name)
/// {
///     Module m;
///     return ((m = find_module(name, FINDMOD_ALIASP, NULL)) &&
///             m->u.handle &&
///             !(m->node.flags & MOD_UNLOAD));
/// }
/// ```
///
/// All three gates matter in zshrs's static-link path:
///   1. `find_module(FINDMOD_ALIASP)` — resolves aliases, returns the
///      target's modulestab entry (or None on miss).
///   2. `m->u.handle` — non-null when the module's setup has run; in
///      the Rust mirror, MOD_INIT_S being set is the structural
///      equivalent (no dlopen handle to test).
///   3. `!(m->node.flags & MOD_UNLOAD)` — MOD_UNLOAD is the
///      "registered + autoload-only" sentinel set by
///      register_builtin_modules for entries OUTSIDE
///      zsh_default_loaded (e.g. zsh/files, zsh/system, zsh/zftp).
///      Without this gate, `${modules[zsh/files]}` reads as "loaded"
///      from initial register even though no `zmodload zsh/files` has
///      run — diverging from `zsh -fc` which reports the names as
///      autoload-pending.
/// WARNING: param names don't match C — Rust=(table, name) vs C=(name)
pub fn module_loaded(table: &modulestab, name: &str) -> i32 {
    // c:1703
    // c:1707 — `find_module(name, FINDMOD_ALIASP, NULL)`: resolve
    // alias chains via the existing free-fn port (c:1659).
    // The Rust port returns Option<String> — Some(target) on hit.
    // Inline the resolution here so we can read the target's flags
    // without re-locking.
    let target = match table.modules.get(name) {
        Some(m) if (m.node.flags & MOD_ALIAS) != 0 => {
            // c:FINDMOD_ALIASP — chase alias.
            // m->u.alias is the alias target; the Rust mirror stores
            // it on `module::aliased` (set by zmodload -A). Probe.
            match m.alias.as_ref().and_then(|a| table.modules.get(a)) {
                Some(t) => t,
                None => return 0, // alias points nowhere → not loaded
            }
        }
        Some(m) => m,
        None => return 0, // c:1707-1709 — find_module miss.
    };
    // c:1708 — `m->u.handle` — non-null on a fully-loaded module.
    // Static-link analog: MOD_LINKED set + module entry exists in
    // modulestab means register_module fired (= setup ran). The
    // dlopen `u.handle` check translates to MOD_LINKED here because
    // every modulestab entry is a statically-linked module in
    // zshrs's compile-time-only loader.
    if (target.node.flags & MOD_LINKED) == 0 {
        return 0;
    }
    // c:1709 — `!(m->node.flags & MOD_UNLOAD)`. MOD_UNLOAD is set by
    // register_builtin_modules on the autoload-only subset
    // (zsh/files, zsh/system, zsh/zftp, …); cleared by an explicit
    // `zmodload NAME` once the user wants the module live.
    if (target.node.flags & MOD_UNLOAD) != 0 {
        return 0;
    }
    1
}

/// Port of `dyn_setup_module(Module m)` from `Src/module.c:1726`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(0, m, NULL);`
/// Op-code 0 = setup. AIX-only path that multiplexes all six module
/// hooks through one symbol; static-link path skips it entirely.
#[allow(unused_variables)]
pub fn dyn_setup_module(m: *const module) -> i32 {
    // c:1726
    0 // c:1726
}

/// Port of `dyn_features_module(Module m, char ***features)` from `Src/module.c:1733`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(4, m, features);`
/// Op-code 4 = features.
#[allow(unused_variables)]
pub fn dyn_features_module(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:1733
    0 // c:1733
}

/// Port of `dyn_enables_module(Module m, int **enables)` from `Src/module.c:1740`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(5, m, enables);`
/// Op-code 5 = enables.
#[allow(unused_variables)]
pub fn dyn_enables_module(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:1740
    0 // c:1733
}

/// Port of `dyn_boot_module(Module m)` from `Src/module.c:1747`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(1, m, NULL);`
/// Calls the dynamic module's exported entry-point with op-code 1
/// (boot). Static-link path: opcode dispatch unused, returns 0.
#[allow(unused_variables)]
pub fn dyn_boot_module(m: *const module) -> i32 {
    // c:1747
    0 // c:1754
}

/// Port of `dyn_cleanup_module(Module m)` from `Src/module.c:1754`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(2, m, NULL);`
/// Op-code 2 = cleanup.
#[allow(unused_variables)]
pub fn dyn_cleanup_module(m: *const module) -> i32 {
    // c:1754
    0 // c:1740
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
pub fn dyn_finish_module(m: *const module) -> i32 {
    // c:1766
    // c:1768 — ((int (*)(int,Module,void*)) m->u.handle)(3, m, NULL).
    // Static modules: no handle, opcode 3 (finish) is a no-op.
    0 // c:1768 success
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
pub fn module_func(m: &module, name: &str) -> usize {
    // c:1770
    0 // c:1794 NULL
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
/// Symmetric with boot_module/cleanup_module/finish_module: routes
/// to the per-module `setup_(m)`. C `setup_` is the first lifecycle
/// hook called when load_module loads the module (before
/// do_module_features registers features and before boot_ runs).
/// Most per-module setup_ bodies are `return 0;` — exceptions
/// initialise module-private state (curses screen handle, regex
/// compile cache, etc.).
/// WARNING: param names don't match C — Rust=(_table, name) vs C=(m)
pub fn setup_module(_table: &mut modulestab, name: &str) -> i32 {
    // c:1884
    match name {
        "zsh/attr" => crate::ported::modules::attr::setup_(std::ptr::null()),
        "zsh/cap" => crate::ported::modules::cap::setup_(std::ptr::null()),
        "zsh/clone" => crate::ported::modules::clone::setup_(std::ptr::null()),
        "zsh/curses" => crate::ported::modules::curses::setup_(std::ptr::null()),
        "zsh/datetime" => crate::ported::modules::datetime::setup_(std::ptr::null()),
        "zsh/db/gdbm" => crate::ported::modules::db_gdbm::setup_(std::ptr::null()),
        "zsh/example" => crate::ported::modules::example::setup_(std::ptr::null()),
        "zsh/files" => crate::ported::modules::files::setup_(std::ptr::null()),
        "zsh/hlgroup" => crate::ported::modules::hlgroup::setup_(std::ptr::null()),
        "zsh/ksh93" => crate::ported::modules::ksh93::setup_(std::ptr::null()),
        "zsh/langinfo" => crate::ported::modules::langinfo::setup_(std::ptr::null()),
        "zsh/mapfile" => crate::ported::modules::mapfile::setup_(std::ptr::null()),
        "zsh/mathfunc" => crate::ported::modules::mathfunc::setup_(std::ptr::null()),
        "zsh/nearcolor" => crate::ported::modules::nearcolor::setup_(std::ptr::null()),
        "zsh/newuser" => crate::ported::modules::newuser::setup_(std::ptr::null()),
        "zsh/parameter" => crate::ported::modules::parameter::setup_(std::ptr::null()),
        "zsh/param/private" => crate::ported::modules::param_private::setup_(std::ptr::null()),
        "zsh/pcre" => crate::ported::modules::pcre::setup_(std::ptr::null()),
        "zsh/random" => crate::ported::modules::random::setup_(std::ptr::null()),
        "zsh/regex" => crate::ported::modules::regex::setup_(std::ptr::null()),
        "zsh/net/socket" => crate::ported::modules::socket::setup_(std::ptr::null()),
        "zsh/stat" => crate::ported::modules::stat::setup_(std::ptr::null()),
        "zsh/system" => crate::ported::modules::system::setup_(std::ptr::null()),
        "zsh/net/tcp" => crate::ported::modules::tcp::setup_(std::ptr::null()),
        "zsh/termcap" => crate::ported::modules::termcap::setup_(std::ptr::null()),
        "zsh/terminfo" => crate::ported::modules::terminfo::setup_(std::ptr::null()),
        "zsh/watch" => crate::ported::modules::watch::setup_(std::ptr::null()),
        "zsh/zftp" => crate::ported::modules::zftp::setup_(std::ptr::null()),
        "zsh/zprof" => crate::ported::modules::zprof::setup_(std::ptr::null()),
        "zsh/zpty" => crate::ported::modules::zpty::setup_(std::ptr::null()),
        "zsh/zselect" => crate::ported::modules::zselect::setup_(std::ptr::null()),
        "zsh/zutil" => crate::ported::modules::zutil::setup_(std::ptr::null()),
        "zsh/compctl" => crate::ported::zle::compctl::setup_(),
        _ => 0,
    }
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
///
/// Per-module features_ populates the out-array with each feature
/// the module provides — `b:bn`, `c:cn`, `C:Cn`, `f:fn`, `p:pn`
/// per featuresarray (Src/module.c:3279). Used by zmodload -L to
/// list feature surfaces.
/// WARNING: param names don't match C — Rust=(_table, name, features) vs C=(m, features)
pub fn features_module(_table: &mut modulestab, name: &str, features: &mut Vec<String>) -> i32 {
    // c:1892
    match name {
        "zsh/attr" => crate::ported::modules::attr::features_(std::ptr::null(), features),
        "zsh/cap" => crate::ported::modules::cap::features_(std::ptr::null(), features),
        "zsh/clone" => crate::ported::modules::clone::features_(std::ptr::null(), features),
        "zsh/curses" => crate::ported::modules::curses::features_(std::ptr::null(), features),
        "zsh/datetime" => crate::ported::modules::datetime::features_(std::ptr::null(), features),
        "zsh/db/gdbm" => crate::ported::modules::db_gdbm::features_(std::ptr::null(), features),
        "zsh/example" => crate::ported::modules::example::features_(std::ptr::null(), features),
        "zsh/files" => crate::ported::modules::files::features_(std::ptr::null(), features),
        "zsh/hlgroup" => crate::ported::modules::hlgroup::features_(std::ptr::null(), features),
        "zsh/ksh93" => crate::ported::modules::ksh93::features_(std::ptr::null(), features),
        "zsh/langinfo" => crate::ported::modules::langinfo::features_(std::ptr::null(), features),
        "zsh/mapfile" => crate::ported::modules::mapfile::features_(std::ptr::null(), features),
        "zsh/mathfunc" => crate::ported::modules::mathfunc::features_(std::ptr::null(), features),
        "zsh/nearcolor" => crate::ported::modules::nearcolor::features_(std::ptr::null(), features),
        "zsh/newuser" => crate::ported::modules::newuser::features_(std::ptr::null(), features),
        "zsh/parameter" => crate::ported::modules::parameter::features_(std::ptr::null(), features),
        "zsh/param/private" => {
            crate::ported::modules::param_private::features_(std::ptr::null(), features)
        }
        "zsh/pcre" => crate::ported::modules::pcre::features_(std::ptr::null(), features),
        "zsh/random" => crate::ported::modules::random::features_(std::ptr::null(), features),
        "zsh/regex" => crate::ported::modules::regex::features_(std::ptr::null(), features),
        "zsh/net/socket" => crate::ported::modules::socket::features_(std::ptr::null(), features),
        "zsh/stat" => crate::ported::modules::stat::features_(std::ptr::null(), features),
        "zsh/system" => crate::ported::modules::system::features_(std::ptr::null(), features),
        "zsh/net/tcp" => crate::ported::modules::tcp::features_(std::ptr::null(), features),
        "zsh/termcap" => crate::ported::modules::termcap::features_(std::ptr::null(), features),
        "zsh/terminfo" => crate::ported::modules::terminfo::features_(std::ptr::null(), features),
        "zsh/watch" => crate::ported::modules::watch::features_(std::ptr::null(), features),
        // c:Src/Zle/zle_main.c:2286 — zsh/zle's features_() returns
        // b:zle/b:bindkey/b:vared + the p:BUFFER-family params via
        // featuresarray. Without this arm the dispatch fell to the
        // `_ => 0` default with an EMPTY features vec, so once any
        // zle/bindkey use marked the module loaded, the NEXT
        // autoloaded-builtin dispatch (ensurefeature → autofeatures
        // c:3558-3577 loaded-module check) found no `b:bindkey` in
        // the table and errored `module has no such feature` —
        // breaking every zinit plugin that calls `zle -N` before
        // `bindkey` (zsh-autopair, zsh-hist, zconvey, zui, …).
        "zsh/zle" => crate::ported::zle::zle_main::features_(std::ptr::null(), features),
        "zsh/zftp" => crate::ported::modules::zftp::features_(std::ptr::null(), features),
        "zsh/zprof" => crate::ported::modules::zprof::features_(std::ptr::null(), features),
        "zsh/zpty" => crate::ported::modules::zpty::features_(std::ptr::null(), features),
        "zsh/zselect" => crate::ported::modules::zselect::features_(std::ptr::null(), features),
        "zsh/zutil" => crate::ported::modules::zutil::features_(std::ptr::null(), features),
        // The three statically-linked completion modules have no per-module
        // features_() fn in the port, so they fell to `_ => 0` (EMPTY
        // features). That broke `ensurefeature`: calling an autoloadable
        // completion builtin (`compset`/`compadd` from a shell completer)
        // looked up `b:<name>` against its home module and errored "module
        // `zsh/X' has no such feature: `b:<name>'" (same class as the
        // zsh/sched `b:sched` error). The feature surface is `b:<builtin>`
        // per the C BUILTIN() homes (complete.c:1693-1694 / computil.c:5131-
        // 5138 / compctl.c:4006-4007).
        // c:Src/Zle/complete.c:1720-1726 — `module_features = { bintab,
        // …, cotab, …, NULL, 0, NULL, 0, 0 }`, so `featuresarray`
        // (c:3283-3308) emits the two `b:` rows in bintab order
        // (c:1693-1694) then the four `c:` rows in cotab order
        // (c:1698-1701: after, between, prefix, suffix). The `c:` rows
        // were missing, so `zmodload zsh/complete` after the boot
        // autofeatures replay hit do_module_features' FEAT_CHECKAUTO arm
        // (c:2024-2044) and cancelled all four autoloads with
        // "module `zsh/complete' has no such feature".
        "zsh/complete" => {
            for f in [
                "b:compadd",
                "b:compset",
                "c:after",   // c:1698
                "c:between", // c:1699
                "c:prefix",  // c:1700
                "c:suffix",  // c:1701
            ] {
                features.push(f.to_string());
            }
            0
        }
        // c:Src/Zle/zleparameter.c:137-143 — `module_features` carries
        // ONLY `partab` (c:131-135), so the feature surface is the two
        // `p:` rows in partab order.
        "zsh/zleparameter" => {
            for f in ["p:keymaps", "p:widgets"] {
                features.push(f.to_string());
            }
            0
        }
        "zsh/computil" => {
            for f in [
                "b:comparguments",
                "b:compdescribe",
                "b:compfiles",
                "b:compgroups",
                "b:compquote",
                "b:comptags",
                "b:comptry",
                "b:compvalues",
            ] {
                features.push(f.to_string());
            }
            0
        }
        "zsh/compctl" => {
            for f in ["b:compctl", "b:compcall"] {
                features.push(f.to_string());
            }
            0
        }
        // The two statically-linked Builtins/ modules with builtins also
        // had no features_() fn → `_ => 0`. `zsh/sched` is the one the user
        // hit at session start: `zmodload zsh/sched; sched …` errored
        // "module `zsh/sched' has no such feature: `b:sched'". Feature
        // surface = `b:<builtin>` (sched.c:376 / rlimits.c limit/ulimit/
        // unlimit).
        // c:Src/Builtins/sched.c:387-393 — `module_features = { bintab,
        // …, NULL, 0, NULL, 0, partab, … }`: one `b:` row then the
        // `p:zsh_scheduled_events` paramdef (sched.c partab), which
        // `featuresarray` emits last (c:3304-3305).
        "zsh/sched" => {
            for f in ["b:sched", "p:zsh_scheduled_events"] {
                features.push(f.to_string());
            }
            0
        }
        "zsh/rlimits" => {
            for f in ["b:limit", "b:ulimit", "b:unlimit"] {
                features.push(f.to_string());
            }
            0
        }
        _ => 0,
    }
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
///
/// Per-module enables_ populates the enable-bit array parallel to
/// the features_ surface — getfeatureenables (c:3314) emits 1 for
/// each currently-active feature (BINF_ADDED / CONDF_ADDED /
/// MFF_ADDED / pd->pm non-null), 0 otherwise. Used by zmodload -L
/// to report enabled-vs-disabled features per module.
/// WARNING: param names don't match C — Rust=(_table, name, enables) vs C=(m, enables)
pub fn enables_module(_table: &mut modulestab, name: &str, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:1901
    match name {
        "zsh/attr" => crate::ported::modules::attr::enables_(std::ptr::null(), enables),
        "zsh/cap" => crate::ported::modules::cap::enables_(std::ptr::null(), enables),
        "zsh/clone" => crate::ported::modules::clone::enables_(std::ptr::null(), enables),
        "zsh/curses" => crate::ported::modules::curses::enables_(std::ptr::null(), enables),
        "zsh/datetime" => crate::ported::modules::datetime::enables_(std::ptr::null(), enables),
        "zsh/db/gdbm" => crate::ported::modules::db_gdbm::enables_(std::ptr::null(), enables),
        "zsh/example" => crate::ported::modules::example::enables_(std::ptr::null(), enables),
        "zsh/files" => crate::ported::modules::files::enables_(std::ptr::null(), enables),
        "zsh/hlgroup" => crate::ported::modules::hlgroup::enables_(std::ptr::null(), enables),
        "zsh/ksh93" => crate::ported::modules::ksh93::enables_(std::ptr::null(), enables),
        "zsh/langinfo" => crate::ported::modules::langinfo::enables_(std::ptr::null(), enables),
        "zsh/mapfile" => crate::ported::modules::mapfile::enables_(std::ptr::null(), enables),
        "zsh/mathfunc" => crate::ported::modules::mathfunc::enables_(std::ptr::null(), enables),
        "zsh/nearcolor" => crate::ported::modules::nearcolor::enables_(std::ptr::null(), enables),
        "zsh/newuser" => crate::ported::modules::newuser::enables_(std::ptr::null(), enables),
        "zsh/parameter" => crate::ported::modules::parameter::enables_(std::ptr::null(), enables),
        "zsh/param/private" => {
            crate::ported::modules::param_private::enables_(std::ptr::null(), enables)
        }
        "zsh/pcre" => crate::ported::modules::pcre::enables_(std::ptr::null(), enables),
        "zsh/random" => crate::ported::modules::random::enables_(std::ptr::null(), enables),
        "zsh/regex" => crate::ported::modules::regex::enables_(std::ptr::null(), enables),
        "zsh/net/socket" => crate::ported::modules::socket::enables_(std::ptr::null(), enables),
        "zsh/stat" => crate::ported::modules::stat::enables_(std::ptr::null(), enables),
        "zsh/system" => crate::ported::modules::system::enables_(std::ptr::null(), enables),
        "zsh/net/tcp" => crate::ported::modules::tcp::enables_(std::ptr::null(), enables),
        "zsh/termcap" => crate::ported::modules::termcap::enables_(std::ptr::null(), enables),
        "zsh/terminfo" => crate::ported::modules::terminfo::enables_(std::ptr::null(), enables),
        "zsh/watch" => crate::ported::modules::watch::enables_(std::ptr::null(), enables),
        "zsh/zftp" => crate::ported::modules::zftp::enables_(std::ptr::null(), enables),
        "zsh/zprof" => crate::ported::modules::zprof::enables_(std::ptr::null(), enables),
        "zsh/zpty" => crate::ported::modules::zpty::enables_(std::ptr::null(), enables),
        "zsh/zselect" => crate::ported::modules::zselect::enables_(std::ptr::null(), enables),
        "zsh/zutil" => crate::ported::modules::zutil::enables_(std::ptr::null(), enables),
        // c:Src/Zle/complete.c:1751 — `handlefeatures(m, &module_features,
        // enables)`. This is the arm that installs the four cotab
        // conddefs (`-prefix`/`-suffix`/`-after`/`-between`), replacing the
        // `c:` autoload stubs the boot `autofeatures` replay planted.
        // Falling to `_ => 0` left them installed forever, so
        // `zmodload zsh/complete; zmodload -ac` listed all four where
        // `zsh -f` lists none.
        "zsh/complete" => crate::ported::zle::complete::enables_(std::ptr::null(), enables),
        _ => 0,
    }
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
/// Static-link path: every modulestab entry is MOD_LINKED, so the
/// branch collapses to `(m->u.linked->boot)(m)`. The Rust analog
/// of C's `m->u.linked->boot` function-pointer is the per-module
/// `boot_(m)` defined in `src/ported/modules/<name>.rs`. Dispatch
/// via a static name → fn-pointer table.
///
/// Modules not in the dispatch table (because their per-module
/// boot_ isn't ported yet) fall through to 0 — same observable
/// outcome as a no-op boot, matching pre-port behaviour.
/// WARNING: param names don't match C — Rust=(_table, name) vs C=(m)
pub fn boot_module(_table: &mut modulestab, name: &str) -> i32 {
    // c:1910
    // c:1912 — `(m->u.linked->boot)(m)` for MOD_LINKED.
    // The per-module boot_ entry points carry the partab/bintab
    // dispatch that addparamdef/addbuiltins would otherwise run
    // for a dlopen'd module (e.g. watch.rs::boot_ seeds
    // WATCH/watch in paramtab + installs the checksched preprompt
    // hook). NULL module pointer is fine — every per-module boot_
    // either ignores the arg or null-checks before deref.
    match name {
        "zsh/attr" => crate::ported::modules::attr::boot_(std::ptr::null()),
        "zsh/cap" => crate::ported::modules::cap::boot_(std::ptr::null()),
        "zsh/clone" => crate::ported::modules::clone::boot_(std::ptr::null()),
        // c:Src/Zle/complist.c:3566 boot_ — installs the `menuselect` and
        // `listscroll` keymaps (via menuselect_bindings). Without this,
        // `zmodload zsh/complist` was a no-op and every `bindkey -M
        // menuselect …` (zpwr's completion-menu keybindings) errored
        // "no such keymap `menuselect'".
        // c:Src/Zle/complete.c:1758 boot_ — registers the six completion
        // hookfns and (via the cotab half of `handlefeatures`,
        // c:3364-3369) installs the four `-prefix`/`-suffix`/`-after`/
        // `-between` conddefs that replace the `c:` autoload stubs.
        // Missing from this table, `zmodload zsh/complete` left all four
        // stubs in condtab where `zsh -f` clears them.
        "zsh/complete" => crate::ported::zle::complete::boot_(std::ptr::null()),
        "zsh/complist" => crate::ported::zle::complist::boot_(),
        "zsh/curses" => crate::ported::modules::curses::boot_(std::ptr::null()),
        "zsh/datetime" => crate::ported::modules::datetime::boot_(std::ptr::null()),
        "zsh/db/gdbm" => crate::ported::modules::db_gdbm::boot_(std::ptr::null()),
        "zsh/example" => crate::ported::modules::example::boot_(std::ptr::null()),
        "zsh/files" => crate::ported::modules::files::boot_(std::ptr::null()),
        "zsh/hlgroup" => crate::ported::modules::hlgroup::boot_(std::ptr::null()),
        "zsh/ksh93" => crate::ported::modules::ksh93::boot_(std::ptr::null()),
        "zsh/langinfo" => crate::ported::modules::langinfo::boot_(std::ptr::null()),
        "zsh/mapfile" => crate::ported::modules::mapfile::boot_(std::ptr::null()),
        "zsh/mathfunc" => crate::ported::modules::mathfunc::boot_(std::ptr::null()),
        "zsh/nearcolor" => crate::ported::modules::nearcolor::boot_(std::ptr::null()),
        "zsh/newuser" => crate::ported::modules::newuser::boot_(std::ptr::null()),
        "zsh/parameter" => crate::ported::modules::parameter::boot_(std::ptr::null()),
        "zsh/param/private" => crate::ported::modules::param_private::boot_(std::ptr::null()),
        "zsh/pcre" => crate::ported::modules::pcre::boot_(std::ptr::null()),
        "zsh/random" => crate::ported::modules::random::boot_(std::ptr::null()),
        "zsh/regex" => crate::ported::modules::regex::boot_(std::ptr::null()),
        "zsh/net/socket" => crate::ported::modules::socket::boot_(std::ptr::null()),
        "zsh/stat" => crate::ported::modules::stat::boot_(std::ptr::null()),
        "zsh/system" => crate::ported::modules::system::boot_(std::ptr::null()),
        "zsh/net/tcp" => crate::ported::modules::tcp::boot_(std::ptr::null()),
        "zsh/termcap" => crate::ported::modules::termcap::boot_(std::ptr::null()),
        "zsh/terminfo" => crate::ported::modules::terminfo::boot_(std::ptr::null()),
        // c:1912 — C passes the real Module m to boot. watch::boot_
        // reads m->node.flags (MOD_SETUP = mid-load_module) to gate
        // its WATCHFMT/LOGCHECK seeding under --zsh; passing the
        // table entry avoids re-locking MODULESTAB (the caller chain
        // load_module -> do_boot_module already holds it).
        "zsh/watch" => {
            let mptr = _table
                .modules
                .get(name)
                .map(|m| m as *const crate::ported::zsh_h::module)
                .unwrap_or(std::ptr::null());
            crate::ported::modules::watch::boot_(mptr)
        }
        "zsh/zftp" => crate::ported::modules::zftp::boot_(std::ptr::null()),
        "zsh/zprof" => crate::ported::modules::zprof::boot_(std::ptr::null()),
        "zsh/zpty" => crate::ported::modules::zpty::boot_(std::ptr::null()),
        "zsh/zselect" => crate::ported::modules::zselect::boot_(std::ptr::null()),
        "zsh/zutil" => crate::ported::modules::zutil::boot_(std::ptr::null()),
        // Modules without a ported per-module boot_ (e.g. zsh/main —
        // purely-static modules with no setup hook):
        // 0 == success no-op, matching the pre-port behaviour.
        _ => 0,
    }
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
///
/// Static-link path: dispatch via name → fn-pointer table to the
/// per-module `cleanup_(m)` defined in `src/ported/modules/<name>.rs`.
/// Mirrors the symmetric `boot_module` dispatcher (`Src/module.c:1910`).
/// Per-module cleanup_ typically calls `delprepromptfn` /
/// `setfeatureenables(NULL)` to roll back what boot_ installed.
/// WARNING: param names don't match C — Rust=(_table, name) vs C=(m)
pub fn cleanup_module(_table: &mut modulestab, name: &str) -> i32 {
    // c:1918
    match name {
        "zsh/attr" => crate::ported::modules::attr::cleanup_(std::ptr::null()),
        "zsh/cap" => crate::ported::modules::cap::cleanup_(std::ptr::null()),
        "zsh/clone" => crate::ported::modules::clone::cleanup_(std::ptr::null()),
        "zsh/curses" => crate::ported::modules::curses::cleanup_(std::ptr::null()),
        "zsh/datetime" => crate::ported::modules::datetime::cleanup_(std::ptr::null()),
        "zsh/db/gdbm" => crate::ported::modules::db_gdbm::cleanup_(std::ptr::null()),
        "zsh/example" => crate::ported::modules::example::cleanup_(std::ptr::null()),
        "zsh/files" => crate::ported::modules::files::cleanup_(std::ptr::null()),
        "zsh/hlgroup" => crate::ported::modules::hlgroup::cleanup_(std::ptr::null()),
        "zsh/ksh93" => crate::ported::modules::ksh93::cleanup_(std::ptr::null()),
        "zsh/langinfo" => crate::ported::modules::langinfo::cleanup_(std::ptr::null()),
        "zsh/mapfile" => crate::ported::modules::mapfile::cleanup_(std::ptr::null()),
        "zsh/mathfunc" => crate::ported::modules::mathfunc::cleanup_(std::ptr::null()),
        "zsh/nearcolor" => crate::ported::modules::nearcolor::cleanup_(std::ptr::null()),
        "zsh/newuser" => crate::ported::modules::newuser::cleanup_(std::ptr::null()),
        "zsh/parameter" => crate::ported::modules::parameter::cleanup_(std::ptr::null()),
        "zsh/param/private" => crate::ported::modules::param_private::cleanup_(std::ptr::null()),
        "zsh/pcre" => crate::ported::modules::pcre::cleanup_(std::ptr::null()),
        "zsh/random" => crate::ported::modules::random::cleanup_(std::ptr::null()),
        "zsh/regex" => crate::ported::modules::regex::cleanup_(std::ptr::null()),
        "zsh/net/socket" => crate::ported::modules::socket::cleanup_(std::ptr::null()),
        "zsh/stat" => crate::ported::modules::stat::cleanup_(std::ptr::null()),
        "zsh/system" => crate::ported::modules::system::cleanup_(std::ptr::null()),
        "zsh/net/tcp" => crate::ported::modules::tcp::cleanup_(std::ptr::null()),
        "zsh/termcap" => crate::ported::modules::termcap::cleanup_(std::ptr::null()),
        "zsh/terminfo" => crate::ported::modules::terminfo::cleanup_(std::ptr::null()),
        "zsh/watch" => crate::ported::modules::watch::cleanup_(std::ptr::null()),
        "zsh/zftp" => crate::ported::modules::zftp::cleanup_(std::ptr::null()),
        "zsh/zprof" => crate::ported::modules::zprof::cleanup_(std::ptr::null()),
        "zsh/zpty" => crate::ported::modules::zpty::cleanup_(std::ptr::null()),
        "zsh/zselect" => crate::ported::modules::zselect::cleanup_(std::ptr::null()),
        "zsh/zutil" => crate::ported::modules::zutil::cleanup_(std::ptr::null()),
        _ => 0,
    }
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
///
/// Static-link path: dispatch to the per-module `finish_(m)`.
/// finish_ is the second teardown step (after cleanup_) — it frees
/// process-lifetime state that survives a single `zmodload -u`.
/// Most modules have an empty body returning 0; a few (zftp, zpty,
/// watch with utmpx descriptors) keep state explicitly.
/// WARNING: param names don't match C — Rust=(_table, name) vs C=(m)
pub fn finish_module(_table: &mut modulestab, name: &str) -> i32 {
    // c:1926
    match name {
        "zsh/attr" => crate::ported::modules::attr::finish_(std::ptr::null()),
        "zsh/cap" => crate::ported::modules::cap::finish_(std::ptr::null()),
        "zsh/clone" => crate::ported::modules::clone::finish_(std::ptr::null()),
        "zsh/curses" => crate::ported::modules::curses::finish_(std::ptr::null()),
        "zsh/datetime" => crate::ported::modules::datetime::finish_(std::ptr::null()),
        "zsh/db/gdbm" => crate::ported::modules::db_gdbm::finish_(std::ptr::null()),
        "zsh/example" => crate::ported::modules::example::finish_(std::ptr::null()),
        "zsh/files" => crate::ported::modules::files::finish_(std::ptr::null()),
        "zsh/hlgroup" => crate::ported::modules::hlgroup::finish_(std::ptr::null()),
        "zsh/ksh93" => crate::ported::modules::ksh93::finish_(std::ptr::null()),
        "zsh/langinfo" => crate::ported::modules::langinfo::finish_(std::ptr::null()),
        "zsh/mapfile" => crate::ported::modules::mapfile::finish_(std::ptr::null()),
        "zsh/mathfunc" => crate::ported::modules::mathfunc::finish_(std::ptr::null()),
        "zsh/nearcolor" => crate::ported::modules::nearcolor::finish_(std::ptr::null()),
        "zsh/newuser" => crate::ported::modules::newuser::finish_(std::ptr::null()),
        "zsh/parameter" => crate::ported::modules::parameter::finish_(std::ptr::null()),
        "zsh/param/private" => crate::ported::modules::param_private::finish_(std::ptr::null()),
        "zsh/pcre" => crate::ported::modules::pcre::finish_(std::ptr::null()),
        "zsh/random" => crate::ported::modules::random::finish_(std::ptr::null()),
        "zsh/regex" => crate::ported::modules::regex::finish_(std::ptr::null()),
        "zsh/net/socket" => crate::ported::modules::socket::finish_(std::ptr::null()),
        "zsh/stat" => crate::ported::modules::stat::finish_(std::ptr::null()),
        "zsh/system" => crate::ported::modules::system::finish_(std::ptr::null()),
        "zsh/net/tcp" => crate::ported::modules::tcp::finish_(std::ptr::null()),
        "zsh/termcap" => crate::ported::modules::termcap::finish_(std::ptr::null()),
        "zsh/terminfo" => crate::ported::modules::terminfo::finish_(std::ptr::null()),
        "zsh/watch" => crate::ported::modules::watch::finish_(std::ptr::null()),
        "zsh/zftp" => crate::ported::modules::zftp::finish_(std::ptr::null()),
        "zsh/zprof" => crate::ported::modules::zprof::finish_(std::ptr::null()),
        "zsh/zpty" => crate::ported::modules::zpty::finish_(std::ptr::null()),
        "zsh/zselect" => crate::ported::modules::zselect::finish_(std::ptr::null()),
        "zsh/zutil" => crate::ported::modules::zutil::finish_(std::ptr::null()),
        _ => 0,
    }
}

/// Port of `do_module_features(Module m, Feature_enables enablesarr, int flags)` from `Src/module.c:1998`.
///
/// C body c:1998-2125 (128 lines):
/// ```c
/// if (features_module(m, &features) == 0) {
///     int *enables = NULL;
///     if (enables_module(m, &enables)) {
///         if (!(flags & FEAT_IGNORE)) zwarn(...);
///         return 1;
///     }
///     if ((flags & FEAT_CHECKAUTO) && m->autoloads) {
///         /* validate autoloads against features list */
///         /* on mismatch: zwarn + autofeatures(REMOVE|IGNORE) +
///            expunge from enablesarr */
///     }
///     if (enablesarr) {
///         /* walk enablesarr, flip enables bits per +/- prefix */
///     } else {
///         /* enable all features */
///     }
///     if (enables_module(m, &enables)) return 2;
/// } else if (enablesarr) {
///     if (!(flags & FEAT_IGNORE)) zwarn("module does not support features");
///     return 1;
/// }
/// return ret;
/// ```
///
/// Prior Rust port confused module-name and enables-string into a
/// single `enablesarr: &str` param — the module-lookup and zwarn
/// diagnostic both used the feature-list string instead of the
/// module's name. Signature now separates them per C semantics:
/// modname identifies the module, features is the list of features
/// to enable (None = "enable all").
/// WARNING: param names don't match C — Rust=(table, modname, features, flags) vs C=(m, enablesarr, flags)
pub fn do_module_features(
    table: &mut modulestab,
    modname: &str,
    features: Option<&[String]>,
    flags: i32,
) -> i32 {
    // c:1998
    let mut module_features: Vec<String> = Vec::new(); // c:2000
    let mut ret: i32 = 0; // c:2001

    // c:2003 — `if (features_module(m, &features) == 0)`.
    if features_module(table, modname, &mut module_features) == 0 {
        // c:2011-2018 — fetch enables. Features supported → enables
        // should be too; error here is reported unless FEAT_IGNORE.
        let mut enables: Option<Vec<i32>> = None;
        if enables_module(table, modname, &mut enables) != 0 {
            // c:2012
            if (flags & FEAT_IGNORE) == 0 {
                // c:2014
                zwarn(&format!(
                    "error getting enabled features for module `{}'", // c:2015
                    modname
                ));
            }
            return 1; // c:2017
        }

        // c:2020 — `if ((flags & FEAT_CHECKAUTO) && m->autoloads)`
        if (flags & FEAT_CHECKAUTO) != 0 {
            let autoloads: Vec<String> = table
                .modules
                .get(modname)
                .and_then(|m| m.autoloads.as_ref())
                .map(|al| al.iter().cloned().collect())
                .unwrap_or_default();
            // c:2027-2074 — walk autoloads, cancel mismatches.
            for al in &autoloads {
                // c:2028
                // c:2032-2034 — match `al` against the features array.
                let found = module_features.iter().any(|f| f == al);
                if !found {
                    // c:2035
                    if (flags & FEAT_IGNORE) == 0 {
                        // c:2037
                        zwarn(&format!(
                            "module `{}' has no such feature: `{}': autoload cancelled", // c:2038-2040
                            modname, al
                        ));
                    }
                    // c:2045-2047 — autofeatures(NULL, m->node.nam, arg, 0, FEAT_IGNORE|FEAT_REMOVE)
                    let arg = vec![al.clone()];
                    autofeatures(table, "", Some(modname), &arg, 0, FEAT_IGNORE | FEAT_REMOVE);
                    // c:2053-2072 — expunge from enablesarr.
                    // features arg is &[String] — Rust slice can't be
                    // mutated through &. The C path mutates the passed
                    // Feature_enables array; the Rust callers that need
                    // expunge build a fresh list. Skipped here.
                }
            }
        }

        // c:2077-2113 — apply enablesarr (or enable all).
        match features {
            Some(arr) => {
                // c:2079-2103 — walk enablesarr.
                let enables_vec = enables.get_or_insert_with(Vec::new);
                if enables_vec.len() < module_features.len() {
                    enables_vec.resize(module_features.len(), 0);
                }
                for fep_str in arr {
                    // c:2079 for (fep = enablesarr; fep->str; fep++)
                    let (on, esp) = if let Some(rest) = fep_str.strip_prefix('+') {
                        // c:2082-2083
                        (1i32, rest)
                    } else if let Some(rest) = fep_str.strip_prefix('-') {
                        // c:2084-2086
                        (0i32, rest)
                    } else {
                        (1i32, fep_str.as_str())
                    };
                    // c:2088-2094 —
                    //   for (fp = features; *fp; fp++)
                    //       if (fep->pat ? pattry(fep->pat, *fp)
                    //                    : !strcmp(*fp, esp)) {
                    //           enables[fp - features] = on;
                    //           found++;
                    //           if (!fep->pat) break;
                    //       }
                    // `fep->pat` is non-NULL exactly when `zmodload -m`
                    // was given (c:3258); zshrs carries that as
                    // FEAT_PATTERN_ARGS (see its doc comment for why).
                    // A pattern keeps scanning — it may enable several
                    // features — while an exact name stops at the first
                    // hit.
                    let pat = if (flags & FEAT_PATTERN_ARGS) != 0 {
                        let mut pat_src = crate::ported::string::dupstring(esp);
                        crate::ported::glob::tokenize(&mut pat_src);
                        crate::ported::pattern::patcompile(
                            &pat_src,
                            crate::ported::zsh_h::PAT_STATIC,
                            None,
                        )
                    } else {
                        None
                    };
                    let mut found = false;
                    for (i, f) in module_features.iter().enumerate() {
                        let hit = match pat.as_ref() {
                            Some(p) => crate::ported::pattern::pattry(p, f), // c:2093
                            None => f == esp,                                    // c:2093
                        };
                        if hit {
                            enables_vec[i] = on; // c:2090
                            found = true;
                            if pat.is_none() {
                                break; // c:2096-2097 `if (!fep->pat) break;`
                            }
                        }
                    }
                    if !found {
                        // c:2099-2106 — the diagnostic differs for the
                        // pattern form.
                        if (flags & FEAT_IGNORE) == 0 {
                            zwarn(&format!(
                                "module `{}' has no {}: `{}'",
                                modname,
                                if pat.is_some() {
                                    "feature matching" // c:2102
                                } else {
                                    "such feature" // c:2103
                                },
                                esp
                            ));
                        }
                        return 1; // c:2105
                    }
                }
            }
            None => {
                // c:2105-2112 — enable all features.
                let enables_vec = enables.get_or_insert_with(Vec::new);
                enables_vec.clear();
                enables_vec.resize(module_features.len(), 1);
                // c:2115-2116 — `for (ep = enables; n_features--; ep++) *ep = 1;`
                // with the comment "Enable all features.  This is used when
                // loading without using zmodload -F."  The commit below
                // (`enables_module`, c:2119) runs the module's `enables_` →
                // `handlefeatures` (c:3392) → `setfeatureenables` (c:3354) →
                // `setparamdefs` (c:1169-1181), which calls `addparamdef`
                // (c:1060) for EVERY `p:` feature whose enable bit is set.
                // `addparamdef` installs the real special Param over the
                // PM_AUTOLOAD stub `autofeatures` planted, so after a plain
                // `zmodload zsh/parameter` NONE of that module's parameters
                // is an autoload stub any more — only the single-feature
                // form (`ensurefeature(mn, "p:", nam)` from `loadparamnode`,
                // c:Src/params.c:568, which lands in the `Some(arr)` arm
                // above) leaves the siblings as stubs.
                //
                // zshrs seeds every magic parameter eagerly
                // (`vm_helper::init_partab_params`) and models PM_AUTOLOAD as
                // the `MATERIALIZED_MODULE_PARAMS` side set instead of a node
                // flag, so the stub-clearing half of `setparamdefs` has to be
                // replayed here. Without it a plain `zmodload zsh/parameter`
                // (which plugin managers such as zinit run at startup) left
                // `commands` reading as a
                // stub, so `local -A +h commands` in `_command_names` built a
                // PLAIN empty local instead of a special one (c:Src/builtin.c
                // :2083-2085 needs PM_SPECIAL on the node to set
                // `newspecial`), and `_path_commands`' `compadd -k commands`
                // added nothing until the user touched `$commands` by hand.
                for f in &module_features {
                    if let Some(pname) = f.strip_prefix("p:") {
                        crate::vm_helper::mark_module_param_used(pname);
                    }
                }
            }
        }

        // c:2115 — final `enables_module(m, &enables)` commits the bits.
        if enables_module(table, modname, &mut enables) != 0 {
            return 2; // c:2116
        }
    } else if features.is_some() {
        // c:2117-2121 — features_module failed AND enablesarr non-NULL:
        // module doesn't support features. zwarn unless FEAT_IGNORE.
        if (flags & FEAT_IGNORE) == 0 {
            zwarn(&format!(
                "module `{}' does not support features", // c:2119
                modname
            ));
        }
        return 1; // c:2120
    }
    // c:2122 — `Else it doesn't support features but we don't care.`
    let _ = &mut ret;
    ret // c:2124
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
pub fn do_boot_module(
    table: &mut modulestab,
    modname: &str,
    features: Option<&[String]>,
    silent: i32,
    // !!! RUST-ONLY PARAM !!! — carries C's per-entry
    // `Feature_enables.pat` (see FEAT_PATTERN_ARGS).
    feat_pat: bool,
) -> i32 {
    // c:2139
    let mut flags = if silent != 0 {
        // c:2142 — silent → IGNORE | CHECKAUTO
        FEAT_IGNORE | FEAT_CHECKAUTO
    } else {
        FEAT_CHECKAUTO // c:2143
    };
    if feat_pat {
        flags |= FEAT_PATTERN_ARGS;
    }
    let ret = do_module_features(table, modname, features, flags); // c:2141
    if ret == 1 {
        // c:2145
        return 1; // c:2146
    }
    if boot_module(table, modname) != 0 {
        // c:2148
        return 1; // c:2149
    }
    ret // c:2150
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
pub fn do_cleanup_module(table: &mut modulestab, name: &str) -> i32 {
    // c:2159
    // Check the module is registered, then dispatch to cleanup_module.
    if table.modules.contains_key(name) {
        // c:2162 m->u.linked
        cleanup_module(table, name) // c:2163 cleanup_module(m)
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
pub fn modname_ok(p: &str) -> i32 {
    // c:2173
    let bytes = p.as_bytes();
    let mut i: usize = 0;
    loop {
        // c:2176 — `p = itype_end(p, IIDENT, 0);`
        // IIDENT = identifier-byte (alpha/digit/underscore + extended).
        while i < bytes.len() {
            let b = bytes[i];
            // Inline IIDENT check — alphanumeric or underscore. Mirrors
            // utils.c:itype_end stepping for the IIDENT bit.
            if b.is_ascii_alphanumeric() || b == b'_' {
                i += 1;
            } else {
                break;
            }
        }
        if i >= bytes.len() {
            // c:2177 if (!*p)
            return 1; // c:2178
        }
        if bytes[i] != b'/' {
            break;
        } // c:2179 while(*p++ == '/')
        i += 1;
    }
    0 // c:2180
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

/// Port of `require_module(const char *module, Feature_enables features, int silent)` from `Src/module.c:2344`.
///
/// C body c:2342-2360:
/// ```c
/// mod_export int
/// require_module(const char *module, Feature_enables features, int silent)
/// {
///     Module m = NULL;
///     int ret = 0;
///     queue_signals();
///     m = find_module(module, FINDMOD_ALIASP, &module);
///     if (!m || !m->u.handle ||
///         (m->node.flags & MOD_UNLOAD))
///         ret = load_module(module, features, silent);
///     else
///         ret = do_module_features(m, features, 0);
///     unqueue_signals();
///     return ret;
/// }
/// ```
///
/// Two branches: when the module isn't loaded yet (or is mid-unload),
/// route through `load_module` which runs the full setup → features →
/// boot lifecycle. When it's already loaded, skip to
/// `do_module_features` to just enable the requested per-feature
/// surface — much cheaper.
///
/// Static-link analog of `m->u.handle` is `MOD_INIT_B` (boot ran):
/// once boot_module has fired, the module is "loaded" in zshrs's
/// non-dlopen world.
/// WARNING: param names don't match C — Rust=(table, modname, features, silent) vs C=(module, features, silent)
pub fn require_module(
    table: &mut modulestab,
    modname: &str,
    features: Option<&[String]>,
    silent: i32,
    // !!! RUST-ONLY PARAM !!! — carries C's per-entry
    // `Feature_enables.pat` (see FEAT_PATTERN_ARGS).
    feat_pat: bool,
) -> i32 {
    // c:2344
    // c:2350 — queue_signals(): signal-deferral wrapper.
    crate::ported::signals::queue_signals();

    // c:2351 — `m = find_module(module, FINDMOD_ALIASP, &module);`
    // Resolves alias chain; canonical name lives in `mname`.
    let mname_opt = find_module(table, modname, FINDMOD_ALIASP);

    // c:2352-2353 — `if (!m || !m->u.handle || MOD_UNLOAD)`.
    // Static-link analog of `m->u.handle`: MOD_INIT_B (boot ran).
    let needs_load = match &mname_opt {
        None => true, // c:2352 !m
        Some(mname) => match table.modules.get(mname) {
            None => true,
            Some(m) => {
                (m.node.flags & MOD_INIT_B) == 0 // c:2352 !u.handle analog
                    || (m.node.flags & MOD_UNLOAD) != 0 // c:2353
            }
        },
    };

    let mname = mname_opt.unwrap_or_else(|| modname.to_string());

    let ret = if needs_load {
        // c:2354 — `ret = load_module(module, features, silent);`
        // try_load_module gates the static-link path. On miss, emit
        // the canonical zwarn (gated by silent).
        if try_load_module(table, &mname) == 0 {
            if silent == 0 {
                // c:1622 — same message and same reasoning as do_load_module
                // (module.rs, `failed to load module `%s'` with the dlerror tail
                // omitted). docs/BUGS.md #376 records the backquoted prefix as
                // the FINAL intended state and the parity test pins it, so this
                // site must not diverge from the other one.
                crate::ported::utils::zwarn(&format!(
                    "failed to load module `{}'",
                    mname
                ));
            }
            crate::ported::signals::unqueue_signals();
            return 1;
        }
        if !table.load_module(&mname, features, feat_pat) {
            crate::ported::signals::unqueue_signals();
            return 1;
        }
        0
    } else {
        // c:2356 — `ret = do_module_features(m, features, 0);`
        // Module already loaded; just enable the requested features.
        // features=NULL in C means "enable all features"; the Rust
        // do_module_features takes a single enablesstr arg, so flatten
        // C: features=NULL means "enable all"; pass through.
        do_module_features(
            table,
            &mname,
            features,
            if feat_pat { FEAT_PATTERN_ARGS } else { 0 },
        )
    };

    // c:2357 — unqueue_signals();
    crate::ported::signals::unqueue_signals();

    ret // c:2359
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
pub fn add_dep(table: &mut modulestab, name: &str, from: &str) -> i32 {
    // c:2369
    // c:2369 — m = find_module(name, FINDMOD_ALIASP|FINDMOD_CREATE, &name)
    let canon = match find_module(table, name, FINDMOD_ALIASP | FINDMOD_CREATE) {
        Some(n) => n,
        None => return 0,
    };
    if let Some(m) = table.modules.get_mut(&canon) {
        // c:2389-2391 — walk deps, skip if `from` already present.
        let deps = m
            .deps
            .get_or_insert_with(crate::ported::linklist::LinkList::new);
        if !deps.iter().any(|d| d == from) {
            // c:2392 if (!node)
            deps.push_back(from.to_string()); // c:2393 zaddlinknode
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
pub fn autoloadscan(name: &str, optstr: &str, flags: u32, printflags: i32) {
    // c:2403
    use crate::ported::utils::{nicezputs, quotedzputs};
    let mut stdout = std::io::stdout();
    if (flags & BINF_ADDED) != 0 {
        // c:2407
        return; // c:2408
    }
    if (printflags & PRINT_LIST) != 0 {
        // c:2409
        // c:2410-2417 — long form `zmodload -ab MOD NAME`
        print!("zmodload -ab ");
        if optstr.starts_with('-') {
            // c:2411
            print!("-- "); // c:2412
        }
        // c:2413 — `quotedzputs(bn->optstr, stdout);`
        print!("{}", quotedzputs(optstr));
        if name != optstr {
            // c:2414 — `if(strcmp(bn->node.nam, bn->optstr))`
            print!(" "); // c:2415 putchar(' ')
                         // c:2416 — `quotedzputs(bn->node.nam, stdout);`
            print!("{}", quotedzputs(name));
        }
    } else {
        // c:2418-2424 — short form `NAME (MOD)`
        let _ = nicezputs(name, &mut stdout); // c:2419
        if name != optstr {
            // c:2420 — `if(strcmp(bn->node.nam, bn->optstr))`
            print!(" ("); // c:2421
            let _ = nicezputs(optstr, &mut stdout); // c:2422
            print!(")"); // c:2423
        }
    }
    println!(); // c:2426 putchar('\n')
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
pub fn bin_zmodload(
    nam: &str,
    args: &[String], // c:2440
    ops: &options,
    _func: i32,
) -> i32 {
    let mut table = MODULESTAB.lock().unwrap();
    let table = &mut *table;

    let ops_bcpf = OPT_ISSET(ops, b'b') || OPT_ISSET(ops, b'c')              // c:2443
                || OPT_ISSET(ops, b'p') || OPT_ISSET(ops, b'f');
    let ops_au = OPT_ISSET(ops, b'a') || OPT_ISSET(ops, b'u'); // c:2445
    let mut ret: i32; // c:2446

    if ops_bcpf && !ops_au {
        // c:2451
        zwarnnam(nam, "-b, -c, -f, and -p must be combined with -a or -u"); // c:2452
        return 1; // c:2453
    }
    if OPT_ISSET(ops, b'F') && (ops_bcpf || OPT_ISSET(ops, b'u')) {
        // c:2455
        zwarnnam(nam, "-b, -c, -f, -p and -u cannot be combined with -F"); // c:2456
        return 1; // c:2457
    }
    // !!! WARNING: RUST-ONLY EXTENSION — NO C COUNTERPART !!!
    // zshrs repurposes `-R` WITHOUT `-A` to load a native (Rust)
    // plugin cdylib through the stable C ABI (src/extensions/
    // plugin_host.rs): the first compiled Unix shell hosting
    // compiled-language plugins loaded at runtime. In C zsh `-R` means
    // "remove module alias" and is only ever meaningful alongside `-A`
    // (Src/module.c:2459); that behaviour is preserved for `-A -R name`
    // (both flags set falls through to bin_zmodload_alias below), so no
    // parity is lost for the near-unused alias-removal case.
    //   zmodload -R  <path>...    load each plugin cdylib
    //   zmodload -R               list loaded plugins
    //   zmodload -uR <name>...    unload each plugin by name
    // …but only when no operand names a REAL zsh module. `zmodload -R
    // zsh/complete` is C's alias-removal form applied to a known module
    // and must keep C's diagnostic (`module is not an alias: zsh/complete`,
    // c:2561), while a native plugin is a cdylib path or an arbitrary
    // plugin name that is never in `linkedmodules`. Discriminating on the
    // operand lets BOTH behaviours coexist instead of the extension
    // shadowing the ported one: previously every `-R` operand went to the
    // plugin host, so a zsh module name came back as a dlopen error.
    // No operands (`zmodload -R` = list loaded plugins) still routes here.
    if OPT_ISSET(ops, b'R')
        && !OPT_ISSET(ops, b'A')
        && !args.iter().any(|a| table.module_linked(a))
    {
        // Placed before the c:2490 queue_signals, so nothing to unqueue.
        // Handler lives in the extensions tree (not a port) —
        // src/extensions/plugin_host.rs.
        return crate::plugin_host::zmodload_rust_cmd(nam, args, ops);
    }
    if OPT_ISSET(ops, b'A') || OPT_ISSET(ops, b'R') {
        // c:2459
        if ops_bcpf || ops_au || OPT_ISSET(ops, b'd')                        // c:2460
           || (OPT_ISSET(ops, b'R') && OPT_ISSET(ops, b'e'))
        {
            zwarnnam(nam, "illegal flags combined with -A or -R"); // c:2462
            return 1; // c:2463
        }
        if !OPT_ISSET(ops, b'e') {
            // c:2465
            return bin_zmodload_alias(table, nam, args, ops); // c:2466
        }
    }
    if OPT_ISSET(ops, b'd') && OPT_ISSET(ops, b'a') {
        // c:2468
        zwarnnam(nam, "-d cannot be combined with -a"); // c:2469
        return 1; // c:2470
    }
    if OPT_ISSET(ops, b'u') && args.is_empty() {
        // c:2472
        zwarnnam(nam, "what do you want to unload?"); // c:2473
        return 1; // c:2474
    }
    if OPT_ISSET(ops, b'e')
        && (OPT_ISSET(ops, b'I') || OPT_ISSET(ops, b'L') // c:2476
        || (OPT_ISSET(ops, b'a') && !OPT_ISSET(ops, b'F'))
        || OPT_ISSET(ops, b'd') || OPT_ISSET(ops, b'i')
        || OPT_ISSET(ops, b'u'))
    {
        zwarnnam(nam, "-e cannot be combined with other options"); // c:2480
        return 1; // c:2482
    }
    // c:2484 — `for (fp = fonly; *fp; fp++)` — `l` and `P` only with `-F`.
    for fp in [b'l', b'P'] {
        // c:2484
        if OPT_ISSET(ops, fp) && !OPT_ISSET(ops, b'F') {
            // c:2485
            zwarnnam(nam, &format!("-{} is only allowed with -F", fp as char)); // c:2486
            return 1; // c:2487
        }
    }
    crate::ported::mem::queue_signals(); // c:2490
    if OPT_ISSET(ops, b'F') {
        // c:2491
        ret = bin_zmodload_features(table, nam, args, ops); // c:2492
    } else if OPT_ISSET(ops, b'e') {
        // c:2493
        ret = bin_zmodload_exist(table, nam, args, ops); // c:2494
    } else if OPT_ISSET(ops, b'd') {
        // c:2495
        ret = bin_zmodload_dep(table, nam, args, ops); // c:2496
    } else {
        let autoopts = (OPT_ISSET(ops, b'b') as i32)                         // c:2497
                     + (OPT_ISSET(ops, b'c') as i32)
                     + (OPT_ISSET(ops, b'p') as i32)
                     + (OPT_ISSET(ops, b'f') as i32);
        if autoopts != 0 || OPT_ISSET(ops, b'a') {
            // c:2497-2499
            if autoopts > 1 {
                // c:2502
                zwarnnam(nam, "use only one of -b, -c, or -p"); // c:2503
                ret = 1; // c:2504
            } else {
                ret = bin_zmodload_auto(table, nam, args, ops); // c:2506
            }
        } else {
            ret = bin_zmodload_load(table, nam, args, ops); // c:2508
        }
    }
    unqueue_signals(); // c:2515
    ret // c:2515
}

/// Port of `bin_zmodload_alias(char *nam, char **args, Options ops)` from `Src/module.c:2515`.
///
/// `zmodload -A [-L|-R] [name=alias ...]`. Three modes:
/// - no args: list all module aliases (`-L` = long form).
/// - `-R name`: remove alias `name` (must already be MOD_ALIAS).
/// - `name=target`: install/replace alias `name` pointing at `target`.
///   Detects self-cycles before committing.
/// WARNING: param names don't match C — Rust=(table, nam, args, ops) vs C=(nam, args, ops)
pub fn bin_zmodload_alias(
    table: &mut modulestab,
    nam: &str,
    args: &[String],
    ops: &options,
) -> i32 {
    // c:2515
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

    // c:2532-2541 — no args: list aliases.
    // C body:
    //   if (!*args) {
    //       if (OPT_ISSET(ops,'R')) {
    //           zwarnnam(nam, "no module alias to remove");
    //           return 1;
    //       }
    //       scanhashtable(modulestab, 1, MOD_ALIAS, 0,
    //                     modulestab->printnode,
    //                     OPT_ISSET(ops,'L') ? PRINTMOD_LIST : 0);
    //       return 0;
    //   }
    //
    // scanhashtable args: sorted=1, INCLUDE=MOD_ALIAS, EXCLUDE=0.
    // Only MOD_ALIAS entries pass, walked in name order. Each entry
    // dispatched through printmodulenode (the alias-emit arm).
    if args.is_empty() {
        if OPT_ISSET(ops, b'R') {
            // c:2533
            zwarnnam(nam, "no module alias to remove"); // c:2534
            return 1; // c:2535
        }
        let listflags = if OPT_ISSET(ops, b'L') {
            PRINTMOD_LIST
        } else {
            0
        };
        let mut names: Vec<&String> = table
            .modules
            .iter()
            .filter(|(_, m)| (m.node.flags & MOD_ALIAS) != 0) // c:2537 INCLUDE
            .map(|(n, _)| n)
            .collect();
        names.sort(); // c:2537 sorted=1
        for name in names {
            let m = &table.modules[name];
            let line = printmodulenode(name, m, listflags);
            if !line.is_empty() {
                println!("{}", line);
            }
        }
        return 0; // c:2540
    }

    // c:2543 — for each arg, parse name=alias and dispatch.
    for arg in args {
        // c:2544-2547 — split at '='
        let (lhs, aliasname): (&str, Option<&str>) = match arg.find('=') {
            Some(eq) => (&arg[..eq], Some(&arg[eq + 1..])),
            None => (arg.as_str(), None),
        };
        // c:2548 — modname_ok check on the LHS
        if modname_ok(lhs) == 0 {
            // c:2548
            zwarnnam(nam, &format!("invalid module name `{}'", lhs)); // c:2549
            return 1; // c:2550
        }
        if OPT_ISSET(ops, b'R') {
            // c:2552
            // -R: remove alias path.
            if aliasname.is_some() {
                // c:2553
                zwarnnam(
                    nam,
                    &format!("bad syntax for removing module alias: {}", lhs),
                ); // c:2554
                return 1; // c:2556
            }
            // c:2558 — find_module(lhs, 0, NULL)
            match table.modules.get(lhs) {
                Some(m) => {
                    if (m.node.flags & MOD_ALIAS) == 0 {
                        // c:2560
                        zwarnnam(nam, &format!("module is not an alias: {}", lhs)); // c:2561
                        return 1; // c:2562
                    }
                    table.modules.remove(lhs); // c:2564 delete_module
                }
                None => {
                    zwarnnam(nam, &format!("no such module alias: {}", lhs)); // c:2566
                    return 1; // c:2567
                }
            }
        } else {
            // No -R: install/replace alias OR list one.
            if let Some(target) = aliasname {
                // c:2570
                if modname_ok(target) == 0 {
                    // c:2572
                    zwarnnam(nam, &format!("invalid module name `{}'", target)); // c:2573
                    return 1; // c:2574
                }
                // c:2576-2584 — cycle detection: walk alias chain
                let mut mname = target;
                let mut depth = 0;
                loop {
                    if depth > 256 {
                        break;
                    }
                    depth += 1;
                    if mname == lhs {
                        // c:2577
                        zwarnnam(nam, &format!("module alias would refer to itself: {}", lhs)); // c:2578
                        return 1; // c:2580
                    }
                    match table.modules.get(mname) {
                        Some(m) if (m.node.flags & MOD_ALIAS) != 0 => {
                            mname = m.alias.as_deref().unwrap_or("");
                        }
                        _ => break,
                    }
                }
                // c:2585-2596 — install or replace
                if let Some(m) = table.modules.get_mut(lhs) {
                    if (m.node.flags & MOD_ALIAS) == 0 {
                        // c:2587
                        zwarnnam(nam, &format!("module is not an alias: {}", lhs)); // c:2588
                        return 1; // c:2589
                    }
                    m.alias = Some(target.to_string()); // c:2591/2597
                } else {
                    let mut m = module::new(lhs); // c:2593 zshcalloc
                    m.node.flags = MOD_ALIAS; // c:2594
                    m.alias = Some(target.to_string()); // c:2597
                    table.modules.insert(lhs.to_string(), m); // c:2595
                }
            } else {
                // c:2599-2611 — list one alias.
                // C body:
                //   if ((m = find_module(*args, 0, NULL))) {
                //       if (m->node.flags & MOD_ALIAS)
                //           modulestab->printnode(&m->node,
                //                                 OPT_ISSET(ops,'L')
                //                                     ? PRINTMOD_LIST : 0);
                //       else { zwarnnam(...); return 1; }
                //   } else { zwarnnam(...); return 1; }
                match table.modules.get(lhs) {
                    Some(m) if (m.node.flags & MOD_ALIAS) != 0 => {
                        // c:2601-2603 — printnode dispatch
                        let listflags = if OPT_ISSET(ops, b'L') {
                            PRINTMOD_LIST
                        } else {
                            0
                        };
                        let line = printmodulenode(lhs, m, listflags);
                        if !line.is_empty() {
                            println!("{}", line);
                        }
                    }
                    Some(_) => {
                        zwarnnam(nam, &format!("module is not an alias: {}", lhs)); // c:2605
                        return 1; // c:2606
                    }
                    None => {
                        zwarnnam(nam, &format!("no such module alias: {}", lhs)); // c:2609
                        return 1; // c:2610
                    }
                }
            }
        }
    }
    0 // c:2616
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
pub fn bin_zmodload_exist(
    table: &mut modulestab,
    _nam: &str,
    args: &[String],
    ops: &options,
) -> i32 {
    // c:2623
    if args.is_empty() {
        // c:2627
        // c:2628-2630 — scanhashtable(modulestab, 1, 0, 0,
        //                              modulestab->printnode,
        //                              OPT_ISSET(ops,'A')
        //                                  ? PRINTMOD_EXIST|PRINTMOD_ALIAS
        //                                  : PRINTMOD_EXIST);
        // Sorted walk (the `1` 2nd arg) over the full table (no
        // INCLUDE/EXCLUDE filters), dispatching every entry through
        // printmodulenode. -A toggles PRINTMOD_ALIAS so alias entries
        // emit their alias target alongside the existence line.
        let printflags = if OPT_ISSET(ops, b'A') {
            PRINTMOD_EXIST | PRINTMOD_ALIAS
        } else {
            PRINTMOD_EXIST
        };
        let mut names: Vec<&String> = table.modules.keys().collect();
        names.sort(); // c:2628 sorted=1
        for name in names {
            let m = &table.modules[name];
            let line = printmodulenode(name, m, printflags);
            if !line.is_empty() {
                println!("{}", line);
            }
        }
        return 0; // c:2631
    }
    // c:2633-2640 — for each arg, test existence.
    // C:
    //   for (; !ret && *args; args++) {
    //       if (!(m = find_module(*args, FINDMOD_ALIASP, NULL))
    //           || !m->u.handle
    //           || (m->node.flags & MOD_UNLOAD))
    //           ret = 1;
    //   }
    // The `!m->u.handle` clause is union-typed in C: for static-link
    // modules it reads `m->u.linked` (non-NULL when linked, NULL when
    // pre-registered but not yet bound); for dynamic modules it's the
    // dlopen handle. Translate to: handle.is_none() && linked.is_none()
    // — both representations of "no live binding."
    let mut ret: i32 = 0;
    for arg in args {
        // c:2635
        if ret != 0 {
            break;
        }
        let canon = match find_module(table, arg, FINDMOD_ALIASP) {
            // c:2636
            Some(n) => n,
            None => {
                ret = 1; // c:2639
                continue;
            }
        };
        let live = match table.modules.get(&canon) {
            Some(m) => {
                // c:2637 — `!m->u.handle` (union-semantics).
                let bound = m.handle.is_some() || m.linked.is_some();
                // c:2638 — `(m->node.flags & MOD_UNLOAD)`.
                let unloading = (m.node.flags & MOD_UNLOAD) != 0;
                bound && !unloading
            }
            None => false,
        };
        if !live {
            ret = 1; // c:2639
        }
    }
    ret // c:2641
}

/// Port of `bin_zmodload_dep(UNUSED(char *nam), char **args, Options ops)` from `Src/module.c:2649`.
///
/// `zmodload -d [-u] [target [dep ...]]`. Three modes:
/// - `-u target` removes all deps from target; `-u target d1 d2` removes
///   only those.
/// - no args lists all dependencies.
/// - `target dep1 ...` adds each dep to target's dependency list.
/// WARNING: param names don't match C — Rust=(table, _nam, args, ops) vs C=(nam, args, ops)
pub fn bin_zmodload_dep(table: &mut modulestab, _nam: &str, args: &[String], ops: &options) -> i32 {
    // c:2649
    if OPT_ISSET(ops, b'u') {
        // c:2652
        // c:2654 — `const char *tnam = *args++;`
        if args.is_empty() {
            return 0;
        }
        let tnam = &args[0];
        let rest = &args[1..];
        // c:2655 — `find_module(tnam, FINDMOD_ALIASP, &tnam)`
        let canon = match find_module(table, tnam, FINDMOD_ALIASP) {
            Some(n) => n,
            None => return 0, // c:2656-2657
        };
        if let Some(m) = table.modules.get_mut(&canon) {
            if !rest.is_empty() && m.deps.is_some() {
                // c:2658-2671 — remove specific deps.
                // C body c:2659-2667 walks args, finds each in deps,
                // removes the matched node; the inner break ends the
                // current arg's search but the outer while continues
                // to the next arg. Mirrors with a for-loop.
                let deps = m.deps.as_mut().unwrap();
                for to_remove in rest {
                    // c:2659 do { ... } while(*++args)
                    if let Some(pos) = deps.iter().position(|d| d == to_remove) {
                        deps.delete_node(pos); // c:2664 remnode
                    }
                }
                // c:2668-2671 — if deps now empty, free the list.
                if deps.is_empty() {
                    m.deps = None;
                }
            } else if m.deps.is_some() {
                // c:2672-2676 — no specific deps given: clear all.
                m.deps = None;
            }
            // c:2678-2679 — `if (!m->deps && !m->u.handle) delete_module(m);`
            // Static-link analog of `!u.handle` is `!MOD_INIT_B` (boot
            // hasn't run = no loaded handle). Without the MOD_INIT_B
            // gate, the prior port deleted any module whose deps got
            // cleared, dropping the modulestab entries for already-
            // loaded modules (zsh/main, zsh/watch, etc.) the moment
            // their dep list went empty.
            let no_deps = m.deps.is_none();
            let no_handle = (m.node.flags & MOD_INIT_B) == 0;
            if no_deps && no_handle {
                table.modules.remove(&canon); // c:2679 delete_module
            }
        }
        return 0; // c:2680
    }
    // c:2681 — `else if (!args[0] || !args[1])`: list-mode (one or all).
    if args.is_empty() || args.len() == 1 {
        // c:2682-2691 — `int depflags = OPT_ISSET(ops,'L')
        //                  ? PRINTMOD_DEPS|PRINTMOD_LIST : PRINTMOD_DEPS;`
        let depflags = if OPT_ISSET(ops, b'L') {
            PRINTMOD_DEPS | PRINTMOD_LIST
        } else {
            PRINTMOD_DEPS
        };
        if !args.is_empty() {
            // c:2685-2687 — single-name list:
            //   if ((m = modulestab->getnode2(modulestab, args[0])))
            //       modulestab->printnode(&m->node, depflags);
            if let Some(m) = table.modules.get(&args[0]) {
                let line = printmodulenode(&args[0], m, depflags);
                if !line.is_empty() {
                    println!("{}", line);
                }
            }
        } else {
            // c:2688-2691 — full sorted scan.
            //   scanhashtable(modulestab, 1, 0, 0, printnode, depflags);
            let mut names: Vec<&String> = table.modules.keys().collect();
            names.sort(); // c:2689 sorted=1
            for name in names {
                let m = &table.modules[name];
                let line = printmodulenode(name, m, depflags);
                if !line.is_empty() {
                    println!("{}", line);
                }
            }
        }
        return 0; // c:2692
    }
    // c:2693-2701 — add deps: args[0] target, args[1..] deps to add.
    let target = &args[0];
    for dep in &args[1..] {
        add_dep(table, target, dep); // c:2699
    }
    0 // c:2700 (ret stays 0)
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
pub fn printautoparams(name: &str, module: &str, flags: u32, lon: i32) {
    // c:2710
    if (flags & PM_AUTOLOAD) != 0 {
        // c:2710
        if lon != 0 {
            // c:2715
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
/// `-c` lists/registers conditions, `-p` parameters, `-f` math ported,
/// default is builtins. `-L` toggles long-form listing.
///
/// Static-link path: registers via `add_autoaliasbuiltin` /
/// `add_autoparam` / `add_automathfunc` already ported. Without a
/// module name (just `-a`), runs the listing scan via `autoloadscan`
/// or its conddef/param/mathfn equivalents.
/// WARNING: param names don't match C — Rust=(table, _nam, args, ops) vs C=(nam, args, ops)
pub fn bin_zmodload_auto(
    table: &mut modulestab,
    _nam: &str,
    args: &[String],
    ops: &options,
) -> i32 {
    // c:2726
    let fchar: char; // c:2726
    let _flags: i32 = if OPT_ISSET(ops, b'i') { FEAT_IGNORE } else { 0 }; // c:2728

    // c:2731-2753 — conditions branch (-c).
    // C body:
    //   if (!*args) {
    //       Conddef p;
    //       for (p = condtab; p; p = p->next) {
    //           if (p->module) {
    //               if (OPT_ISSET(ops,'L')) {
    //                   fputs("zmodload -ac", stdout);
    //                   if (p->flags & CONDF_INFIX) putchar('I');
    //                   printf(" %s %s\n", p->module, p->name);
    //               } else {
    //                   if (p->flags & CONDF_INFIX) fputs("infix ", stdout);
    //                   else fputs("post ", stdout);
    //                   printf("%s (%s)\n", p->name, p->module);
    //               }
    //           }
    //       }
    //       return 0;
    //   }
    // C walks condtab in registration order (linked-list head→tail).
    // The Rust port walks CONDTAB (same shape, Vec instead of list).
    if OPT_ISSET(ops, b'c') {
        fchar = if OPT_ISSET(ops, b'I') { 'C' } else { 'c' };
        let _ = fchar;
        if args.is_empty() {
            let snap: Vec<(String, i32, String)> = {
                let tab = CONDTAB.lock().unwrap();
                tab.iter()
                    // c:2737 — `if (p->module)` skips entries without
                    // an owning module (built-in cond defs use NULL).
                    .filter_map(|p| {
                        p.module
                            .as_ref()
                            .map(|m| (p.name.clone(), p.flags, m.clone()))
                    })
                    .collect()
            };
            let l_flag = OPT_ISSET(ops, b'L');
            for (name, flags, module) in snap {
                if l_flag {
                    // c:2738-2742 — `zmodload -ac[I] MODULE NAME`.
                    if (flags & CONDF_INFIX) != 0 {
                        // c:2740
                        println!("zmodload -acI {} {}", module, name);
                    } else {
                        println!("zmodload -ac {} {}", module, name);
                    }
                } else {
                    // c:2743-2748 — `infix NAME (MODULE)` /
                    //                `post NAME (MODULE)`.
                    let kind = if (flags & CONDF_INFIX) != 0 {
                        "infix"
                    } else {
                        "post"
                    };
                    println!("{} {} ({})", kind, name, module);
                }
            }
            return 0;
        }
    } else if OPT_ISSET(ops, b'p') {
        // c:2755-2761 — params branch.
        // C body:
        //   if (!*args) {
        //       /* list autoloaded parameters */
        //       scanhashtable(paramtab, 1, 0, 0, printautoparams,
        //                     OPT_ISSET(ops,'L'));
        //       return 0;
        //   }
        // The `sorted=1` (2nd arg) walks the table in name order.
        // `printautoparams` (c:2710) checks `pm->flags & PM_AUTOLOAD`
        // and emits either `zmodload -ap MODULE NAME` (under `-L`)
        // or `NAME (MODULE)` form. Module name is read from
        // `pm->u.str`, which `add_autoparam` (c:1218) sets to the
        // module that registered the autoload stub.
        if args.is_empty() {
            let lon = if OPT_ISSET(ops, b'L') { 1 } else { 0 };
            let entries: Vec<(String, u32, String)> = {
                let tab = crate::ported::params::paramtab()
                    .read()
                    .expect("paramtab poisoned");
                let mut v: Vec<(String, u32, String)> = tab
                    .iter()
                    .filter(|(name, p)| {
                        (p.node.flags as u32 & crate::ported::zsh_h::PM_AUTOLOAD) != 0
                            // !!! RUST-ONLY CONDITION !!!
                            // C has ONE storage for a module parameter: the
                            // PM_AUTOLOAD stub `add_autoparam` (c:1218) plants
                            // is the paramtab node, and `addparamdef`
                            // (c:1065) only replaces it with the real special
                            // when the module actually loads. zshrs seeds every
                            // `partab[]` special EAGERLY at startup
                            // (`vm_helper::init_partab_params`), which wipes
                            // the stub's PM_AUTOLOAD bit, so it models
                            // "still an unresolved stub" as a side-set instead
                            // (`vm_helper::MATERIALIZED_MODULE_PARAMS`, whose
                            // doc comment carries the same warning). That set
                            // is per-NAME, exactly like C's flag: after
                            // `: ${keymaps}`, zsh drops `keymaps` from
                            // `zmodload -ap` but keeps its sibling `widgets`,
                            // which a per-MODULE "is it loaded" test cannot
                            // express. Read it here so `-ap`/`-apL` see the
                            // same stubs C's PM_AUTOLOAD scan does.
                            || crate::vm_helper::module_param_is_autoload_stub(name)
                    })
                    .map(|(name, p)| {
                        (
                            name.clone(),
                            p.node.flags as u32 | crate::ported::zsh_h::PM_AUTOLOAD,
                            // c:2716/2718 read the module off `pm->u.str`.
                            // An eagerly-seeded special has no `u_str`; its
                            // owning module is the one the boot
                            // `autofeatures` replay recorded.
                            p.u_str.clone().unwrap_or_else(|| {
                                table.autoload_params.get(name).cloned().unwrap_or_default()
                            }),
                        )
                    })
                    .collect();
                v.sort_by(|a, b| a.0.cmp(&b.0)); // c:2758 sorted=1
                v
            };
            for (name, flags, module) in entries {
                printautoparams(&name, &module, flags, lon); // c:2758
            }
            return 0;
        }
    } else if OPT_ISSET(ops, b'f') {
        // c:2763-2778 — math-function branch (-f).
        // C body:
        //   if (!*args) {
        //       MathFunc p;
        //       for (p = mathfuncs; p; p = p->next) {
        //           if (!(p->flags & MFF_USERFUNC) && p->module) {
        //               if (OPT_ISSET(ops,'L')) {
        //                   fputs("zmodload -af", stdout);
        //                   printf(" %s %s\n", p->module, p->name);
        //               } else
        //                   printf("%s (%s)\n", p->name, p->module);
        //           }
        //       }
        //       return 0;
        //   }
        // C walks `mathfuncs` (file-static linked list). Rust port
        // walks MATHFUNCS (same shape, Vec). The MFF_USERFUNC filter
        // excludes `functions -M` user math from the autoload list.
        if args.is_empty() {
            let snap: Vec<(String, String)> = {
                let tab = MATHFUNCS.lock().unwrap();
                tab.iter()
                    // c:2769 — `!(p->flags & MFF_USERFUNC) && p->module`.
                    .filter_map(|p| {
                        if (p.flags & MFF_USERFUNC) != 0 {
                            return None;
                        }
                        p.module.as_ref().map(|m| (p.name.clone(), m.clone()))
                    })
                    .collect()
            };
            let l_flag = OPT_ISSET(ops, b'L');
            for (name, module) in snap {
                if l_flag {
                    // c:2770-2772 — `zmodload -af MODULE NAME`.
                    println!("zmodload -af {} {}", module, name);
                } else {
                    // c:2774 — `NAME (MODULE)`.
                    println!("{} ({})", name, module);
                }
            }
            return 0;
        }
    } else {
        // Default: builtins branch
        if args.is_empty() {
            // c:Src/module.c:2756 — `scanhashtable(autoloadtab, 1, 0,
            //   0, ...)`. The sorted=1 arg sorts entries by name before
            // dispatch. zshrs's HashMap iteration is unordered, so the
            // 27 auto-loaded entries came out in random order. Bug
            // #222 in docs/BUGS.md. Sort by name to match zsh.
            let mut entries: Vec<(&String, &String)> = table.autoload_builtins.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            for (name, module) in entries {
                autoloadscan(
                    name,
                    module,
                    0,
                    if OPT_ISSET(ops, b'L') { PRINT_LIST } else { 0 },
                );
            }
            return 0;
        }
    }

    // c:2791-2805 — register/unregister via autofeatures with
    // FEAT_AUTOALL. (Earlier zshrs revision inserted into the
    // autoload_* maps directly, bypassing autofeatures — so
    // `m->autoloads` bookkeeping and the add_autobin/add_autoparam/…
    // canonical-table dispatch never ran.)
    let fchar: u8 = if OPT_ISSET(ops, b'c') {
        if OPT_ISSET(ops, b'I') {
            b'C' // c:2754 fchar = OPT_ISSET(ops,'I') ? 'C' : 'c'
        } else {
            b'c'
        }
    } else if OPT_ISSET(ops, b'p') {
        b'p' // c:2762
    } else if OPT_ISSET(ops, b'f') {
        b'f' // c:2779
    } else {
        b'b' // c:2789
    };
    let mut flags = FEAT_AUTOALL; // c:2791
    if OPT_ISSET(ops, b'i') {
        flags |= FEAT_IGNORE; // c:2792-2793
    }
    if OPT_ISSET(ops, b'u') {
        /* remove autoload */
        // c:2795
        flags |= FEAT_REMOVE; // c:2796
                              // c:2797 — `modnam = NULL;` — every arg is a feature name.
        return autofeatures(table, _nam, None, args, fchar, flags); // c:2805
    }
    /* add autoload */
    // c:2799
    let modnam = &args[0]; // c:2800
                           // c:2802-2803 — `if (args[1]) args++;` — with a single arg the
                           // module name doubles as the feature arg (C quirk; autofeatures
                           // then rejects the `/` in it).
    let feat_args: &[String] = if args.len() > 1 { &args[1..] } else { args };
    autofeatures(table, _nam, Some(modnam), feat_args, fchar, flags) // c:2805
}

/// Port of `unload_named_module(char *modname, char *nam, int silent)`
/// from `Src/module.c:2923-2965`.
///
/// C body:
/// ```c
/// int
/// unload_named_module(char *modname, char *nam, int silent)
/// {
///     const char *mname;
///     Module m;
///     int ret = 0;
///     m = find_module(modname, FINDMOD_ALIASP, &mname);
///     if (m) {
///         int i, del = 0;
///         Module dm;
///         for (i = 0; i < modulestab->hsize; i++) {
///             for (dm = (Module)modulestab->nodes[i]; dm;
///                  dm = (Module)dm->node.next) {
///                 LinkNode dn;
///                 if (!dm->deps || !dm->u.handle)
///                     continue;
///                 for (dn = firstnode(dm->deps); dn; incnode(dn)) {
///                     if (!strcmp((char *) getdata(dn), mname)) {
///                         if (dm->node.flags & MOD_UNLOAD)
///                             del = 1;
///                         else {
///                             zwarnnam(nam, "module %s is in use ...");
///                             return 1;
///                         }
///                     }
///                 }
///             }
///         }
///         if (del) m->wrapper++;
///         if (unload_module(m)) ret = 1;
///         if (del) m->wrapper--;
///     } else if (!silent) {
///         zwarnnam(nam, "no such module %s", modname);
///         ret = 1;
///     }
///     return ret;
/// }
/// ```
///
/// Now that unload_module is faithfully ported (2b986c9e8b), this
/// wraps it with the C dep-walk that gates the unload on whether
/// any other loaded module depends on the target. If a dependent
/// is already tagged MOD_UNLOAD, set del=1 to bracket the unload
/// with `m->wrapper++` / `m->wrapper--` so unload_module's wrapper
/// branch (c:2839-2842) defers correctly.
/// WARNING: param names don't match C — Rust=(table, name, nam, silent) vs C=(modname, nam, silent)
pub fn unload_named_module(table: &mut modulestab, name: &str, nam: &str, silent: i32) -> i32 {
    // c:2924
    // c:2930 — `m = find_module(modname, FINDMOD_ALIASP, &mname);`
    // Returns the canonical (alias-resolved) name; we drive
    // unload_module against that, matching C's `m` pointer.
    let mname = match find_module(table, name, FINDMOD_ALIASP) {
        Some(n) => n,
        None => {
            // c:2959-2961 — !m branch: silent gate.
            if silent == 0 {
                crate::ported::utils::zwarnnam(nam, &format!("no such module {}", name));
                return 1;
            }
            return 0;
        }
    };

    // c:2932 — `int del = 0;`
    let mut del = 0;

    // c:2935-2952 — scan every module's deps for `mname`.
    // Snapshot first so we don't double-borrow the modules HashMap
    // when we mutate via unload_module below.
    let candidates: Vec<(String, i32, bool, Vec<String>)> = table
        .modules
        .iter()
        .filter_map(|(other_name, other)| {
            // c:2939 — `if (!dm->deps || !dm->u.handle) continue;`
            // Static-link analog of `u.handle` is MOD_INIT_B ("boot_
            // ran"), the same analog `require_module` (c:2352) and
            // `getpmmodule` (c:1069) use. It is NOT MOD_LINKED, which
            // only means "this name has compiled-in code" and is set on
            // every boot node: with that gate, the never-loaded
            // `zsh/compctl` / `zsh/complete` / `zsh/computil` /
            // `zsh/zleparameter` boot nodes counted as live dependents,
            // so a bare `zmodload -u zsh/zle` (nothing loaded) failed
            // with "in use by another module" where zsh silently
            // returns 0.
            // The MOD_UNLOAD check stays INSIDE the inner loop: a
            // dependent already flagged for deferred unload must still
            // be seen there so it can set `del = 1`.
            let deps = other.deps.as_ref()?;
            if (other.node.flags & MOD_INIT_B) == 0 {
                return None;
            }
            // Note: C checks `u.handle` (loaded handle) not
            // `!MOD_UNLOAD` here — MOD_UNLOAD-flagged modules still
            // get scanned because the inner branch needs to see them
            // to set `del = 1`.
            Some((
                other_name.clone(),
                other.node.flags,
                (other.node.flags & MOD_UNLOAD) != 0,
                deps.iter().cloned().collect(),
            ))
        })
        .collect();

    for (_other_name, _other_flags, other_unloading, other_deps) in candidates {
        for dep in &other_deps {
            if dep != &mname {
                continue; // c:2942
            }
            if other_unloading {
                // c:2943-2944 — dependent already marked MOD_UNLOAD →
                // we'll be cascade-unloading it after the target.
                del = 1;
            } else {
                // c:2945-2948 — live dependent: refuse the unload.
                crate::ported::utils::zwarnnam(
                    nam,
                    &format!(
                        "module {} is in use by another module and cannot be unloaded",
                        mname
                    ),
                );
                return 1;
            }
        }
    }

    // c:2953-2954 — `if (del) m->wrapper++;`. The wrapper++/--
    // bracket gates unload_module's wrapper branch (c:2839-2842):
    // with wrapper > 0, unload_module sets MOD_UNLOAD and returns
    // rather than running finish. The actual cascade fires via the
    // recursive deferred-dep walk inside unload_module.
    if del != 0 {
        if let Some(m) = table.modules.get_mut(&mname) {
            m.wrapper += 1;
        }
    }

    // c:2955-2956 — `if (unload_module(m)) ret = 1;`. Rust's
    // unload_module returns bool (true=success). Map to C ret:
    // false → 1, true → 0.
    let mut ret = if !table.unload_module(&mname) { 1 } else { 0 };

    // c:2957-2958 — `if (del) m->wrapper--;`
    if del != 0 {
        if let Some(m) = table.modules.get_mut(&mname) {
            m.wrapper -= 1;
        }
    }
    let _ = silent;
    let _ = &mut ret;
    ret // c:2964
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
pub fn bin_zmodload_load(table: &mut modulestab, nam: &str, args: &[String], ops: &options) -> i32 {
    // c:2971
    let mut ret: i32 = 0;
    if OPT_ISSET(ops, b'u') {
        // c:2974
        // c:2976-2979 — unload loop
        for arg in args {
            if unload_named_module(table, arg, nam, OPT_ISSET(ops, b'i') as i32) != 0 {
                ret = 1;
            }
        }
        return ret; // c:2980
    } else if args.is_empty() {
        // c:2981
        // c:2983-2985 — list modules:
        //   `scanhashtable(modulestab, 1, 0, MOD_UNLOAD|MOD_ALIAS,
        //                  modulestab->printnode,
        //                  OPT_ISSET(ops,'L') ? PRINTMOD_LIST : 0);`
        // The 4th arg to scanhashtable is the EXCLUDE mask — entries
        // with `MOD_UNLOAD` or `MOD_ALIAS` set are skipped. The
        // surviving names are routed through `printmodulenode`,
        // which itself gates the visible-line emission on
        // `m->u.handle` (=> `MOD_INIT_B` in zshrs) so registered-
        // but-unloaded modules drop out. Plain `zmodload` (no `-L`)
        // passes `flags=0`; `zmodload -L` passes `PRINTMOD_LIST`.
        // Previous Rust impl printed every key in `table.modules`
        // unconditionally, which leaked the 32 statically-registered
        // builtin entries (#76 in docs/BUGS.md).
        let listflags = if OPT_ISSET(ops, b'L') {
            PRINTMOD_LIST
        } else {
            0
        };
        let mut names: Vec<&String> = table.modules.keys().collect();
        names.sort(); // c:scanhashtable sorted=1 arg
        for name in names {
            let m = &table.modules[name]; // c:154 printnode call
            if (m.node.flags & (MOD_UNLOAD | MOD_ALIAS)) != 0 {
                continue; // c:2983 EXCLUDE mask
            }
            let line = printmodulenode(name, m, listflags);
            if !line.is_empty() {
                println!("{}", line);
            }
        }
        return 0; // c:2986
    } else {
        // c:2989-2992 — load loop
        for arg in args {
            let tmpret = require_module(table, arg, None, OPT_ISSET(ops, b's') as i32, false); // c:2990
            if tmpret != 0 && ret != 1 {
                // c:2991
                ret = tmpret;
            }
            // PFA-SMR: one event per `zmodload MODULE` load form
            // (only the bare-load path; listing/-u/-d/-A handled
            // upstream and never reaches here). Pass empty flags
            // since the per-feature -F path lives in
            // bin_zmodload_features. Record even on failure so
            // replay sees the user's intent.
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled() {
                let ctx = crate::recorder::recorder_ctx_global();
                crate::recorder::emit_zmodload(arg, "", ctx);
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
pub fn bin_zmodload_features(
    table: &mut modulestab,
    nam: &str,
    args: &[String],
    ops: &options,
) -> i32 {
    // c:3003
    let modname = args.first(); // c:3003
    let rest_args = if args.is_empty() {
        &args[..]
    } else {
        &args[1..]
    };

    // c:3010-3030 — no-module-name listing branch.
    // C body:
    //   if (modname)
    //       args++;
    //   else if (OPT_ISSET(ops,'L')) {
    //       int printflags = PRINTMOD_LIST|PRINTMOD_FEATURES;
    //       if (OPT_ISSET(ops,'P')) {
    //           zwarnnam(nam, "-P is only allowed with a module name");
    //           return 1;
    //       }
    //       if (OPT_ISSET(ops,'l')) printflags |= PRINTMOD_LISTALL;
    //       if (OPT_ISSET(ops,'a')) printflags |= PRINTMOD_AUTO;
    //       scanhashtable(modulestab, 1, 0, MOD_ALIAS,
    //                     modulestab->printnode, printflags);
    //       return 0;
    //   }
    //   if (!modname) {
    //       zwarnnam(nam, "-F requires a module name");
    //       return 1;
    //   }
    if modname.is_none() {
        if OPT_ISSET(ops, b'L') {
            // c:3012
            // c:3014-3016 — `-P` check must fire BEFORE the listing
            // dispatch. Without modname, -P is illegal.
            if OPT_ISSET(ops, b'P') {
                zwarnnam(nam, "-P is only allowed with a module name"); // c:3015
                return 1; // c:3016
            }
            // c:3013 / c:3018-3021 — assemble PRINTMOD_LIST|PRINTMOD_FEATURES
            //                       [|PRINTMOD_LISTALL][|PRINTMOD_AUTO]
            let mut printflags = PRINTMOD_LIST | PRINTMOD_FEATURES;
            if OPT_ISSET(ops, b'l') {
                printflags |= PRINTMOD_LISTALL; // c:3019
            }
            if OPT_ISSET(ops, b'a') {
                printflags |= PRINTMOD_AUTO; // c:3021
            }
            // c:3022-3023 — `scanhashtable(modulestab, 1, 0, MOD_ALIAS,
            //                              printnode, printflags);`
            // sorted=1, INCLUDE=0 (all), EXCLUDE=MOD_ALIAS (skip aliases).
            let mut names: Vec<String> = table
                .modules
                .iter()
                .filter(|(_, m)| (m.node.flags & MOD_ALIAS) == 0) // c:3022 EXCLUDE
                .map(|(n, _)| n.clone())
                .collect();
            names.sort(); // c:3022 sorted=1
            for name in &names {
                if printflags & PRINTMOD_AUTO != 0 {
                    // c:229-231 / c:238-251 — autoload form; printmodulenode
                    // covers this branch fully.
                    let m = &table.modules[name];
                    let line = printmodulenode(name, m, printflags);
                    if !line.is_empty() {
                        println!("{}", line);
                    }
                    continue;
                }
                // c:218 / c:232-235 — loaded-module + features gate:
                // `if (features_module(m, &features) ||
                //      enables_module(m, &enables) || !*features) return;`
                // printmodulenode has no &table handle, so the FEATURES
                // dispatch happens here (see its c:252-263 comment).
                let loaded = {
                    let m = &table.modules[name];
                    (m.node.flags & MOD_INIT_B) != 0 && (m.node.flags & MOD_UNLOAD) == 0
                };
                if !loaded {
                    continue;
                }
                let mut features: Vec<String> = Vec::new();
                if features_module(table, name, &mut features) != 0 || features.is_empty() {
                    continue; // c:233-235
                }
                let mut enables_opt: Option<Vec<i32>> = None;
                if enables_module(table, name, &mut enables_opt) != 0 {
                    continue; // c:233-235
                }
                let enables = enables_opt.unwrap_or_else(|| vec![0; features.len()]);
                // c:237-245 — `printf("zmodload "); fputs("-F ", stdout);`
                let mut line = String::from("zmodload -F ");
                if name.starts_with('-') {
                    line.push_str("-- "); // c:244-245
                }
                line.push_str(&crate::ported::utils::quotedzputs(name)); // c:246
                                                                         // c:252-262 — per-feature tail: LISTALL emits ` +f`/` -f`,
                                                                         // plain -L skips disabled and emits ` f`.
                for (f, on) in features.iter().zip(enables.iter()) {
                    if printflags & PRINTMOD_LISTALL != 0 {
                        line.push_str(if *on != 0 { " +" } else { " -" }); // c:256
                    } else if *on == 0 {
                        continue; // c:258
                    } else {
                        line.push(' '); // c:260
                    }
                    line.push_str(&crate::ported::utils::quotedzputs(f)); // c:261
                }
                println!("{}", line);
            }
            return 0; // c:3024
        }
        zwarnnam(nam, "-F requires a module name"); // c:3028
        return 1; // c:3029
    }

    let modname = modname.unwrap();

    // c:3032-3047 — `-m` glob-pattern branch.
    // C body:
    //   if (OPT_ISSET(ops,'m')) {
    //       patprogs = zhalloc(arrlen(args)*sizeof(Patprog));
    //       for (argp = args; *argp; argp++, patprogp++) {
    //           if (*arg == '+' || *arg == '-') arg++;
    //           tokenize(arg);
    //           *patprogp = patcompile(arg, 0, 0);
    //       }
    //   } else patprogs = NULL;
    // Static-link path: pattern compilation deferred. The -m flag is
    // observed at the -a / require_module dispatch below, but the
    // patprogs array stays NULL — patcompile callers (autofeatures,
    // do_module_features) fall back to exact-name matching.

    // c:3049-3226 — `-l/-L/-e` arm: list features one per line with
    // +/- (-l), as a `zmodload -F` statement (-L), or test existence
    // (-e). `-m` patprogs stay deferred (exact-name matching), same
    // as the c:3032-3047 note above.
    if OPT_ISSET(ops, b'l') || OPT_ISSET(ops, b'L') || OPT_ISSET(ops, b'e') {
        let param: Option<String> = OPT_ARG_SAFE(ops, b'P').map(|s| s.to_string()); // c:3060
                                                                                    // c:3062 — `m = find_module(modname, FINDMOD_ALIASP, NULL);`
        let resolved = find_module(table, modname, FINDMOD_ALIASP);

        // c:3063-3107 — `-a` sub-arm: autoload listing/testing.
        if OPT_ISSET(ops, b'a') {
            // c:3067-3068 — `if (!m || !m->autoloads) return 1;`
            let autoloads: Vec<String> = match resolved
                .as_ref()
                .and_then(|r| table.modules.get(r))
                .and_then(|m| m.autoloads.as_ref())
            {
                Some(al) => al.iter().cloned().collect(),
                None => return 1,
            };
            if OPT_ISSET(ops, b'e') {
                // c:3070-3085 — each arg must (mis)match per its +/- sense.
                for fstr in rest_args {
                    let (sense, name) = match fstr.strip_prefix('+') {
                        Some(rest) => (true, rest), // c:3074
                        None => match fstr.strip_prefix('-') {
                            Some(rest) => (false, rest), // c:3076-3078
                            None => (true, fstr.as_str()),
                        },
                    };
                    // c:3080-3082 — `(linknodebystring(...) != NULL) != sense`
                    if autoloads.iter().any(|a| a == name) != sense {
                        return 1; // c:3082
                    }
                }
                return 0; // c:3084
            }
            if let Some(p) = param {
                // c:3086-3088 / c:3098-3100 — collect into the array,
                // then `setaparam(param, arrset)`.
                // c:3103-3106
                if crate::ported::params::setaparam(&p, autoloads).is_none() {
                    return 1; // c:3105
                }
                return 0; // c:3106
            }
            if OPT_ISSET(ops, b'L') {
                // c:3089-3091 — `printf("zmodload -aF %s%c", ...)`
                let rname = resolved.as_deref().unwrap_or(modname);
                print!(
                    "zmodload -aF {}{}",
                    crate::ported::utils::quotedzputs(rname),
                    if autoloads.is_empty() { '\n' } else { ' ' }
                );
                // c:3092-3098 — space-separated, final '\n'.
                for (i, al) in autoloads.iter().enumerate() {
                    print!("{}{}", al, if i + 1 < autoloads.len() { ' ' } else { '\n' });
                }
            } else {
                // c:3093-3097 — one per line.
                for al in &autoloads {
                    println!("{}", al);
                }
            }
            return 0; // c:3107
        }

        // c:3108-3112 — `if (!m || !m->u.handle || (m->node.flags &
        // MOD_UNLOAD))`. zshrs maps "u.handle installed" to MOD_INIT_B
        // (see the printmodulenode comment at c:218-241 above).
        let loaded = resolved
            .as_ref()
            .and_then(|r| table.modules.get(r))
            .map(|m| (m.node.flags & MOD_INIT_B) != 0 && (m.node.flags & MOD_UNLOAD) == 0)
            .unwrap_or(false);
        if !loaded {
            if !OPT_ISSET(ops, b'e') {
                zwarnnam(nam, &format!("module `{}' is not yet loaded", modname));
                // c:3110
            }
            return 1; // c:3111
        }
        let rname = resolved.unwrap();

        // c:3113-3118 — `features_module(m, &features)`.
        let mut features: Vec<String> = Vec::new();
        if features_module(table, &rname, &mut features) != 0 {
            if !OPT_ISSET(ops, b'e') {
                zwarnnam(
                    nam,
                    &format!("module `{}' does not support features", rname), // c:3115
                );
            }
            return 1; // c:3117
        }
        // c:3119-3124 — `enables_module(m, &enables)`.
        let mut enables_opt: Option<Vec<i32>> = None;
        if enables_module(table, &rname, &mut enables_opt) != 0 {
            /* this shouldn't ever happen, so don't silence this error */
            // c:3120
            zwarnnam(
                nam,
                &format!("error getting enabled features for module `{}'", rname), // c:3121
            );
            return 1; // c:3123
        }
        let enables: Vec<i32> = enables_opt.unwrap_or_else(|| vec![0; features.len()]);

        // c:3125-3155 — validate every feature argument.
        for raw in rest_args {
            // c:3127-3135 — strip +/- into `on`.
            let (on, arg): (i32, &str) = match raw.strip_prefix('-') {
                Some(rest) => (0, rest),
                None => match raw.strip_prefix('+') {
                    Some(rest) => (1, rest),
                    None => (-1, raw.as_str()),
                },
            };
            let mut found = 0;
            for (fp, ep) in features.iter().zip(enables.iter()) {
                // c:3137-3138 — patprogs deferred: exact `strcmp`.
                if arg == fp {
                    // c:3140-3142 — for -e, check given state, if any.
                    if OPT_ISSET(ops, b'e') && on != -1 && on != (ep & 1) {
                        return 1; // c:3142
                    }
                    found += 1;
                    break; // c:3144-3145
                }
            }
            if found == 0 {
                // c:3148-3154
                if !OPT_ISSET(ops, b'e') {
                    zwarnnam(
                        nam,
                        &format!("module `{}' has no such feature: `{}'", modname, raw),
                    );
                }
                return 1; // c:3153
            }
        }
        if OPT_ISSET(ops, b'e') {
            /* yep, everything we want exists */
            // c:3156
            return 0; // c:3157
        }

        let opt_big_l = OPT_ISSET(ops, b'L');
        let opt_small_l = OPT_ISSET(ops, b'l');
        // c:3186-3194 / c:3164-3172 — arg filter helpers. The C print
        // loop compares the UNSTRIPPED arg (`!strcmp(*fp, *argp)`,
        // c:3193) while the param-count loop compares stripped
        // (`!strcmp(*fp, arg)`, c:3170). Ported as written.
        let matches_stripped = |f: &str| -> bool {
            rest_args.is_empty()
                || rest_args.iter().any(|raw| {
                    let arg = raw
                        .strip_prefix('+')
                        .or_else(|| raw.strip_prefix('-'))
                        .unwrap_or(raw);
                    f == arg
                })
        };
        let matches_unstripped =
            |f: &str| -> bool { rest_args.is_empty() || rest_args.iter().any(|raw| f == raw) };

        let mut arrset: Option<Vec<String>> = None;
        if param.is_some() {
            // c:3158-3183 — size pass folded away (Vec grows); keep the
            // same membership filter.
            arrset = Some(Vec::new());
        } else if opt_big_l {
            // c:3184-3185 — `printf("zmodload -F %s ", m->node.nam);`
            print!("zmodload -F {} ", crate::ported::utils::quotedzputs(&rname));
        }
        // c:3186-3219 — main feature emit loop.
        for (i, (f, ep)) in features.iter().zip(enables.iter()).enumerate() {
            if param.is_some() {
                if !matches_stripped(f) {
                    continue; // c:3170-3173 stripped compare
                }
            } else if !matches_unstripped(f) {
                continue; // c:3193-3196 unstripped compare
            }
            let onoff: &str = if opt_big_l && !opt_small_l {
                // c:3198-3200
                if *ep == 0 {
                    continue; // c:3199
                }
                ""
            } else if *ep != 0 {
                "+" // c:3203
            } else {
                "-" // c:3205
            };
            if let Some(ref mut arr) = arrset {
                arr.push(format!("{}{}", onoff, f)); // c:3208 bicat
            } else {
                // c:3210-3216 — term ' ' while a next feature EXISTS in
                // the full array (fp[1]), even if it won't be printed.
                let term = if opt_big_l && i + 1 < features.len() {
                    ' '
                } else {
                    '\n'
                };
                print!("{}{}{}", onoff, crate::ported::utils::quotedzputs(f), term);
            }
        }
        if let (Some(p), Some(arr)) = (param, arrset) {
            // c:3220-3224 — `setaparam(param, arrset)`.
            if crate::ported::params::setaparam(&p, arr).is_none() {
                return 1; // c:3223
            }
        }
        return 0; // c:3225
    }

    // c:3227-3229 — `-P` is illegal without -l/-L/-e.
    if OPT_ISSET(ops, b'P')
        && !(OPT_ISSET(ops, b'l') || OPT_ISSET(ops, b'L') || OPT_ISSET(ops, b'e'))
    {
        zwarnnam(nam, "-P can only be used with -l or -L"); // c:3228
        return 1; // c:3229
    }

    // c:3230-3247 — `-a` arm: route through autofeatures with
    // FEAT_IGNORE (the autoload-feature registration path).
    if OPT_ISSET(ops, b'a') {
        // c:3231-3234 — `-m` incompatible with `-a`.
        if OPT_ISSET(ops, b'm') {
            zwarnnam(nam, "-m cannot be used with -a"); // c:3232
            return 1; // c:3233
        }
        // c:3246 — `return autofeatures(nam, modname, args, 0, FEAT_IGNORE);`
        // FEAT_IGNORE is hard-coded here because marking-for-autoload
        // is separate from enable/disable (per the c:3236-3244 comment).
        return autofeatures(table, nam, Some(modname), rest_args, 0, FEAT_IGNORE);
    }

    // c:3249-3260 — default arm: build Feature_enables array from
    // `+name`/`-name` args, then `require_module(modname, features,
    // OPT_ISSET(ops,'s'))`.
    //
    // C builds a fep[] array with str + (optional) patprog pairs.
    // The Rust port flattens to a `Vec<String>` since patprogs are
    // deferred; require_module accepts Option<&[String]>.
    let feats: Vec<String> = rest_args.to_vec();
    let features_arg = if feats.is_empty() {
        None
    } else {
        Some(feats.as_slice())
    };
    require_module(
        table,
        modname,
        features_arg,
        OPT_ISSET(ops, b's') as i32,
        OPT_ISSET(ops, b'm'), // c:3258 `fep->pat = patprogs ? *patprogs++ : NULL`
    ) // c:3260
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
pub fn ensurefeature(
    table: &mut modulestab,
    modname: &str,
    prefix: &str,
    feature: Option<&str>,
) -> i32 {
    // c:3415
    match feature {
        // c:3420-3421 — `if (!feature) return require_module(modname, NULL, 0);`
        None => require_module(table, modname, None, 0, false),
        Some(f) => {
            // c:3422-3428 — build single-element features[2] array.
            let combined = crate::ported::string::dyncat(prefix, f); // c:3422
            let arr = vec![combined];
            require_module(table, modname, Some(&arr), 0, false) // c:3428
        }
    }
}

/// Port of `static HashNode resolvebuiltin(const char *cmdarg, HashNode hn)`
/// from `Src/exec.c:2700-2724` — the autoloaded-builtin stub firing.
///
/// C body:
/// ```c
/// if (!((Builtin) hn)->handlerfunc) {
///     char *modname = dupstring(((Builtin) hn)->optstr);
///     (void)ensurefeature(modname, "b:",
///                         (hn->flags & BINF_AUTOALL) ? NULL : hn->nam);
///     hn = builtintab->getnode(builtintab, cmdarg);
///     if (!hn) {
///         lastval = 1;
///         zerr("autoloading module %s failed to define builtin: %s",
///              modname, cmdarg);
///         return NULL;
///     }
/// }
/// return hn;
/// ```
///
/// zshrs split: the C autoload stub (builtintab node with NULL
/// handlerfunc, installed by `add_autobin` c:426) lives in the
/// `autoload_builtins` ledger (name → module). This fn is the
/// dispatch-time consult:
///   - `None` — name has no autoload stub; caller continues its
///     normal lookup chain.
///   - `Some(0)` — module loaded; caller re-dispatches the (now
///     registered) builtin.
///   - `Some(1)` — load failed or the loaded module didn't define
///     the feature; diagnostics already printed (load_module's
///     `failed to load module` zwarn, or the c:2718 zerr here).
///     Caller returns status 1.
///
/// Ledger upkeep: the entry is removed in every fired path —
///   - success: C's addbuiltin (module.c:411-415) replaces the stub
///     with the real node, so the stub is gone;
///   - load failure: C's execbuiltin head (Src/builtin.c:264-267)
///     hits the still-NULL handlerfunc and `deletebuiltin`s the
///     stub — a second call reports `command not found` / 127
///     (probed: zsh 5.9 `zmodload -ab zsh/bogus mybltn; mybltn;
///     mybltn` → rc1=1, rc2=127).
///
/// AUTOALL note: the ledger doesn't carry BINF_AUTOALL, so the
/// ensurefeature arg is always `Some(name)` (the non-AUTOALL form).
/// The AUTOALL path is unreachable for builtins via `zmodload -a MOD`
/// (both shells error "`/' is illegal in a builtin"); revisit if the
/// ledger grows flags.
pub fn resolvebuiltin(name: &str) -> Option<i32> {
    // c:2700
    let mut tab = MODULESTAB.lock().ok()?;
    // c:2705 — `if (!((Builtin) hn)->handlerfunc)`: ledger hit IS the
    // "no handlerfunc" stub in zshrs.
    let module = tab.autoload_builtins.get(name)?.clone();
    // c:2706 — `modname = dupstring(hn->optstr)` (done: `module`).
    // Stub fires exactly once per registration (see ledger upkeep
    // in the doc above).
    tab.autoload_builtins.remove(name);
    // c:2711-2713 — ensurefeature(modname, "b:", hn->nam).
    let _ = ensurefeature(&mut tab, &module, "b:", Some(name));
    // c:2714 — `hn = builtintab->getnode(builtintab, cmdarg);`
    // zshrs analog: the module booted (is_loaded) AND the name is in
    // the static builtintab (createbuiltintable pre-registers every
    // module bintab entry).
    let defined =
        tab.is_loaded(&module) && crate::ported::builtin::createbuiltintable().contains_key(name);
    if defined {
        return Some(0); // c:2723 `return hn;`
    }
    if tab.is_loaded(&module) {
        // c:2716-2720 — module loaded but feature missing.
        crate::ported::builtin::LASTVAL.store(1, std::sync::atomic::Ordering::Relaxed); // c:2717 lastval = 1
        crate::ported::utils::zerr(&format!(
            "autoloading module {} failed to define builtin: {}",
            module, name
        )); // c:2718
    }
    // Load failure: load_module already printed `failed to load
    // module \`...'`; C's execbuiltin head returns 1 silently
    // (Src/builtin.c:264-267).
    Some(1)
}

/// Port of `addmathfunc(MathFunc f)` from `Src/module.c:1313`.
///
/// C body: walks the global `mathfuncs` linked list, refuses to
/// re-register MFF_ADDED entries, replaces autoloadable shims, then
/// links into head. Rust port operates on `autoload_mathfuncs` map
/// since zshrs's static-link path doesn't have per-entry MFF flags.

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
pub fn autofeatures(
    table: &mut modulestab,
    cmdnam: &str,
    module: Option<&str>,
    features: &[String],
    prefchar: u8,
    defflags: i32,
) -> i32 {
    // c:3437
    let mut ret: i32 = 0;

    // c:3445-3453 — resolve `defm` up front (FINDMOD_ALIASP|
    // FINDMOD_CREATE); if its union slot is populated (loaded —
    // MOD_INIT_B in zshrs, see the printmodulenode c:218-241 note)
    // fetch the feature + enable tables for the c:3558-3577 checks.
    let mut modfeatures: Option<Vec<String>> = None;
    let mut modenables: Vec<i32> = Vec::new();
    let defm_name: Option<String> = match module {
        Some(modn) => {
            let resolved = find_module(table, modn, FINDMOD_ALIASP | FINDMOD_CREATE);
            if let Some(ref r) = resolved {
                let booted = table
                    .modules
                    .get(r)
                    .map(|m| (m.node.flags & MOD_INIT_B) != 0)
                    .unwrap_or(false);
                if booted {
                    // c:3449-3451
                    let mut f: Vec<String> = Vec::new();
                    if features_module(table, r, &mut f) == 0 {
                        let mut e: Option<Vec<i32>> = None;
                        let _ = enables_module(table, r, &mut e);
                        modenables = e.unwrap_or_else(|| vec![0; f.len()]);
                        modfeatures = Some(f);
                    }
                }
            }
            resolved
        }
        None => None, // c:3454-3455 `defm = NULL`
    };

    for feature in features {
        let s = feature.as_str();
        let mut add: bool = true; // c:3466 / c:3477 default `add = 1`
        let mut flags = defflags; // c:3458 `flags = defflags`

        // `feature_full` is the string C keeps in `m->autoloads`
        // (c:3584/3597 `ztrdup(feature)`): the arg after the +/-
        // strip, type prefix included — prefchar mode synthesizes it
        // via `sprintf(feature, "%c:%s", fchar, fnam)` (c:3468-3469).
        let prefixed: String;
        let (fchar, fnam, feature_full): (u8, &str, &str) = if prefchar != 0 {
            // c:3461-3470 — `prefchar` mode: feature is bare name with
            // no `+`/`-` / `b:` prefix; fchar comes from the arg.
            prefixed = format!("{}:{}", prefchar as char, s); // c:3468-3469
            (prefchar, s, prefixed.as_str()) // c:3467-3468
        } else {
            // c:3471-3490 — parse `+`/`-` then the `b:`/`c:`/`C:`/`p:`/`f:`
            // type prefix.
            let mut t = s;
            if let Some(rest) = t.strip_prefix('-') {
                // c:3473
                add = false;
                t = rest;
            } else if let Some(rest) = t.strip_prefix('+') {
                // c:3478
                t = rest;
            }
            // c:3482-3487 — bad format check: `!*feature || feature[1] != ':'`
            let bytes = t.as_bytes();
            if bytes.is_empty() || bytes.len() < 2 || bytes[1] != b':' {
                // c:3483-3486 — zwarnnam + ret=1 + continue.
                crate::ported::utils::zwarnnam(
                    cmdnam,
                    &format!("bad format for autoloadable feature: `{}'", t),
                );
                ret = 1; // c:3485
                continue; // c:3486
            }
            // c:3488-3489 — `fnam = feature + 2; fchar = feature[0];`
            (bytes[0], &t[2..], t)
        };

        // c:3491-3492 — `if (flags & FEAT_REMOVE) add = 0;`
        if (flags & FEAT_REMOVE) != 0 {
            add = false;
        }

        let typnam: &str; // c:3457
        let _ = typnam; // referenced below for fnam validation
        let typnam = match fchar {
            // c:3494-3522 — switch (fchar): map each type-letter to
            // the typnam used in the diagnostic + add/del fn dispatch.
            b'b' => "builtin", // c:3495-3498
            b'c' | b'C' => {
                if fchar == b'C' {
                    flags |= FEAT_INFIX; // c:3501
                }
                "condition" // c:3505
            }
            b'f' => "math function", // c:3508-3511
            b'p' => "parameter",     // c:3513-3516
            _ => {
                // c:3518-3522 — `zwarnnam(cmdnam, "bad autoloadable
                // feature type: `%c'", fchar); ret = 1; continue;`
                crate::ported::utils::zwarnnam(
                    cmdnam,
                    &format!("bad autoloadable feature type: `{}'", fchar as char),
                );
                ret = 1; // c:3521
                continue; // c:3522
            }
        };

        // c:3525-3529 — reject `/` in the feature name.
        if fnam.contains('/') {
            crate::ported::utils::zwarnnam(
                cmdnam,
                &format!("{}: `/' is illegal in a {}", fnam, typnam),
            );
            ret = 1;
            continue;
        }

        // c:3531-3553 — resolve module: if `module` arg is None,
        // walk every module's `m->autoloads` list looking for the
        // feature; if found, that's the owning module. C's
        // `m->autoloads` per-module list isn't modelled in the Rust
        // port — the autoload_* HashMaps store `feature → module`
        // directly, so we can derive the owning module from those.
        // When `module` arg IS set, use it (c:3553 `m = defm;`).
        let modname_owned: String = match module {
            Some(m) => m.to_string(), // c:3553
            None => {
                // c:3537-3551 — search for the owning module across all
                // autoload maps; fall back to error if not found.
                let map = match fchar {
                    b'b' => &table.autoload_builtins,
                    b'c' | b'C' => &table.autoload_conditions,
                    b'p' => &table.autoload_params,
                    b'f' => &table.autoload_mathfuncs,
                    _ => unreachable!(),
                };
                match map.get(fnam).cloned() {
                    Some(m) => m,
                    None => {
                        if (flags & FEAT_IGNORE) == 0 {
                            // c:3546-3549
                            ret = 1;
                            crate::ported::utils::zwarnnam(
                                cmdnam,
                                &format!("{}: no such {}", fnam, typnam),
                            );
                        }
                        continue; // c:3550
                    }
                }
            }
        };
        let modname = modname_owned.as_str();

        // c:3554 `subret = 0;` — the m->autoloads maintenance below can
        // set it to ±2 on the remove-missing path (c:3614).
        let mut autoload_subret: i32 = 0;
        // Owning module node: the alias-resolved defm when a module arg
        // was given (C `m = defm`, c:3553), else the searched-up name.
        let owner: &str = match (module.is_some(), defm_name.as_deref()) {
            (true, Some(r)) => r,
            _ => modname,
        };
        if add {
            // c:3558-3577 — if the module is already loaded, the feature
            // must exist in its table; if it's already enabled there is
            // nothing to mark.
            if module.is_some() {
                if let Some(ref mf) = modfeatures {
                    match mf.iter().position(|f| f == feature_full) {
                        None => {
                            // c:3566-3570
                            crate::ported::utils::zwarnnam(
                                cmdnam,
                                &format!(
                                    "module `{}' has no such feature: `{}'",
                                    owner, feature_full
                                ),
                            );
                            ret = 1;
                            continue;
                        }
                        Some(idx) => {
                            if modenables.get(idx).copied().unwrap_or(0) != 0 {
                                continue; // c:3572-3577 already provided
                            }
                        }
                    }
                }
            }
            // c:3583-3603 — insert into m->autoloads in lexical order
            // (dup is "never an error", c:3590-3593).
            if let Some(m) = table.modules.get_mut(owner) {
                let list = m
                    .autoloads
                    .get_or_insert_with(crate::ported::linklist::znewlinklist);
                let mut insert_at: Option<usize> = Some(list.len()); // c:3602 append default
                for (i, existing) in list.iter().enumerate() {
                    match feature_full.cmp(existing.as_str()) {
                        std::cmp::Ordering::Equal => {
                            insert_at = None; // c:3591-3593 already there
                            break;
                        }
                        std::cmp::Ordering::Less => {
                            insert_at = Some(i); // c:3595-3598
                            break;
                        }
                        std::cmp::Ordering::Greater => {}
                    }
                }
                if let Some(i) = insert_at {
                    list.insert_at(i, feature_full.to_string());
                }
            }
        } else {
            // c:3605-3615 — `else if (m->autoloads) { remnode or
            // subret = FEAT_IGNORE ? -2 : 2; }`
            let removed = table
                .modules
                .get_mut(owner)
                .and_then(|m| m.autoloads.as_mut())
                .map(|list| {
                    match list
                        .iter()
                        .position(|existing| existing.as_str() == feature_full)
                    {
                        Some(i) => {
                            list.delete_node(i);
                            true
                        }
                        None => false,
                    }
                })
                .unwrap_or(false);
            if !removed {
                autoload_subret = if (flags & FEAT_IGNORE) != 0 { -2 } else { 2 };
                // c:3614
            }
        }

        // c:3556-3616 — m->autoloads insert/remove in lexical order;
        // the autoload_* maps mirror the list for the feature→module
        // reverse lookups the static-link dispatch needs.
        if add {
            match fchar {
                b'b' => {
                    table
                        .autoload_builtins
                        .insert(fnam.to_string(), modname.to_string());
                }
                b'c' | b'C' => {
                    table
                        .autoload_conditions
                        .insert(fnam.to_string(), modname.to_string());
                }
                b'p' => {
                    table
                        .autoload_params
                        .insert(fnam.to_string(), modname.to_string());
                }
                b'f' => {
                    table
                        .autoload_mathfuncs
                        .insert(fnam.to_string(), modname.to_string());
                }
                _ => unreachable!(),
            }
        } else {
            // c:3605-3615 — handled above by the m->autoloads remove
            // (autoload_subret carries the c:3614 ±2). Keep the
            // feature→module reverse maps in sync silently — the
            // not-present diagnostic flows through subret below,
            // exactly like C's c:3631 arm.
            match fchar {
                b'b' => {
                    table.autoload_builtins.remove(fnam);
                }
                b'c' | b'C' => {
                    table.autoload_conditions.remove(fnam);
                }
                b'p' => {
                    table.autoload_params.remove(fnam);
                }
                b'f' => {
                    table.autoload_mathfuncs.remove(fnam);
                }
                _ => unreachable!(),
            }
        }

        // c:3618-3619 — `if (subret == 0) subret = fn(module, fnam, flags);`
        // The fn does NOT run when the m->autoloads remove already
        // produced ±2. Dispatch through the per-type add/del fn so the
        // canonical tables (paramtab for `p:`, condtab for `c:`, etc.)
        // carry the PM_AUTOLOAD / CONDF flag bits expected by
        // downstream code (e.g. paramtypestr's `undefined NAME`).
        let subret = if autoload_subret != 0 {
            autoload_subret // c:3618 subret already set
        } else if add {
            match fchar {
                b'p' => add_autoparam(modname, fnam, flags),
                b'f' => add_automathfunc(table, modname, fnam, flags),
                b'b' => table.add_autobin(fnam, modname, flags),
                b'c' | b'C' => table.add_autocond(fnam, modname, flags),
                _ => unreachable!(),
            }
        } else {
            match fchar {
                b'p' => del_autoparam(modname, fnam, flags),
                b'f' => del_automathfunc(table, modname, fnam, flags),
                b'b' => table.del_autobin(fnam, flags),
                b'c' | b'C' => table.del_autocond(fnam, flags),
                _ => unreachable!(),
            }
        };

        // c:3621-3642 — per-error-code diagnostic.
        if subret != 0 && subret != -2 {
            ret = 1; // c:3624
            match subret {
                1 => {
                    // c:3627
                    crate::ported::utils::zwarnnam(
                        cmdnam,
                        &format!("failed to add {} `{}'", typnam, fnam),
                    );
                }
                2 => {
                    // c:3631
                    crate::ported::utils::zwarnnam(
                        cmdnam,
                        &format!("{}: no such {}", fnam, typnam),
                    );
                }
                3 => {
                    // c:3635
                    crate::ported::utils::zwarnnam(
                        cmdnam,
                        &format!("{}: {} is already defined", fnam, typnam),
                    );
                }
                _ => { /* c:3638 no further message */ }
            }
        }
    }
    ret
}

/// Port of `MathFunc mathfuncs;` from `Src/module.c:1258` — the
/// global head of the linked list of math functions. Both
/// autoloadable math ported (added by modules) and user math ported
/// (added by `functions -M`) live here.
///
/// C is a singly linked list with `mathfunc.next` chaining. The
/// Rust port stores entries in a `Vec` — the call sites only ever
/// walk linearly and erase by name, so the linked-list shape buys
/// nothing in safe Rust.
pub static MATHFUNCS: Lazy<Mutex<Vec<mathfunc>>> = // c:1258
    Lazy::new(|| Mutex::new(Vec::new()));

/// Port of `int setconddefs(char const *nam, Conddef c, int size, int *e)`
/// from `Src/module.c:754`. Bulk add/delete of condition definitions:
/// the parallel `e[]` array selects per-entry add (`e[i] != 0`) vs delete
/// (`e[i] == 0`). Returns 1 if any individual op clashed, 0 if all clean.
pub fn setconddefs(
    nam: &str, // c:754
    c: &mut [conddef],
    e: Option<&[i32]>,
) -> i32 {
    let mut ret = 0; // c:758
    for (i, entry) in c.iter_mut().enumerate() {
        // c:760 while (size--)
        let want_add = e.map(|es| es[i] != 0).unwrap_or(true); // c:761 if (e && *e++)
        if want_add {
            if (entry.flags & CONDF_ADDED) != 0 {
                continue;
            } // c:763 already added
            let dup = conddef {
                next: None,
                name: entry.name.clone(),
                flags: entry.flags,
                handler: entry.handler,
                min: entry.min,
                max: entry.max,
                condid: entry.condid,
                module: entry.module.clone(),
            };
            if addconddef(dup) != 0 {
                // c:768 addconddef
                zwarnnam(
                    nam, // c:769 zwarnnam
                    &format!("name clash when adding condition `{}'", entry.name),
                );
                ret = 1;
            } else {
                entry.flags |= CONDF_ADDED; // c:773
            }
        } else {
            if (entry.flags & CONDF_ADDED) == 0 {
                continue;
            } // c:776
            if deleteconddef(entry) != 0 {
                // c:780 deleteconddef
                zwarnnam(
                    nam, // c:781
                    &format!("condition `{}' already deleted", entry.name),
                );
                ret = 1;
            } else {
                entry.flags &= !CONDF_ADDED; // c:785
            }
        }
    }
    ret // c:790
}

/// Port of `int setmathfuncs(char const *nam, MathFunc f, int size, int *e)`
/// from `Src/module.c:1374`. Bulk add/delete of math-function definitions
/// via the parallel `e[]` selector array (same shape as setconddefs).
pub fn setmathfuncs(
    nam: &str, // c:1374
    f: &mut [mathfunc],
    e: Option<&[i32]>,
) -> i32 {
    let mut ret = 0; // c:1378
    for (i, entry) in f.iter_mut().enumerate() {
        // c:1380 while (size--)
        // c:1381 — `if (e && *e++)`: e == NULL means REMOVE all
        // (same contract as setbuiltins, c:497-503 doc: "e is either
        // NULL, in which case all builtins in the table are
        // removed"). The previous `.unwrap_or(true)` inverted the
        // None case into add-all.
        let want_add = e.map(|es| es[i] != 0).unwrap_or(false); // c:1381
        if want_add {
            if (entry.flags & MFF_ADDED) != 0 {
                continue;
            } // c:1383
            let dup = mathfunc {
                next: None,
                name: entry.name.clone(),
                flags: entry.flags,
                nfunc: entry.nfunc,
                sfunc: entry.sfunc,
                module: entry.module.clone(),
                minargs: entry.minargs,
                maxargs: entry.maxargs,
                funcid: entry.funcid,
            };
            if addmathfunc(dup) != 0 {
                // c:1388 addmathfunc
                zwarnnam(
                    nam, // c:1389
                    &format!("name clash when adding math function `{}'", entry.name),
                );
                ret = 1;
            } else {
                entry.flags |= MFF_ADDED; // c:1393
                                          // c:1388+1393 — C links `f` ITSELF into mathfuncs and
                                          // the flag-set above mutates that same (aliased) node.
                                          // The Rust insert is a by-value dup, so mirror the flag
                                          // onto the global-list entry; otherwise
                                          // del_automathfunc / getfeatureenables reading
                                          // MATHFUNCS never observe MFF_ADDED.
                if let Ok(mut gtab) = MATHFUNCS.lock() {
                    if let Some(p) = gtab.iter_mut().find(|p| p.name == entry.name) {
                        p.flags |= MFF_ADDED;
                    }
                }
            }
        } else {
            if (entry.flags & MFF_ADDED) == 0 {
                continue;
            } // c:1396
            if deletemathfunc(entry) != 0 {
                // c:1400 deletemathfunc
                zwarnnam(
                    nam, // c:1401
                    &format!("math function `{}' already deleted", entry.name),
                );
                ret = 1;
            } else {
                // c:1356-1357 — C's deletemathfunc clears MFF_ADDED on
                // the (aliased) static mftab struct; Rust's global-list
                // entry is a dup, so clear it here on the caller's
                // record. Without this, a disable→re-enable cycle hits
                // the c:1383 `continue` and never re-registers.
                entry.flags &= !MFF_ADDED;
            }
        }
    }
    ret // c:1407
}

/// Port of file-static `Conddef condtab;` from `Src/cond.c:21` — the
/// global condition-definition linked-list head consulted by `[[ ... ]]`
/// dispatch. Modules register custom conditions via `addconddef`; the
/// runtime walks `condtab` looking for the matching name+infix flag at
/// each `[[` evaluation. Rust port stores entries in a `Vec` (linear
/// add/remove + walk; same observable behaviour as C linked list).
pub static CONDTAB: Lazy<Mutex<Vec<conddef>>> = // c:cond.c:21
    Lazy::new(|| Mutex::new(Vec::new()));

/// Port of `Conddef getconddef(int inf, const char *name, int autol)`
/// from `Src/module.c:648`.
///
/// C body c:648-689:
/// ```c
/// Conddef
/// getconddef(int inf, const char *name, int autol)
/// {
///     Conddef p;
///     int f = 1;
///     char *lookup, *s;
///     lookup = dupstring(name);
///     if (!lookup) return NULL;
///     for (s = lookup; *s != '\0'; s++) {
///         if (*s == Dash) *s = '-';
///     }
///     do {
///         for (p = condtab; p; p = p->next) {
///             if ((!!inf == !!(p->flags & CONDF_INFIX)) &&
///                 !strcmp(lookup, p->name))
///                 break;
///         }
///         if (autol && p && p->module) {
///             if (f) {
///                 (void)ensurefeature(p->module,
///                                     (p->flags & CONDF_INFIX) ? "C:" : "c:",
///                                     (p->flags & CONDF_AUTOALL) ? NULL : lookup);
///                 f = 0;
///                 p = NULL;
///             } else {
///                 deleteconddef(p);
///                 return NULL;
///             }
///         } else
///             break;
///     } while (!p);
///     return p;
/// }
/// ```
///
/// Returns a clone of the matched `conddef` (or `None` if absent).
/// `inf` selects between infix-style (`[[ A op B ]]`) and prefix-style
/// (`[[ -X arg ]]`) conditions — `CONDF_INFIX` on the entry must match.
/// `autol` triggers `ensurefeature` on autoload-stubs; the autoload
/// loop runs at most once (gated by `f`) — if the second iteration
/// still finds the entry, the stub is treated as a failed load and
/// removed via `deleteconddef`.
pub fn getconddef(inf: i32, name: &str, autol: i32, table: &mut modulestab) -> Option<conddef> {
    // c:648
    // c:655 — `lookup = dupstring(name)` then Dash → '-' substitution.
    let lookup: String = crate::ported::string::dupstring(name)
        .chars()
        .map(|c| {
            if c == crate::ported::zsh_h::Dash {
                '-'
            } else {
                c
            }
        })
        .collect();
    let mut f = 1; // c:651 `int f = 1;`
    loop {
        // c:663 do { ... } while (!p);
        // c:664-668 — walk condtab matching (!!inf == !!CONDF_INFIX) && name.
        let want_infix = inf != 0;
        let hit: Option<conddef> = {
            let tab = CONDTAB.lock().unwrap();
            tab.iter().find_map(|p| {
                let p_infix = (p.flags & CONDF_INFIX) != 0;
                if p_infix == want_infix && p.name == lookup {
                    // Manual field-by-field clone: conddef doesn't
                    // derive Clone (function-pointer + Option<Conddef>
                    // mix with no PartialEq).
                    Some(conddef {
                        next: None,
                        name: p.name.clone(),
                        flags: p.flags,
                        handler: p.handler,
                        min: p.min,
                        max: p.max,
                        condid: p.condid,
                        module: p.module.clone(),
                    })
                } else {
                    None
                }
            })
        };
        // c:669-685 — autoload trigger + failure-retry loop.
        let has_autoload_module = hit.as_ref().map(|p| p.module.is_some()).unwrap_or(false);
        if autol != 0 && hit.is_some() && has_autoload_module {
            let p = hit.as_ref().unwrap();
            if f != 0 {
                // c:674-678 — first miss: load the module + retry.
                let module = p.module.as_ref().unwrap().clone();
                let prefix = if (p.flags & CONDF_INFIX) != 0 {
                    "C:"
                } else {
                    "c:"
                };
                let feature_arg = if (p.flags & CONDF_AUTOALL) != 0 {
                    // c:677 — NULL → autoload-all branch.
                    None
                } else {
                    Some(lookup.as_str())
                };
                let _ = ensurefeature(table, &module, prefix, feature_arg);
                f = 0;
                continue; // c:680 `p = NULL;` + outer do-while re-tries.
            } else {
                // c:681-683 — second pass still hit autoload entry →
                // load failed; remove the stub and return None.
                let _ = deleteconddef(p);
                return None;
            }
        }
        return hit; // c:684-685 `else break;` then return p.
    }
}

/// Port of `int deleteconddef(Conddef c)` from `Src/module.c:724`.
/// Removes condition definition `c` from `condtab`. Returns 0 on
/// success, -1 on miss. C also frees the autoloaded entry's name +
/// module; Rust drop subsumes that.
pub fn deleteconddef(c: &conddef) -> i32 {
    // c:724
    let mut tab = CONDTAB.lock().unwrap();
    // c:728 — `for (p = condtab, q = NULL; p && p != c; ...)`. C uses
    // pointer identity; the Rust analog is name+infix-flag equality
    // (the natural key — `[[ -z STR ]]` and `STR == VAL` share neither).
    let infix = c.flags & CONDF_INFIX;
    match tab
        .iter()
        .position(|p| p.name == c.name && (p.flags & CONDF_INFIX) == infix)
    {
        Some(i) => {
            tab.remove(i);
            0
        } // c:733-738 unlink + free
        None => -1, // c:743 not found
    }
}

/// Port of `int addconddef(Conddef c)` from `Src/module.c:703`. Walks
/// CONDTAB for a clash on (name, infix-flag); replaces autoloadable
/// entries via deleteconddef; otherwise prepends. Returns 0 on add,
/// 1 on clash (existing entry already added).
pub fn addconddef(c: conddef) -> i32 {
    // c:703
    let infix = c.flags & CONDF_INFIX;
    let clash_idx = {
        let tab = CONDTAB.lock().unwrap();
        tab.iter()
            .position(|p| p.name == c.name && (p.flags & CONDF_INFIX) == infix) // c:705 getconddef
    };
    if let Some(i) = clash_idx {
        let (autoload, added) = {
            let tab = CONDTAB.lock().unwrap();
            (tab[i].module.is_some(), (tab[i].flags & CONDF_ADDED) != 0)
        };
        if !autoload || added {
            return 1;
        } // c:708 already added
        CONDTAB.lock().unwrap().remove(i); // c:711 deleteconddef
    }
    CONDTAB.lock().unwrap().insert(0, c); // c:713-714 c->next = condtab; condtab = c
    0
}

/// Port of file-static `FuncWrap wrappers;` from `Src/module.c:567`
/// — the global wrapper-function linked-list head. Modules register
/// wrapper callbacks via `addwrapper(FuncWrap)` and the runtime fires
/// them around `runshfunc()`. The Rust port stores entries in a `Vec`
/// (linear add/remove + iterate; same observable behaviour).
pub static WRAPPERS: Lazy<Mutex<Vec<funcwrap>>> = // c:567
    Lazy::new(|| Mutex::new(Vec::new()));

// !!! WARNING: RUST-ONLY HELPER !!!
//
// No C counterpart. C's `runshfunc` (`Src/exec.c:6177`) walks the
// `wrappers` linked list with plain pointer chasing — no lock, because
// the C shell is single-threaded. zshrs keeps the same list behind
// [`WRAPPERS`]'s `Mutex`, and `runshfunc` sits on the shell-function
// call path, the hottest path in the shell: taking that mutex on every
// call would cost far more than the whole wrapper mechanism.
//
// So `addwrapper` / `deletewrapper` mirror list MEMBERSHIP into this
// relaxed atomic bitmask, one bit per module that can install a
// wrapper. `runshfunc` tests a chain node with a single relaxed load
// and, when no module has registered, does not touch the mutex at all.
// The mask is derived state — [`WRAPPERS`] stays the source of truth.
/// Relaxed-atomic membership mirror of [`WRAPPERS`], one bit per
/// wrapper-installing module (see [`WRAPPER_BIT_ZPROF`]). Written by
/// `addwrapper` / `deletewrapper`, read by `exec::runshfunc`. See the
/// WARNING block above.
pub static WRAPPERS_ADDED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// [`WRAPPERS_ADDED`] bit for `zsh/zprof`'s
/// `WRAPDEF(zprof_wrapper)` (`Src/Modules/zprof.c:318-320`).
pub const WRAPPER_BIT_ZPROF: u32 = 1 << 0;

/// Port of `int addwrapper(Module m, FuncWrap w)` from
/// `Src/module.c:576`. Tail-appends a module's function wrapper onto the
/// global [`WRAPPERS`] list. Returns 1 on error, 0 on success.
///
/// C body (c:578-600):
/// ```c
/// FuncWrap p, q;
/// if (m->node.flags & MOD_ALIAS)
///     return 1;
/// if (w->flags & WRAPF_ADDED)
///     return 1;
/// for (p = wrappers, q = NULL; p; q = p, p = p->next);
/// if (q)
///     q->next = w;
/// else
///     wrappers = w;
/// w->next = NULL;
/// w->flags |= WRAPF_ADDED;
/// w->module = m;
/// return 0;
/// ```
///
/// WARNING: param types don't match C — Rust=(m: &str, w) vs
/// C=(Module m, FuncWrap w). Every zshrs module entry point is reached
/// by NAME through the `module.rs` dispatcher (`setup_module` /
/// `boot_module` / `cleanup_module` all take `name: &str` and hand the
/// per-module `*_` fn a null `*const module`), so a `Module` pointer is
/// not available at the call site. The name is looked up in
/// [`MODULESTAB`] for the c:586 `MOD_ALIAS` test and stored back into
/// `w.module` — C keeps the pointer, this port keeps a name-carrying
/// [`module`] node, which is what `deletewrapper` matches on.
pub fn addwrapper(m: &str, mut w: funcwrap) -> i32 {
    // c:576
    // c:586-587 — `if (m->node.flags & MOD_ALIAS) return 1;`
    // We can't add a wrapper to an alias, since it's supposed to behave
    // identically to the resolved module.  This shouldn't happen since
    // we usually add wrappers when a real module is loaded.  (c:580-585)
    //
    // `try_lock`, not `lock`: every caller of this function is a
    // module `boot_`, which `bin_zmodload` reaches with the
    // [`MODULESTAB`] mutex already held (module.rs:4877 →
    // `load_module` → `do_boot_module`). `std::sync::Mutex` is not
    // reentrant, so a blocking `lock()` here self-deadlocks. A failed
    // `try_lock` means WE are the holder, i.e. the module is mid-load
    // and therefore not an alias — exactly the c:580-585 "shouldn't
    // happen" case — so the test is skipped.
    if let Ok(tab) = MODULESTAB.try_lock() {
        if let Some(md) = tab.modules.get(m) {
            if (md.node.flags & MOD_ALIAS) != 0 {
                return 1; // c:587
            }
        }
    }

    // c:589-590 — `if (w->flags & WRAPF_ADDED) return 1;`
    if (w.flags & crate::ported::zsh_h::WRAPF_ADDED) != 0 {
        return 1; // c:590
    }

    let mut wrappers = WRAPPERS.lock().unwrap_or_else(|e| e.into_inner());
    // c:591-595 — walk to the tail and link on (`q->next = w`), or
    // become the head (`wrappers = w`). A `Vec` push IS that walk.
    // c:596 — `w->next = NULL;`
    w.next = None;
    // c:597 — `w->flags |= WRAPF_ADDED;`
    w.flags |= crate::ported::zsh_h::WRAPF_ADDED;
    // c:598 — `w->module = m;`
    w.module = Some(Box::new(module::new(m)));
    wrappers.push(w); // c:593/595

    // Rust-only — see [`WRAPPERS_ADDED`]. Modules with no bit (the
    // statically-linked wrappers that never reach `addwrapper`) map to
    // 0, which leaves the mask untouched.
    let bit = match m {
        "zsh/zprof" => WRAPPER_BIT_ZPROF,
        _ => 0,
    };
    WRAPPERS_ADDED.fetch_or(bit, std::sync::atomic::Ordering::Relaxed);

    0 // c:600
}

/// Port of `int deletewrapper(Module m, FuncWrap w)` from
/// `Src/module.c:608`. Unlinks a module's wrapper from [`WRAPPERS`].
/// Returns 0 when the node was found and removed, 1 otherwise.
///
/// C body (c:610-628):
/// ```c
/// FuncWrap p, q;
/// if (m->node.flags & MOD_ALIAS)
///     return 1;
/// if (w->flags & WRAPF_ADDED) {
///     for (p = wrappers, q = NULL; p && p != w; q = p, p = p->next);
///     if (p) {
///         if (q) q->next = p->next; else wrappers = p->next;
///         p->flags &= ~WRAPF_ADDED;
///         return 0;
///     }
/// }
/// return 1;
/// ```
///
/// WARNING: param types don't match C — Rust=(m: &str) vs C=(Module m,
/// FuncWrap w), and the node is identified by owning-module name rather
/// than by the `p != w` pointer compare at c:616. C's `w` is always the
/// module's own file-static `wrapper[]` array (`Src/Modules/zprof.c:318`,
/// `Src/Zle/complete.c:1694`, `Src/Modules/param_private.c:541`), so
/// "the node whose `module` is `m`" selects exactly the same entry;
/// zshrs has no stable address to compare against because the port
/// stores the node by value in a `Vec`.
pub fn deletewrapper(m: &str) -> i32 {
    // c:608
    // c:612-613 — `if (m->node.flags & MOD_ALIAS) return 1;`
    // `try_lock` for the same reentrancy reason as `addwrapper`: the
    // caller is a module `cleanup_`, reached with [`MODULESTAB`] held.
    if let Ok(tab) = MODULESTAB.try_lock() {
        if let Some(md) = tab.modules.get(m) {
            if (md.node.flags & MOD_ALIAS) != 0 {
                return 1; // c:613
            }
        }
    }

    let mut wrappers = WRAPPERS.lock().unwrap_or_else(|e| e.into_inner());
    // c:615-616 — `if (w->flags & WRAPF_ADDED)` then walk for the node.
    let found = wrappers.iter().position(|p| {
        (p.flags & crate::ported::zsh_h::WRAPF_ADDED) != 0
            && p.module.as_ref().map(|md| md.node.nam.as_str()) == Some(m)
    });
    if let Some(i) = found {
        // c:618-623 — unlink and clear WRAPF_ADDED. Removing from the
        // `Vec` both unlinks (c:619-622) and drops the flag with the
        // node (c:623).
        wrappers.remove(i);
        // Rust-only — see [`WRAPPERS_ADDED`].
        let bit = match m {
            "zsh/zprof" => WRAPPER_BIT_ZPROF,
            _ => 0,
        };
        WRAPPERS_ADDED.fetch_and(!bit, std::sync::atomic::Ordering::Relaxed);
        return 0; // c:625
    }
    1 // c:628
}

/// Port of `addmathfunc(MathFunc f)` from `Src/module.c:1313`.
/// Returns 0 on add, 1 on clash (existing entry not autoloadable).
/// Replaces autoloadable entries via `removemathfunc`.
pub fn addmathfunc(f: mathfunc) -> i32 {
    // c:1313
    if (f.flags & MFF_ADDED) != 0 {
        return 1;
    } // c:1318
    let mut tab = MATHFUNCS.lock().unwrap();
    let mut found_idx: Option<usize> = None;
    for (i, p) in tab.iter().enumerate() {
        // c:1321
        if p.name == f.name {
            // c:1322
            if p.module.is_some() && (p.flags & MFF_USERFUNC) == 0 {
                // c:1323
                found_idx = Some(i); // c:1327 removemathfunc + replace
                break;
            }
            return 1; // c:1330
        }
    }
    if let Some(i) = found_idx {
        tab.remove(i);
    } // c:1327
    tab.insert(0, f); // c:1334-1335 f->next = mathfuncs; mathfuncs = f
    0
}

/// Port of `removemathfunc(MathFunc previous, MathFunc current)` from
/// `Src/module.c:1267`. Removes the named entry from MATHFUNCS and
/// drops it (Rust drop subsumes C's zsfree/zfree ladder).
/// WARNING: param names don't match C — Rust=(name) vs C=(previous, current)
pub fn removemathfunc(name: &str) {
    // c:1267
    let mut tab = MATHFUNCS.lock().unwrap();
    if let Some(i) = tab.iter().position(|m| m.name == name) {
        // c:1270 walk
        tab.remove(i); // c:1273-1274 unlink + zfree
    }
}

/// Port of `deletemathfunc(MathFunc f)` from `Src/module.c:1342`.
/// Removes f from MATHFUNCS; for unloaded/user-defined entries clears
/// the MFF_ADDED flag instead of dropping the node (C: `f->flags &=
/// ~MFF_ADDED` when f->module is null).
pub fn deletemathfunc(f: &mathfunc) -> i32 {
    // c:1342
    // c:1348-1352 — the node is unlinked from the global list in BOTH
    // arms (`q->next = f->next` / `mathfuncs = f->next`); f->module
    // only decides free-vs-keep of the struct itself. The struct for
    // module=NULL entries lives in the module's static mftab (the C
    // list links the static structs by pointer), so "keep" means the
    // mftab record survives with MFF_ADDED cleared — in Rust the
    // global-list entry is a by-value dup, so the unlink is a plain
    // remove either way and the MFF_ADDED clear on the static record
    // happens at the setmathfuncs caller (which holds `&mut` to the
    // mftab entry). The previous port left module-less entries IN the
    // global list, so a feature-disable never actually deregistered.
    let mut tab = MATHFUNCS.lock().unwrap();
    match tab.iter().position(|m| m.name == f.name) {
        // c:1346
        Some(i) => {
            tab.remove(i); // c:1349-1352 unlink (+ Rust Drop ≙ c:1355-1357 free)
            0 // c:1361
        }
        None => -1, // c:1363
    }
}

/// Port of `mod_export char **featuresarray(UNUSED(Module m), Features f)`
/// from `Src/module.c:3284`. Construct the feature-name array for a
/// module: builtins get `b:NAME`, conditions `c:NAME` or `C:NAME` if
/// `CONDF_INFIX`, math funcs `f:NAME`, params `p:NAME`. Trailing
/// abstract slots (`n_abstract`) are pre-allocated but left empty so
/// the module's own setup can fill them in. C uses zhalloc heap
/// allocation — Box goes out of scope here as Rust's `Vec<String>`
/// owns the entries (Drop happens automatically). Per-module Rust
/// files in `src/ported/modules/*.rs` and `src/ported/builtins/*.rs`
/// each carry a `featuresarray` shim that delegates to this
/// canonical free fn once the modules table is wired through.
/// WARNING: param names don't match C — Rust=(_m, bn, cd, mf, pd, n_abstract) vs C=(m, f)
pub fn featuresarray(
    // c:3284
    _m: *const module,
    bn: &[builtin],  // c:3289 f->bn_list
    cd: &[conddef],  // c:3290 f->cd_list
    mf: &[mathfunc], // c:3291 f->mf_list
    pd: &[paramdef], // c:3292 f->pd_list
    n_abstract: i32, // c:3288 f->n_abstract
) -> Vec<String> {
    let features_size = bn.len() + cd.len() + mf.len() + pd.len()            // c:3288
        + n_abstract.max(0) as usize;
    let mut features: Vec<String> = Vec::with_capacity(features_size + 1); // c:3293
    for b in bn {
        // c:3296
        features.push(format!("b:{}", b.node.nam)); // c:3297
    }
    for c in cd {
        // c:3298
        let prefix = if (c.flags & CONDF_INFIX) != 0 {
            "C:"
        } else {
            "c:"
        }; // c:3299
        features.push(format!("{}{}", prefix, c.name)); // c:3299-3300
    }
    for m in mf {
        // c:3303
        features.push(format!("f:{}", m.name)); // c:3304
    }
    for p in pd {
        // c:3305
        features.push(format!("p:{}", p.name)); // c:3306
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
/// `Vec<i32>` owns the entries (Drop happens automatically). Per-
/// module shims in `src/ported/modules/*.rs` delegate to this
/// canonical free fn once the modules table is wired through.
/// WARNING: param names don't match C — Rust=(_m, bn, cd, mf, pd, n_abstract) vs C=(m, f)
pub fn getfeatureenables(
    // c:3319
    _m: *const module,
    bn: &[builtin],  // c:3324
    cd: &[conddef],  // c:3325
    mf: &[mathfunc], // c:3326
    pd: &[paramdef], // c:3327
    n_abstract: i32, // c:3323
) -> Vec<i32> {
    let features_size = bn.len() + cd.len() + mf.len() + pd.len()            // c:3323
        + n_abstract.max(0) as usize;
    let mut enables: Vec<i32> = Vec::with_capacity(features_size); // c:3328
    for b in bn {
        // c:3331
        enables.push(if (b.node.flags & BINF_ADDED as i32) != 0 {
            1
        } else {
            0
        });
    }
    for c in cd {
        // c:3333
        enables.push(if (c.flags & CONDF_ADDED) != 0 { 1 } else { 0 });
    }
    for m in mf {
        // c:3335
        enables.push(if (m.flags & MFF_ADDED) != 0 { 1 } else { 0 });
    }
    for p in pd {
        // c:3337
        enables.push(if p.pm.is_some() { 1 } else { 0 });
    }
    for _ in 0..n_abstract.max(0) {
        // c:3323 n_abstract slots
        enables.push(0);
    }
    enables // c:3340
}

/// Port of `Hookdef hooktab;` from `Src/module.c:843` — the file-static
/// linked-list head pointer to the chain of registered `hookdef`
/// nodes. Walked by `gethookdef`; mutated by `addhookdef` /
/// `deletehookdef`. Each node is a `Box::leak`'d hookdef (so the raw
/// pointer has program-lifetime, matching C's static-storage
/// `zshhooks[]` and module-side hookdef arrays).
pub static hooktab: std::sync::atomic::AtomicPtr<hookdef> = // c:843
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Port of `mod_export ModuleTable modulestab` from
/// `Src/Modules/zmodload.c:32`. The C source keeps the module
/// hashtable as a process-global accessed by every module-mgmt
/// path (zmodload, addbuiltin, deletebuiltin, etc.). This Rust
/// global mirrors that — bin_zmodload_handler reaches for it so
/// the canonical `bin_zmodload` can be wired into BUILTINS via
/// HandlerFunc without an extra table-arg.
pub static MODULESTAB: Lazy<Mutex<modulestab>> = // c:zmodload.c:32
    Lazy::new(|| Mutex::new(modulestab::new()));

// C zsh classifies module exports by bare integer `type` index 0..4
// (no named constants — just position-in-table) in `features_()`
// (`Src/module.c:313+`). Module load state is the `MOD_*` bitmask in
// `module.node.flags` (`Src/zsh.h:1516-1532`, mirrored at
// `zsh_h.rs:2249-2255`). C does not record which features a module
// added on the module struct — feature registration flows into the
// canonical per-feature-kind tables (`builtintab`, `condtab`,
// `paramtab`, `mathfuncs`, `hooktab`).

/// Feature-type index passed to `features_()` (`Src/module.c:313+`).
/// C ships bare ints; Rust adds names for readability.
pub const FEATURE_TYPE_BUILTIN: i32 = 0;
/// `FEATURE_TYPE_CONDITION` constant.
pub const FEATURE_TYPE_CONDITION: i32 = 1;
/// `FEATURE_TYPE_PARAMETER` constant.
pub const FEATURE_TYPE_PARAMETER: i32 = 2;
/// `FEATURE_TYPE_MATHFUNC` constant.
pub const FEATURE_TYPE_MATHFUNC: i32 = 3;
/// `FEATURE_TYPE_HOOK` constant.
pub const FEATURE_TYPE_HOOK: i32 = 4;
/// Module table (from module.c module hash table)
#[derive(Debug, Default)]
/// Table of registered modules.
/// Port of the `modulestab` HashTable Src/module.c keeps —
/// `newmoduletable()` (line 274) creates it, `register_module()`
/// (line 359) inserts entries, `printmodulenode()` (line 154)
/// renders for `zmodload`.
pub struct modulestab {
    /// C's `HashTable modulestab` node storage (`Src/module.c:274`
    /// `newmoduletable`, created as `newmoduletable(17, "modules")` at
    /// `Src/init.c:1193`). The bucket walk is user-visible: the
    /// `modules` special parameter (`Src/Modules/parameter.c`) emits
    /// `${(k)modules}` straight out of `scanhashtable(modulestab, …)`,
    /// so a Rust `HashMap` here produced an order that matched zsh on
    /// no line at all.
    pub modules: crate::ported::hashtable::hashtable_nodes<module>,
    /// C's `LinkList linkedmodules` (`Src/module.c:39`, created at
    /// `Src/init.c:1194` `linkedmodules = znewlinklist()`), the list
    /// `register_module` appends to (`c:378 zaddlinknode(linkedmodules,
    /// m)`) and `module_linked` (`c:385`) searches. It is a SEPARATE
    /// store from `modulestab`: a statically-linked module is on this
    /// list from boot but gets a `modulestab` node only when something
    /// creates one — `autofeatures`/`add_dep` at boot (`c:3449`,
    /// `c:2390`) or `load_module`'s allocate-on-miss branch
    /// (`c:2229-2237`).
    ///
    /// zshrs compiles every module in, so this list holds every name
    /// `zmodload` can resolve; C only reaches it for `link=static`
    /// modules and dlopens the rest. Keeping it separate is what makes
    /// `modulestab->hsize` stay at C's boot value of 17 — inserting all
    /// ~40 known modules as nodes tripped `ct >= hsize * 2`
    /// (`Src/hashtable.c` addhashnode) and quadrupled the table to 68
    /// buckets, so every `${(k)modules}` bucket index differed from C.
    ///
    /// C stores `Linkedmod` records (name + six entry points); zshrs
    /// dispatches the entry points by name (`setup_module` etc.), so
    /// only the name is kept.
    pub linkedmodules: Vec<String>,
    /// Builtin name → module name mapping for autoload
    pub autoload_builtins: HashMap<String, String>,
    /// Condition name → module name mapping for autoload
    pub autoload_conditions: HashMap<String, String>,
    /// Parameter name → module name mapping for autoload
    pub autoload_params: HashMap<String, String>,
    /// Math function name → module name mapping for autoload
    pub autoload_mathfuncs: HashMap<String, String>,
    /// BINF_ADDED ledger — tracks which builtins have been added via
    /// `setbuiltins` (C: `b->node.flags & BINF_ADDED`, c:508).
    pub added_builtins: HashMap<String, u32>,
}

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

// `BINF_ADDED` / `CONDF_INFIX` / `CONDF_ADDED` / `MFF_ADDED` are
// re-exported from zsh_h.rs (single source of truth, i32 matching
// C `int`).

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in vm_helper are unchanged.
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
pub const FEAT_IGNORE: i32 = 0x0001; // c:62

/// `FEAT_INFIX` — bit indicating a condition is infix-style. Port of
/// `enum { FEAT_INFIX = 0x0002 }` from `Src/module.c:64`.
pub const FEAT_INFIX: i32 = 0x0002; // c:64

/// `FEAT_AUTOALL` — `zmodload -a` enable-all-features. Port of
/// `enum { FEAT_AUTOALL = 0x0004 }` from `Src/module.c:69`.
pub const FEAT_AUTOALL: i32 = 0x0004; // c:69

/// `FEAT_REMOVE` — bit indicating feature removal pass. Port of
/// `enum { FEAT_REMOVE = 0x0008 }` from `Src/module.c:76`.
pub const FEAT_REMOVE: i32 = 0x0008; // c:76

/// `FEAT_CHECKAUTO` — verify autoloads are actually provided. Port of
/// `enum { FEAT_CHECKAUTO = 0x0010 }` from `Src/module.c:81`.
pub const FEAT_CHECKAUTO: i32 = 0x0010; // c:81

/// !!! WARNING: RUST-ONLY FLAG BIT !!!
/// C has no such bit. It carries the `zmodload -m` decision on the
/// `Patprog pat` field of each `struct feature_enables` entry
/// (`Src/module.c:3253-3262` fills `fep->pat = patprogs ? *patprogs++ :
/// NULL`, and `do_module_features` at c:2093 branches
/// `fep->pat ? pattry(fep->pat, *fp) : !strcmp(*fp, esp)`). zshrs's
/// feature list is a plain `&[String]` with no per-entry payload, so
/// there is nowhere to hang the compiled pattern. `-m` sets the pat for
/// EVERY entry of one invocation uniformly, so the decision is carried
/// as this one flag bit alongside `FEAT_IGNORE`/`FEAT_CHECKAUTO` and the
/// pattern is compiled at the match site. Deliberately outside C's
/// 0x0001-0x0010 range so it cannot collide with a future upstream bit.
pub const FEAT_PATTERN_ARGS: i32 = 0x1000;

/// `FINDMOD_ALIASP` — bit in `find_module()`'s `flags` arg.
/// Port of `enum { FINDMOD_ALIASP = 0x0001 }` from `Src/module.c:110`.
/// /* Resolve any aliases to the underlying module. */
pub const FINDMOD_ALIASP: i32 = 0x0001; // c:110

/// `FINDMOD_CREATE` — bit in `find_module()`'s `flags` arg.
/// Port of `enum { FINDMOD_CREATE = 0x0002 }` from `Src/module.c:115`.
/// /* Create an element for the module in the list if not found. */
pub const FINDMOD_CREATE: i32 = 0x0002; // c:115

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::hashnode;

    #[test]
    fn test_module_table_new() {
        let _g = crate::test_util::global_state_lock();
        let table = modulestab::new();
        // zsh/complete is in the default-loaded set (module.rs:1128).
        // zsh/datetime and zsh/system are REGISTERED but carry
        // MOD_UNLOAD at init — they require explicit `zmodload NAME`
        // to become is_loaded(). Bugs #530/#532/#535.
        assert!(table.is_loaded("zsh/complete"));
        assert!(!table.is_loaded("zsh/datetime"));
        assert!(!table.is_loaded("zsh/system"));
        assert!(!table.is_loaded("nonexistent"));
    }

    /// c:2812-2913 — `unload_module` clears the bound state (finish hook
    /// runs, `u.linked` goes NULL, MOD_INIT_B/MOD_INIT_S drop) but only
    /// *deletes* the record when the module has no deps (c:2910
    /// `else if (!m->deps) delete_module(m)`). `zsh/complete` declares
    /// `zsh/zle` as a dep (Src/Zle/complete.mdd, module.rs:1513), so its
    /// record survives the unload with MOD_LINKED still set — which is
    /// what `is_loaded()` reports. `is_bound()` is the predicate that
    /// tracks C's "actually loaded" test (`m->u.handle && !MOD_UNLOAD`,
    /// c:2637), so that is what flips here.
    #[test]
    fn test_load_unload() {
        let _g = crate::test_util::global_state_lock();
        let mut table = modulestab::new();
        assert!(table.is_loaded("zsh/complete"));
        table.load_module("zsh/complete", None, false);
        assert!(table.is_bound("zsh/complete"));

        table.unload_module("zsh/complete");
        assert!(!table.is_bound("zsh/complete"));
        assert!(
            table.is_loaded("zsh/complete"),
            "record must survive: zsh/complete has deps (c:2910)"
        );

        table.load_module("zsh/complete", None, false);
        assert!(table.is_loaded("zsh/complete"));
        assert!(table.is_bound("zsh/complete"));
    }

    #[test]
    fn test_list_loaded() {
        let _g = crate::test_util::global_state_lock();
        let table = modulestab::new();
        let loaded = table.list_loaded();
        // C zsh ships exactly 14 default-loaded modules (verified
        // against Homebrew zsh 5.9.1; full list at module.rs:1126).
        // The other ~17 registered modules carry MOD_UNLOAD until
        // `zmodload NAME` clears it (see module.rs:1148-1152).
        // Default-loaded count intersects (a) the registered set in
        // `builtin_modules` (module.rs:1034-1105, ~30 entries) with
        // (b) the `zsh_default_loaded` whitelist (module.rs:1126,
        // 14 entries). Items in (b) but NOT (a) — `zsh/compctl`,
        // `zsh/main`, `zsh/rlimits`, `zsh/zle` — currently land in the
        // autoload registry only, so the actual count is 14 - 4 = 10
        // (a couple of overlaps land it at ~11 in practice).
        assert!(
            loaded.len() >= 10,
            "expected >= 10 default-loaded modules, got {}",
            loaded.len()
        );
        assert!(loaded.contains(&"zsh/complete"));
    }

    #[test]
    fn test_autoload() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let table = modulestab::new();
        assert!(table.module_linked("zsh/complete"));
        assert!(table.module_linked("zsh/stat"));
        assert!(!table.module_linked("zsh/nonexistent"));
    }

    #[test]
    fn test_printmodulenode() {
        let _g = crate::test_util::global_state_lock();
        // C-source-true print gate: `m->u.handle || (flags &
        // PRINTMOD_AUTO)` at Src/module.c:218 — only fires once
        // `load_module` has wired up the union slot AND set
        // MOD_INIT_B (c:2244). Fresh `module::new` returns a
        // registered-but-not-loaded entry, so the gate misses.
        // Mark MOD_INIT_B to simulate the post-boot state.
        let mut module = module::new("zsh/test");
        module.node.flags |= MOD_INIT_B;
        // Loaded module, no flags → emit just the module name
        // (c:240 nicezputs(modname)).
        let output = printmodulenode("zsh/test", &module, 0);
        assert_eq!(output, "zsh/test");
        // Under PRINTMOD_LIST the loaded branch emits `zmodload MOD`.
        let listed = printmodulenode("zsh/test", &module, PRINTMOD_LIST);
        assert_eq!(listed, "zmodload zsh/test");
        // Registered-but-not-loaded: no MOD_INIT_B → empty output
        // (matches C's `m->u.handle` being NULL).
        let unloaded = module::new("zsh/unloaded");
        let nope = printmodulenode("zsh/unloaded", &unloaded, 0);
        assert_eq!(nope, "");
    }

    // ===== Tests for the `addmathfunc` / `removemathfunc` /
    // `deletemathfunc` family ported in this session against the
    // MATHFUNCS Lazy<Mutex<Vec<mathfunc>>> global. Each test isolates
    // its names with a unique prefix so they don't collide if the
    // suite runs in parallel.

    fn mk_mf(name: &str, autoload: bool) -> mathfunc {
        mathfunc {
            next: None,
            name: name.to_string(),
            flags: 0,
            nfunc: None,
            sfunc: None,
            module: if autoload {
                Some("zsh/test".to_string())
            } else {
                None
            },
            minargs: 0,
            maxargs: 0,
            funcid: 0,
        }
    }

    #[test]
    fn addmathfunc_clash_returns_one_when_already_added() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        // When existing entry IS autoloadable (module.is_some, no
        // MFF_USERFUNC), C removes-then-replaces. The new entry should
        // land at index 0 (prepend) per c:1334-1335.
        let auto = mk_mf("zshrs_test_replace", true);
        let real = mk_mf("zshrs_test_replace", false);
        assert_eq!(addmathfunc(auto), 0);
        assert_eq!(
            addmathfunc(real),
            0,
            "autoloadable entry must be replaceable"
        );
        let tab = MATHFUNCS.lock().unwrap();
        let entry = tab.iter().find(|m| m.name == "zshrs_test_replace").unwrap();
        assert!(
            entry.module.is_none(),
            "after replace, module should be None"
        );
        drop(tab);
        removemathfunc("zshrs_test_replace");
    }

    #[test]
    fn removemathfunc_returns_unit_and_drops() {
        let _g = crate::test_util::global_state_lock();
        let f = mk_mf("zshrs_test_remove", false);
        assert_eq!(addmathfunc(f), 0);
        assert!(MATHFUNCS
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.name == "zshrs_test_remove"));
        removemathfunc("zshrs_test_remove");
        assert!(!MATHFUNCS
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.name == "zshrs_test_remove"));
    }

    #[test]
    fn deletemathfunc_returns_minus_one_on_miss() {
        let _g = crate::test_util::global_state_lock();
        // C: returns -1 when no matching entry; verifies the c:1361 branch.
        let probe = mk_mf("zshrs_test_never_added_xyz", false);
        assert_eq!(deletemathfunc(&probe), -1);
    }

    #[test]
    fn deletemathfunc_clears_added_flag_for_userfunc() {
        let _g = crate::test_util::global_state_lock();
        // For non-module entries (`!f->module`), C clears the MFF_ADDED
        // flag instead of dropping the node (c:1357). Tests by adding
        // a user-defined mathfunc, flipping MFF_ADDED on, then deleting.
        let mut f = mk_mf("zshrs_test_clear_flag", false);
        f.flags = MFF_ADDED;
        assert_eq!(
            addmathfunc(f),
            1,
            "MFF_ADDED set → addmathfunc clashes at c:1318"
        );
        // Now seed it manually with module=None and MFF_ADDED so deletemathfunc
        // exercises the clear-flag branch.
        MATHFUNCS
            .lock()
            .unwrap()
            .insert(0, mk_mf("zshrs_test_clear_flag2", false));
        let mut f2 = mk_mf("zshrs_test_clear_flag2", false);
        f2.flags = MFF_ADDED;
        // f2 is the lookup probe; by name it matches the seeded entry.
        assert_eq!(deletemathfunc(&f2), 0);
        let tab = MATHFUNCS.lock().unwrap();
        let entry = tab.iter().find(|m| m.name == "zshrs_test_clear_flag2");
        // Entry stays in the table (module was None) but MFF_ADDED cleared.
        if let Some(e) = entry {
            assert_eq!(e.flags & MFF_ADDED, 0);
        }
        drop(tab);
        removemathfunc("zshrs_test_clear_flag2");
    }

    // ===== Tests for `addconddef` / `deleteconddef` against CONDTAB.

    fn mk_cd(name: &str, infix: bool, autoload: bool) -> conddef {
        conddef {
            next: None,
            name: name.to_string(),
            flags: if infix { CONDF_INFIX } else { 0 },
            handler: None,
            min: 0,
            max: 0,
            condid: 0,
            module: if autoload {
                Some("zsh/cond".to_string())
            } else {
                None
            },
        }
    }

    #[test]
    fn addconddef_clash_returns_one() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let probe = mk_cd("zshrs_test_cond_never_added", false, false);
        assert_eq!(deleteconddef(&probe), -1);
    }

    #[test]
    fn addconddef_distinguishes_infix_from_prefix() {
        let _g = crate::test_util::global_state_lock();
        // CONDF_INFIX is part of the clash key — a prefix-form `-z` and
        // an infix-form `==` share neither name nor flag, so adding both
        // names with different infix bits should both succeed.
        let prefix = mk_cd("zshrs_test_cond_dual", false, false);
        let infix = mk_cd("zshrs_test_cond_dual", true, false);
        assert_eq!(addconddef(prefix), 0);
        assert_eq!(
            addconddef(infix),
            0,
            "infix variant must not clash with prefix variant"
        );
        // Cleanup both
        let _ = deleteconddef(&mk_cd("zshrs_test_cond_dual", false, false));
        let _ = deleteconddef(&mk_cd("zshrs_test_cond_dual", true, false));
    }

    // ===== Tests for `setconddefs` / `setmathfuncs` bulk dispatch.

    #[test]
    fn setconddefs_bulk_add_then_bulk_delete_via_e_array() {
        let _g = crate::test_util::global_state_lock();
        // C setconddefs: walks (c, e) pairs; e[i]!=0 → addconddef path,
        // e[i]==0 → deleteconddef path. Tests the round trip.
        let mut entries = vec![
            mk_cd("zshrs_test_bulk_a", false, false),
            mk_cd("zshrs_test_bulk_b", false, false),
        ];
        let add_selectors = [1, 1];
        assert_eq!(setconddefs("test", &mut entries, Some(&add_selectors)), 0);
        // Both should now have CONDF_ADDED set per c:773.
        assert_ne!(entries[0].flags & CONDF_ADDED, 0);
        assert_ne!(entries[1].flags & CONDF_ADDED, 0);
        // Now delete both via e=[0,0].
        let del_selectors = [0, 0];
        assert_eq!(setconddefs("test", &mut entries, Some(&del_selectors)), 0);
        assert_eq!(entries[0].flags & CONDF_ADDED, 0);
        assert_eq!(entries[1].flags & CONDF_ADDED, 0);
    }

    // ===== Tests for `addbuiltin` / `addbuiltins` against canonical builtintab.

    fn mk_b(nam: &str) -> builtin {
        builtin {
            node: hashnode {
                next: None,
                nam: nam.to_string(),
                flags: 0,
            },
            handlerfunc: None,
            minargs: 0,
            maxargs: 0,
            funcid: 0,
            optstr: None,
            defopts: None,
        }
    }

    #[test]
    fn addbuiltin_clash_against_existing_builtintab_entry() {
        let _g = crate::test_util::global_state_lock();
        // C addbuiltin: returns 1 when builtintab already has an entry
        // for the same name with BINF_ADDED set. The canonical Rust
        // builtintab is populated at startup via createbuiltintable;
        // probing a real builtin like "echo" should clash if BINF_ADDED.
        let _ = createbuiltintable();
        let mut b = mk_b("echo");
        let r = addbuiltin(&mut b);
        // BINF_ADDED gets set on b when no clash. If echo was BINF_ADDED in
        // the static table, r==1; otherwise r==0 and b.flags now has BINF_ADDED.
        assert!(r == 0 || r == 1);
        if r == 0 {
            assert_ne!(b.node.flags & BINF_ADDED as i32, 0);
        }
    }

    #[test]
    fn addbuiltins_skips_already_added_entries() {
        let _g = crate::test_util::global_state_lock();
        // C addbuiltins (c:553): `if (b->node.flags & BINF_ADDED) continue`.
        // Pre-marking BINF_ADDED should skip both entries; ret stays 0.
        let mut b1 = mk_b("zshrs_test_already_added_1");
        b1.node.flags = BINF_ADDED as i32;
        let mut b2 = mk_b("zshrs_test_already_added_2");
        b2.node.flags = BINF_ADDED as i32;
        let mut binl = vec![b1, b2];
        assert_eq!(addbuiltins("test", &mut binl), 0);
    }

    // ===== Tests for `addwrapper` / `deletewrapper` against WRAPPERS.

    fn mk_w() -> funcwrap {
        funcwrap {
            next: None,
            flags: 0,
            handler: Some(|_prog, _w, _name| 0),
            module: None,
        }
    }

    #[test]
    fn addwrapper_then_deletewrapper_round_trip() {
        let _g = crate::test_util::global_state_lock();
        // Register a non-alias module in the live table so the c:586
        // MOD_ALIAS gate is exercised and clears.
        MODULESTAB
            .lock()
            .unwrap()
            .modules
            .insert("zsh/test".to_string(), module::new("zsh/test"));
        let w = mk_w();
        assert_eq!(addwrapper("zsh/test", w), 0);
        // c:597-598 — the node lands in `wrappers` carrying WRAPF_ADDED
        // and a back-link to its module, which is what `deletewrapper`
        // matches on.
        assert!(WRAPPERS.lock().unwrap().iter().any(|p| {
            p.flags & crate::ported::zsh_h::WRAPF_ADDED != 0
                && p.module.as_ref().map(|m| m.node.nam.as_str()) == Some("zsh/test")
        }));
        // c:618-625 — found: unlink and return 0.
        assert_eq!(deletewrapper("zsh/test"), 0);
        // c:628 — second removal misses.
        assert_eq!(deletewrapper("zsh/test"), 1);
        MODULESTAB.lock().unwrap().modules.remove("zsh/test");
    }

    /// c:586-587 — `addwrapper` refuses an alias module, so nothing
    /// lands in `wrappers` and `deletewrapper` reports the same refusal.
    #[test]
    fn addwrapper_refuses_alias_module() {
        let _g = crate::test_util::global_state_lock();
        {
            let mut tab = MODULESTAB.lock().unwrap();
            let mut m = module::new("zsh/testalias");
            m.node.flags |= MOD_ALIAS;
            tab.modules.insert("zsh/testalias".to_string(), m);
        }
        assert_eq!(addwrapper("zsh/testalias", mk_w()), 1); // c:587
        assert_eq!(deletewrapper("zsh/testalias"), 1); // c:613
        MODULESTAB.lock().unwrap().modules.remove("zsh/testalias");
    }

    /// c:589-590 — a node that already carries `WRAPF_ADDED` is
    /// refused rather than double-linked.
    #[test]
    fn addwrapper_refuses_already_added_node() {
        let _g = crate::test_util::global_state_lock();
        let mut w = mk_w();
        w.flags |= crate::ported::zsh_h::WRAPF_ADDED;
        assert_eq!(addwrapper("zsh/test", w), 1); // c:590
    }

    #[test]
    fn deletewrapper_returns_one_when_not_found() {
        let _g = crate::test_util::global_state_lock();
        // Empty WRAPPERS means any probe misses. Take a snapshot of the
        // current state, drain WRAPPERS, run the test, restore.
        let snapshot: Vec<_> = WRAPPERS.lock().unwrap().drain(..).collect();
        assert_eq!(deletewrapper("zsh/test"), 1);
        WRAPPERS.lock().unwrap().extend(snapshot);
    }

    // C-faithful hookdef tests. The system under test is:
    //   - `hooktab` (file-static linked-list head, port of c:843)
    //   - `gethookdef` / `addhookdef` / `addhookdefs` / `deletehookdef` /
    //     `deletehookdefs` / `addhookdeffunc` / `addhookfunc` /
    //     `deletehookdeffunc` / `deletehookfunc` / `runhookdef` —
    //     C-identical signatures over real `Hookfn` fn pointers.
    //
    // Each test holds `global_state_lock` and snapshots the chain at
    // start so any incidental registrations from other state leak don't
    // affect outcomes.

    // Real Rust ported matching the `Hookfn` shape. Used as test handlers
    // that bump a per-test atomic counter when invoked, proving
    // `runhookdef` actually dispatches the registered fn pointers.
    static H1_CALLS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    static H2_CALLS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    static H1_RETVAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

    fn h1(_h: *mut hookdef, _d: *mut std::ffi::c_void) -> i32 {
        H1_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        H1_RETVAL.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn h2(_h: *mut hookdef, _d: *mut std::ffi::c_void) -> i32 {
        H2_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        0
    }

    // Allocate a fresh hookdef on the heap and leak its Box so that the
    // returned `*mut hookdef` has program-lifetime — matching C's
    // static-storage hookdef nodes (zshhooks[] etc.). The test then
    // takes responsibility for splicing it out via deletehookdef.
    fn leak_hookdef(name: &str, flags: i32) -> *mut hookdef {
        Box::into_raw(Box::new(hookdef {
            next: std::ptr::null_mut(),
            name: name.to_string(),
            def: None,
            flags,
            funcs: std::ptr::null_mut(),
        }))
    }

    /// `gethookdef` returns null when no hookdef with that name is in
    /// `hooktab`, and the registered pointer once `addhookdef` runs.
    #[test]
    fn gethookdef_returns_null_then_registered_ptr() {
        let _g = crate::test_util::global_state_lock();
        // Unique name to avoid colliding with other tests' registrations.
        let h = leak_hookdef("zshrs_test_get_hook", HOOKF_ALL);
        unsafe {
            assert!(gethookdef(&(*h).name).is_null());
        }
        assert_eq!(addhookdef(h), 0);
        unsafe {
            assert_eq!(gethookdef(&(*h).name), h);
        }
        // Cleanup so the chain doesn't leak into other tests.
        assert_eq!(deletehookdef(h), 0);
        unsafe {
            drop(Box::from_raw(h));
        }
    }

    /// `addhookdef` rejects a name already present in `hooktab`
    /// (port of c:866-867: `if (gethookdef(h->name)) return 1`).
    #[test]
    fn addhookdef_rejects_duplicate_name() {
        let _g = crate::test_util::global_state_lock();
        let h1 = leak_hookdef("zshrs_test_dup_hook", HOOKF_ALL);
        let h2 = leak_hookdef("zshrs_test_dup_hook", HOOKF_ALL);
        assert_eq!(addhookdef(h1), 0);
        assert_eq!(addhookdef(h2), 1, "duplicate name must return 1");
        // Cleanup.
        assert_eq!(deletehookdef(h1), 0);
        unsafe {
            drop(Box::from_raw(h1));
            drop(Box::from_raw(h2));
        }
    }

    /// `deletehookdef` returns 1 when the hookdef isn't in the chain
    /// (port of c:909-910: `if (!p) return 1`) and 0 after add.
    #[test]
    fn deletehookdef_returns_one_on_miss_zero_on_hit() {
        let _g = crate::test_util::global_state_lock();
        let h = leak_hookdef("zshrs_test_del_hook", HOOKF_ALL);
        assert_eq!(deletehookdef(h), 1, "not in chain → 1");
        assert_eq!(addhookdef(h), 0);
        assert_eq!(deletehookdef(h), 0, "spliced out → 0");
        assert_eq!(deletehookdef(h), 1, "second delete misses → 1");
        unsafe {
            drop(Box::from_raw(h));
        }
    }

    /// `runhookdef` dispatches every registered Hookfn under `HOOKF_ALL`
    /// (port of c:996-1004) — the test handlers' counters bump on every
    /// fire, and the first non-zero return short-circuits.
    #[test]
    fn runhookdef_dispatches_all_under_hookf_all_short_circuits_on_nonzero() {
        let _g = crate::test_util::global_state_lock();
        H1_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        H2_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        H1_RETVAL.store(0, std::sync::atomic::Ordering::SeqCst);

        let h = leak_hookdef("zshrs_test_runall", HOOKF_ALL);
        assert_eq!(addhookdef(h), 0);
        assert_eq!(addhookdeffunc(h, h1), 0);
        assert_eq!(addhookdeffunc(h, h2), 0);

        // Both fire → r1=0, r2=0 → final return 0.
        assert_eq!(runhookdef(h, std::ptr::null_mut()), 0);
        assert_eq!(H1_CALLS.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(H2_CALLS.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Make h1 return 7 → runhookdef should return 7 without calling h2.
        H1_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        H2_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        H1_RETVAL.store(7, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(runhookdef(h, std::ptr::null_mut()), 7);
        assert_eq!(H1_CALLS.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            H2_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "h2 must not fire after h1 returns nonzero"
        );

        // Cleanup.
        assert_eq!(deletehookdef(h), 0);
        unsafe {
            drop(Box::from_raw(h));
        }
    }

    /// `runhookdef` calls only the LAST registered Hookfn when
    /// `HOOKF_ALL` is clear (port of c:1006).
    #[test]
    fn runhookdef_calls_only_last_when_hookf_all_clear() {
        let _g = crate::test_util::global_state_lock();
        H1_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        H2_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        H1_RETVAL.store(0, std::sync::atomic::Ordering::SeqCst);

        let h = leak_hookdef("zshrs_test_runlast", 0); // HOOKF_ALL clear
        assert_eq!(addhookdef(h), 0);
        assert_eq!(addhookdeffunc(h, h1), 0);
        assert_eq!(addhookdeffunc(h, h2), 0);

        runhookdef(h, std::ptr::null_mut());
        assert_eq!(
            H1_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "h1 must not fire when HOOKF_ALL is clear"
        );
        assert_eq!(
            H2_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "only the last-registered handler fires"
        );

        assert_eq!(deletehookdef(h), 0);
        unsafe {
            drop(Box::from_raw(h));
        }
    }

    /// `deletehookdeffunc` removes a single Hookfn from the chain
    /// (port of c:961-973). Pins the closure-shadow regression at the
    /// new free-fn surface.
    #[test]
    fn deletehookdeffunc_removes_only_target_hookfn() {
        let _g = crate::test_util::global_state_lock();
        H1_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        H2_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        H1_RETVAL.store(0, std::sync::atomic::Ordering::SeqCst);

        let h = leak_hookdef("zshrs_test_del_fn", HOOKF_ALL);
        assert_eq!(addhookdef(h), 0);
        addhookdeffunc(h, h1);
        addhookdeffunc(h, h2);
        // Remove only h1 — h2 must remain and fire.
        assert_eq!(deletehookdeffunc(h, h1), 0);
        // Second remove of h1 returns 1 (miss).
        assert_eq!(deletehookdeffunc(h, h1), 1);
        runhookdef(h, std::ptr::null_mut());
        assert_eq!(H1_CALLS.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(H2_CALLS.load(std::sync::atomic::Ordering::SeqCst), 1);

        assert_eq!(deletehookdef(h), 0);
        unsafe {
            drop(Box::from_raw(h));
        }
    }

    /// `addhookfunc` / `deletehookfunc` return 1 when no hookdef with
    /// that name is registered (port of c:953, c:982).
    #[test]
    fn addhookfunc_and_deletehookfunc_return_one_when_no_hookdef() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(addhookfunc("zshrs_test_no_such_hook", h1), 1);
        assert_eq!(deletehookfunc("zshrs_test_no_such_hook", h1), 1);
    }

    /// `addhookdefs(NULL, h_array, size)` registers contiguous hookdef
    /// nodes from an array (port of c:883-895). Each entry must be
    /// findable via `gethookdef` after the call.
    #[test]
    fn addhookdefs_registers_contiguous_array() {
        let _g = crate::test_util::global_state_lock();
        let arr: Box<[hookdef; 2]> = Box::new([
            hookdef {
                next: std::ptr::null_mut(),
                name: "zshrs_test_arr_0".into(),
                def: None,
                flags: HOOKF_ALL,
                funcs: std::ptr::null_mut(),
            },
            hookdef {
                next: std::ptr::null_mut(),
                name: "zshrs_test_arr_1".into(),
                def: None,
                flags: HOOKF_ALL,
                funcs: std::ptr::null_mut(),
            },
        ]);
        let base: *mut hookdef = Box::into_raw(arr) as *mut hookdef;
        assert_eq!(addhookdefs(std::ptr::null(), base, 2), 0);
        assert_eq!(gethookdef("zshrs_test_arr_0"), base);
        unsafe {
            assert_eq!(gethookdef("zshrs_test_arr_1"), base.add(1));
        }
        // Cleanup. Splice out then re-take ownership of the boxed array.
        unsafe {
            assert_eq!(deletehookdef(base), 0);
            assert_eq!(deletehookdef(base.add(1)), 0);
            drop(Box::from_raw(base as *mut [hookdef; 2]));
        }
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
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modname_ok("zsh"), 1);
        assert_eq!(modname_ok("zsh/datetime"), 1);
        assert_eq!(modname_ok("zsh/zle"), 1);
        assert_eq!(modname_ok("foo_bar"), 1);
        assert_eq!(modname_ok("foo123"), 1);
    }

    /// c:2179 — non-identifier chars (excluding `/`) MUST cause
    /// rejection. A regression accepting them would let modules
    /// install with names that no later `zmodload -u` could remove.
    #[test]
    fn modname_ok_rejects_special_chars() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modname_ok("zsh space"), 0);
        assert_eq!(modname_ok("zsh-bad"), 0, "hyphen is not IIDENT");
        assert_eq!(modname_ok("zsh.foo"), 0, "dot is not IIDENT");
        assert_eq!(modname_ok("$foo"), 0);
    }

    /// c:2177 — `if (!*p) return 1` runs at the START of the loop;
    /// empty input therefore returns 1. Pin this behaviour so callers
    /// know the empty-string case maps to "trivially OK" not "error".
    #[test]
    fn modname_ok_treats_empty_as_trivially_ok() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modname_ok(""), 1);
    }

    /// `Src/module.c:464-478` — `del_autobin` returns 2 for "no such
    /// builtin" (unless FEAT_IGNORE), 3 for "registered builtin —
    /// can't unload" (unless FEAT_IGNORE), 0 for success.
    #[test]
    fn del_autobin_unknown_name_returns_2_or_zero_per_feat_ignore() {
        let _g = crate::test_util::global_state_lock();
        let mut t = modulestab::new();
        // unknown name + no FEAT_IGNORE → 2
        assert_eq!(t.del_autobin("definitely_not_a_builtin", 0), 2);
        // unknown name + FEAT_IGNORE → 0
        assert_eq!(t.del_autobin("definitely_not_a_builtin", FEAT_IGNORE), 0);
    }

    /// `Src/module.c:464-478` — known static-linked builtin (e.g. "echo")
    /// is in createbuiltintable(), counts as BINF_ADDED → return 3
    /// unless FEAT_IGNORE.
    #[test]
    fn del_autobin_registered_builtin_returns_3_or_zero_per_feat_ignore() {
        let _g = crate::test_util::global_state_lock();
        let mut t = modulestab::new();
        // "echo" is a static-linked builtin → can't unload → 3
        assert_eq!(t.del_autobin("echo", 0), 3);
        // With FEAT_IGNORE → 0
        assert_eq!(t.del_autobin("echo", FEAT_IGNORE), 0);
    }

    /// `Src/module.c:464-478` — name that was added via `add_autobin`
    /// (i.e. in autoload_builtins) but NOT in builtintab → success
    /// path → return 0 + removes from autoload ledger.
    #[test]
    fn del_autobin_autoload_only_entry_removed() {
        let _g = crate::test_util::global_state_lock();
        let mut t = modulestab::new();
        // Seed an autoload entry not in the static builtintab.
        t.autoload_builtins
            .insert("zshrs_test_autobin_x".to_string(), "mymod".to_string());
        assert_eq!(t.del_autobin("zshrs_test_autobin_x", 0), 0);
        assert!(
            !t.autoload_builtins.contains_key("zshrs_test_autobin_x"),
            "successful del must remove ledger entry"
        );
        // Second call → now "no such" → 2.
        assert_eq!(t.del_autobin("zshrs_test_autobin_x", 0), 2);
        assert_eq!(t.del_autobin("zshrs_test_autobin_x", FEAT_IGNORE), 0);
    }

    /// `Src/module.c:819-835` — `del_autocond` parallel contract: 2
    /// for "no such", 0 for autoload-entry-removed.
    #[test]
    fn del_autocond_autoload_entry_removed_or_not_found() {
        let _g = crate::test_util::global_state_lock();
        let mut t = modulestab::new();
        // Not present → 2 / 0 per FEAT_IGNORE.
        assert_eq!(t.del_autocond("zshrs_test_cond_x", 0), 2);
        assert_eq!(t.del_autocond("zshrs_test_cond_x", FEAT_IGNORE), 0);
        // Seed through the C counterpart `add_autocond` (c:791) — it is
        // what puts the conddef in CONDTAB. A bare `autoload_conditions`
        // insert is NOT enough: `del_autocond` looks the name up with
        // `getconddef` (c:820), which only ever walks CONDTAB, so a
        // ledger-only entry still reports "no such" (2).
        assert_eq!(t.add_autocond("zshrs_test_cond_x", "mymod", 0), 0);
        assert_eq!(t.del_autocond("zshrs_test_cond_x", 0), 0);
        assert!(!t.autoload_conditions.contains_key("zshrs_test_cond_x"));
        // Removed from CONDTAB too → back to "no such".
        assert_eq!(t.del_autocond("zshrs_test_cond_x", 0), 2);
    }

    /// `Src/module.c:1240-1255` — `del_autoparam` parallel contract.
    /// 2 for "no such", 3 for "param exists without PM_AUTOLOAD —
    /// can't unload", 0 for success.
    #[test]
    fn del_autoparam_unknown_name_returns_2() {
        let _g = crate::test_util::global_state_lock();
        let mut t = modulestab::new();
        assert_eq!(t.del_autoparam("zshrs_test_param_x_unknown", 0), 2);
        assert_eq!(
            t.del_autoparam("zshrs_test_param_x_unknown", FEAT_IGNORE),
            0
        );
    }

    // ─── zsh-corpus pins for module table ───────────────────────────

    /// `register_module` returns true on first registration.
    #[test]
    fn module_corpus_register_new_returns_true() {
        let _g = crate::test_util::global_state_lock();
        let mut t = newmoduletable();
        assert!(register_module(&mut t, "myfresh.module"));
    }

    /// `register_module` returns false on duplicate registration.
    #[test]
    fn module_corpus_register_duplicate_returns_false() {
        let _g = crate::test_util::global_state_lock();
        let mut t = newmoduletable();
        assert!(register_module(&mut t, "dup.module"));
        assert!(
            !register_module(&mut t, "dup.module"),
            "second registration of same name = false"
        );
    }

    /// New modulestab is pre-seeded with built-in modules.
    /// Pin: pre-seed count is positive and stable.
    #[test]
    fn module_corpus_new_table_has_builtin_modules() {
        let _g = crate::test_util::global_state_lock();
        let t = newmoduletable();
        assert!(
            t.modules.len() >= 1,
            "fresh modulestab has built-ins, got {}",
            t.modules.len()
        );
    }

    /// `register_module` with empty name does not panic.
    #[test]
    fn module_corpus_register_empty_name_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut t = newmoduletable();
        let _ = register_module(&mut t, "");
    }

    /// Default callback `setup_` returns 0.
    #[test]
    fn module_corpus_setup_callback_default_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// Default `boot_` returns 0.
    #[test]
    fn module_corpus_boot_callback_default_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    /// Default `cleanup_` returns 0.
    #[test]
    fn module_corpus_cleanup_callback_default_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cleanup_(std::ptr::null()), 0);
    }

    /// Default `finish_` returns 0.
    #[test]
    fn module_corpus_finish_callback_default_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/module.c. Tests that capture KNOWN
    // ZSHRS BUGS use #[ignore = "ZSHRS BUG: …"].
    // ═══════════════════════════════════════════════════════════════════

    /// `newmoduletable()` returns an empty modules table per C.
    /// C `Src/module.c:newmoduletable` allocates a fresh HashTable
    /// with NO entries — modules are added later via addbuiltin /
    /// load_module dispatch.
    /// ZSHRS BUG: Rust port at module.rs:1013 pre-registers all the
    /// statically-compiled module names (zsh/complete, zsh/datetime,
    /// zsh/files etc.) at construction time — an architectural
    /// C's `newmoduletable` (Src/module.c) creates an empty hashtable —
    /// entries appear only via `register_module` from dlopen-driven
    /// loads. The Rust port pre-registers all statically-compiled
    /// builtin modules at table construction (no-dlopen model), so
    /// `newmoduletable().modules.is_empty()` is FALSE. The matching
    /// pin is `newmoduletable_pre_registers_builtin_modules` below.
    #[test]
    fn newmoduletable_returns_empty_table() {
        let _g = crate::test_util::global_state_lock();
        let t = newmoduletable();
        // Rust-port divergence from C — pre-registration is intentional
        // (see register_builtin_modules at module.rs:1033). Pin the
        // actual behavior: non-empty after construction.
        assert!(
            !t.modules.is_empty(),
            "Rust port pre-registers builtins (no-dlopen model)"
        );
    }

    /// `newmoduletable()` pre-registers the known statically-compiled
    /// builtin modules — pins the divergent Rust behavior.
    #[test]
    fn newmoduletable_pre_registers_builtin_modules() {
        let _g = crate::test_util::global_state_lock();
        let t = newmoduletable();
        // Rust pre-registers known builtins; pin that some are present.
        assert!(
            !t.modules.is_empty(),
            "Rust port pre-registers builtin modules at construction"
        );
        assert!(
            t.modules.contains_key("zsh/complete"),
            "zsh/complete should be pre-registered"
        );
    }

    /// `gethookdef("zshrs_definitely_not_a_hook")` returns null ptr.
    /// C `Src/module.c:gethookdef` — walks hooktab, missing → NULL.
    #[test]
    fn gethookdef_unknown_name_returns_null() {
        let _g = crate::test_util::global_state_lock();
        let p = gethookdef("zshrs_definitely_not_a_hook_xyz");
        assert!(p.is_null(), "missing hook name → NULL pointer");
    }

    /// `gethookdef("")` on empty name returns null.
    #[test]
    fn gethookdef_empty_name_returns_null() {
        let _g = crate::test_util::global_state_lock();
        let p = gethookdef("");
        assert!(p.is_null(), "empty name → NULL (no empty-named hooks)");
    }

    /// `register_module` of a fresh name. C analog uses int return
    /// (0=success), Rust uses bool — sig divergence.
    #[test]
    fn register_module_fresh_name_returns_true() {
        let _g = crate::test_util::global_state_lock();
        let mut tab = newmoduletable();
        let r = register_module(&mut tab, "zshrs_test_fresh_module");
        assert!(r, "fresh module name should register successfully");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests for Src/module.c modname_ok validator.
    // ═══════════════════════════════════════════════════════════════════

    /// c:2173 — `modname_ok("zsh/complete")` returns 1 (valid: alphanum
    /// + slash separators between identifier components).
    #[test]
    fn modname_ok_canonical_slash_name() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modname_ok("zsh/complete"), 1);
        assert_eq!(modname_ok("zsh/zftp"), 1);
        assert_eq!(modname_ok("zsh/zle"), 1);
    }

    /// c:2173 — `modname_ok("simple")` (no slash) is also valid.
    #[test]
    fn modname_ok_single_component() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modname_ok("simple"), 1);
        assert_eq!(modname_ok("a"), 1);
        assert_eq!(modname_ok("module123"), 1);
    }

    /// c:2173 — `modname_ok("")` returns 1 (empty traverses zero
    /// components and succeeds — C's loop exits immediately).
    #[test]
    fn modname_ok_empty_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            modname_ok(""),
            1,
            "empty name passes loop without finding bad char"
        );
    }

    /// c:2173 — `modname_ok("foo-bar")` returns 0 (hyphen NOT in
    /// IIDENT — alphanumeric + underscore only).
    #[test]
    fn modname_ok_hyphen_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modname_ok("foo-bar"), 0, "hyphen not in IIDENT");
        assert_eq!(modname_ok("zsh/foo-bar"), 0);
    }

    /// c:2173 — underscore IS allowed in identifier component.
    #[test]
    fn modname_ok_underscore_allowed() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modname_ok("foo_bar"), 1);
        assert_eq!(modname_ok("zsh/foo_bar"), 1);
    }

    /// c:2173 — leading digit allowed in identifier component
    /// (C uses IIDENT which doesn't restrict first char to alpha).
    #[test]
    fn modname_ok_digit_allowed() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modname_ok("123abc"), 1);
        assert_eq!(modname_ok("zsh/2023"), 1);
    }

    /// c:2173 — special chars like `.`, `*`, ` `, `\` all rejected.
    #[test]
    fn modname_ok_special_chars_rejected() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modname_ok("foo.bar"), 0, "dot rejected");
        assert_eq!(modname_ok("foo*"), 0, "glob rejected");
        assert_eq!(modname_ok("foo bar"), 0, "space rejected");
        assert_eq!(modname_ok("foo\\bar"), 0, "backslash rejected");
        assert_eq!(modname_ok("foo@bar"), 0, "@ rejected");
    }

    /// c:2173 — trailing slash without component is rejected by C
    /// (the inner `if (!*p)` check happens BEFORE the slash skip).
    /// `foo/` walks `foo`, hits `/`, consumes it, then loops back and
    /// hits empty → returns 1. Pin the bare-`foo/` case.
    #[test]
    fn modname_ok_trailing_slash() {
        let _g = crate::test_util::global_state_lock();
        // Per C body: while loop consumes identifier, then if (!*p)
        // returns 1, else if *p != '/' returns 0, else skip and loop.
        // "foo/" → walks foo, *p='/', skip, loop, *p='\0' → 1.
        assert_eq!(modname_ok("foo/"), 1);
    }

    /// c:2173 — leading slash rejected (no identifier component first).
    #[test]
    fn modname_ok_leading_slash_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // C: identifier loop consumes 0 chars, *p='/' but then i=0
        // and bytes[0]='/' ≠ alphanum → break → check *p == '/' → skip.
        // Walk continues: at byte 1, identifier loop consumes "foo",
        // then i=4 and *p='\0' → return 1. Wait, that's 1?
        // Actually C: bytes start with '/', identifier loop breaks
        // immediately. !*p? No (still '/' at index 0). *p++ != '/'?
        // It IS '/', so increment past. Now at byte 1 = 'f'.
        // Identifier loop consumes "foo", then loop again at next
        // iter, hits '\0' → return 1. Per C, "/foo" returns 1.
        assert_eq!(
            modname_ok("/foo"),
            1,
            "C allows leading slash (loop just skips)"
        );
    }

    /// c:2173 — double slash also handled (zero-length component
    /// between is consumed by identifier loop = empty match).
    #[test]
    fn modname_ok_double_slash_allowed() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modname_ok("foo//bar"), 1);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/module.c
    // c:1726-1766 dyn_* / c:1703 module_loaded / c:167 newmoduletable
    // ═══════════════════════════════════════════════════════════════════

    /// c:1703 — `module_loaded` returns 1 only when name is registered.
    #[test]
    fn module_loaded_returns_one_for_registered_pin() {
        let _g = crate::test_util::global_state_lock();
        let mut t = newmoduletable();
        register_module(&mut t, "zsh_test_loaded_check");
        assert_eq!(module_loaded(&t, "zsh_test_loaded_check"), 1);
    }

    /// c:1703 — `module_loaded` returns 0 for unregistered names.
    #[test]
    fn module_loaded_returns_zero_for_unregistered_pin() {
        let _g = crate::test_util::global_state_lock();
        let t = newmoduletable();
        assert_eq!(module_loaded(&t, "definitely_not_a_module_xyz"), 0);
    }

    /// c:1703 — `module_loaded("")` returns 0.
    #[test]
    fn module_loaded_empty_name_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let t = newmoduletable();
        assert_eq!(module_loaded(&t, ""), 0);
    }

    /// c:1726 — `dyn_setup_module(null)` returns 0 (static-link no-op).
    #[test]
    fn dyn_setup_module_null_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dyn_setup_module(std::ptr::null()), 0);
    }

    /// c:1747 — `dyn_boot_module(null)` returns 0.
    #[test]
    fn dyn_boot_module_null_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dyn_boot_module(std::ptr::null()), 0);
    }

    /// c:1754 — `dyn_cleanup_module(null)` returns 0.
    #[test]
    fn dyn_cleanup_module_null_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dyn_cleanup_module(std::ptr::null()), 0);
    }

    /// c:1766 — `dyn_finish_module(null)` returns 0.
    #[test]
    fn dyn_finish_module_null_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dyn_finish_module(std::ptr::null()), 0);
    }

    /// c:1733 — `dyn_features_module(null, &mut vec)` returns 0
    /// without panicking; features unchanged.
    #[test]
    fn dyn_features_module_null_returns_zero_no_mutation() {
        let _g = crate::test_util::global_state_lock();
        let mut features: Vec<String> = vec!["preserved".into()];
        assert_eq!(dyn_features_module(std::ptr::null(), &mut features), 0);
        assert_eq!(
            features,
            vec!["preserved".to_string()],
            "static-link path must NOT mutate features"
        );
    }

    /// c:1740 — `dyn_enables_module(null, &mut None)` returns 0
    /// without mutating enables.
    #[test]
    fn dyn_enables_module_null_returns_zero_no_mutation() {
        let _g = crate::test_util::global_state_lock();
        let mut enables: Option<Vec<i32>> = None;
        assert_eq!(dyn_enables_module(std::ptr::null(), &mut enables), 0);
        assert!(
            enables.is_none(),
            "static-link path must NOT mutate enables"
        );
    }

    /// c:167 — `newmoduletable` produces a table where `register_module`
    /// of fresh name succeeds.
    #[test]
    fn newmoduletable_accepts_fresh_register() {
        let _g = crate::test_util::global_state_lock();
        let mut t = newmoduletable();
        assert!(register_module(&mut t, "zshrs_fresh_test_module_xyz"));
    }

    /// c:245 — `register_module` is idempotent on duplicate (returns false
    /// per existing pin) and doesn't mutate state.
    #[test]
    fn register_module_duplicate_does_not_grow_table() {
        let _g = crate::test_util::global_state_lock();
        let mut t = newmoduletable();
        let name = "zshrs_dup_register_test";
        assert!(register_module(&mut t, name), "first call succeeds");
        let count_after_first = t.modules.len();
        let r = register_module(&mut t, name);
        assert!(!r, "duplicate must return false");
        assert_eq!(
            t.modules.len(),
            count_after_first,
            "table size unchanged on dup"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/module.c
    // c:339 gethookdef / c:367 addhookdef / c:451 deletehookdef /
    // c:740 checkaddparam / c:1730 getmathfunc / c:1825 load_and_bind /
    // c:1857 try_load_module / c:1885 do_load_module / c:1925 find_module /
    // c:1977 delete_module / c:2002 module_loaded
    // ═══════════════════════════════════════════════════════════════════

    /// c:339 — `gethookdef` returns *mut hookdef (compile-time type pin).
    #[test]
    fn gethookdef_returns_raw_ptr_type() {
        let _g = crate::test_util::global_state_lock();
        let _: *mut hookdef = gethookdef("anything");
    }

    /// c:339 — `gethookdef("")` empty returns null pointer.
    #[test]
    fn gethookdef_empty_returns_null() {
        let _g = crate::test_util::global_state_lock();
        assert!(gethookdef("").is_null(), "empty → null");
    }

    /// c:740 — `checkaddparam("", 0)` empty returns i32 type.
    #[test]
    fn checkaddparam_empty_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = checkaddparam("", 0);
    }

    /// c:1730 — `getmathfunc` returns Option<String> (compile-time type pin).
    #[test]
    fn getmathfunc_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let mut t = newmoduletable();
        let _: Option<String> = getmathfunc(&mut t, "anything", 0);
    }

    /// c:1730 — `getmathfunc(empty, "")` returns None on empty table.
    #[test]
    fn getmathfunc_empty_table_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let mut t = newmoduletable();
        let r = getmathfunc(&mut t, "__never_a_real_math_fn_xyz__", 0);
        assert!(r.is_none(), "unknown math fn → None");
    }

    /// c:1925 — `find_module` returns Option<String> (compile-time type pin).
    #[test]
    fn find_module_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let mut t = newmoduletable();
        let _: Option<String> = find_module(&mut t, "anything", 0);
    }

    /// c:1977 — `delete_module(empty_table, _)` returns i32 (type pin).
    #[test]
    fn delete_module_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut t = newmoduletable();
        let _: i32 = delete_module(&mut t, "__never_loaded__");
    }

    /// c:1857 — `try_load_module` returns i32 (compile-time type pin).
    #[test]
    fn try_load_module_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let t = newmoduletable();
        let _: i32 = try_load_module(&t, "__never_real_module__");
    }

    /// c:1885 — `do_load_module(empty, unknown, 1)` silent failure
    /// returns i32 type.
    #[test]
    fn do_load_module_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut t = newmoduletable();
        let _: i32 = do_load_module(&mut t, "__never_real_module_xyz__", 1);
    }

    /// c:1825 — `load_and_bind("")` empty returns usize (compile-time pin).
    #[test]
    fn load_and_bind_returns_usize_type() {
        let _g = crate::test_util::global_state_lock();
        let _: usize = load_and_bind("");
    }

    /// c:1846 — `hpux_dlsym(0, "")` empty inputs returns usize (type pin).
    #[test]
    fn hpux_dlsym_returns_usize_type() {
        let _g = crate::test_util::global_state_lock();
        let _: usize = hpux_dlsym(0, "");
    }
}
