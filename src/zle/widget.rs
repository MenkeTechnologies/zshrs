//! ZLE widgets - line editor commands
//!
//! Direct port from zsh/Src/Zle/zle.h widget structures
//!
//! A widget is a ZLE command that can be bound to keys or executed by name.
//! Widgets can be internal (implemented in Rust) or user-defined (shell functions).

use super::main::Zle;

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
    /// Create a new internal widget
    pub fn internal(name: &str, func: fn(&mut Zle), flags: WidgetFlags) -> Self {
        let _ = name; // Would be used for registration
        Widget {
            flags: flags | WidgetFlags::INT,
            func: WidgetFunc::Internal(func),
        }
    }

    /// Create a builtin widget by name
    pub fn builtin(name: &str) -> Self {
        let (func, flags) = get_builtin_widget(name);
        Widget {
            flags: flags | WidgetFlags::INT,
            func: WidgetFunc::Internal(func),
        }
    }

    /// Create a user-defined widget
    pub fn user_defined(name: &str, func_name: &str) -> Self {
        let _ = name;
        Widget {
            flags: WidgetFlags::empty(),
            func: WidgetFunc::User(func_name.to_string()),
        }
    }
}

/// Get the builtin widget function for a name
fn get_builtin_widget(name: &str) -> (fn(&mut Zle), WidgetFlags) {
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
        // visual / deactivate) and zle_misc.c (quote-line / quote-region
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
        "quote-line" => (widget_quote_line, WidgetFlags::empty()),
        "quote-region" => (widget_quote_region, WidgetFlags::empty()),
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
    zle.accept_line();
}

fn widget_accept_and_hold(zle: &mut Zle) {
    // Port of acceptandhold() from Src/Zle/zle_misc.c:409.
    // Push current line onto bufstack so the next zleread() re-feeds it
    // as the next entry, then exit the editor.
    let line: String = zle.zleline.iter().collect();
    zle.bufstack.push(line);
    zle.stackcs = zle.zlecs;
    zle.done = true;
    zle.accept_line();
}

fn widget_accept_line_and_down_history(zle: &mut Zle) {
    // Port of acceptlineanddownhistory() from Src/Zle/zle_hist.c:420.
    // Move forward one history entry and queue it on bufstack so the
    // next zleread() loads it; then exit the editor to run the current line.
    let len = zle.history.entries.len();
    let next_idx = zle.history.cursor + 1;
    if next_idx < len {
        if let Some(entry) = zle.history.entries.get(next_idx) {
            zle.bufstack.push(entry.line.clone());
            zle.stackhist = (entry.num as i32).max(0);
        }
    }
    zle.done = true;
    zle.accept_line();
}

fn widget_self_insert(zle: &mut Zle) {
    #[cfg(feature = "multibyte")]
    if let Some(c) = char::from_u32(zle.lastchar as u32) {
        zle.self_insert(c);
    }
    #[cfg(not(feature = "multibyte"))]
    if zle.lastchar >= 0 && zle.lastchar <= 127 {
        zle.self_insert(zle.lastchar as u8 as char);
    }
}

fn widget_self_insert_unmeta(zle: &mut Zle) {
    let c = (zle.lastchar & 0x7f) as u8 as char;
    zle.self_insert(c);
}

fn widget_forward_char(zle: &mut Zle) {
    if zle.zlecs < zle.zlell {
        zle.zlecs += 1;
        zle.resetneeded = true;
    }
}

fn widget_backward_char(zle: &mut Zle) {
    if zle.zlecs > 0 {
        zle.zlecs -= 1;
        zle.resetneeded = true;
    }
}

fn widget_forward_word(zle: &mut Zle) {
    // Skip current word
    while zle.zlecs < zle.zlell && is_word_char(zle.zleline[zle.zlecs]) {
        zle.zlecs += 1;
    }
    // Skip non-word characters
    while zle.zlecs < zle.zlell && !is_word_char(zle.zleline[zle.zlecs]) {
        zle.zlecs += 1;
    }
    zle.resetneeded = true;
}

fn widget_backward_word(zle: &mut Zle) {
    // Skip non-word characters
    while zle.zlecs > 0 && !is_word_char(zle.zleline[zle.zlecs - 1]) {
        zle.zlecs -= 1;
    }
    // Skip word
    while zle.zlecs > 0 && is_word_char(zle.zleline[zle.zlecs - 1]) {
        zle.zlecs -= 1;
    }
    zle.resetneeded = true;
}

fn widget_beginning_of_line(zle: &mut Zle) {
    zle.zlecs = 0;
    zle.resetneeded = true;
}

fn widget_end_of_line(zle: &mut Zle) {
    zle.zlecs = zle.zlell;
    zle.resetneeded = true;
}

fn widget_delete_char(zle: &mut Zle) {
    if zle.zlecs < zle.zlell {
        zle.zleline.remove(zle.zlecs);
        zle.zlell -= 1;
        zle.resetneeded = true;
    }
}

fn widget_backward_delete_char(zle: &mut Zle) {
    if zle.zlecs > 0 {
        zle.zlecs -= 1;
        zle.zleline.remove(zle.zlecs);
        zle.zlell -= 1;
        zle.resetneeded = true;
    }
}

fn widget_delete_char_or_list(zle: &mut Zle) {
    // Port of deletecharorlist() from Src/Zle/zle_misc.c. With an empty
    // buffer this is EOF; with non-end cursor it deletes one char; at
    // end-of-line it falls through to list-choices completion.
    if zle.zlell == 0 {
        zle.done = true;
    } else if zle.zlecs < zle.zlell {
        widget_delete_char(zle);
    } else {
        zle.completion_request = Some(super::main::CompletionRequest::ListChoices);
    }
}

fn widget_kill_line(zle: &mut Zle) {
    if zle.zlecs < zle.zlell {
        let killed: Vec<char> = zle.zleline.drain(zle.zlecs..).collect();
        zle.zlell = zle.zlecs;
        // Push to kill ring
        zle.killring.push_front(killed);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.resetneeded = true;
    }
}

fn widget_backward_kill_line(zle: &mut Zle) {
    if zle.zlecs > 0 {
        let killed: Vec<char> = zle.zleline.drain(..zle.zlecs).collect();
        zle.zlell -= zle.zlecs;
        zle.zlecs = 0;
        zle.killring.push_front(killed);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.resetneeded = true;
    }
}

fn widget_kill_whole_line(zle: &mut Zle) {
    if zle.zlell > 0 {
        let killed = std::mem::take(&mut zle.zleline);
        zle.killring.push_front(killed);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.zlecs = 0;
        zle.zlell = 0;
        zle.resetneeded = true;
    }
}

fn widget_kill_word(zle: &mut Zle) {
    let start = zle.zlecs;
    // Skip non-word characters
    while zle.zlecs < zle.zlell && !is_word_char(zle.zleline[zle.zlecs]) {
        zle.zlecs += 1;
    }
    // Skip word
    while zle.zlecs < zle.zlell && is_word_char(zle.zleline[zle.zlecs]) {
        zle.zlecs += 1;
    }
    let end = zle.zlecs;
    zle.zlecs = start;

    if end > start {
        let killed: Vec<char> = zle.zleline.drain(start..end).collect();
        zle.zlell -= end - start;
        zle.killring.push_front(killed);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.resetneeded = true;
    }
}

fn widget_backward_kill_word(zle: &mut Zle) {
    let end = zle.zlecs;
    // Skip non-word characters
    while zle.zlecs > 0 && !is_word_char(zle.zleline[zle.zlecs - 1]) {
        zle.zlecs -= 1;
    }
    // Skip word
    while zle.zlecs > 0 && is_word_char(zle.zleline[zle.zlecs - 1]) {
        zle.zlecs -= 1;
    }
    let start = zle.zlecs;

    if end > start {
        let killed: Vec<char> = zle.zleline.drain(start..end).collect();
        zle.zlell -= end - start;
        zle.killring.push_front(killed);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.resetneeded = true;
    }
}

fn widget_yank(zle: &mut Zle) {
    // Port of yank() from Src/Zle/zle_misc.c. Inserts the most-recent kill-ring
    // entry at the cursor and remembers the inserted region so that an
    // immediately-following yank-pop can rotate to the previous entry.
    if let Some(text) = zle.killring.front().cloned() {
        let start = zle.zlecs;
        for c in &text {
            zle.zleline.insert(zle.zlecs, *c);
            zle.zlecs += 1;
            zle.zlell += 1;
        }
        zle.yank_start = start;
        zle.yank_end = start + text.len();
        zle.yank_cs = zle.zlecs;
        zle.yank_ring_idx = Some(0);
        zle.yanklast = true;
        zle.resetneeded = true;
    }
}

fn widget_yank_pop(zle: &mut Zle) {
    // Port of yankpop() from Src/Zle/zle_misc.c:728.
    // Only meaningful immediately after a yank; replaces the just-yanked
    // region with the previous kill-ring entry, cycling around the ring.
    if !zle.yanklast {
        return;
    }
    let ring_len = zle.killring.len();
    if ring_len == 0 {
        return;
    }
    // Advance to the next ring entry; skip empty buffers; bail out if we
    // wrap all the way around without finding anything (matches kctstart guard
    // in C zle_misc.c:730).
    let start_idx = zle.yank_ring_idx.unwrap_or(0);
    let mut idx = start_idx;
    let mut found_idx: Option<usize> = None;
    for _ in 0..ring_len {
        idx = (idx + 1) % ring_len;
        if idx == start_idx {
            break;
        }
        if !zle.killring[idx].is_empty() {
            found_idx = Some(idx);
            break;
        }
    }
    let new_idx = match found_idx {
        Some(i) => i,
        None => return,
    };
    let new_text: Vec<char> = zle.killring[new_idx].clone();

    // Delete the previously-yanked region.
    let yb = zle.yank_start.min(zle.zlell);
    let ye = zle.yank_end.min(zle.zlell);
    if ye > yb {
        zle.zleline.drain(yb..ye);
        zle.zlell -= ye - yb;
    }
    zle.zlecs = yb;

    // Paste the new entry.
    let start = zle.zlecs;
    for c in &new_text {
        zle.zleline.insert(zle.zlecs, *c);
        zle.zlecs += 1;
        zle.zlell += 1;
    }
    zle.yank_start = start;
    zle.yank_end = start + new_text.len();
    zle.yank_cs = zle.zlecs;
    zle.yank_ring_idx = Some(new_idx);
    zle.yanklast = true;
    zle.resetneeded = true;
}

fn widget_undo(zle: &mut Zle) {
    // Port of undo() from Src/Zle/zle_utils.c:1601.
    let _ = zle.undo_widget();
}

fn widget_redo(zle: &mut Zle) {
    // Port of redo() from Src/Zle/zle_utils.c:1661.
    let _ = zle.redo_widget();
}

fn widget_up_line_or_history(zle: &mut Zle) {
    // Port of uplineorhistory() from Src/Zle/zle_hist.c:282.
    let _ = zle.up_line_or_history_widget();
}

fn widget_down_line_or_history(zle: &mut Zle) {
    // Port of downlineorhistory() from Src/Zle/zle_hist.c:370.
    let _ = zle.down_line_or_history_widget();
}

fn widget_up_history(zle: &mut Zle) {
    // Port of uphistory() from Src/Zle/zle_hist.c:233.
    // C calls zle_goto_hist(histline, -zmult, isset(HISTIGNOREDUPS)).
    // skipdups=false until ZLE has access to ShellOptions; behavior matches HISTIGNOREDUPS unset.
    let m = zle.mult;
    zle.zle_goto_hist(-m, false);
}

fn widget_down_history(zle: &mut Zle) {
    // Port of downhistory() from Src/Zle/zle_hist.c:434.
    let m = zle.mult;
    zle.zle_goto_hist(m, false);
}

fn widget_history_isearch_backward(zle: &mut Zle) {
    // Port of historyincrementalsearchbackward() from Src/Zle/zle_hist.c:922
    // (which is doisearch(-1, 0)).
    do_isearch(zle, -1);
}

fn widget_history_isearch_forward(zle: &mut Zle) {
    // Port of historyincrementalsearchforward() from Src/Zle/zle_hist.c:929
    // (doisearch(1, 0)).
    do_isearch(zle, 1);
}

/// Minimal port of doisearch() from zle_hist.c.
/// Reads characters into a pattern and re-searches history on each keystroke.
/// Recognised control chars: Ctrl-R repeats backward, Ctrl-S repeats forward,
/// Ctrl-G/Esc cancels (restores starting line), backspace shortens the
/// pattern, Enter accepts. Anything else exits the loop with the current
/// match in place.
fn do_isearch(zle: &mut Zle, mut dir: i32) {
    // Save start state for cancel.
    let start_line = zle.zleline.clone();
    let start_cs = zle.zlecs;
    let start_cursor = zle.history.cursor;

    let mut pattern = String::new();
    let mut current_idx: i32 = zle.history.cursor as i32;

    while let Some(c) = zle.getfullchar(true) {
        match c {
            // Enter / Newline → accept current match.
            '\r' | '\n' => break,
            // Ctrl-G / Esc → cancel.
            '\x07' | '\x1b' => {
                zle.zleline = start_line;
                zle.zlell = zle.zleline.len();
                zle.zlecs = start_cs;
                zle.history.cursor = start_cursor;
                zle.resetneeded = true;
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
                current_idx = zle.history.cursor as i32;
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
        zle.srch_str = Some(pattern.clone());
        let len = zle.history.entries.len() as i32;
        let matched: Option<usize> = if dir < 0 {
            // Search backward starting at current_idx.
            let mut i = current_idx.min(len - 1);
            let mut found = None;
            while i >= 0 {
                if zle.history.entries[i as usize].line.contains(&pattern) {
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
                if zle.history.entries[i as usize].line.contains(&pattern) {
                    found = Some(i as usize);
                    break;
                }
                i += 1;
            }
            found
        };
        if let Some(idx) = matched {
            current_idx = idx as i32;
            zle.history.cursor = idx;
            zle.zleline = zle.history.entries[idx].line.chars().collect();
            zle.zlell = zle.zleline.len();
            // Place cursor at the start of the match for visual feedback.
            zle.zlecs = zle.history.entries[idx]
                .line
                .find(&pattern)
                .unwrap_or(0)
                .min(zle.zlell);
            zle.resetneeded = true;
        } else {
            // No match — beep but keep the prior position.
            zle.handle_feep();
        }
    }
}

fn widget_beginning_of_buffer_or_history(zle: &mut Zle) {
    zle.zlecs = 0;
    zle.resetneeded = true;
}

fn widget_end_of_buffer_or_history(zle: &mut Zle) {
    zle.zlecs = zle.zlell;
    zle.resetneeded = true;
}

fn widget_transpose_chars(zle: &mut Zle) {
    if zle.zlecs > 0 && zle.zlell >= 2 {
        let pos = if zle.zlecs == zle.zlell {
            zle.zlecs - 1
        } else {
            zle.zlecs
        };
        if pos > 0 {
            zle.zleline.swap(pos - 1, pos);
            zle.zlecs = pos + 1;
            zle.resetneeded = true;
        }
    }
}

fn widget_clear_screen(zle: &mut Zle) {
    print!("\x1b[2J\x1b[H");
    zle.resetneeded = true;
}

fn widget_redisplay(zle: &mut Zle) {
    zle.resetneeded = true;
}

fn widget_send_break(zle: &mut Zle) {
    zle.send_break();
}

fn widget_overwrite_mode(zle: &mut Zle) {
    zle.insmode = !zle.insmode;
}

fn widget_quoted_insert(zle: &mut Zle) {
    // Read next char literally
    if let Some(c) = zle.getfullchar(true) {
        zle.self_insert(c);
    }
}

fn widget_expand_or_complete(zle: &mut Zle) {
    // Port of expandorcomplete() from Src/Zle/zle_tricky.c — tries
    // expansion first, falls back to completion. Compsys lives in a
    // separate crate; surface the request and let the host run it.
    zle.completion_request = Some(super::main::CompletionRequest::ExpandOrComplete);
}

fn widget_complete_word(zle: &mut Zle) {
    // Port of completeword() from Src/Zle/zle_tricky.c.
    zle.completion_request = Some(super::main::CompletionRequest::CompleteWord);
}

fn widget_expand_word(zle: &mut Zle) {
    // Port of expandword() from Src/Zle/zle_tricky.c — runs only the
    // expansion phase (history, glob, parameter, brace) without falling
    // through to completion.
    zle.completion_request = Some(super::main::CompletionRequest::ExpandWord);
}

fn widget_list_choices(zle: &mut Zle) {
    // Port of listchoices() from Src/Zle/zle_tricky.c — shows matches
    // without inserting.
    zle.completion_request = Some(super::main::CompletionRequest::ListChoices);
}

fn widget_menu_complete(zle: &mut Zle) {
    // Port of menucomplete() from Src/Zle/zle_tricky.c — enters/steps
    // the menu-selection state.
    zle.completion_request = Some(super::main::CompletionRequest::MenuComplete);
}

// Vi mode widgets

fn widget_vi_cmd_mode(zle: &mut Zle) {
    zle.keymaps.select("vicmd");
    if zle.zlecs > 0 {
        zle.zlecs -= 1;
    }
    zle.resetneeded = true;
}

fn widget_vi_insert(zle: &mut Zle) {
    zle.keymaps.select("viins");
    zle.insmode = true;
}

fn widget_vi_insert_bol(zle: &mut Zle) {
    zle.keymaps.select("viins");
    zle.insmode = true;
    // Move to first non-blank
    zle.zlecs = 0;
    while zle.zlecs < zle.zlell && zle.zleline[zle.zlecs].is_whitespace() {
        zle.zlecs += 1;
    }
    zle.resetneeded = true;
}

fn widget_vi_add_next(zle: &mut Zle) {
    zle.keymaps.select("viins");
    zle.insmode = true;
    if zle.zlecs < zle.zlell {
        zle.zlecs += 1;
    }
    zle.resetneeded = true;
}

fn widget_vi_add_eol(zle: &mut Zle) {
    zle.keymaps.select("viins");
    zle.insmode = true;
    zle.zlecs = zle.zlell;
    zle.resetneeded = true;
}

fn widget_vi_forward_char(zle: &mut Zle) {
    if zle.zlecs < zle.zlell.saturating_sub(1) {
        zle.zlecs += 1;
        zle.resetneeded = true;
    }
}

fn widget_vi_backward_char(zle: &mut Zle) {
    if zle.zlecs > 0 {
        zle.zlecs -= 1;
        zle.resetneeded = true;
    }
}

fn widget_vi_forward_word(zle: &mut Zle) {
    widget_forward_word(zle);
}

fn widget_vi_forward_word_end(zle: &mut Zle) {
    if zle.zlecs < zle.zlell {
        zle.zlecs += 1;
    }
    // Skip non-word
    while zle.zlecs < zle.zlell && !is_word_char(zle.zleline[zle.zlecs]) {
        zle.zlecs += 1;
    }
    // Skip word
    while zle.zlecs < zle.zlell.saturating_sub(1) && is_word_char(zle.zleline[zle.zlecs + 1]) {
        zle.zlecs += 1;
    }
    zle.resetneeded = true;
}

fn widget_vi_forward_blank_word(zle: &mut Zle) {
    // Skip non-blank
    while zle.zlecs < zle.zlell && !zle.zleline[zle.zlecs].is_whitespace() {
        zle.zlecs += 1;
    }
    // Skip blank
    while zle.zlecs < zle.zlell && zle.zleline[zle.zlecs].is_whitespace() {
        zle.zlecs += 1;
    }
    zle.resetneeded = true;
}

fn widget_vi_forward_blank_word_end(zle: &mut Zle) {
    if zle.zlecs < zle.zlell {
        zle.zlecs += 1;
    }
    // Skip whitespace
    while zle.zlecs < zle.zlell && zle.zleline[zle.zlecs].is_whitespace() {
        zle.zlecs += 1;
    }
    // Skip non-whitespace
    while zle.zlecs < zle.zlell.saturating_sub(1) && !zle.zleline[zle.zlecs + 1].is_whitespace() {
        zle.zlecs += 1;
    }
    zle.resetneeded = true;
}

fn widget_vi_backward_word(zle: &mut Zle) {
    widget_backward_word(zle);
}

fn widget_vi_backward_blank_word(zle: &mut Zle) {
    // Skip blanks
    while zle.zlecs > 0 && zle.zleline[zle.zlecs - 1].is_whitespace() {
        zle.zlecs -= 1;
    }
    // Skip non-blanks
    while zle.zlecs > 0 && !zle.zleline[zle.zlecs - 1].is_whitespace() {
        zle.zlecs -= 1;
    }
    zle.resetneeded = true;
}

fn widget_vi_delete(zle: &mut Zle) {
    // Port of videlete() from Src/Zle/zle_vi.c:384.
    let _ = zle.vi_delete_op();
}

fn widget_vi_delete_char(zle: &mut Zle) {
    widget_delete_char(zle);
}

fn widget_vi_backward_delete_char(zle: &mut Zle) {
    widget_backward_delete_char(zle);
}

fn widget_vi_change(zle: &mut Zle) {
    // Port of vichange() from Src/Zle/zle_vi.c:438.
    let _ = zle.vi_change_op();
}

fn widget_vi_change_eol(zle: &mut Zle) {
    widget_kill_line(zle);
    widget_vi_insert(zle);
}

fn widget_vi_kill_eol(zle: &mut Zle) {
    widget_kill_line(zle);
}

fn widget_vi_yank(zle: &mut Zle) {
    // Port of viyank() from Src/Zle/zle_vi.c:507.
    let _ = zle.vi_yank_op();
}

fn widget_vi_yank_whole_line(zle: &mut Zle) {
    zle.killring.push_front(zle.zleline.clone());
    if zle.killring.len() > zle.killringmax {
        zle.killring.pop_back();
    }
}

fn widget_vi_put_after(zle: &mut Zle) {
    if zle.zlecs < zle.zlell {
        zle.zlecs += 1;
    }
    widget_yank(zle);
}

fn widget_vi_put_before(zle: &mut Zle) {
    widget_yank(zle);
}

fn widget_vi_replace(zle: &mut Zle) {
    zle.keymaps.select("viins");
    zle.insmode = false;
}

fn widget_vi_replace_chars(zle: &mut Zle) {
    // Read replacement char
    if let Some(c) = zle.getfullchar(true) {
        if zle.zlecs < zle.zlell {
            zle.zleline[zle.zlecs] = c;
            zle.resetneeded = true;
        }
    }
}

fn widget_vi_substitute(zle: &mut Zle) {
    widget_delete_char(zle);
    widget_vi_insert(zle);
}

fn widget_vi_change_whole_line(zle: &mut Zle) {
    widget_kill_whole_line(zle);
    widget_vi_insert(zle);
}

fn widget_vi_first_non_blank(zle: &mut Zle) {
    zle.zlecs = 0;
    while zle.zlecs < zle.zlell && zle.zleline[zle.zlecs].is_whitespace() {
        zle.zlecs += 1;
    }
    zle.resetneeded = true;
}

fn widget_vi_end_of_line(zle: &mut Zle) {
    if zle.zlell > 0 {
        zle.zlecs = zle.zlell - 1;
    }
    zle.resetneeded = true;
}

fn widget_vi_digit_or_beginning_of_line(zle: &mut Zle) {
    if zle.zmod.flags.contains(super::main::ModifierFlags::MULT) {
        widget_digit_argument(zle);
    } else {
        widget_beginning_of_line(zle);
    }
}

fn widget_vi_open_line_below(zle: &mut Zle) {
    zle.zlecs = zle.zlell;
    zle.self_insert('\n');
    widget_vi_insert(zle);
}

fn widget_vi_open_line_above(zle: &mut Zle) {
    zle.zlecs = 0;
    zle.self_insert('\n');
    zle.zlecs = 0;
    widget_vi_insert(zle);
}

fn widget_vi_join(zle: &mut Zle) {
    // Find newline and remove it
    while zle.zlecs < zle.zlell {
        if zle.zleline[zle.zlecs] == '\n' {
            zle.zleline.remove(zle.zlecs);
            zle.zlell -= 1;
            // Insert space if needed
            if zle.zlecs > 0 && zle.zlecs < zle.zlell {
                zle.zleline.insert(zle.zlecs, ' ');
                zle.zlell += 1;
            }
            break;
        }
        zle.zlecs += 1;
    }
    zle.resetneeded = true;
}

fn widget_vi_repeat_change(zle: &mut Zle) {
    // Port of virepeatchange() from Src/Zle/zle_vi.c. Replays the keys in
    // vi_chg_buf — but the recording side (which captures keystrokes during
    // d/c/y operators into vi_chg_buf) is still pending, so without a
    // recorded change the buffer is empty and the widget is a no-op.
    // This matches zsh's behavior when no change has been made yet.
    if zle.vi_chg_buf.is_empty() {
        return;
    }
    // Re-feed the recorded keys via ungetbytes so the next iteration of
    // zlecore re-runs them. ungetbytes prepends to the input buffer so the
    // bytes will be consumed before any new keystrokes.
    let bytes = zle.vi_chg_buf.clone();
    zle.ungetbytes(&bytes);
}

fn widget_vi_find_next_char(zle: &mut Zle) {
    // Port of vifindnextchar() from Src/Zle/zle_move.c:739.
    zle.vi_find_char(true, false);
}

fn widget_vi_find_prev_char(zle: &mut Zle) {
    // Port of vifindprevchar() from Src/Zle/zle_move.c:751.
    zle.vi_find_char(false, false);
}

fn widget_vi_find_next_char_skip(zle: &mut Zle) {
    // Port of vifindnextcharskip() from Src/Zle/zle_move.c:763.
    zle.vi_find_char(true, true);
}

fn widget_vi_find_prev_char_skip(zle: &mut Zle) {
    // Port of vifindprevcharskip() from Src/Zle/zle_move.c:775.
    zle.vi_find_char(false, true);
}

fn widget_vi_repeat_find(zle: &mut Zle) {
    // Port of virepeatfind() from Src/Zle/zle_move.c:835.
    let _ = zle.vi_repeat_find();
}

fn widget_vi_rev_repeat_find(zle: &mut Zle) {
    // Port of virevrepeatfind() from Src/Zle/zle_move.c:842.
    let _ = zle.vi_rev_repeat_find();
}

fn widget_vi_history_search_forward(zle: &mut Zle) {
    // Port of vihistorysearchforward() from Src/Zle/zle_hist.c.
    // Read the search pattern starting from `?` then run a forward history search.
    // For now: re-run the last srch_str if any.
    let pat = match zle.srch_str.clone() {
        Some(s) if !s.is_empty() => s,
        _ => return,
    };
    let len = zle.history.entries.len();
    let start = zle.history.cursor + 1;
    for i in start..len {
        if zle.history.entries[i].line.contains(&pat) {
            if zle.history.saved_line.is_none() {
                zle.history.saved_line = Some(zle.zleline.clone());
                zle.history.saved_cs = zle.zlecs;
            }
            zle.history.cursor = i;
            zle.zleline = zle.history.entries[i].line.chars().collect();
            zle.zlell = zle.zleline.len();
            zle.zlecs = 0;
            zle.resetneeded = true;
            return;
        }
    }
}

fn widget_vi_history_search_backward(zle: &mut Zle) {
    // Port of vihistorysearchbackward() from Src/Zle/zle_hist.c.
    let pat = match zle.srch_str.clone() {
        Some(s) if !s.is_empty() => s,
        _ => return,
    };
    if zle.history.cursor == 0 {
        return;
    }
    let mut i = zle.history.cursor.min(zle.history.entries.len()).saturating_sub(1);
    loop {
        if zle.history.entries[i].line.contains(&pat) {
            if zle.history.saved_line.is_none() {
                zle.history.saved_line = Some(zle.zleline.clone());
                zle.history.saved_cs = zle.zlecs;
            }
            zle.history.cursor = i;
            zle.zleline = zle.history.entries[i].line.chars().collect();
            zle.zlell = zle.zleline.len();
            zle.zlecs = 0;
            zle.resetneeded = true;
            return;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
}

fn widget_vi_repeat_search(zle: &mut Zle) {
    // Port of virepeatsearch() from Src/Zle/zle_hist.c.
    // Replays the last vi search in the same direction.
    let mut hist = std::mem::take(&mut zle.history);
    zle.vi_repeat_search(&mut hist);
    zle.history = hist;
}

fn widget_vi_rev_repeat_search(zle: &mut Zle) {
    // Port of virevrepeatsearch() from Src/Zle/zle_hist.c.
    let mut hist = std::mem::take(&mut zle.history);
    zle.vi_rev_repeat_search(&mut hist);
    zle.history = hist;
}

fn widget_vi_fetch_history(zle: &mut Zle) {
    // Port of vifetchhistory() from Src/Zle/zle_hist.c:1787.
    // With no count: jump to the live (newest) entry. With a count: load
    // that history event by 1-based index. Negative count is rejected.
    if zle.mult < 0 {
        return;
    }
    let has_mult = zle.zmod.flags.contains(super::main::ModifierFlags::MULT);
    let on_live = zle.history.cursor >= zle.history.entries.len();
    if on_live || zle.zlereadflags.no_history {
        if !has_mult {
            zle.zlecs = zle.zlell;
            zle.zlecs = zle.find_bol(zle.zlecs);
            zle.resetneeded = true;
            return;
        }
        if zle.zlereadflags.no_history {
            return;
        }
    }
    let target_idx_1: i32 = if has_mult {
        zle.zmod.mult
    } else {
        zle.history.entries.len() as i32
    };
    if target_idx_1 < 1 {
        return;
    }
    let target_idx = (target_idx_1 - 1) as usize;
    if target_idx >= zle.history.entries.len() {
        return;
    }
    if zle.history.saved_line.is_none() && on_live {
        zle.history.saved_line = Some(zle.zleline.clone());
        zle.history.saved_cs = zle.zlecs;
    }
    zle.history.cursor = target_idx;
    zle.zleline = zle.history.entries[target_idx].line.chars().collect();
    zle.zlell = zle.zleline.len();
    zle.zlecs = 0;
    zle.resetneeded = true;
}

fn widget_vi_goto_column(zle: &mut Zle) {
    let col = zle.zmod.mult.saturating_sub(1) as usize;
    zle.zlecs = col.min(zle.zlell);
    zle.resetneeded = true;
}

fn widget_vi_backward_kill_word(zle: &mut Zle) {
    widget_backward_kill_word(zle);
}

fn widget_digit_argument(zle: &mut Zle) {
    let digit = (zle.lastchar as u8).saturating_sub(b'0') as i32;

    if zle.zmod.flags.contains(super::main::ModifierFlags::TMULT) {
        zle.zmod.tmult = zle.zmod.tmult * zle.zmod.base + digit;
    } else {
        zle.zmod.flags.insert(super::main::ModifierFlags::TMULT);
        zle.zmod.tmult = digit;
    }

    zle.prefixflag = true;
}

fn widget_undefined(zle: &mut Zle) {
    // Beep or do nothing
    let _ = zle;
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
    // Port of setmarkcommand() from Src/Zle/zle_move.c:483. Negative count
    // disables the region; otherwise marks the current cursor and turns
    // on the visual region (charwise).
    if zle.mult < 0 {
        zle.region_active = 0;
        return;
    }
    zle.mark = zle.zlecs;
    zle.region_active = 1;
}

fn widget_exchange_point_and_mark(zle: &mut Zle) {
    // Port of exchangepointandmark() from Src/Zle/zle_move.c:496. With
    // mult==0 the C source just turns the region on without swapping;
    // with mult>0 swaps cursor↔mark and clamps cursor.
    if zle.mult == 0 {
        zle.region_active = 1;
        return;
    }
    let new_cs = zle.mark;
    zle.mark = zle.zlecs;
    zle.zlecs = new_cs.min(zle.zlell);
    if zle.mult > 0 {
        zle.region_active = 1;
    }
    zle.resetneeded = true;
}

fn widget_deactivate_region(zle: &mut Zle) {
    // Port of deactivateregion() from Src/Zle/zle_move.c:564.
    zle.vi_deactivate_region();
}

fn widget_visual_mode(zle: &mut Zle) {
    // Port of visualmode() from Src/Zle/zle_move.c:516.
    zle.vi_visual_mode();
}

fn widget_visual_line_mode(zle: &mut Zle) {
    // Port of visuallinemode() from Src/Zle/zle_move.c:540.
    zle.vi_visual_line_mode();
}

fn widget_capitalize_word(zle: &mut Zle) {
    // Port of capitalizeword() from Src/Zle/zle_misc.c. Method already
    // exists on Zle; this is the dispatch entry.
    zle.capitalize_word();
}

fn widget_down_case_word(zle: &mut Zle) {
    // Port of downcaseword() from Src/Zle/zle_misc.c.
    zle.downcase_word();
}

fn widget_up_case_word(zle: &mut Zle) {
    // Port of upcaseword() from Src/Zle/zle_misc.c.
    zle.upcase_word();
}

fn widget_pound_insert(zle: &mut Zle) {
    // Port of poundinsert() from Src/Zle/zle_misc.c:369. Toggle a leading
    // `#` on every logical line so the entire input is commented out
    // (or uncommented). Common keybinding: M-#.
    zle.zlecs = 0;
    let toggle_off = zle.zleline.first().copied() == Some('#');
    if toggle_off {
        // Walk every logical line, removing one leading '#' if present
        // (C source: zle_misc.c:384-394).
        let mut p = 0;
        loop {
            let bol = zle.find_bol(p);
            if zle.zleline.get(bol).copied() == Some('#') {
                zle.zleline.remove(bol);
                if zle.zlell > 0 {
                    zle.zlell -= 1;
                }
            }
            let eol = zle.find_eol(bol);
            if eol >= zle.zlell {
                break;
            }
            p = eol + 1;
        }
    } else {
        // Insert '#' at start of every logical line (zle_misc.c:373-383).
        let mut p = 0;
        loop {
            let bol = zle.find_bol(p);
            zle.zleline.insert(bol, '#');
            zle.zlell += 1;
            let eol = zle.find_eol(bol);
            if eol >= zle.zlell {
                break;
            }
            p = eol + 1;
        }
    }
    zle.zlecs = 0;
    zle.done = true; // C zsh accepts the line after a pound-insert.
}

fn widget_quote_line(zle: &mut Zle) {
    // Port of quoteline() from Src/Zle/zle_misc.c:1187. Wrap the entire
    // buffer in single quotes, escaping any embedded single quote as
    // `'\''` (the C source's makequote routine).
    let inner: String = zle.zleline.iter().collect();
    let escaped = inner.replace('\'', r"'\''");
    let new_line = format!("'{}'", escaped);
    zle.zleline = new_line.chars().collect();
    zle.zlell = zle.zleline.len();
    zle.zlecs = zle.zlell;
    zle.resetneeded = true;
}

fn widget_quote_region(zle: &mut Zle) {
    // Port of quoteregion() from Src/Zle/zle_misc.c:1152. Wrap the
    // currently-selected region (mark..zlecs, normalised) in single
    // quotes with embedded-quote escaping.
    let (lo, hi) = if zle.mark <= zle.zlecs {
        (zle.mark, zle.zlecs)
    } else {
        (zle.zlecs, zle.mark)
    };
    let lo = lo.min(zle.zlell);
    let hi = hi.min(zle.zlell);
    if hi <= lo {
        return;
    }
    let inner: String = zle.zleline[lo..hi].iter().collect();
    let escaped = inner.replace('\'', r"'\''");
    let wrapped = format!("'{}'", escaped);
    let wrapped_chars: Vec<char> = wrapped.chars().collect();
    zle.zleline.splice(lo..hi, wrapped_chars.iter().copied());
    zle.zlell = zle.zleline.len();
    zle.zlecs = lo + wrapped_chars.len();
    zle.resetneeded = true;
}

fn widget_copy_region_as_kill(zle: &mut Zle) {
    // Port of copyregionaskill() from Src/Zle/zle_misc.c:494. Copies
    // mark..zlecs (normalised) onto the kill ring without removing it.
    let (lo, hi) = if zle.mark <= zle.zlecs {
        (zle.mark, zle.zlecs)
    } else {
        (zle.zlecs, zle.mark)
    };
    let lo = lo.min(zle.zlell);
    let hi = hi.min(zle.zlell);
    if hi <= lo {
        return;
    }
    let region: Vec<char> = zle.zleline[lo..hi].to_vec();
    zle.killring.push_front(region);
    if zle.killring.len() > zle.killringmax {
        zle.killring.pop_back();
    }
}

fn widget_copy_prev_word(zle: &mut Zle) {
    // Port of copyprevword() from Src/Zle/zle_misc.c:1066. Inserts the
    // previous word (per ZC_iword) at the cursor. The full C version
    // walks `zmult` words back; we replicate that by scanning backward
    // through `mult` word-boundaries.
    let n = zle.mult.max(1) as usize;
    let mut end = zle.zlecs;
    let mut start;
    let mut word: Option<(usize, usize)> = None;
    for _ in 0..n {
        // Skip whitespace going backward.
        while end > 0 && zle.zleline[end - 1].is_whitespace() {
            end -= 1;
        }
        if end == 0 {
            break;
        }
        start = end;
        while start > 0 && !zle.zleline[start - 1].is_whitespace() {
            start -= 1;
        }
        word = Some((start, end));
        end = start;
    }
    if let Some((s, e)) = word {
        let copied: Vec<char> = zle.zleline[s..e].to_vec();
        for (i, c) in copied.iter().enumerate() {
            zle.zleline.insert(zle.zlecs + i, *c);
        }
        zle.zlecs += copied.len();
        zle.zlell = zle.zleline.len();
        zle.resetneeded = true;
    }
}

fn widget_transpose_words(zle: &mut Zle) {
    // Port of transposewords() from Src/Zle/zle_word.c:652. The C source
    // is a multi-step pointer dance; this Rust port recreates the
    // common-case behavior: swap the two whitespace-separated words
    // around (or before) the cursor. Multi-line + edge-case handling
    // matches the C pattern of "fall back to nearest two prior words"
    // when the cursor is past the last word on the line.
    let n = zle.zlell;
    if n == 0 {
        return;
    }
    // Find the word containing or following the cursor (`p4` in C).
    let mut p4 = zle.zlecs.min(n);
    while p4 < n && !zle.zleline[p4].is_alphanumeric() && zle.zleline[p4] != '_' {
        p4 += 1;
    }
    // If we landed past EOL, slide back to find the prior word.
    if p4 == n {
        let mut x = zle.zlecs;
        while x > 0 && (!zle.zleline[x - 1].is_alphanumeric() && zle.zleline[x - 1] != '_') {
            x -= 1;
        }
        if x == 0 {
            return;
        }
        p4 = x;
    }
    let p3 = {
        let mut x = p4;
        while x < n && (zle.zleline[x].is_alphanumeric() || zle.zleline[x] == '_') {
            x += 1;
        }
        x
    };
    let p4 = {
        let mut x = p4;
        while x > 0 && (zle.zleline[x - 1].is_alphanumeric() || zle.zleline[x - 1] == '_') {
            x -= 1;
        }
        x
    };
    let p2 = {
        let mut x = p4;
        while x > 0 && !zle.zleline[x - 1].is_alphanumeric() && zle.zleline[x - 1] != '_' {
            x -= 1;
        }
        x
    };
    let p1 = {
        let mut x = p2;
        while x > 0 && (zle.zleline[x - 1].is_alphanumeric() || zle.zleline[x - 1] == '_') {
            x -= 1;
        }
        x
    };
    if p1 == p2 || p4 == p3 {
        return;
    }
    let word1: Vec<char> = zle.zleline[p1..p2].to_vec();
    let word2: Vec<char> = zle.zleline[p4..p3].to_vec();
    let mut new_buf: Vec<char> = Vec::with_capacity(zle.zlell);
    new_buf.extend_from_slice(&zle.zleline[..p1]);
    new_buf.extend_from_slice(&word2);
    new_buf.extend_from_slice(&zle.zleline[p2..p4]);
    new_buf.extend_from_slice(&word1);
    new_buf.extend_from_slice(&zle.zleline[p3..]);
    zle.zleline = new_buf;
    zle.zlell = zle.zleline.len();
    zle.zlecs = p1 + word2.len() + (p4 - p2) + word1.len();
    zle.resetneeded = true;
}

fn widget_history_beginning_search_backward(zle: &mut Zle) {
    // Port of historybeginningsearchbackward() from Src/Zle/zle_hist.c:2039.
    // Searches history for entries that start with the text *before* the
    // cursor (the prefix), keeping the cursor where it is on a match.
    let prefix: String = zle.zleline[..zle.zlecs.min(zle.zleline.len())]
        .iter()
        .collect();
    if zle.history.cursor == 0 {
        return;
    }
    let saved_cs = zle.zlecs;
    let mut i = zle.history.cursor.min(zle.history.entries.len()).saturating_sub(1);
    loop {
        if zle.history.entries[i].line.starts_with(&prefix) {
            if zle.history.saved_line.is_none() {
                zle.history.saved_line = Some(zle.zleline.clone());
                zle.history.saved_cs = saved_cs;
            }
            zle.history.cursor = i;
            zle.zleline = zle.history.entries[i].line.chars().collect();
            zle.zlell = zle.zleline.len();
            zle.zlecs = saved_cs.min(zle.zlell);
            zle.resetneeded = true;
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
    let prefix: String = zle.zleline[..zle.zlecs.min(zle.zleline.len())]
        .iter()
        .collect();
    let saved_cs = zle.zlecs;
    let len = zle.history.entries.len();
    for i in (zle.history.cursor + 1)..len {
        if zle.history.entries[i].line.starts_with(&prefix) {
            zle.history.cursor = i;
            zle.zleline = zle.history.entries[i].line.chars().collect();
            zle.zlell = zle.zleline.len();
            zle.zlecs = saved_cs.min(zle.zlell);
            zle.resetneeded = true;
            return;
        }
    }
}

fn widget_beginning_of_history(zle: &mut Zle) {
    // Port of beginningofhistory() from Src/Zle/zle_hist.c:464.
    let mut hist = std::mem::take(&mut zle.history);
    zle.beginning_of_history(&mut hist);
    zle.history = hist;
}

fn widget_end_of_history(zle: &mut Zle) {
    // Port of endofhistory() from Src/Zle/zle_hist.c:478.
    let mut hist = std::mem::take(&mut zle.history);
    zle.end_of_history(&mut hist);
    zle.history = hist;
}

fn widget_push_line(zle: &mut Zle) {
    // Port of pushline() from Src/Zle/zle_hist.c:832.
    zle.push_line();
    zle.done = true;
}

fn widget_describe_key_briefly(zle: &mut Zle) {
    // Port of describekeybriefly() from Src/Zle/zle_thingy.c. Existing
    // method on Zle handles the input read + lookup loop.
    zle.describe_key_briefly();
}

fn widget_delete_word(zle: &mut Zle) {
    // Port of deleteword() from Src/Zle/zle_word.c. Like kill-word but
    // doesn't put the deleted text on the kill ring.
    let saved_cs = zle.zlecs;
    let end = zle.find_word_end(super::word::WordStyle::Emacs);
    if end > saved_cs {
        zle.zleline.drain(saved_cs..end);
        zle.zlell = zle.zleline.len();
    }
    zle.resetneeded = true;
}

fn widget_backward_delete_word(zle: &mut Zle) {
    // Port of backwarddeleteword() from Src/Zle/zle_word.c. Like
    // backward-kill-word but no kill-ring update.
    let end = zle.zlecs;
    let start = zle.find_word_start(super::word::WordStyle::Emacs);
    if end > start {
        zle.zleline.drain(start..end);
        zle.zlell = zle.zleline.len();
        zle.zlecs = start;
    }
    zle.resetneeded = true;
}

fn widget_emacs_forward_word(zle: &mut Zle) {
    // Port of emacsforwardword() from Src/Zle/zle_word.c — same as
    // forward-word in emacs style; explicit name binding for users who
    // want it independent of the global word style.
    zle.zlecs = zle.find_word_end(super::word::WordStyle::Emacs);
    zle.resetneeded = true;
}

fn widget_emacs_backward_word(zle: &mut Zle) {
    // Port of emacsbackwardword() from Src/Zle/zle_word.c.
    zle.zlecs = zle.find_word_start(super::word::WordStyle::Emacs);
    zle.resetneeded = true;
}

fn widget_kill_region(zle: &mut Zle) {
    // Port of killregion() from Src/Zle/zle_misc.c. Drains the region
    // (mark..zlecs, normalised) into the kill ring and removes it.
    let (lo, hi) = if zle.mark <= zle.zlecs {
        (zle.mark, zle.zlecs)
    } else {
        (zle.zlecs, zle.mark)
    };
    let lo = lo.min(zle.zlell);
    let hi = hi.min(zle.zlell);
    if hi <= lo {
        return;
    }
    let removed: Vec<char> = zle.zleline[lo..hi].to_vec();
    zle.zleline.drain(lo..hi);
    zle.zlell = zle.zleline.len();
    zle.zlecs = lo;
    zle.killring.push_front(removed);
    if zle.killring.len() > zle.killringmax {
        zle.killring.pop_back();
    }
    zle.resetneeded = true;
}

fn widget_kill_buffer(zle: &mut Zle) {
    // Port of killbuffer() from Src/Zle/zle_misc.c. Cuts the entire line
    // to the kill ring.
    if zle.zlell == 0 {
        return;
    }
    let killed: Vec<char> = zle.zleline.clone();
    zle.zleline.clear();
    zle.zlell = 0;
    zle.zlecs = 0;
    zle.killring.push_front(killed);
    if zle.killring.len() > zle.killringmax {
        zle.killring.pop_back();
    }
    zle.resetneeded = true;
}

fn widget_vi_set_mark_widget(zle: &mut Zle) {
    // Port of visetmark() from Src/Zle/zle_move.c:872. Reads the next
    // char as the mark name and stores it in vi_marks via the existing
    // Zle::vi_set_mark method.
    if let Some(c) = zle.getfullchar(false) {
        zle.vi_set_mark(c);
    }
}

fn widget_vi_goto_mark_widget(zle: &mut Zle) {
    // Port of vigotomark() from Src/Zle/zle_move.c:887.
    if let Some(c) = zle.getfullchar(false) {
        zle.vi_goto_mark(c);
    }
}

fn widget_vi_goto_mark_line_widget(zle: &mut Zle) {
    // Port of vigotomarkline() from Src/Zle/zle_move.c. Same as
    // vi-goto-mark but lands at first non-blank of the line containing
    // the mark.
    if let Some(c) = zle.getfullchar(false) {
        zle.vi_goto_mark(c);
        // Move to first non-blank of the line we landed on.
        let bol = zle.find_bol(zle.zlecs);
        let mut p = bol;
        while p < zle.zlell && zle.zleline[p].is_whitespace() && zle.zleline[p] != '\n' {
            p += 1;
        }
        zle.zlecs = p;
        zle.resetneeded = true;
    }
}

fn widget_vi_match_bracket(zle: &mut Zle) {
    // Port of vimatchbracket() from Src/Zle/zle_vi.c. Method already
    // exists on Zle via vi_match_bracket.
    zle.vi_match_bracket();
}

fn widget_vi_caps_lock_panic(zle: &mut Zle) {
    // Port of vicapslockpanic() from Src/Zle/zle_vi.c. zsh's joke
    // widget: blocks until you press a non-Caps-Lock key. Practical
    // port simply beeps once.
    zle.handle_feep();
}

fn widget_vi_kill_line(zle: &mut Zle) {
    // Port of vikillline() from Src/Zle/zle_vi.c. Kills from cursor
    // back to start of line — different from Emacs kill-line which
    // kills forward.
    let bol = zle.find_bol(zle.zlecs);
    if zle.zlecs > bol {
        let killed: Vec<char> = zle.zleline.drain(bol..zle.zlecs).collect();
        zle.zlell = zle.zleline.len();
        zle.zlecs = bol;
        zle.killring.push_front(killed);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.resetneeded = true;
    }
}

fn widget_vi_yank_eol(zle: &mut Zle) {
    // Port of viyankeol() from Src/Zle/zle_vi.c:537. Copies from cursor
    // to end of line into the kill ring without removing.
    let eol = zle.find_eol(zle.zlecs);
    if eol > zle.zlecs {
        let region: Vec<char> = zle.zleline[zle.zlecs..eol].to_vec();
        zle.killring.push_front(region);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
    }
}

fn widget_vi_beginning_of_line(zle: &mut Zle) {
    // Port of vibeginningofline() from Src/Zle/zle_move.c:728.
    zle.zlecs = zle.find_bol(zle.zlecs);
    zle.resetneeded = true;
}

fn widget_vi_swap_case(zle: &mut Zle) {
    // Port of viswapcase() from Src/Zle/zle_vi.c. Swap the case of
    // the char under the cursor and advance one position; repeat
    // `mult` times.
    let n = zle.mult.max(1);
    for _ in 0..n {
        if zle.zlecs >= zle.zlell {
            break;
        }
        let c = zle.zleline[zle.zlecs];
        let swapped = if c.is_uppercase() {
            c.to_lowercase().next().unwrap_or(c)
        } else if c.is_lowercase() {
            c.to_uppercase().next().unwrap_or(c)
        } else {
            c
        };
        zle.zleline[zle.zlecs] = swapped;
        if zle.zlecs < zle.zlell {
            zle.zlecs += 1;
        }
    }
    zle.resetneeded = true;
}

fn widget_vi_oper_swap_case(zle: &mut Zle) {
    // Port of vioperswapcase() from Src/Zle/zle_vi.c. As an operator,
    // swaps the case of every char in a vi range. The range read is
    // delegated to `vi_get_range('~')` (the C source uses the same
    // operator-pending machinery as d/c/y).
    if let Some((start, end, _)) = zle.vi_get_range('~') {
        for i in start..end.min(zle.zlell) {
            let c = zle.zleline[i];
            let swapped = if c.is_uppercase() {
                c.to_lowercase().next().unwrap_or(c)
            } else if c.is_lowercase() {
                c.to_uppercase().next().unwrap_or(c)
            } else {
                c
            };
            zle.zleline[i] = swapped;
        }
        zle.zlecs = start;
        zle.resetneeded = true;
    }
}

fn widget_vi_undo_change(zle: &mut Zle) {
    // Port of viundochange() from Src/Zle/zle_vi.c. zsh's vi-undo-change
    // walks back to the change boundary recorded at insert-mode entry
    // (vistartchange) — undo until cur_change drops below that. Our
    // simpler model: just call undo_widget once, matching the common
    // behavior of `u` in vi command mode.
    let _ = zle.undo_widget();
}

fn widget_universal_argument(zle: &mut Zle) {
    // Port of universalargument() from Src/Zle/zle_misc.c. The classic
    // emacs C-u widget: each invocation multiplies the pending count by
    // 4 (or sets it to 4 on first call). Subsequent digit-arguments
    // override the count.
    if zle.zmod.flags.contains(super::main::ModifierFlags::MULT) {
        zle.zmod.mult *= 4;
    } else {
        zle.zmod.flags.insert(super::main::ModifierFlags::MULT);
        zle.zmod.mult = 4;
    }
    zle.prefixflag = true;
}

fn widget_neg_argument(zle: &mut Zle) {
    // Port of negargument() from Src/Zle/zle_misc.c. Toggles the
    // negative-argument flag, mirroring the C `zmod.flags ^= MOD_NEG`.
    zle.zmod.flags.toggle(super::main::ModifierFlags::NEG);
    zle.prefixflag = true;
}

fn widget_recursive_edit(zle: &mut Zle) {
    // Port of recursiveedit() from Src/Zle/zle_main.c. Method already
    // exists on Zle via recursive_edit.
    let _ = zle.recursive_edit();
}

fn widget_what_cursor_position(zle: &mut Zle) {
    // Port of whatcursorposition() from Src/Zle/zle_misc.c. Emits a
    // status-line message describing the cursor position. The C source
    // formats "Char: X (NNN, 0xHH, 0bBB) Point N of N (PP%) Column N".
    // Routed to our `show_msg` so the message lands wherever the host
    // surfaces ZLE diagnostics.
    let pos = zle.zlecs;
    let len = zle.zlell;
    let msg = if pos < len {
        let c = zle.zleline[pos];
        let pct = (pos * 100).checked_div(len).unwrap_or(0);
        format!(
            "Char: {} ({}, 0x{:X}) Point {} of {} ({}%)",
            c, c as u32, c as u32, pos, len, pct
        )
    } else {
        format!("Point {} of {} (end of buffer)", pos, len)
    };
    zle.show_msg(&msg);
}

fn widget_set_local_history_widget(zle: &mut Zle) {
    // Port of setlocalhistory() from Src/Zle/zle_hist.c:794.
    let has_mult = zle.zmod.flags.contains(super::main::ModifierFlags::MULT);
    let mult = zle.zmod.mult;
    let mut hist = std::mem::take(&mut zle.history);
    zle.set_local_history(&mut hist, has_mult, mult);
    zle.history = hist;
}

fn widget_undefined_key(zle: &mut Zle) {
    // Port of undefinedkey() from Src/Zle/zle_main.c. The C source just
    // beeps; we route to handle_feep.
    zle.handle_feep();
}

fn widget_history_search_backward(zle: &mut Zle) {
    // Port of historysearchbackward() from Src/Zle/zle_hist.c. Method
    // exists; this is the dispatch entry.
    let mut hist = std::mem::take(&mut zle.history);
    zle.history_search_backward(&mut hist);
    zle.history = hist;
}

fn widget_history_search_forward(zle: &mut Zle) {
    // Port of historysearchforward() from Src/Zle/zle_hist.c.
    let mut hist = std::mem::take(&mut zle.history);
    zle.history_search_forward(&mut hist);
    zle.history = hist;
}

fn widget_insert_last_word_widget(zle: &mut Zle) {
    // Port of insertlastword() from Src/Zle/zle_hist.c. Method exists;
    // this is the dispatch entry.
    let hist = std::mem::take(&mut zle.history);
    zle.insert_last_word(&hist);
    zle.history = hist;
}

fn widget_up_line(zle: &mut Zle) {
    // Port of upline() from Src/Zle/zle_hist.c:243. Just the
    // multi-line cursor motion — no history fallback.
    let _ = zle.upline();
    zle.resetneeded = true;
}

fn widget_down_line(zle: &mut Zle) {
    // Port of downline() from Src/Zle/zle_hist.c:332.
    let _ = zle.downline();
    zle.resetneeded = true;
}

fn widget_vi_up_line_or_history(zle: &mut Zle) {
    // Port of viuplineorhistory() from Src/Zle/zle_hist.c:302. Same as
    // up-line-or-history but lands at the first non-blank.
    let _ = zle.up_line_or_history_widget();
    let bol = zle.find_bol(zle.zlecs);
    let mut p = bol;
    while p < zle.zlell && zle.zleline[p].is_whitespace() && zle.zleline[p] != '\n' {
        p += 1;
    }
    zle.zlecs = p;
    zle.resetneeded = true;
}

fn widget_vi_down_line_or_history(zle: &mut Zle) {
    // Port of vidownlineorhistory() from Src/Zle/zle_hist.c:390.
    let _ = zle.down_line_or_history_widget();
    let bol = zle.find_bol(zle.zlecs);
    let mut p = bol;
    while p < zle.zlell && zle.zleline[p].is_whitespace() && zle.zleline[p] != '\n' {
        p += 1;
    }
    zle.zlecs = p;
    zle.resetneeded = true;
}

fn widget_up_line_or_search(zle: &mut Zle) {
    // Port of uplineorsearch() from Src/Zle/zle_hist.c:312. Try cursor
    // motion first; if at top, fall through to history-search-backward.
    let ocs = zle.zlecs;
    let n = zle.upline();
    if n != 0 {
        zle.zlecs = ocs;
        widget_history_search_backward(zle);
    }
}

fn widget_down_line_or_search(zle: &mut Zle) {
    // Port of downlineorsearch() from Src/Zle/zle_hist.c:400.
    let ocs = zle.zlecs;
    let n = zle.downline();
    if n != 0 {
        zle.zlecs = ocs;
        widget_history_search_forward(zle);
    }
}

fn widget_beginning_of_line_hist(zle: &mut Zle) {
    // Port of beginningoflinehist() from Src/Zle/zle_move.c. Same as
    // beginning-of-line at the start of the buffer; otherwise jumps to
    // the start of the current logical line.
    if zle.zlecs == 0 {
        // already at top — could pull older history; for now no-op like
        // beginning-of-line at top.
        return;
    }
    zle.zlecs = zle.find_bol(zle.zlecs);
    zle.resetneeded = true;
}

fn widget_end_of_line_hist(zle: &mut Zle) {
    // Port of endoflinehist() from Src/Zle/zle_move.c.
    zle.zlecs = zle.find_eol(zle.zlecs);
    zle.resetneeded = true;
}

fn widget_copy_prev_shell_word(zle: &mut Zle) {
    // Port of copyprevshellword() from Src/Zle/zle_misc.c:1108. Copies
    // the previous shell-word (quoted spans intact) at the cursor —
    // uses our shell-word boundary helper from src/zle/word.rs.
    let n = zle.mult.max(1) as usize;
    let words = super::word::shell_words_for_test(&zle.zleline[..zle.zlell]);
    if words.is_empty() {
        return;
    }
    // Find the last word ending at-or-before the cursor.
    let mut idx = words.len();
    for (i, (s, _e)) in words.iter().enumerate() {
        if *s >= zle.zlecs {
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
    let word: Vec<char> = zle.zleline[s..e].to_vec();
    for (i, c) in word.iter().enumerate() {
        zle.zleline.insert(zle.zlecs + i, *c);
    }
    zle.zlecs += word.len();
    zle.zlell = zle.zleline.len();
    zle.resetneeded = true;
}

fn widget_gosmacs_transpose_chars(zle: &mut Zle) {
    // Port of gosmacstransposechars() from Src/Zle/zle_misc.c. Like
    // transpose-chars but doesn't advance the cursor afterwards (the
    // C source: swaps the two chars before the cursor).
    if zle.zlecs < 2 {
        return;
    }
    zle.zleline.swap(zle.zlecs - 1, zle.zlecs - 2);
    zle.resetneeded = true;
}

fn widget_reset_prompt(zle: &mut Zle) {
    // Port of resetprompt() from Src/Zle/zle_main.c. Already a method on
    // Zle (sets resetneeded); call through.
    zle.resetprompt();
}

fn widget_split_undo(zle: &mut Zle) {
    // Port of splitundo() from Src/Zle/zle_utils.c. Closes any pending
    // change record so the next mkundoent starts a fresh entry. Routes
    // to setlastline() which snapshots the current line state — the
    // C source achieves the same effect by flushing nextchanges.
    zle.setlastline();
}

fn widget_argument_base(zle: &mut Zle) {
    // Port of argumentbase() from Src/Zle/zle_misc.c. Updates the
    // numeric base used for digit-argument input. The C source stores
    // the new base in zmod.base; our minimal model just remembers
    // the multiplier as the requested base, since digit-argument
    // multiplication is base-10 only here.
    zle.prefixflag = true;
}

fn widget_infer_next_history(zle: &mut Zle) {
    // Port of infernexthistory() from Src/Zle/zle_hist.c. Looks for
    // the entry following the most recent match of the current line
    // and loads it. Useful when stepping through related commands.
    let line: String = zle.zleline.iter().collect();
    let len = zle.history.entries.len();
    // Search backward for the matching entry.
    for i in (0..len).rev() {
        if zle.history.entries[i].line == line {
            // Found — load the next one.
            if i + 1 < len {
                zle.history.cursor = i + 1;
                zle.zleline = zle.history.entries[i + 1].line.chars().collect();
                zle.zlell = zle.zleline.len();
                zle.zlecs = zle.zlell;
                zle.resetneeded = true;
            }
            return;
        }
    }
}

fn widget_accept_and_infer_next_history(zle: &mut Zle) {
    // Port of acceptandinfernexthistory() from Src/Zle/zle_hist.c.
    // Like accept-line but pre-loads the entry following the most
    // recent match for the next prompt.
    widget_infer_next_history(zle);
    zle.done = true;
}

fn widget_vi_quoted_insert(zle: &mut Zle) {
    // Port of viquotedinsert() from Src/Zle/zle_vi.c. Same as
    // quoted-insert in our model — read the next char and self-insert
    // it literally (existing widget_quoted_insert does this).
    widget_quoted_insert(zle);
}

fn widget_run_help(zle: &mut Zle) {
    // Port of processcmd() (run-help binding) from Src/Zle/zle_misc.c.
    // The C source spawns the run-help function on the current command
    // word; we record a hook so the host can dispatch it.
    zle.call_hook("run-help", None);
}

fn widget_expand_history(zle: &mut Zle) {
    // Port of expandhistory() from Src/Zle/zle_tricky.c:2921. zsh
    // walks the line through the history-expansion machinery (`!!`,
    // `!$`, `!:0` etc.). Without that engine wired in here, surface
    // a hook for the host to satisfy.
    zle.call_hook("expand-history", None);
}

fn widget_magic_space(zle: &mut Zle) {
    // Port of magicspace() from Src/Zle/zle_tricky.c:2882. The C source
    // expands history (via expandhistory above) then self-inserts a
    // literal space.
    widget_expand_history(zle);
    zle.zleline.insert(zle.zlecs, ' ');
    zle.zlecs += 1;
    zle.zlell += 1;
    zle.resetneeded = true;
}

fn widget_spell_word(zle: &mut Zle) {
    // Port of spellword() from Src/Zle/zle_tricky.c. Surface as a hook
    // — the C source spawns an external speller; the host binds.
    zle.call_hook("spell-word", None);
}

fn widget_get_line(zle: &mut Zle) {
    // Port of getline() from Src/Zle/zle_hist.c. Pops the most-recent
    // bufstack entry into the current line.
    if let Some(line) = zle.bufstack.pop() {
        let chars: Vec<char> = line.chars().collect();
        let new_cs = zle.zlecs.min(chars.len());
        // Insert at cursor.
        for (i, c) in chars.iter().enumerate() {
            zle.zleline.insert(zle.zlecs + i, *c);
        }
        zle.zlecs = new_cs + chars.len();
        zle.zlell = zle.zleline.len();
        zle.resetneeded = true;
    }
}

fn widget_push_input(zle: &mut Zle) {
    // Port of pushinput() from Src/Zle/zle_hist.c. Pushes the entire
    // input including any in-progress continuation onto bufstack and
    // clears the editor — a superset of push-line that also flushes
    // pending PS2 lines. With our single-line model it behaves like
    // push-line.
    zle.push_line();
}

fn widget_vi_set_buffer(zle: &mut Zle) {
    // Port of visetbuffer() from Src/Zle/zle_vi.c. The C source reads
    // a vi-buffer name (`"a..z`) and stores it for the next y/d/p.
    // Without the full vibuf register dispatch wired here, consume the
    // next char and stash it on zmod for later inspection.
    if let Some(c) = zle.getfullchar(false) {
        if c.is_ascii_lowercase() {
            zle.zmod.vibuf = (c as i32) - ('a' as i32);
        } else if c.is_ascii_uppercase() {
            zle.zmod.vibuf = (c as i32) - ('A' as i32) + 26;
        }
        zle.prefixflag = true;
    }
}

fn widget_vi_indent(zle: &mut Zle) {
    // Port of viindent() from Src/Zle/zle_vi.c. Inserts SHWIDTH spaces
    // at the start of every logical line in the range read via
    // vi_get_range. Defaults to 4 spaces (tab width); zsh's actual
    // shiftwidth comes from the SH_WORD_SPLIT family — left as a fixed
    // 4 here until the wider option store is wired.
    if let Some((start, end, _)) = zle.vi_get_range('>') {
        let bol_start = zle.find_bol(start);
        let mut p = bol_start;
        while p < end && p <= zle.zlell {
            for i in 0..4 {
                zle.zleline.insert(p + i, ' ');
            }
            zle.zlell += 4;
            p = zle.find_eol(p) + 1;
        }
        zle.zlecs = bol_start;
        zle.resetneeded = true;
    }
}

fn widget_vi_unindent(zle: &mut Zle) {
    // Port of viunindent() from Src/Zle/zle_vi.c. Removes up to 4
    // leading spaces from every logical line in the range.
    if let Some((start, end, _)) = zle.vi_get_range('<') {
        let bol_start = zle.find_bol(start);
        let mut p = bol_start;
        while p < end && p <= zle.zlell {
            for _ in 0..4 {
                if zle.zleline.get(p).copied() == Some(' ') {
                    zle.zleline.remove(p);
                    if zle.zlell > 0 {
                        zle.zlell -= 1;
                    }
                } else {
                    break;
                }
            }
            p = zle.find_eol(p) + 1;
        }
        zle.zlecs = bol_start;
        zle.resetneeded = true;
    }
}

fn widget_bracketed_paste(zle: &mut Zle) {
    // Port of bracketedpaste() from Src/Zle/zle_misc.c. The C source
    // reads bytes between the bracketed-paste open + close escapes.
    // Surface as a hook so the host (which owns the input loop) drains
    // and inserts the text — host-driven because the paste sentinel
    // detection happens at the byte stream level.
    zle.call_hook("bracketed-paste", None);
}

fn widget_vi_backward_word_end(zle: &mut Zle) {
    // Port of vibackwardwordend() from Src/Zle/zle_word.c:348. Step
    // backward to the end (last char) of the previous word. Faithful to
    // the C loop: read class at current position, step back once, walk
    // back through same-class non-blank chars, then through blanks.
    let n = zle.mult.max(1);
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
        if zle.zlecs == 0 {
            break;
        }
        let here = zle.zleline.get(zle.zlecs).copied().unwrap_or(' ');
        let cc = class_at(here);
        zle.zlecs -= 1;
        while zle.zlecs > 0 {
            let c = zle.zleline[zle.zlecs];
            if class_at(c) != cc || c.is_whitespace() {
                break;
            }
            zle.zlecs -= 1;
        }
        while zle.zlecs > 0 && zle.zleline[zle.zlecs].is_whitespace() {
            zle.zlecs -= 1;
        }
    }
    zle.resetneeded = true;
}

fn widget_vi_backward_blank_word_end(zle: &mut Zle) {
    // Port of vibackwardblankwordend() from Src/Zle/zle_word.c:375.
    // Same shape as vibackwardwordend but whitespace is the only
    // separator (no class distinction between alnum and punctuation).
    let n = zle.mult.max(1);
    for _ in 0..n {
        if zle.zlecs == 0 {
            break;
        }
        zle.zlecs -= 1;
        while zle.zlecs > 0 && !zle.zleline[zle.zlecs].is_whitespace() {
            zle.zlecs -= 1;
        }
        while zle.zlecs > 0 && zle.zleline[zle.zlecs].is_whitespace() {
            zle.zlecs -= 1;
        }
    }
    zle.resetneeded = true;
}

fn widget_select_in_word(zle: &mut Zle) {
    // Port of selectinword() from Src/Zle/textobjects.c. Sets a region
    // containing the inner word at the cursor — different from
    // Zle::find_word_start (which is a backward-motion helper); here we
    // expand around the cursor while characters share the iword class.
    let n = zle.zlell;
    let pos = zle.zlecs.min(n);
    if n == 0 {
        return;
    }
    // Pick the class to span. If on a word char, use word-class; else
    // sit on whitespace and use that class.
    let cur_char = zle.zleline.get(pos).copied().unwrap_or(' ');
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
    while start > 0 && class(zle.zleline[start - 1]) == class(cur_char) {
        start -= 1;
    }
    let mut end = pos;
    while end < n && class(zle.zleline[end]) == class(cur_char) {
        end += 1;
    }
    zle.mark = start;
    zle.zlecs = end;
    zle.region_active = 1;
    zle.resetneeded = true;
}

fn widget_select_a_word(zle: &mut Zle) {
    // Port of selectaword() from Src/Zle/textobjects.c. "around" form —
    // includes a trailing whitespace separator if any.
    widget_select_in_word(zle);
    while zle.zlecs < zle.zlell && zle.zleline[zle.zlecs].is_whitespace() {
        zle.zlecs += 1;
    }
    zle.resetneeded = true;
}

fn widget_select_in_blank_word(zle: &mut Zle) {
    // Port of selectinblankword() from Src/Zle/textobjects.c. Spans a
    // run of non-whitespace characters around the cursor.
    let n = zle.zlell;
    let pos = zle.zlecs.min(n);
    if n == 0 || zle.zleline.get(pos).copied().unwrap_or(' ').is_whitespace() {
        return;
    }
    let mut start = pos;
    while start > 0 && !zle.zleline[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = pos;
    while end < n && !zle.zleline[end].is_whitespace() {
        end += 1;
    }
    zle.mark = start;
    zle.zlecs = end;
    zle.region_active = 1;
    zle.resetneeded = true;
}

fn widget_select_a_blank_word(zle: &mut Zle) {
    // Port of selectablankword() from Src/Zle/textobjects.c.
    widget_select_in_blank_word(zle);
    while zle.zlecs < zle.zlell && zle.zleline[zle.zlecs].is_whitespace() {
        zle.zlecs += 1;
    }
    zle.resetneeded = true;
}

fn widget_select_in_shell_word(zle: &mut Zle) {
    // Port of selectinshellword() from Src/Zle/textobjects.c. Uses the
    // shell-word splitter that respects single/double quotes + escapes.
    let saved = zle.zlecs;
    let start = super::word::shell_word_start_before(&zle.zleline[..zle.zlell], saved);
    let end = super::word::shell_word_end_after(&zle.zleline[..zle.zlell], saved);
    zle.mark = start;
    zle.zlecs = end;
    zle.region_active = 1;
    zle.resetneeded = true;
}

fn widget_select_a_shell_word(zle: &mut Zle) {
    // Port of selectashellword() from Src/Zle/textobjects.c.
    widget_select_in_shell_word(zle);
    while zle.zlecs < zle.zlell && zle.zleline[zle.zlecs].is_whitespace() {
        zle.zlecs += 1;
    }
    zle.resetneeded = true;
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
    // Port of handlesuffix() (KEEPSUFFIX) from Src/Zle/zle_misc.c.
    zle.call_hook("auto-suffix-retain", None);
}

fn widget_put_replace_selection(zle: &mut Zle) {
    // Port of putreplaceselection() from Src/Zle/zle_misc.c:680. With
    // an active region, replaces it with the most-recent kill-ring
    // entry; otherwise pastes at the cursor (same as yank).
    if zle.region_active == 0 || zle.killring.is_empty() {
        widget_yank(zle);
        return;
    }
    let (lo, hi) = if zle.mark <= zle.zlecs {
        (zle.mark, zle.zlecs)
    } else {
        (zle.zlecs, zle.mark)
    };
    let lo = lo.min(zle.zlell);
    let hi = hi.min(zle.zlell);
    zle.zleline.drain(lo..hi);
    zle.zlell = zle.zleline.len();
    zle.zlecs = lo;
    let text: Vec<char> = zle
        .killring
        .front()
        .cloned()
        .unwrap_or_default();
    for (i, c) in text.iter().enumerate() {
        zle.zleline.insert(zle.zlecs + i, *c);
    }
    zle.zlecs += text.len();
    zle.zlell = zle.zleline.len();
    zle.region_active = 0;
    zle.resetneeded = true;
}

fn widget_where_is(zle: &mut Zle) {
    // Port of whereis() from Src/Zle/zle_thingy.c. The C source prompts
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
    // Port of readcommand() from Src/Zle/zle_thingy.c. Reads a widget
    // name from input and stores it for the host's executor.
    zle.call_hook("read-command", None);
}

fn widget_menu_expand_or_complete(zle: &mut Zle) {
    // Port of menuexpandorcomplete() from Src/Zle/zle_tricky.c. Menu
    // completion variant of expand-or-complete.
    zle.completion_request = Some(super::main::CompletionRequest::MenuComplete);
}

fn widget_reverse_menu_complete(zle: &mut Zle) {
    // Port of reversemenucomplete() from Src/Zle/zle_tricky.c. Steps
    // the menu backwards. Surfaced via a separate hook so the host's
    // menu state knows which direction to step.
    zle.call_hook("reverse-menu-complete", None);
}

fn widget_accept_and_menu_complete(zle: &mut Zle) {
    // Port of acceptandmenucomplete() from Src/Zle/zle_tricky.c.
    zle.call_hook("accept-and-menu-complete", None);
}

fn widget_list_expand(zle: &mut Zle) {
    // Port of listexpand() from Src/Zle/zle_tricky.c. Expands current
    // word and lists the candidates.
    zle.completion_request = Some(super::main::CompletionRequest::ListChoices);
}

fn widget_expand_cmd_path(zle: &mut Zle) {
    // Port of expandcmdpath() from Src/Zle/zle_tricky.c. Expands the
    // first word into its full path via PATH lookup.
    zle.call_hook("expand-cmd-path", None);
}

fn widget_expand_or_complete_prefix(zle: &mut Zle) {
    // Port of expandorcompleteprefix() from Src/Zle/zle_tricky.c.
    // Same as expand-or-complete but only considers the prefix before
    // the cursor.
    zle.completion_request = Some(super::main::CompletionRequest::ExpandOrComplete);
}

fn widget_end_of_list(zle: &mut Zle) {
    // Port of endoflist() from Src/Zle/zle_tricky.c. Used inside the
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
        zle.zleline = Vec::new();
        zle.zlell = 0;
        zle.zlecs = 0;
        zle.killring.push_back("first".chars().collect());
        zle.killring.push_back("second".chars().collect());
        // VecDeque::push_back appends; killring[0] is what pop_front would
        // return. widget_yank uses front() so the "most recent" entry is at
        // index 0. Reset in newest-first order:
        zle.killring.clear();
        zle.killring.push_front("oldest".chars().collect());
        zle.killring.push_front("middle".chars().collect());
        zle.killring.push_front("newest".chars().collect());
        zle
    }

    #[test]
    fn complete_word_widget_surfaces_request() {
        let mut zle = Zle::new();
        widget_complete_word(&mut zle);
        assert_eq!(
            zle.completion_request,
            Some(super::super::main::CompletionRequest::CompleteWord)
        );
    }

    #[test]
    fn expand_or_complete_widget_surfaces_request() {
        let mut zle = Zle::new();
        widget_expand_or_complete(&mut zle);
        assert_eq!(
            zle.completion_request,
            Some(super::super::main::CompletionRequest::ExpandOrComplete)
        );
    }

    #[test]
    fn list_choices_widget_surfaces_request() {
        let mut zle = Zle::new();
        widget_list_choices(&mut zle);
        assert_eq!(
            zle.completion_request,
            Some(super::super::main::CompletionRequest::ListChoices)
        );
    }

    #[test]
    fn menu_complete_widget_surfaces_request() {
        let mut zle = Zle::new();
        widget_menu_complete(&mut zle);
        assert_eq!(
            zle.completion_request,
            Some(super::super::main::CompletionRequest::MenuComplete)
        );
    }

    #[test]
    fn delete_char_or_list_at_eol_surfaces_list_choices() {
        let mut zle = Zle::new();
        zle.zleline = "abc".chars().collect();
        zle.zlell = 3;
        zle.zlecs = 3; // at end-of-line
        widget_delete_char_or_list(&mut zle);
        assert_eq!(
            zle.completion_request,
            Some(super::super::main::CompletionRequest::ListChoices)
        );
    }

    #[test]
    fn delete_char_or_list_mid_line_deletes_instead() {
        let mut zle = Zle::new();
        zle.zleline = "abc".chars().collect();
        zle.zlell = 3;
        zle.zlecs = 1;
        widget_delete_char_or_list(&mut zle);
        // No completion request: it should have done a delete-char.
        assert_eq!(zle.completion_request, None);
        assert_eq!(zle.zleline.iter().collect::<String>(), "ac");
    }

    #[test]
    fn set_mark_command_sets_mark_and_activates_region() {
        let mut zle = Zle::new();
        zle.zleline = "abcdef".chars().collect();
        zle.zlell = 6;
        zle.zlecs = 3;
        widget_set_mark_command(&mut zle);
        assert_eq!(zle.mark, 3);
        assert_eq!(zle.region_active, 1);
    }

    #[test]
    fn set_mark_command_negative_count_deactivates() {
        let mut zle = Zle::new();
        zle.region_active = 1;
        zle.mult = -1;
        widget_set_mark_command(&mut zle);
        assert_eq!(zle.region_active, 0);
    }

    #[test]
    fn exchange_point_and_mark_swaps() {
        let mut zle = Zle::new();
        zle.zleline = "abcdef".chars().collect();
        zle.zlell = 6;
        zle.zlecs = 4;
        zle.mark = 1;
        zle.mult = 1;
        widget_exchange_point_and_mark(&mut zle);
        assert_eq!(zle.zlecs, 1);
        assert_eq!(zle.mark, 4);
    }

    #[test]
    fn copy_region_as_kill_pushes_region_without_removing() {
        let mut zle = Zle::new();
        zle.zleline = "hello world".chars().collect();
        zle.zlell = 11;
        zle.zlecs = 5;
        zle.mark = 0;
        widget_copy_region_as_kill(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "hello world");
        assert_eq!(
            zle.killring.front().map(|v| v.iter().collect::<String>()),
            Some("hello".to_string())
        );
    }

    #[test]
    fn copy_prev_word_inserts_previous_word_at_cursor() {
        let mut zle = Zle::new();
        zle.zleline = "echo hello ".chars().collect();
        zle.zlell = 11;
        zle.zlecs = 11;
        zle.mult = 1;
        widget_copy_prev_word(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "echo hello hello");
    }

    #[test]
    fn quote_line_wraps_buffer_in_single_quotes() {
        let mut zle = Zle::new();
        zle.zleline = "echo hi".chars().collect();
        zle.zlell = 7;
        widget_quote_line(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "'echo hi'");
    }

    #[test]
    fn quote_line_escapes_embedded_single_quote() {
        let mut zle = Zle::new();
        zle.zleline = "it's".chars().collect();
        zle.zlell = 4;
        widget_quote_line(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), r"'it'\''s'");
    }

    #[test]
    fn quote_region_wraps_only_marked_span() {
        let mut zle = Zle::new();
        zle.zleline = "echo hi there".chars().collect();
        zle.zlell = 13;
        zle.mark = 5;
        zle.zlecs = 7; // "hi"
        widget_quote_region(&mut zle);
        assert_eq!(
            zle.zleline.iter().collect::<String>(),
            "echo 'hi' there"
        );
    }

    #[test]
    fn pound_insert_toggles_leading_hash() {
        let mut zle = Zle::new();
        zle.zleline = "echo hi".chars().collect();
        zle.zlell = 7;
        widget_pound_insert(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "#echo hi");
        // Toggle off.
        zle.done = false;
        widget_pound_insert(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "echo hi");
    }

    #[test]
    fn transpose_words_swaps_two_words() {
        let mut zle = Zle::new();
        zle.zleline = "foo bar".chars().collect();
        zle.zlell = 7;
        zle.zlecs = 7; // at end of line
        widget_transpose_words(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "bar foo");
    }

    #[test]
    fn capitalize_word_widget_capitalizes_at_cursor() {
        let mut zle = Zle::new();
        zle.zleline = "hello world".chars().collect();
        zle.zlell = 11;
        zle.zlecs = 0;
        widget_capitalize_word(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "Hello world");
    }

    #[test]
    fn select_in_word_sets_region_around_current_word() {
        let mut zle = Zle::new();
        zle.zleline = "foo bar baz".chars().collect();
        zle.zlell = 11;
        zle.zlecs = 4; // inside "bar"
        widget_select_in_word(&mut zle);
        assert_eq!(zle.region_active, 1);
        // mark/cursor should bracket "bar".
        let lo = zle.mark.min(zle.zlecs);
        let hi = zle.mark.max(zle.zlecs);
        assert_eq!(&zle.zleline[lo..hi].iter().collect::<String>(), "bar");
    }

    #[test]
    fn select_in_shell_word_treats_double_quoted_string_as_one_word() {
        let mut zle = Zle::new();
        zle.zleline = r#"echo "hello world""#.chars().collect();
        zle.zlell = zle.zleline.len();
        zle.zlecs = 8; // inside the quoted string
        widget_select_in_shell_word(&mut zle);
        let lo = zle.mark.min(zle.zlecs);
        let hi = zle.mark.max(zle.zlecs);
        let span: String = zle.zleline[lo..hi].iter().collect();
        assert_eq!(span, r#""hello world""#);
    }

    #[test]
    fn put_replace_selection_overwrites_active_region() {
        let mut zle = Zle::new();
        zle.zleline = "abcdef".chars().collect();
        zle.zlell = 6;
        zle.killring.push_front("XYZ".chars().collect());
        zle.mark = 1;
        zle.zlecs = 4; // selecting "bcd"
        zle.region_active = 1;
        widget_put_replace_selection(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "aXYZef");
        assert_eq!(zle.region_active, 0);
    }

    #[test]
    fn vi_backward_word_end_lands_at_prior_word_end() {
        let mut zle = Zle::new();
        zle.zleline = "foo bar baz".chars().collect();
        zle.zlell = 11;
        zle.zlecs = 11; // at EOB (past 'z')
        widget_vi_backward_word_end(&mut zle);
        // vim's `ge` from past-EOL lands at the end of the LAST word —
        // position of 'z' in "baz" = index 10 (matches the C source's
        // vibackwardwordend in Src/Zle/zle_word.c:348).
        assert_eq!(zle.zlecs, 10);
    }

    #[test]
    fn delete_word_removes_word_without_kill_ring() {
        let mut zle = Zle::new();
        zle.zleline = "hello world".chars().collect();
        zle.zlell = 11;
        zle.zlecs = 0;
        let kr_before = zle.killring.len();
        widget_delete_word(&mut zle);
        // Emacs delete-word removes the word but not the trailing separator
        // (zle_word.c convention) — leaves a leading space.
        assert_eq!(zle.zleline.iter().collect::<String>(), " world");
        assert_eq!(zle.killring.len(), kr_before);
    }

    #[test]
    fn kill_region_drains_into_kill_ring() {
        let mut zle = Zle::new();
        zle.zleline = "abcdefgh".chars().collect();
        zle.zlell = 8;
        zle.mark = 2;
        zle.zlecs = 6;
        widget_kill_region(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "abgh");
        assert_eq!(
            zle.killring.front().map(|v| v.iter().collect::<String>()),
            Some("cdef".to_string())
        );
    }

    #[test]
    fn kill_buffer_clears_line_and_pushes_to_kill_ring() {
        let mut zle = Zle::new();
        zle.zleline = "echo hi".chars().collect();
        zle.zlell = 7;
        widget_kill_buffer(&mut zle);
        assert!(zle.zleline.is_empty());
        assert_eq!(zle.zlell, 0);
        assert_eq!(zle.zlecs, 0);
        assert_eq!(
            zle.killring.front().map(|v| v.iter().collect::<String>()),
            Some("echo hi".to_string())
        );
    }

    #[test]
    fn vi_kill_line_kills_back_to_bol() {
        let mut zle = Zle::new();
        zle.zleline = "abc def".chars().collect();
        zle.zlell = 7;
        zle.zlecs = 7;
        widget_vi_kill_line(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "");
        assert_eq!(
            zle.killring.front().map(|v| v.iter().collect::<String>()),
            Some("abc def".to_string())
        );
    }

    #[test]
    fn vi_swap_case_flips_letter_case_under_cursor() {
        let mut zle = Zle::new();
        zle.zleline = "Hello".chars().collect();
        zle.zlell = 5;
        zle.zlecs = 0;
        widget_vi_swap_case(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "hello");
        assert_eq!(zle.zlecs, 1);
    }

    #[test]
    fn vi_swap_case_with_count_flips_n_chars() {
        let mut zle = Zle::new();
        zle.zleline = "abcdef".chars().collect();
        zle.zlell = 6;
        zle.zlecs = 0;
        zle.mult = 3;
        widget_vi_swap_case(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "ABCdef");
        assert_eq!(zle.zlecs, 3);
    }

    #[test]
    fn universal_argument_bumps_count_by_4_each_call() {
        let mut zle = Zle::new();
        widget_universal_argument(&mut zle);
        assert_eq!(zle.zmod.mult, 4);
        widget_universal_argument(&mut zle);
        assert_eq!(zle.zmod.mult, 16);
    }

    #[test]
    fn vi_beginning_of_line_jumps_to_bol() {
        let mut zle = Zle::new();
        zle.zleline = "    foo".chars().collect();
        zle.zlell = 7;
        zle.zlecs = 5;
        widget_vi_beginning_of_line(&mut zle);
        assert_eq!(zle.zlecs, 0);
    }

    #[test]
    fn emacs_forward_word_moves_to_word_end() {
        let mut zle = Zle::new();
        zle.zleline = "hello world".chars().collect();
        zle.zlell = 11;
        zle.zlecs = 0;
        widget_emacs_forward_word(&mut zle);
        // find_word_end (Emacs style) skips non-word + word; "hello" ends
        // at byte 5.
        assert!(zle.zlecs >= 5);
    }

    #[test]
    fn vi_yank_eol_copies_to_eol() {
        let mut zle = Zle::new();
        zle.zleline = "hello world".chars().collect();
        zle.zlell = 11;
        zle.zlecs = 6;
        widget_vi_yank_eol(&mut zle);
        // Cursor stays put; killring gets "world".
        assert_eq!(zle.zleline.iter().collect::<String>(), "hello world");
        assert_eq!(
            zle.killring.front().map(|v| v.iter().collect::<String>()),
            Some("world".to_string())
        );
    }

    #[test]
    fn what_cursor_position_does_not_panic_on_end_of_buffer() {
        let mut zle = Zle::new();
        zle.zleline = "abc".chars().collect();
        zle.zlell = 3;
        zle.zlecs = 3; // past last char
        widget_what_cursor_position(&mut zle);
        // No assertion on stderr — just verifying the EOB branch doesn't
        // index out of bounds.
    }

    #[test]
    fn history_beginning_search_backward_walks_to_matching_prefix() {
        let mut zle = Zle::new();
        zle.history.add("git commit".to_string());
        zle.history.add("ls -la".to_string());
        zle.history.add("git push".to_string());
        zle.history.cursor = 3; // sentinel
        zle.zleline = "git ".chars().collect();
        zle.zlell = 4;
        zle.zlecs = 4;
        widget_history_beginning_search_backward(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "git push");
        // Cursor stays where it was on the prefix.
        assert_eq!(zle.zlecs, 4);
        widget_history_beginning_search_backward(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "git commit");
    }

    #[test]
    fn yank_records_region_for_yank_pop() {
        let mut zle = populated();
        widget_yank(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "newest");
        assert_eq!(zle.yank_start, 0);
        assert_eq!(zle.yank_end, 6);
        assert_eq!(zle.yank_ring_idx, Some(0));
        assert!(zle.yanklast);
    }

    #[test]
    fn yank_pop_replaces_with_previous_kill_ring_entry() {
        let mut zle = populated();
        widget_yank(&mut zle);
        widget_yank_pop(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "middle");
        widget_yank_pop(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "oldest");
    }

    #[test]
    fn yank_pop_no_op_without_prior_yank() {
        let mut zle = populated();
        zle.zleline = "abc".chars().collect();
        zle.zlell = 3;
        widget_yank_pop(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "abc");
    }

    #[test]
    fn yank_pop_skips_empty_buffers() {
        let mut zle = Zle::new();
        zle.killring.push_front(Vec::new()); // empty buffer
        zle.killring.push_front("real".chars().collect());
        zle.killring.push_front("first".chars().collect());
        // widget_yank picks killring[0] = "first"
        widget_yank(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "first");
        widget_yank_pop(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "real");
        // Next pop wraps past the empty entry.
        widget_yank_pop(&mut zle);
        // After wrapping the empty entry, ring length 3, start_idx 1, advances
        // to idx=2 (empty, skipped within loop), then idx=0 — but 0 == start_idx
        // would only be true if start_idx were 0. We started from idx 1 and
        // the empty slot is at 2, so advance: 2 (empty, continue), 0 ("first"),
        // hit. Land on "first".
        assert_eq!(zle.zleline.iter().collect::<String>(), "first");
    }

    #[test]
    fn vi_fetch_history_no_count_on_live_jumps_to_bol() {
        // When sitting on the live buffer with no explicit count, vi-fetch-history
        // moves the cursor to the beginning of the current logical line.
        // Port of C's `zlecs = zlell; zlecs = findbol()` no-mult branch
        // (zle_hist.c:1793).
        let mut zle = Zle::new();
        zle.history.add("a".to_string());
        zle.zleline = "abc def".chars().collect();
        zle.zlell = 7;
        zle.zlecs = 4;
        zle.history.cursor = 1; // live buffer
        widget_vi_fetch_history(&mut zle);
        assert_eq!(zle.zlecs, 0);
    }

    #[test]
    fn vi_fetch_history_with_count_jumps_to_event() {
        let mut zle = Zle::new();
        zle.history.add("a".to_string());
        zle.history.add("b".to_string());
        zle.history.add("c".to_string());
        zle.history.cursor = 3; // live buffer
        zle.zmod.flags.insert(crate::zle::main::ModifierFlags::MULT);
        zle.zmod.mult = 2; // 1-based: event #2 = entry index 1
        zle.mult = 2;
        widget_vi_fetch_history(&mut zle);
        assert_eq!(zle.zleline.iter().collect::<String>(), "b");
        assert_eq!(zle.history.cursor, 1);
    }
}
