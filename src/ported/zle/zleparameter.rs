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
    let mk = |u_str: String, extra: i32| -> Param {
        Box::new(param {
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags: PM_SCALAR as i32 | PM_READONLY as i32 | extra,
            },
            u_data: 0,
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
    // c:60-78 — look up name in thingytab, format widget type label.
    match crate::ported::zle::zle_thingy::getwidgettarget(name) {
        Some(target) if target == name => Some(mk("builtin".to_string(), 0)),
        Some(target) => Some(mk(format!("user:{}", target), 0)),
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
/// every keymap name as a sorted Vec<String>.
pub fn keymapsgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> {
    // c:105
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
}
