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
// Deferred shims — `unimplemented!()` placeholders so the C-source
// fn names remain searchable. Bodies need the `thingytab` HashTable
// substrate (zle_thingy.c:60 createthingytab), Widget free-list,
// the active-editor zlestate (zlecs/zlemetaline/zleline), the
// keymap binding table, the watch_fd table for `zle -F`, and the
// rest of the live ZLE session globals — none of which are ported
// yet. Each panics on call so silent fakes can't escape — per the
// no-shortcuts rule.
// =====================================================================

/// Port of `addzlefunction()` from `Src/Zle/zle_thingy.c:281`.
/// Registers a C-internal widget by name into the thingytab and
/// the widget table.
pub fn addzlefunction() -> i32 {                                             // c:281
    unimplemented!("zle_thingy.rs::addzlefunction — c:281 deferred (thingytab + widget alloc)");
}

/// Port of `bin_zle()` from `Src/Zle/zle_thingy.c:343`. Top-level
/// `zle` builtin dispatcher — switches on the first option char
/// (`-l`, `-N`, `-A`, `-D`, `-K`, `-U`, `-R`, `-M`, etc.) into the
/// per-mode bin_zle_* below.
pub fn bin_zle() -> i32 {                                                    // c:343
    unimplemented!("zle_thingy.rs::bin_zle — c:343 deferred (Options + bin_zle_* dispatch table)");
}

/// Port of `bin_zle_call()` from `Src/Zle/zle_thingy.c:703`.
/// `zle widget args...` invocation from inside another widget.
pub fn bin_zle_call() -> i32 {                                               // c:703
    unimplemented!("zle_thingy.rs::bin_zle_call — c:703 deferred (execzlefunc + zle session state)");
}

/// Port of `bin_zle_complete()` from `Src/Zle/zle_thingy.c:600`.
/// `zle -C name comp-widget func` — register completion widget.
pub fn bin_zle_complete() -> i32 {                                           // c:600
    unimplemented!("zle_thingy.rs::bin_zle_complete — c:600 deferred (Widget WIDGET_NCOMP path + thingytab)");
}

/// Port of `bin_zle_del()` from `Src/Zle/zle_thingy.c:548`.
/// `zle -D widget` — delete a widget binding.
pub fn bin_zle_del() -> i32 {                                                // c:548
    unimplemented!("zle_thingy.rs::bin_zle_del — c:548 deferred (deletezlefunction + thingytab.removenode)");
}

/// Port of `bin_zle_fd()` from `Src/Zle/zle_thingy.c:857`.
/// `zle -F fd handler` — register an fd watcher invoked when fd
/// becomes readable while the editor is idle.
pub fn bin_zle_fd() -> i32 {                                                 // c:857
    unimplemented!("zle_thingy.rs::bin_zle_fd — c:857 deferred (watch_fd table + zle main-loop poll integration)");
}

/// Port of `bin_zle_flags()` from `Src/Zle/zle_thingy.c:651`.
/// `zle -f flag...` — toggle ZLE_KILL / ZLE_KEEPSUFFIX / etc. on
/// the current widget.
pub fn bin_zle_flags() -> i32 {                                              // c:651
    unimplemented!("zle_thingy.rs::bin_zle_flags — c:651 deferred (active-widget pointer + ZLE_* flag mask)");
}

/// Port of `bin_zle_invalidate()` from `Src/Zle/zle_thingy.c:830`.
/// `zle -I` — invalidate the display so the next refresh fully
/// redraws (used after external `print` to terminal from a widget).
pub fn bin_zle_invalidate() -> i32 {                                         // c:830
    unimplemented!("zle_thingy.rs::bin_zle_invalidate — c:830 deferred (zle_refresh trashzle/clearflag globals)");
}

/// Port of `bin_zle_keymap()` from `Src/Zle/zle_thingy.c:488`.
/// `zle -K keymap` — switch the current keymap.
pub fn bin_zle_keymap() -> i32 {                                             // c:488
    unimplemented!("zle_thingy.rs::bin_zle_keymap — c:488 deferred (curkeymap + keymapname globals)");
}

/// Port of `bin_zle_link()` from `Src/Zle/zle_thingy.c:567`.
/// `zle -L old new` — alias one widget name to another.
pub fn bin_zle_link() -> i32 {                                               // c:567
    unimplemented!("zle_thingy.rs::bin_zle_link — c:567 deferred (rthingy + bindwidget alias path)");
}

/// Port of `bin_zle_list()` from `Src/Zle/zle_thingy.c:393`.
/// `zle -l` — print all widget bindings.
pub fn bin_zle_list() -> i32 {                                               // c:393
    unimplemented!("zle_thingy.rs::bin_zle_list — c:393 deferred (thingytab.scantab + scanlistwidgets)");
}

/// Port of `bin_zle_mesg()` from `Src/Zle/zle_thingy.c:459`.
/// `zle -M message` — display a transient message during widget run.
pub fn bin_zle_mesg() -> i32 {                                               // c:459
    unimplemented!("zle_thingy.rs::bin_zle_mesg — c:459 deferred (statusline buffer + zle_refresh hook)");
}

/// Port of `bin_zle_new()` from `Src/Zle/zle_thingy.c:584`.
/// `zle -N name [func]` — bind a user-defined widget to a shfunc.
pub fn bin_zle_new() -> i32 {                                                // c:584
    unimplemented!("zle_thingy.rs::bin_zle_new — c:584 deferred (Widget WIDGET_INT clear + addzlefunction)");
}

/// Port of `bin_zle_refresh()` from `Src/Zle/zle_thingy.c:418`.
/// `zle -R` — force the display to refresh now.
pub fn bin_zle_refresh() -> i32 {                                            // c:418
    unimplemented!("zle_thingy.rs::bin_zle_refresh — c:418 deferred (zrefresh() draw call)");
}

/// Port of `bin_zle_transform()` from `Src/Zle/zle_thingy.c:955`.
/// `zle -T tcfn fn` — install pre-display transformation hook.
pub fn bin_zle_transform() -> i32 {                                          // c:955
    unimplemented!("zle_thingy.rs::bin_zle_transform — c:955 deferred (transformations table + redisplay hook)");
}

/// Port of `bin_zle_unget()` from `Src/Zle/zle_thingy.c:473`.
/// `zle -U str` — push back a string to be re-read as input.
pub fn bin_zle_unget() -> i32 {                                              // c:473
    unimplemented!("zle_thingy.rs::bin_zle_unget — c:473 deferred (ungetbytes/ungetkeys input pushback)");
}

/// Port of `bindwidget()` from `Src/Zle/zle_thingy.c:199`.
/// Associate a Thingy with a Widget (consumes a widget reference).
pub fn bindwidget() -> i32 {                                                 // c:199
    unimplemented!("zle_thingy.rs::bindwidget — c:199 deferred (Thingy.widget assignment + samew circular list)");
}

/// Port of `createthingytab()` from `Src/Zle/zle_thingy.c:60`.
/// Allocate the global `thingytab` HashTable.
pub fn createthingytab() -> i32 {                                            // c:60
    unimplemented!("zle_thingy.rs::createthingytab — c:60 deferred (HashTable substrate)");
}

/// Port of `deletezlefunction()` from `Src/Zle/zle_thingy.c:310`.
/// Remove a widget by Thingy reference, freeing its storage.
pub fn deletezlefunction() -> i32 {                                          // c:310
    unimplemented!("zle_thingy.rs::deletezlefunction — c:310 deferred (thingytab + freewidget cascade)");
}

/// Port of `emptythingytab()` from `Src/Zle/zle_thingy.c:80`.
/// Clear all entries from the thingytab during teardown.
pub fn emptythingytab() -> i32 {                                             // c:80
    unimplemented!("zle_thingy.rs::emptythingytab — c:80 deferred (thingytab.scantab + scanemptythingies)");
}

/// Port of `freethingynode()` from `Src/Zle/zle_thingy.c:118`.
/// HashTable free-node callback for thingies.
pub fn freethingynode() -> i32 {                                             // c:118
    unimplemented!("zle_thingy.rs::freethingynode — c:118 deferred (HashNode free + Thingy storage drop)");
}

/// Port of `freewidget()` from `Src/Zle/zle_thingy.c:257`.
/// Drop a Widget when the last Thingy referring to it is freed.
pub fn freewidget() -> i32 {                                                 // c:257
    unimplemented!("zle_thingy.rs::freewidget — c:257 deferred (Widget storage drop + comp-widget union cleanup)");
}

/// Port of `init_thingies()` from `Src/Zle/zle_thingy.c:1022`.
/// Boot-time thingytab population from the built-in widget table.
pub fn init_thingies() -> i32 {                                              // c:1022
    unimplemented!("zle_thingy.rs::init_thingies — c:1022 deferred (built-in widget table iteration)");
}

/// Port of `makethingynode()` from `Src/Zle/zle_thingy.c:108`.
/// Allocate a fresh Thingy struct (HashTable createnode callback).
pub fn makethingynode() -> i32 {                                             // c:108
    unimplemented!("zle_thingy.rs::makethingynode — c:108 deferred (HashTable createnode signature)");
}

/// Port of `refthingy()` from `Src/Zle/zle_thingy.c:138`.
/// Bump a Thingy's reference count.
pub fn refthingy() -> i32 {                                                  // c:138
    unimplemented!("zle_thingy.rs::refthingy — c:138 deferred (Thingy storage handle)");
}

/// Port of `rthingy()` from `Src/Zle/zle_thingy.c:158`.
/// Look up a Thingy by name; create-and-bind-to-empty-widget if absent.
pub fn rthingy() -> i32 {                                                    // c:158
    unimplemented!("zle_thingy.rs::rthingy — c:158 deferred (thingytab.gettab + makethingynode + insert)");
}

/// Port of `rthingy_nocreate()` from `Src/Zle/zle_thingy.c:169`.
/// Look up a Thingy by name; return NULL if absent (no creation).
pub fn rthingy_nocreate() -> i32 {                                           // c:169
    unimplemented!("zle_thingy.rs::rthingy_nocreate — c:169 deferred (thingytab.gettab read-only)");
}

/// Port of `scanemptythingies()` from `Src/Zle/zle_thingy.c:96`.
/// HashTable scancallback used by `emptythingytab` to drop each node.
pub fn scanemptythingies() -> i32 {                                          // c:96
    unimplemented!("zle_thingy.rs::scanemptythingies — c:96 deferred (HashTable scancallback signature)");
}

/// Port of `scanlistwidgets()` from `Src/Zle/zle_thingy.c:505`.
/// HashTable scancallback used by `bin_zle_list` to print each node.
pub fn scanlistwidgets() -> i32 {                                            // c:505
    unimplemented!("zle_thingy.rs::scanlistwidgets — c:505 deferred (HashTable scancallback + Widget pretty-print)");
}

/// Port of `unbindwidget()` from `Src/Zle/zle_thingy.c:230`.
/// Detach a Thingy from its Widget, optionally overriding immortal flag.
pub fn unbindwidget() -> i32 {                                               // c:230
    unimplemented!("zle_thingy.rs::unbindwidget — c:230 deferred (Thingy.samew circular-list edit)");
}

/// Port of `unrefthingy()` from `Src/Zle/zle_thingy.c:147`.
/// Drop a Thingy reference; free if rc reaches 0.
pub fn unrefthingy() -> i32 {                                                // c:147
    unimplemented!("zle_thingy.rs::unrefthingy — c:147 deferred (Thingy storage handle + freewidget cascade)");
}

/// Port of `zle_usable()` from `Src/Zle/zle_thingy.c:634`.
/// True iff a ZLE session is currently active (i.e., we're inside
/// `zleread`).
pub fn zle_usable() -> i32 {                                                 // c:634
    unimplemented!("zle_thingy.rs::zle_usable — c:634 deferred (zle_active global / sigtrapped state)");
}
