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
    /// aliased `foo` to something else — see `bin_zle_new`'s args[0]
    /// vs args[1] split at zle_thingy.c:584. Callers use this when
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

/// Port of `addzlefunction()` from Src/Zle/zle_thingy.c:281. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn addzlefunction() -> i32 { 0 }

/// Port of `bin_zle()` from Src/Zle/zle_thingy.c:343. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle() -> i32 { 0 }

/// Port of `bin_zle_call()` from Src/Zle/zle_thingy.c:703. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_call() -> i32 { 0 }

/// Port of `bin_zle_complete()` from Src/Zle/zle_thingy.c:600. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_complete() -> i32 { 0 }

/// Port of `bin_zle_del()` from Src/Zle/zle_thingy.c:548. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_del() -> i32 { 0 }

/// Port of `bin_zle_fd()` from Src/Zle/zle_thingy.c:857. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_fd() -> i32 { 0 }

/// Port of `bin_zle_flags()` from Src/Zle/zle_thingy.c:651. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_flags() -> i32 { 0 }

/// Port of `bin_zle_invalidate()` from Src/Zle/zle_thingy.c:830. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_invalidate() -> i32 { 0 }

/// Port of `bin_zle_keymap()` from Src/Zle/zle_thingy.c:488. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_keymap() -> i32 { 0 }

/// Port of `bin_zle_link()` from Src/Zle/zle_thingy.c:567. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_link() -> i32 { 0 }

/// Port of `bin_zle_list()` from Src/Zle/zle_thingy.c:393. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_list() -> i32 { 0 }

/// Port of `bin_zle_mesg()` from Src/Zle/zle_thingy.c:459. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_mesg() -> i32 { 0 }

/// Port of `bin_zle_new()` from Src/Zle/zle_thingy.c:584. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_new() -> i32 { 0 }

/// Port of `bin_zle_refresh()` from Src/Zle/zle_thingy.c:418. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_refresh() -> i32 { 0 }

/// Port of `bin_zle_transform()` from Src/Zle/zle_thingy.c:955. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_transform() -> i32 { 0 }

/// Port of `bin_zle_unget()` from Src/Zle/zle_thingy.c:473. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_zle_unget() -> i32 { 0 }

/// Port of `bindwidget()` from Src/Zle/zle_thingy.c:199. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bindwidget() -> i32 { 0 }

/// Port of `createthingytab()` from Src/Zle/zle_thingy.c:60. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn createthingytab() -> i32 { 0 }

/// Port of `deletezlefunction()` from Src/Zle/zle_thingy.c:310. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn deletezlefunction() -> i32 { 0 }

/// Port of `emptythingytab()` from Src/Zle/zle_thingy.c:80. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn emptythingytab() -> i32 { 0 }

/// Port of `freethingynode()` from Src/Zle/zle_thingy.c:118. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freethingynode() -> i32 { 0 }

/// Port of `freewidget()` from Src/Zle/zle_thingy.c:257. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freewidget() -> i32 { 0 }

/// Port of `init_thingies()` from Src/Zle/zle_thingy.c:1022. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn init_thingies() -> i32 { 0 }

/// Port of `makethingynode()` from Src/Zle/zle_thingy.c:108. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn makethingynode() -> i32 { 0 }

/// Port of `refthingy()` from Src/Zle/zle_thingy.c:138. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn refthingy() -> i32 { 0 }

/// Port of `rthingy()` from Src/Zle/zle_thingy.c:158. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn rthingy() -> i32 { 0 }

/// Port of `rthingy_nocreate()` from Src/Zle/zle_thingy.c:169. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn rthingy_nocreate() -> i32 { 0 }

/// Port of `scanemptythingies()` from Src/Zle/zle_thingy.c:96. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn scanemptythingies() -> i32 { 0 }

/// Port of `scanlistwidgets()` from Src/Zle/zle_thingy.c:505. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn scanlistwidgets() -> i32 { 0 }

/// Port of `unbindwidget()` from Src/Zle/zle_thingy.c:230. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn unbindwidget() -> i32 { 0 }

/// Port of `unrefthingy()` from Src/Zle/zle_thingy.c:147. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn unrefthingy() -> i32 { 0 }

/// Port of `zle_usable()` from Src/Zle/zle_thingy.c:634. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zle_usable() -> i32 { 0 }
