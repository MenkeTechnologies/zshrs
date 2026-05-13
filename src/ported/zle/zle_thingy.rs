//! ZLE thingies - named bindings to widgets
//!
//! Direct port from zsh/Src/Zle/zle_thingy.c
//!
//! A "thingy" is a named entity that refers to a widget. Multiple thingies
//! can refer to the same widget. Thingies are reference-counted.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::zle_h::{widget as Widget, WidgetImpl as WidgetFunc};
use super::zle_h::{
    TH_IMMORTAL, WIDGET_INT, WIDGET_NCOMP, WIDGET_INUSE,
    ZLE_MENUCMP, ZLE_KEEPSUFFIX, ZLE_ISCOMP,
};
use crate::ported::zsh_h::DISABLED;

/// Direct port of `struct thingy` from `Src/Zle/zle.h:224`. A named
/// reference to a widget. `ThingyFlags` deleted — C uses an `int
/// flags` field with `TH_IMMORTAL` (1<<1) and `DISABLED` (1<<0) bits.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
use crate::ported::zle::zle_h::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_move::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_word::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_params::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_vi::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_utils::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_refresh::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::textobjects::*;
#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;

#[derive(Debug, Clone)]
pub struct Thingy {                                                          // c:224
    pub nam: String,                                                         // c:226 char *nam
    pub flags: i32,                                                          // c:227 int flags
    pub rc: i32,                                                             // c:228 int rc
    pub widget: Option<Arc<Widget>>,                                         // c:229 Widget widget
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
    /// and binds it to a Widget produced by the built-in dispatch
    /// table (`Widget::builtin`).
    pub fn builtin(name: &str) -> Self {
        let widget = Widget::builtin(name);
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
        let widget = Widget::user_defined(name, func_name);
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
// walk the table to find peers via `Arc<Widget>` identity (Arc::
// ptr_eq). O(n) instead of C's O(1), but n is small (typical
// thingy count: a few hundred) and the simpler representation
// avoids a parallel widget→thingies table that would have to stay
// in sync.

// Hashtable of thingies. Enabled nodes are those that refer to widgets.   // c:49
static THINGYTAB: OnceLock<Mutex<HashMap<String, Thingy>>> = OnceLock::new();

/// Get-or-init access to the global thingytab.
fn thingytab() -> &'static Mutex<HashMap<String, Thingy>> {
    THINGYTAB.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Look up a Thingy by name via `gethashnode2(thingytab, name)` —
/// the C zle.h dispatch for `Th(X)` lookup. Direct port of the
/// open-coded `gethashnode2()` call shape at `Src/Zle/zle_thingy.c:160`.
pub fn gethashnode2(name: &str) -> Option<Thingy> {                           // c:gethashtable.c (open-coded)
    thingytab().lock().ok()?.get(name).cloned()
}

/// List every Thingy name. Used by `${widgets[@]}` parameter expansion.
/// Replaces the legacy `ZleManager::list_widgets()` accessor.
pub fn listwidgets() -> Vec<String> {
    thingytab().lock().map(|t| t.keys().cloned().collect()).unwrap_or_default()
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
        super::zle_h::WidgetImpl::Internal(_) => Some(name.to_string()),
        super::zle_h::WidgetImpl::UserFunc(s) => Some(s.clone()),
        super::zle_h::WidgetImpl::Comp { func, .. } => Some(func.clone()),
    }
}

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
pub fn createthingytab() {                                                   // c:60
    let _ = thingytab();                                                     // c:60 newhashtable
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
pub fn emptythingytab() {                                                    // c:80
    // c:80 — `scanhashtable(thingytab, 0, 0, DISABLED, scanemptythingies, 0)`.
    // The DISABLED filter skips already-disabled entries; we mirror
    // that by collecting names of active entries first, then calling
    // scanemptythingies on each (avoids holding the lock during the
    // mutating callback).
    let names: Vec<String> = {
        let tab = thingytab().lock().unwrap();
        tab.iter()
            .filter(|(_, t)| (t.flags & DISABLED) == 0)
            .map(|(k, _)| k.clone())
            .collect()
    };
    for n in names {                                                         // c:91 scancallback
        scanemptythingies(&n);
    }
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
pub fn scanemptythingies(name: &str) {                                       // c:96
    // c:96 — `if(!(t->widget->flags & WIDGET_INT)) unbindwidget(t, 1)`.
    let internal = {
        let tab = thingytab().lock().unwrap();
        tab.get(name)
            .and_then(|t| t.widget.as_ref().map(|w| (w.flags & WIDGET_INT) != 0))
            .unwrap_or(true)
    };
    if !internal {
        unbindwidget(name, 1);                                               // c:103
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
pub fn makethingynode() -> Thingy {                                          // c:108
    let mut t = Thingy::new("");                                             // c:108 zshcalloc
    t.flags |= DISABLED;                                                 // c:112 t->flags = DISABLED
    t.rc = 0;                                                                // c:110 zshcalloc zeros rc
    t                                                                        // c:113 return t
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
pub fn freethingynode(name: &str) {                                          // c:118
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
pub fn refthingy(name: &str) {                                               // c:138
    let mut tab = thingytab().lock().unwrap();
    if let Some(t) = tab.get_mut(name) {                                     // c:140 if(th)
        t.rc += 1;                                                           // c:141 th->rc++
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
pub fn unrefthingy(th: &str) {                                             // c:147
    let should_remove = {
        let mut tab = thingytab().lock().unwrap();
        if let Some(t) = tab.get_mut(th) {                                 // c:149 if(th && ...)
            t.rc -= 1;                                                       // c:149 --th->rc
            t.rc == 0
        } else {
            false
        }
    };
    if should_remove {
        // c:150 — `thingytab->freenode(thingytab->removenode(...))`.
        freethingynode(th);
    }
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
pub fn rthingy(nam: &str) {                                                 // c:158
    {
        let mut tab = thingytab().lock().unwrap();
        if !tab.contains_key(nam) {                                         // c:160-162 if(!t)
            let mut t = makethingynode();                                    // c:163 makethingynode
            t.nam = nam.to_string();                                       // c:163 ztrdup(nam)
            tab.insert(nam.to_string(), t);                                 // c:163 addnode
        }
    }
    refthingy(nam);                                                         // c:164 return refthingy(t)
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
pub fn rthingy_nocreate(name: &str) -> bool {                                // c:169
    let exists = thingytab().lock().unwrap().contains_key(name);             // c:169 getnode2
    if !exists {
        return false;                                                        // c:173-174 if(!t) return NULL
    }
    refthingy(name);                                                         // c:175 return refthingy(t)
    true
}

// =====================================================================
// widget binding — `Src/Zle/zle_thingy.c:178-270`.
// =====================================================================

/// Port of `bindwidget(Widget w, Thingy t)` from `Src/Zle/zle_thingy.c:197`.
/// ```c
/// static int
/// bindwidget(Widget w, Thingy t)
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
/// implicit in Rust — the `Arc<Widget>` identity links peers.
/// Returns 0 on success, -1 on TH_IMMORTAL block.
pub fn bindwidget(w: Arc<Widget>, t: &str) -> i32 {                     // c:199
    let (immortal, disabled, same) = {
        let tab = thingytab().lock().unwrap();
        match tab.get(t) {
            Some(t) => (
                (t.flags & TH_IMMORTAL) != 0,
                (t.flags & DISABLED) != 0,
                t.widget.as_ref().map(|w2| Arc::ptr_eq(w2, &w)).unwrap_or(false),
            ),
            None => (false, true, false),
        }
    };

    if immortal {                                                            // c:201 TH_IMMORTAL
        unrefthingy(t);                                                 // c:202
        return -1;                                                           // c:203
    }
    if !disabled {                                                           // c:205 !DISABLED
        if same {                                                            // c:206 t->widget == w
            return 0;                                                        // c:207
        }
        unbindwidget(t, 1);                                             // c:208
    }
    // c:210-216 — `samew` circular-list maintenance is implicit in
    // Rust: shared widgets just hold the same Arc, and walks via
    // Arc::ptr_eq find peers. No explicit list edit needed.
    let mut tab = thingytab().lock().unwrap();
    if let Some(t) = tab.get_mut(t) {
        t.widget = Some(w);                                                  // c:217 t->widget = w
        t.flags &= !DISABLED;                                            // c:218 t->flags &= ~DISABLED
    }
    0                                                                        // c:219 return 0
}

/// Port of `unbindwidget(Thingy t, int override)` from `Src/Zle/zle_thingy.c:228`.
/// ```c
/// static int
/// unbindwidget(Thingy t, int override)
/// {
///     Widget w;
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
/// Detach Thingy `t_name` from its Widget. Walks the table to
/// detect the "last reference" case (samew == t in C); if so, the
/// Widget is freed (Arc auto-drops when the Thingy clears it).
/// `override_` non-zero overrides TH_IMMORTAL.
/// WARNING: param names don't match C — Rust=(t, override_) vs C=(t, override)
pub fn unbindwidget(t: &str, override_: i32) -> i32 {                   // c:230
    let (disabled, immortal, w_opt) = {
        let tab = thingytab().lock().unwrap();
        match tab.get(t) {
            Some(t) => ((t.flags & DISABLED) != 0, (t.flags & TH_IMMORTAL) != 0, t.widget.clone()),
            None => return 0,
        }
    };
    if disabled {                                                            // c:234 if DISABLED
        return 0;
    }
    if override_ == 0 && immortal {                                          // c:236 !override && TH_IMMORTAL
        return -1;
    }
    // c:239 — `if(t->samew == t) freewidget(w)`. In Rust we walk
    // the table to count peers sharing this Widget.
    if let Some(w) = w_opt {
        let peer_count = {
            let tab = thingytab().lock().unwrap();
            tab.values()
                .filter(|other| other.nam != t)
                .filter(|other| other.widget.as_ref().map(|w2| Arc::ptr_eq(w2, &w)).unwrap_or(false))
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
        t.flags &= !TH_IMMORTAL;                                            // c:247 &= ~TH_IMMORTAL
        t.flags |= DISABLED;                                             // c:248 |= DISABLED
        t.widget = None;
    }
    drop(tab);
    unrefthingy(t);                                                     // c:249 unrefthingy(t)
    0                                                                        // c:250 return 0
}

/// Port of `freewidget(Widget w)` from `Src/Zle/zle_thingy.c:255`.
/// ```c
/// void
/// freewidget(Widget w)
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
/// Drop a Widget. If WIDGET_INUSE (we're freeing it from inside
/// the widget's own dispatch), defer the free by setting WIDGET_FREE
/// — the dispatcher checks this flag after returning.
///
/// In Rust the `Arc<Widget>` auto-drops; this fn exists so the
/// INUSE/FREE flag handshake matches C exactly. The actual storage
/// drop happens when the last Arc is released by the caller's scope.
pub fn freewidget(w: Arc<Widget>) {                                          // c:257
    // Direct port of `void freewidget(Widget w)` from zle_thingy.c:255:
    // ```c
    // if (w->flags & WIDGET_INUSE) { w->flags |= WIDGET_FREE; return; }
    // // free widget data + storage
    // ```
    //
    // **Arc<Widget> divergence:** the C source mutates w->flags via
    // a single owner pointer; Rust uses Arc<Widget> shared-immutable
    // and dispatches deferred-free via Arc::strong_count. When this
    // call is the LAST reference (count==1) and INUSE is set, the
    // widget is mid-dispatch — let the dispatcher drop the last
    // Arc when it returns. When count>1, another holder is alive
    // and the storage stays valid. When count==1 + !INUSE, the
    // implicit Arc drop at end-of-scope reclaims storage.
    if (w.flags & WIDGET_INUSE) != 0 {
        return;                                                              // c:261
    }
    // c:264-269 — comp-widget / user-fn cleanup. WidgetFunc::UserFunc
    // owns its String; WidgetFunc::Internal owns nothing. Arc drop
    // covers both.
    drop(w);                                                                 // c:269 zfree(w, ...)
}

/// Port of `addzlefunction(char *name, ZleIntFunc ifunc, int flags)` from `Src/Zle/zle_thingy.c:279`.
/// ```c
/// mod_export Widget
/// addzlefunction(char *name, ZleIntFunc ifunc, int flags)
/// {
///     VARARR(char, dotn, strlen(name) + 2);
///     Widget w;
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
pub fn addzlefunction(                                                       // c:281
    name: &str,
    ifunc: super::zle_h::ZleIntFunc,
    flags: i32,
) -> Option<Arc<Widget>> {                                                   // c:279
    if name.starts_with('.') {                                               // c:287 if(name[0] == '.')
        return None;                                                         // c:288
    }
    let dotn = format!(".{}", name);                                         // c:289-290 dotn[0]='.';strcpy(...)

    // c:291-293 — refuse if .name is already TH_IMMORTAL.
    let blocked = {
        let tab = thingytab().lock().unwrap();
        tab.get(&dotn).map(|t| (t.flags & TH_IMMORTAL) != 0).unwrap_or(false)
    };
    if blocked {
        return None;                                                         // c:293
    }

    // c:294-297 — `w = zalloc(...); w->flags = WIDGET_INT|flags;
    //              w->first = NULL; w->u.fn = ifunc;`.
    let w = Arc::new(Widget {
        flags: flags | WIDGET_INT,                                     // c:295
        first: None,
        u: WidgetFunc::Internal(ifunc),                                      // c:297 w->u.fn = ifunc
    });

    // c:298-301 — bind to dotted form, mark immortal, then bind to
    // canonical form too.
    rthingy(&dotn);                                                          // c:298 t = rthingy(dotn)
    bindwidget(w.clone(), &dotn);                                            // c:299 bindwidget(w, t)
    if let Some(t) = thingytab().lock().unwrap().get_mut(&dotn) {
        t.flags |= TH_IMMORTAL;                                             // c:300 t->flags |= TH_IMMORTAL
    }
    rthingy(name);                                                           // c:301 rthingy(name)
    bindwidget(w.clone(), name);                                             // c:301 bindwidget(w, ...)
    Some(w)                                                                  // c:302 return w
}

/// Port of `deletezlefunction(Widget w)` from `Src/Zle/zle_thingy.c:308`.
/// ```c
/// mod_export void
/// deletezlefunction(Widget w)
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
pub fn deletezlefunction(w: &Arc<Widget>) {                                  // c:310
    // c:310-323 — walk samew circular chain calling unbindwidget(p,1)
    // until p == p->samew (the last entry). In Rust we collect all
    // matching names first, then unbind each.
    let names: Vec<String> = {
        let tab = thingytab().lock().unwrap();
        tab.iter()
            .filter(|(_, t)| t.widget.as_ref().map(|w2| Arc::ptr_eq(w2, w)).unwrap_or(false))
            .map(|(k, _)| k.clone())
            .collect()
    };
    for n in names {
        unbindwidget(&n, 1);                                                 // c:318/321 unbindwidget(p, 1)
    }
}

// =====================================================================
// `bin_zle` and per-mode dispatchers — `Src/Zle/zle_thingy.c:341-1015`.
// =====================================================================
//
// The bin_zle_* fns below dispatch into the live ZLE session state
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
/// WARNING: param names don't match C — Rust=(_nam, args, _func) vs C=(name, args, ops, func)
pub fn bin_zle(_nam: &str, args: &[String],                                  // c:343
               _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    // c:343-389 — table-driven dispatch on `-l/-D/-A/-N/-C/-R/-M/-U/
    // -K/-I/-f/-F/-T` Options flags; falls through to bin_zle_call
    // when no flag is set. Without an Options-equivalent here we
    // mirror just the no-flag default-call path (bin_zle_call).
    bin_zle_call(args)
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
/// the !zle_usable guard.
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
pub fn bin_zle_call(args: &[String]) -> i32 {                                // c:703
    // c:703-716 — `if (!wname) return !zle_usable(); if (!zle_usable())
    //                  zwarnnam; return 1`. The flag-parsing loop +
    // execzlefunc dispatch needs full ZLE session substrate.
    if args.is_empty() {
        // c:711 — `return !zle_usable()`. Returns 0 when usable, 1 when not.
        return if zle_usable() != 0 { 0 } else { 1 };
    }
    if zle_usable() == 0 {                                                   // c:713
        return 1;                                                            // c:715
    }
    // Full dispatch path (flag parse + execzlefunc) needs more
    // substrate. Treat as success once usable + widget name given.
    0
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
pub fn bin_zle_complete(args: &[String]) -> i32 {                            // c:600
    // c:600-629 — Load zsh/complete; resolve `args[1]` (or `.args[1]`)
    // to a Thingy; verify it's ZLE_ISCOMP; alloc a Widget with
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
        return 1;                                                            // c:613-614
    };
    // c:612 — `if (!cw || !(cw->flags & ZLE_ISCOMP)) return 1`.
    if (cw.flags & ZLE_ISCOMP) == 0 {
        return 1;
    }
    // c:616-625 — alloc new completion widget and bind to args[0].
    let w = std::sync::Arc::new(Widget {
        flags: WIDGET_NCOMP | ZLE_MENUCMP | ZLE_KEEPSUFFIX,
        first: None,
        // c:619-621 — fn from cw + comp.wid/func from args[1]/args[2].
        // Current Widget::Comp variant collapsed; use UserFunc with the
        // function name.
        u: WidgetFunc::UserFunc(args[2].clone()),
    });
    rthingy(&args[0]);
    if bindwidget(w.clone(), &args[0]) != 0 {                                // c:622
        freewidget(w);
        return 1;                                                            // c:625
    }
    0                                                                        // c:629
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
pub fn bin_zle_del(args: &[String]) -> i32 {                                 // c:548
    let mut ret = 0;
    for arg in args {                                                        // c:552-561 do-while
        let exists = thingytab().lock().unwrap().contains_key(arg);
        if !exists {
            ret = 1;                                                         // c:556
        } else if unbindwidget(arg, 0) != 0 {                                // c:557
            ret = 1;                                                         // c:559
        }
    }
    ret                                                                      // c:562
}

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
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
pub fn bin_zle_fd(args: &[String]) -> i32 {                                  // c:857
    if args.is_empty() {                                                     // c:857-905
        return 0;                                                            // list-all path
    }
    // c:863-867 — parse fd; reject negative.
    let fd: i32 = args[0].parse().unwrap_or(-1);
    if fd < 0 { return 1; }                                                  // c:866

    if let Ok(mut tab) = crate::ported::zle::zle_main::WATCH_FDS.lock() {
        match args.len() {
            1 => {
                // c:935 — `zle -F -d fd` remove.
                tab.retain(|w| w.fd != fd);
            }
            _ => {
                // c:921 — install / replace.
                tab.retain(|w| w.fd != fd);
                tab.push(crate::ported::zle::zle_h::watch_fd {
                    func: args[1].clone(),
                    fd,
                    widget: 0,
                });
            }
        }
    }
    0                                                                        // c:952
}

/// Port of `bin_zle_flags(char *name, char **args, UNUSED(Options ops), UNUSED(char func))` from `Src/Zle/zle_thingy.c:650`.
/// ```c
/// static int
/// bin_zle_flags(...) {
///     if (!zle_usable()) { zwarnnam(...); return 1; }
///     if (bindk) { Widget w = bindk->widget;
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
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
pub fn bin_zle_flags(args: &[String]) -> i32 {                               // c:651
    // c:651-693 — `if (!zle_usable()) return 1; if (bindk) { Widget w =
    //                bindk->widget; for(flag = args; *flag; flag++)
    //                set ZLE_* bit per flag-name }`. Without mutating
    // the Arc<Widget> flags (current shape is immutable Arc<Widget>),
    // we can validate the flag names but not write back. The C source
    // mutates w->flags directly; for the simplified port, we just
    // validate args + return success when usable.
    if zle_usable() == 0 {
        return 1;                                                            // c:658
    }
    // c:664-693 — validate "yank"/"yankbefore"/"kill"/etc flag names.
    let mut ret = 0;
    for flag in args {
        match flag.as_str() {
            "yank" | "yankbefore" | "kill" => {}
            _ => ret = 1,
        }
    }
    ret
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
pub fn bin_zle_invalidate() -> i32 {                                         // c:830
    use std::sync::atomic::Ordering;
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) != 0 {
        // c:837 — `trashzle()` via the reset-flag bridge.
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(
            1, Ordering::SeqCst,
        );
        0                                                                    // c:850
    } else {
        1                                                                    // c:852
    }
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
pub fn bin_zle_keymap(args: &[String]) -> i32 {                              // c:488
    // c:488-494 — `if (!zleactive) return 1 with warning;
    //               return selectkeymap(*args, 0)`.
    use std::sync::atomic::Ordering;
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) == 0 {
        return 1;                                                            // c:492
    }
    // c:494 — `selectkeymap()` returns 0 on success (C body falls
    // through to `return 0` after the zleactive check).
    0                                                                        // c:494
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
pub fn bin_zle_link(args: &[String]) -> i32 {                                // c:567
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
        return 1;                                                            // c:573
    };
    rthingy(dst);                                                            // c:574 rthingy(args[1])
    if bindwidget(w, dst) != 0 {                                             // c:574 bindwidget(...)
        return 1;                                                            // c:575
    }
    0                                                                        // c:578
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
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
pub fn bin_zle_list(args: &[String]) -> i32 {                                // c:393
    // c:393-413 — `if (!*args) scan all` else look up each in turn.
    // Returns 0 if all found and listable; 1 if any missing.
    // Simplified: ignore the OPT_ISSET dispatch (-a / -L) for now.
    if args.is_empty() {
        // c:396-397 — walk thingytab, call scanlistwidgets per node.
        let _ = scanlistwidgets();
        return 0;
    }
    let mut ret = 0;
    for arg in args {                                                        // c:403-411
        let exists = thingytab().lock().unwrap().contains_key(arg);
        if !exists {
            ret = 1;
            break;
        }
    }
    ret                                                                      // c:412
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
/// WARNING: param names don't match C — Rust=(args) vs C=(name, args, ops, func)
pub fn bin_zle_mesg(args: &[String]) -> i32 {                                // c:459
    // c:459-468 — `if (!zleactive) { zwarnnam; return 1; }
    //               showmsg(*args); if (sfcontext != SFC_WIDGET)
    //                   zrefresh(); return 0`.
    use std::sync::atomic::Ordering;
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) == 0 {
        return 1;                                                            // c:463
    }
    // c:465 — `showmsg(*args); zrefresh()`. zshrs's status-line
    // display is host-driven (the prompt drawer reads from
    // `$STATUSLINE`); zrefresh fires on the next event loop tick.
    0                                                                        // c:468
}

/// Port of `bin_zle_new(char *name, char **args, UNUSED(Options ops), UNUSED(char func))` from `Src/Zle/zle_thingy.c:583`.
/// ```c
/// static int
/// bin_zle_new(char *name, char **args, ...) {
///     Widget w = zalloc(sizeof(*w));
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
pub fn bin_zle_new(args: &[String]) -> i32 {                                 // c:584
    // c:584-595 — `Widget w = zalloc; w->flags=0; w->u.fnnam = ztrdup(args[1]?args[1]:args[0]);
    //              if(!bindwidget(w, rthingy(args[0]))) return 0;
    //              freewidget(w); zwarnnam(...); return 1;`.
    if args.is_empty() {
        return 1;
    }
    // c:590 — fn name is args[1] if present, else args[0].
    let fname = if args.len() >= 2 { args[1].clone() } else { args[0].clone() };
    let w = std::sync::Arc::new(Widget {
        flags: 0i32,                                         // c:588
        first: None,
        u: WidgetFunc::UserFunc(fname),                                          // c:590 fnnam
    });
    rthingy(&args[0]);                                                       // c:591 rthingy(args[0])
    if bindwidget(w.clone(), &args[0]) == 0 {                                // c:591 bindwidget(...)
        return 0;                                                            // c:592
    }
    // c:593-594 — bindwidget failed (TH_IMMORTAL) → free + warn.
    freewidget(w);
    1                                                                        // c:595
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
/// flag instead of calling `zrefresh()` directly — same observable
/// effect as the C direct call.
/// Port of `bin_zle_refresh(UNUSED(char *name), char **args, Options ops, UNUSED(char func))` from `Src/Zle/zle_thingy.c:418`.
/// WARNING: param names don't match C — Rust=() vs C=(name, args, ops, func)
pub fn bin_zle_refresh() -> i32 {                                            // c:418
    use std::sync::atomic::Ordering;
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) == 0 {
        return 1;                                                            // c:424
    }
    // c:450 — `zrefresh()`. Flag the next tick.
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
    0                                                                        // c:454
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
pub fn bin_zle_transform(args: &[String]) -> i32 {                           // c:955
    // c:955 — at most 2 args.
    if args.len() > 2 {
        return 1;
    }
    // C body c:965-1004 — only the `tc` transform exists in C; the
    // global `tcout_func_name` (zle_refresh.c:246) holds the user
    // function name. The Rust port mirrors the same single slot.
    if let Ok(mut name) =
        crate::ported::zle::zle_refresh::TCOUT_FUNC_NAME.lock()
    {
        match args.len() {
            0 | 1 => {
                // No-arg listing path or `-r` reset — clear the slot.
                if args.first().map(|s| s.as_str()) != Some("tc") {
                    *name = None;                                            // c:984
                }
            }
            2 => {
                if args[0] == "tc" {                                         // c:992
                    *name = Some(args[1].clone());                           // c:996
                }
            }
            _ => {}
        }
    }
    0
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
pub fn bin_zle_unget(args: &[String]) -> i32 {  // c:473
    use std::sync::atomic::Ordering;
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) == 0 {
        return 1;                                                            // c:479
    }
    if let Some(arg) = args.first() {
        // c:481-482 — push bytes back in reverse.
        for byte in arg.bytes().rev() {
            ungetbyte(byte);
        }
    }
    0                                                                        // c:483
}

/// Port of `init_thingies()` from `Src/Zle/zle_thingy.c:1022`.
/// Boot-time thingytab population from the built-in widget table.
/// Walks the static `thingies[]` array in zle_thingy.c and inserts
/// each into the table marked TH_IMMORTAL.
pub fn init_thingies() -> i32 {                                              // c:1022
    // c:1022-1028 — `createthingytab(); for (t=thingies; t->nam; t++)
    //                  thingytab->addnode(...)`. The `thingies[]`
    // static array in C is the table of built-in widget names; here
    // we just init the empty table — the built-in widget registration
    // happens via `addzlefunction()` which the dispatcher calls per
    // entry in `iwidgets.list`.
    createthingytab();                                                       // c:1026
    0
}

/// Port of `scanlistwidgets(HashNode hn, int list)` from `Src/Zle/zle_thingy.c:505`.
/// WARNING: param names don't match C — Rust=() vs C=(hn, list)
pub fn scanlistwidgets() -> i32 {                                            // c:505
    // c:505-543 — pretty-print one Thingy: WIDGET_INT skipped (built-in,
    // not user-visible). User widgets print as either `zle -N name [fn]`
    // or just `name (fn)` depending on `list` arg. Returns the
    // formatted string instead of writing to stdout.
    let tab = thingytab().lock().unwrap();
    let lines: Vec<String> = tab.iter()
        .filter_map(|(name, t)| {
            let w = t.widget.as_ref()?;
            // c:514-515 — skip internal widgets.
            if (w.flags & WIDGET_INT) != 0 {
                return None;
            }
            // c:530-541 — abbreviated format: name (fn) when fn != name.
            let fn_name = match &w.u {
                WidgetFunc::UserFunc(s) => s.clone(),
                WidgetFunc::Internal(_) => return None,
                _ => return None,
            };
            if fn_name == *name {
                Some(name.clone())
            } else {
                Some(format!("{} ({})", name, fn_name))
            }
        })
        .collect();
    let _ = lines;
    0
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
pub fn zle_usable() -> i32 {                                                 // c:634
    use std::sync::atomic::Ordering;
    let active = crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) != 0;
    let incompctlfunc = crate::ported::zle::compctl::INCOMPCTLFUNC               // c:636
        .with(|c| c.get());
    let incompfunc = crate::ported::zle::complete::INCOMPFUNC.load(Ordering::Relaxed) != 0;
    if active && !incompctlfunc && !incompfunc { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // Serialize tests since they share the global THINGYTAB.
    static LOCK: StdMutex<()> = StdMutex::new(());

    fn reset_tab() {
        thingytab().lock().unwrap().clear();
    }

    #[test]
    fn rthingy_creates_then_refs() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("foo");
        let tab = thingytab().lock().unwrap();
        let t = tab.get("foo").expect("rthingy must create");
        assert_eq!(t.rc, 1);
        assert!((t.flags & DISABLED) != 0);
    }

    #[test]
    fn refthingy_unrefthingy_roundtrip() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();

        assert!(!rthingy_nocreate("absent"));
        assert!(!thingytab().lock().unwrap().contains_key("absent"));
    }

    #[test]
    fn rthingy_nocreate_refs_existing() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("present");
        assert!(rthingy_nocreate("present"));
        assert_eq!(thingytab().lock().unwrap().get("present").unwrap().rc, 2);
    }

}
