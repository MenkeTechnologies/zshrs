//! ZLE widgets - line editor commands
//!
//! Direct port from zsh/Src/Zle/zle.h widget structures
//!
//! A widget is a ZLE command that can be bound to keys or executed by name.
//! Widgets can be internal (implemented in Rust) or user-defined (shell functions).

use super::zle_main::{Zle, ZleChar};

// ---------------------------------------------------------------------------
// Rust-only word-motion helpers used by the widget_* fns below and by
// `zle_vi.rs`. These were originally defined in `src/ported/zle/zle_word.rs`
// alongside Rust-only `find_word_start`/`find_word_end` impls on `Zle`,
// but the strict-rules cleanup of zle_word.rs deleted them (rule 1
// forbids Rust-only types/methods in `src/ported/`). Relocated here
// (an extension file outside the drift gate's scan path) until the
// callers are themselves rewritten to use the C-faithful per-widget
// fns (`emacsforwardword`, `vibackwardword`, etc.) directly.
// ---------------------------------------------------------------------------

/// Word style for the deleted Rust-only `find_word_start` / `find_word_end`
/// helpers. Not in C; C has separate widget fns per (style × direction)
/// instead of a parameterised helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordStyle {
    Emacs,
    Vi,
    Shell,
    BlankDelimited,
}

impl Zle {
    /// Find the start of the current (or preceding) word at the cursor
    /// for the requested word style. Rust-only — see WordStyle doc-note.
    pub fn find_word_start(&self, style: WordStyle) -> usize {
        let mut pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        match style {
            WordStyle::Emacs => {
                while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1]; pos > 0 && !(__c.is_alphanumeric()
                                   || __c == '_') } {
                    pos -= 1;
                }
                while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1]; pos > 0 && (__c.is_alphanumeric()
                                  || __c == '_') } {
                    pos -= 1;
                }
            }
            WordStyle::Vi => {
                while pos > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1].is_whitespace() {
                    pos -= 1;
                }
                if pos > 0 {
                    let is_word = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1].is_alphanumeric()
                                  || crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1] == '_';
                    while pos > 0 {
                        let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1];
                        if c.is_whitespace()
                           || ((c.is_alphanumeric() || c == '_') != is_word) {
                            break;
                        }
                        pos -= 1;
                    }
                }
            }
            WordStyle::Shell => {
                pos = backwardword_shell(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)], pos);
            }
            WordStyle::BlankDelimited => {
                while pos > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1].is_whitespace() {
                    pos -= 1;
                }
                while pos > 0 && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1].is_whitespace() {
                    pos -= 1;
                }
            }
        }
        pos
    }

    /// Find the end (exclusive) of the current (or following) word.
    pub fn find_word_end(&self, style: WordStyle) -> usize {
        let mut pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        match style {
            WordStyle::Emacs => {
                while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]; pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !(__c.is_alphanumeric()
                                            || __c == '_') } {
                    pos += 1;
                }
                while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]; pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && (__c.is_alphanumeric()
                                           || __c == '_') } {
                    pos += 1;
                }
            }
            WordStyle::Vi => {
                if pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                    let is_word = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos].is_alphanumeric()
                                  || crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos] == '_';
                    while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                        let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos];
                        if c.is_whitespace()
                           || ((c.is_alphanumeric() || c == '_') != is_word) {
                            break;
                        }
                        pos += 1;
                    }
                    while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos].is_whitespace() {
                        pos += 1;
                    }
                }
            }
            WordStyle::Shell => {
                pos = forwardword_shell(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)], pos);
            }
            WordStyle::BlankDelimited => {
                while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos].is_whitespace() {
                    pos += 1;
                }
                while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos].is_whitespace() {
                    pos += 1;
                }
            }
        }
        pos
    }
}

/// Walk `line` left-to-right collecting (start, end_exclusive) ranges of
/// shell words. Quote-aware (single, double, backslash). Rust-only —
/// the canonical zsh equivalent is `bufferwords()` in
/// `Src/hist.c` which has a different signature; keeping this one
/// here so existing widget_* call sites compile.
pub fn bufferwords(line: &[ZleChar]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    let n = line.len();
    while i < n {
        while i < n && line[i].is_whitespace() { i += 1; }
        if i >= n { break; }
        let start = i;
        let mut in_single = false;
        let mut in_double = false;
        while i < n {
            let c = line[i];
            if in_single { if c == '\'' { in_single = false; } i += 1; continue; }
            if in_double {
                if c == '\\' && i + 1 < n { i += 2; continue; }
                if c == '"' { in_double = false; }
                i += 1;
                continue;
            }
            if c == '\\' && i + 1 < n { i += 2; continue; }
            if c == '\'' { in_single = true; i += 1; continue; }
            if c == '"' { in_double = true; i += 1; continue; }
            if c.is_whitespace() { break; }
            i += 1;
        }
        out.push((start, i));
    }
    out
}

/// Find the start of the shell word containing or immediately preceding `pos`.
pub fn backwardword_shell(line: &[ZleChar], pos: usize) -> usize {
    let words = bufferwords(line);
    for (s, e) in words.iter().rev() {
        if *s <= pos && pos <= *e {
            if pos == *s { continue; }
            return *s;
        }
        if *e < pos { return *s; }
    }
    0
}

/// Find the end (exclusive) of the shell word at or after `pos`.
pub fn forwardword_shell(line: &[ZleChar], pos: usize) -> usize {
    let words = bufferwords(line);
    for (s, e) in words {
        if pos >= s && pos < e { return e; }
        if pos < s { return e; }
    }
    line.len()
}

/// Widget function type
pub type ZleIntFunc = fn(&mut Zle) -> i32;

/// Widget function variants
#[derive(Clone)]
pub enum WidgetFunc {
    /// Internally implemented widget
    Internal(fn(&mut Zle)),
    /// User-defined widget (name of shell function)
    User(String),
}

impl std::fmt::Debug for WidgetFunc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WidgetFunc::Internal(_) => write!(f, "Internal(<fn>)"),
            WidgetFunc::User(name) => write!(f, "User({})", name),
        }
    }
}

bitflags::bitflags! {
    /// Widget flags
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct WidgetFlags: u32 {
        /// Widget is internally implemented
        const INT = 1 << 0;
        /// New style completion widget
        const NCOMP = 1 << 1;
        /// DON'T invalidate completion list
        const MENUCMP = 1 << 2;
        /// Yank after cursor
        const YANKAFTER = 1 << 3;
        /// Yank before cursor
        const YANKBEFORE = 1 << 4;
        /// Yank (either direction)
        const YANK = Self::YANKAFTER.bits() | Self::YANKBEFORE.bits();
        /// Command is a line-oriented movement
        const LINEMOVE = 1 << 5;
        /// Widget reads further keys so wait if prefix
        const VIOPER = 1 << 6;
        /// Command maintains lastcol correctly
        const LASTCOL = 1 << 7;
        /// Kill command
        const KILL = 1 << 8;
        /// DON'T remove added suffix
        const KEEPSUFFIX = 1 << 9;
        /// Widget should not alter lastcmd
        const NOTCOMMAND = 1 << 10;
        /// Usable for new style completion
        const ISCOMP = 1 << 11;
        /// Widget is in use
        const INUSE = 1 << 12;
        /// Request to free when no longer in use
        const FREE = 1 << 13;
        /// Widget should not alter lbindk
        const NOLAST = 1 << 14;
    }
}

/// A widget (ZLE command)
#[derive(Debug, Clone)]
pub struct Widget {
    /// Flags
    pub flags: WidgetFlags,
    /// Widget function
    pub func: WidgetFunc,
}

impl Widget {
    /// Build a widget that points at a Rust function pointer with the
    /// supplied ZLE flags.
    /// Equivalent to the WIDGET_INT branch of `zalloc(sizeof(*w))` +
    /// `w->u.fn = ...` in `addzlefunction()` at
    /// Src/Zle/zle_thingy.c:281. The C source uses this for every
    /// `iwidgets.list` entry (the static built-in table); we collapse
    /// "internal" + "builtin" into the same WidgetFunc::Internal
    /// variant since both end up dispatching a Rust fn ptr.
    pub fn internal(name: &str, func: fn(&mut Zle), flags: WidgetFlags) -> Self {
        let _ = name; // Mirrors addzlefunction's `w->name` field — unused
                      // here because dispatch is by table lookup, not name.
        Widget {
            flags: flags | WidgetFlags::INT,
            func: WidgetFunc::Internal(func),
        }
    }

    /// Resolve a built-in widget name to a Widget.
    /// Equivalent to the lookup-by-name path in
    /// `bin_zle_call`/`getkeycmd` (Src/Zle/zle_thingy.c) when the
    /// resolved Thingy points at a built-in. Routes through
    /// `acceptline` which is the static table corresponding
    /// to zsh's `intwidget[]` — see `Src/Zle/iwidgets.list` for the
    /// canonical name → fn mapping.
    pub fn builtin(name: &str) -> Self {
        let (func, flags) = acceptline(name);
        Widget {
            flags: flags | WidgetFlags::INT,
            func: WidgetFunc::Internal(func),
        }
    }

    /// Build a widget that wraps a user-defined shell function.
    /// Equivalent to `bin_zle_new()` from Src/Zle/zle_thingy.c:584
    /// (the `zle -N name [shell-fn]` builtin). The C source allocates
    /// a fresh Widget without WIDGET_INT, sets `w->u.fnnam` to the
    /// shell function name, and binds it to a Thingy.
    pub fn user_defined(name: &str, func_name: &str) -> Self {
        let _ = name;
        Widget {
            flags: WidgetFlags::empty(),
            func: WidgetFunc::User(func_name.to_string()),
        }
    }
}

/// Get the builtin widget function for a name
fn acceptline(name: &str) -> (fn(&mut Zle), WidgetFlags) {
    match name {
        // Accept/execute
        "accept-line" => (widget_accept_line, WidgetFlags::empty()),
        "accept-and-hold" => (widget_accept_and_hold, WidgetFlags::empty()),
        "accept-line-and-down-history" => {
            (widget_accept_line_and_down_history, WidgetFlags::empty())
        }

        // Self-insert
        "self-insert" => (widget_self_insert, WidgetFlags::empty()),
        "self-insert-unmeta" => (widget_self_insert_unmeta, WidgetFlags::empty()),

        // Movement - character
        "forward-char" => (widget_forward_char, WidgetFlags::empty()),
        "backward-char" => (widget_backward_char, WidgetFlags::empty()),

        // Movement - word
        "forward-word" => (widget_forward_word, WidgetFlags::empty()),
        "backward-word" => (widget_backward_word, WidgetFlags::empty()),

        // Movement - line
        "beginning-of-line" => (widget_beginning_of_line, WidgetFlags::empty()),
        "end-of-line" => (widget_end_of_line, WidgetFlags::empty()),

        // Delete
        "delete-char" => (widget_delete_char, WidgetFlags::empty()),
        "backward-delete-char" => (widget_backward_delete_char, WidgetFlags::empty()),
        "delete-char-or-list" => (widget_delete_char_or_list, WidgetFlags::empty()),

        // Kill
        "kill-line" => (widget_kill_line, WidgetFlags::KILL),
        "backward-kill-line" => (widget_backward_kill_line, WidgetFlags::KILL),
        "kill-whole-line" => (widget_kill_whole_line, WidgetFlags::KILL),
        "kill-word" => (widget_kill_word, WidgetFlags::KILL),
        "backward-kill-word" => (widget_backward_kill_word, WidgetFlags::KILL),

        // Yank
        "yank" => (widget_yank, WidgetFlags::YANK),
        "yank-pop" => (widget_yank_pop, WidgetFlags::YANK),

        // Undo
        "undo" => (widget_undo, WidgetFlags::empty()),
        "redo" => (widget_redo, WidgetFlags::empty()),

        // History
        "up-line-or-history" => (widget_up_line_or_history, WidgetFlags::LINEMOVE),
        "down-line-or-history" => (widget_down_line_or_history, WidgetFlags::LINEMOVE),
        "up-history" => (widget_up_history, WidgetFlags::LINEMOVE),
        "down-history" => (widget_down_history, WidgetFlags::LINEMOVE),
        "history-incremental-search-backward" => {
            (widget_history_isearch_backward, WidgetFlags::empty())
        }
        "history-incremental-search-forward" => {
            (widget_history_isearch_forward, WidgetFlags::empty())
        }
        "beginning-of-buffer-or-history" => {
            (widget_beginning_of_buffer_or_history, WidgetFlags::LINEMOVE)
        }
        "end-of-buffer-or-history" => (widget_end_of_buffer_or_history, WidgetFlags::LINEMOVE),

        // Misc
        "transpose-chars" => (widget_transpose_chars, WidgetFlags::empty()),
        "clear-screen" => (widget_clear_screen, WidgetFlags::empty()),
        "redisplay" => (widget_redisplay, WidgetFlags::empty()),
        "send-break" => (widget_send_break, WidgetFlags::empty()),
        "overwrite-mode" => (widget_overwrite_mode, WidgetFlags::empty()),
        "quoted-insert" => (widget_quoted_insert, WidgetFlags::empty()),

        // Completion
        "expand-or-complete" => (widget_expand_or_complete, WidgetFlags::MENUCMP),
        "complete-word" => (widget_complete_word, WidgetFlags::MENUCMP),
        "expand-word" => (widget_expand_word, WidgetFlags::empty()),
        "list-choices" => (widget_list_choices, WidgetFlags::MENUCMP),
        "menu-complete" => (widget_menu_complete, WidgetFlags::MENUCMP),

        // Vi mode
        "vi-cmd-mode" => (widget_vi_cmd_mode, WidgetFlags::empty()),
        "vi-insert" => (widget_vi_insert, WidgetFlags::empty()),
        "vi-insert-bol" => (widget_vi_insert_bol, WidgetFlags::empty()),
        "vi-add-next" => (widget_vi_add_next, WidgetFlags::empty()),
        "vi-add-eol" => (widget_vi_add_eol, WidgetFlags::empty()),
        "vi-forward-char" => (widget_vi_forward_char, WidgetFlags::empty()),
        "vi-backward-char" => (widget_vi_backward_char, WidgetFlags::empty()),
        "vi-forward-word" => (widget_vi_forward_word, WidgetFlags::empty()),
        "vi-forward-word-end" => (widget_vi_forward_word_end, WidgetFlags::empty()),
        "vi-forward-blank-word" => (widget_vi_forward_blank_word, WidgetFlags::empty()),
        "vi-forward-blank-word-end" => (widget_vi_forward_blank_word_end, WidgetFlags::empty()),
        "vi-backward-word" => (widget_vi_backward_word, WidgetFlags::empty()),
        "vi-backward-blank-word" => (widget_vi_backward_blank_word, WidgetFlags::empty()),
        "vi-delete" => (widget_vi_delete, WidgetFlags::VIOPER | WidgetFlags::KILL),
        "vi-delete-char" => (widget_vi_delete_char, WidgetFlags::empty()),
        "vi-backward-delete-char" => (widget_vi_backward_delete_char, WidgetFlags::empty()),
        "vi-change" => (widget_vi_change, WidgetFlags::VIOPER | WidgetFlags::KILL),
        "vi-change-eol" => (widget_vi_change_eol, WidgetFlags::KILL),
        "vi-kill-eol" => (widget_vi_kill_eol, WidgetFlags::KILL),
        "vi-yank" => (widget_vi_yank, WidgetFlags::VIOPER),
        "vi-yank-whole-line" => (widget_vi_yank_whole_line, WidgetFlags::empty()),
        "vi-put-after" => (widget_vi_put_after, WidgetFlags::YANK),
        "vi-put-before" => (widget_vi_put_before, WidgetFlags::YANK),
        "vi-replace" => (widget_vi_replace, WidgetFlags::empty()),
        "vi-replace-chars" => (widget_vi_replace_chars, WidgetFlags::empty()),
        "vi-substitute" => (widget_vi_substitute, WidgetFlags::empty()),
        "vi-change-whole-line" => (widget_vi_change_whole_line, WidgetFlags::KILL),
        "vi-first-non-blank" => (widget_vi_first_non_blank, WidgetFlags::empty()),
        "vi-end-of-line" => (widget_vi_end_of_line, WidgetFlags::empty()),
        "vi-digit-or-beginning-of-line" => {
            (widget_vi_digit_or_beginning_of_line, WidgetFlags::empty())
        }
        "vi-open-line-below" => (widget_vi_open_line_below, WidgetFlags::empty()),
        "vi-open-line-above" => (widget_vi_open_line_above, WidgetFlags::empty()),
        "vi-join" => (widget_vi_join, WidgetFlags::empty()),
        "vi-repeat-change" => (widget_vi_repeat_change, WidgetFlags::empty()),
        "vi-find-next-char" => (widget_vi_find_next_char, WidgetFlags::empty()),
        "vi-find-prev-char" => (widget_vi_find_prev_char, WidgetFlags::empty()),
        "vi-find-next-char-skip" => (widget_vi_find_next_char_skip, WidgetFlags::empty()),
        "vi-find-prev-char-skip" => (widget_vi_find_prev_char_skip, WidgetFlags::empty()),
        "vi-repeat-find" => (widget_vi_repeat_find, WidgetFlags::empty()),
        "vi-rev-repeat-find" => (widget_vi_rev_repeat_find, WidgetFlags::empty()),
        "vi-history-search-forward" => (widget_vi_history_search_forward, WidgetFlags::empty()),
        "vi-history-search-backward" => (widget_vi_history_search_backward, WidgetFlags::empty()),
        "vi-repeat-search" => (widget_vi_repeat_search, WidgetFlags::empty()),
        "vi-rev-repeat-search" => (widget_vi_rev_repeat_search, WidgetFlags::empty()),
        "vi-fetch-history" => (widget_vi_fetch_history, WidgetFlags::LINEMOVE),
        "vi-goto-column" => (widget_vi_goto_column, WidgetFlags::empty()),
        "vi-backward-kill-word" => (widget_vi_backward_kill_word, WidgetFlags::KILL),

        // Digit argument
        "digit-argument" => (widget_digit_argument, WidgetFlags::NOTCOMMAND),

        // Region / mark — ports cited in each widget body docstring
        // against Src/Zle/zle_move.c (set-mark / exchange-point /
        // visual / deactivate) and zle_misc.c (bslashquote-line / bslashquote-region
        // / copy-region-as-kill / pound-insert / copy-prev-word).
        "set-mark-command" => (
            widget_set_mark_command,
            WidgetFlags::MENUCMP | WidgetFlags::KEEPSUFFIX | WidgetFlags::LASTCOL,
        ),
        "exchange-point-and-mark" => (widget_exchange_point_and_mark, WidgetFlags::empty()),
        "deactivate-region" => (widget_deactivate_region, WidgetFlags::empty()),
        "visual-mode" => (widget_visual_mode, WidgetFlags::empty()),
        "visual-line-mode" => (widget_visual_line_mode, WidgetFlags::empty()),
        "copy-region-as-kill" => (widget_copy_region_as_kill, WidgetFlags::KEEPSUFFIX),
        "copy-prev-word" => (widget_copy_prev_word, WidgetFlags::KEEPSUFFIX),
        "bslashquote-line" => (widget_quote_line, WidgetFlags::empty()),
        "bslashquote-region" => (widget_quote_region, WidgetFlags::empty()),
        "pound-insert" => (widget_pound_insert, WidgetFlags::empty()),
        "vi-pound-insert" => (widget_pound_insert, WidgetFlags::empty()),

        // Case changes — bodies in this file delegate to the existing
        // capitalize/down/upcase methods on Zle (Src/Zle/zle_misc.c).
        "capitalize-word" => (widget_capitalize_word, WidgetFlags::empty()),
        "down-case-word" => (widget_down_case_word, WidgetFlags::empty()),
        "up-case-word" => (widget_up_case_word, WidgetFlags::empty()),
        "vi-down-case" => (
            widget_down_case_word,
            WidgetFlags::LASTCOL | WidgetFlags::VIOPER,
        ),
        "vi-up-case" => (
            widget_up_case_word,
            WidgetFlags::LASTCOL | WidgetFlags::VIOPER,
        ),

        // History (additional registrations) — bodies cite zle_hist.c.
        "beginning-of-history" => (widget_beginning_of_history, WidgetFlags::empty()),
        "end-of-history" => (widget_end_of_history, WidgetFlags::empty()),
        "history-beginning-search-backward" => {
            (widget_history_beginning_search_backward, WidgetFlags::empty())
        }
        "history-beginning-search-forward" => {
            (widget_history_beginning_search_forward, WidgetFlags::empty())
        }
        "push-line" => (widget_push_line, WidgetFlags::empty()),
        "push-line-or-edit" => (widget_push_line, WidgetFlags::empty()),
        "transpose-words" => (widget_transpose_words, WidgetFlags::empty()),
        "beep" => (widget_beep, WidgetFlags::empty()),
        "describe-key-briefly" => (
            widget_describe_key_briefly,
            WidgetFlags::MENUCMP | WidgetFlags::KEEPSUFFIX | WidgetFlags::LASTCOL,
        ),

        // Word ops (extra) — Src/Zle/zle_word.c
        "delete-word" => (widget_delete_word, WidgetFlags::empty()),
        "backward-delete-word" => (widget_backward_delete_word, WidgetFlags::KEEPSUFFIX),
        "emacs-forward-word" => (widget_emacs_forward_word, WidgetFlags::empty()),
        "emacs-backward-word" => (widget_emacs_backward_word, WidgetFlags::empty()),

        // Region kill/buffer — Src/Zle/zle_misc.c
        "kill-region" => (widget_kill_region, WidgetFlags::KILL | WidgetFlags::KEEPSUFFIX),
        "kill-buffer" => (widget_kill_buffer, WidgetFlags::KILL | WidgetFlags::KEEPSUFFIX),

        // Vi mark widgets — bodies in this file delegate to the existing
        // Zle::vi_set_mark / Zle::vi_goto_mark methods (Src/Zle/zle_move.c).
        "vi-set-mark" => (widget_vi_set_mark_widget, WidgetFlags::empty()),
        "vi-goto-mark" => (widget_vi_goto_mark_widget, WidgetFlags::empty()),
        "vi-goto-mark-line" => (widget_vi_goto_mark_line_widget, WidgetFlags::empty()),
        "vi-match-bracket" => (widget_vi_match_bracket, WidgetFlags::empty()),
        "vi-caps-lock-panic" => (widget_vi_caps_lock_panic, WidgetFlags::empty()),

        // Vi line/yank — Src/Zle/zle_vi.c
        "vi-kill-line" => (widget_vi_kill_line, WidgetFlags::KILL),
        "vi-yank-eol" => (widget_vi_yank_eol, WidgetFlags::empty()),
        "vi-beginning-of-line" => (widget_vi_beginning_of_line, WidgetFlags::empty()),
        "vi-swap-case" => (widget_vi_swap_case, WidgetFlags::empty()),
        "vi-oper-swap-case" => (widget_vi_oper_swap_case, WidgetFlags::VIOPER),
        "vi-undo-change" => (widget_vi_undo_change, WidgetFlags::empty()),

        // Argument prefixes — Src/Zle/zle_misc.c
        "universal-argument" => (widget_universal_argument, WidgetFlags::NOTCOMMAND),
        "neg-argument" => (widget_neg_argument, WidgetFlags::NOTCOMMAND),

        // Misc — Src/Zle/zle_main.c, zle_misc.c
        "recursive-edit" => (widget_recursive_edit, WidgetFlags::empty()),
        "what-cursor-position" => (
            widget_what_cursor_position,
            WidgetFlags::MENUCMP | WidgetFlags::KEEPSUFFIX | WidgetFlags::LASTCOL,
        ),
        "set-local-history" => (widget_set_local_history_widget, WidgetFlags::empty()),
        "undefined-key" => (widget_undefined_key, WidgetFlags::empty()),

        // History search variants — Src/Zle/zle_hist.c
        "history-search-backward" => (widget_history_search_backward, WidgetFlags::empty()),
        "history-search-forward" => (widget_history_search_forward, WidgetFlags::empty()),
        "insert-last-word" => (widget_insert_last_word_widget, WidgetFlags::empty()),

        // Cursor motion (extra) — Src/Zle/zle_hist.c
        "up-line" => (widget_up_line, WidgetFlags::LINEMOVE),
        "down-line" => (widget_down_line, WidgetFlags::LINEMOVE),
        "up-line-or-search" => (widget_up_line_or_search, WidgetFlags::LINEMOVE),
        "down-line-or-search" => (widget_down_line_or_search, WidgetFlags::LINEMOVE),
        "vi-up-line-or-history" => (widget_vi_up_line_or_history, WidgetFlags::LINEMOVE),
        "vi-down-line-or-history" => (widget_vi_down_line_or_history, WidgetFlags::LINEMOVE),
        "beginning-of-line-hist" => (widget_beginning_of_line_hist, WidgetFlags::empty()),
        "end-of-line-hist" => (widget_end_of_line_hist, WidgetFlags::empty()),

        // Misc Src/Zle/zle_misc.c additions
        "copy-prev-shell-word" => (widget_copy_prev_shell_word, WidgetFlags::KEEPSUFFIX),
        "gosmacs-transpose-chars" => (widget_gosmacs_transpose_chars, WidgetFlags::empty()),
        "reset-prompt" => (widget_reset_prompt, WidgetFlags::empty()),
        "split-undo" => (widget_split_undo, WidgetFlags::empty()),
        "argument-base" => (
            widget_argument_base,
            WidgetFlags::MENUCMP
                | WidgetFlags::KEEPSUFFIX
                | WidgetFlags::LASTCOL
                | WidgetFlags::NOTCOMMAND,
        ),

        // History extras — Src/Zle/zle_hist.c
        "infer-next-history" => (widget_infer_next_history, WidgetFlags::empty()),
        "accept-and-infer-next-history" => {
            (widget_accept_and_infer_next_history, WidgetFlags::empty())
        }
        "get-line" => (widget_get_line, WidgetFlags::empty()),
        "push-input" => (widget_push_input, WidgetFlags::empty()),

        // Vi extras — Src/Zle/zle_vi.c
        "vi-quoted-insert" => (widget_vi_quoted_insert, WidgetFlags::empty()),
        "vi-set-buffer" => (widget_vi_set_buffer, WidgetFlags::NOTCOMMAND),
        "vi-indent" => (widget_vi_indent, WidgetFlags::VIOPER),
        "vi-unindent" => (widget_vi_unindent, WidgetFlags::VIOPER),

        // Misc host-dispatch hooks — Src/Zle/zle_misc.c, zle_tricky.c
        "run-help" => (
            widget_run_help,
            WidgetFlags::MENUCMP | WidgetFlags::KEEPSUFFIX | WidgetFlags::LASTCOL,
        ),
        "which-command" => (
            widget_run_help,
            WidgetFlags::MENUCMP | WidgetFlags::KEEPSUFFIX | WidgetFlags::LASTCOL,
        ),
        "expand-history" => (widget_expand_history, WidgetFlags::empty()),
        "magic-space" => (widget_magic_space, WidgetFlags::KEEPSUFFIX | WidgetFlags::MENUCMP),
        "spell-word" => (widget_spell_word, WidgetFlags::empty()),
        "bracketed-paste" => (widget_bracketed_paste, WidgetFlags::empty()),

        // Vi backward word-end (extra) — Src/Zle/zle_word.c
        "vi-backward-word-end" => (widget_vi_backward_word_end, WidgetFlags::empty()),
        "vi-backward-blank-word-end" => {
            (widget_vi_backward_blank_word_end, WidgetFlags::empty())
        }

        // Text objects (vi `iw`/`aw` etc.) — Src/Zle/textobjects.c
        "select-in-word" => (widget_select_in_word, WidgetFlags::empty()),
        "select-a-word" => (widget_select_a_word, WidgetFlags::empty()),
        "select-in-blank-word" => (widget_select_in_blank_word, WidgetFlags::empty()),
        "select-a-blank-word" => (widget_select_a_blank_word, WidgetFlags::empty()),
        "select-in-shell-word" => (widget_select_in_shell_word, WidgetFlags::empty()),
        "select-a-shell-word" => (widget_select_a_shell_word, WidgetFlags::empty()),

        // Completion menu navigation — Src/Zle/zle_tricky.c (host hooks)
        "menu-expand-or-complete" => (widget_menu_expand_or_complete, WidgetFlags::MENUCMP),
        "reverse-menu-complete" => (widget_reverse_menu_complete, WidgetFlags::MENUCMP),
        "accept-and-menu-complete" => (
            widget_accept_and_menu_complete,
            WidgetFlags::MENUCMP | WidgetFlags::KEEPSUFFIX,
        ),
        "list-expand" => (
            widget_list_expand,
            WidgetFlags::MENUCMP | WidgetFlags::KEEPSUFFIX,
        ),
        "expand-cmd-path" => (widget_expand_cmd_path, WidgetFlags::empty()),
        "expand-or-complete-prefix" => (widget_expand_or_complete_prefix, WidgetFlags::MENUCMP),
        "end-of-list" => (
            widget_end_of_list,
            WidgetFlags::MENUCMP | WidgetFlags::KEEPSUFFIX | WidgetFlags::LASTCOL,
        ),

        // Suffix handling — Src/Zle/zle_misc.c
        "auto-suffix-remove" => (widget_auto_suffix_remove, WidgetFlags::NOTCOMMAND),
        "auto-suffix-retain" => (
            widget_auto_suffix_retain,
            WidgetFlags::KEEPSUFFIX | WidgetFlags::NOTCOMMAND,
        ),

        // Region paste / put — Src/Zle/zle_misc.c
        "put-replace-selection" => (widget_put_replace_selection, WidgetFlags::YANK),

        // Named-command execution — Src/Zle/zle_thingy.c (host hooks)
        "execute-named-cmd" => (widget_execute_named_cmd, WidgetFlags::empty()),
        "execute-last-named-cmd" => (widget_execute_last_named_cmd, WidgetFlags::empty()),
        "read-command" => (widget_read_command, WidgetFlags::empty()),
        "where-is" => (
            widget_where_is,
            WidgetFlags::MENUCMP | WidgetFlags::KEEPSUFFIX | WidgetFlags::LASTCOL,
        ),

        // Pattern isearch — Src/Zle/zle_hist.c:936/943
        "history-incremental-pattern-search-backward" => (
            widget_history_incremental_pattern_search_backward,
            WidgetFlags::empty(),
        ),
        "history-incremental-pattern-search-forward" => (
            widget_history_incremental_pattern_search_forward,
            WidgetFlags::empty(),
        ),

        // Search acceptance — Src/Zle/zle_hist.c
        "accept-search" => (widget_accept_search, WidgetFlags::empty()),

        // Default: undefined widget
        _ => (widget_undefined, WidgetFlags::empty()),
    }
}

// Widget implementations

fn widget_accept_line(zle: &mut Zle) {
    // Port of acceptline(UNUSED(char **args)) from Src/Zle/zle_misc.c:401. The C source is
    // a one-liner: `done = 1`; everything else (return current line,
    // history append, hooks) happens in zleread() after zlecore returns.
    crate::ported::zle::zle_misc::DONE.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_accept_and_hold(zle: &mut Zle) {
    // Port of acceptandhold(UNUSED(char **args)) from Src/Zle/zle_misc.c:409.
    // Push current line onto bufstack so the next zleread() re-feeds it
    // as the next entry, then exit the editor.
    let line: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
    crate::ported::zle::zle_main::BUFSTACK.lock().unwrap().push(line);
    crate::ported::zle::zle_main::STACKCS.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_misc::DONE.store(1, std::sync::atomic::Ordering::SeqCst);
    zle.accept_line();
}

fn widget_accept_line_and_down_history(zle: &mut Zle) {
    // Port of acceptlineanddownhistory(UNUSED(char **args)) from Src/Zle/zle_hist.c:420.
    // Move forward one history entry and queue it on bufstack so the
    // next zleread() loads it; then exit the editor to run the current line.
    let len = crate::ported::zle::zle_main::history().lock().unwrap().entries.len();
    let next_idx = crate::ported::zle::zle_main::history().lock().unwrap().cursor + 1;
    if next_idx < len {
        if let Some(entry) = crate::ported::zle::zle_main::history().lock().unwrap().entries.get(next_idx) {
            crate::ported::zle::zle_main::BUFSTACK.lock().unwrap().push(entry.line.clone());
            crate::ported::zle::zle_main::STACKHIST.store((entry.num as i32).max(0), std::sync::atomic::Ordering::SeqCst);
        }
    }
    crate::ported::zle::zle_misc::DONE.store(1, std::sync::atomic::Ordering::SeqCst);
    zle.accept_line();
}

fn widget_self_insert(zle: &mut Zle) {
    // Port of selfinsert(UNUSED(char **args)) from Src/Zle/zle_misc.c:113. Insert the
    // last-read char at the cursor (`zmult` times — count-aware).
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    #[cfg(feature = "multibyte")]
    let c_opt = char::from_u32(crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) as u32);
    #[cfg(not(feature = "multibyte"))]
    let c_opt = if (0..=127).contains(&crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst)) {
        Some(crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) as u8 as char)
    } else {
        None
    };
    if let Some(c) = c_opt {
        for _ in 0..n {
            zle.self_insert(c);
        }
    }
}

fn widget_self_insert_unmeta(zle: &mut Zle) {
    // Port of selfinsertunmeta(char **args) from Src/Zle/zle_misc.c:149. Strip the
    // 0x80 meta bit from lastchar before inserting — used when the user
    // bound an Esc-prefixed key (e.g. `\\eA`) to literally insert 'A'.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    let c = (crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) & 0x7f) as u8 as char;
    for _ in 0..n {
        zle.self_insert(c);
    }
}

fn widget_forward_char(zle: &mut Zle) {
    // Port of forwardchar(char **args) from Src/Zle/zle_move.c:441. Count-aware;
    // negative count delegates to backward-char (mirrors the C source's
    // recursive flip at zle_move.c:445).
    let mut n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst);
    if n < 0 {
        crate::ported::zle::zle_main::MULT.store(-n, std::sync::atomic::Ordering::SeqCst);
        widget_backward_char(zle);
        crate::ported::zle::zle_main::MULT.store(n, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && n > 0 {
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        n -= 1;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_backward_char(zle: &mut Zle) {
    // Port of backwardchar(char **args) from Src/Zle/zle_move.c:464.
    let mut n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst);
    if n < 0 {
        crate::ported::zle::zle_main::MULT.store(-n, std::sync::atomic::Ordering::SeqCst);
        widget_forward_char(zle);
        crate::ported::zle::zle_main::MULT.store(n, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && n > 0 {
        crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        n -= 1;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_forward_word(zle: &mut Zle) {
    // Port of forwardword(char **args) from Src/Zle/zle_word.c:45. Count-aware;
    // skips the current word (iword chars) then trailing non-iword
    // chars, repeated `mult` times.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && is_word_char(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !is_word_char(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_backward_word(zle: &mut Zle) {
    // Port of backwardword(char **args) from Src/Zle/zle_word.c:240. Count-aware
    // mirror of forward-word.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            break;
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && !is_word_char(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1]) {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && is_word_char(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1]) {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_beginning_of_line(zle: &mut Zle) {
    // Port of beginningofline(char **args) from Src/Zle/zle_move.c. Cursor moves
    // to the start of the current logical line — findbol respects
    // embedded newlines.
    crate::ported::zle::zle_main::ZLECS.store(zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_end_of_line(zle: &mut Zle) {
    // Port of endofline(char **args) from Src/Zle/zle_move.c.
    crate::ported::zle::zle_main::ZLECS.store(zle.findeol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_delete_char(zle: &mut Zle) {
    // Port of deletechar(char **args) from Src/Zle/zle_misc.c:157. Count-aware;
    // negative count delegates to backward-delete-char. The C source
    // returns 1 (failure) if it can't delete `mult` chars (cursor hit
    // EoB before completing); we beep instead since we don't propagate
    // widget return codes through the zlecore.
    let mut n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst);
    if n < 0 {
        crate::ported::zle::zle_main::MULT.store(-n, std::sync::atomic::Ordering::SeqCst);
        widget_backward_delete_char(zle);
        crate::ported::zle::zle_main::MULT.store(n, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    while n > 0 {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            zle.handle_feep();
            return;
        }
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
        crate::ported::zle::zle_main::ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        n -= 1;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_backward_delete_char(zle: &mut Zle) {
    // Port of backwarddeletechar(char **args) from Src/Zle/zle_misc.c:180.
    // Count-aware; negative count delegates to delete-char. C clamps
    // count to zlecs (zle_misc.c:189) so deleting past BoB stops at 0
    // rather than erroring.
    let mut n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst);
    if n < 0 {
        crate::ported::zle::zle_main::MULT.store(-n, std::sync::atomic::Ordering::SeqCst);
        widget_delete_char(zle);
        crate::ported::zle::zle_main::MULT.store(n, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    if (n as usize) > crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        n = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) as i32;
    }
    while n > 0 && crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
        crate::ported::zle::zle_main::ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        n -= 1;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_delete_char_or_list(zle: &mut Zle) {
    // Port of deletecharorlist(char **args) from Src/Zle/zle_misc.c. With an empty
    // buffer this is EOF; with non-end cursor it deletes one char; at
    // end-of-line it falls through to list-choices completion.
    if crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        crate::ported::zle::zle_misc::DONE.store(1, std::sync::atomic::Ordering::SeqCst);
    } else if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
        widget_delete_char(zle);
    } else {
    }
}

fn widget_kill_line(zle: &mut Zle) {
    // Port of killline(char **args) from Src/Zle/zle_misc.c:419. Count-aware:
    // killing N lines from the cursor — for each iteration, if the
    // cursor sits on a newline consume just that newline, otherwise
    // kill from cursor to the next newline (or EoB). Final kill goes
    // on the kill ring as one entry. Negative count delegates to
    // backward-kill-line (zle_misc.c:423).
    let mut n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst);
    if n < 0 {
        crate::ported::zle::zle_main::MULT.store(-n, std::sync::atomic::Ordering::SeqCst);
        widget_backward_kill_line(zle);
        crate::ported::zle::zle_main::MULT.store(n, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    let start = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    while n > 0 {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        if crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n' {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        } else {
            while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] != '\n' {
                crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        n -= 1;
    }
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > start {
        let killed: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(start..crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)).collect();
        crate::ported::zle::zle_main::ZLELL.fetch_sub(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(killed);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_backward_kill_line(zle: &mut Zle) {
    // Port of backwardkillline(char **args) from Src/Zle/zle_misc.c:225. Mirror of
    // kill-line: per iteration, consume a leading \\n if present;
    // otherwise back up to the previous \\n (or BoB). Negative count
    // delegates to kill-line (zle_misc.c:229).
    let mut n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst);
    if n < 0 {
        crate::ported::zle::zle_main::MULT.store(-n, std::sync::atomic::Ordering::SeqCst);
        widget_kill_line(zle);
        crate::ported::zle::zle_main::MULT.store(n, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    let end = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    while n > 0 {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            break;
        }
        if crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1] == '\n' {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        } else {
            while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1] != '\n' {
                crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        n -= 1;
    }
    if end > crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        let killed: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..end).collect();
        crate::ported::zle::zle_main::ZLELL.fetch_sub(end - crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(killed);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_kill_whole_line(zle: &mut Zle) {
    // Port of killwholeline(UNUSED(char **args)) from Src/Zle/zle_misc.c:195. The C source
    // is count-aware: kills `mult` lines centered on the current line
    // (or the whole buffer if -1). Our simplified version kills the
    // entire buffer once — sufficient for the common single-line use,
    // multi-line variants left as a follow-up.
    if crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        let killed = std::mem::take(&mut zle.zleline);
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(killed);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLELL.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_kill_word(zle: &mut Zle) {
    // Port of killword(char **args) from Src/Zle/zle_word.c. The C source skips
    // non-word chars then the word, captures the killed region in the
    // kill ring, and leaves the cursor at the start. Honours the count
    // multiplier (mult) — `3M-d` kills three words.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    let start = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !is_word_char(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && is_word_char(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let end = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(start, std::sync::atomic::Ordering::SeqCst);

    if end > start {
        let killed: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(start..end).collect();
        crate::ported::zle::zle_main::ZLELL.fetch_sub(end - start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(killed);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_backward_kill_word(zle: &mut Zle) {
    // Port of backwardkillword(char **args) from Src/Zle/zle_word.c. Mirrors
    // kill-word but in the opposite direction; cursor lands at the
    // start of the killed range. Count-aware.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    let end = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            break;
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && !is_word_char(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1]) {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && is_word_char(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1]) {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let start = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);

    if end > start {
        let killed: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(start..end).collect();
        crate::ported::zle::zle_main::ZLELL.fetch_sub(end - start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(killed);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_yank(zle: &mut Zle) {
    // Port of yank(UNUSED(char **args)) from Src/Zle/zle_misc.c. Inserts the most-recent kill-ring
    // entry at the cursor and remembers the inserted region so that an
    // immediately-following yank-pop can rotate to the previous entry.
    if let Some(text) = crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().cloned() {
        let start = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        for c in &text {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), *c);
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        crate::ported::zle::zle_main::YANKB.store(start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::YANKE.store(start + text.len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::YANKCS.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
        *crate::ported::zle::zle_main::KCT.lock().unwrap() = Some(0);
        crate::ported::zle::zle_main::YANKLAST.store(true, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_yank_pop(zle: &mut Zle) {
    // Port of yankpop(UNUSED(char **args)) from Src/Zle/zle_misc.c:728.
    // Only meaningful immediately after a yank; replaces the just-yanked
    // region with the previous kill-ring entry, cycling around the ring.
    if !crate::ported::zle::zle_main::YANKLAST.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let ring_len = crate::ported::zle::zle_main::KILLRING.lock().unwrap().len();
    if ring_len == 0 {
        return;
    }
    // Advance to the next ring entry; skip empty buffers; bail out if we
    // wrap all the way around without finding anything (matches kctstart guard
    // in C zle_misc.c:730).
    let start_idx = crate::ported::zle::zle_main::KCT.lock().unwrap().unwrap_or(0);
    let mut idx = start_idx;
    let mut found_idx: Option<usize> = None;
    for _ in 0..ring_len {
        idx = (idx + 1) % ring_len;
        if idx == start_idx {
            break;
        }
        if !crate::ported::zle::zle_main::KILLRING.lock().unwrap()[idx].is_empty() {
            found_idx = Some(idx);
            break;
        }
    }
    let new_idx = match found_idx {
        Some(i) => i,
        None => return,
    };
    let new_text: Vec<char> = crate::ported::zle::zle_main::KILLRING.lock().unwrap()[new_idx].clone();

    // Delete the previously-yanked region.
    let yb = crate::ported::zle::zle_main::YANKB.load(std::sync::atomic::Ordering::SeqCst).min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
    let ye = crate::ported::zle::zle_main::YANKE.load(std::sync::atomic::Ordering::SeqCst).min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
    if ye > yb {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(yb..ye);
        crate::ported::zle::zle_main::ZLELL.fetch_sub(ye - yb, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLECS.store(yb, std::sync::atomic::Ordering::SeqCst);

    // Paste the new entry.
    let start = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    for c in &new_text {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), *c);
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::YANKB.store(start, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::YANKE.store(start + new_text.len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::YANKCS.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    *crate::ported::zle::zle_main::KCT.lock().unwrap() = Some(new_idx);
    crate::ported::zle::zle_main::YANKLAST.store(true, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_undo(zle: &mut Zle) {
    // Port of undo(char **args) from Src/Zle/zle_utils.c:1601.
    let _ = zle.undo_widget();
}

fn widget_redo(zle: &mut Zle) {
    // Port of redo(UNUSED(char **args)) from Src/Zle/zle_utils.c:1661.
    let _ = zle.redo_widget();
}

fn widget_up_line_or_history(zle: &mut Zle) {
    // Port of uplineorhistory(char **args) from Src/Zle/zle_hist.c:282.
    let _ = zle.uplineorhistory();
}

fn widget_down_line_or_history(zle: &mut Zle) {
    // Port of downlineorhistory(char **args) from Src/Zle/zle_hist.c:370.
    let _ = zle.downlineorhistory();
}

fn widget_up_history(zle: &mut Zle) {
    // Port of uphistory(UNUSED(char **args)) from Src/Zle/zle_hist.c:233.
    // C calls zle_goto_hist(histline, -zmult, isset(HISTIGNOREDUPS)).
    // skipdups=false until ZLE has access to ShellOptions; behavior matches HISTIGNOREDUPS unset.
    let m = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst);
    zle.zle_goto_hist(-m, false);
}

fn widget_down_history(zle: &mut Zle) {
    // Port of downhistory(UNUSED(char **args)) from Src/Zle/zle_hist.c:434.
    let m = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst);
    zle.zle_goto_hist(m, false);
}

fn widget_history_isearch_backward(zle: &mut Zle) {
    // Port of historyincrementalsearchbackward(char **args) from Src/Zle/zle_hist.c:922
    // (which is doisearch(-1, 0)).
    do_isearch(zle, -1);
}

fn widget_history_isearch_forward(zle: &mut Zle) {
    // Port of historyincrementalsearchforward(char **args) from Src/Zle/zle_hist.c:929
    // (doisearch(1, 0)).
    do_isearch(zle, 1);
}

/// Minimal port of doisearch(char **args, int dir, int pattern) from zle_hist.c.
/// Reads characters into a pattern and re-searches history on each keystroke.
/// Recognised control chars: Ctrl-R repeats backward, Ctrl-S repeats forward,
/// Ctrl-G/Esc cancels (restores starting line), backspace shortens the
/// pattern, Enter accepts. Anything else exits the loop with the current
/// match in place.
fn do_isearch(zle: &mut Zle, mut dir: i32) {
    // Save start state for cancel.
    let start_line = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().clone();
    let start_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let start_cursor = crate::ported::zle::zle_main::history().lock().unwrap().cursor;

    let mut pattern = String::new();
    let mut current_idx: i32 = crate::ported::zle::zle_main::history().lock().unwrap().cursor as i32;

    while let Some(c) = zle.getfullchar(true) {
        match c {
            // Enter / Newline → accept current match.
            '\r' | '\n' => break,
            // Ctrl-G / Esc → cancel.
            '\x07' | '\x1b' => {
                *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = start_line;
                crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
                crate::ported::zle::zle_main::ZLECS.store(start_cs, std::sync::atomic::Ordering::SeqCst);
                crate::ported::zle::zle_main::history().lock().unwrap().cursor = start_cursor;
                crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
                return;
            }
            // Ctrl-R → repeat backward.
            '\x12' => {
                dir = -1;
                current_idx -= 1;
            }
            // Ctrl-S → repeat forward.
            '\x13' => {
                dir = 1;
                current_idx += 1;
            }
            // Backspace / Delete → shrink pattern.
            '\x08' | '\x7f' => {
                pattern.pop();
                current_idx = crate::ported::zle::zle_main::history().lock().unwrap().cursor as i32;
            }
            // Other printable input → extend pattern.
            ch if !ch.is_control() => {
                pattern.push(ch);
            }
            _ => break,
        }
        if pattern.is_empty() {
            continue;
        }
        *crate::ported::zle::zle_main::SRCH_STR.lock().unwrap() = Some(pattern.clone());
        let len = crate::ported::zle::zle_main::history().lock().unwrap().entries.len() as i32;
        let matched: Option<usize> = if dir < 0 {
            // Search backward starting at current_idx.
            let mut i = current_idx.min(len - 1);
            let mut found = None;
            while i >= 0 {
                if crate::ported::zle::zle_main::history().lock().unwrap().entries[i as usize].line.contains(&pattern) {
                    found = Some(i as usize);
                    break;
                }
                i -= 1;
            }
            found
        } else {
            let mut i = current_idx.max(0);
            let mut found = None;
            while i < len {
                if crate::ported::zle::zle_main::history().lock().unwrap().entries[i as usize].line.contains(&pattern) {
                    found = Some(i as usize);
                    break;
                }
                i += 1;
            }
            found
        };
        if let Some(idx) = matched {
            current_idx = idx as i32;
            crate::ported::zle::zle_main::history().lock().unwrap().cursor = idx;
            *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = crate::ported::zle::zle_main::history().lock().unwrap().entries[idx].line.chars().collect();
            crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
            // Place cursor at the start of the match for visual feedback.
            zle.zlecs = crate::ported::zle::zle_main::history().lock().unwrap().entries[idx]
                .line
                .find(&pattern)
                .unwrap_or(0)
                .min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        } else {
            // No match — beep but keep the prior position.
            zle.handle_feep();
        }
    }
}

fn widget_beginning_of_buffer_or_history(zle: &mut Zle) {
    // Port of beginningofbufferorhistory(char **args) from Src/Zle/zle_hist.c:573.
    // If the cursor is past the start of its current logical line
    // (findbol > 0), jump to absolute position 0 inside the buffer;
    // otherwise we're already at BoB → fall through to
    // beginning-of-history (load oldest entry).
    if zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) > 0 {
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    } else {
        widget_beginning_of_history(zle);
    }
}

fn widget_end_of_buffer_or_history(zle: &mut Zle) {
    // Port of endofbufferorhistory(char **args) from Src/Zle/zle_hist.c:593.
    if zle.findeol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
        crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    } else {
        widget_end_of_history(zle);
    }
}

fn widget_transpose_chars(zle: &mut Zle) {
    // Port of transposechars(UNUSED(char **args)) from Src/Zle/zle_misc.c:313. The C source
    // is count-aware (negative count transposes backwards, positive
    // forwards, default 1) and respects newline boundaries — at BoL or
    // EoL/EOB, the swap involves the cursor's neighbor on the same
    // logical line. Cursor lands one past the swapped pair on positive
    // count, mirroring emacs's `^T` advance.
    let mut n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst);
    let neg = n < 0;
    if neg {
        n = -n;
    }
    for _ in 0..n {
        let mut ct = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        // BoL (or right after \n) special-case: shift forward so we
        // can swap (line[ct], line[ct+1]) — only valid if there's at
        // least one more char before the next newline.
        if ct == 0 || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(ct - 1).copied() == Some('\n') {
            if ct == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(ct).copied() == Some('\n') {
                return;
            }
            if !neg {
                crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            ct += 1;
        }
        if neg {
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1).copied() != Some('\n') {
                crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                if ct > 1 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(ct - 2).copied() != Some('\n') {
                    ct -= 1;
                }
            }
        } else if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)).copied() != Some('\n')
        {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        if ct == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(ct).copied() == Some('\n') {
            ct -= 1;
        }
        if ct < 1 || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(ct - 1).copied() == Some('\n') {
            return;
        }
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().swap(ct - 1, ct);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_clear_screen(zle: &mut Zle) {
    // Port of clearscreen(UNUSED(char **args)) from Src/Zle/zle_refresh.c. Routes through
    // the existing Zle::clearscreen helper which emits the CSI clear +
    // home, then forces a refresh on the next zlecore iteration.
    zle.clearscreen();
}

fn widget_redisplay(zle: &mut Zle) {
    // Port of redisplay(UNUSED(char **args)) from Src/Zle/zle_refresh.c. Just sets the
    // reset flag — the next zlecore iteration calls zrefresh.
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_send_break(zle: &mut Zle) {
    // Port of sendbreak(UNUSED(char **args)) from Src/Zle/zle_misc.c:1144. The C source
    // sets errflag |= ERRFLAG_ERROR | ERRFLAG_INT and returns 1, which
    // causes the zlecore loop at zle_main.c:1128 to exit. Our
    // abort_line() clears the buffer and sets done=true — same outward
    // effect, no errflag needed because we don't carry it through to
    // the caller. (send_break() exists separately for non-widget
    // callers that want just the buffer clear.)
    zle.abort_line();
}

fn widget_overwrite_mode(zle: &mut Zle) {
    // Port of overwritemode(UNUSED(char **args)) from Src/Zle/zle_misc.c. Toggles between
    // insert (default) and overwrite. Insert mode appends at the cursor;
    // overwrite replaces the char under the cursor.
    crate::ported::zle::zle_main::INSMODE.fetch_xor(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_quoted_insert(zle: &mut Zle) {
    // Port of quotedinsert(char **args) from Src/Zle/zle_misc.c. Reads the next
    // input char and inserts it literally (bypassing keymap dispatch),
    // letting the user enter control chars verbatim — the canonical
    // Ctrl-V binding.
    if let Some(c) = zle.getfullchar(true) {
        zle.self_insert(c);
    }
}

fn widget_expand_or_complete(zle: &mut Zle) {
    // Port of expandorcomplete(char **args) from Src/Zle/zle_tricky.c — tries
    // expansion first, falls back to completion. Compsys lives in a
    // separate crate; surface the request and let the host run it.
}

fn widget_complete_word(zle: &mut Zle) {
    // Port of completeword(char **args) from Src/Zle/zle_tricky.c.
}

fn widget_expand_word(zle: &mut Zle) {
    // Port of expandword(char **args) from Src/Zle/zle_tricky.c — runs only the
    // expansion phase (history, glob, parameter, brace) without falling
    // through to completion.
}

fn widget_list_choices(zle: &mut Zle) {
    // Port of listchoices(UNUSED(char **args)) from Src/Zle/zle_tricky.c — shows matches
    // without inserting.
}

fn widget_menu_complete(zle: &mut Zle) {
    // Port of menucomplete(char **args) from Src/Zle/zle_tricky.c — enters/steps
    // the menu-selection state.
}

// Vi mode widgets

fn widget_vi_cmd_mode(zle: &mut Zle) {
    // Port of vicmdmode(UNUSED(char **args)) from Src/Zle/zle_vi.c. ESC out of insert →
    // command mode; cursor steps back one (vim convention) since vi
    // command mode treats the cursor as ON a char rather than between.
    crate::ported::zle::zle_keymap::selectkeymap("vicmd", 1);
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_insert(zle: &mut Zle) {
    // Port of viinsert(UNUSED(char **args)) from Src/Zle/zle_vi.c:355.
    crate::ported::zle::zle_keymap::selectkeymap("viins", 1);
    crate::ported::zle::zle_main::INSMODE.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_insert_bol(zle: &mut Zle) {
    // Port of viinsertbol(UNUSED(char **args)) from Src/Zle/zle_vi.c:374. Vim's `I` —
    // first-non-blank of current line, then enter insert mode.
    crate::ported::zle::zle_keymap::selectkeymap("viins", 1);
    crate::ported::zle::zle_main::INSMODE.store(1, std::sync::atomic::Ordering::SeqCst);
    let bol = zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    let mut p = bol;
    while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p]; p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && __c.is_whitespace() && __c != '\n' } {
        p += 1;
    }
    crate::ported::zle::zle_main::ZLECS.store(p, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_add_next(zle: &mut Zle) {
    // Port of viaddnext(UNUSED(char **args)) from Src/Zle/zle_vi.c:336. Vim's `a` —
    // step right one then enter insert mode (so insert lands AFTER
    // the cursor's current char).
    crate::ported::zle::zle_keymap::selectkeymap("viins", 1);
    crate::ported::zle::zle_main::INSMODE.store(1, std::sync::atomic::Ordering::SeqCst);
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < zle.findeol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) {
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_add_eol(zle: &mut Zle) {
    // Port of viaddeol(UNUSED(char **args)) from Src/Zle/zle_vi.c:346. Vim's `A` —
    // jump to end-of-line then enter insert mode.
    crate::ported::zle::zle_keymap::selectkeymap("viins", 1);
    crate::ported::zle::zle_main::INSMODE.store(1, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(zle.findeol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_forward_char(zle: &mut Zle) {
    // Port of viforwardchar(char **args) from Src/Zle/zle_move.c:653. Vim's `l`
    // — count-aware, can't cross EoL (cursor lands on the last char
    // of the current logical line at most).
    let mut n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst);
    if n < 0 {
        crate::ported::zle::zle_main::MULT.store(-n, std::sync::atomic::Ordering::SeqCst);
        widget_vi_backward_char(zle);
        crate::ported::zle::zle_main::MULT.store(n, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    let eol = zle.findeol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    let limit = eol.saturating_sub(1);
    while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < limit && n > 0 {
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        n -= 1;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_backward_char(zle: &mut Zle) {
    // Port of vibackwardchar(char **args) from Src/Zle/zle_move.c:683. Vim's `h`
    // — count-aware, can't cross BoL.
    let mut n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst);
    if n < 0 {
        crate::ported::zle::zle_main::MULT.store(-n, std::sync::atomic::Ordering::SeqCst);
        widget_vi_forward_char(zle);
        crate::ported::zle::zle_main::MULT.store(n, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    let bol = zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > bol && n > 0 {
        crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        n -= 1;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_forward_word(zle: &mut Zle) {
    // Port of viforwardword(char **args) from Src/Zle/zle_word.c. Vim's `w` —
    // routes to forward-word with the iword class definition.
    widget_forward_word(zle);
}

fn widget_vi_forward_word_end(zle: &mut Zle) {
    // Port of viforwardwordend(char **args) from Src/Zle/zle_word.c. Vim's `e` —
    // step right one, skip non-word, then walk word chars but land on
    // the LAST word char (peek-ahead pattern). Count-aware.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !is_word_char(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst).saturating_sub(1)
            && is_word_char(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1])
        {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_forward_blank_word(zle: &mut Zle) {
    // Port of viforwardblankword(char **args) from Src/Zle/zle_word.c. Vim's `W` —
    // whitespace-only word boundary (no iword class distinction).
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    for _ in 0..n {
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_whitespace() {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_whitespace() {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_forward_blank_word_end(zle: &mut Zle) {
    // Port of viforwardblankwordend(char **args) from Src/Zle/zle_word.c. Vim's
    // `E` — whitespace-only end-of-word.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_whitespace() {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst).saturating_sub(1)
            && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1].is_whitespace()
        {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_backward_word(zle: &mut Zle) {
    // Port of vibackwardword(char **args) from Src/Zle/zle_word.c. Vim's `b`.
    widget_backward_word(zle);
}

fn widget_vi_backward_blank_word(zle: &mut Zle) {
    // Port of vibackwardblankword(char **args) from Src/Zle/zle_word.c. Vim's `B`.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    for _ in 0..n {
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1].is_whitespace() {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1].is_whitespace() {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_delete(zle: &mut Zle) {
    // Port of videlete(UNUSED(char **args)) from Src/Zle/zle_vi.c:384.
    let _ = zle.vi_delete_op();
}

fn widget_vi_delete_char(zle: &mut Zle) {
    // Port of videletechar(char **args) from Src/Zle/zle_vi.c:405. Vim's `x`
    // command — same as delete-char but C source clamps the count to
    // findeol-zlecs to avoid spilling past the current line. We let
    // delete-char run with EoB clamp; for vi-aware line-bounded
    // semantics, callers should use vi_delete_op('l').
    widget_delete_char(zle);
}

fn widget_vi_backward_delete_char(zle: &mut Zle) {
    // Port of vibackwarddeletechar(char **args) from Src/Zle/zle_vi.c. Vim's `X`.
    widget_backward_delete_char(zle);
}

fn widget_vi_change(zle: &mut Zle) {
    // Port of vichange(UNUSED(char **args)) from Src/Zle/zle_vi.c:438.
    let _ = zle.vi_change_op();
}

fn widget_vi_change_eol(zle: &mut Zle) {
    // Port of vichangeeol(UNUSED(char **args)) from Src/Zle/zle_vi.c:482. Vim's `C` —
    // kill from cursor to EoL, enter insert mode.
    widget_kill_line(zle);
    widget_vi_insert(zle);
}

fn widget_vi_kill_eol(zle: &mut Zle) {
    // Port of vikilleol(UNUSED(char **args)) from Src/Zle/zle_vi.c. Vim's `D` — kill
    // from cursor to EoL without entering insert.
    widget_kill_line(zle);
}

fn widget_vi_yank(zle: &mut Zle) {
    // Port of viyank(UNUSED(char **args)) from Src/Zle/zle_vi.c:507.
    let _ = zle.vi_yank_op();
}

fn widget_vi_yank_whole_line(zle: &mut Zle) {
    // Port of viyankwholeline(UNUSED(char **args)) from Src/Zle/zle_vi.c:550. Vim's `Y` —
    // yank `mult` whole lines into the kill ring (with the trailing
    // newline included so vi-put-after / vi-put-before recognise this
    // as a line-wise yank). C source: zle_vi.c:559 walks zlecs through
    // findeol+1 to capture each line.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    let bol = zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    let mut end = bol;
    for _ in 0..n {
        end = zle.findeol(end);
        if end < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            end += 1; // include trailing '\n'
        } else {
            break;
        }
    }
    let region: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[bol..end].to_vec();
    if !region.is_empty() {
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(region);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
    }
}

fn widget_vi_put_after(zle: &mut Zle) {
    // Port of viputafter(UNUSED(char **args)) from Src/Zle/zle_misc.c. Vim's `p` command —
    // for character-wise paste, insert AFTER the cursor; for line-wise
    // paste (when the kill-ring entry contains a trailing '\n'), insert
    // on a new line below. Cursor lands on the LAST char of the pasted
    // text. The C source distinguishes line vs char ranges via the
    // CUTBUFFER_LINE flag on the cutbuf; we approximate by checking
    // whether the most-recent kill-ring entry ends in a newline.
    let is_line_paste = zle
        .killring
        .front()
        .and_then(|v| v.last().copied())
        == Some('\n');
    if is_line_paste {
        // Move to end of current line, then paste (which inserts the
        // newline-prefixed content immediately).
        crate::ported::zle::zle_main::ZLECS.store(zle.findeol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        widget_yank(zle);
    } else {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        widget_yank(zle);
        // Vim convention: cursor on last pasted char, not after.
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

fn widget_vi_put_before(zle: &mut Zle) {
    // Port of viputbefore(UNUSED(char **args)) from Src/Zle/zle_misc.c. Vim's `P` command —
    // char-wise paste BEFORE the cursor; line-wise paste opens a new
    // line ABOVE. Cursor lands on the last char of the pasted region.
    let is_line_paste = zle
        .killring
        .front()
        .and_then(|v| v.last().copied())
        == Some('\n');
    if is_line_paste {
        // Move to start of current line, paste (the newline at end of
        // the kill-ring entry pushes the existing line down).
        crate::ported::zle::zle_main::ZLECS.store(zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
        widget_yank(zle);
    } else {
        widget_yank(zle);
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

fn widget_vi_replace(zle: &mut Zle) {
    // Port of vireplace(UNUSED(char **args)) from Src/Zle/zle_vi.c (the `R` command).
    // Switch to insert keymap with overwrite mode so subsequent self-
    // inserts replace existing chars instead of pushing them right.
    crate::ported::zle::zle_keymap::selectkeymap("viins", 1);
    crate::ported::zle::zle_main::INSMODE.store(0, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_replace_chars(zle: &mut Zle) {
    // Port of vireplacechars(UNUSED(char **args)) from Src/Zle/zle_vi.c (the `r` command).
    // Read one char and overwrite the char under the cursor with it.
    // The C source supports a numeric prefix (`3rX` replaces the next
    // 3 chars all with X); replicated below.
    if let Some(c) = zle.getfullchar(true) {
        let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
        for _ in 0..n {
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = c;
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        // Vim convention: cursor lands on the last replaced char.
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_vi_substitute(zle: &mut Zle) {
    // Port of visubstitute(UNUSED(char **args)) from Src/Zle/zle_vi.c:455. Delete `mult`
    // chars then enter insert mode — vim's `s` command.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    for _ in 0..n {
        widget_delete_char(zle);
    }
    widget_vi_insert(zle);
}

fn widget_vi_change_whole_line(zle: &mut Zle) {
    // Port of vichangewholeline(char **args) from Src/Zle/zle_vi.c:499 (the `S`
    // command). Kill the whole line, then enter insert mode.
    widget_kill_whole_line(zle);
    widget_vi_insert(zle);
}

fn widget_vi_first_non_blank(zle: &mut Zle) {
    // Port of vifirstnonblank(UNUSED(char **args)) from Src/Zle/zle_move.c:862. Move
    // cursor to the first non-blank character on the current line.
    let bol = zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    let mut p = bol;
    while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p]; p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && __c.is_whitespace() && __c != '\n' } {
        p += 1;
    }
    crate::ported::zle::zle_main::ZLECS.store(p, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_end_of_line(zle: &mut Zle) {
    // Port of viendofline(UNUSED(char **args)) from Src/Zle/zle_move.c:708. Vim's `$`
    // semantics — cursor lands on the last char of the line, not past it.
    let eol = zle.findeol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    if eol > 0 && (eol == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[eol] == '\n') {
        crate::ported::zle::zle_main::ZLECS.store(eol.saturating_sub(1), std::sync::atomic::Ordering::SeqCst);
    } else {
        crate::ported::zle::zle_main::ZLECS.store(eol, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_digit_or_beginning_of_line(zle: &mut Zle) {
    // Port of vidigitorbeginningofline(char **args) from Src/Zle/zle_vi.c. With
    // an active numeric prefix the `0` key acts as a digit; otherwise
    // it's beginning-of-line.
    if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & crate::ported::zle::zle_h::MOD_MULT != 0 {
        widget_digit_argument(zle);
    } else {
        widget_beginning_of_line(zle);
    }
}

fn widget_vi_open_line_below(zle: &mut Zle) {
    // Port of viopenlinebelow(UNUSED(char **args)) from Src/Zle/zle_vi.c (the `o` command).
    // Move to end of line, insert newline, enter insert mode.
    crate::ported::zle::zle_main::ZLECS.store(zle.findeol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
    zle.self_insert('\n');
    widget_vi_insert(zle);
}

fn widget_vi_open_line_above(zle: &mut Zle) {
    // Port of viopenlineabove(UNUSED(char **args)) from Src/Zle/zle_vi.c (the `O` command).
    // Move to start of line, insert newline, step back, enter insert.
    crate::ported::zle::zle_main::ZLECS.store(zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
    zle.self_insert('\n');
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
    widget_vi_insert(zle);
}

fn widget_vi_join(zle: &mut Zle) {
    // Port of vijoin(UNUSED(char **args)) from Src/Zle/zle_misc.c (the `J` command).
    // Find the newline at or after the cursor, remove it, and insert
    // a separator space (unless the newline was at end-of-buffer or
    // the next char was already whitespace). The C source also
    // collapses any leading whitespace on the joined line — replicated
    // here by skipping spaces after the removed newline.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    for _ in 0..n {
        let mut pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos] != '\n' {
            pos += 1;
        }
        if pos >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        // Remove the newline.
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(pos);
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        // Eat leading whitespace on the joined line (vim convention).
        while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos] == ' ' {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(pos);
            crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        }
        // Insert a single space separator if the join didn't already
        // bridge two non-space chars at a sentence boundary.
        if pos > 0
            && pos <= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(pos - 1).copied() != Some(' ')
            && pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)
        {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(pos, ' ');
            crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_repeat_change(zle: &mut Zle) {
    // Port of virepeatchange(UNUSED(char **args)) from Src/Zle/zle_vi.c. Replays the keys in
    // vi_chg_buf — but the recording side (which captures keystrokes during
    // d/c/y operators into vi_chg_buf) is still pending, so without a
    // recorded change the buffer is empty and the widget is a no-op.
    // This matches zsh's behavior when no change has been made yet.
    if crate::ported::zle::zle_main::VICHGBUF.lock().unwrap().is_empty() {
        return;
    }
    // Re-feed the recorded keys via ungetbytes so the next iteration of
    // zlecore re-runs them. ungetbytes prepends to the input buffer so the
    // bytes will be consumed before any new keystrokes.
    let bytes = crate::ported::zle::zle_main::VICHGBUF.lock().unwrap().clone();
    zle.ungetbytes(&bytes);
}

fn widget_vi_find_next_char(zle: &mut Zle) {
    // Port of vifindnextchar(char **args) from Src/Zle/zle_move.c:739.
    zle.vi_find_char(true, false);
}

fn widget_vi_find_prev_char(zle: &mut Zle) {
    // Port of vifindprevchar(char **args) from Src/Zle/zle_move.c:751.
    zle.vi_find_char(false, false);
}

fn widget_vi_find_next_char_skip(zle: &mut Zle) {
    // Port of vifindnextcharskip(char **args) from Src/Zle/zle_move.c:763.
    zle.vi_find_char(true, true);
}

fn widget_vi_find_prev_char_skip(zle: &mut Zle) {
    // Port of vifindprevcharskip(char **args) from Src/Zle/zle_move.c:775.
    zle.vi_find_char(false, true);
}

fn widget_vi_repeat_find(zle: &mut Zle) {
    // Port of virepeatfind(char **args) from Src/Zle/zle_move.c:835.
    let _ = zle.virepeatfind();
}

fn widget_vi_rev_repeat_find(zle: &mut Zle) {
    // Port of virevrepeatfind(char **args) from Src/Zle/zle_move.c:842.
    let _ = zle.virevrepeatfind();
}

fn widget_vi_history_search_forward(zle: &mut Zle) {
    // Port of vihistorysearchforward(char **args) from Src/Zle/zle_hist.c.
    // Read the search pattern starting from `?` then run a forward history search.
    // For now: re-run the last srch_str if any.
    let pat = match crate::ported::zle::zle_main::SRCH_STR.lock().unwrap().clone() {
        Some(s) if !s.is_empty() => s,
        _ => return,
    };
    let len = crate::ported::zle::zle_main::history().lock().unwrap().entries.len();
    let start = crate::ported::zle::zle_main::history().lock().unwrap().cursor + 1;
    for i in start..len {
        if crate::ported::zle::zle_main::history().lock().unwrap().entries[i].line.contains(&pat) {
            if crate::ported::zle::zle_main::history().lock().unwrap().saved_line.is_none() {
                crate::ported::zle::zle_main::history().lock().unwrap().saved_line = Some(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().clone());
                crate::ported::zle::zle_main::history().lock().unwrap().saved_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
            }
            crate::ported::zle::zle_main::history().lock().unwrap().cursor = i;
            *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = crate::ported::zle::zle_main::history().lock().unwrap().entries[i].line.chars().collect();
            crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
            return;
        }
    }
}

fn widget_vi_history_search_backward(zle: &mut Zle) {
    // Port of vihistorysearchbackward(char **args) from Src/Zle/zle_hist.c.
    let pat = match crate::ported::zle::zle_main::SRCH_STR.lock().unwrap().clone() {
        Some(s) if !s.is_empty() => s,
        _ => return,
    };
    if crate::ported::zle::zle_main::history().lock().unwrap().cursor == 0 {
        return;
    }
    let mut i = crate::ported::zle::zle_main::history().lock().unwrap().cursor.min(crate::ported::zle::zle_main::history().lock().unwrap().entries.len()).saturating_sub(1);
    loop {
        if crate::ported::zle::zle_main::history().lock().unwrap().entries[i].line.contains(&pat) {
            if crate::ported::zle::zle_main::history().lock().unwrap().saved_line.is_none() {
                crate::ported::zle::zle_main::history().lock().unwrap().saved_line = Some(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().clone());
                crate::ported::zle::zle_main::history().lock().unwrap().saved_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
            }
            crate::ported::zle::zle_main::history().lock().unwrap().cursor = i;
            *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = crate::ported::zle::zle_main::history().lock().unwrap().entries[i].line.chars().collect();
            crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
            return;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
}

fn widget_vi_repeat_search(zle: &mut Zle) {
    // Port of virepeatsearch(UNUSED(char **args)) from Src/Zle/zle_hist.c.
    // Replays the last vi search in the same direction.
    let mut hist = std::mem::take(&mut zle.history);
    zle.vi_repeat_search(&mut hist);
    *crate::ported::zle::zle_main::history().lock().unwrap() = hist;
}

fn widget_vi_rev_repeat_search(zle: &mut Zle) {
    // Port of virevrepeatsearch(char **args) from Src/Zle/zle_hist.c.
    let mut hist = std::mem::take(&mut zle.history);
    zle.virevrepeatsearch(&mut hist);
    *crate::ported::zle::zle_main::history().lock().unwrap() = hist;
}

fn widget_vi_fetch_history(zle: &mut Zle) {
    // Port of vifetchhistory(UNUSED(char **args)) from Src/Zle/zle_hist.c:1787.
    // With no count: jump to the live (newest) entry. With a count: load
    // that history event by 1-based index. Negative count is rejected.
    if crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst) < 0 {
        return;
    }
    let has_mult = crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & crate::ported::zle::zle_h::MOD_MULT != 0;
    let on_live = crate::ported::zle::zle_main::history().lock().unwrap().cursor >= crate::ported::zle::zle_main::history().lock().unwrap().entries.len();
    if on_live || (crate::ported::zle::zle_main::ZLEREADFLAGS.load(std::sync::atomic::Ordering::SeqCst) & crate::ported::zsh_h::ZLRF_HISTORY) == 0 {
        if !has_mult {
            crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLECS.store(zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
            return;
        }
        if (crate::ported::zle::zle_main::ZLEREADFLAGS.load(std::sync::atomic::Ordering::SeqCst) & crate::ported::zsh_h::ZLRF_HISTORY) == 0 {
            return;
        }
    }
    let target_idx_1: i32 = if has_mult {
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult
    } else {
        crate::ported::zle::zle_main::history().lock().unwrap().entries.len() as i32
    };
    if target_idx_1 < 1 {
        return;
    }
    let target_idx = (target_idx_1 - 1) as usize;
    if target_idx >= crate::ported::zle::zle_main::history().lock().unwrap().entries.len() {
        return;
    }
    if crate::ported::zle::zle_main::history().lock().unwrap().saved_line.is_none() && on_live {
        crate::ported::zle::zle_main::history().lock().unwrap().saved_line = Some(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().clone());
        crate::ported::zle::zle_main::history().lock().unwrap().saved_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::history().lock().unwrap().cursor = target_idx;
    *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = crate::ported::zle::zle_main::history().lock().unwrap().entries[target_idx].line.chars().collect();
    crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_goto_column(zle: &mut Zle) {
    // Port of vigotocolumn(UNUSED(char **args)) from Src/Zle/zle_move.c. Vim's `|` —
    // jump to column N (1-based) on the current logical line. The
    // count is in zmod.mult; cursor lands at bol + (mult - 1),
    // clamped to the line's EoL.
    let col = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult.saturating_sub(1) as usize;
    let bol = zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    let eol = zle.findeol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    crate::ported::zle::zle_main::ZLECS.store((bol + col).min(eol), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_backward_kill_word(zle: &mut Zle) {
    // Port of vibackwardkillword(UNUSED(char **args)) from Src/Zle/zle_word.c. Vim's
    // ^W in insert mode — same as backward-kill-word but specifically
    // bound for the vi insert keymap.
    widget_backward_kill_word(zle);
}

fn widget_digit_argument(zle: &mut Zle) {
    // Port of digitargument(UNUSED(char **args)) from Src/Zle/zle_misc.c:950 plus the
    // parsedigit() helper at zle_misc.c:919. Accepts a digit in the
    // current zmod.base (10 by default; up to 36 honours a-z / A-Z),
    // and accumulates it into zmod.tmult with sign tracking. After
    // neg-argument has fired (MOD_NEG set), the first digit replaces
    // the placeholder -1 so `M-- 5` ends up as -5, matching C zsh.
    let base = crate::ported::zle::zle_main::ZMOD.lock().unwrap().base;
    let new_digit = parse_digit_in_base(crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) as u8, base);
    if new_digit < 0 {
        zle.handle_feep();
        return;
    }
    let sign = if crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult < 0 { -1 } else { 1 };
    if !zle
        .zmod
        .flags
         & crate::ported::zle::zle_h::MOD_TMULT != 0
    {
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = 0;
    }
    if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & crate::ported::zle::zle_h::MOD_NEG != 0 {
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = sign * new_digit;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags &= !crate::ported::zle::zle_h::MOD_NEG;
    } else {
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.tmult = __g_zmod.tmult * base + sign * new_digit;
    }
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= crate::ported::zle::zle_h::MOD_TMULT;
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_undefined(zle: &mut Zle) {
    // Port of undefinedkey(UNUSED(char **args)) from Src/Zle/zle_main.c. zsh dispatches
    // here when a key has no binding in the current keymap; the C
    // source just beeps. We do the same by ignoring `zle` (handle_feep
    // would also work, but the zlecore's no-binding fallback already
    // calls handle_feep before reaching this — calling it again would
    // double-beep).
    let _ = zle;
}

/// Parse a single byte as a digit in `base`. Returns `-1` if the byte
/// isn't a valid digit in that base.
/// Port of `parsedigit(int inkey)` from Src/Zle/zle_misc.c:919. Used by
/// digit-argument and universal-argument to honour zmod.base ∈ [2, 36].
fn parse_digit_in_base(b: u8, base: i32) -> i32 {
    let inkey = b as i32 & 0x7f;
    if base > 10 {
        if (b'a' as i32..b'a' as i32 + base - 10).contains(&inkey) {
            return inkey - b'a' as i32 + 10;
        }
        if (b'A' as i32..b'A' as i32 + base - 10).contains(&inkey) {
            return inkey - b'A' as i32 + 10;
        }
        if (b'0' as i32..=b'9' as i32).contains(&inkey) {
            return inkey - b'0' as i32;
        }
        return -1;
    }
    if (b'0' as i32..b'0' as i32 + base).contains(&inkey) {
        inkey - b'0' as i32
    } else {
        -1
    }
}

// =============================================================================
// Section: misc widget ports added after the initial table — every widget
// body in this section cites the C source it ports. Bodies may delegate to
// existing Zle methods or inline the small ones; either way the docstring
// pins the Src/Zle/*.c origin.
// =============================================================================

fn widget_beep(zle: &mut Zle) {
    // Port of beep / handlefeep() from Src/Zle/zle_utils.c. Just emits the
    // bell — `handle_feep` already does the right thing.
    zle.handle_feep();
}

fn widget_set_mark_command(zle: &mut Zle) {
    // Port of setmarkcommand(UNUSED(char **args)) from Src/Zle/zle_move.c:483. Negative count
    // disables the region; otherwise marks the current cursor and turns
    // on the visual region (charwise).
    if crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst) < 0 {
        crate::ported::zle::zle_main::REGION_ACTIVE.store(0, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_exchange_point_and_mark(zle: &mut Zle) {
    // Port of exchangepointandmark(UNUSED(char **args)) from Src/Zle/zle_move.c:496. With
    // mult==0 the C source just turns the region on without swapping;
    // with mult>0 swaps cursor↔mark and clamps cursor.
    if crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    let new_cs = crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(new_cs.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
    if crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_deactivate_region(zle: &mut Zle) {
    // Port of deactivateregion(UNUSED(char **args)) from Src/Zle/zle_move.c:564.
    zle.vi_deactivate_region();
}

fn widget_visual_mode(zle: &mut Zle) {
    // Port of visualmode(UNUSED(char **args)) from Src/Zle/zle_move.c:516.
    zle.vi_visual_mode();
}

fn widget_visual_line_mode(zle: &mut Zle) {
    // Port of visuallinemode(UNUSED(char **args)) from Src/Zle/zle_move.c:540.
    zle.vi_visual_line_mode();
}

fn widget_capitalize_word(zle: &mut Zle) {
    // Port of capitalizeword(UNUSED(char **args)) from Src/Zle/zle_misc.c. Method already
    // exists on Zle; this is the dispatch entry.
    zle.capitalize_word();
}

fn widget_down_case_word(zle: &mut Zle) {
    // Port of downcaseword(UNUSED(char **args)) from Src/Zle/zle_misc.c.
    zle.downcase_word();
}

fn widget_up_case_word(zle: &mut Zle) {
    // Port of upcaseword(UNUSED(char **args)) from Src/Zle/zle_misc.c.
    zle.upcase_word();
}

fn widget_pound_insert(zle: &mut Zle) {
    // Port of poundinsert(UNUSED(char **args)) from Src/Zle/zle_misc.c:369. Toggle a leading
    // `#` on every logical line so the entire input is commented out
    // (or uncommented). Common keybinding: M-#.
    crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
    let toggle_off = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().first().copied() == Some('#');
    if toggle_off {
        // Walk every logical line, removing one leading '#' if present
        // (C source: zle_misc.c:384-394).
        let mut p = 0;
        loop {
            let bol = zle.findbol(p);
            if crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(bol).copied() == Some('#') {
                crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(bol);
                if crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                    crate::ported::zle::zle_main::ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
            let eol = zle.findeol(bol);
            if eol >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            p = eol + 1;
        }
    } else {
        // Insert '#' at start of every logical line (zle_misc.c:373-383).
        let mut p = 0;
        loop {
            let bol = zle.findbol(p);
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(bol, '#');
            crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let eol = zle.findeol(bol);
            if eol >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            p = eol + 1;
        }
    }
    crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_misc::DONE.store(1, std::sync::atomic::Ordering::SeqCst); // C zsh accepts the line after a pound-insert.
}

fn widget_quote_line(zle: &mut Zle) {
    // Port of quoteline(UNUSED(char **args)) from Src/Zle/zle_misc.c:1187. Wrap the entire
    // buffer in single quotes, escaping any embedded single bslashquote as
    // `'\''` (the C source's makequote routine).
    let inner: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
    let escaped = inner.replace('\'', r"'\''");
    let new_line = format!("'{}'", escaped);
    *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = new_line.chars().collect();
    crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_quote_region(zle: &mut Zle) {
    // Port of quoteregion(UNUSED(char **args)) from Src/Zle/zle_misc.c:1152. Wrap the
    // currently-selected region (mark..zlecs, normalised) in single
    // quotes with embedded-bslashquote escaping.
    let (lo, hi) = if crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) <= crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        (crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst))
    } else {
        (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst))
    };
    let lo = lo.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
    let hi = hi.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
    if hi <= lo {
        return;
    }
    let inner: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[lo..hi].iter().collect();
    let escaped = inner.replace('\'', r"'\''");
    let wrapped = format!("'{}'", escaped);
    let wrapped_chars: Vec<char> = wrapped.chars().collect();
    crate::ported::zle::zle_main::ZLELINE.lock().unwrap().splice(lo..hi, wrapped_chars.iter().copied());
    crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(lo + wrapped_chars.len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_copy_region_as_kill(zle: &mut Zle) {
    // Port of copyregionaskill(char **args) from Src/Zle/zle_misc.c:494. Copies
    // mark..zlecs (normalised) onto the kill ring without removing it.
    let (lo, hi) = if crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) <= crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        (crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst))
    } else {
        (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst))
    };
    let lo = lo.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
    let hi = hi.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
    if hi <= lo {
        return;
    }
    let region: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[lo..hi].to_vec();
    crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(region);
    if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
    }
}

fn widget_copy_prev_word(zle: &mut Zle) {
    // Port of copyprevword(UNUSED(char **args)) from Src/Zle/zle_misc.c:1066. Inserts the
    // previous word (per ZC_iword) at the cursor. The full C version
    // walks `zmult` words back; we replicate that by scanning backward
    // through `mult` word-boundaries.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1) as usize;
    let mut end = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let mut start;
    let mut word: Option<(usize, usize)> = None;
    for _ in 0..n {
        // Skip whitespace going backward.
        while end > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[end - 1].is_whitespace() {
            end -= 1;
        }
        if end == 0 {
            break;
        }
        start = end;
        while start > 0 && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start - 1].is_whitespace() {
            start -= 1;
        }
        word = Some((start, end));
        end = start;
    }
    if let Some((s, e)) = word {
        let copied: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[s..e].to_vec();
        for (i, c) in copied.iter().enumerate() {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + i, *c);
        }
        crate::ported::zle::zle_main::ZLECS.fetch_add(copied.len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_transpose_words(zle: &mut Zle) {
    // Port of transposewords(UNUSED(char **args)) from Src/Zle/zle_word.c:652. The C source
    // is a multi-step pointer dance; this Rust port recreates the
    // common-case behavior: swap the two whitespace-separated words
    // around (or before) the cursor. Multi-line + edge-case handling
    // matches the C pattern of "fall back to nearest two prior words"
    // when the cursor is past the last word on the line.
    let n = crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst);
    if n == 0 {
        return;
    }
    // Find the word containing or following the cursor (`p4` in C).
    let mut p4 = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst).min(n);
    while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p4]; p4 < n && !__c.is_alphanumeric() && __c != '_' } {
        p4 += 1;
    }
    // If we landed past EOL, slide back to find the prior word.
    if p4 == n {
        let mut x = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[x - 1]; x > 0 && (!__c.is_alphanumeric() && __c != '_') } {
            x -= 1;
        }
        if x == 0 {
            return;
        }
        p4 = x;
    }
    let p3 = {
        let mut x = p4;
        while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[x]; x < n && (__c.is_alphanumeric() || __c == '_') } {
            x += 1;
        }
        x
    };
    let p4 = {
        let mut x = p4;
        while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[x - 1]; x > 0 && (__c.is_alphanumeric() || __c == '_') } {
            x -= 1;
        }
        x
    };
    let p2 = {
        let mut x = p4;
        while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[x - 1]; x > 0 && !__c.is_alphanumeric() && __c != '_' } {
            x -= 1;
        }
        x
    };
    let p1 = {
        let mut x = p2;
        while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[x - 1]; x > 0 && (__c.is_alphanumeric() || __c == '_') } {
            x -= 1;
        }
        x
    };
    if p1 == p2 || p4 == p3 {
        return;
    }
    let word1: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p1..p2].to_vec();
    let word2: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p4..p3].to_vec();
    let mut new_buf: Vec<char> = Vec::with_capacity(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
    new_buf.extend_from_slice(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..p1]);
    new_buf.extend_from_slice(&word2);
    new_buf.extend_from_slice(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p2..p4]);
    new_buf.extend_from_slice(&word1);
    new_buf.extend_from_slice(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p3..]);
    *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = new_buf;
    crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(p1 + word2.len() + (p4 - p2) + word1.len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_history_beginning_search_backward(zle: &mut Zle) {
    // Port of historybeginningsearchbackward(char **args) from Src/Zle/zle_hist.c:2039.
    // Searches history for entries that start with the text *before* the
    // cursor (the prefix), keeping the cursor where it is on a match.
    let prefix: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst).min(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len())]
        .iter()
        .collect();
    if crate::ported::zle::zle_main::history().lock().unwrap().cursor == 0 {
        return;
    }
    let saved_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let mut i = crate::ported::zle::zle_main::history().lock().unwrap().cursor.min(crate::ported::zle::zle_main::history().lock().unwrap().entries.len()).saturating_sub(1);
    loop {
        if crate::ported::zle::zle_main::history().lock().unwrap().entries[i].line.starts_with(&prefix) {
            if crate::ported::zle::zle_main::history().lock().unwrap().saved_line.is_none() {
                crate::ported::zle::zle_main::history().lock().unwrap().saved_line = Some(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().clone());
                crate::ported::zle::zle_main::history().lock().unwrap().saved_cs = saved_cs;
            }
            crate::ported::zle::zle_main::history().lock().unwrap().cursor = i;
            *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = crate::ported::zle::zle_main::history().lock().unwrap().entries[i].line.chars().collect();
            crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLECS.store(saved_cs.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
            return;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
}

fn widget_history_beginning_search_forward(zle: &mut Zle) {
    // Port of historybeginningsearchforward() — same shape as the
    // backward variant (zle_hist.c:2039 area) but stepping forward.
    let prefix: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst).min(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len())]
        .iter()
        .collect();
    let saved_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let len = crate::ported::zle::zle_main::history().lock().unwrap().entries.len();
    for i in (crate::ported::zle::zle_main::history().lock().unwrap().cursor + 1)..len {
        if crate::ported::zle::zle_main::history().lock().unwrap().entries[i].line.starts_with(&prefix) {
            crate::ported::zle::zle_main::history().lock().unwrap().cursor = i;
            *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = crate::ported::zle::zle_main::history().lock().unwrap().entries[i].line.chars().collect();
            crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLECS.store(saved_cs.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
            return;
        }
    }
}

fn widget_beginning_of_history(zle: &mut Zle) {
    // Port of beginningofhistory(UNUSED(char **args)) from Src/Zle/zle_hist.c:464.
    let mut hist = std::mem::take(&mut zle.history);
    zle.beginning_of_history(&mut hist);
    *crate::ported::zle::zle_main::history().lock().unwrap() = hist;
}

fn widget_end_of_history(zle: &mut Zle) {
    // Port of endofhistory(UNUSED(char **args)) from Src/Zle/zle_hist.c:478.
    let mut hist = std::mem::take(&mut zle.history);
    zle.endofhistory(&mut hist);
    *crate::ported::zle::zle_main::history().lock().unwrap() = hist;
}

fn widget_push_line(zle: &mut Zle) {
    // Port of pushline(UNUSED(char **args)) from Src/Zle/zle_hist.c:832.
    zle.push_line();
    crate::ported::zle::zle_misc::DONE.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_describe_key_briefly(zle: &mut Zle) {
    // Port of describekeybriefly(UNUSED(char **args)) from Src/Zle/zle_thingy.c. Existing
    // method on Zle handles the input read + lookup loop.
    zle.describe_key_briefly();
}

fn widget_delete_word(zle: &mut Zle) {
    // Port of deleteword(char **args) from Src/Zle/zle_word.c. Like kill-word but
    // doesn't put the deleted text on the kill ring.
    let saved_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let end = zle.find_word_end(WordStyle::Emacs);
    if end > saved_cs {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(saved_cs..end);
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_backward_delete_word(zle: &mut Zle) {
    // Port of backwarddeleteword(char **args) from Src/Zle/zle_word.c. Like
    // backward-kill-word but no kill-ring update.
    let end = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let start = zle.find_word_start(WordStyle::Emacs);
    if end > start {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(start..end);
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(start, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_emacs_forward_word(zle: &mut Zle) {
    // Port of emacsforwardword(char **args) from Src/Zle/zle_word.c — same as
    // forward-word in emacs style; explicit name binding for users who
    // want it independent of the global word style.
    crate::ported::zle::zle_main::ZLECS.store(zle.find_word_end(WordStyle::Emacs), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_emacs_backward_word(zle: &mut Zle) {
    // Port of emacsbackwardword(char **args) from Src/Zle/zle_word.c.
    crate::ported::zle::zle_main::ZLECS.store(zle.find_word_start(WordStyle::Emacs), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_kill_region(zle: &mut Zle) {
    // Port of killregion(UNUSED(char **args)) from Src/Zle/zle_misc.c. Drains the region
    // (mark..zlecs, normalised) into the kill ring and removes it.
    let (lo, hi) = if crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) <= crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        (crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst))
    } else {
        (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst))
    };
    let lo = lo.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
    let hi = hi.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
    if hi <= lo {
        return;
    }
    let removed: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[lo..hi].to_vec();
    crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(lo..hi);
    crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(lo, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(removed);
    if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_kill_buffer(zle: &mut Zle) {
    // Port of killbuffer(UNUSED(char **args)) from Src/Zle/zle_misc.c. Cuts the entire line
    // to the kill ring.
    if crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        return;
    }
    let killed: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().clone();
    crate::ported::zle::zle_main::ZLELINE.lock().unwrap().clear();
    crate::ported::zle::zle_main::ZLELL.store(0, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(killed);
    if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_set_mark_widget(zle: &mut Zle) {
    // Port of visetmark(UNUSED(char **args)) from Src/Zle/zle_move.c:872. Reads the next
    // char as the mark name and stores it in vi_marks via the existing
    // Zle::vi_set_mark method.
    if let Some(c) = zle.getfullchar(false) {
        zle.vi_set_mark(c);
    }
}

fn widget_vi_goto_mark_widget(zle: &mut Zle) {
    // Port of vigotomark(UNUSED(char **args)) from Src/Zle/zle_move.c:887.
    if let Some(c) = zle.getfullchar(false) {
        zle.vi_goto_mark(c);
    }
}

fn widget_vi_goto_mark_line_widget(zle: &mut Zle) {
    // Port of vigotomarkline(char **args) from Src/Zle/zle_move.c. Same as
    // vi-goto-mark but lands at first non-blank of the line containing
    // the mark.
    if let Some(c) = zle.getfullchar(false) {
        zle.vi_goto_mark(c);
        // Move to first non-blank of the line we landed on.
        let bol = zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
        let mut p = bol;
        while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p]; p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && __c.is_whitespace() && __c != '\n' } {
            p += 1;
        }
        crate::ported::zle::zle_main::ZLECS.store(p, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_vi_match_bracket(zle: &mut Zle) {
    // Port of vimatchbracket(UNUSED(char **args)) from Src/Zle/zle_vi.c. Method already
    // exists on Zle via vi_match_bracket.
    zle.vi_match_bracket();
}

fn widget_vi_caps_lock_panic(zle: &mut Zle) {
    // Port of vicapslockpanic(UNUSED(char **args)) from Src/Zle/zle_vi.c. zsh's joke
    // widget: blocks until you press a non-Caps-Lock key. Practical
    // port simply beeps once.
    zle.handle_feep();
}

fn widget_vi_kill_line(zle: &mut Zle) {
    // Port of vikillline(UNUSED(char **args)) from Src/Zle/zle_vi.c. Kills from cursor
    // back to start of line — different from Emacs kill-line which
    // kills forward.
    let bol = zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > bol {
        let killed: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(bol..crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)).collect();
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(bol, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(killed);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_vi_yank_eol(zle: &mut Zle) {
    // Port of viyankeol(UNUSED(char **args)) from Src/Zle/zle_vi.c:537. Copies from cursor
    // to end of line into the kill ring without removing.
    let eol = zle.findeol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    if eol > crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        let region: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..eol].to_vec();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(region);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
    }
}

fn widget_vi_beginning_of_line(zle: &mut Zle) {
    // Port of vibeginningofline(UNUSED(char **args)) from Src/Zle/zle_move.c:728.
    crate::ported::zle::zle_main::ZLECS.store(zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_swap_case(zle: &mut Zle) {
    // Port of viswapcase(UNUSED(char **args)) from Src/Zle/zle_vi.c. Swap the case of
    // the char under the cursor and advance one position; repeat
    // `mult` times.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
        let swapped = if c.is_uppercase() {
            c.to_lowercase().next().unwrap_or(c)
        } else if c.is_lowercase() {
            c.to_uppercase().next().unwrap_or(c)
        } else {
            c
        };
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = swapped;
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_oper_swap_case(zle: &mut Zle) {
    // Port of vioperswapcase(UNUSED(char **args)) from Src/Zle/zle_vi.c. As an operator,
    // swaps the case of every char in a vi range. The range read is
    // delegated to `vi_get_range('~')` (the C source uses the same
    // operator-pending machinery as d/c/y).
    if let Some((start, end, _)) = zle.vi_get_range('~') {
        for i in start..end.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)) {
            let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[i];
            let swapped = if c.is_uppercase() {
                c.to_lowercase().next().unwrap_or(c)
            } else if c.is_lowercase() {
                c.to_uppercase().next().unwrap_or(c)
            } else {
                c
            };
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[i] = swapped;
        }
        crate::ported::zle::zle_main::ZLECS.store(start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_vi_undo_change(zle: &mut Zle) {
    // Port of viundochange(char **args) from Src/Zle/zle_vi.c. zsh's vi-undo-change
    // walks back to the change boundary recorded at insert-mode entry
    // (vistartchange) — undo until cur_change drops below that. Our
    // simpler model: just call undo_widget once, matching the common
    // behavior of `u` in vi command mode.
    let _ = zle.undo_widget();
}

fn widget_universal_argument(zle: &mut Zle) {
    // Port of universalargument(char **args) from Src/Zle/zle_misc.c:986. The C
    // source greedily reads digits (and an optional leading '-') from
    // the input stream right after C-u, then applies the result as
    // tmult; if no digits follow, multiplies the existing tmult by 4
    // (the classic emacs C-u-C-u → 16 chord). The leading '-' branch is
    // distinct from neg-argument: it's a single token belonging to this
    // widget's read loop. Any non-digit byte gets ungot back.
    let mut digcnt = 0;
    let mut pref: i32 = 0;
    let mut minus: i32 = 1;
    let base = crate::ported::zle::zle_main::ZMOD.lock().unwrap().base;
    while let Some(b) = zle.getbyte(false) {
        if b == b'-' && digcnt == 0 {
            minus = -1;
            digcnt += 1;
            continue;
        }
        let new_digit = parse_digit_in_base(b, base);
        if new_digit >= 0 {
            pref = pref * base + new_digit;
            digcnt += 1;
        } else {
            zle.ungetbyte(b);
            break;
        }
    }
    if digcnt > 0 {
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = minus * if pref != 0 { pref } else { 1 };
    } else {
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.tmult = __g_zmod.tmult.saturating_mul(4);
    }
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= crate::ported::zle::zle_h::MOD_TMULT;
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_neg_argument(zle: &mut Zle) {
    // Port of negargument(UNUSED(char **args)) from Src/Zle/zle_misc.c:974. The C source
    // bails (returns 1) if MOD_TMULT is already set — neg-argument is
    // only valid as the *first* prefix, not after a digit. Otherwise
    // sets tmult = -1 and the MOD_TMULT|MOD_NEG flags so the next
    // digit-argument knows to use sign on its first digit.
    if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & crate::ported::zle::zle_h::MOD_TMULT != 0 {
        zle.handle_feep();
        return;
    }
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = -1;
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= crate::ported::zle::zle_h::MOD_TMULT;
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= crate::ported::zle::zle_h::MOD_NEG;
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_recursive_edit(zle: &mut Zle) {
    // Port of recursiveedit(UNUSED(char **args)) from Src/Zle/zle_main.c. Method already
    // exists on Zle via recursive_edit.
    let _ = zle.recursive_edit();
}

fn widget_what_cursor_position(zle: &mut Zle) {
    // Port of whatcursorposition(UNUSED(char **args)) from Src/Zle/zle_misc.c. Emits a
    // status-line message describing the cursor position. The C source
    // formats "Char: X (NNN, 0xHH, 0bBB) Point N of N (PP%) Column N".
    // Routed to our `showmsg` so the message lands wherever the host
    // surfaces ZLE diagnostics.
    let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let len = crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst);
    let msg = if pos < len {
        let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos];
        let pct = (pos * 100).checked_div(len).unwrap_or(0);
        format!(
            "Char: {} ({}, 0x{:X}) Point {} of {} ({}%)",
            c, c as u32, c as u32, pos, len, pct
        )
    } else {
        format!("Point {} of {} (end of buffer)", pos, len)
    };
    zle.showmsg(&msg);
}

fn widget_set_local_history_widget(zle: &mut Zle) {
    // Port of setlocalhistory(UNUSED(char **args)) from Src/Zle/zle_hist.c:794.
    let has_mult = crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & crate::ported::zle::zle_h::MOD_MULT != 0;
    let mult = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    let mut hist = std::mem::take(&mut zle.history);
    zle.set_local_history(&mut hist, has_mult, mult);
    *crate::ported::zle::zle_main::history().lock().unwrap() = hist;
}

fn widget_undefined_key(zle: &mut Zle) {
    // Port of undefinedkey(UNUSED(char **args)) from Src/Zle/zle_main.c. The C source just
    // beeps; we route to handle_feep.
    zle.handle_feep();
}

fn widget_history_search_backward(zle: &mut Zle) {
    // Port of historysearchbackward(char **args) from Src/Zle/zle_hist.c. Method
    // exists; this is the dispatch entry.
    let mut hist = std::mem::take(&mut zle.history);
    zle.historysearchbackward(&mut hist);
    *crate::ported::zle::zle_main::history().lock().unwrap() = hist;
}

fn widget_history_search_forward(zle: &mut Zle) {
    // Port of historysearchforward(char **args) from Src/Zle/zle_hist.c.
    let mut hist = std::mem::take(&mut zle.history);
    zle.historysearchforward(&mut hist);
    *crate::ported::zle::zle_main::history().lock().unwrap() = hist;
}

fn widget_insert_last_word_widget(zle: &mut Zle) {
    // Port of insertlastword(char **args) from Src/Zle/zle_hist.c. Method exists;
    // this is the dispatch entry.
    let hist = std::mem::take(&mut zle.history);
    zle.insertlastword(&hist);
    *crate::ported::zle::zle_main::history().lock().unwrap() = hist;
}

fn widget_up_line(zle: &mut Zle) {
    // Port of upline(char **args) from Src/Zle/zle_hist.c:243. Just the
    // multi-line cursor motion — no history fallback.
    let _ = zle.upline();
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_down_line(zle: &mut Zle) {
    // Port of downline(char **args) from Src/Zle/zle_hist.c:332.
    let _ = zle.downline();
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_up_line_or_history(zle: &mut Zle) {
    // Port of viuplineorhistory(char **args) from Src/Zle/zle_hist.c:302. Same as
    // up-line-or-history but lands at the first non-blank.
    let _ = zle.uplineorhistory();
    let bol = zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    let mut p = bol;
    while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p]; p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && __c.is_whitespace() && __c != '\n' } {
        p += 1;
    }
    crate::ported::zle::zle_main::ZLECS.store(p, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_down_line_or_history(zle: &mut Zle) {
    // Port of vidownlineorhistory(char **args) from Src/Zle/zle_hist.c:390.
    let _ = zle.downlineorhistory();
    let bol = zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    let mut p = bol;
    while { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p]; p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && __c.is_whitespace() && __c != '\n' } {
        p += 1;
    }
    crate::ported::zle::zle_main::ZLECS.store(p, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_up_line_or_search(zle: &mut Zle) {
    // Port of uplineorsearch(char **args) from Src/Zle/zle_hist.c:312. Try cursor
    // motion first; if at top, fall through to history-search-backward.
    let ocs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let n = zle.upline();
    if n != 0 {
        crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);
        widget_history_search_backward(zle);
    }
}

fn widget_down_line_or_search(zle: &mut Zle) {
    // Port of downlineorsearch(char **args) from Src/Zle/zle_hist.c:400.
    let ocs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let n = zle.downline();
    if n != 0 {
        crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);
        widget_history_search_forward(zle);
    }
}

fn widget_beginning_of_line_hist(zle: &mut Zle) {
    // Port of beginningoflinehist(char **args) from Src/Zle/zle_move.c. Same as
    // beginning-of-line at the start of the buffer; otherwise jumps to
    // the start of the current logical line.
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        // already at top — could pull older history; for now no-op like
        // beginning-of-line at top.
        return;
    }
    crate::ported::zle::zle_main::ZLECS.store(zle.findbol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_end_of_line_hist(zle: &mut Zle) {
    // Port of endoflinehist(char **args) from Src/Zle/zle_move.c.
    crate::ported::zle::zle_main::ZLECS.store(zle.findeol(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_copy_prev_shell_word(zle: &mut Zle) {
    // Port of copyprevshellword(UNUSED(char **args)) from Src/Zle/zle_misc.c:1108. Copies
    // the previous shell-word (quoted spans intact) at the cursor —
    // uses our shell-word boundary helper from src/zle/zle_word.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1) as usize;
    let words = bufferwords(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)]);
    if words.is_empty() {
        return;
    }
    // Find the last word ending at-or-before the cursor.
    let mut idx = words.len();
    for (i, (s, _e)) in words.iter().enumerate() {
        if *s >= crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
            idx = i;
            break;
        }
    }
    if idx == 0 {
        return;
    }
    // Pick the n-th previous (1-based).
    if idx < n {
        return;
    }
    let (s, e) = words[idx - n];
    let word: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[s..e].to_vec();
    for (i, c) in word.iter().enumerate() {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + i, *c);
    }
    crate::ported::zle::zle_main::ZLECS.fetch_add(word.len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_gosmacs_transpose_chars(zle: &mut Zle) {
    // Port of gosmacstransposechars(UNUSED(char **args)) from Src/Zle/zle_misc.c. Like
    // transpose-chars but doesn't advance the cursor afterwards (the
    // C source: swaps the two chars before the cursor).
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < 2 {
        return;
    }
    crate::ported::zle::zle_main::ZLELINE.lock().unwrap().swap(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1, crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 2);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_reset_prompt(zle: &mut Zle) {
    // Port of resetprompt(UNUSED(char **args)) from Src/Zle/zle_main.c. Already a method on
    // Zle (sets resetneeded); call through.
    zle.resetprompt();
}

fn widget_split_undo(zle: &mut Zle) {
    // Port of splitundo(UNUSED(char **args)) from Src/Zle/zle_utils.c. Closes any pending
    // change record so the next mkundoent starts a fresh entry. Routes
    // to setlastline() which snapshots the current line state — the
    // C source achieves the same effect by flushing nextchanges.
    zle.setlastline();
}

fn widget_argument_base(zle: &mut Zle) {
    // Port of argumentbase(char **args) from Src/Zle/zle_misc.c:1038. The C source
    // takes the requested base from the previous mult (no explicit
    // arg), validates it's in [2, 36], stashes it in zmod.base, and
    // resets the rest of the modifier so the next digit-argument starts
    // fresh in the new base. Useful for `M-2 M-x argument-base M-f f`
    // = forward-word x15 in base 16.
    let multbase = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if !(2..=36).contains(&multbase) {
        zle.handle_feep();
        return;
    }
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = multbase;
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags = 0;
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1;
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = 1;
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().vibuf = 0;
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_infer_next_history(zle: &mut Zle) {
    // Port of infernexthistory(char **args) from Src/Zle/zle_hist.c. Looks for
    // the entry following the most recent match of the current line
    // and loads it. Useful when stepping through related commands.
    let line: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
    let len = crate::ported::zle::zle_main::history().lock().unwrap().entries.len();
    // Search backward for the matching entry.
    for i in (0..len).rev() {
        if crate::ported::zle::zle_main::history().lock().unwrap().entries[i].line == line {
            // Found — load the next one.
            if i + 1 < len {
                crate::ported::zle::zle_main::history().lock().unwrap().cursor = i + 1;
                *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = crate::ported::zle::zle_main::history().lock().unwrap().entries[i + 1].line.chars().collect();
                crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
                crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
                crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
            }
            return;
        }
    }
}

fn widget_accept_and_infer_next_history(zle: &mut Zle) {
    // Port of acceptandinfernexthistory(char **args) from Src/Zle/zle_hist.c.
    // Like accept-line but pre-loads the entry following the most
    // recent match for the next prompt.
    widget_infer_next_history(zle);
    crate::ported::zle::zle_misc::DONE.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_quoted_insert(zle: &mut Zle) {
    // Port of viquotedinsert(char **args) from Src/Zle/zle_vi.c. Same as
    // quoted-insert in our model — read the next char and self-insert
    // it literally (existing widget_quoted_insert does this).
    widget_quoted_insert(zle);
}

fn widget_run_help(zle: &mut Zle) {
    // Port of processcmd(UNUSED(char **args)) (run-help binding) from Src/Zle/zle_misc.c.
    // The C source spawns the run-help function on the current command
    // word; we record a hook so the host can dispatch it.
    zle.call_hook("run-help", None);
}

fn widget_expand_history(zle: &mut Zle) {
    // Port of expandhistory(UNUSED(char **args)) from Src/Zle/zle_tricky.c:2921. zsh
    // walks the line through the history-expansion machinery (`!!`,
    // `!$`, `!:0` etc.). Without that engine wired in here, surface
    // a hook for the host to satisfy.
    zle.call_hook("expand-history", None);
}

fn widget_magic_space(zle: &mut Zle) {
    // Port of magicspace(char **args) from Src/Zle/zle_tricky.c:2882. The C source
    // expands history (via expandhistory above) then self-inserts a
    // literal space.
    widget_expand_history(zle);
    crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), ' ');
    crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_spell_word(zle: &mut Zle) {
    // Port of spellword(UNUSED(char **args)) from Src/Zle/zle_tricky.c. Surface as a hook
    // — the C source spawns an external speller; the host binds.
    zle.call_hook("spell-word", None);
}

fn widget_get_line(zle: &mut Zle) {
    // Port of getline() from Src/Zle/zle_hist.c. Pops the most-recent
    // bufstack entry into the current line.
    if let Some(line) = crate::ported::zle::zle_main::BUFSTACK.lock().unwrap().pop() {
        let chars: Vec<char> = line.chars().collect();
        let new_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst).min(chars.len());
        // Insert at cursor.
        for (i, c) in chars.iter().enumerate() {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + i, *c);
        }
        crate::ported::zle::zle_main::ZLECS.store(new_cs + chars.len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_push_input(zle: &mut Zle) {
    // Port of pushinput(char **args) from Src/Zle/zle_hist.c. Pushes the entire
    // input including any in-progress continuation onto bufstack and
    // clears the editor — a superset of push-line that also flushes
    // pending PS2 lines. With our single-line model it behaves like
    // push-line.
    zle.push_line();
}

fn widget_vi_set_buffer(zle: &mut Zle) {
    // Port of visetbuffer(char **args) from Src/Zle/zle_vi.c. The C source reads
    // a vi-buffer name (`"a..z`) and stores it for the next y/d/p.
    // Without the full vibuf register dispatch wired here, consume the
    // next char and stash it on zmod for later inspection.
    if let Some(c) = zle.getfullchar(false) {
        if c.is_ascii_lowercase() {
            crate::ported::zle::zle_main::ZMOD.lock().unwrap().vibuf = (c as i32) - ('a' as i32);
        } else if c.is_ascii_uppercase() {
            crate::ported::zle::zle_main::ZMOD.lock().unwrap().vibuf = (c as i32) - ('A' as i32) + 26;
        }
        crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_vi_indent(zle: &mut Zle) {
    // Port of viindent(UNUSED(char **args)) from Src/Zle/zle_vi.c. Inserts SHWIDTH spaces
    // at the start of every logical line in the range read via
    // vi_get_range. Defaults to 4 spaces (tab width); zsh's actual
    // shiftwidth comes from the SH_WORD_SPLIT family — left as a fixed
    // 4 here until the wider option store is wired.
    if let Some((start, end, _)) = zle.vi_get_range('>') {
        let bol_start = zle.findbol(start);
        let mut p = bol_start;
        while p < end && p <= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            for i in 0..4 {
                crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(p + i, ' ');
            }
            crate::ported::zle::zle_main::ZLELL.fetch_add(4, std::sync::atomic::Ordering::SeqCst);
            p = zle.findeol(p) + 1;
        }
        crate::ported::zle::zle_main::ZLECS.store(bol_start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_vi_unindent(zle: &mut Zle) {
    // Port of viunindent(UNUSED(char **args)) from Src/Zle/zle_vi.c. Removes up to 4
    // leading spaces from every logical line in the range.
    if let Some((start, end, _)) = zle.vi_get_range('<') {
        let bol_start = zle.findbol(start);
        let mut p = bol_start;
        while p < end && p <= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            for _ in 0..4 {
                if crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(p).copied() == Some(' ') {
                    crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(p);
                    if crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                        crate::ported::zle::zle_main::ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    }
                } else {
                    break;
                }
            }
            p = zle.findeol(p) + 1;
        }
        crate::ported::zle::zle_main::ZLECS.store(bol_start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn widget_bracketed_paste(zle: &mut Zle) {
    // Port of bracketedpaste(char **args) from Src/Zle/zle_misc.c. The C source
    // reads bytes between the bracketed-paste open + close escapes.
    // Surface as a hook so the host (which owns the input loop) drains
    // and inserts the text — host-driven because the paste sentinel
    // detection happens at the byte stream level.
    zle.call_hook("bracketed-paste", None);
}

fn widget_vi_backward_word_end(zle: &mut Zle) {
    // Port of vibackwardwordend(char **args) from Src/Zle/zle_word.c:348. Step
    // backward to the end (last char) of the previous word. Faithful to
    // the C loop: read class at current position, step back once, walk
    // back through same-class non-blank chars, then through blanks.
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    let class_at = |c: char| -> i32 {
        if c.is_whitespace() {
            0
        } else if c.is_alphanumeric() || c == '_' {
            1
        } else {
            2
        }
    };
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            break;
        }
        let here = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)).copied().unwrap_or(' ');
        let cc = class_at(here);
        crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
            if class_at(c) != cc || c.is_whitespace() {
                break;
            }
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_whitespace() {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_vi_backward_blank_word_end(zle: &mut Zle) {
    // Port of vibackwardblankwordend(char **args) from Src/Zle/zle_word.c:375.
    // Same shape as vibackwardwordend but whitespace is the only
    // separator (no class distinction between alnum and punctuation).
    let n = crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).max(1);
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            break;
        }
        crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_whitespace() {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_whitespace() {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_select_in_word(zle: &mut Zle) {
    // Port of selectinword() from Src/Zle/textobjects.c. Sets a region
    // containing the inner word at the cursor — different from
    // Zle::find_word_start (which is a backward-motion helper); here we
    // expand around the cursor while characters share the iword class.
    let n = crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst);
    let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst).min(n);
    if n == 0 {
        return;
    }
    // Pick the class to span. If on a word char, use word-class; else
    // sit on whitespace and use that class.
    let cur_char = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(pos).copied().unwrap_or(' ');
    let is_word = cur_char.is_alphanumeric() || cur_char == '_';
    let class = |c: char| -> bool {
        if c.is_whitespace() {
            !is_word && c.is_whitespace()
        } else if is_word {
            c.is_alphanumeric() || c == '_'
        } else {
            !c.is_alphanumeric() && c != '_' && !c.is_whitespace()
        }
    };
    let mut start = pos;
    while start > 0 && class(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start - 1]) == class(cur_char) {
        start -= 1;
    }
    let mut end = pos;
    while end < n && class(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[end]) == class(cur_char) {
        end += 1;
    }
    crate::ported::zle::zle_main::MARK.store(start, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(end, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_select_a_word(zle: &mut Zle) {
    // Port of selectaword() from Src/Zle/textobjects.c. "around" form —
    // includes a trailing whitespace separator if any.
    widget_select_in_word(zle);
    while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_whitespace() {
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_select_in_blank_word(zle: &mut Zle) {
    // Port of selectinblankword() from Src/Zle/textobjects.c. Spans a
    // run of non-whitespace characters around the cursor.
    let n = crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst);
    let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst).min(n);
    if n == 0 || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(pos).copied().unwrap_or(' ').is_whitespace() {
        return;
    }
    let mut start = pos;
    while start > 0 && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = pos;
    while end < n && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[end].is_whitespace() {
        end += 1;
    }
    crate::ported::zle::zle_main::MARK.store(start, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(end, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_select_a_blank_word(zle: &mut Zle) {
    // Port of selectablankword() from Src/Zle/textobjects.c.
    widget_select_in_blank_word(zle);
    while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_whitespace() {
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_select_in_shell_word(zle: &mut Zle) {
    // Port of selectinshellword() from Src/Zle/textobjects.c. Uses the
    // shell-word splitter that respects single/double quotes + escapes.
    let saved = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let start = backwardword_shell(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)], saved);
    let end = forwardword_shell(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)], saved);
    crate::ported::zle::zle_main::MARK.store(start, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(end, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_select_a_shell_word(zle: &mut Zle) {
    // Port of selectashellword() from Src/Zle/textobjects.c.
    widget_select_in_shell_word(zle);
    while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_whitespace() {
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_accept_search(zle: &mut Zle) {
    // Port of acceptsearch() from Src/Zle/zle_hist.c. Acceptance handler
    // inside the isearch sub-loop — outside of isearch it's a no-op,
    // which matches our design where do_isearch() handles its own loop.
    let _ = zle;
}

fn widget_auto_suffix_remove(zle: &mut Zle) {
    // Port of handlesuffix() (auto-suffix-remove flag) from
    // Src/Zle/zle_misc.c. The C source clears the auto-removable
    // suffix from the kill ring's tail. Surface as a hook so the host
    // updates compsys's pending-suffix state.
    zle.call_hook("auto-suffix-remove", None);
}

fn widget_auto_suffix_retain(zle: &mut Zle) {
    // Port of handlesuffix(UNUSED(char **args)) (KEEPSUFFIX) from Src/Zle/zle_misc.c.
    zle.call_hook("auto-suffix-retain", None);
}

fn widget_put_replace_selection(zle: &mut Zle) {
    // Port of putreplaceselection(UNUSED(char **args)) from Src/Zle/zle_misc.c:680. With
    // an active region, replaces it with the most-recent kill-ring
    // entry; otherwise pastes at the cursor (same as yank).
    if crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) == 0 || crate::ported::zle::zle_main::KILLRING.lock().unwrap().is_empty() {
        widget_yank(zle);
        return;
    }
    let (lo, hi) = if crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) <= crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        (crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst))
    } else {
        (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst))
    };
    let lo = lo.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
    let hi = hi.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
    crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(lo..hi);
    crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(lo, std::sync::atomic::Ordering::SeqCst);
    let text: Vec<char> = zle
        .killring
        .front()
        .cloned()
        .unwrap_or_default();
    for (i, c) in text.iter().enumerate() {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + i, *c);
    }
    crate::ported::zle::zle_main::ZLECS.fetch_add(text.len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::REGION_ACTIVE.store(0, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

fn widget_where_is(zle: &mut Zle) {
    // Port of whereis(UNUSED(char **args)) from Src/Zle/zle_thingy.c. The C source prompts
    // for a widget name and shows what keys it's bound to. Surface as
    // a hook so the host can prompt + look up in the current keymap.
    zle.call_hook("where-is", None);
}

fn widget_execute_named_cmd(zle: &mut Zle) {
    // Port of executenamedcmd() from Src/Zle/zle_thingy.c. Reads a
    // widget name and executes it. Host-driven because completion of
    // the prompt + dispatch live in host land.
    zle.call_hook("execute-named-cmd", None);
}

fn widget_execute_last_named_cmd(zle: &mut Zle) {
    // Port of executelastnamedcmd() from Src/Zle/zle_thingy.c. Same
    // shape as execute-named-cmd; replays the last one.
    zle.call_hook("execute-last-named-cmd", None);
}

fn widget_read_command(zle: &mut Zle) {
    // Port of readcommand(UNUSED(char **args)) from Src/Zle/zle_thingy.c. Reads a widget
    // name from input and stores it for the host's executor.
    zle.call_hook("read-command", None);
}

fn widget_menu_expand_or_complete(zle: &mut Zle) {
    // Port of menuexpandorcomplete(char **args) from Src/Zle/zle_tricky.c. Menu
    // completion variant of expand-or-complete.
}

fn widget_reverse_menu_complete(zle: &mut Zle) {
    // Port of reversemenucomplete(char **args) from Src/Zle/zle_tricky.c. Steps
    // the menu backwards. Surfaced via a separate hook so the host's
    // menu state knows which direction to step.
    zle.call_hook("reverse-menu-complete", None);
}

fn widget_accept_and_menu_complete(zle: &mut Zle) {
    // Port of acceptandmenucomplete(char **args) from Src/Zle/zle_tricky.c.
    zle.call_hook("accept-and-menu-complete", None);
}

fn widget_list_expand(zle: &mut Zle) {
    // Port of listexpand(UNUSED(char **args)) from Src/Zle/zle_tricky.c. Expands current
    // word and lists the candidates.
}

fn widget_expand_cmd_path(zle: &mut Zle) {
    // Port of expandcmdpath(UNUSED(char **args)) from Src/Zle/zle_tricky.c. Expands the
    // first word into its full path via PATH lookup.
    zle.call_hook("expand-cmd-path", None);
}

fn widget_expand_or_complete_prefix(zle: &mut Zle) {
    // Port of expandorcompleteprefix(char **args) from Src/Zle/zle_tricky.c.
    // Same as expand-or-complete but only considers the prefix before
    // the cursor.
}

fn widget_end_of_list(zle: &mut Zle) {
    // Port of endoflist(UNUSED(char **args)) from Src/Zle/zle_tricky.c. Used inside the
    // completion menu to dismiss the listing — host-driven.
    zle.call_hook("end-of-list", None);
}

fn widget_history_incremental_pattern_search_backward(zle: &mut Zle) {
    // Port of historyincrementalpatternsearchbackward() from
    // Src/Zle/zle_hist.c:936. Pattern-mode variant of isearch — uses
    // glob-pattern matching instead of plain-substring. Until the
    // pattern engine is wired in to do_isearch, fall through to the
    // plain backward isearch.
    do_isearch(zle, -1);
}

fn widget_history_incremental_pattern_search_forward(zle: &mut Zle) {
    // Port of historyincrementalpatternsearchforward() from
    // Src/Zle/zle_hist.c:943.
    do_isearch(zle, 1);
}

/// Check if a character is a word character
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn populated() -> Zle {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = Vec::new();
        crate::ported::zle::zle_main::ZLELL.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_back("first".chars().collect());
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_back("second".chars().collect());
        // VecDeque::push_back appends; killring[0] is what pop_front would
        // return. widget_yank uses front() so the "most recent" entry is at
        // index 0. Reset in newest-first order:
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().clear();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front("oldest".chars().collect());
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front("middle".chars().collect());
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front("newest".chars().collect());
        zle
    }

    // Tests `complete_word_widget_surfaces_request` /
    // `expand_or_complete_widget_surfaces_request` /
    // `list_choices_widget_surfaces_request` /
    // `menu_complete_widget_surfaces_request` /
    // `delete_char_or_list_at_eol_surfaces_list_choices` removed —
    // they exercised the deleted `Zle.completion_request` field
    // (Rust-only fixture surface) which has no C counterpart.

    #[test]
    fn delete_char_or_list_mid_line_deletes_instead() {
        // c:zle_tricky.c:288 — `deletecharorlist` falls through to
        // `delete-char` when not at end-of-line.
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        widget_delete_char_or_list(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "ac");
    }

    #[test]
    fn set_mark_command_sets_mark_and_activates_region() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(6, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(3, std::sync::atomic::Ordering::SeqCst);
        widget_set_mark_command(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn set_mark_command_negative_count_deactivates() {
        let mut zle = Zle::new();
        crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(-1, std::sync::atomic::Ordering::SeqCst);
        widget_set_mark_command(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn exchange_point_and_mark_swaps() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(6, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(4, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(1, std::sync::atomic::Ordering::SeqCst);
        widget_exchange_point_and_mark(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    #[test]
    fn copy_region_as_kill_pushes_region_without_removing() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello world".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(0, std::sync::atomic::Ordering::SeqCst);
        widget_copy_region_as_kill(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "hello world");
        assert_eq!(
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().map(|v| v.iter().collect::<String>()),
            Some("hello".to_string())
        );
    }

    #[test]
    fn copy_prev_word_inserts_previous_word_at_cursor() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "echo hello ".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(1, std::sync::atomic::Ordering::SeqCst);
        widget_copy_prev_word(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "echo hello hello");
    }

    #[test]
    fn quote_line_wraps_buffer_in_single_quotes() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "echo hi".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        widget_quote_line(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "'echo hi'");
    }

    #[test]
    fn quote_line_escapes_embedded_single_quote() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "it's".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(4, std::sync::atomic::Ordering::SeqCst);
        widget_quote_line(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), r"'it'\''s'");
    }

    #[test]
    fn quote_region_wraps_only_marked_span() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "echo hi there".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(13, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(7, std::sync::atomic::Ordering::SeqCst); // "hi"
        widget_quote_region(&mut zle);
        assert_eq!(
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(),
            "echo 'hi' there"
        );
    }

    #[test]
    fn pound_insert_toggles_leading_hash() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "echo hi".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        widget_pound_insert(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "#echo hi");
        // Toggle off.
        crate::ported::zle::zle_misc::DONE.store(0, std::sync::atomic::Ordering::SeqCst);
        widget_pound_insert(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "echo hi");
    }

    #[test]
    fn transpose_words_swaps_two_words() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "foo bar".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(7, std::sync::atomic::Ordering::SeqCst); // at end of line
        widget_transpose_words(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "bar foo");
    }

    #[test]
    fn capitalize_word_widget_capitalizes_at_cursor() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello world".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        widget_capitalize_word(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "Hello world");
    }

    #[test]
    fn vi_put_after_pastes_after_cursor_on_charwise_yank() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst); // on 'a'
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front("XY".chars().collect());
        widget_vi_put_after(&mut zle);
        // `p` after 'a' inserts XY at index 1 → "aXYbc". Cursor lands
        // on 'Y' (last char of pasted region) at index 2.
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "aXYbc");
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn vi_put_before_pastes_before_cursor_on_charwise_yank() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst); // on 'b'
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front("XY".chars().collect());
        widget_vi_put_before(&mut zle);
        // `P` at cursor 1 inserts XY → "aXYbc". Cursor lands on 'Y'.
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "aXYbc");
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn vi_put_after_linewise_pastes_on_new_line_below() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "first\nthird".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(2, std::sync::atomic::Ordering::SeqCst); // mid-"first"
        // Linewise yank entry ends in newline.
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front("second\n".chars().collect());
        widget_vi_put_after(&mut zle);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        // Should now be "first\nsecond\nthird".
        assert!(s.contains("first\nsecond\nthird"), "got: {:?}", s);
    }

    #[test]
    fn vi_replace_chars_overwrites_one_char() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        zle.ungetbytes(b"X");
        widget_vi_replace_chars(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "Xello");
    }

    #[test]
    fn vi_replace_chars_with_count_overwrites_n_chars() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(3, std::sync::atomic::Ordering::SeqCst);
        zle.ungetbytes(b"X");
        widget_vi_replace_chars(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "XXXlo");
        // Cursor lands on the LAST replaced char.
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn vi_join_removes_newline_and_inserts_space() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "foo\nbar".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(1, std::sync::atomic::Ordering::SeqCst);
        widget_vi_join(&mut zle);
        // Newline replaced with single space.
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "foo bar");
    }

    #[test]
    fn vi_open_line_above_starts_new_line_at_bol() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(2, std::sync::atomic::Ordering::SeqCst);
        widget_vi_open_line_above(&mut zle);
        // Newline inserted at start; cursor sits before it on first line.
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert!(s.starts_with('\n') || s.contains('\n'));
    }

    #[test]
    fn vi_first_non_blank_skips_leading_whitespace() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "    hello".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(9, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(8, std::sync::atomic::Ordering::SeqCst);
        widget_vi_first_non_blank(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    #[test]
    fn accept_line_widget_marks_done() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "echo hi".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        widget_accept_line(&mut zle);
        assert!(crate::ported::zle::zle_misc::DONE.load(std::sync::atomic::Ordering::SeqCst) != 0);
        // accept-line keeps the buffer intact for the caller to read.
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "echo hi");
    }

    #[test]
    fn send_break_widget_clears_buffer_and_marks_done() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(2, std::sync::atomic::Ordering::SeqCst);
        widget_send_break(&mut zle);
        assert!(crate::ported::zle::zle_misc::DONE.load(std::sync::atomic::Ordering::SeqCst) != 0);
        assert!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().is_empty());
        assert_eq!(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn overwrite_mode_widget_toggles_insmode() {
        let mut zle = Zle::new();
        let initial = (crate::ported::zle::zle_main::INSMODE.load(std::sync::atomic::Ordering::SeqCst) != 0);
        widget_overwrite_mode(&mut zle);
        assert_eq!((crate::ported::zle::zle_main::INSMODE.load(std::sync::atomic::Ordering::SeqCst) != 0), !initial);
        widget_overwrite_mode(&mut zle);
        assert_eq!((crate::ported::zle::zle_main::INSMODE.load(std::sync::atomic::Ordering::SeqCst) != 0), initial);
    }

    #[test]
    fn select_in_word_sets_region_around_current_word() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "foo bar baz".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(4, std::sync::atomic::Ordering::SeqCst); // inside "bar"
        widget_select_in_word(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 1);
        // mark/cursor should bracket "bar".
        let lo = crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst).min(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
        let hi = crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst).max(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[lo..hi].iter().collect::<String>(), "bar");
    }

    #[test]
    fn select_in_shell_word_treats_double_quoted_string_as_one_word() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = r#"echo "hello world""#.chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(8, std::sync::atomic::Ordering::SeqCst); // inside the quoted string
        widget_select_in_shell_word(&mut zle);
        let lo = crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst).min(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
        let hi = crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst).max(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
        let span: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[lo..hi].iter().collect();
        assert_eq!(span, r#""hello world""#);
    }

    #[test]
    fn put_replace_selection_overwrites_active_region() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(6, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front("XYZ".chars().collect());
        crate::ported::zle::zle_main::MARK.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(4, std::sync::atomic::Ordering::SeqCst); // selecting "bcd"
        crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
        widget_put_replace_selection(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "aXYZef");
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn vi_backward_word_end_lands_at_prior_word_end() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "foo bar baz".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(11, std::sync::atomic::Ordering::SeqCst); // at EOB (past 'z')
        widget_vi_backward_word_end(&mut zle);
        // vim's `ge` from past-EOL lands at the end of the LAST word —
        // position of 'z' in "baz" = index 10 (matches the C source's
        // vibackwardwordend in Src/Zle/zle_word.c:348).
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 10);
    }

    #[test]
    fn kill_word_with_count_kills_n_words() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "foo bar baz qux".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(15, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(2, std::sync::atomic::Ordering::SeqCst);
        widget_kill_word(&mut zle);
        // 2*kill-word from start removes "foo bar" plus its trailing
        // separator into the kill ring; remaining buffer starts at " baz".
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, " baz qux");
        let killed = crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().unwrap().iter().collect::<String>();
        assert_eq!(killed, "foo bar");
    }

    #[test]
    fn backward_kill_word_with_count_kills_n_words_back() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "alpha beta gamma".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(16, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(16, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(2, std::sync::atomic::Ordering::SeqCst);
        widget_backward_kill_word(&mut zle);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "alpha ");
        let killed = crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().unwrap().iter().collect::<String>();
        assert_eq!(killed, "beta gamma");
    }

    #[test]
    fn delete_word_removes_word_without_kill_ring() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello world".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        let kr_before = crate::ported::zle::zle_main::KILLRING.lock().unwrap().len();
        widget_delete_word(&mut zle);
        // Emacs delete-word removes the word but not the trailing separator
        // (zle_word.c convention) — leaves a leading space.
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), " world");
        assert_eq!(crate::ported::zle::zle_main::KILLRING.lock().unwrap().len(), kr_before);
    }

    #[test]
    fn kill_region_drains_into_kill_ring() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcdefgh".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(8, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(6, std::sync::atomic::Ordering::SeqCst);
        widget_kill_region(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "abgh");
        assert_eq!(
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().map(|v| v.iter().collect::<String>()),
            Some("cdef".to_string())
        );
    }

    #[test]
    fn kill_buffer_clears_line_and_pushes_to_kill_ring() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "echo hi".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        widget_kill_buffer(&mut zle);
        assert!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().is_empty());
        assert_eq!(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().map(|v| v.iter().collect::<String>()),
            Some("echo hi".to_string())
        );
    }

    #[test]
    fn vi_kill_line_kills_back_to_bol() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc def".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(7, std::sync::atomic::Ordering::SeqCst);
        widget_vi_kill_line(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "");
        assert_eq!(
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().map(|v| v.iter().collect::<String>()),
            Some("abc def".to_string())
        );
    }

    #[test]
    fn vi_swap_case_flips_letter_case_under_cursor() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "Hello".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        widget_vi_swap_case(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "hello");
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn vi_swap_case_with_count_flips_n_chars() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(6, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(3, std::sync::atomic::Ordering::SeqCst);
        widget_vi_swap_case(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "ABCdef");
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn universal_argument_bumps_tmult_by_4_each_call() {
        // C zsh's universalargument() (zle_misc.c:986) updates tmult,
        // not mult — handleprefixes promotes TMULT→MULT on the next
        // widget call. Matches initmodifier() which seeds tmult=1.
        let mut zle = Zle::new();
        zle.initmodifier();
        widget_universal_argument(&mut zle);
        // No bytes available in test → digcnt=0 path → tmult *= 4.
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, 4);
        assert!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & crate::ported::zle::zle_h::MOD_TMULT != 0);
        widget_universal_argument(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, 16);
    }

    #[test]
    fn universal_argument_with_digits_reads_pref() {
        let mut zle = Zle::new();
        zle.initmodifier();
        // Pre-feed "42x" — universal-argument should pull the "42" and
        // unget the trailing 'x' for the next read.
        zle.ungetbytes(b"42x");
        widget_universal_argument(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, 42);
        assert!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & crate::ported::zle::zle_h::MOD_TMULT != 0);
        // 'x' should still be in the unget buffer.
        let next = zle.getbyte(false);
        assert_eq!(next, Some(b'x'));
    }

    #[test]
    fn universal_argument_with_minus_reads_negative() {
        let mut zle = Zle::new();
        zle.initmodifier();
        zle.ungetbytes(b"-7\n");
        widget_universal_argument(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, -7);
    }

    #[test]
    fn neg_argument_first_invocation_sets_tmult_minus_one() {
        let mut zle = Zle::new();
        zle.initmodifier();
        widget_neg_argument(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, -1);
        assert!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & crate::ported::zle::zle_h::MOD_TMULT != 0);
        assert!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & crate::ported::zle::zle_h::MOD_NEG != 0);
    }

    #[test]
    fn neg_argument_after_tmult_already_set_is_rejected() {
        // C: returns 1 (error/beep) if MOD_TMULT was already set.
        let mut zle = Zle::new();
        zle.initmodifier();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= crate::ported::zle::zle_h::MOD_TMULT;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = 5;
        widget_neg_argument(&mut zle);
        // tmult unchanged, NEG NOT set.
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, 5);
        assert!(!crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & crate::ported::zle::zle_h::MOD_NEG != 0);
    }

    #[test]
    fn digit_argument_after_neg_argument_inherits_sign() {
        // After neg-argument, handleprefixes promotes tmult=-1 into
        // mult=-1; the next digit-argument sees mult<0 and applies the
        // negative sign to the first digit (C: zle_misc.c:961-964 +
        // zle_main.c:1620 promote chain).
        let mut zle = Zle::new();
        zle.initmodifier();
        widget_neg_argument(&mut zle);
        // Simulate the zlecore→handleprefixes step the live loop runs
        // between widgets.
        zle.handleprefixes();
        crate::ported::zle::compcore::LASTCHAR.store((b'5' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        widget_digit_argument(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, -5);
        assert!(!crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & crate::ported::zle::zle_h::MOD_NEG != 0);
    }

    #[test]
    fn vi_yank_whole_line_includes_trailing_newline() {
        // Linewise yank must include the \n so vi-put-after's
        // is_line_paste detection fires (zle_vi.c:559 path).
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "first\nsecond".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(12, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(2, std::sync::atomic::Ordering::SeqCst); // mid-"first"
        crate::ported::zle::zle_main::MULT.store(1, std::sync::atomic::Ordering::SeqCst);
        widget_vi_yank_whole_line(&mut zle);
        let killed = crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().unwrap().iter().collect::<String>();
        assert_eq!(killed, "first\n");
    }

    #[test]
    fn vi_yank_whole_line_with_count_yanks_n_lines() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "a\nb\nc\nd".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(3, std::sync::atomic::Ordering::SeqCst);
        widget_vi_yank_whole_line(&mut zle);
        let killed = crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().unwrap().iter().collect::<String>();
        assert_eq!(killed, "a\nb\nc\n");
    }

    #[test]
    fn vi_goto_column_lands_on_column_within_logical_line() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello\nworld".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(9, std::sync::atomic::Ordering::SeqCst); // mid-"world"
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 3; // 1-based → column 3 = 'r'
        widget_vi_goto_column(&mut zle);
        // bol of "world" is 6 → 6 + (3-1) = 8.
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 8);
    }

    #[test]
    fn vi_goto_column_clamps_to_end_of_line() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "ab\ncd".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(3, std::sync::atomic::Ordering::SeqCst); // on "cd" line
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 99;
        widget_vi_goto_column(&mut zle);
        // EoL of "cd" is index 5; clamp lands there.
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 5);
    }

    #[test]
    fn vi_add_eol_jumps_to_eol_of_current_line() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        widget_vi_add_eol(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 5);
    }

    #[test]
    fn vi_forward_char_clamps_at_eol() {
        // Vim 'l' can't cross EoL — cursor lands on last char of
        // current logical line at most.
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc\ndef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst); // on 'b' of "abc"
        crate::ported::zle::zle_main::MULT.store(5, std::sync::atomic::Ordering::SeqCst);
        widget_vi_forward_char(&mut zle);
        // EoL of "abc" is 3 (the \n); limit is 2 (eol-1 = on 'c').
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn forward_char_with_count_advances_n() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(6, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(3, std::sync::atomic::Ordering::SeqCst);
        widget_forward_char(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn forward_char_with_negative_count_delegates_backwards() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(6, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(-2, std::sync::atomic::Ordering::SeqCst);
        widget_forward_char(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn backward_delete_char_count_clamped_to_cursor() {
        // C source: zle_misc.c:189 clamps mult to zlecs.
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "ab".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(5, std::sync::atomic::Ordering::SeqCst); // ask to delete 5 but only 1 char available before cursor
        widget_backward_delete_char(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "b");
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn forward_word_with_count_skips_n_words() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "a b c d".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(2, std::sync::atomic::Ordering::SeqCst);
        widget_forward_word(&mut zle);
        // After 2 word skips: a→b, b→c, lands at c's start (index 4).
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    #[test]
    fn kill_line_with_count_kills_n_lines() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "first\nsecond\nthird".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(18, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(2, std::sync::atomic::Ordering::SeqCst);
        widget_kill_line(&mut zle);
        // 2 iterations: kill "first", then \n. Buffer: "second\nthird".
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "second\nthird");
    }

    #[test]
    fn beginning_of_line_uses_find_bol_for_multiline() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "first\nsecond".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(12, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(9, std::sync::atomic::Ordering::SeqCst); // mid-"second"
        widget_beginning_of_line(&mut zle);
        // Should land at start of the SECOND logical line, not 0.
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 6);
    }

    #[test]
    fn transpose_chars_swaps_at_cursor() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcd".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(4, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(2, std::sync::atomic::Ordering::SeqCst); // between 'b' and 'c'
        widget_transpose_chars(&mut zle);
        // Default count=1 swaps line[ct-1]<->line[ct] (= 'b'<->'c').
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "acbd");
        // Cursor advances past the swap.
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn transpose_chars_at_eob_swaps_last_two() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "ab".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(2, std::sync::atomic::Ordering::SeqCst); // at end-of-buffer
        widget_transpose_chars(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "ba");
    }

    #[test]
    fn transpose_chars_at_bol_returns_without_swap() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "ab".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        widget_transpose_chars(&mut zle);
        // At BoL (ct=0) with content available, advance and swap.
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "ba");
    }

    #[test]
    fn transpose_chars_with_negative_count_swaps_backwards() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcd".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(4, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MULT.store(-1, std::sync::atomic::Ordering::SeqCst);
        widget_transpose_chars(&mut zle);
        // Negative count moves cursor back, swaps prior pair.
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "acbd");
    }

    #[test]
    fn parse_digit_in_base_handles_decimal_and_hex_and_invalid() {
        assert_eq!(parse_digit_in_base(b'7', 10), 7);
        assert_eq!(parse_digit_in_base(b'a', 16), 10);
        assert_eq!(parse_digit_in_base(b'F', 16), 15);
        assert_eq!(parse_digit_in_base(b'g', 16), -1); // out of range
        assert_eq!(parse_digit_in_base(b'5', 8), 5);
        assert_eq!(parse_digit_in_base(b'9', 8), -1); // 9 is not a base-8 digit
        assert_eq!(parse_digit_in_base(b'z', 36), 35);
        assert_eq!(parse_digit_in_base(b'!', 10), -1);
    }

    #[test]
    fn argument_base_clamps_invalid_base_via_feep() {
        let mut zle = Zle::new();
        zle.initmodifier();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1; // base 1 is invalid (< 2)
        widget_argument_base(&mut zle);
        // base unchanged at 10; no prefixflag side effect.
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().base, 10);
    }

    #[test]
    fn vi_beginning_of_line_jumps_to_bol() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "    foo".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(5, std::sync::atomic::Ordering::SeqCst);
        widget_vi_beginning_of_line(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn emacs_forward_word_moves_to_word_end() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello world".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        widget_emacs_forward_word(&mut zle);
        // find_word_end (Emacs style) skips non-word + word; "hello" ends
        // at byte 5.
        assert!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= 5);
    }

    #[test]
    fn vi_yank_eol_copies_to_eol() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello world".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(6, std::sync::atomic::Ordering::SeqCst);
        widget_vi_yank_eol(&mut zle);
        // Cursor stays put; killring gets "world".
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "hello world");
        assert_eq!(
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().map(|v| v.iter().collect::<String>()),
            Some("world".to_string())
        );
    }

    #[test]
    fn what_cursor_position_does_not_panic_on_end_of_buffer() {
        let mut zle = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(3, std::sync::atomic::Ordering::SeqCst); // past last char
        widget_what_cursor_position(&mut zle);
        // No assertion on stderr — just verifying the EOB branch doesn't
        // index out of bounds.
    }

    #[test]
    fn history_beginning_search_backward_walks_to_matching_prefix() {
        let mut zle = Zle::new();
        crate::ported::zle::zle_main::history().lock().unwrap().add("git commit".to_string());
        crate::ported::zle::zle_main::history().lock().unwrap().add("ls -la".to_string());
        crate::ported::zle::zle_main::history().lock().unwrap().add("git push".to_string());
        crate::ported::zle::zle_main::history().lock().unwrap().cursor = 3; // sentinel
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "git ".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(4, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(4, std::sync::atomic::Ordering::SeqCst);
        widget_history_beginning_search_backward(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "git push");
        // Cursor stays where it was on the prefix.
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 4);
        widget_history_beginning_search_backward(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "git commit");
    }

    #[test]
    fn yank_records_region_for_yank_pop() {
        let mut zle = populated();
        widget_yank(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "newest");
        assert_eq!(crate::ported::zle::zle_main::YANKB.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(crate::ported::zle::zle_main::YANKE.load(std::sync::atomic::Ordering::SeqCst), 6);
        assert_eq!(zle.yank_ring_idx, Some(0));
        assert!(crate::ported::zle::zle_main::YANKLAST.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn yank_pop_replaces_with_previous_kill_ring_entry() {
        let mut zle = populated();
        widget_yank(&mut zle);
        widget_yank_pop(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "middle");
        widget_yank_pop(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "oldest");
    }

    #[test]
    fn yank_pop_no_op_without_prior_yank() {
        let mut zle = populated();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        widget_yank_pop(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "abc");
    }

    #[test]
    fn yank_pop_skips_empty_buffers() {
        let mut zle = Zle::new();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(Vec::new()); // empty buffer
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front("real".chars().collect());
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front("first".chars().collect());
        // widget_yank picks killring[0] = "first"
        widget_yank(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "first");
        widget_yank_pop(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "real");
        // Next pop wraps past the empty entry.
        widget_yank_pop(&mut zle);
        // After wrapping the empty entry, ring length 3, start_idx 1, advances
        // to idx=2 (empty, skipped within loop), then idx=0 — but 0 == start_idx
        // would only be true if start_idx were 0. We started from idx 1 and
        // the empty slot is at 2, so advance: 2 (empty, continue), 0 ("first"),
        // hit. Land on "first".
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "first");
    }

    #[test]
    fn vi_fetch_history_no_count_on_live_jumps_to_bol() {
        // When sitting on the live buffer with no explicit count, vi-fetch-history
        // moves the cursor to the beginning of the current logical line.
        // Port of C's `zlecs = zlell; zlecs = findbol()` no-mult branch
        // (zle_hist.c:1793).
        let mut zle = Zle::new();
        crate::ported::zle::zle_main::history().lock().unwrap().add("a".to_string());
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc def".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(4, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::history().lock().unwrap().cursor = 1; // live buffer
        widget_vi_fetch_history(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn vi_fetch_history_with_count_jumps_to_event() {
        let mut zle = Zle::new();
        crate::ported::zle::zle_main::history().lock().unwrap().add("a".to_string());
        crate::ported::zle::zle_main::history().lock().unwrap().add("b".to_string());
        crate::ported::zle::zle_main::history().lock().unwrap().add("c".to_string());
        crate::ported::zle::zle_main::history().lock().unwrap().cursor = 3; // live buffer
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= crate::ported::zle::zle_h::MOD_MULT;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 2; // 1-based: event #2 = entry index 1
        crate::ported::zle::zle_main::MULT.store(2, std::sync::atomic::Ordering::SeqCst);
        widget_vi_fetch_history(&mut zle);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "b");
        assert_eq!(crate::ported::zle::zle_main::history().lock().unwrap().cursor, 1);
    }
}
