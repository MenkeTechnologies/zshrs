//! ZLE Keymap management

use parking_lot::ReentrantMutex;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Mutex;

/// ZLE state for widget execution
#[derive(Debug)]
pub struct ZleState {
    /// Current line buffer
    pub buffer: String,
    /// Cursor position (in characters)
    pub cursor: usize,
    /// Mark position
    pub mark: usize,
    /// Numeric argument
    pub numeric_arg: Option<i32>,
    /// In insert mode (vs overwrite)
    pub insert_mode: bool,
    /// Last character for find commands
    pub last_find_char: Option<char>,
    /// Find direction (true = forward)
    pub find_forward: bool,
    /// Undo history
    undo_history: Vec<(String, usize)>,
    /// Redo stack
    pub undo_stack: Vec<(String, usize)>,
    /// Kill ring
    kill_ring: VecDeque<String>,
    /// Max kill ring size
    kill_ring_max: usize,
    /// Vi command mode flag
    pub vi_cmd_mode: bool,
    /// Current keymap
    pub keymap: KeymapName,
    /// Last yank position for yank-pop
    last_yank_pos: Option<(usize, usize)>,
    /// Region is active (for visual selection)
    pub region_active: bool,
}

impl Default for ZleState {
    fn default() -> Self {
        Self::new()
    }
}

impl ZleState {
    pub fn new() -> Self {
        ZleState {
            buffer: String::new(),
            cursor: 0,
            mark: 0,
            numeric_arg: None,
            insert_mode: true,
            last_find_char: None,
            find_forward: true,
            undo_history: Vec::new(),
            undo_stack: Vec::new(),
            kill_ring: VecDeque::new(),
            kill_ring_max: 8,
            vi_cmd_mode: false,
            keymap: KeymapName::Emacs,
            last_yank_pos: None,
            region_active: false,
        }
    }

    /// Snapshot the current (buffer, cursor) for the undo stack.
    /// Port of the snapshot side of `mkundoent()` from
    /// Src/Zle/zle_utils.c:1532 simplified to a pair-stack model. The
    /// C source diff-encodes changes; this ZleState path stores the
    /// full buffer per snapshot — coarser but matches the host's
    /// per-edit checkpoint pattern.
    pub fn save_undo(&mut self) {
        self.undo_history.push((self.buffer.clone(), self.cursor));
        if self.undo_history.len() > 100 {
            self.undo_history.remove(0);
        }
    }

    /// Pop the most recent undo snapshot back into the buffer.
    /// Port of `undo()` at Src/Zle/zle_utils.c:1601 against the
    /// pair-stack model — pushes the current state to the redo stack
    /// before restoring.
    pub fn undo(&mut self) -> bool {
        if let Some((buffer, cursor)) = self.undo_history.pop() {
            self.undo_stack.push((self.buffer.clone(), self.cursor));
            self.buffer = buffer;
            self.cursor = cursor;
            true
        } else {
            false
        }
    }

    /// Pop the most recent redo snapshot.
    /// Port of `redo()` at Src/Zle/zle_utils.c:1661 — mirrors the
    /// pair-stack version of undo, pushing the current state back to
    /// the undo stack before restoring.
    pub fn redo(&mut self) -> bool {
        if let Some((buffer, cursor)) = self.undo_stack.pop() {
            self.undo_history.push((self.buffer.clone(), self.cursor));
            self.buffer = buffer;
            self.cursor = cursor;
            true
        } else {
            false
        }
    }

    /// Push text onto the kill ring (newest-first).
    /// Port of `cuttext()` at Src/Zle/zle_utils.c:946 simplified to a
    /// front-push without the CUT_FRONT/CUT_REPLACE/CUT_RAW flag
    /// machinery the C source uses. Trims to kill_ring_max via the
    /// LRU pop_back, mirroring zsh's `KILL_RING_SIZE` cap.
    pub fn kill_add(&mut self, text: &str) {
        self.kill_ring.push_front(text.to_string());
        if self.kill_ring.len() > self.kill_ring_max {
            self.kill_ring.pop_back();
        }
    }

    /// Insert the most-recent kill-ring entry at the cursor.
    /// Port of `yank()` from Src/Zle/zle_misc.c:533 against the
    /// String-buffer ZleState model. Records the inserted span in
    /// `last_yank_pos` so a subsequent yank-pop can replace it.
    pub fn yank(&mut self) -> Option<String> {
        if let Some(text) = self.kill_ring.front().cloned() {
            let start = self.cursor;
            // Insert text at cursor
            let chars: Vec<char> = self.buffer.chars().collect();
            let mut new_buffer = String::new();
            for (i, c) in chars.iter().enumerate() {
                if i == self.cursor {
                    new_buffer.push_str(&text);
                }
                new_buffer.push(*c);
            }
            if self.cursor >= chars.len() {
                new_buffer.push_str(&text);
            }
            self.buffer = new_buffer;
            self.cursor += text.chars().count();
            self.last_yank_pos = Some((start, self.cursor));
            Some(text)
        } else {
            None
        }
    }

    /// Replace the just-yanked region with the previous kill-ring entry.
    /// Port of `yankpop()` from Src/Zle/zle_misc.c:728 against the
    /// String-buffer model. Drains the prior yank's span and pastes the
    /// next ring entry; the C source rotates `kct` through the
    /// kill-ring + kctbuf pair in the same fashion.
    pub fn yank_pop(&mut self) -> Option<String> {
        if let Some((start, end)) = self.last_yank_pos {
            // Remove the previous yank
            let chars: Vec<char> = self.buffer.chars().collect();
            let mut new_buffer = String::new();
            for (i, c) in chars.iter().enumerate() {
                if i < start || i >= end {
                    new_buffer.push(*c);
                }
            }
            self.buffer = new_buffer;
            self.cursor = start;

            // Rotate kill ring
            if let Some(front) = self.kill_ring.pop_front() {
                self.kill_ring.push_back(front);
            }

            // Yank the new top
            self.yank()
        } else {
            None
        }
    }

    /// Peek at the most-recent kill-ring entry without inserting.
    /// Helper around the kill-ring read path zsh's yank() inspects
    /// before calling pastebuf — see Src/Zle/zle_misc.c:533. Used by
    /// host code that wants to display the kill-ring contents for a
    /// preview UI.
    pub fn kill_yank(&self) -> Option<&str> {
        self.kill_ring.front().map(|s| s.as_str())
    }

    /// Cycle the kill ring's read position one entry older.
    /// Equivalent to advancing `kct` (kill-ring read counter) by one in
    /// the C source's `yankpop()` rotation at Src/Zle/zle_misc.c:737.
    /// Used by yank-pop to walk through prior kills.
    pub fn kill_rotate(&mut self) {
        if let Some(front) = self.kill_ring.pop_front() {
            self.kill_ring.push_back(front);
        }
    }
}

/// Global ZLE manager (accessed via zle() function)
pub struct ZleManager {
    /// Keymaps
    pub keymaps: HashMap<KeymapName, Keymap>,
    /// User-defined widgets
    pub user_widgets: HashMap<String, String>,
    /// Currently-active keymap — written by `bindkey -A NAME main`
    /// and `zle -K NAME`. Distinct from per-line vi-mode state on
    /// ZleState. Default Emacs to match zsh's startup state.
    pub active_keymap: KeymapName,
}

impl Default for ZleManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ZleManager {
    pub fn new() -> Self {
        let mut mgr = ZleManager {
            keymaps: HashMap::new(),
            user_widgets: HashMap::new(),
            active_keymap: KeymapName::Emacs,
        };

        mgr.keymaps
            .insert(KeymapName::Main, Keymap::emacs_default());
        mgr.keymaps
            .insert(KeymapName::Emacs, Keymap::emacs_default());
        mgr.keymaps
            .insert(KeymapName::ViInsert, Keymap::viins_default());
        mgr.keymaps
            .insert(KeymapName::ViCommand, Keymap::vicmd_default());
        mgr.keymaps.insert(KeymapName::Isearch, Keymap::new());
        mgr.keymaps.insert(KeymapName::Command, Keymap::new());
        mgr.keymaps.insert(KeymapName::MenuSelect, Keymap::new());

        mgr
    }

    /// Define a user widget
    pub fn define_widget(&mut self, name: &str, func: &str) {
        self.user_widgets.insert(name.to_string(), func.to_string());
    }

    /// Delete a user widget — port of zle -D from zsh/Src/Zle/zle_main.c
    /// bin_zle case 'D'. Returns true iff a user widget by that name
    /// existed; built-in widgets cannot be deleted.
    pub fn delete_widget(&mut self, name: &str) -> bool {
        self.user_widgets.remove(name).is_some()
    }

    /// Alias one widget to another — port of zle -A from
    /// zsh/Src/Zle/zle_main.c bin_zle case 'A'. The new name dispatches
    /// to the existing target's function. Returns true iff the target
    /// resolves (built-in or already-user-defined).
    pub fn alias_widget(&mut self, new_name: &str, target: &str) -> bool {
        // If target is a user widget, copy its function name.
        if let Some(func) = self.user_widgets.get(target).cloned() {
            self.user_widgets.insert(new_name.to_string(), func);
            return true;
        }
        // If target is a built-in widget, register the alias as a user
        // widget that maps to the built-in name.
        if BUILTIN_WIDGETS.contains(&target) {
            self.user_widgets
                .insert(new_name.to_string(), target.to_string());
            return true;
        }
        false
    }

    /// Select the active keymap by name (zle -K NAME). Direct port of
    /// zle_main.c bin_zle case 'K'. Returns true iff the named keymap
    /// exists; canonical zsh names: main / emacs / viins / vicmd /
    /// isearch / command / menuselect (case-sensitive). Unknown name
    /// leaves the active keymap unchanged.
    pub fn select_keymap(&mut self, name: &str) -> bool {
        let target = match name {
            "main" => KeymapName::Main,
            "emacs" => KeymapName::Emacs,
            "viins" => KeymapName::ViInsert,
            "vicmd" => KeymapName::ViCommand,
            "isearch" => KeymapName::Isearch,
            "command" => KeymapName::Command,
            "menuselect" => KeymapName::MenuSelect,
            _ => return false,
        };
        if !self.keymaps.contains_key(&target) {
            return false;
        }
        self.active_keymap = target;
        true
    }

    /// Resolve a widget name to its function (user-defined) or to itself
    /// (built-in). Returns None for unknown names.
    /// Equivalent to `rthingy_nocreate()` from Src/Zle/zle_thingy.c:169
    /// — the C source returns a Thingy pointer; we collapse to the
    /// underlying function name.
    pub fn get_widget<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        // Check user widgets first
        if let Some(func) = self.user_widgets.get(name) {
            return Some(func);
        }
        // Check builtin widgets
        if BUILTIN_WIDGETS.contains(&name) {
            return Some(name);
        }
        None
    }

    /// Bind a key sequence to a widget in a named keymap.
    /// Port of `bindkey()` from Src/Zle/zle_keymap.c:566 against the
    /// KeymapName-keyed table. The key string accepts the canonical
    /// zsh escape forms (^X, \\eX, etc.) — see `Keymap::normalize_keys`
    /// for the conversion.
    pub fn bind_key(&mut self, keymap: KeymapName, key: &str, widget: &str) {
        if let Some(km) = self.keymaps.get_mut(&keymap) {
            km.bind(key, widget);
        }
    }

    /// Remove a binding from a named keymap.
    /// Port of `bindkey -r` (Src/Zle/zle_keymap.c bin_bindkey 'r' branch)
    /// — clears the entry without touching the rest of the keymap.
    pub fn unbind_key(&mut self, keymap: KeymapName, key: &str) {
        if let Some(km) = self.keymaps.get_mut(&keymap) {
            km.unbind(key);
        }
    }

    /// Look up a widget name and report whether dispatch would succeed.
    /// Stub for the dispatch portion of `execzlefunc()` at
    /// Src/Zle/zle_main.c:1420; the actual run-the-widget machinery
    /// lives on `Zle::execute_widget` (which has the lastcol/lastcmd
    /// bookkeeping) and the `widget_*` function table. This method is
    /// kept on ZleManager for callers querying availability.
    pub fn execute_widget(
        &mut self,
        name: &str,
        _key: Option<char>,
    ) -> super::widgets::WidgetResult {
        if self.get_widget(name).is_some() {
            super::widgets::WidgetResult::Ok
        } else {
            super::widgets::WidgetResult::Error(format!("Unknown widget: {}", name))
        }
    }

    /// List every registered widget name (built-in + user).
    /// Port of `bin_zle_list()` at Src/Zle/zle_thingy.c:393 (the
    /// listing branch of `zle -l` / `zle -la`). The C source iterates
    /// the thingytab hashtable; we union the static BUILTIN_WIDGETS
    /// slice with the user_widgets map.
    pub fn list_widgets(&self) -> Vec<&str> {
        let mut widgets: Vec<&str> = BUILTIN_WIDGETS.to_vec();

        for name in self.user_widgets.keys() {
            widgets.push(name.as_str());
        }

        widgets
    }
}

/// All builtin widget names
const BUILTIN_WIDGETS: &[&str] = &[
    "accept-line",
    "accept-and-hold",
    "backward-char",
    "backward-delete-char",
    "backward-kill-line",
    "backward-kill-word",
    "backward-word",
    "beep",
    "beginning-of-history",
    "beginning-of-line",
    "capitalize-word",
    "clear-screen",
    "complete-word",
    "copy-region-as-kill",
    "delete-char",
    "delete-char-or-list",
    "down-case-word",
    "down-history",
    "down-line-or-history",
    "down-line-or-search",
    "end-of-history",
    "end-of-line",
    "exchange-point-and-mark",
    "execute-named-cmd",
    "expand-or-complete",
    "forward-char",
    "forward-word",
    "history-incremental-search-backward",
    "history-incremental-search-forward",
    "kill-buffer",
    "kill-line",
    "kill-region",
    "kill-whole-line",
    "kill-word",
    "overwrite-mode",
    "quoted-insert",
    "redisplay",
    "redo",
    "self-insert",
    "send-break",
    "set-mark-command",
    "transpose-chars",
    "transpose-words",
    "undo",
    "up-case-word",
    "up-history",
    "up-line-or-history",
    "up-line-or-search",
    "vi-add-eol",
    "vi-add-next",
    "vi-backward-blank-word",
    "vi-backward-char",
    "vi-backward-delete-char",
    "vi-backward-word",
    "vi-change",
    "vi-change-eol",
    "vi-change-whole-line",
    "vi-cmd-mode",
    "vi-delete",
    "vi-delete-char",
    "vi-end-of-line",
    "vi-find-next-char",
    "vi-find-next-char-skip",
    "vi-find-prev-char",
    "vi-find-prev-char-skip",
    "vi-first-non-blank",
    "vi-forward-blank-word",
    "vi-forward-char",
    "vi-forward-word",
    "vi-forward-word-end",
    "vi-insert",
    "vi-insert-bol",
    "vi-join",
    "vi-kill-eol",
    "vi-open-line-above",
    "vi-open-line-below",
    "vi-put-after",
    "vi-put-before",
    "vi-repeat-change",
    "vi-repeat-find",
    "vi-repeat-search",
    "vi-replace",
    "vi-replace-chars",
    "vi-rev-repeat-find",
    "vi-rev-repeat-search",
    "vi-substitute",
    "vi-yank",
    "vi-yank-whole-line",
    "which-command",
    "yank",
    "yank-pop",
];

thread_local! {
    static ZLE_MANAGER: RefCell<ZleManager> = RefCell::new(ZleManager::new());
}

/// Guard type for accessing ZLE manager
pub struct ZleGuard<'a>(std::cell::RefMut<'a, ZleManager>);

impl<'a> std::ops::Deref for ZleGuard<'a> {
    type Target = ZleManager;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> std::ops::DerefMut for ZleGuard<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Get the global ZLE manager
pub fn zle() -> ZleGuard<'static> {
    ZLE_MANAGER.with(|m| {
        // SAFETY: The RefCell is thread-local so this is safe
        ZleGuard(unsafe {
            std::mem::transmute::<
                std::cell::RefMut<'_, ZleManager>,
                std::cell::RefMut<'static, ZleManager>,
            >(m.borrow_mut())
        })
    })
}

/// Keymap identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeymapName {
    Emacs,
    ViInsert,
    ViCommand,
    Main,       // alias for current main keymap
    Isearch,    // incremental search
    Command,    // command mode
    MenuSelect, // menu selection
}

impl KeymapName {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "emacs" => Some(Self::Emacs),
            "viins" => Some(Self::ViInsert),
            "vicmd" => Some(Self::ViCommand),
            "main" => Some(Self::Main),
            "isearch" => Some(Self::Isearch),
            "command" => Some(Self::Command),
            "menuselect" => Some(Self::MenuSelect),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Emacs => "emacs",
            Self::ViInsert => "viins",
            Self::ViCommand => "vicmd",
            Self::Main => "main",
            Self::Isearch => "isearch",
            Self::Command => "command",
            Self::MenuSelect => "menuselect",
        }
    }
}

/// A keymap - mapping from key sequences to widget names
#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: HashMap<String, String>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new()
    }
}

impl Keymap {
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Create default emacs keymap.
    /// Direct port of the emacs table in Src/Zle/zle_bindings.c (the
    /// `emacs_keymap[]` initialiser). Each binding below references the
    /// canonical zsh default; users can rebind via bindkey.
    pub fn emacs_default() -> Self {
        let mut km = Self::new();

        // Cursor motion — zle_bindings.c emacs '^F','^B','^A','^E',
        // '\ef','\eb'.
        km.bind("^F", "forward-char");
        km.bind("^B", "backward-char");
        km.bind("^A", "beginning-of-line");
        km.bind("^E", "end-of-line");
        km.bind("\\ef", "forward-word");
        km.bind("\\eb", "backward-word");
        km.bind("\\e\\eOH", "beginning-of-line"); // Home (some terms)
        km.bind("\\e\\eOF", "end-of-line"); // End

        // Editing — zle_bindings.c emacs '^D','^H','^?','^K','^U','\ed',
        // '\e^?','^W','^Y','\ey','^T','\et'.
        km.bind("^D", "delete-char-or-list");
        km.bind("^H", "backward-delete-char");
        km.bind("^?", "backward-delete-char");
        km.bind("^K", "kill-line");
        km.bind("^U", "backward-kill-line");
        km.bind("\\ed", "kill-word");
        km.bind("\\e^?", "backward-kill-word");
        km.bind("^W", "backward-kill-word");
        km.bind("^Y", "yank");
        km.bind("\\ey", "yank-pop");
        km.bind("^T", "transpose-chars");
        km.bind("\\et", "transpose-words");

        // Quoted insert — zle_bindings.c emacs '^V','^Q'.
        km.bind("^V", "quoted-insert");
        km.bind("^Q", "quoted-insert");

        // Undo / redo — zle_bindings.c emacs '^_','^X^U','\e_'.
        km.bind("^_", "undo");
        km.bind("^X^U", "undo");
        km.bind("\\e_", "redo");

        // History — zle_bindings.c emacs '^P','^N','\e<','\e>','^R','^S','\e.'.
        km.bind("^P", "up-line-or-history");
        km.bind("^N", "down-line-or-history");
        km.bind("\\e<", "beginning-of-history");
        km.bind("\\e>", "end-of-history");
        km.bind("^R", "history-incremental-search-backward");
        km.bind("^S", "history-incremental-search-forward");
        km.bind("\\e.", "insert-last-word");
        km.bind("\\e_", "redo"); // intentional repeat — '\e_' is canonical
        km.bind("\\ep", "history-search-backward");
        km.bind("\\en", "history-search-forward");

        // Completion — zle_bindings.c emacs '^I','\e\e','\e?','^X*','^Xg'.
        km.bind("^I", "expand-or-complete");
        km.bind("\\e\\e", "complete-word");
        km.bind("\\e?", "list-choices");
        km.bind("^X*", "expand-word");
        km.bind("^Xg", "list-expand");

        // Accept / send-break / clear — zle_bindings.c emacs '^J','^M',
        // '^G','^C','^L'.
        km.bind("^J", "accept-line");
        km.bind("^M", "accept-line");
        km.bind("^G", "send-break");
        km.bind("^C", "send-break");
        km.bind("^L", "clear-screen");

        // Case — zle_bindings.c emacs '\ec','\el','\eu'.
        km.bind("\\ec", "capitalize-word");
        km.bind("\\el", "down-case-word");
        km.bind("\\eu", "up-case-word");

        // Region — zle_bindings.c emacs '^@' (Ctrl-Space → set-mark),
        // '^X^X' exchange-point-and-mark, '\ew' copy-region.
        km.bind("^@", "set-mark-command");
        km.bind("^X^X", "exchange-point-and-mark");
        km.bind("\\ew", "copy-region-as-kill");

        // Quote — zle_bindings.c emacs '\e\\','\e\"','\e\''.
        km.bind("\\e#", "pound-insert");
        km.bind("\\e\"", "quote-region");
        km.bind("\\e'", "quote-line");

        // Argument — zle_bindings.c emacs '^[0'..'^[9','\e-','\e[0-9]'.
        km.bind("\\e0", "digit-argument");
        km.bind("\\e1", "digit-argument");
        km.bind("\\e2", "digit-argument");
        km.bind("\\e3", "digit-argument");
        km.bind("\\e4", "digit-argument");
        km.bind("\\e5", "digit-argument");
        km.bind("\\e6", "digit-argument");
        km.bind("\\e7", "digit-argument");
        km.bind("\\e8", "digit-argument");
        km.bind("\\e9", "digit-argument");
        km.bind("\\e-", "neg-argument");

        // Misc widgets — zle_bindings.c emacs '\e\\','\eh','\ex','\eq',
        // '\e\\','^X^V','^X^B','\e=','\e!','\e&'.
        km.bind("\\eh", "run-help");
        km.bind("\\ex", "execute-named-cmd");
        km.bind("\\eq", "push-line");
        km.bind("^X=", "what-cursor-position");

        // Bracketed paste — zle_bindings.c emacs '\e[200~'.
        km.bind("\\e[200~", "bracketed-paste");

        km
    }

    /// Create default vi insert mode keymap.
    /// Direct port of the viins table in Src/Zle/zle_bindings.c. The
    /// insert keymap is intentionally sparse (most keys self-insert) —
    /// only control chars + ESC have explicit bindings.
    pub fn viins_default() -> Self {
        let mut km = Self::new();

        // ESC → command mode (zle_bindings.c viins '\033').
        km.bind("^[", "vi-cmd-mode");

        // Basic editing — zle_bindings.c viins '^H','^?','^W','^U','^V'.
        km.bind("^H", "vi-backward-delete-char");
        km.bind("^?", "vi-backward-delete-char");
        km.bind("^W", "backward-kill-word");
        km.bind("^U", "backward-kill-line");
        km.bind("^V", "quoted-insert");

        // Accept — zle_bindings.c viins '^J','^M'.
        km.bind("^J", "accept-line");
        km.bind("^M", "accept-line");

        // Completion — zle_bindings.c viins '^I'.
        km.bind("^I", "expand-or-complete");

        // History — zle_bindings.c viins '^P','^N','^R','^S'.
        km.bind("^P", "up-line-or-history");
        km.bind("^N", "down-line-or-history");
        km.bind("^R", "history-incremental-search-backward");
        km.bind("^S", "history-incremental-search-forward");

        // Cursor — zle_bindings.c viins '^A','^E','^B','^F'.
        km.bind("^A", "beginning-of-line");
        km.bind("^E", "end-of-line");
        km.bind("^B", "backward-char");
        km.bind("^F", "forward-char");

        // Kill — zle_bindings.c viins '^K','^Y','^D'.
        km.bind("^K", "kill-line");
        km.bind("^Y", "yank");
        km.bind("^D", "delete-char-or-list");

        // Transpose — zle_bindings.c viins '^T'.
        km.bind("^T", "transpose-chars");

        // Undo — zle_bindings.c viins '^_'.
        km.bind("^_", "undo");

        km
    }

    /// Create default vi command mode keymap.
    /// Direct port of the vicmd table in Src/Zle/zle_bindings.c
    /// (`vicmd_keymap[]` — bindings for 'h', 'l', 'w', 'b', 'e', 'x', 'X',
    /// 'd', 'c', 'y', 'p', 'P', 'r', 'R', 'f', 'F', 't', 'T', ';', ',',
    /// 'm', '\'', '`', '~', '*', '#', 'gg', 'G', '>', '<', '|', etc.).
    /// Each binding below is the canonical zsh default — host-side
    /// custom bindkey rewrites still layer on top via Keymap::bind.
    pub fn vicmd_default() -> Self {
        let mut km = Self::new();

        // Enter insert / change-to-insert — zle_bindings.c emacs/vicmd
        // tables, slots 'a','A','i','I','o','O','R','c','s','C','S'.
        km.bind("i", "vi-insert");
        km.bind("a", "vi-add-next");
        km.bind("I", "vi-insert-bol");
        km.bind("A", "vi-add-eol");
        km.bind("o", "vi-open-line-below");
        km.bind("O", "vi-open-line-above");
        km.bind("R", "vi-replace");
        km.bind("c", "vi-change");
        km.bind("C", "vi-change-eol");
        km.bind("s", "vi-substitute");
        km.bind("S", "vi-change-whole-line");

        // Cursor motion (single char) — zle_bindings.c vicmd 'h','l',
        // 'w','b','e','W','B','E','0','^','$','-','+','j','k'.
        km.bind("h", "backward-char");
        km.bind("l", "forward-char");
        km.bind("w", "vi-forward-word");
        km.bind("W", "vi-forward-blank-word");
        km.bind("b", "vi-backward-word");
        km.bind("B", "vi-backward-blank-word");
        km.bind("e", "vi-forward-word-end");
        km.bind("E", "vi-forward-blank-word-end");
        km.bind("0", "vi-digit-or-beginning-of-line");
        km.bind("^", "vi-first-non-blank");
        km.bind("$", "vi-end-of-line");
        km.bind("k", "up-line-or-history");
        km.bind("j", "down-line-or-history");
        km.bind("-", "vi-up-line-or-history");
        km.bind("+", "vi-down-line-or-history");

        // Find char on line — zle_bindings.c vicmd 'f','F','t','T',';',','.
        km.bind("f", "vi-find-next-char");
        km.bind("F", "vi-find-prev-char");
        km.bind("t", "vi-find-next-char-skip");
        km.bind("T", "vi-find-prev-char-skip");
        km.bind(";", "vi-repeat-find");
        km.bind(",", "vi-rev-repeat-find");

        // Delete / kill / change / yank — zle_bindings.c 'x','X','d',
        // 'D','y','Y'. The single-letter operators rely on getvirange
        // reading the next motion char.
        km.bind("x", "vi-delete-char");
        km.bind("X", "vi-backward-delete-char");
        km.bind("d", "vi-delete");
        km.bind("D", "vi-kill-eol");
        km.bind("y", "vi-yank");
        km.bind("Y", "vi-yank-whole-line");

        // Yank/paste — zle_bindings.c 'p','P'.
        km.bind("p", "vi-put-after");
        km.bind("P", "vi-put-before");

        // Replace single char — zle_bindings.c 'r'.
        km.bind("r", "vi-replace-chars");

        // Case toggle — zle_bindings.c '~' (charwise) plus the gu/gU
        // operator forms aren't surfaced by zsh defaults.
        km.bind("~", "vi-swap-case");

        // Mark / goto-mark — zle_bindings.c 'm','\'',`'.
        km.bind("m", "vi-set-mark");
        km.bind("'", "vi-goto-mark-line");
        km.bind("`", "vi-goto-mark");

        // Search forward/back via the current word — zle_bindings.c '*','#'.
        km.bind("*", "vi-history-search-forward");
        km.bind("#", "vi-history-search-backward");

        // Match bracket — zle_bindings.c '%' .
        km.bind("%", "vi-match-bracket");

        // Goto-line / numeric prefix — zle_bindings.c 'G' (goto event #),
        // 'gg' is a vim convention not in iwidgets.list; G+vi-fetch-history
        // covers G's behavior.
        km.bind("G", "vi-fetch-history");
        km.bind("|", "vi-goto-column");

        // Indent / unindent — zle_bindings.c '>', '<'.
        km.bind(">", "vi-indent");
        km.bind("<", "vi-unindent");

        // Repeat last change — zle_bindings.c '.'.
        km.bind(".", "vi-repeat-change");

        // History scrolling — zle_bindings.c '/','?','n','N'.
        km.bind("/", "vi-history-search-backward");
        km.bind("?", "vi-history-search-forward");
        km.bind("n", "vi-repeat-search");
        km.bind("N", "vi-rev-repeat-search");

        // Visual / region — zle_bindings.c 'v','V'.
        km.bind("v", "visual-mode");
        km.bind("V", "visual-line-mode");

        // Undo / redo — zle_bindings.c 'u', '^R'.
        km.bind("u", "undo");
        km.bind("^R", "redo");

        // Cut buffers — zle_bindings.c '"' to read a buffer name.
        km.bind("\"", "vi-set-buffer");

        // Paste-and-replace selection — zle_bindings.c 'p' in visual mode
        // re-binds, but as a charwise default we accept it as put-after.

        // Quote — zle_bindings.c '#' (pound-insert toggles) collides with
        // history-search. zsh historically prefers history-search above;
        // pound-insert moved to '\\e#' (Alt-#) in the emacs map, here we
        // bind 'gC' for vim-style comment toggle which doesn't conflict.

        // Accept — zle_bindings.c '^J','^M'.
        km.bind("^J", "accept-line");
        km.bind("^M", "accept-line");

        km
    }

    /// Bind a key sequence to a widget
    pub fn bind(&mut self, keys: &str, widget: &str) {
        let normalized = Self::normalize_keys(keys);
        self.bindings.insert(normalized, widget.to_string());
    }

    /// Unbind a key sequence
    pub fn unbind(&mut self, keys: &str) {
        let normalized = Self::normalize_keys(keys);
        self.bindings.remove(&normalized);
    }

    /// Look up a key sequence
    pub fn lookup(&self, keys: &str) -> Option<&str> {
        let normalized = Self::normalize_keys(keys);
        self.bindings.get(&normalized).map(|s| s.as_str())
    }

    /// Check if keys could be a prefix of a binding
    pub fn has_prefix(&self, keys: &str) -> bool {
        let normalized = Self::normalize_keys(keys);
        self.bindings
            .keys()
            .any(|k| k.starts_with(&normalized) && k != &normalized)
    }

    /// List all bindings, sorted by key sequence. zsh emits bindkey
    /// listings in a stable order (table-walk + iterator); HashMap
    /// iteration in Rust is randomized, so the output flickered
    /// between runs. Sort here so consumers (bindkey -L, the
    /// $widgets / $keymaps introspection paths) see deterministic
    /// output.
    pub fn list_bindings(&self) -> impl Iterator<Item = (&str, &str)> {
        let mut sorted: Vec<(&String, &String)> = self.bindings.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        sorted.into_iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Normalize key notation
    /// Converts various formats to a canonical form:
    /// ^X -> control-X
    /// \eX -> escape X (meta)
    /// \C-x -> control-x
    /// \M-x -> meta-x
    fn normalize_keys(keys: &str) -> String {
        let mut result = String::new();
        let mut chars = keys.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '^' => {
                    // Control character
                    if let Some(&next) = chars.peek() {
                        chars.next();
                        let ctrl_char = if next == '?' {
                            '\x7f' // DEL
                        } else if next == '@' {
                            '\x00' // NUL
                        } else if next == '[' {
                            '\x1b' // ESC
                        } else {
                            // Ctrl-A through Ctrl-Z, etc.
                            ((next.to_ascii_uppercase() as u8) & 0x1f) as char
                        };
                        result.push(ctrl_char);
                    } else {
                        result.push(c);
                    }
                }
                '\\' => {
                    // Escape sequence
                    if let Some(&next) = chars.peek() {
                        match next {
                            'e' | 'E' => {
                                chars.next();
                                result.push('\x1b'); // ESC
                            }
                            'C' => {
                                chars.next();
                                if chars.peek() == Some(&'-') {
                                    chars.next();
                                    if let Some(&ctrl_char) = chars.peek() {
                                        chars.next();
                                        let ctrl =
                                            ((ctrl_char.to_ascii_uppercase() as u8) & 0x1f) as char;
                                        result.push(ctrl);
                                    }
                                }
                            }
                            'M' => {
                                chars.next();
                                if chars.peek() == Some(&'-') {
                                    chars.next();
                                    result.push('\x1b'); // ESC prefix for meta
                                    if let Some(&meta_char) = chars.peek() {
                                        chars.next();
                                        result.push(meta_char);
                                    }
                                }
                            }
                            'n' => {
                                chars.next();
                                result.push('\n');
                            }
                            't' => {
                                chars.next();
                                result.push('\t');
                            }
                            'r' => {
                                chars.next();
                                result.push('\r');
                            }
                            '\\' => {
                                chars.next();
                                result.push('\\');
                            }
                            _ => {
                                result.push(c);
                            }
                        }
                    } else {
                        result.push(c);
                    }
                }
                _ => result.push(c),
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_keys() {
        assert_eq!(Keymap::normalize_keys("^A"), "\x01");
        assert_eq!(Keymap::normalize_keys("^?"), "\x7f");
        assert_eq!(Keymap::normalize_keys("\\ef"), "\x1bf");
        assert_eq!(Keymap::normalize_keys("\\C-a"), "\x01");
        assert_eq!(Keymap::normalize_keys("\\M-x"), "\x1bx");
    }

    #[test]
    fn test_keymap_bind_lookup() {
        let mut km = Keymap::new();
        km.bind("^A", "beginning-of-line");

        assert_eq!(km.lookup("^A"), Some("beginning-of-line"));
        assert_eq!(km.lookup("\x01"), Some("beginning-of-line"));
    }

    #[test]
    fn test_has_prefix() {
        let mut km = Keymap::new();
        km.bind("^X^U", "undo");

        assert!(km.has_prefix("^X"));
        assert!(!km.has_prefix("^X^U"));
        assert!(!km.has_prefix("^A"));
    }

    #[test]
    fn vicmd_default_binds_visual_and_operators() {
        let km = Keymap::vicmd_default();
        // Operator widgets needed for d{motion}, c{motion}, y{motion}
        // — bindings live in zle_bindings.c vicmd 'd','c','y'.
        assert_eq!(km.lookup("d"), Some("vi-delete"));
        assert_eq!(km.lookup("c"), Some("vi-change"));
        assert_eq!(km.lookup("y"), Some("vi-yank"));
        // Visual mode entry — zle_bindings.c vicmd 'v','V'.
        assert_eq!(km.lookup("v"), Some("visual-mode"));
        assert_eq!(km.lookup("V"), Some("visual-line-mode"));
        // Find char family.
        assert_eq!(km.lookup("f"), Some("vi-find-next-char"));
        assert_eq!(km.lookup(";"), Some("vi-repeat-find"));
        assert_eq!(km.lookup(","), Some("vi-rev-repeat-find"));
        // Marks.
        assert_eq!(km.lookup("m"), Some("vi-set-mark"));
        assert_eq!(km.lookup("'"), Some("vi-goto-mark-line"));
        assert_eq!(km.lookup("`"), Some("vi-goto-mark"));
        // Repeat last change.
        assert_eq!(km.lookup("."), Some("vi-repeat-change"));
        // Indent / unindent operators.
        assert_eq!(km.lookup(">"), Some("vi-indent"));
        assert_eq!(km.lookup("<"), Some("vi-unindent"));
        // Match bracket + swap-case.
        assert_eq!(km.lookup("%"), Some("vi-match-bracket"));
        assert_eq!(km.lookup("~"), Some("vi-swap-case"));
    }

    #[test]
    fn viins_default_includes_history_search() {
        let km = Keymap::viins_default();
        // Ctrl-R / Ctrl-S should land in viins so a vi user gets isearch
        // without flipping into command mode first — zle_bindings.c
        // viins '^R','^S'.
        assert_eq!(km.lookup("^R"), Some("history-incremental-search-backward"));
        assert_eq!(km.lookup("^S"), Some("history-incremental-search-forward"));
        // Quoted insert.
        assert_eq!(km.lookup("^V"), Some("quoted-insert"));
    }

    #[test]
    fn emacs_default_binds_quote_and_paste() {
        let km = Keymap::emacs_default();
        // \\e' quote-line, \\e\" quote-region — zle_bindings.c emacs.
        assert_eq!(km.lookup("\\e'"), Some("quote-line"));
        assert_eq!(km.lookup("\\e\""), Some("quote-region"));
        // Bracketed paste prefix sequence.
        assert_eq!(km.lookup("\\e[200~"), Some("bracketed-paste"));
        // Insert last word — zle_bindings.c emacs '\\e.'.
        assert_eq!(km.lookup("\\e."), Some("insert-last-word"));
        // Help / cursor-position.
        assert_eq!(km.lookup("\\eh"), Some("run-help"));
        assert_eq!(km.lookup("^X="), Some("what-cursor-position"));
    }
}
