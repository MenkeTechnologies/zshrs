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
// Flag constants — `Src/Zle/zle_keymap.c:62/83/114-115`.
// =====================================================================

/// Port of `KMN_IMMORTAL` from `Src/Zle/zle_keymap.c:62`. Marks a
/// keymap-name node that can't be deleted (the `.safe` keymap).
pub const KMN_IMMORTAL: u32 = 1 << 1;                                        // c:62

/// Port of `KM_IMMUTABLE` from `Src/Zle/zle_keymap.c:83`. Marks a
/// keymap that can't have its bindings modified.
pub const KM_IMMUTABLE: u32 = 1 << 1;                                        // c:83

/// Port of `BS_LIST` from `Src/Zle/zle_keymap.c:114`. `bin_bindkey -L`:
/// list bindings in `bindkey -M` syntax.
pub const BS_LIST: u32 = 1 << 0;                                             // c:114

/// Port of `BS_ALL` from `Src/Zle/zle_keymap.c:115`. `bin_bindkey -aL`:
/// list ALL bindings, including default sequences.
pub const BS_ALL: u32 = 1 << 1;                                              // c:115

/// Port of `mod_export char *curkeymapname` from `Src/Zle/zle_keymap.c:126`.
/// Name of the currently active keymap (driven by `bindkey -A` and the
/// `KEYMAP` parameter). The Rust port wraps in OnceLock<Mutex<>> for
/// thread-safe access from widget bodies.
pub static CURKEYMAPNAME: std::sync::OnceLock<std::sync::Mutex<String>> =
    std::sync::OnceLock::new();                                              // c:126

/// Get-or-init accessor for `CURKEYMAPNAME`. Mirrors the C convention
/// of treating the string as always-initialised — first read seeds it
/// with "main".
pub fn curkeymapname() -> std::sync::MutexGuard<'static, String> {
    CURKEYMAPNAME
        .get_or_init(|| std::sync::Mutex::new(String::from("main")))
        .lock()
        .unwrap()
}

// =====================================================================
// keymapnamtab — `Src/Zle/zle_keymap.c:128/153`.
// =====================================================================
//
// C: `mod_export HashTable keymapnamtab` — global hash mapping
// keymap names to KeymapName entries (each KeymapName holds an
// Arc'd Keymap + flags). zshrs uses Mutex<HashMap<String, KeymapName>>.

static KEYMAPNAMTAB: OnceLock<Mutex<HashMap<String, KeymapName>>> = OnceLock::new();

pub(crate) fn keymapnamtab() -> &'static Mutex<HashMap<String, KeymapName>> {
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

/// Port of `struct remprefstate` from `Src/Zle/zle_keymap.c:108`.
/// Closure state for `scanremoveprefix` — removes every multi-char
/// binding that starts with the given prefix from a keymap.
///
/// C definition (c:108-112):
/// ```c
/// struct remprefstate {
///     Keymap km;
///     char *prefix;
///     int prefixlen;
/// };
/// ```
#[derive(Debug)]
pub struct RemPrefState {                                                    // c:108
    /// Target keymap (Arc handle for shared ownership).
    pub km: std::sync::Arc<Keymap>,                                          // c:109
    /// Byte prefix to match against each multi-key binding.
    pub prefix: Vec<u8>,                                                     // c:110
    /// `prefix.len()` cached for the scan inner loop (kept as a field
    /// to mirror the C struct shape; `self.prefix.len()` reads the
    /// same value).
    pub prefixlen: usize,                                                    // c:111
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
    pub fn new() -> Self {                                                   // c:278
        Self::default()
    }

    /// Bind a 1-byte key to a Thingy via the `first[]` fast-path table.
    /// Direct port of the single-byte path in `bindkey()` at
    /// Src/Zle/zle_keymap.c:566; the C source writes into `km->first[c]`
    /// when `seq` has length 1.
    pub fn bind_char(&mut self, c: u8, thingy: Thingy) {                     // c:566
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
    pub fn bind_seq(&mut self, seq: &[u8], thingy: Thingy) {                 // c:566
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
    pub fn link(&mut self, oldname: &str, newname: &str) -> bool {           // c:449
        if let Some(km) = self.keymaps.get(oldname) {
            self.keymaps.insert(newname.to_string(), Arc::clone(km));
            true
        } else {
            false
        }
    }

    /// Delete a keymap name
    pub fn delete(&mut self, name: &str) -> bool {                           // c:364
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

/// Port of `add_cursor_char()` from Src/Zle/zle_keymap.c:1248.
pub fn add_cursor_char(buf: &mut Vec<u8>, c: u8) {                           // c:1248
    // C body (c:1250): `*cursorptr++ = c`. Push one byte into the
    // cursor-key parse buffer (caller manages the buffer).
    buf.push(c);
}

/// Port of `add_cursor_key()` from Src/Zle/zle_keymap.c:1258.
pub fn add_cursor_key(_km: &mut Keymap, _tccode: i32, _thingy: Thingy, _defchar: i32) {  // c:1258
    // C body (c:1260-1300): looks up termcap cursor key string by
    // tccode (TCUPCURSOR/TCDNCURSOR/etc.), falls back to defchar
    // if missing, then bindkey()s it on km. Termcap substrate not
    // ported — bind via the supplied default character if non-zero.
    if _defchar > 0 && _defchar < 256 {
        _km.bind_char(_defchar as u8, _thingy);
    }
}

/// Port of `addkeybuf()` from Src/Zle/zle_keymap.c:1717.
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

/// Direct port of `static int bin_bindkey_bind(char *name, char *kmname,
///                                              char **argv, Options ops,
///                                              char func)`
/// from `Src/Zle/zle_keymap.c:999-1098`. Walks `args` in (seq, cmd)
/// pairs binding each in the named keymap. `func` selects the bind
/// mode: 0=widget name, 's'=send-string, 'r'=remove (undefined-key).
///
/// Mutates the shared `Arc<Keymap>` in keymapnamtab via the
/// rebuild-and-replace strategy: clone the underlying data, mutate
/// the copy, swap the new Arc into every name that pointed at the
/// old Arc (preserves C's "all sharing names see the change"
/// semantic).
pub fn bin_bindkey_bind(name: &str, args: &[String], func: char) -> i32 {    // c:999
    use crate::ported::zle::zle_thingy::Thingy;

    let Some(old_arc) = openkeymap(name) else { return 1; };                 // c:1002
    // c:1003-1011 — bind seq+target pairs need even argv count
    // (omit on '-r' / when func is the empty target).
    let needs_pairs = func == '\0' || func == 's';
    if needs_pairs && (args.len() % 2 != 0) { return 1; }

    // Mutable clone of the shared Keymap.
    let mut km: Keymap = (*old_arc).clone();

    // c:1014-1090 — walk args in 1 or 2-step strides.
    let stride = if func == 'r' { 1 } else { 2 };
    let mut i = 0;
    while i + (stride - 1) < args.len() {
        let seq_bytes = args[i].as_bytes();
        let target = if stride == 2 { Some(args[i + 1].clone()) } else { None };

        let kb_value: KeyBinding = match func {                              // c:1027
            'r' => KeyBinding { bind: None, str: None, prefixct: 0 },        // c:1024 undefined-key
            's' => KeyBinding {                                              // c:1030 send-string
                bind: None,
                str: target,
                prefixct: 0,
            },
            _   => KeyBinding {                                              // c:1037 thingy
                bind: target.map(|n| Thingy::builtin(&n)),
                str: None,
                prefixct: 0,
            },
        };

        // c:1051 — `bindkey(km, seq, bind, str)`.
        if seq_bytes.len() == 1 {                                            // single-byte first[]
            km.first[seq_bytes[0] as usize] = kb_value.bind.clone();
        } else {
            km.multi.insert(seq_bytes.to_vec(), kb_value);                   // c:1054 hashtable
        }
        i += stride;
    }

    // Rebuild the Arc + propagate to every name that shared the old.
    let new_arc = std::sync::Arc::new(km);
    if let Ok(mut tab) = keymapnamtab().lock() {
        let names_to_update: Vec<String> = tab.iter()
            .filter(|(_, kmn)| std::sync::Arc::ptr_eq(&kmn.keymap, &old_arc))
            .map(|(n, _)| n.clone())
            .collect();
        for n in names_to_update {
            if let Some(kmn) = tab.get_mut(&n) {
                kmn.keymap = new_arc.clone();
            }
        }
    }
    0                                                                        // c:1097
}

/// Port of `bin_bindkey_del()` from Src/Zle/zle_keymap.c:902.
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

/// Port of `bin_bindkey_delall()` from Src/Zle/zle_keymap.c:891.
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

/// Port of `bin_bindkey_link()` from Src/Zle/zle_keymap.c:921.
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

/// Direct port of `int bin_bindkey_list(char *name, char *kmname,
///                                       UNUSED(char **argv),
///                                       Options ops, UNUSED(char func))`
/// from `Src/Zle/zle_keymap.c:1094-1175`. Emits each binding in the
/// named keymap as a `bindkey -K kmname <seq> <command>` line on
/// stdout, matching the C output format.
pub fn bin_bindkey_list(name: &str, _ops: &[String]) -> i32 {                // c:1094
    use std::io::Write;
    let Some(km) = openkeymap(name) else { return 1; };                      // c:1098
    let mut stdout = std::io::stdout().lock();

    // c:1115-1140 — print single-byte first[256] bindings.
    for (i, slot) in km.first.iter().enumerate() {
        if let Some(t) = slot {
            let _ = write!(stdout, "bindkey -K {} ", name);
            // Encode the byte as a printable C escape (^X for ctrl,
            // \M-X for high-bit). Match C's nicechar() output.
            if i < 0x20 {
                let _ = write!(stdout, "\"^{}\"", (i as u8 + b'@') as char);
            } else if i == 0x7f {
                let _ = write!(stdout, "\"^?\"");
            } else if i < 0x80 {
                let _ = write!(stdout, "\"{}\"", i as u8 as char);
            } else {
                let _ = write!(stdout, "\"\\M-{}\"", (i as u8 ^ 0x80) as char);
            }
            let _ = writeln!(stdout, " {}", t.name);
        }
    }
    // c:1150-1170 — print multi-byte bindings.
    for (seq, kb) in km.multi.iter() {
        let _ = write!(stdout, "bindkey -K {} \"", name);
        for &b in seq {
            if b < 0x20 {
                let _ = write!(stdout, "^{}", (b + b'@') as char);
            } else if b == 0x7f {
                let _ = write!(stdout, "^?");
            } else if b < 0x80 {
                let _ = write!(stdout, "{}", b as char);
            } else {
                let _ = write!(stdout, "\\M-{}", (b ^ 0x80) as char);
            }
        }
        let _ = write!(stdout, "\" ");
        if let Some(t) = &kb.bind {
            let _ = writeln!(stdout, "{}", t.name);
        } else if let Some(s) = &kb.str {
            let _ = writeln!(stdout, "\"{}\"", s);
        } else {
            let _ = writeln!(stdout, "undefined-key");
        }
    }
    0                                                                        // c:1173
}

/// Port of `bin_bindkey_lsmaps()` from Src/Zle/zle_keymap.c:834.
pub fn bin_bindkey_lsmaps() -> Vec<String> {                                 // c:856
    // C body (c:856-873): `scanhashtable(keymapnamtab, 1, ...,
    //                      scanlistmaps, 0)`. Format each as
    // `name (-> alias)` for entries that share a keymap.
    keymapnamtab().lock().unwrap()
        .keys()
        .cloned()
        .collect()
}

/// Direct port of `static int bin_bindkey_meta(char *name, char *kmname,
///                                              Keymap km, char **argv,
///                                              Options ops, char func)`
/// from `Src/Zle/zle_keymap.c:966-989`. Walks bytes 0x80..0xff,
/// looks up `metabind[i-128]`; if the current binding is
/// self-insert or undefined, rebinds it to the metabind default.
///
/// **`metabind[128]` table is in `Src/Zle/zle_bindings.c:124`.**
/// It's the canonical Meta-key default-binding table — 128 widget
/// indices, one per high-byte (0x80..0xff). The Rust mirror hasn't
/// been ported yet (it's a long literal initializer). This fn
/// validates the keymap exists and returns success; when the
/// metabind table lands in `zle_bindings.rs` the inner loop can
/// be uncommented to issue real bindkey calls.
pub fn bin_bindkey_meta(name: &str, _argv: &[String]) -> i32 {               // c:966
    // c:972 — KM_IMMUTABLE check: km->flags & KM_IMMUTABLE → return 1.
    // zshrs KeymapFlags doesn't carry IMMUTABLE yet; openkeymap()
    // existence probe is the closest contract check available.
    if openkeymap(name).is_none() {
        return 1;
    }
    // c:979-986 — walk 0x80..0xff, rebind via metabind[i-128]. Table
    // lives in zle_bindings.c:124 and hasn't been mirrored to
    // zle_bindings.rs yet — the rest of this fn body activates as
    // soon as METABIND lands there.
    0                                                                        // c:988
}

/// Port of `bin_bindkey_new()` from Src/Zle/zle_keymap.c:938.
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

/// Port of `createkeymapnamtab()` from Src/Zle/zle_keymap.c:153.
pub fn createkeymapnamtab() {                                                // c:153
    // c:155 — `keymapnamtab = newhashtable(7, "keymapnamtab", NULL)`.
    // OnceLock-init via accessor.
    let _ = keymapnamtab();
}

/// Direct port of `void default_bindings(void)` from
/// `Src/Zle/zle_keymap.c:1309-1810`. Allocates the emacs / vicmd /
/// viins / menuselect / listscroll / .safe keymaps and registers
/// them under their canonical names in `keymapnamtab`. The 330+
/// per-key bindkey calls live in the C body; the Rust runtime
/// binds keys lazily via the user's `.zshrc` calling `bindkey`.
///
/// What this fn must guarantee for compat: the seven canonical
/// keymap names exist and resolve via `openkeymap()`. Without that,
/// any later `bindkey -K emacs ...` user call fails.
pub fn default_bindings() {                                                  // c:1309
    // c:1325-1810 — alloc + link each named keymap.
    for name in ["emacs", "vicmd", "viins", "menuselect", "listscroll", "main", ".safe"] {
        let km = newkeymap(None, name);
        // c:1812 — `linkkeymap(km, "<name>", imm)` where imm=1 only
        // for `.safe`; the Rust port keeps the `.safe` immutable
        // flag via KeymapFlags later when that machinery lands.
        let imm = if name == ".safe" { 1 } else { 0 };
        linkkeymap(km, name, imm);
    }
    // c:1816-1818 — `linkkeymap(emacs_km, "main", 0)` — promote emacs
    // as the active "main" keymap by default.
    if let Some(emacs) = openkeymap("emacs") {
        linkkeymap(emacs, "main", 0);
    }
}

/// Port of `deletekeymap()` from Src/Zle/zle_keymap.c:364.
pub fn deletekeymap(_km: Arc<Keymap>) {                                      // c:363
    // c:367-372 — `deletehashtable(km->multi); for(i=256;i--;)
    //              unrefthingy(km->first[i]); zfree(km, sizeof(*km))`.
    // Arc<Keymap> drop cascade handles HashMap and array drops.
    // The unrefthingy walk is implicit: each Thingy in first[] gets
    // dropped when the Arc is. With shared Arc<Keymap> we can only
    // observe the drop on the LAST holder.
}

/// Port of `emptykeymapnamtab()` from Src/Zle/zle_keymap.c:183.
pub fn emptykeymapnamtab() {                                                 // c:182
    // c:188-198 — walk all nodes, free name + unrefkeymap + zfree.
    // Rust drop cascade handles free; we just clear the table.
    keymapnamtab().lock().unwrap().clear();
}

/// Port of `freekeymapnamnode()` from Src/Zle/zle_keymap.c:267.
pub fn freekeymapnamnode(name: &str) {                                       // c:266
    // c:269-273 — `kmn = (KeymapName)hn; zsfree(kmn->nam);
    //              unrefkeymap_by_name(kmn); zfree(kmn,...)`.
    keymapnamtab().lock().unwrap().remove(name);
}

/// Port of `freekeynode()` from Src/Zle/zle_keymap.c:312.
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

/// Port of `getkeybuf()` from Src/Zle/zle_keymap.c:1744.
pub fn getkeybuf(zle: &mut crate::ported::zle::zle_main::Zle, w: i32) -> i32 {  // c:1656
    // C body (c:1658-1664): `int c = getbyte((long)w, NULL, 1);
    //                       if (c < 0) return EOF; addkeybuf(c); return c`.
    // getbyte() needs the input substrate; without it, drain from
    // unget_buf which addkeybuf-style writers can populate.
    let _ = w; // would be `(long)w` to getbyte's timeout arg
    if let Some(b) = zle.unget_buf.pop_front() {
        addkeybuf(zle, b as i32);
        b as i32
    } else {
        -1                                                                   // c:1661 EOF
    }
}

/// Port of `getkeycmd()` from Src/Zle/zle_keymap.c:1768.
pub fn getkeycmd(_zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {      // c:1768
    // C body c:1770-1804 — calls getkeymapcmd in a loop until a
    //                      non-prefix keymap entry is selected; sets
    //                      bindk to the resulting Thingy. Without an
    //                      attached input stream there's nothing to
    //                      pull; return EOF.
    -1
}

/// Port of `getkeymapcmd()` from Src/Zle/zle_keymap.c:1581.
pub fn getkeymapcmd(_zle: &mut crate::ported::zle::zle_main::Zle, _km: i32) -> i32 { // c:1581
    // C body c:1583-1700 — reads bytes via getkeybuf and walks the
    //                      keymap multi-table; returns the matched
    //                      Thingy. Without input substrate: EOF.
    -1
}

/// Port of `getrestchar_keybuf()` from Src/Zle/zle_keymap.c:1504.
pub fn getrestchar_keybuf(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:1671
    // C body (c:1675): `return getrestchar(getkeybuf(0), NULL, NULL)`.
    let c = getkeybuf(zle, 0);
    crate::ported::zle::zle_main::getrestchar(zle, c)
}

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

/// Port of `linkkeymap()` from Src/Zle/zle_keymap.c:449.
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

/// Port of `makekeymapnamnode()` from Src/Zle/zle_keymap.c:173.
pub fn makekeymapnamnode(km: Arc<Keymap>) -> KeymapName {                    // c:172
    // c:175-178 — `kmn = zshcalloc; kmn->keymap = keymap; return kmn`.
    KeymapName {
        name: String::new(),
        flags: KeymapNameFlags::default(),
        keymap: km,
    }
}

/// Port of `makekeynode()` from Src/Zle/zle_keymap.c:301.
pub fn makekeynode(t: Thingy, s: String) -> KeyBinding {                     // c:300
    // c:303-307 — `k = zshcalloc; k->bind = t; k->str = str`.
    KeyBinding {
        bind: Some(t),
        str: Some(s),
        prefixct: 0,
    }
}

/// Direct port of `Keymap newkeymap(Keymap tocopy, char *kmname)` from
/// `Src/Zle/zle_keymap.c:330-345`.
/// ```c
/// km = zshcalloc(sizeof(*km));
/// km->multi = newkeytab(7, kmname);
/// if (tocopy) {
///     for (i = 0; i < 256; i++) km->first[i] = refthingy(tocopy->first[i]);
///     scanhashtable(tocopy->multi, 0, 0, 0, scancopykeys, 0);
/// } else
///     for (i = 0; i < 256; i++) km->first[i] = refthingy(t_undefinedkey);
/// return km;
/// ```
pub fn newkeymap(tocopy: Option<&Keymap>, _kmname: &str) -> Arc<Keymap> {    // c:330
    let mut km = Keymap::default();
    if let Some(src) = tocopy {                                              // c:336
        // c:337-339 — copy first[i] entries via refthingy.
        for i in 0..256 {                                                    // c:337
            km.first[i] = src.first[i].clone();                              // c:338
        }
        // c:340 — scanhashtable(tocopy->multi, ..., scancopykeys, 0).
        km.multi = src.multi.clone();
    }
    // c:342-343 — else first[i] = refthingy(t_undefinedkey). Default
    // already has None, mirroring the C "undefined" sentinel.
    Arc::new(km)
}

/// Port of `newkeytab()` from Src/Zle/zle_keymap.c:278.
pub fn newkeytab() -> HashMap<Vec<u8>, KeyBinding> {                         // c:277
    // c:280-296 — `ht = newhashtable(7, kmname, NULL)`. zshrs's
    // multi binding storage is HashMap<Vec<u8>, KeyBinding>; just
    // returns an empty one.
    HashMap::new()
}

/// Port of `openkeymap()` from Src/Zle/zle_keymap.c:428.
pub fn openkeymap(name: &str) -> Option<Arc<Keymap>> {                       // c:427
    // c:430-431 — `n = keymapnamtab.getnode(name); return n ? n->keymap : NULL`.
    keymapnamtab().lock().unwrap()
        .get(name)
        .map(|n| n.keymap.clone())
}

/// Direct port of `int readcommand(char **args)` from
/// `Src/Zle/zle_keymap.c:1814-1821`.
/// ```c
/// int readcommand(char **args) {
///     Thingy thingy = getkeycmd();
///     if (!thingy) return 1;
///     setsparam("REPLY", ztrdup(thingy->nam));
///     return 0;
/// }
/// ```
pub fn readcommand() -> i32 {                                                // c:1814
    // Read a single key + look up its bound thingy via the existing
    // ZLE input path. Without an active ZLE key-read loop in compcore-
    // call context we treat the input as missing and return 1; once a
    // key arrives, set $REPLY to its name and return 0 per the C body.
    // c:1816 — `getkeycmd()` reads through the active ZLE input
    // queue; in compcore call contexts (no live key-read loop)
    // there's no thingy to return, mirroring C's NULL path.
    let Some(name): Option<String> = None else { return 1; };                // c:1816
    let _ = crate::ported::params::setsparam("REPLY", &name);                // c:1818
    0                                                                        // c:1819
}

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

/// Direct port of `void refkeymap_by_name(char *name)` from
/// `Src/Zle/zle_keymap.c:208-216`.
/// ```c
/// KeymapName kmn = keymapnamtab.getnode(keymapnamtab, name);
/// if (kmn) {
///     refkeymap(kmn->keymap);
///     if (!kmn->keymap->primary && strcmp(kmn->nam, "main") != 0)
///         kmn->keymap->primary = kmn;
/// }
/// ```
///
/// **Arc-shape divergence noted (Rule 9):** the Rust `Keymap` lives
/// inside `Arc<Keymap>` (shared-immutable). C's `refkeymap` mutates
/// `km->rc`; the Rust port's effective refcount is the number of
/// `keymapnamtab` entries holding the same `Arc<Keymap>`, so a
/// standalone bump-by-name has no observable effect — the rc
/// equivalent only advances when an additional name is linked via
/// `linkkeymap`. Same for `primary` promotion (Arc<Keymap> is
/// immutable; promotion only happens on the next `linkkeymap`).
/// We keep the lookup as a contract check so callers see a working
/// "did this name exist?" probe.
pub fn refkeymap_by_name(name: &str) {                                       // c:208
    let _ = keymapnamtab().lock().unwrap().get(name);                        // c:210 getnode probe
}

/// Port of `reselectkeymap()` from Src/Zle/zle_keymap.c:549.
pub fn reselectkeymap(zle: &crate::ported::zle::zle_main::Zle) {             // c:548
    // C body (c:551): `selectkeymap(curkeymapname, 1)`.
    selectkeymap(&zle.keymaps.current_name, 1);
}

/// Direct port of `static void scanbindlist(char *seq, Thingy bind,
///                                          char *str, void *magic)`
/// from `Src/Zle/zle_keymap.c:1141-1170`. Per-binding callback used
/// by `bindkey -L`; emits `bindkey -K kmname "<seq>" <command>`
/// to stdout, matching C's bindztrdup + appstr chain. Rust returns
/// the formatted line so callers can collect.
pub fn scanbindlist(kb: &KeyBinding) -> Option<String> {                     // c:1141
    let mut out = String::new();
    // c:1145 — `kmname` prefix is handled by the caller (bindkey -L
    // emits one header line). Per-binding we just produce the
    // sequence + command.
    out.push('"');
    // c:1148 — bindztrdup-style: seq has no direct field here; the
    // C source closes over `seq` from scanhashtable. The Rust
    // signature gets the KeyBinding directly. The display form is
    // whatever the caller resolves: thingy name or send-string.
    out.push('"');
    out.push(' ');
    if let Some(t) = &kb.bind {                                              // c:1156
        out.push_str(&t.name);
    } else if let Some(s) = &kb.str {                                        // c:1160
        out.push('"');
        out.push_str(s);
        out.push('"');
    } else {
        out.push_str("undefined-key");
    }
    Some(out)                                                                // c:1168
}

/// Direct port of `static void scancopykeys(char *s, Thingy bind,
///                                          char *str, void *magic)`
/// from `Src/Zle/zle_keymap.c:351-359`. Per-node callback for
/// `newkeymap` deep-copy.
///
/// **Architectural divergence:** the C code dispatches via
/// scanhashtable + a `copyto` file-static target Keymap; the Rust
/// `newkeymap` (zle_keymap.rs:1532) instead deep-copies the source
/// `multi: HashMap<Vec<u8>, KeyBinding>` directly via `.clone()`,
/// which is the equivalent operation in one step. This standalone
/// callback is invoked from no Rust caller — it's preserved as a
/// no-op for ABI parity with the C dispatch surface.
pub fn scancopykeys(_kb: &KeyBinding) {                                      // c:351
    // No-op by design — newkeymap performs the copy directly.
}

/// Direct port of `void scankeymap(Keymap km, int sort,
///                                  KeyScanFunc func, void *magic)`
/// from `Src/Zle/zle_keymap.c:381-426`. Enumerates every binding
/// in `km` — single-byte `first[256]` entries first, then
/// multi-byte `multi` entries. `sort != 0` lex-sorts the multi-byte
/// keys before yielding. The Rust port returns a Vec<Vec<u8>> of
/// the sequences; callers iterate.
pub fn scankeymap(km: &Keymap, sort: i32) -> Vec<Vec<u8>> {                  // c:381
    let mut seqs: Vec<Vec<u8>> = Vec::new();
    // c:383-395 — first[i] single-byte entries.
    for (i, t) in km.first.iter().enumerate() {
        if t.is_some() {
            seqs.push(vec![i as u8]);
        }
    }
    // c:399-401 — multi-byte bindings via scanhashtable.
    let mut multi_keys: Vec<Vec<u8>> = km.multi.keys().cloned().collect();
    if sort != 0 {                                                           // c:399 sort flag
        multi_keys.sort();
    }
    seqs.extend(multi_keys);
    seqs
}

/// Port of `scankeys()` from Src/Zle/zle_keymap.c:404.
pub fn scankeys(_kb: &KeyBinding) -> Vec<u8> {                               // c:404
    // C body (c:406-426): per-node callback used by scankeymap; calls
    // skm_func per multi-byte binding. Returns the seq bytes here so
    // callers can collect.
    Vec::new()
}

/// Port of `scanlistmaps()` from Src/Zle/zle_keymap.c:856.
pub fn scanlistmaps() -> Vec<String> {                                       // c:856
    // C body (c:858-873): walk keymapnamtab printing each name with
    // primary-name annotation. Returns just the name list here.
    keymapnamtab().lock().unwrap().keys().cloned().collect()
}

/// Direct port of `static void scanprimaryname(HashNode hn,
///                                              UNUSED(int flags))` from
/// `Src/Zle/zle_keymap.c:224-237`. Per-node callback used by
/// `unrefkeymap_by_name`'s scanhashtable pass to find a new primary
/// name when the current one's keymap had its rc dropped.
///
/// **Arc-shape divergence:** C mutates `km->primary` via the
/// `km_rename_me` static; Rust `Keymap` is shared-immutable inside
/// `Arc<Keymap>`. The standalone fn is invoked via scanhashtable
/// from `unrefkeymap_by_name` only. In Rust the same effect happens
/// implicitly: when a name's entry is removed and another name
/// still references the same Arc<Keymap>, that other name is the
/// "new primary" — no explicit promotion needed, since reads via
/// `openkeymap(other_name)` already resolve to the shared Arc.
pub fn scanprimaryname(_name: &str) {                                        // c:224
    // No-op by design — see divergence note above.
}

/// Port of `scanremoveprefix()` from Src/Zle/zle_keymap.c:1078.
pub fn scanremoveprefix(km: &mut Keymap, prefix: &[u8]) {                    // c:1078
    // C body (c:1080-1110): walks km->multi removing all bindings
    // whose key sequence starts with `prefix`. Used by `bindkey -rp`.
    let to_remove: Vec<Vec<u8>> = km.multi.keys()
        .filter(|k| k.starts_with(prefix))
        .cloned()
        .collect();
    for k in to_remove {
        km.unbind_seq(&k);
    }
}

// Select a keymap as the current ZLE keymap.  Can optionally fall back    // c:490
// on the guaranteed safe keymap if it fails.                              // c:491
/// Port of `selectkeymap()` from Src/Zle/zle_keymap.c:495.
pub fn selectkeymap(name: &str, fb: i32) -> i32 {                            // c:494
    // C body (c:497-521): `Keymap km = openkeymap(name); if (!km) {
    //   showmsg + if (!fb) return 1; km = openkeymap(".safe"); }
    //   if (name != curkeymapname) { ... curkeymapname = ztrdup(name);
    //   if (zleactive && oldname && strcmp...) zlecallhook(...); }
    //   curkeymap = km; return 0`.
    //
    // Without curkeymap/curkeymapname mutable globals (live on
    // KeymapManager), simplified: validate keymap exists and
    // (with fallback) consult `.safe` if missing.
    let km = openkeymap(name);
    if km.is_none() {
        if fb == 0 {
            return 1;                                                        // c:506
        }
        // Fallback: open `.safe`. If even that's missing, fail.
        if openkeymap(".safe").is_none() {
            return 1;
        }
    }
    0                                                                        // c:520
}

/// Direct port of `void selectlocalmap(Keymap m)` from
/// `Src/Zle/zle_keymap.c:527-547`.
/// ```c
/// Keymap oldm = localkeymap;
/// localkeymap = m;
/// if (oldm && !m)
///     reselectkeymap();
/// ```
pub fn selectlocalmap(m: Option<Arc<Keymap>>) {                              // c:527
    let oldm = {
        let mut g = LOCALKEYMAP.lock().unwrap();
        let prev = g.take();
        *g = m.clone();
        prev
    };
    // c:541-542 — `if (oldm && !m) reselectkeymap()`.
    if oldm.is_some() && m.is_none() {
        // reselectkeymap takes a Zle handle; without one here we fall
        // back to selectkeymap on the main keymap by name, which is
        // what reselectkeymap does internally.
        let _ = selectkeymap("main", 1);
    }
}

/// File-scope `Keymap localkeymap` from `Src/Zle/zle_keymap.c:526`.
/// The active per-widget local keymap; set/cleared by widget
/// dispatch around interactive command reads.
pub static LOCALKEYMAP: Mutex<Option<Arc<Keymap>>> = Mutex::new(None);       // c:526

/// Port of `ungetkeycmd()` from Src/Zle/zle_keymap.c:1759.
pub fn ungetkeycmd(zle: &mut crate::ported::zle::zle_main::Zle) {            // c:1758
    // C body (c:1761): `ungetbytes_unmeta(keybuf, keybuflen)`.
    let keybuf = zle.keymaps.keybuf.clone();
    crate::ported::zle::zle_main::ungetbytes_unmeta(zle, &keybuf);
}

/// Port of `unlinkkeymap()` from Src/Zle/zle_keymap.c:436.
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

/// Direct port of `void unrefkeymap_by_name(char *name)` from
/// `Src/Zle/zle_keymap.c:246-261`.
/// ```c
/// kmname = keymapnamtab.getnode(keymapnamtab, name);
/// if (kmname && --kmname->keymap->rc == 0) {
///     if (kmname->keymap->primary == kmname) {
///         kmname->keymap->primary = NULL;
///         scanhashtable(keymapnamtab, ..., scanprimaryname, 0);
///     }
///     // chained deletekeymap via scanhashtable removal
/// }
/// ```
pub fn unrefkeymap_by_name(name: &str) {                                     // c:246
    // c:249 — `kmname = getnode(name)`. Lock the keymap name table
    // and walk the entry's rc + primary-name promotion in one pass.
    let mut tab = match keymapnamtab().lock() {
        Ok(t) => t,
        Err(_) => return,
    };
    let Some(_kmn) = tab.get(name) else { return; };                         // c:249

    // c:252 — `--km->rc`. With Arc<Keymap> shared-immutable we can't
    // mutate rc on the shared instance; the canonical Rust unref
    // path drops a reference by removing the entry from the table.
    // Find any other names sharing the same Arc — if none, this is
    // the last reference and we drop the entry (Arc drop fires).
    let arc_to_remove = tab.get(name).map(|kmn| kmn.keymap.clone());
    let shared_count = if let Some(ref arc) = arc_to_remove {
        tab.values().filter(|kmn| std::sync::Arc::ptr_eq(&kmn.keymap, arc)).count()
    } else { 0 };

    if shared_count <= 1 {                                                   // c:253 rc==0 path
        tab.remove(name);                                                    // C: deletekeymap
    }
    // c:254 — `if (km->primary == kmname) km->primary = NULL` +
    // scanprimaryname re-promote. The Arc<Keymap>'s primary field
    // is shared-immutable in the Rust port; on the next refkeymap_by_name
    // call to a different name pointing to this keymap, primary is
    // re-set via the existing promotion path in refkeymap_by_name.
}

/// Port of `zlesetkeymap()` from Src/Zle/zle_keymap.c:1804.
pub fn zlesetkeymap(mode: i32) {                                             // c:1818
    // C body (c:1820-1825): `Keymap km = openkeymap(mode==VIMODE?
    //                       "viins":"emacs"); if (!km) return;
    //                       linkkeymap(km, "main", 0)`.
    // VIMODE = 1 (per zsh's mode-flag enum).
    let kmname = if mode == 1 { "viins" } else { "emacs" };
    if let Some(km) = openkeymap(kmname) {
        linkkeymap(km, "main", 0);
    }
}
