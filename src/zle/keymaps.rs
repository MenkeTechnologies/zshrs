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

    /// Save current state for undo
    pub fn save_undo(&mut self) {
        self.undo_history.push((self.buffer.clone(), self.cursor));
        if self.undo_history.len() > 100 {
            self.undo_history.remove(0);
        }
    }

    /// Undo last change
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

    /// Redo last undone change
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

    /// Add text to kill ring
    pub fn kill_add(&mut self, text: &str) {
        self.kill_ring.push_front(text.to_string());
        if self.kill_ring.len() > self.kill_ring_max {
            self.kill_ring.pop_back();
        }
    }

    /// Yank from kill ring
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

    /// Yank-pop: replace last yank with next kill ring entry
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

    /// Get text from kill ring (without inserting)
    pub fn kill_yank(&self) -> Option<&str> {
        self.kill_ring.front().map(|s| s.as_str())
    }

    /// Rotate kill ring
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
    user_widgets: HashMap<String, String>,
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

    /// Get a widget by name (returns the function name if user-defined)
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

    /// Bind a key in a keymap
    pub fn bind_key(&mut self, keymap: KeymapName, key: &str, widget: &str) {
        if let Some(km) = self.keymaps.get_mut(&keymap) {
            km.bind(key, widget);
        }
    }

    /// Unbind a key from a keymap
    pub fn unbind_key(&mut self, keymap: KeymapName, key: &str) {
        if let Some(km) = self.keymaps.get_mut(&keymap) {
            km.unbind(key);
        }
    }

    /// Execute a widget (stub - actual execution handled elsewhere)
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

    /// List all widget names
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
        ZleGuard(unsafe { std::mem::transmute(m.borrow_mut()) })
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

    /// Create default emacs keymap
    pub fn emacs_default() -> Self {
        let mut km = Self::new();

        // Movement
        km.bind("^F", "forward-char");
        km.bind("^B", "backward-char");
        km.bind("^A", "beginning-of-line");
        km.bind("^E", "end-of-line");
        km.bind("\\ef", "forward-word"); // Alt-f
        km.bind("\\eb", "backward-word"); // Alt-b

        // Editing
        km.bind("^D", "delete-char");
        km.bind("^H", "backward-delete-char");
        km.bind("^?", "backward-delete-char"); // Backspace
        km.bind("^K", "kill-line");
        km.bind("^U", "backward-kill-line");
        km.bind("\\ed", "kill-word"); // Alt-d
        km.bind("\\e^?", "backward-kill-word"); // Alt-Backspace
        km.bind("^W", "backward-kill-word");
        km.bind("^Y", "yank");
        km.bind("\\ey", "yank-pop"); // Alt-y

        // Undo
        km.bind("^_", "undo");
        km.bind("^X^U", "undo");
        km.bind("\\e_", "redo"); // Alt-_

        // History
        km.bind("^P", "up-line-or-history");
        km.bind("^N", "down-line-or-history");
        km.bind("\\e<", "beginning-of-history");
        km.bind("\\e>", "end-of-history");
        km.bind("^R", "history-incremental-search-backward");
        km.bind("^S", "history-incremental-search-forward");

        // Completion
        km.bind("^I", "expand-or-complete"); // Tab
        km.bind("\\e\\e", "complete-word");

        // Accept/misc
        km.bind("^J", "accept-line"); // Enter
        km.bind("^M", "accept-line"); // Enter
        km.bind("^G", "send-break");
        km.bind("^C", "send-break");
        km.bind("^L", "clear-screen");

        // Transpose
        km.bind("^T", "transpose-chars");
        km.bind("\\et", "transpose-words");

        // Case
        km.bind("\\ec", "capitalize-word");
        km.bind("\\el", "down-case-word");
        km.bind("\\eu", "up-case-word");

        // Region
        km.bind("^@", "set-mark-command"); // Ctrl-Space
        km.bind("^X^X", "exchange-point-and-mark");
        km.bind("\\ew", "copy-region-as-kill");

        km
    }

    /// Create default vi insert mode keymap
    pub fn viins_default() -> Self {
        let mut km = Self::new();

        // Enter command mode
        km.bind("^[", "vi-cmd-mode"); // Escape

        // Basic editing (same as emacs)
        km.bind("^H", "backward-delete-char");
        km.bind("^?", "backward-delete-char");
        km.bind("^W", "backward-kill-word");
        km.bind("^U", "backward-kill-line");

        // Accept
        km.bind("^J", "accept-line");
        km.bind("^M", "accept-line");

        // Completion
        km.bind("^I", "expand-or-complete");

        // History
        km.bind("^P", "up-line-or-history");
        km.bind("^N", "down-line-or-history");

        km
    }

    /// Create default vi command mode keymap
    pub fn vicmd_default() -> Self {
        let mut km = Self::new();

        // Enter insert mode
        km.bind("i", "vi-insert");
        km.bind("a", "vi-add-next");
        km.bind("I", "vi-insert-bol");
        km.bind("A", "vi-add-eol");

        // Movement
        km.bind("h", "backward-char");
        km.bind("l", "forward-char");
        km.bind("w", "forward-word");
        km.bind("b", "backward-word");
        km.bind("0", "beginning-of-line");
        km.bind("^", "beginning-of-line");
        km.bind("$", "end-of-line");

        // Delete
        km.bind("x", "delete-char");
        km.bind("X", "backward-delete-char");
        km.bind("dd", "kill-whole-line");
        km.bind("dw", "kill-word");
        km.bind("db", "backward-kill-word");
        km.bind("d$", "kill-line");
        km.bind("d0", "backward-kill-line");

        // Yank/paste
        km.bind("y", "vi-yank");
        km.bind("p", "vi-put-after");
        km.bind("P", "vi-put-before");

        // History
        km.bind("k", "up-line-or-history");
        km.bind("j", "down-line-or-history");
        km.bind("/", "history-incremental-search-backward");
        km.bind("?", "history-incremental-search-forward");
        km.bind("n", "vi-repeat-search");
        km.bind("N", "vi-rev-repeat-search");

        // Undo
        km.bind("u", "undo");
        km.bind("^R", "redo");

        // Accept
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

    /// List all bindings
    pub fn list_bindings(&self) -> impl Iterator<Item = (&str, &str)> {
        self.bindings.iter().map(|(k, v)| (k.as_str(), v.as_str()))
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
}
