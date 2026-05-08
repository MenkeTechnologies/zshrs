//! ZLE parameter interface
//!
//! Port from zsh/Src/Zle/zleparameter.c (186 lines)
//!
//! Provides the special $widgets associative array and $keymaps parameter
//! that let shell scripts query ZLE's internal state.

use std::collections::HashMap;

/// Format a widget's type label as `$widgets[name]` would show it.
/// Port of `widgetstr()` from Src/Zle/zleparameter.c. The C source
/// emits "builtin" for `iwidgets.list` entries, "user:fnname" for
/// `zle -N` widgets, and "completion:fnname" for `zle -C` ones —
/// matched here verbatim so shell scripts that grep `$widgets`
/// keep working.
pub fn widgetstr(name: &str, is_user: bool, is_completion: bool) -> String {
    if is_completion {
        format!("completion:{}", name)
    } else if is_user {
        format!("user:{}", name)
    } else {
        "builtin".to_string()
    }
}

/// Build the `$widgets` associative array — the snapshot consulted by
/// shell-side `${(k)widgets}` enumeration.
/// Port of `getpmwidgets()` from Src/Zle/zleparameter.c. The C source
/// walks `thingytab` (zle_thingy.c:60 `createthingytab`); we union
/// the static built-in slice with the user + completion widget maps
/// and emit the same per-entry type label widgetstr() produces.
pub fn getpmwidgets(
    builtin_widgets: &[&str],
    user_widgets: &HashMap<String, String>,
    completion_widgets: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut result = HashMap::new();

    for &name in builtin_widgets {
        result.insert(name.to_string(), "builtin".to_string());
    }

    for (name, func) in user_widgets {
        result.insert(name.to_string(), format!("user:{}", func));
    }

    for (name, func) in completion_widgets {
        result.insert(name.to_string(), format!("completion:{}", func));
    }

    result
}

/// Iterate over every widget for the parameter scan path (used by
/// `${(kv)widgets}` and zsh's `print -l ${(k)widgets}`).
/// Port of `scanpmwidgets()` from Src/Zle/zleparameter.c. The C
/// source walks the same thingytab the getpmwidgets path uses but
/// invokes the parameter-scan callback on each entry instead of
/// allocating the full hash.
pub fn scanpmwidgets<F>(
    builtin_widgets: &[&str],
    user_widgets: &HashMap<String, String>,
    completion_widgets: &HashMap<String, String>,
    mut callback: F,
) where
    F: FnMut(&str, &str),
{
    for &name in builtin_widgets {
        callback(name, "builtin");
    }
    for (name, func) in user_widgets {
        callback(name, &format!("user:{}", func));
    }
    for (name, func) in completion_widgets {
        callback(name, &format!("completion:{}", func));
    }
}

/// Build the `$keymaps` array — list of every named keymap.
/// Port of `keymapsgetfn()` from Src/Zle/zleparameter.c. The C
/// source walks `keymapnamtab` (zle_keymap.c:153
/// `createkeymapnamtab`); we surface the host-supplied slice
/// directly since our keymap registry already exposes a Vec view.
pub fn keymapsgetfn(keymaps: &[&str]) -> Vec<String> {
    keymaps.iter().map(|s| s.to_string()).collect()
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
        assert_eq!(widgetstr("self-insert", false, false), "builtin");
        assert_eq!(widgetstr("my-widget", true, false), "user:my-widget");
        assert_eq!(widgetstr("my-comp", false, true), "completion:my-comp");
    }

    #[test]
    fn test_getpmwidgets() {
        let user = HashMap::new();
        let comp = HashMap::new();
        let widgets = getpmwidgets(&["accept-line", "backward-char"], &user, &comp);
        assert_eq!(widgets.get("accept-line"), Some(&"builtin".to_string()));
        assert_eq!(widgets.len(), 2);
    }

    #[test]
    fn test_keymapsgetfn() {
        let keymaps = keymapsgetfn(DEFAULT_KEYMAPS);
        assert!(keymaps.contains(&"emacs".to_string()));
        assert!(keymaps.contains(&"vicmd".to_string()));
    }

    #[test]
    fn test_builtin_widget_count() {
        // zsh has ~160 builtin widgets
        assert!(BUILTIN_WIDGETS.len() > 150);
    }
}

/// Port of `boot_()` from Src/Zle/zleparameter.c:169. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn boot_() -> i32 { 0 }

/// Port of `cleanup_()` from Src/Zle/zleparameter.c:176. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cleanup_() -> i32 { 0 }

/// Port of `enables_()` from Src/Zle/zleparameter.c:162. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn enables_() -> i32 { 0 }

/// Port of `features_()` from Src/Zle/zleparameter.c:154. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn features_() -> i32 { 0 }

/// Port of `finish_()` from Src/Zle/zleparameter.c:183. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn finish_() -> i32 { 0 }

/// Port of `setup_()` from Src/Zle/zleparameter.c:147. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn setup_() -> i32 { 0 }
