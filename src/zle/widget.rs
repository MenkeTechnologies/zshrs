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

        // Default: undefined widget
        _ => (widget_undefined, WidgetFlags::empty()),
    }
}

// Widget implementations

fn widget_accept_line(zle: &mut Zle) {
    zle.accept_line();
}

fn widget_accept_and_hold(zle: &mut Zle) {
    // TODO: implement accept-and-hold
    zle.accept_line();
}

fn widget_accept_line_and_down_history(zle: &mut Zle) {
    // TODO: implement accept-line-and-down-history
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
    if zle.zlell == 0 {
        // On empty line, send EOF
        zle.done = true;
    } else if zle.zlecs < zle.zlell {
        widget_delete_char(zle);
    } else {
        // At end of line, list completions
        // TODO: implement completion listing
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
    if let Some(text) = zle.killring.front().cloned() {
        for c in text {
            zle.zleline.insert(zle.zlecs, c);
            zle.zlecs += 1;
            zle.zlell += 1;
        }
        zle.resetneeded = true;
    }
}

fn widget_yank_pop(zle: &mut Zle) {
    // Rotate kill ring and yank
    if let Some(text) = zle.killring.pop_front() {
        zle.killring.push_back(text);
    }
    // TODO: implement proper yank-pop (replace previous yank)
}

fn widget_undo(zle: &mut Zle) {
    // TODO: implement undo
    let _ = zle;
}

fn widget_redo(zle: &mut Zle) {
    // TODO: implement redo
    let _ = zle;
}

fn widget_up_line_or_history(zle: &mut Zle) {
    // TODO: implement history navigation
    let _ = zle;
}

fn widget_down_line_or_history(zle: &mut Zle) {
    // TODO: implement history navigation
    let _ = zle;
}

fn widget_up_history(zle: &mut Zle) {
    // TODO: implement history navigation
    let _ = zle;
}

fn widget_down_history(zle: &mut Zle) {
    // TODO: implement history navigation
    let _ = zle;
}

fn widget_history_isearch_backward(zle: &mut Zle) {
    // TODO: implement incremental search
    let _ = zle;
}

fn widget_history_isearch_forward(zle: &mut Zle) {
    // TODO: implement incremental search
    let _ = zle;
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
    // TODO: implement completion
    let _ = zle;
}

fn widget_complete_word(zle: &mut Zle) {
    // TODO: implement completion
    let _ = zle;
}

fn widget_expand_word(zle: &mut Zle) {
    // TODO: implement expansion
    let _ = zle;
}

fn widget_list_choices(zle: &mut Zle) {
    // TODO: implement completion listing
    let _ = zle;
}

fn widget_menu_complete(zle: &mut Zle) {
    // TODO: implement menu completion
    let _ = zle;
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
    // TODO: implement vi delete operator
    let _ = zle;
}

fn widget_vi_delete_char(zle: &mut Zle) {
    widget_delete_char(zle);
}

fn widget_vi_backward_delete_char(zle: &mut Zle) {
    widget_backward_delete_char(zle);
}

fn widget_vi_change(zle: &mut Zle) {
    // TODO: implement vi change operator
    let _ = zle;
}

fn widget_vi_change_eol(zle: &mut Zle) {
    widget_kill_line(zle);
    widget_vi_insert(zle);
}

fn widget_vi_kill_eol(zle: &mut Zle) {
    widget_kill_line(zle);
}

fn widget_vi_yank(zle: &mut Zle) {
    // TODO: implement vi yank operator
    let _ = zle;
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
    // TODO: implement vi repeat change
    let _ = zle;
}

fn widget_vi_find_next_char(zle: &mut Zle) {
    // TODO: implement vi find next char
    let _ = zle;
}

fn widget_vi_find_prev_char(zle: &mut Zle) {
    // TODO: implement vi find prev char
    let _ = zle;
}

fn widget_vi_find_next_char_skip(zle: &mut Zle) {
    // TODO: implement vi find next char skip
    let _ = zle;
}

fn widget_vi_find_prev_char_skip(zle: &mut Zle) {
    // TODO: implement vi find prev char skip
    let _ = zle;
}

fn widget_vi_repeat_find(zle: &mut Zle) {
    // TODO: implement vi repeat find
    let _ = zle;
}

fn widget_vi_rev_repeat_find(zle: &mut Zle) {
    // TODO: implement vi reverse repeat find
    let _ = zle;
}

fn widget_vi_history_search_forward(zle: &mut Zle) {
    // TODO: implement vi history search
    let _ = zle;
}

fn widget_vi_history_search_backward(zle: &mut Zle) {
    // TODO: implement vi history search
    let _ = zle;
}

fn widget_vi_repeat_search(zle: &mut Zle) {
    // TODO: implement vi repeat search
    let _ = zle;
}

fn widget_vi_rev_repeat_search(zle: &mut Zle) {
    // TODO: implement vi reverse repeat search
    let _ = zle;
}

fn widget_vi_fetch_history(zle: &mut Zle) {
    // TODO: implement vi fetch history
    let _ = zle;
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

/// Check if a character is a word character
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
