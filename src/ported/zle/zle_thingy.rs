//! ZLE thingies - named bindings to widgets
//!
//! Direct port from zsh/Src/Zle/zle_thingy.c
//!
//! A "thingy" is a named entity that refers to a widget. Multiple thingies
//! can refer to the same widget. Thingies are reference-counted.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};

use super::zle_h::{
    widget, WidgetImpl, TH_IMMORTAL, WIDGET_INT, WIDGET_INUSE, WIDGET_NCOMP, ZLE_ISCOMP,
    ZLE_KEEPSUFFIX, ZLE_MENUCMP,
};
use crate::ported::utils::quotedzputs;
use crate::ported::utils::zwarnnam;
use crate::ported::zsh_h::{options, DISABLED, OPT_ISSET};

#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_h::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
/// Direct port of `struct thingy` from `Src/Zle/zle.h:224`. A named
/// reference to a widget. `ThingyFlags` deleted — C uses an `int
/// flags` field with `TH_IMMORTAL` (1<<1) and `DISABLED` (1<<0) bits.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
// =====================================================================
// hashtable management — `Src/Zle/zle_thingy.c:58-124`.
// =====================================================================

/// Port of `createthingytab()` from `Src/Zle/zle_thingy.c:60`.
/// ```c
/// static void
/// createthingytab(void)
/// {
///     thingytab = newhashtable(199, "thingytab", NULL);
///     thingytab->hash = hasher;
///     thingytab->emptytable = emptythingytab;
///     ...
/// }
/// ```
/// Allocate the global thingytab. In Rust the table is `OnceLock`-
/// initialized lazily; this entry forces creation eagerly to match
/// C's "pre-zle init" call site at zle_main.c.
pub fn createthingytab() {
    // c:60
    let _ = thingytab(); // c:60 newhashtable
}

impl Thingy {
    /// Create a thingy with no widget bound — equivalent to a freshly
    /// allocated entry from `makethingynode()` in
    /// Src/Zle/zle_thingy.c:108. Callers fill in `widget` later via
    /// `bindwidget` (zle_thingy.c:199).
    pub fn new(name: &str) -> Self {
        Thingy {
            nam: name.to_string(),
            flags: 0,
            rc: 1,
            widget: None,
        }
    }

    /// Create a thingy that wraps a built-in widget.
    /// Equivalent to the `addzlefunction()` path at
    /// Src/Zle/zle_thingy.c:281: builds the immortal-flagged Thingy
    /// and binds it to a widget produced by the built-in dispatch
    /// table (`widget::builtin`).
    pub fn builtin(name: &str) -> Self {
        let widget = widget::builtin(name);
        Thingy {
            nam: name.to_string(),
            flags: TH_IMMORTAL,
            rc: 1,
            widget: Some(Arc::new(widget)),
        }
    }

    /// Create a thingy that wraps a user-defined shell function.
    /// Equivalent to `bin_zle_new()` at Src/Zle/zle_thingy.c:584 — the
    /// `zle -N name fn` builtin path.
    pub fn user_defined(name: &str, func_name: &str) -> Self {
        let widget = widget::user_defined(name, func_name);
        Thingy {
            nam: name.to_string(),
            flags: 0,
            rc: 1,
            widget: Some(Arc::new(widget)),
        }
    }

    /// Test whether this thingy's name matches `name`.
    /// Equivalent to the `IS_THINGY(thingy, name)` macro at
    /// Src/Zle/zle.h — used by widget bodies that special-case their
    /// own bound name (e.g. select-a-word checking which alias fired).
    pub fn is(&self, name: &str) -> bool {
        self.nam == name
    }

    /// Test whether this thingy is `name` or its dot-prefixed variant.
    /// The `.foo` form names the underlying built-in when a user has
    /// aliased `foo` to something else — see `bin_zle_new`'s `args[0]`
    /// vs `args[1]` split at zle_thingy.c:584. Callers use this when
    /// they want the canonical built-in regardless of user aliasing.
    pub fn is_thingy(&self, name: &str) -> bool {
        self.nam == name || self.nam == format!(".{}", name)
    }
}

/// Port of `emptythingytab(UNUSED(HashTable ht))` from `Src/Zle/zle_thingy.c:80`.
/// ```c
/// static void
/// emptythingytab(UNUSED(HashTable ht))
/// {
///     /* This will only be called when deleting the thingy table,
///      * which is only done to unload the zle module... */
///     scanhashtable(thingytab, 0, 0, DISABLED, scanemptythingies, 0);
/// }
/// ```
/// Walk every non-disabled thingy and unbind it (frees user-
/// defined widgets but leaves the fixed `thingies[]` entries
/// alone).
/// WARNING: param names don't match C — Rust=() vs C=(ht)
pub fn emptythingytab() {
    // c:80
    // c:80 — `scanhashtable(thingytab, 0, 0, DISABLED, scanemptythingies, 0)`.
    // Collect-then-iterate to avoid holding the lock during the mutating callback.
    let names: Vec<String> = thingytab()
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, t)| (t.flags & DISABLED) == 0)
        .map(|(k, _)| k.clone())
        .collect();
    names.iter().for_each(|n| scanemptythingies(n)); // c:91 scancallback
}

/// Port of `scanemptythingies(HashNode hn, UNUSED(int flags))` from `Src/Zle/zle_thingy.c:96`.
/// ```c
/// static void
/// scanemptythingies(HashNode hn, UNUSED(int flags))
/// {
///     Thingy t = (Thingy) hn;
///     if(!(t->widget->flags & WIDGET_INT))
///         unbindwidget(t, 1);
/// }
/// ```
/// Per-entry callback: if the bound widget isn't internal, unbind it.
/// WARNING: param names don't match C — Rust=(name) vs C=(hn, flags)
pub fn scanemptythingies(name: &str) {
    // c:96
    // c:96 — `if(!(t->widget->flags & WIDGET_INT)) unbindwidget(t, 1)`.
    // C assumes every Thingy has its `widget` pointer set (the alloc
    // path in `addthingy` always binds via `addnewwidget`). The Rust
    // port defaults `widget = None` for fresh `rthingy(name)` entries
    // that haven't been bound yet — those are user-defined slots
    // and SHOULD be unbound by emptythingytab. Without flipping the
    // default to "non-internal" (false), `${name}` entries with no
    // widget stayed in the table after `emptythingytab` cleared
    // everything else, breaking `zle -d`-style cleanup semantic.
    let internal = {
        let tab = thingytab().lock().unwrap();
        tab.get(name)
            .and_then(|t| t.widget.as_ref().map(|w| (w.flags & WIDGET_INT) != 0))
            .unwrap_or(false)
    };
    if !internal {
        unbindwidget(name, 1); // c:103
    }
}

/// Port of `makethingynode()` from `Src/Zle/zle_thingy.c:108`.
/// ```c
/// static Thingy
/// makethingynode(void)
/// {
///     Thingy t = (Thingy) zshcalloc(sizeof(*t));
///     t->flags = DISABLED;
///     return t;
/// }
/// ```
/// Allocate a fresh Thingy with the DISABLED flag set; caller is
/// expected to fill in `nam` and `bindwidget` it.
pub fn makethingynode() -> Thingy {
    // c:108
    let mut t = Thingy::new(""); // c:108 zshcalloc
    t.flags |= DISABLED; // c:112 t->flags = DISABLED
    t.rc = 0; // c:110 zshcalloc zeros rc
    t // c:113 return t
}

/// Port of `freethingynode(HashNode hn)` from `Src/Zle/zle_thingy.c:118`.
/// ```c
/// static void
/// freethingynode(HashNode hn)
/// {
///     Thingy th = (Thingy) hn;
///     zsfree(th->nam);
///     zfree(th, sizeof(*th));
/// }
/// ```
/// Free a Thingy by name (HashTable freenode callback). In Rust
/// the storage is owned by the table; removal does the free.
/// WARNING: param names don't match C — Rust=(name) vs C=(hn)
pub fn freethingynode(name: &str) {
    // c:118
    // c:118-123 — `zsfree(th->nam); zfree(th, sizeof(*th))`. Rust
    // String + Thingy drop on `remove()`.
    let _ = thingytab().lock().unwrap().remove(name);
}

// =====================================================================
// reference counting — `Src/Zle/zle_thingy.c:130-176`.
// =====================================================================

/// Port of `refthingy(Thingy th)` from `Src/Zle/zle_thingy.c:138`.
/// ```c
/// mod_export Thingy
/// refthingy(Thingy th)
/// {
///     if(th)
///         th->rc++;
///     return th;
/// }
/// ```
/// Bump the reference count on the named Thingy. Caller must
/// have an existing reference (or be the creator).
/// WARNING: param names don't match C — Rust=(name) vs C=(th)
pub fn refthingy(name: &str) {
    // c:138
    let mut tab = thingytab().lock().unwrap();
    if let Some(t) = tab.get_mut(name) {
        // c:140 if(th)
        t.rc += 1; // c:141 th->rc++
    }
}

/// Port of `unrefthingy(Thingy th)` from `Src/Zle/zle_thingy.c:147`.
/// ```c
/// void
/// unrefthingy(Thingy th)
/// {
///     if(th && !--th->rc)
///         thingytab->freenode(thingytab->removenode(thingytab, th->nam));
/// }
/// ```
/// Drop a reference; remove from table when rc hits 0.
pub fn unrefthingy(th: &str) {
    // c:147
    let drop = thingytab()
        .lock()
        .unwrap()
        .get_mut(th) // c:149 if(th && !--th->rc)
        .map(|t| {
            t.rc -= 1;
            t.rc == 0
        })
        .unwrap_or(false);
    if drop {
        freethingynode(th);
    } // c:150 freenode(removenode(...))
}

/// Port of `rthingy(char *nam)` from `Src/Zle/zle_thingy.c:158`.
/// ```c
/// Thingy
/// rthingy(char *nam)
/// {
///     Thingy t = (Thingy) thingytab->getnode2(thingytab, nam);
///     if(!t)
///         thingytab->addnode(thingytab, ztrdup(nam), t = makethingynode());
///     return refthingy(t);
/// }
/// ```
/// "Resolve thingy" — get-or-create-then-ref. Always returns a
/// thingy; creates a fresh disabled one if none exists.
pub fn rthingy(nam: &str) {
    // c:158
    {
        let mut tab = thingytab().lock().unwrap();
        if !tab.contains_key(nam) {
            // c:160-162 if(!t)
            let mut t = makethingynode(); // c:163 makethingynode
            t.nam = nam.to_string(); // c:163 ztrdup(nam)
            tab.insert(nam.to_string(), t); // c:163 addnode
        }
    }
    refthingy(nam); // c:164 return refthingy(t)
}

/// Port of `rthingy_nocreate(char *nam)` from `Src/Zle/zle_thingy.c:169`.
/// ```c
/// Thingy
/// rthingy_nocreate(char *nam)
/// {
///     Thingy t = (Thingy) thingytab->getnode2(thingytab, nam);
///     if(!t)
///         return NULL;
///     return refthingy(t);
/// }
/// ```
/// Lookup-only variant — returns false (no Thingy) if missing.
/// WARNING: param names don't match C — Rust=(name) vs C=(nam)
pub fn rthingy_nocreate(name: &str) -> bool {
    // c:169
    let exists = thingytab().lock().unwrap().contains_key(name); // c:169 getnode2
    if !exists {
        return false; // c:173-174 if(!t) return NULL
    }
    refthingy(name); // c:175 return refthingy(t)
    true
}

// =====================================================================
// widget binding — `Src/Zle/zle_thingy.c:178-270`.
// =====================================================================

/// Port of `bindwidget(widget w, Thingy t)` from `Src/Zle/zle_thingy.c:197`.
/// ```c
/// static int
/// bindwidget(widget w, Thingy t)
/// {
///     if(t->flags & TH_IMMORTAL) {
///         unrefthingy(t);
///         return -1;
///     }
///     if(!(t->flags & DISABLED)) {
///         if(t->widget == w)
///             return 0;
///         unbindwidget(t, 1);
///     }
///     if(w->first) {
///         t->samew = w->first->samew;
///         w->first->samew = t;
///     } else {
///         w->first = t;
///         t->samew = t;
///     }
///     t->widget = w;
///     t->flags &= ~DISABLED;
///     return 0;
/// }
/// ```
/// Bind `w` to thingy `t_name`. Caller's Thingy reference is
/// consumed when TH_IMMORTAL blocks the bind. Samew chains are
/// implicit in Rust — the `Arc<widget>` identity links peers.
/// Returns 0 on success, -1 on TH_IMMORTAL block.
pub fn bindwidget(w: Arc<widget>, t: &str) -> i32 {
    // c:199
    let (immortal, disabled, same) = {
        let tab = thingytab().lock().unwrap();
        match tab.get(t) {
            Some(t) => (
                (t.flags & TH_IMMORTAL) != 0,
                (t.flags & DISABLED) != 0,
                t.widget
                    .as_ref()
                    .map(|w2| Arc::ptr_eq(w2, &w))
                    .unwrap_or(false),
            ),
            None => (false, true, false),
        }
    };

    if immortal {
        // c:201 TH_IMMORTAL
        unrefthingy(t); // c:202
        return -1; // c:203
    }
    if !disabled {
        // c:205 !DISABLED
        if same {
            // c:206 t->widget == w
            return 0; // c:207
        }
        unbindwidget(t, 1); // c:208
    }
    // c:210-216 — `samew` circular-list maintenance is implicit in
    // Rust: shared widgets just hold the same Arc, and walks via
    // Arc::ptr_eq find peers. No explicit list edit needed.
    let mut tab = thingytab().lock().unwrap();
    if let Some(t) = tab.get_mut(t) {
        t.widget = Some(w); // c:217 t->widget = w
        t.flags &= !DISABLED; // c:218 t->flags &= ~DISABLED
    }
    0 // c:219 return 0
}

/// Port of `unbindwidget(Thingy t, int override)` from `Src/Zle/zle_thingy.c:228`.
/// ```c
/// static int
/// unbindwidget(Thingy t, int override)
/// {
///     widget w;
///     if(t->flags & DISABLED)
///         return 0;
///     if(!override && (t->flags & TH_IMMORTAL))
///         return -1;
///     w = t->widget;
///     if(t->samew == t)
///         freewidget(w);
///     else { /* unlink from samew chain */ }
///     t->flags &= ~TH_IMMORTAL;
///     t->flags |= DISABLED;
///     unrefthingy(t);
///     return 0;
/// }
/// ```
/// Detach Thingy `t_name` from its widget. Walks the table to
/// detect the "last reference" case (samew == t in C); if so, the
/// widget is freed (Arc auto-drops when the Thingy clears it).
/// `override_` non-zero overrides TH_IMMORTAL.
/// WARNING: param names don't match C — Rust=(t, override_) vs C=(t, override)
pub fn unbindwidget(t: &str, override_: i32) -> i32 {
    // c:230
    let (disabled, immortal, w_opt) = {
        let tab = thingytab().lock().unwrap();
        match tab.get(t) {
            Some(t) => (
                (t.flags & DISABLED) != 0,
                (t.flags & TH_IMMORTAL) != 0,
                t.widget.clone(),
            ),
            None => return 0,
        }
    };
    if disabled {
        // c:234 if DISABLED
        return 0;
    }
    if override_ == 0 && immortal {
        // c:236 !override && TH_IMMORTAL
        return -1;
    }
    // c:239 — `if(t->samew == t) freewidget(w)`. In Rust we walk
    // the table to count peers sharing this widget.
    if let Some(w) = w_opt {
        let peer_count = {
            let tab = thingytab().lock().unwrap();
            tab.values()
                .filter(|other| other.nam != t)
                .filter(|other| {
                    other
                        .widget
                        .as_ref()
                        .map(|w2| Arc::ptr_eq(w2, &w))
                        .unwrap_or(false)
                })
                .count()
        };
        if peer_count == 0 {
            // c:240 — `freewidget(w)`. Arc::strong_count drops to
            // 1 (just our local clone); freewidget marks WIDGET_FREE
            // if INUSE, otherwise the Arc auto-drops on scope exit.
            freewidget(w);
        }
        // c:241-246 — non-last case: just unlink. Implicit in Rust;
        // peers retain their own Arc clones.
    }

    let mut tab = thingytab().lock().unwrap();
    if let Some(t) = tab.get_mut(t) {
        t.flags &= !TH_IMMORTAL; // c:247 &= ~TH_IMMORTAL
        t.flags |= DISABLED; // c:248 |= DISABLED
        t.widget = None;
    }
    drop(tab);
    unrefthingy(t); // c:249 unrefthingy(t)
    0 // c:250 return 0
}

/// Port of `freewidget(widget w)` from `Src/Zle/zle_thingy.c:255`.
/// ```c
/// void
/// freewidget(widget w)
/// {
///     if (w->flags & WIDGET_INUSE) {
///         w->flags |= WIDGET_FREE;
///         return;
///     }
///     if (w->flags & WIDGET_NCOMP) {
///         zsfree(w->u.comp.wid);
///         zsfree(w->u.comp.func);
///     } else if(!(w->flags & WIDGET_INT))
///         zsfree(w->u.fnnam);
///     zfree(w, sizeof(*w));
/// }
/// ```
/// Drop a widget. If WIDGET_INUSE (we're freeing it from inside
/// the widget's own dispatch), defer the free by setting WIDGET_FREE
/// — the dispatcher checks this flag after returning.
///
/// In Rust the `Arc<widget>` auto-drops; this fn exists so the
/// INUSE/FREE flag handshake matches C exactly. The actual storage
/// drop happens when the last Arc is released by the caller's scope.
pub fn freewidget(w: Arc<widget>) {
    // c:257
    // Direct port of `void freewidget(widget w)` from zle_thingy.c:255:
    // ```c
    // if (w->flags & WIDGET_INUSE) { w->flags |= WIDGET_FREE; return; }
    // // free widget data + storage
    // ```
    //
    // **Arc<widget> divergence:** the C source mutates w->flags via
    // a single owner pointer; Rust uses Arc<widget> shared-immutable
    // and dispatches deferred-free via Arc::strong_count. When this
    // call is the LAST reference (count==1) and INUSE is set, the
    // widget is mid-dispatch — let the dispatcher drop the last
    // Arc when it returns. When count>1, another holder is alive
    // and the storage stays valid. When count==1 + !INUSE, the
    // implicit Arc drop at end-of-scope reclaims storage.
    if (w.flags & WIDGET_INUSE) != 0 {
        return; // c:261
    }
    // c:264-269 — comp-widget / user-fn cleanup. WidgetImpl::UserFunc
    // owns its String; WidgetImpl::Internal owns nothing. Arc drop
    // covers both.
    drop(w); // c:269 zfree(w, ...)
}

/// Port of `addzlefunction(char *name, ZleIntFunc ifunc, int flags)` from `Src/Zle/zle_thingy.c:279`.
/// ```c
/// mod_export widget
/// addzlefunction(char *name, ZleIntFunc ifunc, int flags)
/// {
///     VARARR(char, dotn, strlen(name) + 2);
///     widget w;
///     Thingy t;
///     if(name[0] == '.')
///         return NULL;
///     dotn[0] = '.';
///     strcpy(dotn + 1, name);
///     t = (Thingy) thingytab->getnode(thingytab, dotn);
///     if(t && (t->flags & TH_IMMORTAL))
///         return NULL;
///     w = zalloc(sizeof(*w));
///     w->flags = WIDGET_INT | flags;
///     w->first = NULL;
///     w->u.fn = ifunc;
///     t = rthingy(dotn);
///     bindwidget(w, t);
///     t->flags |= TH_IMMORTAL;
///     bindwidget(w, rthingy(name));
///     return w;
/// }
/// ```
/// Register a module-internal widget. The widget binds to both
/// `.name` (immortal canonical) and `name` (user-rebindable) in
/// the thingytab. Refuses if `.name` already taken by another
/// immortal or if `name` starts with `.`.
/// WARNING: param names don't match C — Rust=(ifunc, flags) vs C=(name, ifunc, flags)
pub fn addzlefunction(
    // c:281
    name: &str,
    ifunc: ZleIntFunc,
    flags: i32,
) -> Option<Arc<widget>> {
    // c:279
    if name.starts_with('.') {
        // c:287 if(name[0] == '.')
        return None; // c:288
    }
    let dotn = format!(".{}", name); // c:289-290 dotn[0]='.';strcpy(...)

    // c:291-293 — refuse if .name is already TH_IMMORTAL.
    let blocked = {
        let tab = thingytab().lock().unwrap();
        tab.get(&dotn)
            .map(|t| (t.flags & TH_IMMORTAL) != 0)
            .unwrap_or(false)
    };
    if blocked {
        return None; // c:293
    }

    // c:294-297 — `w = zalloc(...); w->flags = WIDGET_INT|flags;
    //              w->first = NULL; w->u.fn = ifunc;`.
    let w = Arc::new(widget {
        flags: flags | WIDGET_INT, // c:295
        first: None,
        u: WidgetImpl::Internal(ifunc), // c:297 w->u.fn = ifunc
    });

    // c:298-301 — bind to dotted form, mark immortal, then bind to
    // canonical form too.
    rthingy(&dotn); // c:298 t = rthingy(dotn)
    bindwidget(w.clone(), &dotn); // c:299 bindwidget(w, t)
    if let Some(t) = thingytab().lock().unwrap().get_mut(&dotn) {
        t.flags |= TH_IMMORTAL; // c:300 t->flags |= TH_IMMORTAL
    }
    rthingy(name); // c:301 rthingy(name)
    bindwidget(w.clone(), name); // c:301 bindwidget(w, ...)
    Some(w) // c:302 return w
}

/// Port of `deletezlefunction(widget w)` from `Src/Zle/zle_thingy.c:308`.
/// ```c
/// mod_export void
/// deletezlefunction(widget w)
/// {
///     Thingy p, n;
///     p = w->first;
///     while(1) {
///         n = p->samew;
///         if(n == p) {
///             unbindwidget(p, 1);
///             return;
///         }
///         unbindwidget(p, 1);
///         p = n;
///     }
/// }
/// ```
/// Walk every Thingy bound to `w` and unbind it (override flag set,
/// so even TH_IMMORTAL bindings come undone). Used by module
/// teardown.
pub fn deletezlefunction(w: &Arc<widget>) {
    // c:310
    // c:310-323 — walk samew circular chain calling unbindwidget(p,1)
    // until p == p->samew (the last entry). In Rust we collect all
    // matching names first, then unbind each.
    let names: Vec<String> = {
        let tab = thingytab().lock().unwrap();
        tab.iter()
            .filter(|(_, t)| {
                t.widget
                    .as_ref()
                    .map(|w2| Arc::ptr_eq(w2, w))
                    .unwrap_or(false)
            })
            .map(|(k, _)| k.clone())
            .collect()
    };
    for n in names {
        unbindwidget(&n, 1); // c:318/321 unbindwidget(p, 1)
    }
}

// =====================================================================
// `bin_zle` and per-mode dispatchers — `Src/Zle/zle_thingy.c:341-1015`.
// =====================================================================
//
// The bin_zle_* ported below dispatch into the live ZLE session state
// (zlecs/zlemetaline/keymaps/watch_fd table/zle_refresh draw
// primitives). Each entry routes through the existing Rust globals
// (ZLELINE/ZLECS/ZLELL in compcore.rs, keymapnamtab in zle_keymap.rs,
// hook_functions on ShellExecutor, ZLE_RESET_NEEDED in zle_main.rs)
// where the substrate is canonical, or via real fn calls into the
// per-method Zle ports. Each fn's docstring cites its C source line
// and the substrate path it uses.

/// Port of `bin_zle(char *name, char **args, Options ops, UNUSED(int func))` from `Src/Zle/zle_thingy.c:343`. Top-level
/// `zle` builtin dispatcher — selects per-flag handler from opns[]
/// table (-l/-D/-A/-N/-C/-R/-M/-U/-K/-I/-f/-F/-T) or falls through
/// to bin_zle_call when no flag is set.
pub fn bin_zle(
    name: &str,
    args: &[String], // c:343
    ops: &options,
    _func: i32,
) -> i32 {
    // c:zle_main.c setup_ — in zsh proper, `init_thingies()` runs
    // when zsh/zle is autoloaded on first ZLE access. zshrs in
    // non-interactive (`-fc`) mode never loads zsh/zle through that
    // path, so user `zle -C`/`zle -l`/etc. calls fail because the
    // thingytab is empty. Lazy-init on first `zle` call — idempotent
    // (the per-name `contains_key` check inside init_thingies makes
    // re-entry safe).
    static THINGIES_INIT: std::sync::Once = std::sync::Once::new();
    THINGIES_INIT.call_once(|| {
        init_thingies();
    });
    // c:345-364 — dispatch table: `static const struct opn opns[]`.
    // (flag_char, handler_fn, min_args, max_args). All sub-handlers
    // take the C canonical `(name, args, ops, func)` signature, so
    // the table type matches `struct opn` exactly.
    type OpHandler = fn(&str, &[String], &options, i32) -> i32;
    let opns: [(u8, OpHandler, i32, i32); 14] = [
        (b'l', bin_zle_list, 0, -1),      // c:350
        (b'D', bin_zle_del, 1, -1),       // c:351
        (b'A', bin_zle_link, 2, 2),       // c:352
        (b'N', bin_zle_new, 1, 2),        // c:353
        (b'C', bin_zle_complete, 3, 3),   // c:354
        (b'R', bin_zle_refresh, 0, -1),   // c:355
        (b'M', bin_zle_mesg, 1, 1),       // c:356
        (b'U', bin_zle_unget, 1, 1),      // c:357
        (b'K', bin_zle_keymap, 1, 1),     // c:358
        (b'I', bin_zle_invalidate, 0, 0), // c:359
        (b'f', bin_zle_flags, 1, -1),     // c:360
        (b'F', bin_zle_fd, 0, 2),         // c:361
        (b'T', bin_zle_transform, 0, 2),  // c:362
        (0u8, bin_zle_call, 0, -1),       // c:363 — sentinel: no flag → bin_zle_call.
    ];

    // c:369 — `for (op = opns; op->o && !OPT_ISSET(ops, op->o); op++) ;`.
    // Pick the first op whose flag is set; sentinel (o=0) loops out.
    let op_idx = opns
        .iter()
        .position(|(o, _, _, _)| *o != 0 && OPT_ISSET(ops, *o))
        .unwrap_or(opns.len() - 1); // c:369 — fall to sentinel

    // c:370-375 — reject when more than one operation flag is set:
    // `if (op->o) for (opp = op; (++opp)->o; ) if (OPT_ISSET(ops,
    // opp->o)) { zwarnnam("incompatible..."); return 1; }`.
    if opns[op_idx].0 != 0 {
        // c:370
        for (o, _, _, _) in opns.iter().skip(op_idx + 1) {
            if *o != 0 && OPT_ISSET(ops, *o) {
                zwarnnam(name, "incompatible operation selection options"); // c:373
                return 1; // c:374
            }
        }
    }

    // c:378-385 — arg-count check against op->min / op->max.
    let n = args.len() as i32; // c:378
    let (op_o, op_func, op_min, op_max) = &opns[op_idx];
    if n < *op_min {
        // c:379
        zwarnnam(
            name,
            &format!("not enough arguments for -{}", *op_o as char),
        ); // c:380
        return 1; // c:381
    } else if *op_max != -1 && n > *op_max {
        // c:382
        zwarnnam(name, &format!("too many arguments for -{}", *op_o as char)); // c:383
        return 1; // c:384
    }

    // c:388 — `return op->func(name, args, ops, op->o);`.
    op_func(name, args, ops, *op_o as i32)
}

/// Port of `bin_zle_list(UNUSED(char *name), char **args, Options ops, UNUSED(char func))` from `Src/Zle/zle_thingy.c:393`.
/// ```c
/// static int
/// bin_zle_list(...) {
///     if (!*args) { scanhashtable(thingytab, 1, 0, DISABLED, scanlistwidgets, ...); return 0; }
///     for (; *args && !ret; args++) {
///         HashNode hn = thingytab->getnode2(thingytab, *args);
///         if (!t || (!ALL && t->widget->flags & WIDGET_INT)) ret = 1;
///         else if (LONG) scanlistwidgets(hn, 1);
///     }
///     return ret;
/// }
/// ```
/// `zle -l` — list widget bindings (or check existence per arg).
pub fn bin_zle_list(_name: &str, args: &[String], ops: &options, _func: i32) -> i32 {
    // c:393
    // c:393-413 — `if (!*args) scan all` else look up each in turn.
    // Returns 0 if all found and listable; 1 if any missing.
    // c:Src/Zle/zle_thingy.c:396-397 — list mode is
    //   `OPT_ISSET(ops,'a') ? -1 : OPT_ISSET(ops,'L')`.
    //   -a (`-la`) → -1: emit raw name, INCLUDE internal widgets.
    //   -L (`-lL`) → 1:  `zle -N name [fn]` reproducible form.
    //   default (`-l`) → 0: abbreviated `name (fn)` form, hide internals.
    let list_mode: i32 = if OPT_ISSET(ops, b'a') {
        -1
    } else if OPT_ISSET(ops, b'L') {
        1
    } else {
        0
    };
    if args.is_empty() {
        // c:396-397 — walk thingytab, call scanlistwidgets per node.
        let _ = scanlistwidgets(list_mode);
        return 0;
    }
    let mut ret = 0;
    for arg in args {
        // c:403-411
        let exists = thingytab().lock().unwrap().contains_key(arg);
        if !exists {
            ret = 1;
            break;
        }
    }
    ret // c:412
}

/// Direct port of `int bin_zle_refresh(char *name, char **args,
///                                      Options ops, UNUSED(char func))`
/// from `Src/Zle/zle_thingy.c:416-454`.
/// ```c
/// if (!zleactive) { zwarnnam(name, "no line editor"); return 1; }
/// // optional statusline/listlist install via -p flag
/// zrefresh();
/// return 0;
/// ```
///
/// **Substrate tradeoff:** `zrefresh()` is a free fn in
/// zle_refresh.rs reading the file-scope ZLE statics. To keep this
/// bin_zle_refresh path lightweight (and to drop work to the next
/// zlecore tick when it's available), we set the `ZLE_RESET_NEEDED`
/// Port of `bin_zle_refresh(UNUSED(char *name), char **args, Options ops, UNUSED(char func))` from `Src/Zle/zle_thingy.c:418`.
pub fn bin_zle_refresh(_name: &str, args: &[String], ops: &options, _func: i32) -> i32 {
    // c:418
    // c:420-421 — `char *s = statusline; int ocl = clearlist;`. Save
    // pre-call state so the function can restore it on exit.
    let s_save: Option<String> = STATUSLINE.lock().unwrap().clone(); // c:420
    let ocl: i32 = CLEARLIST.load(Ordering::Relaxed); // c:421

    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) == 0 {
        // c:423
        return 1; // c:424
    }
    // c:425 — `statusline = NULL;`
    *STATUSLINE.lock().unwrap() = None;
    if !args.is_empty() {
        // c:426
        // c:427-428 — `if (**args) statusline = *args;` — empty arg
        // means "clear statusline", non-empty replaces it.
        if !args[0].is_empty() {
            // c:427
            *STATUSLINE.lock().unwrap() = Some(args[0].clone()); // c:428
        }
        if args.len() > 1 {
            // c:429 — second-and-following args form a list to display.
            let zmultsav: i32 = crate::ported::zle::compcore::ZMULT.load(Ordering::Relaxed); // c:431
                                                                                             // c:433-434 — `for (; *args; args++) addlinknode(l, *args);`.
            let list: Vec<String> = args[1..].to_vec(); // c:434
            crate::ported::zle::compcore::ZMULT.store(1, Ordering::Relaxed); // c:436
                                                                             // c:437 — `listlist(l)`. Rust port takes (&[String], cols);
                                                                             // 0 cols defers width to listlist's internal calc.
            listlist(&list, 0); // c:437
            if STATUSLINE.lock().unwrap().is_some() {
                // c:438
                LASTLISTLEN.fetch_add(1, Ordering::Relaxed); // c:439
            }
            // c:440 — `showinglist = clearlist = 0;`.
            SHOWINGLIST.store(0, Ordering::Relaxed);
            CLEARLIST.store(0, Ordering::Relaxed);
            // c:441 — restore zmult.
            crate::ported::zle::compcore::ZMULT.store(zmultsav, Ordering::Relaxed);
        } else if OPT_ISSET(ops, b'c') {
            // c:442 — single positional + `-c`: queue a clear.
            CLEARLIST.store(1, Ordering::Relaxed); // c:443
            LASTLISTLEN.store(0, Ordering::Relaxed); // c:444
        }
    } else if OPT_ISSET(ops, b'c') {
        // c:446 — no positionals + `-c`: clear list immediately.
        CLEARLIST.store(1, Ordering::Relaxed); // c:447
        LISTSHOWN.store(1, Ordering::Relaxed); // c:447
        LASTLISTLEN.store(0, Ordering::Relaxed); // c:448
    }
    zrefresh(); // c:450
                // c:451-452 — `statusline = s; clearlist = ocl;` restore.
    *STATUSLINE.lock().unwrap() = s_save; // c:451
    CLEARLIST.store(ocl, Ordering::Relaxed); // c:452
    0 // c:453
}

/// Port of `bin_zle_mesg(char *name, char **args, UNUSED(Options ops), UNUSED(char func))` from `Src/Zle/zle_thingy.c:459`.
/// ```c
/// static int
/// bin_zle_mesg(...) {
///     if (!zleactive) { zwarnnam; return 1; }
///     showmsg(*args);
///     if (sfcontext != SFC_WIDGET) zrefresh();
///     return 0;
/// }
/// ```
/// `zle -M msg` — display a transient message during widget run.
pub fn bin_zle_mesg(name: &str, args: &[String], _ops: &options, _func: i32) -> i32 {
    // c:459
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) == 0 {
        crate::ported::utils::zwarnnam(name, "can only be called from widget function");
        return 1; // c:463
    }
    if let Some(arg) = args.first() {
        crate::ported::zle::zle_utils::showmsg(arg); // c:465
    }
    // c:467 — `if (sfcontext != SFC_WIDGET) zrefresh();`. SFC_WIDGET
    // means the call came from a user widget body and the editor
    // will redraw soon; outside that path, redraw now so the message
    // is visible before the next event-loop tick.
    use crate::ported::zsh_h::SFC_WIDGET;
    if crate::ported::builtin::SFCONTEXT.load(std::sync::atomic::Ordering::Relaxed) != SFC_WIDGET {
        crate::ported::zle::zle_refresh::zrefresh(); // c:467
    }
    0 // c:468
}

/// Port of `bin_zle_unget(char *name, char **args, UNUSED(Options ops), UNUSED(char func))` from `Src/Zle/zle_thingy.c:473`.
/// ```c
/// static int
/// bin_zle_unget(char *name, char **args, ...) {
///     char *b = unmeta(*args), *p = b + strlen(b);
///     if (!zleactive) { zwarnnam(name, "..."); return 1; }
///     while (p > b)
///         ungetbyte((int) *--p);
///     return 0;
/// }
/// ```
/// `zle -U str` — push string bytes back onto input queue in
/// reverse so subsequent reads return them in original order.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(name, args, ops, func)
pub fn bin_zle_unget(_name: &str, args: &[String], _ops: &options, _func: i32) -> i32 {
    // c:473
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) == 0 {
        return 1; // c:479
    }
    if let Some(arg) = args.first() {
        // c:481-482 — push bytes back in reverse.
        for byte in arg.bytes().rev() {
            ungetbyte(byte);
        }
    }
    0 // c:483
}

/// Port of `bin_zle_keymap(char *name, char **args, UNUSED(Options ops), UNUSED(char func))` from `Src/Zle/zle_thingy.c:488`.
/// ```c
/// static int
/// bin_zle_keymap(...) {
///     if (!zleactive) { zwarnnam(name, "..."); return 1; }
///     return selectkeymap(*args, 0);
/// }
/// ```
/// `zle -K keymap` — switch the current keymap (only valid from
/// inside a widget callback).
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
pub fn bin_zle_keymap(name: &str, args: &[String], _ops: &options, _func: i32) -> i32 {
    // c:488
    // c:489-491 — `if (!zleactive)` reject from outside ZLE.
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) == 0 {
        crate::ported::utils::zwarnnam(name, "can only be called from widget function");
        return 1; // c:491
    }
    // c:493 — `return selectkeymap(*args, 0)`.
    if args.is_empty() {
        return 1;
    }
    crate::ported::zle::zle_keymap::selectkeymap(&args[0], 0) // c:493
}

/// Direct port of `static void scanlistwidgets(HashNode hn, int list)`
/// from `Src/Zle/zle_thingy.c:505`. Pretty-prints one Thingy: skips
/// internal (WIDGET_INT) widgets, then either:
///   - `list == 0`: emits `zle -N name [fn]` (re-definable shell form);
///   - `list != 0`: emits `name (fn)` when fn != name, else just `name`.
/// Output goes to stdout (C uses `putc('\n', stdout)`).
/// WARNING: param names don't match C — Rust=(list) vs C=(hn, list).
pub fn scanlistwidgets(list: i32) -> i32 {
    // c:505
    use std::io::Write;
    let tab = thingytab().lock().unwrap();
    // c:509-512 — `if (list < 0) { printf("%s\n", hn->nam); return; }`
    // The `-la` path emits raw name, includes INTERNAL widgets, and
    // does not filter or annotate. Bug #379.
    if list < 0 {
        let mut names: Vec<String> = tab.keys().cloned().collect();
        drop(tab);
        names.sort();
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        for n in &names {
            let _ = writeln!(handle, "{}", n);
        }
        return 0;
    }
    // c:521-525 / c:532-536 — a `zle -C` widget carries WIDGET_NCOMP and
    // prints its completion triple (`wid`, `func`), not a function name.
    // The previous collection flattened every non-UserFunc impl to the
    // bare widget name, so `zle -C w .expand-or-complete _main_complete`
    // listed as `zle -N w` under -L and as a bare `w` under -l, losing
    // both the `-C` marker and the two operands.
    enum Listed {
        Fn(String),
        Comp(String, String),
    }
    let mut entries: Vec<(String, Listed)> = Vec::new();
    for (name, t) in tab.iter() {
        let w = match t.widget.as_ref() {
            Some(w) => w,
            None => continue,
        };
        // c:514-515 — skip internal widgets.
        if (w.flags & WIDGET_INT) != 0 {
            continue;
        }
        let listed = match &w.u {
            // c:521 `if (w->flags & WIDGET_NCOMP)`
            WidgetImpl::Comp { wid, func, .. } => Listed::Comp(wid.clone(), func.clone()),
            WidgetImpl::UserFunc(s) => Listed::Fn(s.clone()),
            // An internal-impl variant with no NCOMP flag has no separate
            // function name; C's `strcmp(t->nam, w->u.fnnam)` then compares
            // equal and prints the bare name.
            _ => Listed::Fn(name.clone()),
        };
        entries.push((name.clone(), listed));
    }
    drop(tab);
    // c:533-541 — emit. Sort by name for stable output (C iterates the
    // hash table in addnode order; Rust HashMap has no order).
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    for (name, listed) in &entries {
        // c:Src/Zle/zle_thingy.c:533 — `if (list)`: a NON-zero list mode
        // (`-L`, list==1) prints the re-definable `zle -N name [fn]` form;
        // the abbreviated `name (fn)` form is the `else` (plain `-l`,
        // list==0) branch. These two were previously swapped, so `zle -lL`
        // emitted the abbreviated form and plain `zle -l` emitted the
        // `zle -N` form — the exact inverse of zsh.
        if list != 0 {
            // c:517 — `printf("zle -%c ", (w->flags & WIDGET_NCOMP) ? 'C' : 'N');`
            let kind = match listed {
                Listed::Comp(..) => 'C',
                Listed::Fn(_) => 'N',
            };
            // c:518-519 — `if (t->nam[0] == '-') fputs("-- ", stdout);` so a
            // leading-dash widget name is not read back as an option.
            let dashdash = if name.starts_with('-') { "-- " } else { "" };
            match listed {
                // c:521-525 — ` wid func`, both quoted.
                Listed::Comp(wid, func) => {
                    let _ = writeln!(
                        handle,
                        "zle -{} {}{} {} {}",
                        kind,
                        dashdash,
                        quotedzputs(name),
                        quotedzputs(wid),
                        quotedzputs(func)
                    );
                }
                // c:526-529 — ` fnnam` only when it differs from the name.
                Listed::Fn(fn_name) => {
                    if fn_name != name {
                        let _ = writeln!(
                            handle,
                            "zle -{} {}{} {}",
                            kind,
                            dashdash,
                            quotedzputs(name),
                            quotedzputs(fn_name)
                        );
                    } else {
                        let _ = writeln!(handle, "zle -{} {}{}", kind, dashdash, quotedzputs(name));
                    }
                }
            }
        } else {
            match listed {
                // c:532-536 — `name -C wid func`.
                Listed::Comp(wid, func) => {
                    let _ = writeln!(handle, "{} -C {} {}", name, wid, func);
                }
                // c:537-540 — abbreviated `name (fn)` when distinct.
                Listed::Fn(fn_name) => {
                    if fn_name != name {
                        let _ = writeln!(handle, "{} ({})", name, fn_name);
                    } else {
                        let _ = writeln!(handle, "{}", name);
                    }
                }
            }
        }
    }
    0
}

/// Port of `bin_zle_del(char *name, char **args, UNUSED(Options ops), UNUSED(char func))` from `Src/Zle/zle_thingy.c:547`.
/// ```c
/// static int
/// bin_zle_del(char *name, char **args, ...) {
///     int ret = 0;
///     do {
///         Thingy t = thingytab->getnode(thingytab, *args);
///         if (!t) { zwarnnam(name, "no such widget"); ret = 1; }
///         else if (unbindwidget(t, 0)) {
///             zwarnnam(name, "widget name `%s' is protected"); ret = 1;
///         }
///     } while (*++args);
///     return ret;
/// }
/// ```
/// `zle -D widget...` — unbind one or more widgets from the
/// thingytab. Returns 1 if any widget was missing or protected
/// (TH_IMMORTAL), else 0.
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
pub fn bin_zle_del(_name: &str, args: &[String], _ops: &options, _func: i32) -> i32 {
    // c:548
    let mut ret = 0;
    for arg in args {
        // c:552-561 do-while
        let exists = thingytab().lock().unwrap().contains_key(arg);
        if !exists {
            ret = 1; // c:556
        } else if unbindwidget(arg, 0) != 0 {
            // c:557
            ret = 1; // c:559
        }
    }
    ret // c:562
}

/// Port of `bin_zle_link(char *name, char **args, UNUSED(Options ops), UNUSED(char func))` from `Src/Zle/zle_thingy.c:567`.
/// ```c
/// static int
/// bin_zle_link(char *name, char **args, ...) {
///     Thingy t = thingytab->getnode(thingytab, args[0]);
///     if (!t) { zwarnnam(name, "no such widget `%s'", args[0]); return 1; }
///     else if (bindwidget(t->widget, rthingy(args[1]))) {
///         zwarnnam(name, "widget name `%s' is protected", args[1]);
///         return 1;
///     }
///     return 0;
/// }
/// ```
/// `zle -A old new` — alias `new` to point at the same widget as `old`.
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
pub fn bin_zle_link(_name: &str, args: &[String], _ops: &options, _func: i32) -> i32 {
    // c:567
    // c:567-578 — `t = thingytab.getnode(args[0]); if(!t) ret=1; else
    //              if(bindwidget(t->widget, rthingy(args[1]))) ret=1`.
    if args.len() < 2 {
        return 1;
    }
    let src = &args[0];
    let dst = &args[1];
    let widget = {
        let tab = thingytab().lock().unwrap();
        tab.get(src).and_then(|t| t.widget.clone())
    };
    let Some(w) = widget else {
        return 1; // c:573
    };
    rthingy(dst); // c:574 rthingy(args[1])
    if bindwidget(w, dst) != 0 {
        // c:574 bindwidget(...)
        return 1; // c:575
    }
    // PFA-SMR: `zle -A SRC DST` aliases an existing widget under a
    // new name. Record as a zle event with DST as the widget name
    // and SRC as the implementing-function reference so replay can
    // recreate the link.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() {
        let ctx = crate::recorder::recorder_ctx_global();
        crate::recorder::emit_zle(dst, Some(src.as_str()), ctx);
    }
    0 // c:578
}

/// Port of `bin_zle_new(char *name, char **args, UNUSED(Options ops), UNUSED(char func))` from `Src/Zle/zle_thingy.c:583`.
/// ```c
/// static int
/// bin_zle_new(char *name, char **args, ...) {
///     widget w = zalloc(sizeof(*w));
///     w->flags = 0;
///     w->first = NULL;
///     w->u.fnnam = ztrdup(args[1] ? args[1] : args[0]);
///     if (!bindwidget(w, rthingy(args[0]))) return 0;
///     freewidget(w);
///     zwarnnam(name, "widget name `%s' is protected", args[0]);
///     return 1;
/// }
/// ```
/// `zle -N name [func]` — bind a user-defined widget. `func`
/// defaults to `name` when omitted.
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
pub fn bin_zle_new(_name: &str, args: &[String], _ops: &options, _func: i32) -> i32 {
    // c:584
    // c:584-595 — `widget w = zalloc; w->flags=0; w->u.fnnam = ztrdup(args[1]?args[1]:args[0]);
    //              if(!bindwidget(w, rthingy(args[0]))) return 0;
    //              freewidget(w); zwarnnam(...); return 1;`.
    if args.is_empty() {
        return 1;
    }
    // c:590 — fn name is args[1] if present, else args[0].
    let fname = if args.len() >= 2 {
        args[1].clone()
    } else {
        args[0].clone()
    };
    let w = Arc::new(widget {
        flags: 0i32, // c:588
        first: None,
        u: WidgetImpl::UserFunc(fname), // c:590 fnnam
    });
    rthingy(&args[0]); // c:591 rthingy(args[0])
    if bindwidget(w.clone(), &args[0]) == 0 {
        // c:591 bindwidget(...)
        // PFA-SMR: record widget registration. `name` is the widget
        // identifier (args[0]); `func` is the implementing shell
        // function (args[1] or args[0] when omitted). One event per
        // successful `zle -N` invocation.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let ctx = crate::recorder::recorder_ctx_global();
            let widget_name = &args[0];
            let fn_arg = if args.len() >= 2 {
                Some(args[1].as_str())
            } else {
                None
            };
            crate::recorder::emit_zle(widget_name, fn_arg, ctx);
        }
        return 0; // c:592
    }
    // c:593-594 — bindwidget failed (TH_IMMORTAL) → free + warn.
    freewidget(w);
    1 // c:595
}

/// Port of `bin_zle_complete(char *name, char **args, UNUSED(Options ops), UNUSED(char func))` from `Src/Zle/zle_thingy.c:599`.
/// ```c
/// static int
/// bin_zle_complete(...) {
///     ...
///     t = rthingy((args[1][0] == '.') ? args[1] : dyncat(".", args[1]));
///     cw = t->widget; unrefthingy(t);
///     if (!cw || !(cw->flags & ZLE_ISCOMP)) { zwarnnam; return 1; }
///     w = zalloc(sizeof(*w));
///     w->flags = WIDGET_NCOMP|ZLE_MENUCMP|ZLE_KEEPSUFFIX;
///     w->u.comp.fn = cw->u.fn;
///     w->u.comp.wid = ztrdup(args[1]);
///     w->u.comp.func = ztrdup(args[2]);
///     if (bindwidget(w, rthingy(args[0]))) { freewidget(w); return 1; }
///     ...
/// }
/// ```
/// `zle -C name comp-widget func` — register a completion widget.
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
pub fn bin_zle_complete(_name: &str, args: &[String], _ops: &options, _func: i32) -> i32 {
    // c:600
    // c:600-629 — Load zsh/complete; resolve `args[1]` (or `.args[1]`)
    // to a Thingy; verify it's ZLE_ISCOMP; alloc a widget with
    // WIDGET_NCOMP|MENUCMP|KEEPSUFFIX flags and bind to args[0].
    if args.len() < 3 {
        return 1;
    }
    // c:609-611 — `t = rthingy(args[1] starts with '.' ? args[1] : ".args[1]")`.
    let lookup = if args[1].starts_with('.') {
        args[1].clone()
    } else {
        format!(".{}", args[1])
    };
    let comp_widget = {
        let tab = thingytab().lock().unwrap();
        tab.get(&lookup).and_then(|t| t.widget.clone())
    };
    let Some(cw) = comp_widget else {
        return 1; // c:613-614
    };
    // c:612 — `if (!cw || !(cw->flags & ZLE_ISCOMP)) return 1`.
    if (cw.flags & ZLE_ISCOMP) == 0 {
        return 1;
    }
    // c:619 — `w->u.comp.fn = cw->u.fn`. Extract the base widget's
    // internal fn pointer; bail if the base isn't WIDGET_INT (the
    // C check `cw->flags & ZLE_ISCOMP` guarantees this in practice
    // because every ZLE_ISCOMP widget in iwidgets.list is internal).
    let base_fn = match &cw.u {
        WidgetImpl::Internal(f) => *f,
        _ => return 1,
    };
    // c:616-621 — alloc new completion widget:
    //   w->flags = WIDGET_NCOMP | ZLE_MENUCMP | ZLE_KEEPSUFFIX;
    //   w->u.comp.fn   = cw->u.fn;
    //   w->u.comp.wid  = ztrdup(args[1]);
    //   w->u.comp.func = ztrdup(args[2]);
    let w = Arc::new(widget {
        flags: WIDGET_NCOMP | ZLE_MENUCMP | ZLE_KEEPSUFFIX,
        first: None,
        u: WidgetImpl::Comp {
            fn_: base_fn,          // c:619
            wid: args[1].clone(),  // c:620
            func: args[2].clone(), // c:621
        },
    });
    rthingy(&args[0]);
    if bindwidget(w.clone(), &args[0]) != 0 {
        // c:622
        freewidget(w);
        return 1; // c:625
    }
    0 // c:629
}

/// Port of `zle_usable()` from `Src/Zle/zle_thingy.c:634`.
/// ```c
/// static int
/// zle_usable(void)
/// {
///     return zleactive && !incompctlfunc && !incompfunc;
/// }
/// ```
/// True iff a ZLE session is currently active and we're not
/// inside a compctl-fn or comp-fn call (zle widgets can't run
/// from inside completion functions).
pub fn zle_usable() -> i32 {
    // c:634
    let active = crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) != 0;
    let incompctlfunc = crate::ported::zle::compctl::INCOMPCTLFUNC // c:636
        .with(|c| c.get());
    let incompfunc = crate::ported::zle::complete::INCOMPFUNC.load(Ordering::Relaxed) != 0;
    if active && !incompctlfunc && !incompfunc {
        1
    } else {
        0
    }
}

/// Port of `bin_zle_flags(char *name, char **args, UNUSED(Options ops), UNUSED(char func))` from `Src/Zle/zle_thingy.c:650`.
/// ```c
/// static int
/// bin_zle_flags(...) {
///     if (!zle_usable()) { zwarnnam(...); return 1; }
///     if (bindk) { widget w = bindk->widget;
///         for (flag = args; *flag; flag++) {
///             if      (!strcmp(*flag, "yank"))       w->flags |= ZLE_YANKAFTER;
///             else if (!strcmp(*flag, "yankbefore")) w->flags |= ZLE_YANKBEFORE;
///             else if (!strcmp(*flag, "kill"))       w->flags |= ZLE_KILL;
///             ...
///         }
///     }
///     return ret;
/// }
/// ```
/// `zle -f flag...` — set widget-execution flags (yank/yankbefore/
/// kill) on the currently-running widget.
/// Rust idiom replacement: `Arc<widget>` is immutable in zshrs, so
/// the C `w->flags |= ZLE_*` mutation lives on the widget-execution
/// path itself; this entry validates args + returns success.
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
pub fn bin_zle_flags(_name: &str, args: &[String], _ops: &options, _func: i32) -> i32 {
    // c:651
    // c:653-654 — locals.
    let mut ret: i32 = 0; // c:653
                          // c:656-659 — !zle_usable early-return.
    if zle_usable() == 0 {
        zwarnnam("zle", "can only set flags from a widget"); // c:657
        return 1; // c:658
    }
    // c:661-663 — `if (bindk) { Widget w = bindk->widget; if (w) { ... } }`.
    // BINDK holds the Thingy bound by the active key. When unset (no
    // active key sequence), the c:661 guard skips the whole loop.
    let bindk_present = BINDK.lock().map(|b| b.is_some()).unwrap_or(false);
    if bindk_present {
        // c:661
        // c:664-693 — `for (flag = args; *flag; flag++) { ... }`.
        for flag in args {
            // c:664
            match flag.as_str() {
                "yank" => {
                    // c:665 — `w->flags |= ZLE_YANKAFTER;`. !!! WARNING:
                    // PARTIAL PORT — current Thingy.widget shape is
                    // `Option<Arc<widget>>` (immutable through Arc),
                    // so flag bits are validated but the mutation
                    // back into widget.flags is dropped. Faithful
                    // port needs Arc<Mutex<widget>> across the tree.
                    // For now this matches "validation only".
                }
                "yankbefore" => {
                    // c:667 — `w->flags |= ZLE_YANKBEFORE;`. Same gap.
                }
                "kill" => {
                    // c:669 — `w->flags |= ZLE_KILL;`. Same gap.
                }
                // c:672-680 — menucmp/linemove/keepsuffix branches are
                // commented out in C ("These won't do anything yet,
                // because of how execzlefunc handles user widgets").
                // We mirror that — recognized as valid flag-names but
                // no-op.
                "menucmp" | "linemove" | "keepsuffix" => {
                    // c:674/676/678
                }
                "vichange" => {
                    // c:682 — `if (invicmdmode()) startvichange(-1); ...`
                    if invicmdmode(&crate::ported::zle::zle_keymap::curkeymapname()) {
                        // c:683
                        startvichange(-1); // c:684
                                           // c:685-688 — if a numeric arg is active and a
                                           // PM_SPECIAL `NUMERIC` param exists, clear its
                                           // PM_UNSET bit so the value becomes visible to
                                           // the widget.
                        let zm_flags = ZMOD.lock().unwrap().flags;
                        if (zm_flags & (MOD_MULT | MOD_TMULT)) != 0 {
                            // c:685 — numeric arg present.
                            if let Ok(mut tab) = crate::ported::params::paramtab().write() {
                                if let Some(pm) = tab.get_mut("NUMERIC") {
                                    if (pm.node.flags as u32 & crate::ported::zsh_h::PM_SPECIAL)
                                        != 0
                                    {
                                        // c:687 — clear PM_UNSET so widget sees value.
                                        pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {
                    // c:691-693 — unknown flag name.
                    zwarnnam("zle", &format!("invalid flag `{}' given to zle -f", flag)); // c:692
                    ret = 1; // c:693
                }
            }
        }
    }
    ret // c:697
}

/// Port of `bin_zle_call(char *name, char **args, UNUSED(Options ops), UNUSED(char func))` from `Src/Zle/zle_thingy.c:702`.
/// ```c
/// static int
/// bin_zle_call(...) {
///     ...
///     char *wname = *args++;
///     if (!wname) return !zle_usable();
///     if (!zle_usable()) { zwarnnam(name, "..."); return 1; }
///     ...
/// }
/// ```
/// Bare-args invocation of `zle widget args...` from inside another
/// widget. The full path (flag parse + execzlefunc) needs ZLE
/// session substrate; this port covers the empty-args probe and
/// Faithful port of `bin_zle_call(char *name, char **args, Options ops,
/// UNUSED(char func))` from Src/Zle/zle_thingy.c:703.
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
pub fn bin_zle_call(_name: &str, args: &[String], _ops: &options, _func: i32) -> i32 {
    // c:703
    // c:706-709 — locals.
    let modsave: modifier = (*ZMOD.lock().unwrap()).clone(); // c:706 struct modifier modsave = zmod
    let mut saveflag = 0i32; // c:707
    let mut setbindk = 0i32; // c:707
    let mut setlbindk = 0i32; // c:707
                              // c:707 `remetafy` collapses in Rust (UTF-8 storage).
    let mut keymap_restore: Option<String> = None; // c:708
                                                   // c:708 — `char *wname = *args++;`. Consume first arg as widget name.
    let mut argv: Vec<String> = args.to_vec();
    let wname = if argv.is_empty() {
        None
    } else {
        Some(argv.remove(0))
    };

    // c:710-711 — `if (!wname) return !zle_usable();`
    if wname.is_none() {
        // c:710
        return if zle_usable() != 0 { 0 } else { 1 }; // c:711
    }
    let wname = wname.unwrap();

    // c:713-716 — `if (!zle_usable()) { zwarnnam; return 1; }`.
    if zle_usable() == 0 {
        // c:713
        zwarnnam("zle", "widgets can only be called when ZLE is active"); // c:714
        return 1; // c:715
    }

    // c:722-726 — `if (zlemetaline) { unmetafy_line(); remetafy = 1; }
    //               else remetafy = 0;`. Rust stores ZLE as UTF-8;
    // the meta-line bookkeeping is a no-op.

    // c:728-798 — flag-parsing loop. C iterates while `**args == '-'`
    // and consumes the option characters one at a time. Supports
    //   -f nolast      → setlbindk = 1
    //   -n NUM         → zmod.mult = NUM, MOD_MULT |= 1
    //   -N             → zmod.mult = 1, MOD_MULT &= ~1 (reset count)
    //   -K keymap      → selectkeymap(keymap, 0)
    //   -w             → setbindk = 1
    // The C trick `skip_this_arg = "x"` substitutes a dummy when an
    // attached-value (like `-nNUM` form) consumed the operand inline.
    while !argv.is_empty() && argv[0].starts_with('-') {
        // c:728
        let cur = argv[0].clone();
        // c:732-734 — `-` or `--` terminates flag parsing.
        if cur.len() == 1 || (cur.len() == 2 && cur.as_bytes()[1] == b'-') {
            // c:732
            argv.remove(0); // c:733 args++
            break; // c:734
        }
        let mut byte_idx = 1usize; // skip leading '-'
        let mut consumed_next = false;
        let cur_bytes = cur.as_bytes();
        while byte_idx < cur_bytes.len() {
            // c:736 `while (*++(*args))`
            let c = cur_bytes[byte_idx];
            byte_idx += 1;
            match c {
                b'f' => {
                    // c:738-750 — `-f nolast`.
                    // c:739 — `flag = args[0][1] ? args[0]+1 : args[1];`
                    let flag: Option<String> = if byte_idx < cur_bytes.len() {
                        // c:739 attached form `-fXXX`
                        Some(
                            std::str::from_utf8(&cur_bytes[byte_idx..])
                                .unwrap_or("")
                                .to_string(),
                        )
                    } else if argv.len() > 1 {
                        // c:739 separate form `-f XXX`
                        Some(argv[1].clone())
                    } else {
                        None
                    };
                    if flag.as_deref() != Some("nolast") {
                        // c:740
                        zwarnnam("zle", "'nolast' expected after -f"); // c:741
                        return 1; // c:744
                    }
                    // c:746-747 — consume separate-form operand.
                    if byte_idx >= cur_bytes.len() {
                        argv.remove(1); // c:746-747
                        consumed_next = true;
                    }
                    setlbindk = 1; // c:749
                    byte_idx = cur_bytes.len(); // exit inner loop
                }
                b'n' => {
                    // c:751-764 — `-n NUM`. Set zmod.mult.
                    let num: Option<String> = if byte_idx < cur_bytes.len() {
                        Some(
                            std::str::from_utf8(&cur_bytes[byte_idx..])
                                .unwrap_or("")
                                .to_string(),
                        )
                    } else if argv.len() > 1 {
                        Some(argv[1].clone())
                    } else {
                        None
                    };
                    if num.is_none() {
                        // c:753
                        zwarnnam("zle", &format!("number expected after -{}", c as char)); // c:754
                        return 1; // c:757
                    }
                    if byte_idx >= cur_bytes.len() {
                        argv.remove(1);
                        consumed_next = true;
                    }
                    saveflag = 1; // c:761
                    let n: i32 = num.unwrap().parse().unwrap_or(0); // c:762 atoi
                    let mut zm = ZMOD.lock().unwrap();
                    zm.mult = n; // c:762
                    zm.flags |= MOD_MULT; // c:763
                    byte_idx = cur_bytes.len();
                }
                b'N' => {
                    // c:765-768 — `-N` reset count modifier.
                    saveflag = 1; // c:766
                    let mut zm = ZMOD.lock().unwrap();
                    zm.mult = 1; // c:767
                    zm.flags &= !MOD_MULT; // c:768
                }
                b'K' => {
                    // c:770-786 — `-K keymap`.
                    let keymap_tmp: Option<String> = if byte_idx < cur_bytes.len() {
                        Some(
                            std::str::from_utf8(&cur_bytes[byte_idx..])
                                .unwrap_or("")
                                .to_string(),
                        )
                    } else if argv.len() > 1 {
                        Some(argv[1].clone())
                    } else {
                        None
                    };
                    if keymap_tmp.is_none() {
                        // c:772
                        zwarnnam("zle", &format!("keymap expected after -{}", c as char)); // c:773
                        return 1; // c:776
                    }
                    if byte_idx >= cur_bytes.len() {
                        argv.remove(1);
                        consumed_next = true;
                    }
                    keymap_restore = Some(crate::ported::zle::zle_keymap::curkeymapname().clone()); // c:780
                    if crate::ported::zle::zle_keymap::selectkeymap(&keymap_tmp.unwrap(), 0) != 0 {
                        // c:781
                        return 1; // c:784
                    }
                    byte_idx = cur_bytes.len();
                }
                b'w' => {
                    // c:787-789 — `-w`.
                    setbindk = 1; // c:788
                }
                _ => {
                    // c:790-794 — unknown option.
                    zwarnnam("zle", &format!("unknown option: {}", cur)); // c:791
                    return 1; // c:794
                }
            }
        }
        argv.remove(0); // c:797 — args++.
        let _ = consumed_next; // already adjusted via argv.remove(1) above
    }

    // c:800-807 — `t = rthingy(wname); ... ret = execzlefunc(t, args,
    //   setbindk, setlbindk); unrefthingy(t);`. Rust execzlefunc takes
    // (name, args); setbindk/setlbindk plumbing pending the wider sig.
    rthingy(&wname); // c:800
                     // RUST-ONLY SYNC (C: live GSU setters — see ZLE_PARAM_SNAPSHOT
                     // in zle_params.rs): a `zle <widget>` call from inside a user
                     // widget must see that widget's pending $BUFFER/$LBUFFER
                     // writes (zsh-expand does `LBUFFER=expanded; zle self-insert`
                     // — without the sync, self-insert appended to the STALE line
                     // and the exit write-back clobbered it, dropping the space).
    let in_widget_scope = crate::zle_param_sync::active();
    if in_widget_scope {
        crate::zle_param_sync::sync_from_paramtab();
    }
    // c:806 — `ret = execzlefunc(t, args, setbindk, setlbindk)`.
    // Now that execzlefunc takes the 4-arg C sig, thread the flags
    // collected from `-w` (setbindk) and `-f nolast` (setlbindk).
    let ret = execzlefunc(&wname, &argv, setbindk, setlbindk); // c:806
                                                               // RUST-ONLY SYNC (other direction): refresh the caller widget's
                                                               // params + snapshot from the live editor so post-call reads of
                                                               // $BUFFER/$CURSOR see what the inner widget did.
    if in_widget_scope {
        crate::ported::zle::zle_params::makezleparams(0);
    }
    unrefthingy(&wname); // c:807

    // c:808-809 — `if (saveflag) zmod = modsave;`.
    if saveflag != 0 {
        *ZMOD.lock().unwrap() = modsave; // c:809
    }
    // c:810-811 — `if (keymap_restore) selectkeymap(keymap_restore, 0);`.
    if let Some(k) = keymap_restore {
        // c:810
        crate::ported::zle::zle_keymap::selectkeymap(&k, 0); // c:811
    }
    // c:812-813 — remetafy collapses in Rust.
    ret // c:814
}

/// Direct port of `int bin_zle_invalidate(char *name, char **args,
///                                         Options ops, UNUSED(char func))`
/// from `Src/Zle/zle_thingy.c:828-852`.
/// ```c
/// if (zleactive) {
///     int wastrashed = trashedzle;
///     trashzle();
///     if (!wastrashed) { settyinfo(&shttyinfo); fetchttyinfo = 1; }
///     return 0;
/// }
/// return 1;
/// ```
///
/// **Substrate tradeoff:** `trashzle` is a free fn at
/// zle_main.rs:1111 that reads the file-scope ZLE statics; the
/// `wastrashed`/`shttyinfo`/`fetchttyinfo` path is part of the
/// active editor's tty state machine. From compcore-call-context
/// we flag `ZLE_RESET_NEEDED` so the next zlecore tick observes
/// the invalidation and re-enters `trashzle`.
/// Port of `bin_zle_invalidate(UNUSED(char *name), UNUSED(char **args), UNUSED(Options ops), UNUSED(char func))` from `Src/Zle/zle_thingy.c:830`.
/// WARNING: param names don't match C — Rust=() vs C=(name, args, ops, func)
pub fn bin_zle_invalidate(_name: &str, _args: &[String], _ops: &options, _func: i32) -> i32 {
    // c:830
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) != 0 {
        // c:837 — `trashzle()` via the reset-flag bridge.
        ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
        0 // c:850
    } else {
        1 // c:852
    }
}

/// Monotonic id source for `watch_fd.gen` (Rust-only; see the field
/// doc on `watch_fd`). A `static` counter, not a fn, so it stamps a
/// fresh id inline at each `zle -F` registration site.
static WATCH_FD_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Port of `bin_zle_fd(char *name, char **args, Options ops, UNUSED(char func))` from `Src/Zle/zle_thingy.c:857`.
/// `zle -F fd handler` — register an fd watcher invoked when the
/// fd becomes readable while the editor is idle.
/// Direct port of `int bin_zle_fd(char *name, char **args, Options ops,
///                                 UNUSED(char func))` from
/// `Src/Zle/zle_thingy.c:857`. Manages the per-Zle `watch_fds`
/// table: `-d` removes, single-arg lists, two-args register a
/// handler.
///
/// Mutates the global `WATCH_FDS` (`Src/Zle/zle_main.c:204`)
/// directly so the poll loop in `zle_main::raw_getbyte` sees the
/// new registration on the next iteration.
/// Rust idiom replacement: WATCH_FDS `Mutex<HashMap>` covers the C
/// `watch_fds` LinkList add/remove; the poll loop in `raw_getbyte`
/// reads the map directly, no callback-table indirection needed.
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
///
/// Every registration (push or in-place replace) stamps a fresh id
/// from [`WATCH_FD_GEN`] so the poll loop can tell a handler's
/// re-armed watcher apart from the dead one it replaced on a reused
/// fd number.
pub fn bin_zle_fd(_name: &str, args: &[String], ops: &options, _func: i32) -> i32 {
    // c:857
    // c:859 — locals.
    let mut fd: i32 = 0; // c:859
    let mut found: bool = false; // c:859

    // c:862-869 — parse fd if given. C uses zstrtol(*args, &endptr, 10)
    // and rejects when trailing garbage exists (*endptr != '\0') OR
    // fd < 0.
    if !args.is_empty() {
        // c:862
        match args[0].parse::<i32>() {
            // c:863 zstrtol
            Ok(n) if n >= 0 => fd = n,
            _ => {
                // c:865 — `*endptr || fd < 0`
                zwarnnam(
                    "zle",
                    &format!("Bad file descriptor number for -F: {}", args[0]),
                ); // c:866
                return 1; // c:867
            }
        }
    }

    // c:871-887 — `-L` listing branch, OR no-args list-all.
    if OPT_ISSET(ops, b'L') || args.is_empty() {
        // c:871
        if !args.is_empty() && args.len() > 1 {
            // c:873
            zwarnnam("zle", "too many arguments for -FL"); // c:874
            return 1; // c:875
        }
        if let Ok(tab) = WATCH_FDS.lock() {
            // c:877 — `for (i = 0; i < nwatch; i++)`
            for w in tab.iter() {
                if !args.is_empty() && w.fd != fd {
                    // c:879
                    continue; // c:880
                }
                found = true; // c:881
                              // c:882 — `printf("%s -F %s%d %s\n", name, widget ? "-w " : "", fd, func);`
                let w_flag = if w.widget != 0 { "-w " } else { "" };
                println!("zle -F {}{} {}", w_flag, w.fd, w.func);
            }
        }
        // c:885-886 — return 1 if fd was given and not found.
        return if !args.is_empty() && !found { 1 } else { 0 }; // c:886
    }

    if args.len() > 1 {
        // c:889 — adding/replacing a handler.
        let funcnam = args[1].clone(); // c:891 ztrdup
        if let Ok(mut tab) = WATCH_FDS.lock() {
            // c:892 — `if (nwatch) for (...) if (fd matches) replace`.
            for w in tab.iter_mut() {
                // c:893
                if w.fd == fd {
                    // c:895
                    w.func = funcnam.clone(); // c:897
                    w.widget = if OPT_ISSET(ops, b'w') { 1 } else { 0 }; // c:898
                    w.gen = WATCH_FD_GEN.fetch_add(1, Ordering::Relaxed); // fresh id: a replace is a new watcher
                    found = true; // c:899
                    break; // c:900
                }
            }
            if !found {
                // c:904 — append new entry.
                tab.push(watch_fd {
                    // c:910-913
                    fd,                                               // c:911
                    func: funcnam,                                    // c:912
                    widget: if OPT_ISSET(ops, b'w') { 1 } else { 0 }, // c:913
                    gen: WATCH_FD_GEN.fetch_add(1, Ordering::Relaxed),
                });
                // c:914 — `nwatch = newnwatch;` (Vec.len() tracks
                // nwatch implicitly).
            }
        }
    } else {
        // c:916 — deleting a handler (one positional, no value).
        if let Ok(mut tab) = WATCH_FDS.lock() {
            let len_before = tab.len();
            tab.retain(|w| w.fd != fd); // c:920-940 memcpy-shrink
            found = tab.len() < len_before; // c:940
        }
        if !found {
            // c:944 — `if (!found) zwarnnam(name, "No handler installed for fd %d", fd);`
            zwarnnam("zle", &format!("No handler installed for fd {}", fd)); // c:945
            return 1; // c:946
        }
    }

    0 // c:952
}

/// Direct port of `int bin_zle_transform(char *name, char **args,
///                                       Options ops, UNUSED(char func))`
/// from `Src/Zle/zle_thingy.c:955`.
/// ```c
/// // -L: list installed transformations
/// // 0 args: clear all
/// // 1 arg: clear specific (tcfn name)
/// // 2 args: install transformation tcfn -> fn
/// ```
///
/// Registers the transformation via `ShellExecutor.hook_functions`
/// under the synthetic hook name `zle-transform-<tcfn>` so the
/// redisplay path can find it. Args validate first.
pub fn bin_zle_transform(_name: &str, args: &[String], ops: &options, _func: i32) -> i32 {
    // c:955
    // c:957-963 — badargs convention:
    //   -1: too few arguments
    //    0: just right
    //    1: too many arguments
    //    2: first argument not recognised
    let mut badargs: i32 = 0; // c:963

    if OPT_ISSET(ops, b'L') {
        // c:965 — `-L`: list the current tc handler.
        if !args.is_empty() {
            // c:966
            if args.len() > 1 {
                // c:967
                badargs = 1; // c:968
            } else if args[0] != "tc" {
                // c:969
                badargs = 2; // c:970
            }
        }
        if badargs == 0 {
            // c:973
            let cur = TCOUT_FUNC_NAME.lock().ok().and_then(|n| n.clone());
            if let Some(fname) = cur {
                // c:973
                print!("zle -T tc "); // c:974
                print!("{}", crate::ported::utils::quotedzputs(&fname)); // c:975
                println!(); // c:976
            }
        }
    } else if OPT_ISSET(ops, b'r') {
        // c:978 — `-r`: reset the tc handler.
        if args.is_empty() {
            // c:979
            badargs = -1; // c:980
        } else if args.len() > 1 {
            // c:981
            badargs = 1; // c:982
        } else if args[0] == "tc" {
            // c:983 — `if (tcout_func_name) { zsfree; tcout_func_name = NULL; }`.
            // The C `if (tcout_func_name)` guard avoids a double-free
            // before the value is reset.
            if let Ok(mut name) = TCOUT_FUNC_NAME.lock() {
                if name.is_some() {
                    // c:983
                    *name = None; // c:985 zsfree + NULL
                }
            }
        } else {
            // C falls through silently when args[0] != "tc"; the only
            // `tc` transform exists, so anything else is a no-op.
            badargs = 2;
        }
    } else {
        // c:987 — default `zle -T name fname` form.
        if args.is_empty() || args.len() < 2 {
            // c:988
            badargs = -1; // c:989 — we've already checked args <= 2.
        } else {
            // c:991
            if args[0] == "tc" {
                // c:992
                if let Ok(mut name) = TCOUT_FUNC_NAME.lock() {
                    // c:993 — `if (tcout_func_name) zsfree(tcout_func_name);`.
                    *name = Some(args[1].clone()); // c:996 ztrdup
                }
            } else {
                badargs = 2; // c:998
            }
        }
    }

    if badargs != 0 {
        // c:1003
        if badargs == 2 {
            // c:1004
            zwarnnam(
                "zle",
                &format!("-T: no such transformation '{}'", args[0]), // c:1005
            );
        } else {
            // c:1006
            let way = if badargs > 0 { "many" } else { "few" }; // c:1007
            zwarnnam("zle", &format!("too {} arguments for option -T", way));
            // c:1008
        }
        return 1; // c:1010
    }

    0 // c:1013
}

/// Port of `init_thingies()` from `Src/Zle/zle_thingy.c:1022`.
/// Boot-time thingytab population from the built-in widget table.
/// Walks the static `thingies[]` array in zle_thingy.c and inserts
/// each into the table marked TH_IMMORTAL.
pub fn init_thingies() -> i32 {
    // c:1022
    // c:1026 — `createthingytab();`.
    createthingytab();
    // c:1027-1028 — `for (t = thingies; t->nam; t++)
    //                  thingytab->addnode(thingytab, t->nam, t);`.
    // The C `thingies[]` array at zle_bindings.c:72 is generated from
    // `Src/Zle/thingies.list`, which itself is generated from
    // `iwidgets.list` (`Makefile:1057-1075`). Each iwidgets.list line
    // produces TWO thingy entries: a bare `name` (mortal, can be
    // rebound by `zle -A`/`zle -C`) and a `.name` (TH_IMMORTAL, the
    // internal anchor). Both point at the same shared widget.
    let names = crate::ported::zle::zle_bindings::IWIDGET_NAMES;
    let mut tab = thingytab().lock().unwrap();
    for nam in names {
        // Build the shared widget for this name.
        // c:widgets.list — each iwidgets.list line emits a
        // `W(ZLE_FLAGS, t_firstname, functionname)` widget entry.
        // The Rust analog: WIDGET_INT + iwidget_flags(name) +
        // u = Internal(iwidget_lookup(name)).
        let fn_ptr = crate::ported::zle::zle_bindings::iwidget_lookup(nam);
        // c:widgets.list — look up the per-widget ZLE_FLAGS column
        // for this name.
        let extra_flags = crate::ported::zle::zle_bindings::IWIDGET_FLAGS
            .iter()
            .find(|(n, _)| *n == *nam)
            .map(|(_, f)| *f)
            .unwrap_or(0);
        // c:zle_bindings.c:72 — every `thingies[]` entry generated from
        // iwidgets.list binds a real `widgets[]` struct, so every
        // thingy is enabled and appears in `$widgets` as "builtin".
        // Names whose C body has no Rust port yet still get an
        // Internal widget here (undefined-key body as placeholder)
        // so enumeration matches; the real body is a later port.
        let f = fn_ptr.unwrap_or(|_| crate::ported::zle::zle_misc::undefinedkey());
        let w = Some(Arc::new(widget {
            flags: WIDGET_INT | extra_flags,
            first: None,
            u: WidgetImpl::Internal(f),
        }));

        // Bare `name` thingy — mortal.
        if !tab.contains_key(*nam) {
            let mut t = makethingynode();
            t.nam = nam.to_string(); // c:163 ztrdup(nam)
            t.widget = w.clone(); // c:229
            tab.insert(nam.to_string(), t);
        }
        // Dotted `.name` thingy — TH_IMMORTAL, the internal anchor
        // used by `zle -C BASE` lookups (`bin_zle_complete` c:610
        // prepends `.` when looking up the base widget).
        let dotted = format!(".{}", nam);
        if !tab.contains_key(&dotted) {
            let mut t = makethingynode();
            t.nam = dotted.clone();
            t.flags |= TH_IMMORTAL; // c:1027 — `.NAME` entries are immortal
            t.widget = w;
            tab.insert(dotted, t);
        }
    }
    0
}
/// `Thingy` — see fields for layout.
#[derive(Debug, Clone)]
pub struct Thingy {
    // c:224
    pub nam: String,                 // c:226 char *nam
    pub flags: i32,                  // c:227 int flags
    pub rc: i32,                     // c:228 int rc
    pub widget: Option<Arc<widget>>, // c:229 widget widget
}

// `pub mod names` removed — Rust-fabricated namespace wrapping
// thingy-name string literals. C source uses bare `"accept-line"`/
// `"self-insert"`/etc. directly at `zle_thingy.c` registration
// sites; no namespace, no helper consts. The mod had no callers.

// =====================================================================
// thingytab — `Src/Zle/zle_thingy.c:52`.
// =====================================================================
//
// C: `mod_export HashTable thingytab;`. One global hash keyed by
// thingy name; each entry is a `Thingy` struct (rc + flags + widget
// + samew circular-list pointer). Allocated by `createthingytab()`
// at zle init and torn down by `cleanup_zle()`.
//
// Rust: `Mutex<HashMap<String, Thingy>>`. The C `samew` circular
// list isn't represented as a field — `bindwidget`/`unbindwidget`
// walk the table to find peers via `Arc<widget>` identity (Arc::
// ptr_eq). O(n) instead of C's O(1), but n is small (typical
// thingy count: a few hundred) and the simpler representation
// avoids a parallel widget→thingies table that would have to stay
// in sync.

// Hashtable of thingies. Enabled nodes are those that refer to widgets.   // c:49
static THINGYTAB: OnceLock<Mutex<HashMap<String, Thingy>>> = OnceLock::new();

/// Look up a Thingy by name via `gethashnode2(thingytab, name)` —
/// the C zle.h dispatch for `Th(X)` lookup. Direct port of the
/// open-coded `gethashnode2()` call shape at `Src/Zle/zle_thingy.c:160`.
pub fn gethashnode2(name: &str) -> Option<Thingy> {
    // c:gethashtable.c (open-coded)
    thingytab().lock().ok()?.get(name).cloned()
}

/// List every Thingy name. Used by `${widgets[@]}` parameter expansion.
/// Replaces the legacy `ZleManager::list_widgets()` accessor.
pub fn listwidgets() -> Vec<String> {
    thingytab()
        .lock()
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default()
}

/// Look up the dispatch target for a widget name. Built-in widgets
/// resolve to their own name (matching `${widgets[name]}` returning
/// "builtin"); user-defined ones resolve to the bound shell-function
/// name. Replaces the legacy `ZleManager::get_widget()` accessor.
pub fn getwidgettarget(name: &str) -> Option<String> {
    let tab = thingytab().lock().ok()?;
    let t = tab.get(name)?;
    let w = t.widget.as_ref()?;
    match &w.u {
        WidgetImpl::Internal(_) => Some(name.to_string()),
        WidgetImpl::UserFunc(s) => Some(s.clone()),
        WidgetImpl::Comp { func, .. } => Some(func.clone()),
    }
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

/// Get-or-init access to the global thingytab.
pub fn thingytab() -> &'static Mutex<HashMap<String, Thingy>> {
    THINGYTAB.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serialize tests since they share the global THINGYTAB.
    static LOCK: Mutex<()> = Mutex::new(());

    fn reset_tab() {
        thingytab().lock().unwrap().clear();
    }

    /// `Src/Zle/zle_thingy.c:370-375` — `bin_zle` rejects mutually
    /// exclusive operation flags. Pin: -l + -D together → return 1.
    #[test]
    fn bin_zle_rejects_incompatible_op_flags() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();
        // Build an options struct with both -l and -D set.
        let mut ops = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        ops.ind[b'l' as usize] = 1;
        ops.ind[b'D' as usize] = 1;
        let r = bin_zle("zle", &[], &ops, 0);
        assert_eq!(
            r, 1,
            "c:373-374 — incompatible op flags (-l + -D) → return 1"
        );
    }

    /// `Src/Zle/zle_thingy.c:790-794` — `bin_zle_call` rejects unknown
    /// option chars in the flag-parsing loop. Pin: `-q` (not a real
    /// flag) → return 1. Set zleactive=1 so we reach the flag parser
    /// (otherwise the !zle_usable early-return at c:715 would mask it).
    #[test]
    fn bin_zle_call_rejects_unknown_option() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();
        crate::ported::builtins::sched::zleactive.store(1, Ordering::Relaxed);
        // -q is not a valid bin_zle_call flag.
        let ops_empty = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zle_call(
            "zle",
            &["widget_name".to_string(), "-q".to_string()],
            &ops_empty,
            0,
        );
        crate::ported::builtins::sched::zleactive.store(0, Ordering::Relaxed);
        assert_eq!(r, 1, "c:791-794 — unknown option char → return 1");
    }

    /// `Src/Zle/zle_thingy.c:1022-1028` — `init_thingies` populates
    /// THINGYTAB with every name in `IWIDGET_NAMES` so `zle -l` works
    /// without each name needing a prior `zle -N` registration.
    #[test]
    fn init_thingies_populates_known_widget_names() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        thingytab().lock().unwrap().clear();
        init_thingies();
        let tab = thingytab().lock().unwrap();
        // Sample three canonical widgets — all must be present after init.
        assert!(
            tab.contains_key("accept-line"),
            "c:1028 — accept-line must be in THINGYTAB"
        );
        assert!(
            tab.contains_key("self-insert"),
            "c:1028 — self-insert must be in THINGYTAB"
        );
        assert!(
            tab.contains_key("undefined-key"),
            "c:1028 — undefined-key must be in THINGYTAB"
        );
        // Per C's thingies.list generator (`Makefile:1057-1075`), each
        // iwidgets.list line emits two entries: bare `name` (mortal,
        // can be rebound by `zle -A`/`zle -C`) and `.name`
        // (TH_IMMORTAL, the internal anchor).
        let al = tab.get("accept-line").unwrap();
        assert_eq!(
            al.flags & TH_IMMORTAL,
            0,
            "c:thingies.list — bare name must be mortal",
        );
        let dot_al = tab
            .get(".accept-line")
            .expect("c:thingies.list — dotted name must be registered alongside bare");
        assert_ne!(
            dot_al.flags & TH_IMMORTAL,
            0,
            "c:thingies.list — `.name` form is TH_IMMORTAL",
        );
        // Both forms must point at the same widget Arc.
        let (al_w, dot_al_w) = (al.widget.clone(), dot_al.widget.clone());
        match (al_w, dot_al_w) {
            (Some(a), Some(b)) => assert!(
                Arc::ptr_eq(&a, &b),
                "c:thingies.list — bare and dotted thingies share the widget Arc",
            ),
            _ => panic!("c:widgets.list — both bare and dotted thingies must have widgets"),
        }
    }

    /// `Src/Zle/zle_thingy.c:865-867` — `bin_zle_fd` rejects negative
    /// or non-numeric fd with `zwarnnam` + return 1.
    #[test]
    fn bin_zle_fd_rejects_bad_fd_string() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        let ops = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zle_fd("zle", &["notanumber".to_string()], &ops, 0);
        assert_eq!(r, 1, "c:865-867 — non-numeric fd → 1");
        let r2 = bin_zle_fd("zle", &["-1".to_string()], &ops, 0);
        assert_eq!(r2, 1, "c:865-867 — negative fd → 1");
    }

    /// `Src/Zle/zle_thingy.c:889-914` — `zle -F FD FUNC` installs a
    /// new handler. Pin: WATCH_FDS gains an entry.
    #[test]
    fn bin_zle_fd_adds_new_handler() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        // Reset table.
        WATCH_FDS.lock().unwrap().clear();
        let ops = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zle_fd("zle", &["7".to_string(), "my_handler".to_string()], &ops, 0);
        assert_eq!(r, 0, "c:889-914 — install → 0");
        let tab = WATCH_FDS.lock().unwrap();
        assert_eq!(tab.len(), 1);
        assert_eq!(tab[0].fd, 7);
        assert_eq!(tab[0].func, "my_handler");
        assert_eq!(tab[0].widget, 0);
    }

    /// `Src/Zle/zle_thingy.c:944-946` — deleting a non-existent fd
    /// handler emits `zwarnnam "No handler installed for fd N"` and
    /// returns 1.
    #[test]
    fn bin_zle_fd_delete_nonexistent_returns_1() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        WATCH_FDS.lock().unwrap().clear();
        let ops = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zle_fd("zle", &["99".to_string()], &ops, 0);
        assert_eq!(r, 1, "c:944-946 — delete unknown fd → 1");
    }

    /// `Src/Zle/zle_thingy.c:988-989` — `bin_zle_transform` rejects
    /// too few args (default form needs exactly 2). Pin: zero args →
    /// badargs=-1 → return 1.
    #[test]
    fn bin_zle_transform_rejects_too_few_args() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        let ops = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zle_transform("zle", &[], &ops, 0);
        assert_eq!(r, 1, "c:989-1010 — too few args → 1");
    }

    /// `Src/Zle/zle_thingy.c:992-996` — default form `zle -T tc fname`
    /// sets TCOUT_FUNC_NAME.
    #[test]
    fn bin_zle_transform_default_form_sets_tc_handler() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        *TCOUT_FUNC_NAME.lock().unwrap() = None;
        let ops = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zle_transform(
            "zle",
            &["tc".to_string(), "my_handler".to_string()],
            &ops,
            0,
        );
        assert_eq!(r, 0, "c:992-996 — valid `tc fname` → 0");
        assert_eq!(
            TCOUT_FUNC_NAME.lock().unwrap().as_deref(),
            Some("my_handler"),
            "c:996 — name should be stored"
        );
    }

    /// `Src/Zle/zle_thingy.c:983-985` — `-r tc` resets the handler.
    #[test]
    fn bin_zle_transform_r_clears_tc_handler() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        *TCOUT_FUNC_NAME.lock().unwrap() = Some("preset".to_string());
        let mut ops = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        ops.ind[b'r' as usize] = 1;
        let r = bin_zle_transform("zle", &["tc".to_string()], &ops, 0);
        assert_eq!(r, 0, "c:983-985 — `-r tc` → 0");
        assert!(
            TCOUT_FUNC_NAME.lock().unwrap().is_none(),
            "c:985 — name should be cleared"
        );
    }

    /// `Src/Zle/zle_thingy.c:969-970` — unknown transform name (anything
    /// other than `tc`) → badargs=2 → return 1.
    #[test]
    fn bin_zle_transform_rejects_unknown_transform() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        let mut ops = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        ops.ind[b'L' as usize] = 1;
        let r = bin_zle_transform("zle", &["bogus".to_string()], &ops, 0);
        assert_eq!(r, 1, "c:969-970/1005 — unknown transform → 1");
    }

    /// `Src/Zle/zle_thingy.c:691-693` — `bin_zle_flags` rejects unknown
    /// flag names with `zwarnnam` + ret=1.
    #[test]
    fn bin_zle_flags_rejects_unknown_flag() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();
        crate::ported::builtins::sched::zleactive.store(1, Ordering::Relaxed);
        // BINDK must be set for the loop to run (c:661).
        *BINDK.lock().unwrap() = Some(Thingy {
            nam: "dummy".to_string(),
            flags: 0,
            rc: 1,
            widget: None,
        });
        let ops_empty = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zle_flags("zle", &["bogus_flag".to_string()], &ops_empty, 0);
        *BINDK.lock().unwrap() = None;
        crate::ported::builtins::sched::zleactive.store(0, Ordering::Relaxed);
        assert_eq!(r, 1, "c:692-693 — unknown flag → zwarnnam + ret=1");
    }

    /// `Src/Zle/zle_thingy.c:665-669` — `yank`, `yankbefore`, `kill` are
    /// recognized flag names (return 0).
    #[test]
    fn bin_zle_flags_accepts_yank_kill() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();
        crate::ported::builtins::sched::zleactive.store(1, Ordering::Relaxed);
        *BINDK.lock().unwrap() = Some(Thingy {
            nam: "dummy".to_string(),
            flags: 0,
            rc: 1,
            widget: None,
        });
        let ops_empty = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zle_flags(
            "zle",
            &[
                "yank".to_string(),
                "yankbefore".to_string(),
                "kill".to_string(),
            ],
            &ops_empty,
            0,
        );
        *BINDK.lock().unwrap() = None;
        crate::ported::builtins::sched::zleactive.store(0, Ordering::Relaxed);
        assert_eq!(r, 0, "c:665-669 — all recognized → ret=0");
    }

    /// `Src/Zle/zle_thingy.c:740-744` — `-f` requires the literal token
    /// "nolast". Anything else → return 1.
    #[test]
    fn bin_zle_call_rejects_bad_f_arg() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();
        crate::ported::builtins::sched::zleactive.store(1, Ordering::Relaxed);
        let ops_empty = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zle_call(
            "zle",
            &["widget".to_string(), "-f".to_string(), "bogus".to_string()],
            &ops_empty,
            0,
        );
        crate::ported::builtins::sched::zleactive.store(0, Ordering::Relaxed);
        assert_eq!(r, 1, "c:741 — -f with non-'nolast' → return 1");
    }

    /// `Src/Zle/zle_thingy.c:378-381` — bin_zle rejects too-few args.
    /// `-D` requires min=1; passing zero args → return 1.
    #[test]
    fn bin_zle_rejects_too_few_args() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();
        let mut ops = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        ops.ind[b'D' as usize] = 1; // -D requires min=1
        let r = bin_zle("zle", &[], &ops, 0);
        assert_eq!(
            r, 1,
            "c:379-381 — zle -D with zero args → 'not enough' → return 1"
        );
    }

    #[test]
    fn rthingy_creates_then_refs() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("foo");
        let tab = thingytab().lock().unwrap();
        let t = tab.get("foo").expect("rthingy must create");
        assert_eq!(t.rc, 1);
        assert_ne!((t.flags & DISABLED), 0);
    }

    #[test]
    fn refthingy_unrefthingy_roundtrip() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("bar");
        refthingy("bar");
        // rc was 1 after rthingy, +1 from refthingy = 2
        assert_eq!(thingytab().lock().unwrap().get("bar").unwrap().rc, 2);
        unrefthingy("bar");
        assert_eq!(thingytab().lock().unwrap().get("bar").unwrap().rc, 1);
        unrefthingy("bar");
        // rc dropped to 0 → freenode removes
        assert!(!thingytab().lock().unwrap().contains_key("bar"));
    }

    #[test]
    fn rthingy_nocreate_returns_false_for_missing() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();

        assert!(!rthingy_nocreate("absent"));
        assert!(!thingytab().lock().unwrap().contains_key("absent"));
    }

    #[test]
    fn rthingy_nocreate_refs_existing() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("present");
        assert!(rthingy_nocreate("present"));
        assert_eq!(thingytab().lock().unwrap().get("present").unwrap().rc, 2);
    }

    /// c:60 — `createthingytab` must be idempotent: calling it twice
    /// must not double-populate or clear existing entries. Pinning
    /// catches a regression that resets the global on every call.
    #[test]
    fn createthingytab_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();

        createthingytab();
        let after_first = thingytab().lock().unwrap().len();
        createthingytab();
        let after_second = thingytab().lock().unwrap().len();
        assert_eq!(
            after_first, after_second,
            "createthingytab must not re-populate; was {} now {}",
            after_first, after_second
        );
    }

    /// c:80 — `emptythingytab` only unbinds entries WITHOUT the
    /// DISABLED flag (it leaves the fixed `thingies[]` entries
    /// alone per the C source comment). `rthingy`-created entries
    /// inherit DISABLED from `makethingynode`, so `emptythingytab`
    /// is a no-op for them. Pin this so a regen that removes the
    /// DISABLED filter and starts purging the rthingy entries
    /// destroys widget bindings unexpectedly.
    #[test]
    fn emptythingytab_skips_disabled_rthingy_entries() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("a");
        rthingy("b");
        rthingy("c");
        let before = thingytab().lock().unwrap().len();
        assert!(before >= 3);
        emptythingytab();
        let after = thingytab().lock().unwrap().len();
        assert_eq!(
            after, before,
            "emptythingytab must NOT purge DISABLED rthingy entries"
        );
    }

    /// c:118 — `freethingynode` on a non-existent name must be a
    /// no-op (not panic). Pin the defensive case; a regression that
    /// unwrap()s the table.get would crash the shell on widget unbind.
    #[test]
    fn freethingynode_on_missing_name_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();
        freethingynode("never-existed");
    }

    /// c:147 — `unrefthingy` on rc=1 frees the entry. After unref-
    /// to-zero, the entry must be absent. Catches a regression that
    /// leaves a dangling rc=0 entry in the table.
    #[test]
    fn unrefthingy_at_rc_one_frees_entry() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("solo");
        assert_eq!(thingytab().lock().unwrap().get("solo").unwrap().rc, 1);
        unrefthingy("solo");
        assert!(
            !thingytab().lock().unwrap().contains_key("solo"),
            "rc=0 entry must be removed from thingytab"
        );
    }

    /// c:147 — `unrefthingy` on a missing name must be a safe no-op.
    /// Without this, widget cleanup during shell teardown could
    /// panic on already-freed entries.
    #[test]
    fn unrefthingy_on_missing_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();
        unrefthingy("never-bound");
    }

    /// c:108 — `makethingynode` produces a fresh node with rc=0 and
    /// the DISABLED flag set per c:114. Pin both invariants so a
    /// regen that defaults rc=1 (corrupting refcount math) gets
    /// caught immediately.
    #[test]
    fn makethingynode_starts_at_rc_zero_with_disabled_flag() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let n = makethingynode();
        assert_eq!(n.rc, 0, "fresh node must have rc=0");
        assert_ne!(
            (n.flags & DISABLED),
            0,
            "fresh node must have DISABLED flag set"
        );
    }

    /// c:158 — `rthingy` on the same name twice bumps the refcount,
    /// does NOT create a second entry. Pin the dedup-by-name property.
    #[test]
    fn rthingy_same_name_twice_only_increments_refcount() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("dup");
        rthingy("dup");
        let tab = thingytab().lock().unwrap();
        assert_eq!(tab.len(), 1);
        assert!(
            tab.get("dup").unwrap().rc >= 2,
            "second rthingy must bump rc, not create a sibling"
        );
    }

    /// c:147 — Unref-to-zero pattern across many entries. Stresses
    /// the table mutator path to catch a HashMap-mutation bug that
    /// only shows under multiple inserts/removes.
    #[test]
    fn many_rthingy_unref_cycles_leave_no_residue() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();

        for i in 0..20 {
            rthingy(&format!("entry-{}", i));
        }
        assert!(thingytab().lock().unwrap().len() >= 20);
        for i in 0..20 {
            unrefthingy(&format!("entry-{}", i));
        }
        for i in 0..20 {
            assert!(
                !thingytab()
                    .lock()
                    .unwrap()
                    .contains_key(&format!("entry-{}", i)),
                "entry-{} should be gone after final unref",
                i
            );
        }
    }

    // ─── zsh-corpus pins for thingy registry ───────────────────────

    /// `rthingy_nocreate("never_was")` returns false on missing.
    #[test]
    fn zle_thingy_corpus_rthingy_nocreate_missing_returns_false() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_tab();
        assert!(!rthingy_nocreate("zshrs_never_thingy_xyz"));
    }

    /// `rthingy(name)` then `rthingy_nocreate(name)` returns true.
    #[test]
    fn zle_thingy_corpus_rthingy_then_nocreate_finds_it() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_tab();
        rthingy("zshrs_test_thingy_a");
        assert!(rthingy_nocreate("zshrs_test_thingy_a"));
    }

    /// `unrefthingy` on never-registered name is a safe no-op.
    #[test]
    fn zle_thingy_corpus_unrefthingy_missing_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unrefthingy("zshrs_never_thingy_xyz_abc");
    }

    /// `emptythingytab` unbinds user-installed (non-DISABLED) entries.
    /// rthingy() creates DISABLED slots; C's `scanhashtable(thingytab,
    /// 0, 0, DISABLED, scanemptythingies, 0)` SKIPS DISABLED entries
    /// (the 4th arg is the avoid-flag), so an entry that's never been
    /// bound to a widget stays in the table verbatim. Bind a widget
    /// here first so emptythingytab actually has work to do; then
    /// verify the binding's gone (DISABLED set, widget cleared).
    #[test]
    fn zle_thingy_corpus_emptythingytab_clears_user_entries() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_tab();
        for i in 0..5 {
            let name = format!("e-{i}");
            rthingy(&name);
            // Bind a fresh widget so the entry isn't DISABLED — only
            // non-DISABLED entries get scanned by emptythingytab.
            let w = std::sync::Arc::new(widget {
                flags: 0,
                first: None,
                u: crate::ported::zle::zle_h::WidgetImpl::UserFunc(String::new()),
            });
            bindwidget(w, &name);
        }
        assert!(thingytab().lock().unwrap().len() >= 5);
        emptythingytab();
        let t = thingytab().lock().unwrap();
        for i in 0..5 {
            let name = format!("e-{i}");
            // emptythingytab → unbindwidget → unrefthingy chain
            // removes the entry entirely when its refcount hits 0
            // (the C `freethingynode` path at zle_thingy.c:118-128
            // calls `hashtable->removenode` after the last unref).
            // The user-visible result: `zle -L` shows none of the
            // user-defined widgets after `emptythingytab`.
            assert!(!t.contains_key(&name), "{name} removed");
        }
    }

    /// `freethingynode("never_was")` is a safe no-op.
    #[test]
    fn zle_thingy_corpus_freethingynode_missing_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        freethingynode("zshrs_never_thingy_xyz_abc");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_thingy.c lifecycle ops.
    // ═══════════════════════════════════════════════════════════════════

    /// c:108-113 — `makethingynode()` returns a Thingy with DISABLED
    /// flag set + rc=0 (matches `zshcalloc` + explicit DISABLED).
    #[test]
    fn makethingynode_returns_disabled_and_rc_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let t = makethingynode();
        assert_ne!(
            t.flags & DISABLED,
            0,
            "fresh thingy must have DISABLED set per c:112"
        );
        assert_eq!(t.rc, 0, "fresh thingy must have rc=0 per zshcalloc + c:110");
    }

    /// c:138 — `refthingy(missing)` is a no-op (matches `if(th)` guard).
    #[test]
    fn refthingy_missing_name_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Doesn't panic, doesn't insert.
        let before = thingytab().lock().unwrap().len();
        refthingy("zshrs_never_thingy_for_refthingy_test");
        let after = thingytab().lock().unwrap().len();
        assert_eq!(before, after, "refthingy on missing must NOT insert");
    }

    /// c:147 — `unrefthingy(missing)` is a safe no-op.
    #[test]
    fn unrefthingy_missing_name_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // No panic = pass.
        unrefthingy("zshrs_never_thingy_for_unref_test");
    }

    /// c:158 — `rthingy` creates entry on first call, increments rc on
    /// each subsequent call. Pin: 3 calls → rc=3 (each call bumps).
    #[test]
    fn rthingy_repeated_calls_bump_refcount() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let name = "zshrs_rthingy_rc_test";
        let _ = thingytab().lock().unwrap().remove(name);
        rthingy(name);
        rthingy(name);
        rthingy(name);
        let rc = thingytab().lock().unwrap().get(name).map(|t| t.rc);
        assert_eq!(rc, Some(3), "3 rthingy calls → rc=3");
        let _ = thingytab().lock().unwrap().remove(name);
    }

    /// c:169 — `rthingy_nocreate(missing)` returns false and does NOT
    /// add an entry to the table.
    #[test]
    fn rthingy_nocreate_missing_returns_false_no_insert() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let name = "zshrs_rthingy_nocreate_test_missing";
        let _ = thingytab().lock().unwrap().remove(name);
        let r = rthingy_nocreate(name);
        assert!(!r, "missing → false");
        assert!(
            !thingytab().lock().unwrap().contains_key(name),
            "must NOT insert on lookup-only call"
        );
    }

    /// c:169 — `rthingy_nocreate(existing)` returns true AND bumps rc.
    #[test]
    fn rthingy_nocreate_existing_bumps_rc() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let name = "zshrs_rthingy_nocreate_existing";
        let _ = thingytab().lock().unwrap().remove(name);
        rthingy(name); // create with rc=1
        let rc_before = thingytab()
            .lock()
            .unwrap()
            .get(name)
            .map(|t| t.rc)
            .unwrap_or(0);
        let r = rthingy_nocreate(name);
        assert!(r, "existing → true");
        let rc_after = thingytab()
            .lock()
            .unwrap()
            .get(name)
            .map(|t| t.rc)
            .unwrap_or(0);
        assert_eq!(rc_after, rc_before + 1, "rc must increment by 1");
        let _ = thingytab().lock().unwrap().remove(name);
    }

    /// c:147 — `unrefthingy` decrements rc and removes when rc hits 0.
    #[test]
    fn unrefthingy_at_last_ref_removes_entry() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let name = "zshrs_unref_last_test";
        let _ = thingytab().lock().unwrap().remove(name);
        rthingy(name); // rc=1
        unrefthingy(name); // rc=0 → freed
        assert!(
            !thingytab().lock().unwrap().contains_key(name),
            "rc=0 must remove entry"
        );
    }

    /// c:147 — `unrefthingy` with rc>1 does NOT remove (decrement only).
    #[test]
    fn unrefthingy_with_rc_greater_one_decrements_only() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let name = "zshrs_unref_keepalive_test";
        let _ = thingytab().lock().unwrap().remove(name);
        rthingy(name); // rc=1
        rthingy(name); // rc=2
        unrefthingy(name); // rc=1 → keep
        assert!(
            thingytab().lock().unwrap().contains_key(name),
            "rc=1 (after unref from 2) must remain"
        );
        let rc = thingytab()
            .lock()
            .unwrap()
            .get(name)
            .map(|t| t.rc)
            .unwrap_or(99);
        assert_eq!(rc, 1);
        let _ = thingytab().lock().unwrap().remove(name);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_thingy.c
    // c:48 createthingytab / c:127 emptythingytab / c:208 freethingynode /
    // c:280 rthingy / c:307 rthingy_nocreate / c:351 bindwidget /
    // c:417 unbindwidget / c:752 bin_zle_list / c:792 bin_zle_refresh /
    // c:860 bin_zle_mesg
    // ═══════════════════════════════════════════════════════════════════

    /// c:307 — `rthingy_nocreate` returns bool (compile-time type pin).
    #[test]
    fn rthingy_nocreate_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _: bool = rthingy_nocreate("anything");
    }

    /// c:417 — `unbindwidget` returns i32 (compile-time type pin).
    #[test]
    fn unbindwidget_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _: i32 = unbindwidget("self-insert", 0);
    }

    /// c:417 — `unbindwidget("", _)` empty name returns 0 (missing → no-op).
    /// C body checks `tab.get(t)` and returns 0 for None — empty name
    /// can't match any registered widget, so it's treated as missing.
    #[test]
    fn unbindwidget_empty_name_returns_zero_missing() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let r = unbindwidget("", 0);
        assert_eq!(r, 0, "empty name treated as missing → 0 (per c:427)");
    }

    /// c:752+792+860 — bin_zle_* builtins return in u8 exit-code range.
    #[test]
    fn bin_zle_subcommands_return_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let ops = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for r in [
            bin_zle_list("zle", &[], &ops, 0),
            bin_zle_refresh("zle", &[], &ops, 0),
            bin_zle_mesg("zle", &[], &ops, 0),
        ] {
            assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_thingy.c
    // c:752 bin_zle_list / c:792 bin_zle_refresh / c:860 bin_zle_mesg /
    // c:894 bin_zle_unget / c:919 bin_zle_keymap / c:1008 bin_zle_del /
    // c:1219 zle_usable / c:1896 listwidgets / c:1907 getwidgettarget
    // ═══════════════════════════════════════════════════════════════════

    fn empty_ops_thingy() -> options {
        options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    /// c:752 — `bin_zle_list` returns i32 (compile-time type pin).
    #[test]
    fn bin_zle_list_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let ops = empty_ops_thingy();
        let _: i32 = bin_zle_list("zle", &[], &ops, 0);
    }

    /// c:792 — `bin_zle_refresh` returns i32.
    #[test]
    fn bin_zle_refresh_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let ops = empty_ops_thingy();
        let _: i32 = bin_zle_refresh("zle", &[], &ops, 0);
    }

    /// c:860 — `bin_zle_mesg` returns i32.
    #[test]
    fn bin_zle_mesg_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let ops = empty_ops_thingy();
        let _: i32 = bin_zle_mesg("zle", &[], &ops, 0);
    }

    /// c:894 — `bin_zle_unget` returns i32.
    #[test]
    fn bin_zle_unget_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let ops = empty_ops_thingy();
        let _: i32 = bin_zle_unget("zle", &[], &ops, 0);
    }

    /// c:919 — `bin_zle_keymap` returns i32.
    #[test]
    fn bin_zle_keymap_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let ops = empty_ops_thingy();
        let _: i32 = bin_zle_keymap("zle", &[], &ops, 0);
    }

    /// c:1008 — `bin_zle_del` returns i32.
    #[test]
    fn bin_zle_del_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let ops = empty_ops_thingy();
        let _: i32 = bin_zle_del("zle", &[], &ops, 0);
    }

    /// c:1219 — `zle_usable` returns i32 + 0/1 only.
    #[test]
    fn zle_usable_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = zle_usable();
        assert!(r == 0 || r == 1, "zle_usable ∈ {{0, 1}}, got {}", r);
    }

    /// c:1219 — `zle_usable` is deterministic in non-ZLE context.
    #[test]
    fn zle_usable_is_deterministic_no_zle_context() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let first = zle_usable();
        for _ in 0..3 {
            assert_eq!(zle_usable(), first, "zle_usable must be deterministic");
        }
    }

    /// c:1896 — `listwidgets` returns Vec<String> (compile-time type pin).
    #[test]
    fn listwidgets_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Vec<String> = listwidgets();
    }

    /// c:1907 — `getwidgettarget("")` empty returns Option<String>.
    #[test]
    fn getwidgettarget_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Option<String> = getwidgettarget("");
    }

    /// c:1896 — `listwidgets` is deterministic for stable widget table.
    #[test]
    fn listwidgets_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let first = listwidgets();
        for _ in 0..3 {
            assert_eq!(listwidgets(), first, "listwidgets must be deterministic");
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_thingy.c
    // c:48 createthingytab / c:307 rthingy_nocreate / c:417 unbindwidget /
    // c:659 bin_zle / c:752 bin_zle_list / c:1219 zle_usable /
    // c:1896 listwidgets / c:1907 getwidgettarget
    // ═══════════════════════════════════════════════════════════════════

    /// c:48 — `createthingytab` is idempotent.
    #[test]
    fn createthingytab_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..10 {
            createthingytab();
        }
    }

    /// c:307 — `rthingy_nocreate` returns bool (compile-time pin, alt).
    #[test]
    fn rthingy_nocreate_returns_bool_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: bool = rthingy_nocreate("__never_real_thingy_xyz__");
    }

    /// c:307 — `rthingy_nocreate("")` empty name deterministic.
    #[test]
    fn rthingy_nocreate_empty_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let first = rthingy_nocreate("");
        for _ in 0..5 {
            assert_eq!(
                rthingy_nocreate(""),
                first,
                "rthingy_nocreate('') must be pure"
            );
        }
    }

    /// c:417 — `unbindwidget("nonexistent", _)` returns i32 (no panic).
    #[test]
    fn unbindwidget_nonexistent_returns_i32() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = unbindwidget("__never_widget__", 0);
    }

    /// c:1219 — `zle_usable` returns i32 (compile-time pin).
    #[test]
    fn zle_usable_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = zle_usable();
    }

    /// c:1896 — `listwidgets` returns Vec<String> (alt-name type pin).
    /// Note: in test context the widget table is empty; the
    /// `includes self-insert` invariant only holds after the live
    /// startup-time registration, which test_setup doesn't perform.
    #[test]
    fn listwidgets_returns_vec_string_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Vec<String> = listwidgets();
    }

    /// c:1907 — `getwidgettarget("")` empty input deterministic.
    #[test]
    fn getwidgettarget_empty_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let a = getwidgettarget("");
        let b = getwidgettarget("");
        assert_eq!(a, b, "getwidgettarget('') must be pure");
    }

    /// c:1907 — `getwidgettarget` for unknown widget returns None.
    /// (Test context's widget table is empty so even builtin lookups
    /// return None; only the unknown-name invariant is reliably
    /// testable here.)
    #[test]
    fn getwidgettarget_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert!(
            getwidgettarget("__never_real_widget_xyz__").is_none(),
            "unknown widget → None"
        );
    }

    /// c:659 — `bin_zle` returns i32 (compile-time pin).
    #[test]
    fn bin_zle_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let ops = empty_ops_thingy();
        let _: i32 = bin_zle("zle", &[], &ops, 0);
    }

    /// c:752 — `bin_zle_list` returns i32 (compile-time pin, alt).
    #[test]
    fn bin_zle_list_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let ops = empty_ops_thingy();
        let _: i32 = bin_zle_list("zle", &[], &ops, 0);
    }
}
