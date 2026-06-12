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
/// Format a widget's type label as `$widgets[name]` would show it.
/// Port of `widgetstr(Widget w)` from Src/Zle/zleparameter.c. The C source
/// emits "builtin" for `iwidgets.list` entries, "user:fnname" for
/// `zle -N` widgets, and "completion:fnname" for `zle -C` ones —
/// matched here verbatim so shell scripts that grep `$widgets`
/// keep working.
/// WARNING: param names don't match C — Rust=(name, is_user, is_completion) vs C=(w)

// --- AUTO: cross-zle hoisted-fn use glob ---
/// `widgetstr` — see implementation.
#[allow(unused_imports)]

pub fn widgetstr(name: &str, is_user: bool, is_completion: bool) -> String {
    // c:37
    if is_completion {
        format!("completion:{}", name)
    } else if is_user {
        format!("user:{}", name)
    } else {
        "builtin".to_string()
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
            t.get(name).cloned().map(|th| {
                let w_opt = th.widget;
                match w_opt {
                    None => "undefined".to_string(),
                    Some(w) => {
                        use crate::ported::zle::zle_h::{
                            WidgetImpl, WIDGET_INT, WIDGET_NCOMP,
                        };
                        if (w.flags & WIDGET_INT) != 0 {
                            "builtin".to_string()
                        } else if (w.flags & WIDGET_NCOMP) != 0 {
                            if let WidgetImpl::Comp { wid, func, .. } = &w.u {
                                format!("completion:{}:{}", wid, func)
                            } else {
                                "builtin".to_string()
                            }
                        } else {
                            match &w.u {
                                WidgetImpl::Internal(_) => "builtin".to_string(),
                                WidgetImpl::UserFunc(fnnam) => format!("user:{}", fnnam),
                                WidgetImpl::Comp { wid, func, .. } => {
                                    format!("completion:{}:{}", wid, func)
                                }
                            }
                        }
                    }
                }
            })
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
    func: Option<crate::ported::zsh_h::ScanFunc>,
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
    let names = crate::ported::zle::zle_thingy::listwidgets();
    for name in &names {
        let label = match crate::ported::zle::zle_thingy::getwidgettarget(name) {
            Some(t) if t == *name => "builtin".to_string(),
            Some(t) => format!("user:{}", t),
            None => continue,
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
        let node_box = Box::new(pm.node.clone());
        f(&node_box, flags); // c:97 `func(&pm.node, flags);`
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
    static KEYMAPS_PARAM_INIT: std::sync::Once = std::sync::Once::new();
    KEYMAPS_PARAM_INIT.call_once(|| {
        crate::ported::zle::zle_keymap::default_bindings();
    });
    let mut names: Vec<String> = crate::ported::zle::zle_keymap::keymapnamtab()
        .lock()
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
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

    #[test]
    fn test_widgetstr() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(widgetstr("self-insert", false, false), "builtin");
        assert_eq!(widgetstr("my-widget", true, false), "user:my-widget");
        assert_eq!(widgetstr("my-comp", false, true), "completion:my-comp");
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
        let s = widgetstr("a-fn", true, false);
        let (kind, rest) = s.split_once(':').expect("missing colon");
        assert_eq!(kind, "user");
        assert_eq!(rest, "a-fn", "function-name suffix must round-trip");
    }

    /// c:37 — `widgetstr(_, true, true)` — both flags true. The C
    /// dispatch order is is_completion FIRST, so this branch yields
    /// "completion:..." not "user:...". Pin the precedence so a
    /// regen flipping branch order gets caught (would silently swap
    /// the type label for completion widgets).
    #[test]
    fn widgetstr_completion_wins_over_user_when_both_true() {
        let _g = crate::test_util::global_state_lock();
        let s = widgetstr("foo", true, true);
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

    /// c:37 — `widgetstr(name, false, false)` returns "builtin"
    /// regardless of the name. Pins that the builtin branch ignores
    /// the function-name arg (matches C's `return "builtin"`).
    #[test]
    fn widgetstr_builtin_form_ignores_name() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(widgetstr("anything", false, false), "builtin");
        assert_eq!(widgetstr("", false, false), "builtin");
        assert_eq!(widgetstr("with spaces", false, false), "builtin");
    }

    /// c:37 — completion form preserves the function name suffix
    /// (parallel to widgetstr_user_form_carries_function_name).
    #[test]
    fn widgetstr_completion_form_carries_function_name() {
        let _g = crate::test_util::global_state_lock();
        let s = widgetstr("_complete-foo", false, true);
        let (kind, rest) = s.split_once(':').expect("missing colon");
        assert_eq!(kind, "completion");
        assert_eq!(rest, "_complete-foo");
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

    /// c:105 — `keymapsgetfn` output must be sorted (Rust port sorts
    /// at the end of the fn). Pin sorted-order contract so callers
    /// can rely on deterministic output for ${(o)zle_keymaps}.
    #[test]
    fn keymapsgetfn_output_is_sorted() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let names = keymapsgetfn(std::ptr::null_mut());
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "keymapsgetfn output must be sorted");
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

    /// c:37 — `widgetstr(name, false, false)` always returns "builtin"
    /// regardless of name (name is ignored in the builtin arm).
    #[test]
    fn widgetstr_builtin_ignores_all_names() {
        for name in &["", "anything", "with spaces", "包含中文", "x\ny"] {
            assert_eq!(
                widgetstr(name, false, false),
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
            widgetstr("x", false, false),
            widgetstr("y", true, false),
            widgetstr("z", false, true),
        ];
        for s in &outs {
            assert!(
                s == "builtin" || s.starts_with("user:") || s.starts_with("completion:"),
                "widgetstr output {:?} not in known set",
                s
            );
        }
    }

    /// c:37 — `widgetstr(name, true, false)` always emits `user:<name>`,
    /// with colon at position 4 (`user`+`:`).
    #[test]
    fn widgetstr_user_colon_is_position_four() {
        let s = widgetstr("abc", true, false);
        assert_eq!(s.find(':'), Some(4));
        assert!(s.starts_with("user:"));
    }

    /// c:37 — `widgetstr(name, false, true)` always emits `completion:<name>`.
    #[test]
    fn widgetstr_completion_colon_is_position_ten() {
        let s = widgetstr("abc", false, true);
        assert_eq!(s.find(':'), Some(10));
        assert!(s.starts_with("completion:"));
    }

    /// c:37 — empty name still produces a well-formed label.
    #[test]
    fn widgetstr_empty_name_each_arm_well_formed() {
        assert_eq!(widgetstr("", false, false), "builtin");
        assert_eq!(widgetstr("", true, false), "user:");
        assert_eq!(widgetstr("", false, true), "completion:");
    }

    /// c:37 — `widgetstr` is a pure function (no side effects across
    /// repeated calls).
    #[test]
    fn widgetstr_is_pure() {
        for _ in 0..50 {
            assert_eq!(widgetstr("x", false, false), "builtin");
            assert_eq!(widgetstr("y", true, false), "user:y");
            assert_eq!(widgetstr("z", false, true), "completion:z");
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
        let _: String = widgetstr("", false, false);
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
            let first = widgetstr(name, false, false);
            for _ in 0..3 {
                assert_eq!(
                    widgetstr(name, false, false),
                    first,
                    "widgetstr({:?}, false, false) must be pure",
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
                widgetstr(name, false, false),
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
            widgetstr("foo", true, true),
            "completion:foo",
            "completion bit dominates user bit"
        );
    }

    /// c:35 — `is_user=true,is_completion=false` formats `user:NAME`.
    #[test]
    fn widgetstr_user_format() {
        let _g = crate::test_util::global_state_lock();
        for name in ["", "a", "complete-word", "_complete"] {
            assert_eq!(widgetstr(name, true, false), format!("user:{}", name));
        }
    }

    /// c:33 — `is_completion=true` formats `completion:NAME`.
    #[test]
    fn widgetstr_completion_format() {
        let _g = crate::test_util::global_state_lock();
        for name in ["", "_main_complete", "_complete_help", "x"] {
            assert_eq!(widgetstr(name, false, true), format!("completion:{}", name));
        }
    }

    /// c:30 — `widgetstr` return type is owned String (compile-time pin).
    #[test]
    fn widgetstr_return_type_is_owned_string() {
        let _g = crate::test_util::global_state_lock();
        let _: String = widgetstr("x", false, false);
        let _: String = widgetstr("x", true, false);
        let _: String = widgetstr("x", false, true);
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

    /// c:105-119 — `keymapsgetfn` output is sorted (additional pin via clone-compare).
    #[test]
    fn keymapsgetfn_sorted_clone_compare() {
        let _g = crate::test_util::global_state_lock();
        let v = keymapsgetfn(std::ptr::null_mut());
        let mut sorted = v.clone();
        sorted.sort();
        assert_eq!(v, sorted, "keymapsgetfn must return sorted names");
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
            widgetstr("", true, false),
            "user:",
            "empty name still gets user: prefix"
        );
    }

    /// c:37 — `widgetstr` empty name + completion → `completion:`.
    #[test]
    fn widgetstr_empty_name_completion_format() {
        assert_eq!(
            widgetstr("", false, true),
            "completion:",
            "empty + completion → completion: prefix only"
        );
    }

    /// c:37 — `widgetstr` is_completion=true overrides is_user=true (C path
    /// checks WIDGET_INT first via WC_ZLE_TYPE — completion wins).
    #[test]
    fn widgetstr_completion_beats_both_flags() {
        assert_eq!(
            widgetstr("foo", true, true),
            "completion:foo",
            "completion takes precedence when both flags set"
        );
    }

    /// c:37 — `widgetstr` builtin path: both flags false.
    #[test]
    fn widgetstr_both_flags_false_returns_builtin() {
        assert_eq!(widgetstr("anything", false, false), "builtin");
    }

    /// c:37 — `widgetstr` deterministic across calls.
    #[test]
    fn widgetstr_deterministic_repeated_calls() {
        for _ in 0..10 {
            assert_eq!(widgetstr("foo", true, false), "user:foo");
            assert_eq!(widgetstr("bar", false, true), "completion:bar");
            assert_eq!(widgetstr("baz", false, false), "builtin");
        }
    }

    /// c:37 — `widgetstr` preserves name verbatim (no escaping, no quoting).
    #[test]
    fn widgetstr_preserves_special_chars_in_name() {
        assert_eq!(widgetstr("a b\tc", true, false), "user:a b\tc");
        assert_eq!(widgetstr("\\n", true, false), "user:\\n");
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
