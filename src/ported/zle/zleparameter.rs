//! ZLE parameter interface
//!
//! Port from zsh/Src/Zle/zleparameter.c (186 lines)
//!
//! Functions for the zlewidgets special parameter.                          // c:33
//! Functions for the zlekeymaps special parameter.                          // c:102
//!
//! Provides the special $widgets associative array and $keymaps parameter
//! that let shell scripts query ZLE's internal state.

use std::collections::HashMap;

#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_h::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
/// Port of `widgetstr(Widget w)` from `Src/Zle/zleparameter.c:37`.
/// ```c
/// static char *
/// widgetstr(Widget w)
/// {
///     if (!w)
///         return dupstring("undefined");
///     if (w->flags & WIDGET_INT)
///         return dupstring("builtin");
///     if (w->flags & WIDGET_NCOMP) {
///         char *t = (char *) zhalloc(13 + strlen(w->u.comp.wid) +
///                                    strlen(w->u.comp.func));
///         strcpy(t, "completion:");
///         strcat(t, w->u.comp.wid);
///         strcat(t, ":");
///         strcat(t, w->u.comp.func);
///         return t;
///     }
///     return dyncat("user:", w->u.fnnam);
/// }
/// ```
/// Discriminates on the WIDGET FLAGS, exactly as C does. The previous
/// Rust signature took `(name, is_user, is_completion)` — three
/// booleans the C function does not have and no production call site
/// ever supplied, so both `$widgets` readers hand-rolled their own
/// copy of this logic and drifted apart (`scanpmwidgets` still
/// compared target-name-vs-key, the exact mistake Bug #264 fixed in
/// `getpmwidgets`).
pub fn widgetstr(w: Option<&std::sync::Arc<crate::ported::zle::zle_h::widget>>) -> String {
    // c:37
    use crate::ported::zle::zle_h::{WidgetImpl, WIDGET_INT, WIDGET_NCOMP};
    let Some(w) = w else {
        return "undefined".to_string(); // c:39-40
    };
    if (w.flags & WIDGET_INT) != 0 {
        return "builtin".to_string(); // c:41-42
    }
    if (w.flags & WIDGET_NCOMP) != 0 {
        // c:43-52 — `"completion:" wid ":" func`.
        if let WidgetImpl::Comp { wid, func, .. } = &w.u {
            return format!("completion:{}:{}", wid, func);
        }
        return "builtin".to_string();
    }
    // c:54 — `dyncat("user:", w->u.fnnam)`.
    match &w.u {
        WidgetImpl::Internal(_) => "builtin".to_string(),
        WidgetImpl::UserFunc(fnnam) => format!("user:{}", fnnam),
        WidgetImpl::Comp { wid, func, .. } => format!("completion:{}:{}", wid, func),
    }
}

// Functions for the zlewidgets special parameter.                          // c:33
/// Port of `static HashNode getpmwidgets(UNUSED(HashTable ht), const char *name)`
/// from `Src/Zle/zleparameter.c:33-79`. Returns a Param with u_str
/// set to the widget's type label (`builtin` / `user:fn` /
/// `completion:fn`), or PM_UNSET if the widget name isn't in
/// `thingytab` (zle_thingy.c:60).
pub fn getpmwidgets(
    _ht: *mut crate::ported::zsh_h::HashTable,
    name: &str,
) -> Option<crate::ported::zsh_h::Param> {
    // c:33
    use crate::ported::zsh_h::{hashnode, param, Param, PM_READONLY, PM_SCALAR, PM_UNSET};
    // c:Src/Zle/zle_thingy.c:1022 init_thingies — populates thingytab
    // with the internal widget entries (every iwidgets.list name in
    // bare and dotted form). zshrs's non-interactive
    // script mode skips the zsh/zle module load that triggers this,
    // so `$widgets[accept-line]` returned empty until something
    // (bindkey, zle -l, etc.) forced lazy init. Trigger here so
    // direct `widgets` reads work. Idempotent — init_thingies'
    // per-name `contains_key` guard makes re-entry safe. Bug #264.
    static WIDGETS_PARAM_INIT: std::sync::Once = std::sync::Once::new();
    WIDGETS_PARAM_INIT.call_once(|| {
        crate::ported::zle::zle_thingy::init_thingies();
    });
    let mk = |u_str: String, extra: i32| -> Param {
        Box::new(param {
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags: PM_SCALAR as i32 | PM_READONLY as i32 | extra,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: Some(u_str),
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
        })
    };
    // c:Src/Zle/zleparameter.c:37-56 widgetstr — discriminate by
    // widget FLAGS, not name-vs-target comparison:
    //   - undefined widget        → "undefined"
    //   - WIDGET_INT flag         → "builtin"
    //   - WIDGET_NCOMP flag       → "completion:wid:func"
    //   - else (user function)    → "user:fnnam"
    // Bug #264 — the previous Rust port compared `target == name`
    // which made user widgets registered as `zle -N my-fn` (where
    // the function name equals the widget name) report as `builtin`.
    let label_opt = {
        let tab = crate::ported::zle::zle_thingy::thingytab().lock().ok();
        tab.and_then(|t| {
            // c:Src/Zle/zleparameter.c:70-76 —
            //   `if ((th = thingytab->getnode(thingytab, name)) &&
            //        !(th->flags & DISABLED))
            //        pm->u.str = widgetstr(th->widget);
            //    else { pm->u.str = dupstring(""); pm->node.flags |= PM_UNSET; }`
            // A thingy that only exists because something called
            // `rthingy` on an unknown name (`bindkey ^Xz no-such-widget`,
            // c:Src/Zle/zle_keymap.c:1042) is DISABLED and has no widget:
            // it must read back as an UNSET empty scalar, not as
            // "undefined". `widgetstr`'s `!w -> "undefined"` arm
            // (c:zleparameter.c:39-40) stays for a bound-but-widgetless
            // node, which the fixed table never produces.
            t.get(name)
                .filter(|th| (th.flags & crate::ported::zsh_h::DISABLED) == 0) // c:71
                .map(|th| widgetstr(th.widget.as_ref())) // c:72
        })
    };
    match label_opt {
        Some(label) => Some(mk(label, 0)),
        None => Some(mk(String::new(), PM_UNSET as i32)),
    }
}

/// Port of `static void scanpmwidgets(UNUSED(HashTable ht), ScanFunc func, int flags)`
/// from `Src/Zle/zleparameter.c:81-101`. Walks `thingytab` and invokes
/// the callback per entry with a transient Param whose `u_str` is
/// the type label.
pub fn scanpmwidgets(
    _ht: *mut crate::ported::zsh_h::HashTable,
    func: Option<crate::ported::zsh_h::ParamScanFunc>,
    flags: i32,
) {
    // c:81
    use crate::ported::zsh_h::{hashnode, param, PM_READONLY, PM_SCALAR};
    // Same lazy init as getpmwidgets — scanpmwidgets walks
    // thingytab too. Bug #264.
    static WIDGETS_SCAN_INIT: std::sync::Once = std::sync::Once::new();
    WIDGETS_SCAN_INIT.call_once(|| {
        crate::ported::zle::zle_thingy::init_thingies();
    });
    let f = match func {
        Some(f) => f,
        None => return,
    };
    // c:92-98 — `for (i = 0; i < thingytab->hsize; i++)
    //              for (hn = thingytab->nodes[i]; hn; hn = hn->next) {
    //                  pm.node.nam = hn->nam;
    //                  ... pm.u.str = widgetstr(((Thingy) hn)->widget);
    //                  func(&pm.node, flags); }`
    // EVERY node is yielded. Unlike `getpmwidgets` (c:70-71) the scan
    // applies no DISABLED gate, so a name that exists only because
    // `bindkey '^Xz' no-such-widget` called `rthingy`
    // (c:Src/Zle/zle_keymap.c:1042) is a key of `$widgets` even though
    // reading it by subscript gives an unset empty scalar.
    //
    // The port used to drop every widget-less thingy (`getwidgettarget`
    // returns None when `th.widget` is None → `continue`), which is why
    // `${#widgets[(I)zzq-*]}` read 0 where zsh reads 1: the `rthingy`
    // nodes were invisible to the ONLY reader that enumerates keys.
    // The label also hand-rolled a target-name comparison instead of
    // calling `widgetstr` (c:37), so a `zle -C` widget listed as
    // `user:_main_complete` instead of `completion:.wid:_main_complete`.
    let names = crate::ported::zle::zle_thingy::listwidgets();
    for name in &names {
        let label = {
            let tab = crate::ported::zle::zle_thingy::thingytab().lock().ok();
            match tab.as_ref().and_then(|t| t.get(name)) {
                Some(th) => widgetstr(th.widget.as_ref()), // c:97
                None => continue,
            }
        };
        let pm = param {
            node: hashnode {
                next: None,
                nam: name.clone(),
                flags: PM_SCALAR as i32 | PM_READONLY as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: Some(label),
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
        f(&pm, flags); // c:97 `func(&pm.node, flags);`
    }
}

// Functions for the zlekeymaps special parameter.                          // c:105
/// Port of `static char **keymapsgetfn(UNUSED(Param pm))` from
/// `Src/Zle/zleparameter.c:105-119`. Walks `keymapnamtab` and returns
/// every keymap name as a sorted `Vec<String>`.
pub fn keymapsgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> {
    // c:105
    // c:Src/Zle/zle_keymap.c:1224-1230 init_keymaps + default_bindings
    // populate keymapnamtab on zsh/zle module load. zshrs's
    // non-interactive script mode never autoloads zsh/zle, so
    // keymapnamtab stays empty until something triggers it (e.g. a
    // `bindkey` call). Trigger the same lazy init here so a bare
    // `${keymaps}` / `${(@k)keymaps}` read populates the standard
    // 9 keymaps. Idempotent — default_bindings is a no-op when
    // keymapnamtab already has entries. Bug #383.
    // Gate on emptiness, NOT a `Once`. `default_bindings()` rebuilds every
    // keymap from scratch and overwrites keymapnamtab — destructive, not
    // idempotent. A prior `bindkey` runs its own lazy init and may have
    // added user bindings; a blind second `default_bindings()` here would
    // discard them. So only init when nothing has populated keymapnamtab
    // yet — reading `${keymaps}` must never clobber keybindings a config set
    // before the read. (Found via the config-state parity harness: the
    // dump's `${(o)keymaps}` was wiping earlier `bindkey` state. Shared with
    // the bindkey-side gate in zle_keymap.rs.)
    if crate::ported::zle::zle_keymap::keymapnamtab()
        .lock()
        .map(|t| !t.contains_key("main"))
        .unwrap_or(false)
    {
        crate::ported::zle::zle_keymap::default_bindings();
    }
    // c:111-116 —
    //   p = ret = zhalloc((keymapnamtab->ct + 1) * sizeof(char *));
    //   for (i = 0; i < keymapnamtab->hsize; i++)
    //       for (hn = keymapnamtab->nodes[i]; hn; hn = hn->next)
    //           *p++ = dupstring(hn->nam);
    // A RAW BUCKET WALK — bucket 0..hsize-1, each chain head→tail. NOT
    // sorted: `zsh -f -c 'zmodload zsh/zleparameter; print -rl --
    // ${(k)keymaps}'` prints `visual viopp command .safe vicmd main
    // isearch viins emacs`, which is exactly this walk over the
    // 7-bucket table (c:155) with `default_bindings`' link order
    // (c:1449-1472). `keys()` is `hashtable_nodes`' port of that walk
    // (`Src/hashtable.c:420-434`), so the order comes out verbatim; the
    // port used to `sort()` here, which matched zsh on no line.
    crate::ported::zle::zle_keymap::keymapnamtab()
        .lock()
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default()
}

/// Port of `setup_(UNUSED(Module m))` from `Src/Zle/zleparameter.c:147`. C body
/// is `return 0;` (UNUSED `Module m`).
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn setup_() -> i32 {
    // c:147
    0 // c:154
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Zle/zleparameter.c:154`. C body
/// is `*features = featuresarray(m, &module_features); return 0;`.
/// Static-link path: 0.
/// WARNING: param names don't match C — Rust=() vs C=(m, features)
pub fn features_() -> i32 {
    // c:154
    0 // c:162
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Zle/zleparameter.c:162`. C body
/// is `return handlefeatures(m, &module_features, enables);`.
/// Static-link path: 0.
/// WARNING: param names don't match C — Rust=() vs C=(m, enables)
pub fn enables_() -> i32 {
    // c:162
    0 // c:169
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Zle/zleparameter.c:169`. C body is
/// `return 0;` (UNUSED `Module m`).
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn boot_() -> i32 {
    // c:169
    0 // c:176
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Zle/zleparameter.c:176`. C body
/// is `return setfeatureenables(m, &module_features, NULL);`.
/// Static-link path: 0.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn cleanup_() -> i32 {
    // c:176
    0 // c:183
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Zle/zleparameter.c:183`. C body
/// is `return 0;` (UNUSED `Module m`).
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn finish_() -> i32 {
    // c:183
    0 // c:183
}

/// Default builtin widget names for the $widgets parameter
pub const BUILTIN_WIDGETS: &[&str] = &[
    "accept-and-hold",
    "accept-and-infer-next-history",
    "accept-line",
    "accept-line-and-down-history",
    "backward-char",
    "backward-delete-char",
    "backward-kill-line",
    "backward-kill-word",
    "backward-word",
    "beep",
    "beginning-of-buffer-or-history",
    "beginning-of-history",
    "beginning-of-line",
    "beginning-of-line-hist",
    "capitalize-word",
    "clear-screen",
    "complete-word",
    "copy-prev-word",
    "copy-region-as-kill",
    "delete-char",
    "delete-char-or-list",
    "delete-word",
    "describe-key-briefly",
    "digit-argument",
    "down-case-word",
    "down-history",
    "down-line",
    "down-line-or-history",
    "down-line-or-search",
    "emacs-backward-word",
    "emacs-forward-word",
    "end-of-buffer-or-history",
    "end-of-history",
    "end-of-line",
    "end-of-line-hist",
    "exchange-point-and-mark",
    "execute-last-named-cmd",
    "execute-named-cmd",
    "expand-history",
    "expand-or-complete",
    "expand-or-complete-prefix",
    "expand-word",
    "forward-char",
    "forward-word",
    "get-line",
    "gosmacs-transpose-chars",
    "history-beginning-search-backward",
    "history-beginning-search-forward",
    "history-incremental-search-backward",
    "history-incremental-search-forward",
    "history-search-backward",
    "history-search-forward",
    "insert-last-word",
    "kill-buffer",
    "kill-line",
    "kill-region",
    "kill-whole-line",
    "kill-word",
    "list-choices",
    "list-expand",
    "magic-space",
    "menu-complete",
    "menu-expand-or-complete",
    "neg-argument",
    "overwrite-mode",
    "pound-insert",
    "push-input",
    "push-line",
    "push-line-or-edit",
    "quoted-insert",
    "bslashquote-line",
    "bslashquote-region",
    "read-command",
    "recursive-edit",
    "redisplay",
    "redo",
    "reset-prompt",
    "reverse-menu-complete",
    "run-help",
    "self-insert",
    "self-insert-unmeta",
    "send-break",
    "set-mark-command",
    "spell-word",
    "split-undo",
    "transpose-chars",
    "transpose-words",
    "undefined-key",
    "undo",
    "universal-argument",
    "up-case-word",
    "up-history",
    "up-line",
    "up-line-or-history",
    "up-line-or-search",
    "vi-add-eol",
    "vi-add-next",
    "vi-backward-blank-word",
    "vi-backward-char",
    "vi-backward-delete-char",
    "vi-backward-kill-word",
    "vi-backward-word",
    "vi-beginning-of-line",
    "vi-caps-lock-panic",
    "vi-change",
    "vi-change-eol",
    "vi-change-whole-line",
    "vi-cmd-mode",
    "vi-delete",
    "vi-delete-char",
    "vi-digit-or-beginning-of-line",
    "vi-down-line-or-history",
    "vi-end-of-line",
    "vi-fetch-history",
    "vi-find-next-char",
    "vi-find-next-char-skip",
    "vi-find-prev-char",
    "vi-find-prev-char-skip",
    "vi-first-non-blank",
    "vi-forward-blank-word",
    "vi-forward-blank-word-end",
    "vi-forward-char",
    "vi-forward-word",
    "vi-forward-word-end",
    "vi-goto-column",
    "vi-goto-mark",
    "vi-goto-mark-line",
    "vi-history-search-backward",
    "vi-history-search-forward",
    "vi-indent",
    "vi-insert",
    "vi-insert-bol",
    "vi-join",
    "vi-kill-eol",
    "vi-kill-line",
    "vi-match-bracket",
    "vi-open-line-above",
    "vi-open-line-below",
    "vi-oper-swap-case",
    "vi-pound-insert",
    "vi-put-after",
    "vi-put-before",
    "vi-quoted-insert",
    "vi-repeat-change",
    "vi-repeat-find",
    "vi-repeat-search",
    "vi-replace",
    "vi-replace-chars",
    "vi-rev-repeat-find",
    "vi-rev-repeat-search",
    "vi-set-buffer",
    "vi-set-mark",
    "vi-substitute",
    "vi-swap-case",
    "vi-undo-change",
    "vi-unindent",
    "vi-up-line-or-history",
    "vi-yank",
    "vi-yank-eol",
    "vi-yank-whole-line",
    "what-cursor-position",
    "where-is",
    "which-command",
    "yank",
    "yank-pop",
    "zap-to-char",
];

/// Default keymap names
pub const DEFAULT_KEYMAPS: &[&str] = &[
    "emacs", "viins", "vicmd", "viopp", "visual", "isearch", "command", "main", ".safe",
];

#[cfg(test)]
mod tests {
    use super::*;

    // `widgetstr` (c:37) takes a `Widget`, so these build the three
    // widget shapes C discriminates on: WIDGET_INT (`builtin`),
    // WIDGET_NCOMP (`completion:wid:func`) and a plain user function
    // (`user:fnnam`). The wid half of the completion label is fixed at
    // ".wid" so the expected strings stay readable.
    fn w_int() -> std::sync::Arc<crate::ported::zle::zle_h::widget> {
        std::sync::Arc::new(crate::ported::zle::zle_h::widget {
            flags: crate::ported::zle::zle_h::WIDGET_INT,
            first: None,
            u: crate::ported::zle::zle_h::WidgetImpl::Internal(|_| {
                crate::ported::zle::zle_misc::undefinedkey()
            }),
        })
    }

    fn w_user(fnnam: &str) -> std::sync::Arc<crate::ported::zle::zle_h::widget> {
        std::sync::Arc::new(crate::ported::zle::zle_h::widget {
            flags: 0,
            first: None,
            u: crate::ported::zle::zle_h::WidgetImpl::UserFunc(fnnam.to_string()),
        })
    }

    fn w_comp(func: &str) -> std::sync::Arc<crate::ported::zle::zle_h::widget> {
        std::sync::Arc::new(crate::ported::zle::zle_h::widget {
            flags: crate::ported::zle::zle_h::WIDGET_NCOMP,
            first: None,
            u: crate::ported::zle::zle_h::WidgetImpl::Comp {
                fn_: |_| crate::ported::zle::zle_misc::undefinedkey(),
                wid: ".wid".to_string(),
                func: func.to_string(),
            },
        })
    }

    #[test]
    fn test_widgetstr() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        use crate::ported::zle::zle_h::{widget, WidgetImpl, WIDGET_INT, WIDGET_NCOMP};
        use std::sync::Arc;
        // c:39-40 — `if (!w) return dupstring("undefined");`
        assert_eq!(widgetstr(None), "undefined");
        // c:41-42 — `if (w->flags & WIDGET_INT) return "builtin";`
        let int_w = Arc::new(widget {
            flags: WIDGET_INT,
            first: None,
            u: WidgetImpl::Internal(|_| crate::ported::zle::zle_misc::undefinedkey()),
        });
        assert_eq!(widgetstr(Some(&int_w)), "builtin");
        // c:54 — `dyncat("user:", w->u.fnnam)`. The label is the bound
        // FUNCTION name, not the widget name — `zle -N foo foo` and
        // `zle -N foo bar` are distinguishable in `$widgets`.
        let user_w = Arc::new(widget {
            flags: 0,
            first: None,
            u: WidgetImpl::UserFunc("my-fn".to_string()),
        });
        assert_eq!(widgetstr(Some(&user_w)), "user:my-fn");
        // c:43-52 — `"completion:" wid ":" func`, BOTH halves.
        let comp_w = Arc::new(widget {
            flags: WIDGET_NCOMP,
            first: None,
            u: WidgetImpl::Comp {
                wid: ".complete-word".to_string(),
                func: "_main_complete".to_string(),
                fn_: |_| crate::ported::zle::zle_misc::undefinedkey(),
            },
        });
        assert_eq!(
            widgetstr(Some(&comp_w)),
            "completion:.complete-word:_main_complete"
        );
    }

    #[test]
    fn test_getpmwidgets() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        // c:33 — unknown widget returns Param with PM_UNSET flag set
        // and empty u_str. Builtin widget population happens via the
        // host-side widget registry that integrating ZLE init does;
        // here we pin the no-thingytab-match path explicitly.
        use crate::ported::zsh_h::PM_UNSET;
        let pm = getpmwidgets(std::ptr::null_mut(), "definitely-not-a-widget")
            .expect("getpmwidgets always returns Some(Param)");
        assert!(pm.node.flags & PM_UNSET as i32 != 0, "PM_UNSET set");
        assert_eq!(pm.u_str.as_deref(), Some(""));
    }

    /// c:Src/Zle/zleparameter.c:92-98 — `scanpmwidgets` walks EVERY
    /// `thingytab` node and calls `func` on each; unlike `getpmwidgets`
    /// (c:70-71) it applies no DISABLED gate. So a name that exists
    /// only because `bindkey '^Xz' no-such-widget` called `rthingy`
    /// (c:Src/Zle/zle_keymap.c:1042) IS a key of `$widgets`, valued
    /// `undefined` per `widgetstr`'s `!w` arm (c:39-40), even though
    /// reading it by subscript yields an unset empty scalar.
    ///
    /// The port used to `continue` past any thingy with no widget,
    /// which made every `rthingy`-created name invisible to the ONLY
    /// reader that enumerates keys — `${#widgets[(I)zzq-*]}` read 0
    /// where zsh reads 1.
    #[test]
    fn scanpmwidgets_yields_widgetless_thingies_as_undefined() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        crate::ported::zle::zle_thingy::init_thingies();
        crate::ported::zle::zle_thingy::rthingy("zzq-scan-only");

        // The scan callback is a plain `fn`, so collect through a
        // process-global the test owns rather than a closure capture.
        static SEEN: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());
        fn collect(pm: &crate::ported::zsh_h::param, _flags: i32) {
            SEEN.lock()
                .unwrap()
                .push((pm.node.nam.clone(), pm.u_str.clone().unwrap_or_default()));
        }
        SEEN.lock().unwrap().clear();
        scanpmwidgets(std::ptr::null_mut(), Some(collect), 0);
        let seen = SEEN.lock().unwrap();

        let row = seen
            .iter()
            .find(|(n, _)| n == "zzq-scan-only")
            .expect("c:92-98 — the scan yields every node, DISABLED included");
        assert_eq!(row.1, "undefined", "c:39-40 — `widgetstr(NULL)`");
        assert!(
            seen.iter().any(|(n, v)| n == "accept-line" && v == "builtin"),
            "c:41-42 — an internal widget still reports `builtin`"
        );

        // Leave the table as we found it so sibling tests keep their
        // baseline (`unrefthingy` at rc 1 removes the node, c:147-150).
        drop(seen);
        crate::ported::zle::zle_thingy::unrefthingy("zzq-scan-only");
    }

    #[test]
    fn test_keymapsgetfn() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let keymaps = keymapsgetfn(std::ptr::null_mut());
        assert!(keymaps.contains(&"emacs".to_string()));
        assert!(keymaps.contains(&"vicmd".to_string()));
    }

    #[test]
    fn test_builtin_widget_count() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // zsh has ~160 builtin widgets
        assert!(BUILTIN_WIDGETS.len() > 150);
    }

    /// c:37 — `widgetstr` user form preserves the function name in
    /// the suffix so `${widgets[my-widget]}` reads `user:my-fn`.
    /// Pinning the suffix shape catches a regression that drops the
    /// function-name part (which scripts grep for to bind to widgets).
    #[test]
    fn widgetstr_user_form_carries_function_name_after_colon() {
        let _g = crate::test_util::global_state_lock();
        let s = widgetstr(Some(&w_user("a-fn")));
        let (kind, rest) = s.split_once(':').expect("missing colon");
        assert_eq!(kind, "user");
        assert_eq!(rest, "a-fn", "function-name suffix must round-trip");
    }

    /// c:37 — `widgetstr(Some(&w_comp(_)))` — both flags true. The C
    /// dispatch order is is_completion FIRST, so this branch yields
    /// "completion:..." not "user:...". Pin the precedence so a
    /// regen flipping branch order gets caught (would silently swap
    /// the type label for completion widgets).
    #[test]
    fn widgetstr_completion_wins_over_user_when_both_true() {
        let _g = crate::test_util::global_state_lock();
        let s = widgetstr(Some(&w_comp("foo")));
        assert!(
            s.starts_with("completion:"),
            "is_completion must dominate is_user, got: {}",
            s
        );
    }

    /// c:59 — `getpmwidgets` should NOT silently de-dup. If a user
    /// or completion widget shares a name with a builtin, the user/
    /// completion entry overwrites the builtin (HashMap semantics
    /// last-write-wins on equal keys). Pin the overwrite direction
    /// so a regen flipping insert order silently changes which type
    /// `${widgets[x]}` reports.
    // user-override-on-collision and bucket-coverage tests deleted —
    // the new C-faithful `getpmwidgets(*mut HashTable, &str) -> Option<Param>`
    // reads thingytab directly (one source of truth, no merge of
    // separate maps). The merge-order behavior they were pinning
    // no longer applies once user-widget registration goes through
    // thingytab.add().

    /// c:105 — `keymapsgetfn` returns owned Strings, not borrowed
    /// references — mutating the result must NOT affect keymapnamtab.
    #[test]
    fn keymapsgetfn_returns_independent_copies() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut out = keymapsgetfn(std::ptr::null_mut());
        let original_len = out.len();
        out.push("d".to_string());
        let again = keymapsgetfn(std::ptr::null_mut());
        // Mutation didn't affect the source registry.
        assert_eq!(again.len(), original_len);
        assert_eq!(out.len(), original_len + 1);
    }

    /// `BUILTIN_WIDGETS` must not contain duplicates — the C source's
    /// thingytab is keyed by name and would silently dedupe; the
    /// Rust hardcoded list must do the same proactively.
    #[test]
    fn builtin_widgets_has_no_duplicates() {
        let _g = crate::test_util::global_state_lock();
        let unique: std::collections::HashSet<_> = BUILTIN_WIDGETS.iter().copied().collect();
        assert_eq!(
            unique.len(),
            BUILTIN_WIDGETS.len(),
            "duplicate widget name in BUILTIN_WIDGETS — would corrupt $widgets"
        );
    }

    /// `BUILTIN_WIDGETS` entries must follow the `lowercase-with-
    /// hyphens` convention zsh's own widget names use. Catches a
    /// regression that adds an underscore-named or uppercase entry
    /// which couldn't be bound via `bindkey` without quoting.
    #[test]
    fn builtin_widgets_entries_are_kebab_case() {
        let _g = crate::test_util::global_state_lock();
        for w in BUILTIN_WIDGETS {
            assert!(!w.is_empty(), "empty widget name");
            for c in w.chars() {
                assert!(
                    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
                    "widget {:?} has non-kebab-case char {:?}",
                    w,
                    c
                );
            }
            assert!(
                !w.starts_with('-'),
                "widget {:?} starts with '-' — would parse as a flag",
                w
            );
            assert!(!w.ends_with('-'), "widget {:?} ends with '-'", w);
        }
    }

    /// `DEFAULT_KEYMAPS` must include the four POSIX-required
    /// names (emacs, viins, vicmd, main). zsh's startup expects
    /// each of these to exist; a regression that drops "main"
    /// would silently break every user's `bindkey -A main`.
    #[test]
    fn default_keymaps_includes_required_names() {
        let _g = crate::test_util::global_state_lock();
        for required in ["emacs", "viins", "vicmd", "main"] {
            assert!(
                DEFAULT_KEYMAPS.contains(&required),
                "DEFAULT_KEYMAPS missing required name: {}",
                required
            );
        }
    }

    /// c:147-183 — module-lifecycle stubs all return 0 in C.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(), 0);
        assert_eq!(boot_(), 0);
        assert_eq!(cleanup_(), 0);
        assert_eq!(finish_(), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests pinning Src/Zle/zleparameter.c contracts.
    // ═══════════════════════════════════════════════════════════════════

    /// c:37 — `widgetstr(Some(&w_int()))` returns "builtin"
    /// regardless of the name. Pins that the builtin branch ignores
    /// the function-name arg (matches C's `return "builtin"`).
    #[test]
    fn widgetstr_builtin_form_ignores_name() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(widgetstr(Some(&w_int())), "builtin");
        assert_eq!(widgetstr(Some(&w_int())), "builtin");
        assert_eq!(widgetstr(Some(&w_int())), "builtin");
    }

    /// c:37 — completion form preserves the function name suffix
    /// (parallel to widgetstr_user_form_carries_function_name).
    #[test]
    fn widgetstr_completion_form_carries_function_name() {
        let _g = crate::test_util::global_state_lock();
        let s = widgetstr(Some(&w_comp("_complete-foo")));
        // c:43-52 — the label is `"completion:" wid ":" func`, so the
        // function name is the LAST colon-separated field.
        assert_eq!(s, "completion:.wid:_complete-foo");
    }

    /// c:147 — `setup_()` returns 0 (split out from combined test).
    #[test]
    fn zleparameter_setup_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(), 0);
    }

    /// c:154 — `features_()` returns 0 (static-link path).
    #[test]
    fn zleparameter_features_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(features_(), 0);
    }

    /// c:162 — `enables_()` returns 0 (static-link path).
    #[test]
    fn zleparameter_enables_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(enables_(), 0);
    }

    /// c:33 — `getpmwidgets` sets PM_READONLY on every returned Param
    /// (the parameter shape exposes a string scalar the user must
    /// not assign to). PM_SCALAR is the 0-value sentinel ("no type
    /// bit set"), not a settable bit, so we only assert READONLY.
    /// Pin so any future refactor that drops READONLY would be
    /// caught (would let `widgets[x]=...` silently corrupt the table).
    #[test]
    fn getpmwidgets_unknown_widget_has_readonly_flag() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        use crate::ported::zsh_h::PM_READONLY;
        let pm = getpmwidgets(std::ptr::null_mut(), "no-such-widget").expect("always returns Some");
        assert!(pm.node.flags & PM_READONLY as i32 != 0, "PM_READONLY set");
    }

    /// c:33 — returned Param's `nam` field must round-trip the input
    /// name verbatim (callers index into thingytab by this name).
    #[test]
    fn getpmwidgets_param_name_round_trips_input() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let pm = getpmwidgets(std::ptr::null_mut(), "my-test-widget").expect("always returns Some");
        assert_eq!(pm.node.nam, "my-test-widget");
    }

    /// c:113-115 — `keymapsgetfn` emits the RAW `keymapnamtab` bucket
    /// walk (`for i in 0..hsize { for hn in nodes[i] }`), NOT a sorted
    /// list. With the 7-bucket table (`Src/Zle/zle_keymap.c:155`) and
    /// `default_bindings`' link order (c:1449-1472) that walk is what
    /// `zsh -f -c 'zmodload zsh/zleparameter; print -rl -- ${(k)keymaps}'`
    /// prints. This test previously asserted SORTED output, which is a
    /// property C does not have and which made `${(k)keymaps}` differ
    /// from zsh on every line; it now pins the C contract instead.
    #[test]
    fn keymapsgetfn_output_is_the_keymapnamtab_bucket_walk() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let names = keymapsgetfn(std::ptr::null_mut());
        let walk: Vec<String> = crate::ported::zle::zle_keymap::keymapnamtab()
            .lock()
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default();
        assert_eq!(
            names, walk,
            "keymapsgetfn must emit the keymapnamtab bucket walk verbatim"
        );
        // The nine keymaps `default_bindings` links come out in C's
        // bucket order for a 7-bucket front-inserting table — the exact
        // sequence `zsh -f -c 'zmodload zsh/zleparameter;
        // print -rl -- ${(k)keymaps}'` prints. Other keymaps the shared
        // test fixture registers are filtered out; their presence must
        // not perturb the relative order of these nine.
        let boot: Vec<&str> = names
            .iter()
            .map(String::as_str)
            .filter(|n| {
                matches!(
                    *n,
                    "visual"
                        | "viopp"
                        | "command"
                        | ".safe"
                        | "vicmd"
                        | "main"
                        | "isearch"
                        | "viins"
                        | "emacs"
                )
            })
            .collect();
        assert_eq!(
            boot,
            vec![
                "visual", "viopp", "command", ".safe", "vicmd", "main", "isearch", "viins",
                "emacs",
            ],
            "c:155 newhashtable(7) + c:1449-1472 link order"
        );
    }

    /// c:105 — `keymapsgetfn` output must not have duplicate names
    /// (keymapnamtab is hashed by name, so duplicates would indicate
    /// internal corruption).
    #[test]
    fn keymapsgetfn_output_has_no_duplicates() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let names = keymapsgetfn(std::ptr::null_mut());
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(
            unique.len(),
            names.len(),
            "keymapsgetfn returned duplicate keymap names"
        );
    }

    /// c:33 — `getpmwidgets` returns Some for every input (the C
    /// equivalent constructs the Param unconditionally; missing-from-
    /// table is conveyed via PM_UNSET, never via NULL Param).
    #[test]
    fn getpmwidgets_always_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for name in &["x", "", "ANY"] {
            assert!(
                getpmwidgets(std::ptr::null_mut(), name).is_some(),
                "getpmwidgets({:?}) returned None — C never returns NULL",
                name
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zleparameter.c
    // c:37 widgetstr / c:33 getpmwidgets / c:105 keymapsgetfn /
    // c:147-180 lifecycle.
    // ═══════════════════════════════════════════════════════════════════

    /// c:37 — `widgetstr(Some(&w_int()))` always returns "builtin"
    /// regardless of name (name is ignored in the builtin arm).
    #[test]
    fn widgetstr_builtin_ignores_all_names() {
        for name in &["", "anything", "with spaces", "包含中文", "x\ny"] {
            assert_eq!(
                widgetstr(Some(&w_int())),
                "builtin",
                "builtin arm must ignore name {:?}",
                name
            );
        }
    }

    /// c:37 — `widgetstr` output starts with one of three known prefixes.
    #[test]
    fn widgetstr_output_has_canonical_prefix() {
        let outs = [
            widgetstr(Some(&w_int())),
            widgetstr(Some(&w_user("y"))),
            widgetstr(Some(&w_comp("z"))),
        ];
        for s in &outs {
            assert!(
                s == "builtin" || s.starts_with("user:") || s.starts_with("completion:"),
                "widgetstr output {:?} not in known set",
                s
            );
        }
    }

    /// c:37 — `widgetstr(Some(&w_user(name)))` always emits `user:<name>`,
    /// with colon at position 4 (`user`+`:`).
    #[test]
    fn widgetstr_user_colon_is_position_four() {
        let s = widgetstr(Some(&w_user("abc")));
        assert_eq!(s.find(':'), Some(4));
        assert!(s.starts_with("user:"));
    }

    /// c:37 — `widgetstr(Some(&w_comp(name)))` always emits `completion:<name>`.
    #[test]
    fn widgetstr_completion_colon_is_position_ten() {
        let s = widgetstr(Some(&w_comp("abc")));
        assert_eq!(s.find(':'), Some(10));
        assert!(s.starts_with("completion:"));
    }

    /// c:37 — empty name still produces a well-formed label.
    #[test]
    fn widgetstr_empty_name_each_arm_well_formed() {
        assert_eq!(widgetstr(Some(&w_int())), "builtin");
        assert_eq!(widgetstr(Some(&w_user(""))), "user:");
        assert_eq!(widgetstr(Some(&w_comp(""))), "completion:.wid:");
    }

    /// c:37 — `widgetstr` is a pure function (no side effects across
    /// repeated calls).
    #[test]
    fn widgetstr_is_pure() {
        for _ in 0..50 {
            assert_eq!(widgetstr(Some(&w_int())), "builtin");
            assert_eq!(widgetstr(Some(&w_user("y"))), "user:y");
            assert_eq!(widgetstr(Some(&w_comp("z"))), "completion:.wid:z");
        }
    }

    /// c:105 — `keymapsgetfn(null)` is null-safe.
    #[test]
    fn keymapsgetfn_null_pm_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _ = keymapsgetfn(std::ptr::null_mut());
    }

    /// c:105 — `keymapsgetfn` is deterministic (sorted) across calls.
    #[test]
    fn keymapsgetfn_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let first = keymapsgetfn(std::ptr::null_mut());
        for _ in 0..5 {
            assert_eq!(
                keymapsgetfn(std::ptr::null_mut()),
                first,
                "keymapsgetfn must be deterministic"
            );
        }
    }

    /// c:33 — `getpmwidgets` for unknown name sets PM_UNSET bit on
    /// the returned Param (so `${widgets[unknown]}` reports unset).
    #[test]
    fn getpmwidgets_unknown_sets_pm_unset_flag() {
        use crate::ported::zsh_h::PM_UNSET;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let pm =
            getpmwidgets(std::ptr::null_mut(), "zshrs_never_a_widget_xyz").expect("always Some");
        assert_ne!(
            pm.node.flags & PM_UNSET as i32,
            0,
            "unknown widget must have PM_UNSET bit set"
        );
    }

    /// c:81 — `scanpmwidgets` with None callback is a safe no-op
    /// (matches C: NULL func → early return).
    #[test]
    fn scanpmwidgets_none_callback_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        scanpmwidgets(std::ptr::null_mut(), None, 0);
    }

    /// c:147-180 — lifecycle hooks survive interleaved load/unload.
    #[test]
    fn zleparameter_interleaved_lifecycle_safe() {
        // Pattern: setup → boot → cleanup → finish → setup → ...
        for _ in 0..5 {
            assert_eq!(setup_(), 0);
            assert_eq!(boot_(), 0);
            assert_eq!(cleanup_(), 0);
            assert_eq!(finish_(), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zleparameter.c
    // c:30 widgetstr / c:142 keymapsgetfn / c:155-198 lifecycle type pins
    // ═══════════════════════════════════════════════════════════════════

    /// c:30 — `widgetstr` returns String (compile-time type pin).
    #[test]
    fn widgetstr_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = widgetstr(Some(&w_int()));
    }

    /// c:142 — `keymapsgetfn` returns Vec<String> (compile-time type pin).
    #[test]
    fn keymapsgetfn_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = keymapsgetfn(std::ptr::null_mut());
    }

    /// c:155 — `setup_` returns i32 (compile-time type pin).
    #[test]
    fn zleparameter_setup_returns_i32_type() {
        let _: i32 = setup_();
    }

    /// c:164 — `features_` returns i32.
    #[test]
    fn zleparameter_features_returns_i32_type() {
        let _: i32 = features_();
    }

    /// c:173 — `enables_` returns i32.
    #[test]
    fn zleparameter_enables_returns_i32_type() {
        let _: i32 = enables_();
    }

    /// c:181 — `boot_` returns i32.
    #[test]
    fn zleparameter_boot_returns_i32_type() {
        let _: i32 = boot_();
    }

    /// c:190 — `cleanup_` returns i32.
    #[test]
    fn zleparameter_cleanup_returns_i32_type() {
        let _: i32 = cleanup_();
    }

    /// c:198 — `finish_` returns i32.
    #[test]
    fn zleparameter_finish_returns_i32_type() {
        let _: i32 = finish_();
    }

    /// c:155-198 — every lifecycle hook returns 0 (success).
    #[test]
    fn zleparameter_all_lifecycle_hooks_return_zero() {
        assert_eq!(setup_(), 0);
        assert_eq!(features_(), 0);
        assert_eq!(enables_(), 0);
        assert_eq!(boot_(), 0);
        assert_eq!(cleanup_(), 0);
        assert_eq!(finish_(), 0);
    }

    /// c:155 — setup idempotent (callable repeatedly).
    #[test]
    fn zleparameter_setup_idempotent_full_sweep() {
        for _ in 0..10 {
            assert_eq!(setup_(), 0);
        }
    }

    /// c:198 — finish idempotent.
    #[test]
    fn zleparameter_finish_idempotent_full_sweep() {
        for _ in 0..10 {
            assert_eq!(finish_(), 0);
        }
    }

    /// c:30 — `widgetstr` for builtin-mode is pure across inputs.
    #[test]
    fn widgetstr_builtin_is_pure() {
        let _g = crate::test_util::global_state_lock();
        for name in ["", "a", "fwd", "back-word", "complete-word"] {
            let first = widgetstr(Some(&w_int()));
            for _ in 0..3 {
                assert_eq!(
                    widgetstr(Some(&w_int())),
                    first,
                    "widgetstr(builtin widget) must be pure for {:?}",
                    name
                );
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zleparameter.c
    // c:30 widgetstr / c:37 dispatch / c:81 scanpmwidgets /
    // c:105 keymapsgetfn / c:147-198 lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:30 — `widgetstr(name,false,false)` returns "builtin" regardless of name.
    #[test]
    fn widgetstr_builtin_ignores_name() {
        let _g = crate::test_util::global_state_lock();
        for name in ["", "a", "x", "self-insert", "fancy-name"] {
            assert_eq!(
                widgetstr(Some(&w_int())),
                "builtin",
                "builtin mode ignores name; got name={:?}",
                name
            );
        }
    }

    /// c:32-34 — `is_completion=true` wins over `is_user=true`
    /// (completion checked first in the dispatch order).
    #[test]
    fn widgetstr_completion_precedence_over_user() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            widgetstr(Some(&w_comp("foo"))),
            "completion:.wid:foo",
            "WIDGET_NCOMP is checked before the user-function arm"
        );
    }

    /// c:35 — `is_user=true,is_completion=false` formats `user:NAME`.
    #[test]
    fn widgetstr_user_format() {
        let _g = crate::test_util::global_state_lock();
        for name in ["", "a", "complete-word", "_complete"] {
            assert_eq!(widgetstr(Some(&w_user(name))), format!("user:{}", name));
        }
    }

    /// c:33 — `is_completion=true` formats `completion:NAME`.
    #[test]
    fn widgetstr_completion_format() {
        let _g = crate::test_util::global_state_lock();
        for name in ["", "_main_complete", "_complete_help", "x"] {
            assert_eq!(widgetstr(Some(&w_comp(name))), format!("completion:.wid:{}", name));
        }
    }

    /// c:30 — `widgetstr` return type is owned String (compile-time pin).
    #[test]
    fn widgetstr_return_type_is_owned_string() {
        let _g = crate::test_util::global_state_lock();
        let _: String = widgetstr(Some(&w_int()));
        let _: String = widgetstr(Some(&w_user("x")));
        let _: String = widgetstr(Some(&w_comp("x")));
    }

    /// c:81 — `scanpmwidgets` with None callback returns void (safe no-op).
    #[test]
    fn scanpmwidgets_none_callback_safe() {
        let _g = crate::test_util::global_state_lock();
        scanpmwidgets(std::ptr::null_mut(), None, 0);
        scanpmwidgets(std::ptr::null_mut(), None, 0xff);
    }

    /// c:105 — `keymapsgetfn` returns Vec<String> (compile-time pin, alt name).
    #[test]
    fn keymapsgetfn_returns_vec_string_compile_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = keymapsgetfn(std::ptr::null_mut());
    }

    /// c:105-119 — `keymapsgetfn` output is the `keymapnamtab` bucket
    /// order, which is NOT sorted: C's loop is
    /// `for (i = 0; i < hsize; i++) for (hn = nodes[i]; hn; hn = hn->next)`
    /// (c:113-115), and `zsh -f` prints `visual viopp command .safe vicmd
    /// main isearch viins emacs` for the nine boot keymaps. This test
    /// used to assert SORTED output, a property C does not have; it now
    /// pins the property C does have — the emitted order is exactly the
    /// bucket walk, with no reordering applied on top.
    #[test]
    fn keymapsgetfn_matches_bucket_walk_clone_compare() {
        let _g = crate::test_util::global_state_lock();
        let v = keymapsgetfn(std::ptr::null_mut());
        let walk: Vec<String> = crate::ported::zle::zle_keymap::keymapnamtab()
            .lock()
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default();
        assert_eq!(v, walk, "keymapsgetfn must not reorder the bucket walk");
    }

    /// c:105 — `keymapsgetfn` is deterministic (purely a snapshot read).
    #[test]
    fn keymapsgetfn_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = keymapsgetfn(std::ptr::null_mut());
        for _ in 0..5 {
            assert_eq!(
                keymapsgetfn(std::ptr::null_mut()),
                first,
                "must be deterministic across calls"
            );
        }
    }

    /// c:33 — `getpmwidgets` returns Some unconditionally (PM_UNSET signals
    /// not-found; C path mirrors this with `pm.flags |= PM_UNSET`).
    #[test]
    fn getpmwidgets_returns_some_for_unknown_name() {
        let _g = crate::test_util::global_state_lock();
        let pm = getpmwidgets(std::ptr::null_mut(), "definitely-not-a-widget-xyz123");
        assert!(
            pm.is_some(),
            "even unknown name returns Some (flags carry PM_UNSET)"
        );
    }

    /// c:155-198 — every lifecycle hook is independently idempotent
    /// (fine-grained vs the bulk full-sweep test).
    #[test]
    fn zleparameter_features_idempotent() {
        for _ in 0..10 {
            assert_eq!(features_(), 0);
        }
    }

    /// c:162 — `enables_` idempotent.
    #[test]
    fn zleparameter_enables_idempotent() {
        for _ in 0..10 {
            assert_eq!(enables_(), 0);
        }
    }

    /// c:181 — `boot_` idempotent.
    #[test]
    fn zleparameter_boot_idempotent() {
        for _ in 0..10 {
            assert_eq!(boot_(), 0);
        }
    }

    /// c:190 — `cleanup_` idempotent.
    #[test]
    fn zleparameter_cleanup_idempotent() {
        for _ in 0..10 {
            assert_eq!(cleanup_(), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Zle/zleparameter.c
    // c:37 widgetstr / c:33 getpmwidgets / c:91 scanpmwidgets /
    // c:142 keymapsgetfn / c:155-198 module lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:37 — `widgetstr` empty name → `user:` (degenerate but C-faithful).
    #[test]
    fn widgetstr_empty_name_user_format() {
        assert_eq!(
            widgetstr(Some(&w_user(""))),
            "user:",
            "empty name still gets user: prefix"
        );
    }

    /// c:37 — `widgetstr` empty name + completion → `completion:`.
    #[test]
    fn widgetstr_empty_name_completion_format() {
        assert_eq!(
            widgetstr(Some(&w_comp(""))),
            "completion:.wid:",
            "empty func still emits both colons"
        );
    }

    /// c:37 — `widgetstr` is_completion=true overrides is_user=true (C path
    /// checks WIDGET_INT first via WC_ZLE_TYPE — completion wins).
    #[test]
    fn widgetstr_completion_beats_both_flags() {
        assert_eq!(
            widgetstr(Some(&w_comp("foo"))),
            "completion:.wid:foo",
            "WIDGET_NCOMP takes precedence over the user-function arm"
        );
    }

    /// c:37 — `widgetstr` builtin path: both flags false.
    #[test]
    fn widgetstr_both_flags_false_returns_builtin() {
        assert_eq!(widgetstr(Some(&w_int())), "builtin");
    }

    /// c:37 — `widgetstr` deterministic across calls.
    #[test]
    fn widgetstr_deterministic_repeated_calls() {
        for _ in 0..10 {
            assert_eq!(widgetstr(Some(&w_user("foo"))), "user:foo");
            assert_eq!(widgetstr(Some(&w_comp("bar"))), "completion:.wid:bar");
            assert_eq!(widgetstr(Some(&w_int())), "builtin");
        }
    }

    /// c:37 — `widgetstr` preserves name verbatim (no escaping, no quoting).
    #[test]
    fn widgetstr_preserves_special_chars_in_name() {
        assert_eq!(widgetstr(Some(&w_user("a b\tc"))), "user:a b\tc");
        assert_eq!(widgetstr(Some(&w_user("\\n"))), "user:\\n");
    }

    /// c:33 — `getpmwidgets("")` empty name doesn't panic.
    #[test]
    fn getpmwidgets_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = getpmwidgets(std::ptr::null_mut(), "");
    }

    /// c:91 — `scanpmwidgets` with None callback is a no-op (safe).
    #[test]
    fn scanpmwidgets_none_repeat_safe() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            scanpmwidgets(std::ptr::null_mut(), None, 0);
        }
    }

    /// c:142 — `keymapsgetfn` returns Vec<String> (compile-time pin, alt).
    #[test]
    fn keymapsgetfn_returns_vec_string_compile_pin_alt() {
        let _: Vec<String> = keymapsgetfn(std::ptr::null_mut());
    }

    /// c:142 — `keymapsgetfn` doesn't panic on null pm (alt pin).
    #[test]
    fn keymapsgetfn_null_pm_no_panic_alt() {
        let _g = crate::test_util::global_state_lock();
        let _ = keymapsgetfn(std::ptr::null_mut());
    }

    /// c:155 — `setup_` idempotent.
    #[test]
    fn zleparameter_setup_idempotent() {
        for _ in 0..10 {
            assert_eq!(setup_(), 0);
        }
    }

    /// c:198 — `finish_` idempotent.
    #[test]
    fn zleparameter_finish_idempotent() {
        for _ in 0..10 {
            assert_eq!(finish_(), 0);
        }
    }

    /// c:155-198 — full lifecycle sequence safe + all return 0.
    #[test]
    fn zleparameter_full_lifecycle_sequence_safe() {
        assert_eq!(setup_(), 0);
        assert_eq!(features_(), 0);
        assert_eq!(enables_(), 0);
        assert_eq!(boot_(), 0);
        assert_eq!(cleanup_(), 0);
        assert_eq!(finish_(), 0);
    }
}
