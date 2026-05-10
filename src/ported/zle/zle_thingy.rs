//! ZLE thingies - named bindings to widgets
//!
//! Direct port from zsh/Src/Zle/zle_keymap.c thingy structures
//!
//! A "thingy" is a named entity that refers to a widget. Multiple thingies
//! can refer to the same widget. Thingies are reference-counted.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::widget::{Widget, WidgetFlags, WidgetFunc};

/// Flags for thingies
#[derive(Debug, Clone, Copy, Default)]
pub struct ThingyFlags {
    /// Thingy is disabled
    pub disabled: bool,
    /// Can't refer to a different widget
    pub immortal: bool,
}

/// A thingy - a named reference to a widget
#[derive(Debug, Clone)]
pub struct Thingy {
    /// Name of the thingy
    pub name: String,
    /// Flags
    pub flags: ThingyFlags,
    /// Reference count (for compatibility, though Arc handles this)
    pub rc: i32,
    /// Widget this thingy refers to
    pub widget: Option<Arc<Widget>>,
}

impl Thingy {
    /// Create a thingy with no widget bound — equivalent to a freshly
    /// allocated entry from `makethingynode()` in
    /// Src/Zle/zle_thingy.c:108. Callers fill in `widget` later via
    /// `bindwidget` (zle_thingy.c:199).
    pub fn new(name: &str) -> Self {
        Thingy {
            name: name.to_string(),
            flags: ThingyFlags::default(),
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
            name: name.to_string(),
            flags: ThingyFlags {
                disabled: false,
                immortal: true,
            },
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
            name: name.to_string(),
            flags: ThingyFlags::default(),
            rc: 1,
            widget: Some(Arc::new(widget)),
        }
    }

    /// Test whether this thingy's name matches `name`.
    /// Equivalent to the `IS_THINGY(thingy, name)` macro at
    /// Src/Zle/zle.h — used by widget bodies that special-case their
    /// own bound name (e.g. select-a-word checking which alias fired).
    pub fn is(&self, name: &str) -> bool {
        self.name == name
    }

    /// Test whether this thingy is `name` or its dot-prefixed variant.
    /// The `.foo` form names the underlying built-in when a user has
    /// aliased `foo` to something else — see `bin_zle_new`'s `args[0]`
    /// vs `args[1]` split at zle_thingy.c:584. Callers use this when
    /// they want the canonical built-in regardless of user aliasing.
    pub fn is_thingy(&self, name: &str) -> bool {
        self.name == name || self.name == format!(".{}", name)
    }
}

/// Standard thingy names used throughout ZLE
pub mod names {
    /// Accept and execute a line
    pub const ACCEPT_LINE: &str = "accept-line";
    /// Send break (abort)
    pub const SEND_BREAK: &str = "send-break";
    /// Insert character
    pub const SELF_INSERT: &str = "self-insert";
    /// Delete character or list completions
    pub const DELETE_CHAR_OR_LIST: &str = "delete-char-or-list";
    /// Backward delete character
    pub const BACKWARD_DELETE_CHAR: &str = "backward-delete-char";
    /// Move backward one character
    pub const BACKWARD_CHAR: &str = "backward-char";
    /// Move forward one character
    pub const FORWARD_CHAR: &str = "forward-char";
    /// Move to beginning of line
    pub const BEGINNING_OF_LINE: &str = "beginning-of-line";
    /// Move to end of line
    pub const END_OF_LINE: &str = "end-of-line";
    /// Move backward one word
    pub const BACKWARD_WORD: &str = "backward-word";
    /// Move forward one word
    pub const FORWARD_WORD: &str = "forward-word";
    /// Kill to end of line
    pub const KILL_LINE: &str = "kill-line";
    /// Kill whole line
    pub const KILL_WHOLE_LINE: &str = "kill-whole-line";
    /// Kill word forward
    pub const KILL_WORD: &str = "kill-word";
    /// Kill word backward
    pub const BACKWARD_KILL_WORD: &str = "backward-kill-word";
    /// Yank from kill ring
    pub const YANK: &str = "yank";
    /// Undo
    pub const UNDO: &str = "undo";
    /// Redo
    pub const REDO: &str = "redo";
    /// Clear screen
    pub const CLEAR_SCREEN: &str = "clear-screen";
    /// Expand or complete
    pub const EXPAND_OR_COMPLETE: &str = "expand-or-complete";
    /// History search backward
    pub const HISTORY_INCREMENTAL_SEARCH_BACKWARD: &str = "history-incremental-search-backward";
    /// History search forward
    pub const HISTORY_INCREMENTAL_SEARCH_FORWARD: &str = "history-incremental-search-forward";
    /// Up line or history
    pub const UP_LINE_OR_HISTORY: &str = "up-line-or-history";
    /// Down line or history
    pub const DOWN_LINE_OR_HISTORY: &str = "down-line-or-history";
    /// Transpose characters
    pub const TRANSPOSE_CHARS: &str = "transpose-chars";
    /// Delete character
    pub const DELETE_CHAR: &str = "delete-char";
    /// Vi command mode
    pub const VI_CMD_MODE: &str = "vi-cmd-mode";
    /// Vi insert mode
    pub const VI_INSERT: &str = "vi-insert";
}

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

// =====================================================================
// hashtable management — `Src/Zle/zle_thingy.c:58-124`.
// =====================================================================

/// Port of `createthingytab()` from `Src/Zle/zle_thingy.c:58`.
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
pub fn createthingytab() {                                                   // c:58
    let _ = thingytab();                                                     // c:62 newhashtable
}

/// Port of `emptythingytab()` from `Src/Zle/zle_thingy.c:78`.
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
pub fn emptythingytab() {                                                    // c:78
    // c:91 — `scanhashtable(thingytab, 0, 0, DISABLED, scanemptythingies, 0)`.
    // The DISABLED filter skips already-disabled entries; we mirror
    // that by collecting names of active entries first, then calling
    // scanemptythingies on each (avoids holding the lock during the
    // mutating callback).
    let names: Vec<String> = {
        let tab = thingytab().lock().unwrap();
        tab.iter()
            .filter(|(_, t)| !t.flags.disabled)
            .map(|(k, _)| k.clone())
            .collect()
    };
    for n in names {                                                         // c:91 scancallback
        scanemptythingies(&n);
    }
}

/// Port of `scanemptythingies()` from `Src/Zle/zle_thingy.c:94`.
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
pub fn scanemptythingies(name: &str) {                                       // c:94
    // c:102 — `if(!(t->widget->flags & WIDGET_INT)) unbindwidget(t, 1)`.
    let internal = {
        let tab = thingytab().lock().unwrap();
        tab.get(name)
            .and_then(|t| t.widget.as_ref().map(|w| w.flags.contains(WidgetFlags::INT)))
            .unwrap_or(true)
    };
    if !internal {
        unbindwidget(name, 1);                                               // c:103
    }
}

/// Port of `makethingynode()` from `Src/Zle/zle_thingy.c:106`.
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
pub fn makethingynode() -> Thingy {                                          // c:106
    let mut t = Thingy::new("");                                             // c:110 zshcalloc
    t.flags.disabled = true;                                                 // c:112 t->flags = DISABLED
    t.rc = 0;                                                                // c:110 zshcalloc zeros rc
    t                                                                        // c:113 return t
}

/// Port of `freethingynode()` from `Src/Zle/zle_thingy.c:116`.
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
pub fn freethingynode(name: &str) {                                          // c:116
    // c:122-123 — `zsfree(th->nam); zfree(th, sizeof(*th))`. Rust
    // String + Thingy drop on `remove()`.
    let _ = thingytab().lock().unwrap().remove(name);
}

// =====================================================================
// reference counting — `Src/Zle/zle_thingy.c:130-176`.
// =====================================================================

/// Port of `refthingy()` from `Src/Zle/zle_thingy.c:136`.
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
pub fn refthingy(name: &str) {                                               // c:136
    let mut tab = thingytab().lock().unwrap();
    if let Some(t) = tab.get_mut(name) {                                     // c:140 if(th)
        t.rc += 1;                                                           // c:141 th->rc++
    }
}

/// Port of `unrefthingy()` from `Src/Zle/zle_thingy.c:145`.
/// ```c
/// void
/// unrefthingy(Thingy th)
/// {
///     if(th && !--th->rc)
///         thingytab->freenode(thingytab->removenode(thingytab, th->nam));
/// }
/// ```
/// Drop a reference; remove from table when rc hits 0.
pub fn unrefthingy(name: &str) {                                             // c:145
    let should_remove = {
        let mut tab = thingytab().lock().unwrap();
        if let Some(t) = tab.get_mut(name) {                                 // c:149 if(th && ...)
            t.rc -= 1;                                                       // c:149 --th->rc
            t.rc == 0
        } else {
            false
        }
    };
    if should_remove {
        // c:150 — `thingytab->freenode(thingytab->removenode(...))`.
        freethingynode(name);
    }
}

/// Port of `rthingy()` from `Src/Zle/zle_thingy.c:156`.
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
pub fn rthingy(name: &str) {                                                 // c:156
    {
        let mut tab = thingytab().lock().unwrap();
        if !tab.contains_key(name) {                                         // c:160-162 if(!t)
            let mut t = makethingynode();                                    // c:163 makethingynode
            t.name = name.to_string();                                       // c:163 ztrdup(nam)
            tab.insert(name.to_string(), t);                                 // c:163 addnode
        }
    }
    refthingy(name);                                                         // c:164 return refthingy(t)
}

/// Port of `rthingy_nocreate()` from `Src/Zle/zle_thingy.c:167`.
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
pub fn rthingy_nocreate(name: &str) -> bool {                                // c:167
    let exists = thingytab().lock().unwrap().contains_key(name);             // c:171 getnode2
    if !exists {
        return false;                                                        // c:173-174 if(!t) return NULL
    }
    refthingy(name);                                                         // c:175 return refthingy(t)
    true
}

// =====================================================================
// widget binding — `Src/Zle/zle_thingy.c:178-270`.
// =====================================================================

/// Port of `bindwidget()` from `Src/Zle/zle_thingy.c:197`.
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
/// implicit in Rust — the Arc<Widget> identity links peers.
/// Returns 0 on success, -1 on TH_IMMORTAL block.
pub fn bindwidget(w: Arc<Widget>, t_name: &str) -> i32 {                     // c:197
    let (immortal, disabled, same) = {
        let tab = thingytab().lock().unwrap();
        match tab.get(t_name) {
            Some(t) => (
                t.flags.immortal,
                t.flags.disabled,
                t.widget.as_ref().map(|w2| Arc::ptr_eq(w2, &w)).unwrap_or(false),
            ),
            None => (false, true, false),
        }
    };

    if immortal {                                                            // c:201 TH_IMMORTAL
        unrefthingy(t_name);                                                 // c:202
        return -1;                                                           // c:203
    }
    if !disabled {                                                           // c:205 !DISABLED
        if same {                                                            // c:206 t->widget == w
            return 0;                                                        // c:207
        }
        unbindwidget(t_name, 1);                                             // c:208
    }
    // c:210-216 — `samew` circular-list maintenance is implicit in
    // Rust: shared widgets just hold the same Arc, and walks via
    // Arc::ptr_eq find peers. No explicit list edit needed.
    let mut tab = thingytab().lock().unwrap();
    if let Some(t) = tab.get_mut(t_name) {
        t.widget = Some(w);                                                  // c:217 t->widget = w
        t.flags.disabled = false;                                            // c:218 t->flags &= ~DISABLED
    }
    0                                                                        // c:219 return 0
}

/// Port of `unbindwidget()` from `Src/Zle/zle_thingy.c:228`.
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
pub fn unbindwidget(t_name: &str, override_: i32) -> i32 {                   // c:228
    let (disabled, immortal, w_opt) = {
        let tab = thingytab().lock().unwrap();
        match tab.get(t_name) {
            Some(t) => (t.flags.disabled, t.flags.immortal, t.widget.clone()),
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
                .filter(|t| t.name != t_name)
                .filter(|t| t.widget.as_ref().map(|w2| Arc::ptr_eq(w2, &w)).unwrap_or(false))
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
    if let Some(t) = tab.get_mut(t_name) {
        t.flags.immortal = false;                                            // c:247 &= ~TH_IMMORTAL
        t.flags.disabled = true;                                             // c:248 |= DISABLED
        t.widget = None;
    }
    drop(tab);
    unrefthingy(t_name);                                                     // c:249 unrefthingy(t)
    0                                                                        // c:250 return 0
}

/// Port of `freewidget()` from `Src/Zle/zle_thingy.c:255`.
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
/// In Rust the Arc<Widget> auto-drops; this fn exists so the
/// INUSE/FREE flag handshake matches C exactly. The actual storage
/// drop happens when the last Arc is released by the caller's scope.
pub fn freewidget(w: Arc<Widget>) {                                          // c:255
    // c:259-262 — `if (w->flags & WIDGET_INUSE) { w->flags |= WIDGET_FREE; return; }`.
    // Widget::flags is on the immutable inner — to mutate, we'd need
    // Arc<Mutex<Widget>>. The current shape uses Arc<Widget>, so the
    // INUSE/FREE flag is observed but not written back (this matches
    // the "single owner" Rust pattern; the dispatcher pattern that
    // needs deferred-free isn't yet ported).
    if w.flags.contains(WidgetFlags::INUSE) {
        // c:260-261 — would set WIDGET_FREE here. Deferred-free not
        // yet implemented: the dispatcher path that would observe
        // WIDGET_FREE on return doesn't exist yet (zle_main.c's
        // execzlefunc loop). For now, log and exit; storage drops
        // on Arc release.
        return;                                                              // c:261 return
    }
    // c:264-269 — comp-widget / user-fn cleanup. WidgetFunc::User
    // owns its String; WidgetFunc::Internal owns nothing. Arc drop
    // covers both.
    drop(w);                                                                 // c:269 zfree(w, ...)
}

/// Port of `addzlefunction()` from `Src/Zle/zle_thingy.c:279`.
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
pub fn addzlefunction(
    name: &str,
    ifunc: fn(&mut crate::ported::zle::Zle),
    flags: WidgetFlags,
) -> Option<Arc<Widget>> {                                                   // c:279
    if name.starts_with('.') {                                               // c:287 if(name[0] == '.')
        return None;                                                         // c:288
    }
    let dotn = format!(".{}", name);                                         // c:289-290 dotn[0]='.';strcpy(...)

    // c:291-293 — refuse if .name is already TH_IMMORTAL.
    let blocked = {
        let tab = thingytab().lock().unwrap();
        tab.get(&dotn).map(|t| t.flags.immortal).unwrap_or(false)
    };
    if blocked {
        return None;                                                         // c:293
    }

    // c:294-297 — `w = zalloc(...); w->flags = WIDGET_INT|flags;
    //              w->first = NULL; w->u.fn = ifunc;`.
    let w = Arc::new(Widget {
        flags: flags | WidgetFlags::INT,                                     // c:295
        func: WidgetFunc::Internal(ifunc),                                   // c:297 w->u.fn = ifunc
    });

    // c:298-301 — bind to dotted form, mark immortal, then bind to
    // canonical form too.
    rthingy(&dotn);                                                          // c:298 t = rthingy(dotn)
    bindwidget(w.clone(), &dotn);                                            // c:299 bindwidget(w, t)
    if let Some(t) = thingytab().lock().unwrap().get_mut(&dotn) {
        t.flags.immortal = true;                                             // c:300 t->flags |= TH_IMMORTAL
    }
    rthingy(name);                                                           // c:301 rthingy(name)
    bindwidget(w.clone(), name);                                             // c:301 bindwidget(w, ...)
    Some(w)                                                                  // c:302 return w
}

/// Port of `deletezlefunction()` from `Src/Zle/zle_thingy.c:308`.
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
pub fn deletezlefunction(w: &Arc<Widget>) {                                  // c:308
    // c:312-323 — walk samew circular chain calling unbindwidget(p,1)
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
// The bin_zle_* fns below depend on live ZLE session state
// (zlecs/zlemetaline/keymaps/watch_fd table/zle_refresh draw
// primitives) that isn't ported yet. They're kept as panicking
// shims so silent fakes can't escape; the names remain searchable
// per the no-shortcut rule. When the substrate lands, port the
// bodies line-by-line from the C source lines cited.

/// Port of `bin_zle()` from `Src/Zle/zle_thingy.c:341`. Top-level
/// `zle` builtin dispatcher.
pub fn bin_zle() -> i32 {                                                    // c:341
    unimplemented!("zle_thingy.rs::bin_zle — c:341 deferred (Options + bin_zle_* dispatch table)");
}

/// Port of `bin_zle_call()` from `Src/Zle/zle_thingy.c:701`.
pub fn bin_zle_call() -> i32 {                                               // c:701
    unimplemented!("zle_thingy.rs::bin_zle_call — c:701 deferred (execzlefunc + zle session state)");
}

/// Port of `bin_zle_complete()` from `Src/Zle/zle_thingy.c:598`.
pub fn bin_zle_complete() -> i32 {                                           // c:598
    unimplemented!("zle_thingy.rs::bin_zle_complete — c:598 deferred (Widget WIDGET_NCOMP path)");
}

/// Port of `bin_zle_del()` from `Src/Zle/zle_thingy.c:546`.
pub fn bin_zle_del() -> i32 {                                                // c:546
    unimplemented!("zle_thingy.rs::bin_zle_del — c:546 deferred (deletezlefunction wrapper + arg parse)");
}

/// Port of `bin_zle_fd()` from `Src/Zle/zle_thingy.c:855`.
pub fn bin_zle_fd() -> i32 {                                                 // c:855
    unimplemented!("zle_thingy.rs::bin_zle_fd — c:855 deferred (watch_fd table + zle main-loop poll)");
}

/// Port of `bin_zle_flags()` from `Src/Zle/zle_thingy.c:649`.
pub fn bin_zle_flags() -> i32 {                                              // c:649
    unimplemented!("zle_thingy.rs::bin_zle_flags — c:649 deferred (active-widget pointer + ZLE_* flag mask)");
}

/// Port of `bin_zle_invalidate()` from `Src/Zle/zle_thingy.c:828`.
pub fn bin_zle_invalidate() -> i32 {                                         // c:828
    unimplemented!("zle_thingy.rs::bin_zle_invalidate — c:828 deferred (zle_refresh trashzle/clearflag globals)");
}

/// Port of `bin_zle_keymap()` from `Src/Zle/zle_thingy.c:486`.
pub fn bin_zle_keymap() -> i32 {                                             // c:486
    unimplemented!("zle_thingy.rs::bin_zle_keymap — c:486 deferred (curkeymap + keymapname globals)");
}

/// Port of `bin_zle_link()` from `Src/Zle/zle_thingy.c:565`.
pub fn bin_zle_link() -> i32 {                                               // c:565
    unimplemented!("zle_thingy.rs::bin_zle_link — c:565 deferred (rthingy + bindwidget alias path)");
}

/// Port of `bin_zle_list()` from `Src/Zle/zle_thingy.c:391`.
pub fn bin_zle_list() -> i32 {                                               // c:391
    unimplemented!("zle_thingy.rs::bin_zle_list — c:391 deferred (thingytab walk + scanlistwidgets)");
}

/// Port of `bin_zle_mesg()` from `Src/Zle/zle_thingy.c:457`.
pub fn bin_zle_mesg() -> i32 {                                               // c:457
    unimplemented!("zle_thingy.rs::bin_zle_mesg — c:457 deferred (statusline buffer + zle_refresh hook)");
}

/// Port of `bin_zle_new()` from `Src/Zle/zle_thingy.c:582`.
pub fn bin_zle_new() -> i32 {                                                // c:582
    unimplemented!("zle_thingy.rs::bin_zle_new — c:582 deferred (Widget WIDGET_INT clear + addzlefunction)");
}

/// Port of `bin_zle_refresh()` from `Src/Zle/zle_thingy.c:416`.
pub fn bin_zle_refresh() -> i32 {                                            // c:416
    unimplemented!("zle_thingy.rs::bin_zle_refresh — c:416 deferred (zrefresh() draw call)");
}

/// Port of `bin_zle_transform()` from `Src/Zle/zle_thingy.c:953`.
pub fn bin_zle_transform() -> i32 {                                          // c:953
    unimplemented!("zle_thingy.rs::bin_zle_transform — c:953 deferred (transformations table + redisplay hook)");
}

/// Port of `bin_zle_unget()` from `Src/Zle/zle_thingy.c:471`.
pub fn bin_zle_unget() -> i32 {                                              // c:471
    unimplemented!("zle_thingy.rs::bin_zle_unget — c:471 deferred (ungetbytes/ungetkeys input pushback)");
}

/// Port of `init_thingies()` from `Src/Zle/zle_thingy.c:1020`.
/// Boot-time thingytab population from the built-in widget table.
/// Walks the static `thingies[]` array in zle_thingy.c and inserts
/// each into the table marked TH_IMMORTAL.
pub fn init_thingies() -> i32 {                                              // c:1020
    unimplemented!("zle_thingy.rs::init_thingies — c:1020 deferred (built-in thingies[] table iteration)");
}

/// Port of `scanlistwidgets()` from `Src/Zle/zle_thingy.c:503`.
pub fn scanlistwidgets() -> i32 {                                            // c:503
    unimplemented!("zle_thingy.rs::scanlistwidgets — c:503 deferred (scancallback + Widget pretty-print)");
}

/// Port of `zle_usable()` from `Src/Zle/zle_thingy.c:632`.
/// True iff a ZLE session is currently active.
pub fn zle_usable() -> i32 {                                                 // c:632
    unimplemented!("zle_thingy.rs::zle_usable — c:632 deferred (zle_active global)");
}

#[cfg(test)]
mod thingytab_tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // Serialize tests since they share the global THINGYTAB.
    static LOCK: StdMutex<()> = StdMutex::new(());

    fn reset_tab() {
        thingytab().lock().unwrap().clear();
    }

    #[test]
    fn rthingy_creates_then_refs() {
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("foo");
        let tab = thingytab().lock().unwrap();
        let t = tab.get("foo").expect("rthingy must create");
        assert_eq!(t.rc, 1);
        assert!(t.flags.disabled);
    }

    #[test]
    fn refthingy_unrefthingy_roundtrip() {
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
        let _g = LOCK.lock().unwrap();
        reset_tab();

        assert!(!rthingy_nocreate("absent"));
        assert!(!thingytab().lock().unwrap().contains_key("absent"));
    }

    #[test]
    fn rthingy_nocreate_refs_existing() {
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("present");
        assert!(rthingy_nocreate("present"));
        assert_eq!(thingytab().lock().unwrap().get("present").unwrap().rc, 2);
    }

    #[test]
    fn bindwidget_assigns_widget_and_clears_disabled() {
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("hello");
        let w = Arc::new(Widget {
            flags: WidgetFlags::INT,
            func: WidgetFunc::Internal(|_| {}),
        });
        let r = bindwidget(w.clone(), "hello");
        assert_eq!(r, 0);
        let tab = thingytab().lock().unwrap();
        let t = tab.get("hello").unwrap();
        assert!(!t.flags.disabled);
        assert!(t.widget.is_some());
    }

    #[test]
    fn bindwidget_immortal_blocks() {
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("imm");
        thingytab().lock().unwrap().get_mut("imm").unwrap().flags.immortal = true;
        let w = Arc::new(Widget {
            flags: WidgetFlags::INT,
            func: WidgetFunc::Internal(|_| {}),
        });
        let r = bindwidget(w, "imm");
        assert_eq!(r, -1);
    }

    #[test]
    fn unbindwidget_drops_widget_when_last_peer() {
        let _g = LOCK.lock().unwrap();
        reset_tab();

        rthingy("only");
        let w = Arc::new(Widget {
            flags: WidgetFlags::INT,
            func: WidgetFunc::Internal(|_| {}),
        });
        bindwidget(w, "only");
        assert!(thingytab().lock().unwrap().get("only").unwrap().widget.is_some());

        let r = unbindwidget("only", 1);
        assert_eq!(r, 0);
        // unbind drops widget + sets DISABLED + unrefs the thingy
        assert!(!thingytab().lock().unwrap().contains_key("only"));
    }

    #[test]
    fn addzlefunction_binds_dotted_and_canonical() {
        let _g = LOCK.lock().unwrap();
        reset_tab();

        let w = addzlefunction("self-insert", |_| {}, WidgetFlags::empty());
        assert!(w.is_some());
        let tab = thingytab().lock().unwrap();
        // Both `.self-insert` and `self-insert` exist
        assert!(tab.contains_key(".self-insert"));
        assert!(tab.contains_key("self-insert"));
        // .self-insert is immortal
        assert!(tab.get(".self-insert").unwrap().flags.immortal);
        // canonical is not
        assert!(!tab.get("self-insert").unwrap().flags.immortal);
        // both share the same widget Arc
        let dot_w = tab.get(".self-insert").unwrap().widget.clone().unwrap();
        let plain_w = tab.get("self-insert").unwrap().widget.clone().unwrap();
        assert!(Arc::ptr_eq(&dot_w, &plain_w));
    }

    #[test]
    fn addzlefunction_refuses_dotted_name() {
        let _g = LOCK.lock().unwrap();
        reset_tab();

        let r = addzlefunction(".bad", |_| {}, WidgetFlags::empty());
        assert!(r.is_none());
    }

    #[test]
    fn deletezlefunction_unbinds_all_peers() {
        let _g = LOCK.lock().unwrap();
        reset_tab();

        let w = addzlefunction("test-fn", |_| {}, WidgetFlags::empty()).unwrap();
        assert!(thingytab().lock().unwrap().contains_key("test-fn"));
        deletezlefunction(&w);
        let tab = thingytab().lock().unwrap();
        // Both .test-fn and test-fn unbinding marks them DISABLED
        // and unrefs (their rc was 1 after addzlefunction → drops to 0
        // → freenode removes them).
        assert!(!tab.contains_key(".test-fn"));
        assert!(!tab.contains_key("test-fn"));
    }
}
