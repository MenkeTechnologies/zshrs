//! ZLE thingies - named bindings to widgets
//!
//! Direct port from zsh/Src/Zle/zle_keymap.c thingy structures
//!
//! A "thingy" is a named entity that refers to a widget. Multiple thingies
//! can refer to the same widget. Thingies are reference-counted.

use std::sync::Arc;

use super::widget::Widget;

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
    /// Create a new thingy with the given name
    pub fn new(name: &str) -> Self {
        Thingy {
            name: name.to_string(),
            flags: ThingyFlags::default(),
            rc: 1,
            widget: None,
        }
    }

    /// Create a builtin thingy (references a builtin widget)
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

    /// Create a user-defined thingy
    pub fn user_defined(name: &str, func_name: &str) -> Self {
        let widget = Widget::user_defined(name, func_name);
        Thingy {
            name: name.to_string(),
            flags: ThingyFlags::default(),
            rc: 1,
            widget: Some(Arc::new(widget)),
        }
    }

    /// Check if this thingy is a specific named widget
    pub fn is(&self, name: &str) -> bool {
        self.name == name
    }

    /// Check if this thingy is a specific widget or its dot-prefixed variant
    /// (Used for checking against both "widget" and ".widget")
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
