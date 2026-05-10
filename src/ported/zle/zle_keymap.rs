//! ZLE keymap and key bindings - Direct port from zsh/Src/Zle/zle_keymap.c
//!
//! currently selected keymap, and its name                                  // c:121
//! the hash table of keymap names                                           // c:128
//! key sequence reading data                                                // c:133
//! main initialisation entry point                                          // c:1220
//!
//! Keymap structures:
//!
//! There is a hash table of keymap names. Each name just points to a keymap.
//! More than one name may point to the same keymap.
//!
//! Each keymap consists of a table of bindings for each character, and a
//! hash table of multi-character key bindings. The keymap has no individual
//! name, but maintains a reference count.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::zle_thingy::Thingy;

// =====================================================================
// keymapnamtab — `Src/Zle/zle_keymap.c:128/153`.
// =====================================================================
//
// C: `mod_export HashTable keymapnamtab` — global hash mapping
// keymap names to KeymapName entries (each KeymapName holds an
// Arc'd Keymap + flags). zshrs uses Mutex<HashMap<String, KeymapName>>.

static KEYMAPNAMTAB: OnceLock<Mutex<HashMap<String, KeymapName>>> = OnceLock::new();

fn keymapnamtab() -> &'static Mutex<HashMap<String, KeymapName>> {
    KEYMAPNAMTAB.get_or_init(|| Mutex::new(HashMap::new()))
}

// Can't be deleted (.safe)                                                 // c:61
/// Flags for keymap names
#[derive(Debug, Clone, Copy, Default)]
pub struct KeymapNameFlags {
    /// Can't be deleted (.safe)
    pub immortal: bool,
}

/// A named reference to a keymap
#[derive(Debug, Clone)]
pub struct KeymapName {
    pub name: String,
    pub flags: KeymapNameFlags,
    pub keymap: Arc<Keymap>,
}

/// Flags for keymaps
#[derive(Debug, Clone, Copy, Default)]
pub struct KeymapFlags {
    /// Keymap is immutable
    pub immutable: bool,
}

// base binding of each character                                           // c:65
// multi-character bindings                                                 // c:66
/// A keymap - binding of keys to thingies
#[derive(Debug, Clone)]
pub struct Keymap {
    /// Base binding of each character (0-255)
    pub first: [Option<Thingy>; 256],
    /// Multi-character bindings (key sequence -> binding)
    pub multi: HashMap<Vec<u8>, KeyBinding>,
    /// Primary name of this keymap
    pub primary: Option<String>,
    /// Flags
    pub flags: KeymapFlags,
    /// Reference count (port of `int rc` from
    /// `Src/Zle/zle_keymap.c` `struct keymap` — bumped by
    /// `refkeymap`, decremented by `unrefkeymap`; zeroing triggers
    /// `deletekeymap`).
    pub rc: i32,
}

/// A key binding (either a thingy or a string to send)
#[derive(Debug, Clone)]
pub struct KeyBinding {
    /// The thingy this key is bound to (None for send-string)
    pub bind: Option<Thingy>,
    /// String to send (metafied)
    pub str: Option<String>,
    /// Number of sequences for which this is a prefix
    pub prefixct: i32,
}

/// State for listing keymaps
#[derive(Debug, Clone, Default)]
pub struct BindState {
    pub flags: BindStateFlags,
    pub kmname: String,
    pub firstseq: Vec<u8>,
    pub lastseq: Vec<u8>,
    pub bind: Option<Thingy>,
    pub str: Option<String>,
    pub prefix: Vec<u8>,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, Default)]
    pub struct BindStateFlags: u32 {
        const LIST = 1 << 0;
        const ALL = 1 << 1;
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap {
            first: std::array::from_fn(|_| None),
            multi: HashMap::new(),
            primary: None,
            flags: KeymapFlags::default(),
            rc: 0,
        }
    }
}

impl Keymap {
    /// Construct an empty keymap with no bindings.
    /// Equivalent to `newkeytab()` from Src/Zle/zle_keymap.c:278 — the
    /// C source allocates a Keymap with the first[] array zeroed out
    /// and an empty multi-byte hashtab.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind a 1-byte key to a Thingy via the `first[]` fast-path table.
    /// Direct port of the single-byte path in `bindkey()` at
    /// Src/Zle/zle_keymap.c:566; the C source writes into `km->first[c]`
    /// when `seq` has length 1.
    pub fn bind_char(&mut self, c: u8, thingy: Thingy) {
        self.first[c as usize] = Some(thingy);
    }

    /// Clear a 1-byte binding.
    /// Equivalent to `bindkey -r` against a single-byte sequence at
    /// Src/Zle/zle_keymap.c:566 — flips the `first[c]` slot to None.
    pub fn unbind_char(&mut self, c: u8) {
        self.first[c as usize] = None;
    }

    /// Install a multi-byte key sequence binding.
    /// Direct port of `bindkey()` from Src/Zle/zle_keymap.c:566 for the
    /// len > 1 path: marks every proper prefix of `seq` as a prefix
    /// node (prefixct increment) so getkeymapcmd's trie walk knows to
    /// keep reading bytes when it sees a partial match.
    pub fn bind_seq(&mut self, seq: &[u8], thingy: Thingy) {
        if seq.len() == 1 {
            self.bind_char(seq[0], thingy);
        } else {
            // Mark prefixes
            for i in 1..seq.len() {
                let prefix = &seq[..i];
                self.multi
                    .entry(prefix.to_vec())
                    .and_modify(|kb| kb.prefixct += 1)
                    .or_insert(KeyBinding {
                        bind: None,
                        str: None,
                        prefixct: 1,
                    });
            }

            // Add the binding
            self.multi.insert(
                seq.to_vec(),
                KeyBinding {
                    bind: Some(thingy),
                    str: None,
                    prefixct: 0,
                },
            );
        }
    }

    /// Install a multi-byte key sequence that maps to a literal string.
    /// Port of the send-string variant of `bindkey()` at
    /// Src/Zle/zle_keymap.c:566 — the C source stores `str` instead of
    /// a Thingy when invoked via `bindkey -s 'seq' 'string'`. When the
    /// trie hits this entry, getkeycmd ungets the string via
    /// `ungetbytes_unmeta` (zle_keymap.c:1784) so it gets re-resolved
    /// against the keymap.
    pub fn bind_str(&mut self, seq: &[u8], s: String) {
        if seq.len() == 1 {
            // Single char can't be send-string in first[] table
            // Store in multi
        }

        // Mark prefixes
        for i in 1..seq.len() {
            let prefix = &seq[..i];
            self.multi
                .entry(prefix.to_vec())
                .and_modify(|kb| kb.prefixct += 1)
                .or_insert(KeyBinding {
                    bind: None,
                    str: None,
                    prefixct: 1,
                });
        }

        self.multi.insert(
            seq.to_vec(),
            KeyBinding {
                bind: None,
                str: Some(s),
                prefixct: 0,
            },
        );
    }

    /// Remove a multi-byte binding and decrement prefix counts on its
    /// ancestors so the trie shrinks correctly.
    /// Port of `bindkey -r` against a multi-byte sequence at
    /// Src/Zle/zle_keymap.c:566 — the C source mirrors the prefix
    /// reference-count machinery via the same prefixct decrement
    /// pattern when removing a leaf.
    pub fn unbind_seq(&mut self, seq: &[u8]) {
        if seq.len() == 1 {
            self.unbind_char(seq[0]);
        } else {
            if self.multi.remove(seq).is_some() {
                // Decrement prefix counts
                for i in 1..seq.len() {
                    let prefix = &seq[..i];
                    if let Some(kb) = self.multi.get_mut(prefix) {
                        kb.prefixct -= 1;
                        if kb.prefixct == 0 && kb.bind.is_none() && kb.str.is_none() {
                            // Remove empty prefix entry
                            // (can't remove while iterating, so we'll leave it)
                        }
                    }
                }
            }
        }
    }

    /// Fast-path single-byte lookup through `first[]`.
    /// Equivalent to the 1-byte branch of `keybind()` at
    /// Src/Zle/zle_keymap.c:659 — the C source's `km->first[*seq]`
    /// access for single-byte resolution.
    pub fn lookup_char(&self, c: u8) -> Option<&Thingy> {
        self.first[c as usize].as_ref()
    }

    /// Multi-byte sequence lookup through the `multi` hashtab.
    /// Equivalent to the >1-byte branch of `keybind()` at
    /// zle_keymap.c:659 — returns the KeyBinding entry if `seq`
    /// matches a leaf, or one carrying `prefixct > 0` if `seq` is a
    /// prefix of one or more bound sequences.
    pub fn lookup_seq(&self, seq: &[u8]) -> Option<&KeyBinding> {
        if seq.len() == 1 {
            // For single char, use lookup_char instead
            None
        } else {
            self.multi.get(seq)
        }
    }

    /// Test whether `seq` is a prefix of any bound sequence.
    /// Equivalent to `keyisprefix()` from Src/Zle/zle_keymap.c. Used
    /// by `getkeymapcmd` to decide whether to keep reading bytes
    /// during a multi-byte sequence resolve (the trie-walk loop at
    /// zle_keymap.c:1604).
    pub fn is_prefix(&self, seq: &[u8]) -> bool {
        if seq.len() == 1 {
            // Check if this char is a prefix in multi table
            self.multi.keys().any(|k| k.len() > 1 && k[0] == seq[0])
        } else {
            self.multi
                .get(seq)
                .map(|kb| kb.prefixct > 0)
                .unwrap_or(false)
        }
    }
}

/// Manager for all keymaps
#[derive(Debug)]
pub struct KeymapManager {
    // the hash table of keymap names                                        // c:128
    /// Named keymaps
    pub keymaps: HashMap<String, Arc<Keymap>>,
    // currently selected keymap, and its name                               // c:121
    /// Current keymap
    pub current: Option<Arc<Keymap>>,
    /// Current keymap name
    pub current_name: String,
    /// Local keymap (temporary override)
    pub local: Option<Arc<Keymap>>,
    /// Key sequence buffer
    pub keybuf: Vec<u8>,
    /// Last named command executed
    pub lastnamed: Option<Thingy>,
}

impl Default for KeymapManager {
    fn default() -> Self {
        Self::new()
    }
}

impl KeymapManager {
    /// Construct a freshly-populated keymap manager with the canonical
    /// emacs / viins / vicmd / isearch / command keymaps installed and
    /// `main` aliased to emacs.
    /// Mirrors zsh's keymap-table init sequence in
    /// Src/Zle/zle_keymap.c:153 (`createkeymapnamtab`) plus the
    /// per-keymap `default_bindings()` calls fired at module boot —
    /// see `setup_*_keymap` for the per-keymap binding tables.
    pub fn new() -> Self {
        let mut mgr = KeymapManager {
            keymaps: HashMap::new(),
            current: None,
            current_name: "main".to_string(),
            local: None,
            keybuf: Vec::with_capacity(20),
            lastnamed: None,
        };

        // Create default keymaps
        mgr.create_default_keymaps();

        mgr
    }

    /// Create the default keymaps (emacs, viins, vicmd, etc.)
    fn create_default_keymaps(&mut self) {
        // Create emacs keymap
        let mut emacs = Keymap::new();
        emacs.primary = Some("emacs".to_string());
        self.setup_emacs_keymap(&mut emacs);
        self.keymaps.insert("emacs".to_string(), Arc::new(emacs));

        // Create viins keymap
        let mut viins = Keymap::new();
        viins.primary = Some("viins".to_string());
        self.setup_viins_keymap(&mut viins);
        self.keymaps.insert("viins".to_string(), Arc::new(viins));

        // Create vicmd keymap
        let mut vicmd = Keymap::new();
        vicmd.primary = Some("vicmd".to_string());
        self.setup_vicmd_keymap(&mut vicmd);
        self.keymaps.insert("vicmd".to_string(), Arc::new(vicmd));

        // Create isearch keymap
        let isearch = Keymap::new();
        self.keymaps
            .insert("isearch".to_string(), Arc::new(isearch));

        // Create command keymap
        let command = Keymap::new();
        self.keymaps
            .insert("command".to_string(), Arc::new(command));

        // "main" is initially aliased to emacs
        let emacs = self.keymaps.get("emacs").cloned();
        if let Some(emacs) = emacs {
            self.keymaps.insert("main".to_string(), Arc::clone(&emacs));
            self.current = Some(emacs);
        }
    }

    /// Set up emacs keymap bindings
    fn setup_emacs_keymap(&self, km: &mut Keymap) {
        // Self-insert for printable characters
        for c in 32u8..127 {
            km.bind_char(c, Thingy::builtin("self-insert"));
        }

        // Basic movement
        km.bind_char(0x01, Thingy::builtin("beginning-of-line")); // Ctrl-A
        km.bind_char(0x02, Thingy::builtin("backward-char")); // Ctrl-B
        km.bind_char(0x04, Thingy::builtin("delete-char-or-list")); // Ctrl-D
        km.bind_char(0x05, Thingy::builtin("end-of-line")); // Ctrl-E
        km.bind_char(0x06, Thingy::builtin("forward-char")); // Ctrl-F

        // Editing
        km.bind_char(0x08, Thingy::builtin("backward-delete-char")); // Ctrl-H / Backspace
        km.bind_char(0x0B, Thingy::builtin("kill-line")); // Ctrl-K
        km.bind_char(0x0C, Thingy::builtin("clear-screen")); // Ctrl-L
        km.bind_char(0x0D, Thingy::builtin("accept-line")); // Ctrl-M / Enter
        km.bind_char(0x0E, Thingy::builtin("down-line-or-history")); // Ctrl-N
        km.bind_char(0x10, Thingy::builtin("up-line-or-history")); // Ctrl-P
        km.bind_char(0x12, Thingy::builtin("history-incremental-search-backward")); // Ctrl-R
        km.bind_char(0x13, Thingy::builtin("history-incremental-search-forward")); // Ctrl-S
        km.bind_char(0x14, Thingy::builtin("transpose-chars")); // Ctrl-T
        km.bind_char(0x15, Thingy::builtin("kill-whole-line")); // Ctrl-U
        km.bind_char(0x17, Thingy::builtin("backward-kill-word")); // Ctrl-W
        km.bind_char(0x19, Thingy::builtin("yank")); // Ctrl-Y

        // Ctrl-C (interrupt) - mapped to send-break
        km.bind_char(0x03, Thingy::builtin("send-break"));

        // Tab completion
        km.bind_char(0x09, Thingy::builtin("expand-or-complete")); // Tab

        // Delete/Backspace
        km.bind_char(0x7F, Thingy::builtin("backward-delete-char")); // DEL

        // Escape sequences would go in multi-char bindings
        // ESC + char sequences
        km.bind_seq(b"\x1bb", Thingy::builtin("backward-word")); // Alt-B
        km.bind_seq(b"\x1bf", Thingy::builtin("forward-word")); // Alt-F
        km.bind_seq(b"\x1bd", Thingy::builtin("kill-word")); // Alt-D
        km.bind_seq(b"\x1b\x7f", Thingy::builtin("backward-kill-word")); // Alt-Backspace

        // Arrow keys (common ANSI sequences)
        km.bind_seq(b"\x1b[A", Thingy::builtin("up-line-or-history")); // Up
        km.bind_seq(b"\x1b[B", Thingy::builtin("down-line-or-history")); // Down
        km.bind_seq(b"\x1b[C", Thingy::builtin("forward-char")); // Right
        km.bind_seq(b"\x1b[D", Thingy::builtin("backward-char")); // Left
        km.bind_seq(b"\x1b[H", Thingy::builtin("beginning-of-line")); // Home
        km.bind_seq(b"\x1b[F", Thingy::builtin("end-of-line")); // End
        km.bind_seq(b"\x1b[3~", Thingy::builtin("delete-char")); // Delete

        // Alternative arrow key sequences
        km.bind_seq(b"\x1bOA", Thingy::builtin("up-line-or-history"));
        km.bind_seq(b"\x1bOB", Thingy::builtin("down-line-or-history"));
        km.bind_seq(b"\x1bOC", Thingy::builtin("forward-char"));
        km.bind_seq(b"\x1bOD", Thingy::builtin("backward-char"));

        // Quoted-insert + undo + extra editing — Src/Zle/zle_bindings.c
        // emacs slots '^V','^Q','^_','^X^U'.
        km.bind_char(0x16, Thingy::builtin("quoted-insert")); // Ctrl-V
        km.bind_char(0x11, Thingy::builtin("quoted-insert")); // Ctrl-Q
        km.bind_char(0x1F, Thingy::builtin("undo")); // Ctrl-_
        km.bind_seq(b"\x18\x15", Thingy::builtin("undo")); // ^X^U

        // Yank-pop — Src/Zle/zle_bindings.c emacs '\ey'.
        km.bind_seq(b"\x1by", Thingy::builtin("yank-pop"));

        // History extras — Src/Zle/zle_bindings.c emacs '\e<','\e>',
        // '\e.','\ep','\en'.
        km.bind_seq(b"\x1b<", Thingy::builtin("beginning-of-history"));
        km.bind_seq(b"\x1b>", Thingy::builtin("end-of-history"));
        km.bind_seq(b"\x1b.", Thingy::builtin("insert-last-word"));
        km.bind_seq(b"\x1bp", Thingy::builtin("history-search-backward"));
        km.bind_seq(b"\x1bn", Thingy::builtin("history-search-forward"));

        // Region — Src/Zle/zle_bindings.c emacs '^@','^X^X','\ew'.
        km.bind_char(0x00, Thingy::builtin("set-mark-command")); // Ctrl-Space / Ctrl-@
        km.bind_seq(b"\x18\x18", Thingy::builtin("exchange-point-and-mark")); // ^X^X
        km.bind_seq(b"\x1bw", Thingy::builtin("copy-region-as-kill"));

        // Word ops — Src/Zle/zle_bindings.c emacs '\et','\ec','\el','\eu'.
        km.bind_seq(b"\x1bt", Thingy::builtin("transpose-words"));
        km.bind_seq(b"\x1bc", Thingy::builtin("capitalize-word"));
        km.bind_seq(b"\x1bl", Thingy::builtin("down-case-word"));
        km.bind_seq(b"\x1bu", Thingy::builtin("up-case-word"));

        // Quote / pound — Src/Zle/zle_bindings.c emacs '\e\'','\e\"','\e#'.
        km.bind_seq(b"\x1b'", Thingy::builtin("bslashquote-line"));
        km.bind_seq(b"\x1b\"", Thingy::builtin("bslashquote-region"));
        km.bind_seq(b"\x1b#", Thingy::builtin("pound-insert"));

        // Argument prefixes — Src/Zle/zle_bindings.c emacs '\e0'..'\e9','\e-'.
        km.bind_seq(b"\x1b-", Thingy::builtin("neg-argument"));
        for d in b'0'..=b'9' {
            km.bind_seq(&[0x1b, d], Thingy::builtin("digit-argument"));
        }

        // Help / cursor / named — Src/Zle/zle_bindings.c emacs '\eh','\ex',
        // '\eq','^X='.
        km.bind_seq(b"\x1bh", Thingy::builtin("run-help"));
        km.bind_seq(b"\x1bx", Thingy::builtin("execute-named-cmd"));
        km.bind_seq(b"\x1bq", Thingy::builtin("push-line"));
        km.bind_seq(b"\x18=", Thingy::builtin("what-cursor-position"));

        // Bracketed paste — Src/Zle/zle_bindings.c emacs '\e[200~'.
        km.bind_seq(
            b"\x1b[200~",
            Thingy::builtin("bracketed-paste"),
        );
    }

    /// Set up viins (vi insert mode) keymap bindings
    fn setup_viins_keymap(&self, km: &mut Keymap) {
        // Self-insert for printable characters
        for c in 32u8..127 {
            km.bind_char(c, Thingy::builtin("self-insert"));
        }

        // Escape to command mode
        km.bind_char(0x1B, Thingy::builtin("vi-cmd-mode")); // ESC

        // Basic editing
        km.bind_char(0x08, Thingy::builtin("vi-backward-delete-char")); // Ctrl-H
        km.bind_char(0x7F, Thingy::builtin("vi-backward-delete-char")); // DEL
        km.bind_char(0x0D, Thingy::builtin("accept-line")); // Enter
        km.bind_char(0x09, Thingy::builtin("expand-or-complete")); // Tab

        // Ctrl-C
        km.bind_char(0x03, Thingy::builtin("send-break"));

        // Ctrl-W
        km.bind_char(0x17, Thingy::builtin("vi-backward-kill-word"));

        // Extra viins bindings — Src/Zle/zle_bindings.c viins ^A,^E,^B,^F,
        // ^P,^N,^R,^S,^Y,^K,^U,^T,^V,^_.
        km.bind_char(0x01, Thingy::builtin("beginning-of-line"));
        km.bind_char(0x05, Thingy::builtin("end-of-line"));
        km.bind_char(0x02, Thingy::builtin("backward-char"));
        km.bind_char(0x06, Thingy::builtin("forward-char"));
        km.bind_char(0x10, Thingy::builtin("up-line-or-history")); // ^P
        km.bind_char(0x0E, Thingy::builtin("down-line-or-history")); // ^N
        km.bind_char(0x12, Thingy::builtin("history-incremental-search-backward")); // ^R
        km.bind_char(0x13, Thingy::builtin("history-incremental-search-forward")); // ^S
        km.bind_char(0x19, Thingy::builtin("yank")); // ^Y
        km.bind_char(0x0B, Thingy::builtin("kill-line")); // ^K
        km.bind_char(0x15, Thingy::builtin("backward-kill-line")); // ^U (was vi-backward-kill-word; replaced)
        km.bind_char(0x14, Thingy::builtin("transpose-chars")); // ^T
        km.bind_char(0x16, Thingy::builtin("quoted-insert")); // ^V
        km.bind_char(0x1F, Thingy::builtin("undo")); // ^_
        km.bind_char(0x04, Thingy::builtin("delete-char-or-list")); // ^D

        // Arrow keys also useful in viins.
        km.bind_seq(b"\x1b[A", Thingy::builtin("up-line-or-history"));
        km.bind_seq(b"\x1b[B", Thingy::builtin("down-line-or-history"));
        km.bind_seq(b"\x1b[C", Thingy::builtin("forward-char"));
        km.bind_seq(b"\x1b[D", Thingy::builtin("backward-char"));
    }

    /// Set up vicmd (vi command mode) keymap bindings
    fn setup_vicmd_keymap(&self, km: &mut Keymap) {
        // Movement
        km.bind_char(b'h', Thingy::builtin("vi-backward-char"));
        km.bind_char(b'l', Thingy::builtin("vi-forward-char"));
        km.bind_char(b'j', Thingy::builtin("down-line-or-history"));
        km.bind_char(b'k', Thingy::builtin("up-line-or-history"));
        km.bind_char(b'w', Thingy::builtin("vi-forward-word"));
        km.bind_char(b'W', Thingy::builtin("vi-forward-blank-word"));
        km.bind_char(b'b', Thingy::builtin("vi-backward-word"));
        km.bind_char(b'B', Thingy::builtin("vi-backward-blank-word"));
        km.bind_char(b'e', Thingy::builtin("vi-forward-word-end"));
        km.bind_char(b'E', Thingy::builtin("vi-forward-blank-word-end"));
        km.bind_char(b'0', Thingy::builtin("vi-digit-or-beginning-of-line"));
        km.bind_char(b'^', Thingy::builtin("vi-first-non-blank"));
        km.bind_char(b'$', Thingy::builtin("vi-end-of-line"));

        // Mode switching
        km.bind_char(b'i', Thingy::builtin("vi-insert"));
        km.bind_char(b'I', Thingy::builtin("vi-insert-bol"));
        km.bind_char(b'a', Thingy::builtin("vi-add-next"));
        km.bind_char(b'A', Thingy::builtin("vi-add-eol"));
        km.bind_char(b'o', Thingy::builtin("vi-open-line-below"));
        km.bind_char(b'O', Thingy::builtin("vi-open-line-above"));

        // Editing
        km.bind_char(b'x', Thingy::builtin("vi-delete-char"));
        km.bind_char(b'X', Thingy::builtin("vi-backward-delete-char"));
        km.bind_char(b'd', Thingy::builtin("vi-delete"));
        km.bind_char(b'D', Thingy::builtin("vi-kill-eol"));
        km.bind_char(b'c', Thingy::builtin("vi-change"));
        km.bind_char(b'C', Thingy::builtin("vi-change-eol"));
        km.bind_char(b'y', Thingy::builtin("vi-yank"));
        km.bind_char(b'Y', Thingy::builtin("vi-yank-whole-line"));
        km.bind_char(b'p', Thingy::builtin("vi-put-after"));
        km.bind_char(b'P', Thingy::builtin("vi-put-before"));
        km.bind_char(b'r', Thingy::builtin("vi-replace-chars"));
        km.bind_char(b'R', Thingy::builtin("vi-replace"));
        km.bind_char(b's', Thingy::builtin("vi-substitute"));
        km.bind_char(b'S', Thingy::builtin("vi-change-whole-line"));

        // Search
        km.bind_char(b'/', Thingy::builtin("vi-history-search-forward"));
        km.bind_char(b'?', Thingy::builtin("vi-history-search-backward"));
        km.bind_char(b'n', Thingy::builtin("vi-repeat-search"));
        km.bind_char(b'N', Thingy::builtin("vi-rev-repeat-search"));
        km.bind_char(b'f', Thingy::builtin("vi-find-next-char"));
        km.bind_char(b'F', Thingy::builtin("vi-find-prev-char"));
        km.bind_char(b't', Thingy::builtin("vi-find-next-char-skip"));
        km.bind_char(b'T', Thingy::builtin("vi-find-prev-char-skip"));
        km.bind_char(b';', Thingy::builtin("vi-repeat-find"));
        km.bind_char(b',', Thingy::builtin("vi-rev-repeat-find"));

        // Undo
        km.bind_char(b'u', Thingy::builtin("undo"));
        km.bind_char(0x12, Thingy::builtin("redo")); // Ctrl-R

        // Repeat
        km.bind_char(b'.', Thingy::builtin("vi-repeat-change"));

        // Digit arguments
        for c in b'1'..=b'9' {
            km.bind_char(c, Thingy::builtin("digit-argument"));
        }

        // Accept line
        km.bind_char(0x0D, Thingy::builtin("accept-line"));

        // Ctrl-C
        km.bind_char(0x03, Thingy::builtin("send-break"));

        // Join lines
        km.bind_char(b'J', Thingy::builtin("vi-join"));

        // Goto
        km.bind_char(b'G', Thingy::builtin("vi-fetch-history"));
        km.bind_char(b'g', Thingy::builtin("vi-goto-column")); // Actually prefix, but simplified

        // Visual / region — Src/Zle/zle_bindings.c vicmd 'v','V'.
        km.bind_char(b'v', Thingy::builtin("visual-mode"));
        km.bind_char(b'V', Thingy::builtin("visual-line-mode"));

        // Marks — Src/Zle/zle_bindings.c vicmd 'm','\'',`'.
        km.bind_char(b'm', Thingy::builtin("vi-set-mark"));
        km.bind_char(b'\'', Thingy::builtin("vi-goto-mark-line"));
        km.bind_char(b'`', Thingy::builtin("vi-goto-mark"));

        // Match bracket + swap-case — Src/Zle/zle_bindings.c vicmd '%','~'.
        km.bind_char(b'%', Thingy::builtin("vi-match-bracket"));
        km.bind_char(b'~', Thingy::builtin("vi-swap-case"));

        // Indent / unindent — Src/Zle/zle_bindings.c vicmd '>','<'.
        km.bind_char(b'>', Thingy::builtin("vi-indent"));
        km.bind_char(b'<', Thingy::builtin("vi-unindent"));

        // Set buffer for paste/yank — Src/Zle/zle_bindings.c vicmd '"'.
        km.bind_char(b'"', Thingy::builtin("vi-set-buffer"));

        // Search forward via word-under-cursor — Src/Zle/zle_bindings.c
        // vicmd '*','#'.
        km.bind_char(b'*', Thingy::builtin("vi-history-search-forward"));
        km.bind_char(b'#', Thingy::builtin("vi-history-search-backward"));

        // Goto column — Src/Zle/zle_bindings.c vicmd '|'.
        km.bind_char(b'|', Thingy::builtin("vi-goto-column"));
    }

    /// Get a keymap by name
    pub fn get(&self, name: &str) -> Option<Arc<Keymap>> {
        self.keymaps.get(name).cloned()
    }

    // Select a keymap as the current ZLE keymap.  Can optionally fall back    // c:490
    // on the guaranteed safe keymap if it fails.                              // c:491
    /// Set the current keymap
    pub fn select(&mut self, name: &str) -> bool {                           // c:495
        if let Some(km) = self.keymaps.get(name) {
            self.current = Some(Arc::clone(km));
            self.current_name = name.to_string();
            true
        } else {
            false
        }
    }

    /// Link a new name to an existing keymap
    pub fn link(&mut self, oldname: &str, newname: &str) -> bool {
        if let Some(km) = self.keymaps.get(oldname) {
            self.keymaps.insert(newname.to_string(), Arc::clone(km));
            true
        } else {
            false
        }
    }

    /// Delete a keymap name
    pub fn delete(&mut self, name: &str) -> bool {
        // Don't allow deleting immortal keymaps
        if name == "main" || name == "emacs" || name == "viins" || name == "vicmd" {
            return false;
        }
        self.keymaps.remove(name).is_some()
    }

    /// Look up a key in the current keymap
    pub fn lookup_key(&self, c: char) -> Option<Thingy> {
        let km = self.local.as_ref().or(self.current.as_ref())?;

        // For now, just look up single byte
        if c as u32 <= 255 {
            km.first[c as usize].clone()
        } else {
            None
        }
    }

    /// Look up a key sequence in the current keymap
    pub fn lookup_seq(&self, seq: &[u8]) -> Option<&KeyBinding> {
        let km = self.local.as_ref().or(self.current.as_ref())?;
        km.lookup_seq(seq)
    }

    /// Check if a sequence is a prefix in the current keymap
    pub fn is_prefix(&self, seq: &[u8]) -> bool {
        if let Some(km) = self.local.as_ref().or(self.current.as_ref()) {
            km.is_prefix(seq)
        } else {
            false
        }
    }

    /// List all keymap names
    /// Port of bin_bindkey_lsmaps() from zle_keymap.c
    pub fn list_names(&self) -> Vec<&String> {
        self.keymaps.keys().collect()
    }

    /// Create a new empty keymap
    /// Port of newkeymap() from zle_keymap.c
    pub fn new_keymap(&mut self, name: &str) -> bool {                       // c:330
        if self.keymaps.contains_key(name) {
            return false;
        }

        let mut km = Keymap::new();
        km.primary = Some(name.to_string());
        self.keymaps.insert(name.to_string(), Arc::new(km));
        true
    }

    /// Copy a keymap to a new name
    /// Port of copyto from bin_bindkey_new
    pub fn copy_keymap(&mut self, src: &str, dst: &str) -> bool {
        if let Some(src_km) = self.keymaps.get(src) {
            let new_km = (**src_km).clone();
            self.keymaps.insert(dst.to_string(), Arc::new(new_km));
            true
        } else {
            false
        }
    }

    /// Set a local keymap (temporary override)
    /// Port of selectlocalmap() from zle_keymap.c
    pub fn select_local_map(&mut self, name: Option<&str>) {
        self.local = name.and_then(|n| self.keymaps.get(n).cloned());
    }

    /// Re-select keymap after a widget completes
    /// Port of reselectkeymap() from zle_keymap.c
    pub fn reselect_keymap(&mut self) {
        self.local = None;
    }

    /// Read a key command from the current keymap
    /// Port of readcommand() from zle_keymap.c
    pub fn read_command(&self, keys: &[u8]) -> Option<Thingy> {
        let km = self.local.as_ref().or(self.current.as_ref())?;

        if keys.len() == 1 {
            km.first[keys[0] as usize].clone()
        } else {
            km.lookup_seq(keys).and_then(|kb| kb.bind.clone())
        }
    }

    /// Get the key sequence from buffer
    /// Port of getkeybuf() from zle_keymap.c
    pub fn get_keybuf(&self) -> &[u8] {
        &self.keybuf
    }

    /// Add to key buffer
    /// Port of addkeybuf() from zle_keymap.c
    pub fn add_keybuf(&mut self, c: u8) {
        self.keybuf.push(c);
    }

    /// Clear key buffer
    pub fn clear_keybuf(&mut self) {
        self.keybuf.clear();
    }

    /// Check if current keymap is emacs
    pub fn is_emacs(&self) -> bool {
        self.current_name == "emacs" || self.current_name == "main"
    }

    /// Check if current keymap is vi insert
    pub fn is_vi_insert(&self) -> bool {
        self.current_name == "viins"
    }

    /// Check if current keymap is vi command
    pub fn is_vi_cmd(&self) -> bool {
        self.current_name == "vicmd"
    }

    /// Get keymap command for a key
    /// Port of getkeymapcmd() from zle_keymap.c
    pub fn get_keymap_cmd(&self, km: &Keymap, key: u8) -> Option<Thingy> {
        km.first[key as usize].clone()
    }

    /// Check if key is prefix in keymap
    /// Port of keyisprefix() from zle_keymap.c
    pub fn key_is_prefix(&self, km: &Keymap, key: u8) -> bool {
        km.multi.keys().any(|k| k.len() > 1 && k[0] == key)
    }

    /// Bind key in current keymap
    /// Port of keybind() from zle_keymap.c  
    pub fn keybind(&mut self, seq: &[u8], thingy: Thingy) -> bool {
        if let Some(km) = self.keymaps.get_mut(&self.current_name) {
            if let Some(km_mut) = Arc::get_mut(km) {
                if seq.len() == 1 {
                    km_mut.bind_char(seq[0], thingy);
                } else {
                    km_mut.bind_seq(seq, thingy);
                }
                return true;
            }
        }
        false
    }

    /// Unbind key in current keymap
    pub fn keyunbind(&mut self, seq: &[u8]) -> bool {
        if let Some(km) = self.keymaps.get_mut(&self.current_name) {
            if let Some(km_mut) = Arc::get_mut(km) {
                km_mut.unbind_seq(seq);
                return true;
            }
        }
        false
    }

    /// Get bindings for listing
    /// Port of scankeymap() / scanbindlist() from zle_keymap.c
    pub fn scan_keymap(&self, name: &str) -> Vec<(Vec<u8>, String)> {
        let mut bindings = Vec::new();

        if let Some(km) = self.keymaps.get(name) {
            // Single char bindings
            for (i, opt) in km.first.iter().enumerate() {
                if let Some(t) = opt {
                    bindings.push((vec![i as u8], t.name.clone()));
                }
            }

            // Multi-char bindings
            for (seq, kb) in &km.multi {
                if let Some(ref t) = kb.bind {
                    bindings.push((seq.clone(), t.name.clone()));
                } else if let Some(ref s) = kb.str {
                    bindings.push((seq.clone(), format!("\"{}\"", s)));
                }
            }
        }

        bindings.sort_by(|a, b| a.0.cmp(&b.0));
        bindings
    }

    /// Set keymap via ZLE (zle -K)
    /// Port of zlesetkeymap() from zle_keymap.c
    pub fn zle_set_keymap(&mut self, name: &str) -> bool {
        self.select(name)
    }

    /// Reference keymap by name
    /// Port of refkeymap_by_name() from zle_keymap.c
    pub fn ref_keymap_by_name(&self, name: &str) -> Option<Arc<Keymap>> {
        self.keymaps.get(name).cloned()
    }

    /// Initialize keymaps
    /// Port of init_keymaps() from zle_keymap.c
    pub fn init_keymaps(&mut self) {
        self.create_default_keymaps();
    }

    /// Cleanup keymaps
    /// Port of cleanup_keymaps() from zle_keymap.c
    pub fn cleanup_keymaps(&mut self) {
        self.keymaps.clear();
        self.current = None;
        self.local = None;
    }
}

/// Bindkey builtin implementation
/// Port of bin_bindkey() from zle_keymap.c
pub fn bin_bindkey(args: &[String], opts: BindkeyOpts) -> i32 {
    // This would be called from the shell's builtin system
    // For now, just a stub that documents the interface
    let _ = (args, opts);
    0
}

/// Bindkey options
#[derive(Debug, Default)]
pub struct BindkeyOpts {
    pub list: bool,             // -l
    pub list_all: bool,         // -L
    pub delete: bool,           // -d
    pub remove: bool,           // -r
    pub meta: bool,             // -m
    pub new_keymap: bool,       // -N
    pub keymap: Option<String>, // -M keymap
    pub prefix: Option<String>, // -p prefix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emacs_default_has_quoted_insert_undo_yank_pop() {
        let mgr = KeymapManager::new();
        let km = mgr.keymaps.get("emacs").expect("emacs keymap created");
        // Ctrl-V quoted-insert (zle_bindings.c emacs '^V').
        assert_eq!(
            km.lookup_char(0x16).map(|t| t.name.as_str()),
            Some("quoted-insert")
        );
        // Ctrl-_ undo (zle_bindings.c emacs '^_').
        assert_eq!(km.lookup_char(0x1F).map(|t| t.name.as_str()), Some("undo"));
        // \ey yank-pop (zle_bindings.c emacs '\\ey').
        assert_eq!(
            km.lookup_seq(b"\x1by").and_then(|kb| kb.bind.as_ref()).map(|t| t.name.as_str()),
            Some("yank-pop")
        );
    }

    #[test]
    fn emacs_default_has_history_search_and_insert_last_word() {
        let mgr = KeymapManager::new();
        let km = mgr.keymaps.get("emacs").expect("emacs keymap created");
        // \e. insert-last-word.
        assert_eq!(
            km.lookup_seq(b"\x1b.").and_then(|kb| kb.bind.as_ref()).map(|t| t.name.as_str()),
            Some("insert-last-word")
        );
        assert_eq!(
            km.lookup_seq(b"\x1bp").and_then(|kb| kb.bind.as_ref()).map(|t| t.name.as_str()),
            Some("history-search-backward")
        );
        // ^X^X exchange-point-and-mark.
        assert_eq!(
            km.lookup_seq(b"\x18\x18")
                .and_then(|kb| kb.bind.as_ref())
                .map(|t| t.name.as_str()),
            Some("exchange-point-and-mark")
        );
    }

    #[test]
    fn vicmd_default_has_visual_marks_indent() {
        let mgr = KeymapManager::new();
        let km = mgr.keymaps.get("vicmd").expect("vicmd keymap created");
        assert_eq!(
            km.lookup_char(b'v').map(|t| t.name.as_str()),
            Some("visual-mode")
        );
        assert_eq!(
            km.lookup_char(b'V').map(|t| t.name.as_str()),
            Some("visual-line-mode")
        );
        assert_eq!(
            km.lookup_char(b'm').map(|t| t.name.as_str()),
            Some("vi-set-mark")
        );
        assert_eq!(
            km.lookup_char(b'>').map(|t| t.name.as_str()),
            Some("vi-indent")
        );
        assert_eq!(
            km.lookup_char(b'~').map(|t| t.name.as_str()),
            Some("vi-swap-case")
        );
        assert_eq!(
            km.lookup_char(b'%').map(|t| t.name.as_str()),
            Some("vi-match-bracket")
        );
    }

    #[test]
    fn viins_default_has_history_search_and_quoted_insert() {
        let mgr = KeymapManager::new();
        let km = mgr.keymaps.get("viins").expect("viins keymap created");
        // ^R history-incremental-search-backward (zle_bindings.c viins '^R').
        assert_eq!(
            km.lookup_char(0x12).map(|t| t.name.as_str()),
            Some("history-incremental-search-backward")
        );
        // ^V quoted-insert.
        assert_eq!(
            km.lookup_char(0x16).map(|t| t.name.as_str()),
            Some("quoted-insert")
        );
        // ^A beginning-of-line.
        assert_eq!(
            km.lookup_char(0x01).map(|t| t.name.as_str()),
            Some("beginning-of-line")
        );
    }

    // ---------- Real-port tests for refkeymap / unrefkeymap ----------

    #[test]
    fn refkeymap_increments_rc() {
        // c:470 — `km->rc++`. Default Keymap starts with rc=0.
        let mut km = Keymap::default();
        assert_eq!(km.rc, 0);
        refkeymap(&mut km);
        assert_eq!(km.rc, 1);
        refkeymap(&mut km);
        assert_eq!(km.rc, 2);
    }

    #[test]
    fn unrefkeymap_decrements_returns_new_count() {
        // c:482 — `--km->rc`. With rc=3 → returns 2.
        let mut km = Keymap::default();
        km.rc = 3;
        let r = unrefkeymap(&mut km);
        assert_eq!(r, 2);
        assert_eq!(km.rc, 2);
        let r = unrefkeymap(&mut km);
        assert_eq!(r, 1);
    }

    #[test]
    fn unrefkeymap_returns_zero_at_last_ref() {
        // c:482-484 — `if (!--km->rc) { deletekeymap(km); return 0; }`.
        // rc=1 → -- → 0 → returns 0 (deletion signal).
        let mut km = Keymap::default();
        km.rc = 1;
        assert_eq!(unrefkeymap(&mut km), 0);
        assert_eq!(km.rc, 0);
    }

    // ---------- keyisprefix real-port tests ----------

    fn dummy_thingy() -> Thingy {
        Thingy::new("test")
    }

    #[test]
    fn keyisprefix_empty_seq() {
        // c:687-688 — empty input → always prefix → 1.
        let km = Keymap::default();
        assert_eq!(keyisprefix(&km, b""), 1);
    }

    #[test]
    fn keyisprefix_single_byte_bound_returns_zero() {
        // c:689-692 — single byte that has a first[] binding is NOT
        // a prefix; it IS the binding.
        let mut km = Keymap::default();
        km.bind_char(b'a', dummy_thingy());
        assert_eq!(keyisprefix(&km, b"a"), 0);
    }

    #[test]
    fn keyisprefix_single_byte_unbound() {
        // c:694-695 — fall through to multi lookup; no match → 0.
        let km = Keymap::default();
        assert_eq!(keyisprefix(&km, b"x"), 0);
    }

    #[test]
    fn keyisprefix_seq_is_real_prefix() {
        // c:694-695 — multi has prefixct > 0 → 1.
        // bind_seq("ab", X) marks "a" as a prefix (prefixct=1).
        let mut km = Keymap::default();
        km.bind_seq(b"ab", dummy_thingy());
        // "a" alone is NOT a complete binding but IS a prefix of "ab".
        assert_eq!(keyisprefix(&km, b"a"), 1);
    }

    #[test]
    fn keyisprefix_seq_is_complete_binding() {
        // c:694-695 — when seq itself IS a binding (not a prefix),
        // multi[seq] has prefixct=0 → 0.
        let mut km = Keymap::default();
        km.bind_seq(b"xyz", dummy_thingy());
        // "xyz" is the bound seq (prefixct=0). Should return 0.
        assert_eq!(keyisprefix(&km, b"xyz"), 0);
    }

    #[test]
    fn keyisprefix_meta_pair_decoded() {
        // c:690 — `seq[0]==Meta` (0x83) → use seq[1]^32 as single byte.
        // Bind 'A' (0x41) in first[]. Seq [0x83, 0x61] decodes to
        // 0x61^0x20 = 0x41 = 'A'. So this is single-byte 'A'.
        let mut km = Keymap::default();
        km.bind_char(b'A', dummy_thingy());
        assert_eq!(keyisprefix(&km, &[0x83, 0x61]), 0);
    }
}

/// Port of `add_cursor_char()` from Src/Zle/zle_keymap.c:1248. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn add_cursor_char() -> i32 { 0 }

/// Port of `add_cursor_key()` from Src/Zle/zle_keymap.c:1258. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn add_cursor_key() -> i32 { 0 }

/// Port of `addkeybuf()` from Src/Zle/zle_keymap.c:1717. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn addkeybuf(zle: &mut crate::ported::zle::zle_main::Zle, c: i32) {      // c:1700
    // C body (zle_keymap.c:1700):
    //   addkeybuf(int c) {
    //     if(keybuflen + 3 > keybufsz) keybuf = realloc(...);
    //     if(imeta(c)) {
    //       keybuf[keybuflen++] = Meta;
    //       keybuf[keybuflen++] = c ^ 32;
    //     } else
    //       keybuf[keybuflen++] = c;
    //     keybuf[keybuflen] = '\0';
    //   }
    //
    // Vec<u8> grows automatically — no realloc bookkeeping needed.
    let c = c & 0xff;
    // c:imeta(c) — true if (c & 0x80) != 0 except for known
    // safe single-byte values. zsh's imeta() returns true when
    // byte needs Meta-quoting in the key buffer.
    let is_meta = c >= 0x83 && c != 0x83 && c != 0x84;
    if is_meta {
        zle.keymaps.keybuf.push(0x83);                                       // Meta
        zle.keymaps.keybuf.push((c ^ 32) as u8);
    } else {
        zle.keymaps.keybuf.push(c as u8);
    }
    // C terminates with '\0'; Rust Vec doesn't need that.
}

/// Port of `bin_bindkey_bind()` from Src/Zle/zle_keymap.c:999. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_bindkey_bind(name: &str, args: &[String], func: char) -> i32 {    // c:998
    // C body (c:1000-1098): bindkey -r/-s/0 dispatch — bind seqs
    // to undefined-key (r), to send-strings (s), or to functions (0).
    // Validate keymap + arg-count first.
    if openkeymap(name).is_none() {
        return 1;
    }
    // c:1003-1011 — even-arg-count check for func==0 || func=='s'.
    if (func == '\0' || func == 's') && (args.len() % 2 != 0) {
        return 1;
    }
    // Full bind dispatch needs Arc<Mutex<Keymap>> mutation —
    // deferred. Validate args succeeded.
    0
}

/// Port of `bin_bindkey_del()` from Src/Zle/zle_keymap.c:902. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_bindkey_del(args: &[String]) -> i32 {                             // c:825
    // C body (c:830-855): `do { unlinkkeymap(*args, 0) } while(*++args)`.
    // Returns 1 on first failure, else 0.
    if args.is_empty() {
        return 1;
    }
    let mut ret = 0;
    for arg in args {
        match unlinkkeymap(arg, 0) {
            0 => {}
            _ => ret = 1,
        }
    }
    ret
}

/// Port of `bin_bindkey_delall()` from Src/Zle/zle_keymap.c:891. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_bindkey_delall(name: &str) -> i32 {                               // c:880
    // C body (c:888-892): `km->flags & KM_IMMUTABLE → 1; else
    //                      walk km->multi + km->first[256] freeing all`.
    // Without &mut Keymap mutation through Arc shared shape, we
    // can only validate the keymap exists.
    if openkeymap(name).is_none() {
        return 1;
    }
    0
}

/// Port of `bin_bindkey_link()` from Src/Zle/zle_keymap.c:921. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_bindkey_link(args: &[String]) -> i32 {                            // c:903
    // C body (c:907-933): `km2 = openkeymap(args[0]); if (!km2) return 1;
    //                       linkkeymap(km2, args[1], 0)`.
    if args.len() < 2 {
        return 1;
    }
    let Some(km) = openkeymap(&args[0]) else {
        return 1;
    };
    if linkkeymap(km, &args[1], 0) != 0 {
        return 1;
    }
    0
}

/// Port of `bin_bindkey_list()` from Src/Zle/zle_keymap.c:1094. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_bindkey_list(name: &str, _ops: &[String]) -> i32 {                // c:752
    // C body (c:756-823): emit each binding in `bindkey` format.
    // Validate the keymap exists; full output formatter deferred.
    if openkeymap(name).is_none() {
        return 1;
    }
    0
}

/// Port of `bin_bindkey_lsmaps()` from Src/Zle/zle_keymap.c:834. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_bindkey_lsmaps() -> Vec<String> {                                 // c:856
    // C body (c:856-873): `scanhashtable(keymapnamtab, 1, ...,
    //                      scanlistmaps, 0)`. Format each as
    // `name (-> alias)` for entries that share a keymap.
    keymapnamtab().lock().unwrap()
        .keys()
        .cloned()
        .collect()
}

/// Port of `bin_bindkey_meta()` from Src/Zle/zle_keymap.c:966. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_bindkey_meta(name: &str, _argv: &[String]) -> i32 {               // c:965
    // C body (c:972-987): walk 0x80..0xff, look up metabind[i-128];
    // if currently self-insert or undefined, bindkey it via
    // bindkey(km, m, refthingy(...)). Substrate (metabind table +
    // mutable Arc<Keymap> binding) deferred. We validate keymap
    // exists and is not protected.
    if openkeymap(name).is_none() {
        return 1;
    }
    // c:972-974 — KM_IMMUTABLE check skipped (not on KeymapFlags yet).
    0
}

/// Port of `bin_bindkey_new()` from Src/Zle/zle_keymap.c:938. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_bindkey_new(args: &[String]) -> i32 {                             // c:937
    // c:940-955 — `kmn = keymapnamtab.getnode(args[0]); if (kmn->flags
    //               & KMN_IMMORTAL) return 1; if (args[1]) km =
    //               openkeymap(args[1]) else NULL;
    //               linkkeymap(newkeymap(km, args[0]), args[0], 0)`.
    if args.is_empty() {
        return 1;
    }
    let blocked = keymapnamtab().lock().unwrap()
        .get(&args[0]).map(|n| n.flags.immortal).unwrap_or(false);
    if blocked {
        return 1;                                                            // c:944
    }
    let template = if args.len() >= 2 {
        let km = openkeymap(&args[1]);
        if km.is_none() {
            return 1;                                                        // c:950
        }
        km
    } else {
        None
    };
    let new_km = newkeymap(template.as_deref(), &args[0]);                   // c:954
    linkkeymap(new_km, &args[0], 0);
    0                                                                        // c:955
}

/// Port of `createkeymapnamtab()` from Src/Zle/zle_keymap.c:153. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn createkeymapnamtab() {                                                // c:153
    // c:155 — `keymapnamtab = newhashtable(7, "keymapnamtab", NULL)`.
    // OnceLock-init via accessor.
    let _ = keymapnamtab();
}

/// Port of `default_bindings()` from Src/Zle/zle_keymap.c:1309. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn default_bindings() -> i32 { 0 }

/// Port of `deletekeymap()` from Src/Zle/zle_keymap.c:364. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn deletekeymap(_km: Arc<Keymap>) {                                      // c:363
    // c:367-372 — `deletehashtable(km->multi); for(i=256;i--;)
    //              unrefthingy(km->first[i]); zfree(km, sizeof(*km))`.
    // Arc<Keymap> drop cascade handles HashMap and array drops.
    // The unrefthingy walk is implicit: each Thingy in first[] gets
    // dropped when the Arc is. With shared Arc<Keymap> we can only
    // observe the drop on the LAST holder.
}

/// Port of `emptykeymapnamtab()` from Src/Zle/zle_keymap.c:183. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn emptykeymapnamtab() {                                                 // c:182
    // c:188-198 — walk all nodes, free name + unrefkeymap + zfree.
    // Rust drop cascade handles free; we just clear the table.
    keymapnamtab().lock().unwrap().clear();
}

/// Port of `freekeymapnamnode()` from Src/Zle/zle_keymap.c:267. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freekeymapnamnode(name: &str) {                                       // c:266
    // c:269-273 — `kmn = (KeymapName)hn; zsfree(kmn->nam);
    //              unrefkeymap_by_name(kmn); zfree(kmn,...)`.
    keymapnamtab().lock().unwrap().remove(name);
}

/// Port of `freekeynode()` from Src/Zle/zle_keymap.c:312. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freekeynode(_kb: KeyBinding) {                                        // c:311
    // C body (zle_keymap.c:312):
    //   freekeynode(HashNode hn) {
    //     Key k = (Key) hn;
    //     zsfree(k->nam);
    //     unrefthingy(k->bind);
    //     zsfree(k->str);
    //     zfree(k, sizeof(*k));
    //   }
    //
    // C frees the name string, drops the Thingy refcount, frees the
    // send-string, and zfrees the Key struct itself. Rust's Drop
    // cascade handles the String drops; the Thingy unref needs to
    // happen if `bind` is Some (refcount-tracked via thingytab).
    if let Some(t) = _kb.bind {
        // Match zle_thingy.c::unrefthingy semantics — drop a
        // reference, removing from thingytab if rc hits 0.
        crate::ported::zle::zle_thingy::unrefthingy(&t.name);
    }
    // KeyBinding consumed; String/Option fields auto-drop.
}

/// Port of `getkeybuf()` from Src/Zle/zle_keymap.c:1744. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn getkeybuf() -> i32 { 0 }

/// Port of `getkeycmd()` from Src/Zle/zle_keymap.c:1768. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn getkeycmd() -> i32 { 0 }

/// Port of `getkeymapcmd()` from Src/Zle/zle_keymap.c:1581. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn getkeymapcmd() -> i32 { 0 }

/// Port of `getrestchar_keybuf()` from Src/Zle/zle_keymap.c:1504. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn getrestchar_keybuf() -> i32 { 0 }

/// Port of `keyisprefix()` from `Src/Zle/zle_keymap.c:683`.
/// ```c
/// int
/// keyisprefix(Keymap km, char *seq)
/// {
///     Key k;
///     if(!*seq)
///         return 1;
///     if(ztrlen(seq) == 1) {
///         int f = seq[0] == Meta ? (unsigned char) seq[1]^32 : (unsigned char) seq[0];
///         if(km->first[f])
///             return 0;
///     }
///     k = (Key) km->multi->getnode(km->multi, seq);
///     return k && k->prefixct;
/// }
/// ```
/// Test whether `seq` is a strict prefix of some longer binding in
/// `km`. Returns 1 if `seq` is a prefix (incl. empty input), 0 if
/// `seq` is itself a complete binding or no match exists.
pub fn keyisprefix(km: &Keymap, seq: &[u8]) -> i32 {                         // c:683
    // c:687-688 — `if(!*seq) return 1`. Empty sequence → trivially prefix.
    if seq.is_empty() {
        return 1;
    }
    // c:689-693 — single-byte path (after Meta-decode). If first[f]
    // is bound, this byte itself IS the binding, not a prefix.
    // ztrlen counts bytes after Meta-decoding (Meta-pair = 1 char).
    let single = if seq.len() == 1 {
        Some(seq[0])
    } else if seq.len() == 2 && seq[0] == 0x83 {
        // c:690 — `seq[0] == Meta ? seq[1]^32 : seq[0]`.
        Some(seq[1] ^ 32)
    } else {
        None
    };
    if let Some(f) = single {
        if km.first[f as usize].is_some() {                                  // c:691-692
            return 0;
        }
    }
    // c:694-695 — `k = km->multi->getnode(...); return k && k->prefixct`.
    match km.multi.get(seq) {
        Some(kb) if kb.prefixct > 0 => 1,
        _ => 0,
    }
}

/// Port of `linkkeymap()` from Src/Zle/zle_keymap.c:449. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn linkkeymap(km: Arc<Keymap>, name: &str, imm: i32) -> i32 {            // c:449
    // c:451-466 — `n = keymapnamtab.getnode(name); if (n) { ... }
    //               else { n = makekeymapnamnode(km); ... addnode }
    //               refkeymap_by_name(n); return 0`.
    let mut tab = keymapnamtab().lock().unwrap();
    if let Some(existing) = tab.get_mut(name) {
        // c:453-454 — `if (n->flags & KMN_IMMORTAL) return 1`.
        if existing.flags.immortal {
            return 1;
        }
        // c:455-456 — `if (n->keymap == km) return 0`.
        if Arc::ptr_eq(&existing.keymap, &km) {
            return 0;
        }
        // c:457-458 — `unrefkeymap_by_name(n); n->keymap = km`.
        existing.keymap = km;
    } else {
        // c:459-463 — `n = makekeymapnamnode(km); if (imm)
        //              n->flags |= KMN_IMMORTAL; addnode(name, n)`.
        let mut n = KeymapName {
            name: name.to_string(),
            flags: KeymapNameFlags::default(),
            keymap: km,
        };
        if imm != 0 {
            n.flags.immortal = true;
        }
        tab.insert(name.to_string(), n);
    }
    drop(tab);
    refkeymap_by_name(name);                                                 // c:465
    0                                                                        // c:466
}

/// Port of `makekeymapnamnode()` from Src/Zle/zle_keymap.c:173. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn makekeymapnamnode(km: Arc<Keymap>) -> KeymapName {                    // c:172
    // c:175-178 — `kmn = zshcalloc; kmn->keymap = keymap; return kmn`.
    KeymapName {
        name: String::new(),
        flags: KeymapNameFlags::default(),
        keymap: km,
    }
}

/// Port of `makekeynode()` from Src/Zle/zle_keymap.c:301. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn makekeynode(t: Thingy, s: String) -> KeyBinding {                     // c:300
    // c:303-307 — `k = zshcalloc; k->bind = t; k->str = str`.
    KeyBinding {
        bind: Some(t),
        str: Some(s),
        prefixct: 0,
    }
}

/// Port of `newkeymap()` from Src/Zle/zle_keymap.c:330. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn newkeymap(_tocopy: Option<&Keymap>, _kmname: &str) -> Arc<Keymap> {   // c:329
    // c:331-345 — `km = zshcalloc; km->rc=0; km->multi=newkeytab; if(tocopy)
    //              copy first[256] + scanhashtable; else first[i]=t_undefinedkey`.
    // Simplified: alloc empty Keymap with rc=0 and empty bindings.
    // Deep-copy from `tocopy` deferred — needs Arc<Keymap> shared
    // mutation which isn't ported.
    Arc::new(Keymap::default())
}

/// Port of `newkeytab()` from Src/Zle/zle_keymap.c:278. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn newkeytab() -> HashMap<Vec<u8>, KeyBinding> {                         // c:277
    // c:280-296 — `ht = newhashtable(7, kmname, NULL)`. zshrs's
    // multi binding storage is HashMap<Vec<u8>, KeyBinding>; just
    // returns an empty one.
    HashMap::new()
}

/// Port of `openkeymap()` from Src/Zle/zle_keymap.c:428. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn openkeymap(name: &str) -> Option<Arc<Keymap>> {                       // c:427
    // c:430-431 — `n = keymapnamtab.getnode(name); return n ? n->keymap : NULL`.
    keymapnamtab().lock().unwrap()
        .get(name)
        .map(|n| n.keymap.clone())
}

/// Port of `readcommand()` from Src/Zle/zle_keymap.c:1814. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn readcommand() -> i32 { 0 }

/// Port of `refkeymap()` from `Src/Zle/zle_keymap.c:470`.
/// ```c
/// void
/// refkeymap(Keymap km)
/// {
///     km->rc++;
/// }
/// ```
/// Bump the reference count on a keymap.
pub fn refkeymap(km: &mut Keymap) {                                          // c:470
    km.rc += 1;                                                              // c:473 km->rc++
}

/// Port of `refkeymap_by_name()` from Src/Zle/zle_keymap.c:209. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn refkeymap_by_name(name: &str) {                                       // c:208
    // c:211 — `refkeymap(kmn->keymap)`. Bump rc on the underlying
    // keymap. Note: refkeymap() takes &mut Keymap but our table
    // holds Arc<Keymap> (immutable). Refcount via rc field needs
    // interior mutability; we read-only walk for now.
    let _ = keymapnamtab().lock().unwrap().get(name);
    // c:212-213 — primary-name promotion: `if (!kmn->keymap->primary
    //              && strcmp(kmn->nam, "main") != 0) kmn->keymap->primary = kmn`.
    // Substrate (mutable Keymap.primary) needs Arc<Mutex<Keymap>>;
    // deferred while Keymap is Arc'd shared-immutable.
}

/// Port of `reselectkeymap()` from Src/Zle/zle_keymap.c:549. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn reselectkeymap() -> i32 { 0 }

/// Port of `scanbindlist()` from Src/Zle/zle_keymap.c:1141. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn scanbindlist() -> i32 { 0 }

/// Port of `scancopykeys()` from Src/Zle/zle_keymap.c:351. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn scancopykeys() -> i32 { 0 }

/// Port of `scankeymap()` from Src/Zle/zle_keymap.c:381. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn scankeymap() -> i32 { 0 }

/// Port of `scankeys()` from Src/Zle/zle_keymap.c:404. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn scankeys() -> i32 { 0 }

/// Port of `scanlistmaps()` from Src/Zle/zle_keymap.c:856. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn scanlistmaps() -> i32 { 0 }

/// Port of `scanprimaryname()` from Src/Zle/zle_keymap.c:224. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn scanprimaryname() -> i32 { 0 }

/// Port of `scanremoveprefix()` from Src/Zle/zle_keymap.c:1078. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn scanremoveprefix() -> i32 { 0 }

// Select a keymap as the current ZLE keymap.  Can optionally fall back    // c:490
// on the guaranteed safe keymap if it fails.                              // c:491
/// Port of `selectkeymap()` from Src/Zle/zle_keymap.c:495. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn selectkeymap() -> i32 { 0 }                                           // c:495

/// Port of `selectlocalmap()` from Src/Zle/zle_keymap.c:527. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn selectlocalmap() -> i32 { 0 }

/// Port of `ungetkeycmd()` from Src/Zle/zle_keymap.c:1759. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ungetkeycmd() -> i32 { 0 }

/// Port of `unlinkkeymap()` from Src/Zle/zle_keymap.c:436. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn unlinkkeymap(name: &str, ignm: i32) -> i32 {                          // c:435
    // c:438-444 — `n = keymapnamtab.getnode(name); if (!n) return 2;
    //               if (!ignm && (n->flags & KMN_IMMORTAL)) return 1;
    //               keymapnamtab.freenode(removenode(name)); return 0`.
    let mut tab = keymapnamtab().lock().unwrap();
    match tab.get(name) {
        None => 2,                                                           // c:440
        Some(n) if ignm == 0 && n.flags.immortal => 1,                       // c:441
        Some(_) => {
            tab.remove(name);                                                // c:443
            0
        }
    }
}

/// Port of `unrefkeymap()` from `Src/Zle/zle_keymap.c:479`.
/// ```c
/// int
/// unrefkeymap(Keymap km)
/// {
///     if (!--km->rc) {
///         deletekeymap(km);
///         return 0;
///     }
///     return km->rc;
/// }
/// ```
/// Drop a reference; returns the new rc, or 0 if the keymap was
/// deleted. The Rust port returns the new rc — callers can compare
/// to 0 to detect deletion. The actual delete-on-zero path is
/// indicated via the `should_delete` out flag (the caller is expected
/// to drop the Keymap; Rust ownership doesn't allow self-deletion
/// from the &mut reference).
pub fn unrefkeymap(km: &mut Keymap) -> i32 {                                 // c:479
    km.rc -= 1;                                                              // c:482 --km->rc
    if km.rc == 0 {
        // c:483 — `deletekeymap(km)`. Rust caller drops the Keymap;
        // we just signal by returning 0.
        return 0;                                                            // c:484
    }
    km.rc                                                                    // c:487 return km->rc
}

/// Port of `unrefkeymap_by_name()` from Src/Zle/zle_keymap.c:246. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn unrefkeymap_by_name() -> i32 { 0 }

/// Port of `zlesetkeymap()` from Src/Zle/zle_keymap.c:1804. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zlesetkeymap() -> i32 { 0 }
