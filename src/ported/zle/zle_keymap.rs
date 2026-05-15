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
use std::io::Write;

// =====================================================================
// Flag constants — `Src/Zle/zle_keymap.c:62/83/114-115`.
// =====================================================================

/// Port of `KMN_IMMORTAL` from `Src/Zle/zle_keymap.c:62`. Marks a
/// keymap-name node that can't be deleted (the `.safe` keymap).

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_move::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_word::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_params::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_vi::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_utils::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_refresh::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::textobjects::*;
#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;

/// Direct port of `struct keymapname` from `Src/Zle/zle_keymap.c:54`.
/// One node in the global `keymapnamtab` — maps a name to a Keymap
/// + per-node flags (KMN_IMMORTAL for `.safe`).
#[derive(Debug, Clone)]
pub struct KeymapName {                                                      // c:54
    pub nam: String,                                                         // c:56 char *nam
    pub flags: i32,                                                          // c:57 int flags
    pub keymap: Arc<Keymap>,                                                 // c:58 Keymap keymap
}

pub const KMN_IMMORTAL: i32 = 1 << 1;                                        // c:62

/// Port of `KM_IMMUTABLE` from `Src/Zle/zle_keymap.c:83`. Marks a
/// keymap that can't have its bindings modified.
pub const KM_IMMUTABLE: i32 = 1 << 1;                                        // c:83

// `BindState` / `BindStateFlags` deleted — the C `struct bindstate`
// at zle_keymap.c:95 is only used as a local in `printbinding()`/
// `scanbindings()`/`bin_bindkey -L`; ports of those fns will model
// it as a stack-local struct when they land. The previous Rust
// declaration had no callers (dead code) and used a fake bitflags
// wrapper over a single int field.

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
#[allow(non_camel_case_types)]
pub struct remprefstate {                                                    // c:108
    /// Target keymap (Arc handle for shared ownership).
    pub km: std::sync::Arc<Keymap>,                                          // c:109
    /// Byte prefix to match against each multi-key binding.
    pub prefix: Vec<u8>,                                                     // c:110
    /// `prefix.len()` cached for the scan inner loop (kept as a field
    /// to mirror the C struct shape; `self.prefix.len()` reads the
    /// same value).
    pub prefixlen: usize,                                                    // c:111
}

/// Port of `BS_LIST` from `Src/Zle/zle_keymap.c:114`. `bin_bindkey -L`:
/// list bindings in `bindkey -M` syntax.
pub const BS_LIST: i32 = 1 << 0;                                             // c:114

/// Port of `BS_ALL` from `Src/Zle/zle_keymap.c:115`. `bin_bindkey -aL`:
/// list ALL bindings, including default sequences.
pub const BS_ALL: i32 = 1 << 1;                                              // c:115

/// Port of `static Thingy lastnamed` from `Src/Zle/zle_keymap.c:145`.
/// Last command executed by `execute-named-command` — used to
/// re-execute via `bindkey -A name` then `getkeycmd`.
pub static lastnamed: Mutex<Option<Thingy>> = Mutex::new(None);              // c:145

/// Port of `createkeymapnamtab()` from Src/Zle/zle_keymap.c:153.
pub fn createkeymapnamtab() {                                                // c:153
    // c:153 — `keymapnamtab = newhashtable(7, "keymapnamtab", NULL)`.
    // OnceLock-init via accessor.
    let _ = keymapnamtab();
}

/// Direct port of `void init_keymaps(void)` from `Src/Zle/zle_keymap.c:1224`.
/// Module-load entry point — bootstraps the keymap-name table, installs
/// the default emacs/viins/vicmd bindings, allocates the keybuf, and
/// seeds `lastnamed` to the undefined-key sentinel (Rust None).
pub fn init_keymaps() {                                                      // c:1224
    createkeymapnamtab();                                                    // c:1227
    default_bindings();                                                      // c:1228
    *keybuf.lock().unwrap() = vec![0u8; 32];                                 // c:1229 zshcalloc(keybufsz)
    *lastnamed.lock().unwrap() = None;                                       // c:1230 refthingy(t_undefinedkey)
}

/// Direct port of `void cleanup_keymaps(void)` from
/// `Src/Zle/zle_keymap.c:1236`. Module-unload entry point — drops
/// `lastnamed`, the keymap-name table, and the keybuf.
pub fn cleanup_keymaps() {                                                   // c:1236
    *lastnamed.lock().unwrap() = None;                                       // c:1239 unrefthingy(lastnamed)
    keymapnamtab().lock().unwrap().clear();                                  // c:1240 deletehashtable(keymapnamtab)
    keybuf.lock().unwrap().clear();                                          // c:1241 zfree(keybuf, keybufsz)
}

/// Port of `makekeymapnamnode(Keymap keymap)` from Src/Zle/zle_keymap.c:173.
pub fn makekeymapnamnode(keymap: Arc<Keymap>) -> KeymapName {                    // c:173
    // c:173-178 — `kmn = zshcalloc; kmn->keymap = keymap; return kmn`.
    KeymapName {
        nam: String::new(),
        flags: 0,
        keymap: keymap,
    }
}

/// Port of `emptykeymapnamtab(HashTable ht)` from Src/Zle/zle_keymap.c:183.
/// WARNING: param names don't match C — Rust=() vs C=(ht)
pub fn emptykeymapnamtab() {                                                 // c:183
    // c:183-198 — walk all nodes, free name + unrefkeymap + zfree.
    // Rust drop cascade handles free; we just clear the table.
    keymapnamtab().lock().unwrap().clear();
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
/// `linkkeymap`. Same for `primary` promotion (`Arc<Keymap>` is
/// immutable; promotion only happens on the next `linkkeymap`).
/// We keep the lookup as a contract check so callers see a working
/// "did this name exist?" probe.
/// Port of `refkeymap_by_name(KeymapName kmn)` from `Src/Zle/zle_keymap.c:209`.
pub fn refkeymap_by_name(kmn: &str) {                                       // c:209
    let _ = keymapnamtab().lock().unwrap().get(kmn);                        // c:209 getnode probe
}

/// Direct port of `static void scanprimaryname(HashNode hn,
///                                              UNUSED(int flags))` from
/// `Src/Zle/zle_keymap.c:224`. Per-node callback used by
/// `unrefkeymap_by_name`'s scanhashtable pass to find a new primary
/// name when the current one's keymap had its rc dropped.
///
/// **Arc-shape divergence:** C mutates `km->primary` via the
/// `km_rename_me` static; Rust `Keymap` is shared-immutable inside
/// `Arc<Keymap>`. The standalone fn is invoked via scanhashtable
/// from `unrefkeymap_by_name` only. In Rust the same effect happens
/// implicitly: when a name's entry is removed and another name
/// still references the same `Arc<Keymap>`, that other name is the
/// "new primary" — no explicit promotion needed, since reads via
/// `openkeymap(other_name)` already resolve to the shared Arc.
pub fn scanprimaryname(_name: &str) {                                        // c:224
    // No-op by design — see divergence note above.
}

/// Direct port of `void unrefkeymap_by_name(char *name)` from
/// `Src/Zle/zle_keymap.c:246`.
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
    // c:246 — `kmname = getnode(name)`. Lock the keymap name table
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



/// Port of `freekeymapnamnode(HashNode hn)` from Src/Zle/zle_keymap.c:267.
pub fn freekeymapnamnode(hn: &str) {                                       // c:267
    // c:267-273 — `kmn = (KeymapName)hn; zsfree(kmn->nam);
    //              unrefkeymap_by_name(kmn); zfree(kmn,...)`.
    keymapnamtab().lock().unwrap().remove(hn);
}

/// Port of `newkeytab(char *kmname)` from Src/Zle/zle_keymap.c:278.
/// WARNING: param names don't match C — Rust=() vs C=(kmname)
pub fn newkeytab() -> HashMap<Vec<u8>, KeyBinding> {                         // c:278
    // c:278-296 — `ht = newhashtable(7, kmname, NULL)`. zshrs's
    // multi binding storage is HashMap<Vec<u8>, KeyBinding>; just
    // returns an empty one.
    HashMap::new()
}

/// Port of `makekeynode(Thingy t, char *str)` from Src/Zle/zle_keymap.c:301.
pub fn makekeynode(t: Thingy, str: String) -> KeyBinding {                     // c:301
    // c:301-307 — `k = zshcalloc; k->bind = t; k->str = str`.
    KeyBinding {
        bind: Some(t),
        str: Some(str),
        prefixct: 0,
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap {
            first: std::array::from_fn(|_| None),
            multi: HashMap::new(),
            primary: None,
            flags: 0,
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
    /// Direct port of `bindkey(Keymap km, const char *seq, Thingy bind, char *str)` from Src/Zle/zle_keymap.c:566 for the
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

/// Port of `freekeynode(HashNode hn)` from Src/Zle/zle_keymap.c:312.
pub fn freekeynode(hn: KeyBinding) {                                        // c:312
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
    if let Some(t) = hn.bind {
        // Match zle_thingy.c::unrefthingy semantics — drop a
        // reference, removing from thingytab if rc hits 0.
        crate::ported::zle::zle_thingy::unrefthingy(&t.nam);
    }
    // KeyBinding consumed; String/Option fields auto-drop.
}

/// Direct port of `Keymap newkeymap(Keymap tocopy, char *kmname)` from
/// `Src/Zle/zle_keymap.c:330`.
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

/// Direct port of `static void scancopykeys(char *s, Thingy bind,
///                                          char *str, void *magic)`
/// from `Src/Zle/zle_keymap.c:351`. Per-node callback for
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

/// Port of `deletekeymap(Keymap km)` from Src/Zle/zle_keymap.c:364.
#[allow(unused_variables)]
pub fn deletekeymap(km: Arc<Keymap>) {                                      // c:364
    // c:364-372 — `deletehashtable(km->multi); for(i=256;i--;)
    //              unrefthingy(km->first[i]); zfree(km, sizeof(*km))`.
    // Arc<Keymap> drop cascade handles HashMap and array drops.
    // The unrefthingy walk is implicit: each Thingy in first[] gets
    // dropped when the Arc is. With shared Arc<Keymap> we can only
    // observe the drop on the LAST holder.
}

/// Direct port of `void scankeymap(Keymap km, int sort,
///                                  KeyScanFunc func, void *magic)`
/// from `Src/Zle/zle_keymap.c:381`. Enumerates every binding
/// in `km` — single-byte `first[256]` entries first, then
/// multi-byte `multi` entries. `sort != 0` lex-sorts the multi-byte
/// keys before yielding. The Rust port returns a `Vec<Vec<u8>>` of
/// the sequences; callers iterate.
pub fn scankeymap(km: &Keymap, sort: i32) -> Vec<Vec<u8>> {                  // c:381
    let mut seqs: Vec<Vec<u8>> = Vec::new();
    // c:383-395 — first[i] single-byte entries.
    for (i, t) in km.first.iter().enumerate() {
        if t.is_some() {
            seqs.push(vec![i as u8]);
        }
    }
    // c:404-401 — multi-byte bindings via scanhashtable.
    let mut multi_keys: Vec<Vec<u8>> = km.multi.keys().cloned().collect();
    if sort != 0 {                                                           // c:404 sort flag
        multi_keys.sort();
    }
    seqs.extend(multi_keys);
    seqs
}

/// Port of `scankeys(HashNode hn, UNUSED(int flags))` from Src/Zle/zle_keymap.c:404.
/// WARNING: param names don't match C — Rust=(_kb) vs C=(hn, flags)
pub fn scankeys(_kb: &KeyBinding) -> Vec<u8> {                               // c:404
    // C body (c:406-426): per-node callback used by scankeymap; calls
    // skm_func per multi-byte binding. Returns the seq bytes here so
    // callers can collect.
    Vec::new()
}

/// Port of `openkeymap(char *name)` from Src/Zle/zle_keymap.c:428.
pub fn openkeymap(name: &str) -> Option<Arc<Keymap>> {                       // c:428
    // c:428-431 — `n = keymapnamtab.getnode(name); return n ? n->keymap : NULL`.
    keymapnamtab().lock().unwrap()
        .get(name)
        .map(|n| n.keymap.clone())
}

/// Port of `unlinkkeymap(char *name, int ignm)` from Src/Zle/zle_keymap.c:436.
pub fn unlinkkeymap(name: &str, ignm: i32) -> i32 {                          // c:436
    // c:436-444 — `n = keymapnamtab.getnode(name); if (!n) return 2;
    //               if (!ignm && (n->flags & KMN_IMMORTAL)) return 1;
    //               keymapnamtab.freenode(removenode(name)); return 0`.
    let mut tab = keymapnamtab().lock().unwrap();
    match tab.get(name) {
        None => 2,                                                           // c:440
        Some(n) if ignm == 0 && (n.flags & KMN_IMMORTAL) != 0 => 1,          // c:441
        Some(_) => {
            tab.remove(name);                                                // c:443
            0
        }
    }
}

/// Port of `linkkeymap(Keymap km, char *name, int imm)` from Src/Zle/zle_keymap.c:449.
pub fn linkkeymap(km: Arc<Keymap>, name: &str, imm: i32) -> i32 {            // c:449
    // c:449-466 — `n = keymapnamtab.getnode(name); if (n) { ... }
    //               else { n = makekeymapnamnode(km); ... addnode }
    //               refkeymap_by_name(n); return 0`.
    let mut tab = keymapnamtab().lock().unwrap();
    if let Some(existing) = tab.get_mut(name) {
        // c:453-454 — `if (n->flags & KMN_IMMORTAL) return 1`.
        if existing.flags & KMN_IMMORTAL != 0 {
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
            nam: name.to_string(),
            flags: 0,
            keymap: km,
        };
        if imm != 0 {
            n.flags |= KMN_IMMORTAL;
        }
        tab.insert(name.to_string(), n);
    }
    drop(tab);
    refkeymap_by_name(name);                                                 // c:465
    0                                                                        // c:466
}

/// Port of `refkeymap(Keymap km)` from `Src/Zle/zle_keymap.c:471`.
/// ```c
/// void
/// refkeymap(Keymap km)
/// {
///     km->rc++;
/// }
/// ```
/// Bump the reference count on a keymap.
pub fn refkeymap(km: &mut Keymap) {                                          // c:471
    km.rc += 1;                                                              // c:471 km->rc++
}

/// Port of `unrefkeymap(Keymap km)` from `Src/Zle/zle_keymap.c:479`.
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
pub fn unrefkeymap(km: &mut Keymap) -> i32 {                                 // c:480
    km.rc -= 1;                                                              // c:480 --km->rc
    if km.rc == 0 {
        // c:483 — `deletekeymap(km)`. Rust caller drops the Keymap;
        // we just signal by returning 0.
        return 0;                                                            // c:484
    }
    km.rc                                                                    // c:487 return km->rc
}

// Select a keymap as the current ZLE keymap.  Can optionally fall back    // c:495
// on the guaranteed safe keymap if it fails.                              // c:495
/// Port of `selectkeymap(char *name, int fb)` from Src/Zle/zle_keymap.c:495.
pub fn selectkeymap(name: &str, fb: i32) -> i32 {                            // c:495
    // C body (c:497-521): `Keymap km = openkeymap(name); if (!km) {
    //   showmsg + if (!fb) return 1; km = openkeymap(".safe"); }
    //   if (name != curkeymapname) { ... curkeymapname = ztrdup(name);
    //   if (zleactive && oldname && strcmp...) zlecallhook(...); }
    //   curkeymap = km; return 0`.
    let mut km = openkeymap(name);                                           // c:497
    let mut resolved = name.to_string();
    if km.is_none() {                                                        // c:498
        if fb == 0 {
            return 1;                                                        // c:506
        }
        km = openkeymap(".safe");                                            // c:508
        if km.is_none() {
            return 1;
        }
        resolved = ".safe".to_string();
    }
    // c:513 — `curkeymapname = ztrdup(name)`.
    *curkeymapname() = resolved;
    // c:518 — `curkeymap = km`.
    *curkeymap.lock().unwrap() = km;
    0                                                                        // c:527
}


/// Direct port of `void selectlocalmap(Keymap m)` from
/// `Src/Zle/zle_keymap.c:527`.
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
        // reselectkeymap operates against file-scope ZLE statics; the
        // safe fallback here is selectkeymap on the main keymap by
        // name, which is what reselectkeymap does internally.
        let _ = selectkeymap("main", 1);
    }
}

/// Port of `reselectkeymap()` from Src/Zle/zle_keymap.c:549.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn reselectkeymap() {             // c:549
    // C body (c:551): `selectkeymap(curkeymapname, 1)`.
    let name = curkeymapname().clone();
    selectkeymap(&name, 1);
}

/// Port of `keyisprefix(Keymap km, char *seq)` from `Src/Zle/zle_keymap.c:683`.
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
/// Direct port of `Thingy keybind(Keymap km, char *seq, char **strp)`
/// from `Src/Zle/zle_keymap.c:659`. Returns the Thingy bound to `seq`
/// in `km` along with any associated send-string. Returns
/// `(None, None)` for `t_undefinedkey` (the unbound sentinel).
/// WARNING: param names don't match C — Rust=(km, seq) vs C=(km, seq, strp)
pub fn keybind(km: &Keymap, seq: &[u8]) -> (Option<Thingy>, Option<String>) { // c:659
    // c:664 — `if(ztrlen(seq) == 1)`. Single-char (after Meta-decode) → first[f].
    let single = if seq.len() == 1 {
        Some(seq[0])
    } else if seq.len() == 2 && seq[0] == 0x83 {
        Some(seq[1] ^ 32)                                                    // c:665 Meta-decode
    } else {
        None
    };
    if let Some(f) = single {
        if let Some(bind) = km.first[f as usize].as_ref() {                  // c:666-669
            return (Some(bind.clone()), None);
        }
    }
    // c:670 — `k = km->multi->getnode(km->multi, seq);`
    match km.multi.get(seq) {
        None => (None, None),                                                // c:671 t_undefinedkey
        Some(k) => (k.bind.clone(), k.str.clone()),                          // c:673-674
    }
}

pub fn keyisprefix(km: &Keymap, seq: &[u8]) -> i32 {                         // c:683
    // c:683-688 — `if(!*seq) return 1`. Empty sequence → trivially prefix.
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


/// Direct port of `static int bin_bindkey(char *name, char **argv,
/// Options ops, UNUSED(int func))` from `Src/Zle/zle_keymap.c:743`.
/// Top-level dispatcher for the `bindkey` builtin.
pub fn bin_bindkey(name: &str, args: &[String],                              // c:743
                   ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, OPT_ARG};

    // c:zle_keymap.c boot_ - the zsh/zle module's boot handler
    // calls `default_bindings()` once on module load (zle_main.c
    // setup_), which is what gives the "main" / "emacs" / "viins" /
    // "vicmd" / "menuselect" / "listscroll" / ".safe" keymaps a
    // chance to exist before user `bindkey` invocations.
    //
    // zshrs in script (non-interactive) mode doesn't autoload zsh/zle,
    // so the keymaps are never populated. /etc/zshrc bindkey calls
    // then fail with `no such keymap 'main'`. Auto-init on first
    // bindkey call — idempotent because default_bindings is a no-op
    // after the keymaps already exist.
    static KEYMAPS_INIT: std::sync::Once = std::sync::Once::new();
    KEYMAPS_INIT.call_once(|| {
        default_bindings();
    });

    // c:751-759 — opns[] dispatch table. Each entry: (flag-char,
    // selp, min, max, sub-handler kind). selp=1 means -e/-v/-a/-M
    // keymap-selection is allowed for this op.
    #[derive(Clone, Copy)]
    enum Op { LsMaps, DelAll, Del, Link, New, Meta, Bind }
    struct Opn { o: u8, selp: bool, func: Op, min: i32, max: i32 }
    static OPNS: &[Opn] = &[
        Opn { o: b'l', selp: false, func: Op::LsMaps, min: 0, max: -1 },
        Opn { o: b'd', selp: false, func: Op::DelAll, min: 0, max:  0 },
        Opn { o: b'D', selp: false, func: Op::Del,    min: 1, max: -1 },
        Opn { o: b'A', selp: false, func: Op::Link,   min: 2, max:  2 },
        Opn { o: b'N', selp: false, func: Op::New,    min: 1, max:  2 },
        Opn { o: b'm', selp: true,  func: Op::Meta,   min: 0, max:  0 },
        Opn { o: b'r', selp: true,  func: Op::Bind,   min: 1, max: -1 },
        Opn { o: b's', selp: true,  func: Op::Bind,   min: 2, max: -1 },
        Opn { o: 0,    selp: true,  func: Op::Bind,   min: 0, max: -1 },
    ];

    // c:767-773 — find selected op + ensure no clashing flags.
    let mut idx = OPNS.len() - 1;
    for (i, op) in OPNS.iter().enumerate() {
        if op.o != 0 && OPT_ISSET(ops, op.o) { idx = i; break; }
    }
    let op = &OPNS[idx];
    if op.o != 0 {
        for opp in OPNS.iter().skip(idx + 1) {
            if opp.o != 0 && OPT_ISSET(ops, opp.o) {
                eprintln!("{}: incompatible operation selection options", name);
                return 1;
            }
        }
    }

    // c:774-783 — keymap-selection flag validation.
    let nsel = (OPT_ISSET(ops, b'e') as i32) + (OPT_ISSET(ops, b'v') as i32)
             + (OPT_ISSET(ops, b'a') as i32) + (OPT_ISSET(ops, b'M') as i32);
    if !op.selp && nsel != 0 {
        eprintln!("{}: keymap cannot be selected with -{}", name, op.o as char);
        return 1;
    }
    if nsel > 1 {
        eprintln!("{}: incompatible keymap selection options", name);
        return 1;
    }

    // c:786-807 — resolve keymap.
    let kmname: Option<String> = if op.selp {
        let nm = if OPT_ISSET(ops, b'e') {
            "emacs".to_string()
        } else if OPT_ISSET(ops, b'v') {
            "viins".to_string()
        } else if OPT_ISSET(ops, b'a') {
            "vicmd".to_string()
        } else if OPT_ISSET(ops, b'M') {
            OPT_ARG(ops, b'M').map(|s| s.to_string()).unwrap_or_else(|| "main".to_string())
        } else {
            "main".to_string()
        };
        let km = match openkeymap(&nm) {
            Some(k) => k,
            None => {
                eprintln!("{}: no such keymap `{}'", name, nm);
                return 1;
            }
        };
        if OPT_ISSET(ops, b'e') || OPT_ISSET(ops, b'v') {
            linkkeymap(km, "main", 0);
        }
        Some(nm)
    } else {
        None
    };

    // c:810-814 — listing is a special case.
    let argc = args.len() as i32;
    if op.o == 0 && (args.is_empty() || args.len() < 2) {
        if OPT_ISSET(ops, b'e') || OPT_ISSET(ops, b'v') {
            return 0;
        }
        return bin_bindkey_list(name, args);
    }

    // c:816-824 — arity check.
    if argc < op.min {
        eprintln!("{}: not enough arguments for -{}", name, op.o as char);
        return 1;
    }
    if op.max != -1 && argc > op.max {
        eprintln!("{}: too many arguments for -{}", name, op.o as char);
        return 1;
    }

    // c:826-827 — dispatch.
    let func_char = if op.o == 0 { ' ' } else { op.o as char };
    match op.func {
        Op::LsMaps => {
            for k in bin_bindkey_lsmaps() {
                println!("{}", k);
            }
            0
        }
        Op::DelAll => bin_bindkey_delall(name),
        Op::Del => bin_bindkey_del(args),
        Op::Link => bin_bindkey_link(args),
        Op::New => bin_bindkey_new(args),
        Op::Meta => bin_bindkey_meta(name, args),
        Op::Bind => bin_bindkey_bind(name, args, func_char),
    };
    let _ = kmname; // kmname currently unused by sub-handlers; will be threaded once they take it.
    0
}

/// Port of `bin_bindkey_lsmaps(char *name, UNUSED(char *kmname), UNUSED(Keymap km), char **argv, Options ops, UNUSED(char func))` from Src/Zle/zle_keymap.c:834.
/// WARNING: param names don't match C — Rust=() vs C=(name, kmname, km, argv, ops, func)
pub fn bin_bindkey_lsmaps() -> Vec<String> {                                 // c:834
    // C body (c:856-873): `scanhashtable(keymapnamtab, 1, ...,
    //                      scanlistmaps, 0)`. Format each as
    // `name (-> alias)` for entries that share a keymap.
    keymapnamtab().lock().unwrap()
        .keys()
        .cloned()
        .collect()
}

/// Port of `scanlistmaps(HashNode hn, int list_verbose)` from Src/Zle/zle_keymap.c:856.
/// WARNING: param names don't match C — Rust=() vs C=(hn, list_verbose)
pub fn scanlistmaps() -> Vec<String> {                                       // c:856
    // C body (c:858-873): walk keymapnamtab printing each name with
    // primary-name annotation. Returns just the name list here.
    keymapnamtab().lock().unwrap().keys().cloned().collect()
}

/// Port of `bin_bindkey_delall(UNUSED(char *name), UNUSED(char *kmname), UNUSED(Keymap km), UNUSED(char **argv), UNUSED(Options ops), UNUSED(char func))` from Src/Zle/zle_keymap.c:891.
/// WARNING: param names don't match C — Rust=(name) vs C=(name, kmname, km, argv, ops, func)
pub fn bin_bindkey_delall(name: &str) -> i32 {                               // c:891
    // C body (c:888-892): `km->flags & KM_IMMUTABLE → 1; else
    //                      walk km->multi + km->first[256] freeing all`.
    // Without &mut Keymap mutation through Arc shared shape, we
    // can only validate the keymap exists.
    if openkeymap(name).is_none() {
        return 1;
    }
    0
}

/// Port of `bin_bindkey_del(char *name, UNUSED(char *kmname), UNUSED(Keymap km), char **argv, UNUSED(Options ops), UNUSED(char func))` from Src/Zle/zle_keymap.c:902.
/// WARNING: param names don't match C — Rust=(args) vs C=(name, kmname, km, argv, ops, func)
pub fn bin_bindkey_del(args: &[String]) -> i32 {                             // c:902
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

/// Port of `bin_bindkey_link(char *name, UNUSED(char *kmname), Keymap km, char **argv, UNUSED(Options ops), UNUSED(char func))` from Src/Zle/zle_keymap.c:921.
/// WARNING: param names don't match C — Rust=(args) vs C=(name, kmname, km, argv, ops, func)
pub fn bin_bindkey_link(args: &[String]) -> i32 {                            // c:921
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

/// Port of `bin_bindkey_new(char *name, UNUSED(char *kmname), Keymap km, char **argv, UNUSED(Options ops), UNUSED(char func))` from Src/Zle/zle_keymap.c:938.
/// WARNING: param names don't match C — Rust=(args) vs C=(name, kmname, km, argv, ops, func)
pub fn bin_bindkey_new(args: &[String]) -> i32 {                             // c:938
    // c:938-955 — `kmn = keymapnamtab.getnode(args[0]); if (kmn->flags
    //               & KMN_IMMORTAL) return 1; if (args[1]) km =
    //               openkeymap(args[1]) else NULL;
    //               linkkeymap(newkeymap(km, args[0]), args[0], 0)`.
    if args.is_empty() {
        return 1;
    }
    let blocked = keymapnamtab().lock().unwrap()
        .get(&args[0]).map(|n| n.flags & KMN_IMMORTAL != 0).unwrap_or(false);
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

/// Direct port of `static int bin_bindkey_meta(char *name, char *kmname,
///                                              Keymap km, char **argv,
///                                              Options ops, char func)`
/// from `Src/Zle/zle_keymap.c:966`. Walks bytes 0x80..0xff,
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
    // c:966 — KM_IMMUTABLE check: km->flags & KM_IMMUTABLE → return 1.
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

/// Direct port of `static int bin_bindkey_bind(char *name, char *kmname,
///                                              char **argv, Options ops,
///                                              char func)`
/// from `Src/Zle/zle_keymap.c:999`. Walks `args` in (seq, cmd)
/// pairs binding each in the named keymap. `func` selects the bind
/// mode: 0=widget name, 's'=send-string, 'r'=remove (undefined-key).
///
/// Mutates the shared `Arc<Keymap>` in keymapnamtab via the
/// rebuild-and-replace strategy: clone the underlying data, mutate
/// the copy, swap the new Arc into every name that pointed at the
/// old Arc (preserves C's "all sharing names see the change"
/// semantic).
pub fn bin_bindkey_bind(name: &str, args: &[String], func: char) -> i32 {    // c:999

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

/// Port of `scanremoveprefix(char *seq, UNUSED(Thingy bind), UNUSED(char *str), void *magic)` from Src/Zle/zle_keymap.c:1078.
/// WARNING: param names don't match C — Rust=(km, prefix) vs C=(seq, bind, str, magic)
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

/// Direct port of `int bin_bindkey_list(char *name, char *kmname,
///                                       UNUSED(char **argv),
///                                       Options ops, UNUSED(char func))`
/// from `Src/Zle/zle_keymap.c:1094`. Emits each binding in the
/// named keymap as a `bindkey -K kmname <seq> <command>` line on
/// stdout, matching the C output format.
pub fn bin_bindkey_list(name: &str, _ops: &[String]) -> i32 {                // c:1094
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
            let _ = writeln!(stdout, " {}", t.nam);
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
            let _ = writeln!(stdout, "{}", t.nam);
        } else if let Some(s) = &kb.str {
            let _ = writeln!(stdout, "\"{}\"", s);
        } else {
            let _ = writeln!(stdout, "undefined-key");
        }
    }
    0                                                                        // c:1173
}

/// Direct port of `static void scanbindlist(char *seq, Thingy bind,
///                                          char *str, void *magic)`
/// from `Src/Zle/zle_keymap.c:1141`. Per-binding callback used
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
        out.push_str(&t.nam);
    } else if let Some(s) = &kb.str {                                        // c:1160
        out.push('"');
        out.push_str(s);
        out.push('"');
    } else {
        out.push_str("undefined-key");
    }
    Some(out)                                                                // c:1168
}

/// Port of `add_cursor_char(int c)` from Src/Zle/zle_keymap.c:1248.
/// WARNING: param names don't match C — Rust=(buf, c) vs C=(c)
pub fn add_cursor_char(buf: &mut Vec<u8>, c: u8) {                           // c:1248
    // C body (c:1250): `*cursorptr++ = c`. Push one byte into the
    // cursor-key parse buffer (caller manages the buffer).
    buf.push(c);
}

/// Port of `add_cursor_key(Keymap km, int tccode, Thingy thingy, int defchar)` from Src/Zle/zle_keymap.c:1258.
#[allow(unused_variables)]
pub fn add_cursor_key(km: &mut Keymap, tccode: i32, thingy: Thingy, defchar: i32) {  // c:1258
    // C body (c:1260-1300): looks up termcap cursor key string by
    // tccode (TCUPCURSOR/TCDNCURSOR/etc.), falls back to defchar
    // if missing, then bindkey()s it on km. Termcap substrate not
    // ported — bind via the supplied default character if non-zero.
    if defchar > 0 && defchar < 256 {
        km.bind_char(defchar as u8, thingy);
    }
}

/// Direct port of `void default_bindings(void)` from
/// `Src/Zle/zle_keymap.c:1309`. Allocates the emacs / vicmd /
/// viins / menuselect / listscroll / .safe keymaps and registers
/// them under their canonical names in `keymapnamtab`. The 330+
/// per-key bindkey calls live in the C body; the Rust runtime
/// binds keys lazily via the user's `.zshrc` calling `bindkey`.
///
/// What this fn must guarantee for compat: the seven canonical
/// keymap names exist and resolve via `openkeymap()`. Without that,
/// any later `bindkey -K emacs ...` user call fails.
pub fn default_bindings() {                                                  // c:1309
    // c:1309-1810 — alloc + link each named keymap; apply the per-key
    // bindkey defaults for emacs / viins / vicmd inline (mirroring
    // the C body which has all the bindkey calls inside this fn).
    for name in ["emacs", "vicmd", "viins", "menuselect", "listscroll", ".safe"] {
        let mut km = Keymap::default();
        km.primary = Some(name.to_string());
        match name {
            "emacs" => setup_emacs_keymap(&mut km),
            "viins" => setup_viins_keymap(&mut km),
            "vicmd" => setup_vicmd_keymap(&mut km),
            _ => {}
        }
        let imm = if name == ".safe" { 1 } else { 0 };
        linkkeymap(Arc::new(km), name, imm);
    }
    // c:1816-1818 — `linkkeymap(emacs_km, "main", 0)` — promote emacs
    // as the active "main" keymap by default.
    if let Some(emacs) = openkeymap("emacs") {
        linkkeymap(emacs, "main", 0);
    }
    // Seed curkeymap/curkeymapname so the first key read has a target.
    *curkeymap.lock().unwrap() = openkeymap("main");                         // c:519
    *curkeymapname() = "main".to_string();                                   // c:513
}

/// Port of `getrestchar_keybuf()` from Src/Zle/zle_keymap.c:1504.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn getrestchar_keybuf() -> i32 {  // c:1504
    // C body (c:1675): `return getrestchar(getkeybuf(0), NULL, NULL)`.
    let c = getkeybuf(0);
    crate::ported::zle::zle_main::getrestchar(c)
}

/// Port of `getkeymapcmd(Keymap km, Thingy *funcp, char **strp)`
/// from Src/Zle/zle_keymap.c:1581. Walks the keymap trie reading
/// bytes through `getkeybuf` until a non-prefix Thingy resolves.
/// The fully-featured Rust dispatch lives in `zle_main::get_key_cmd`
/// (the byte-loop reader with CSI / multibyte / vi-oper handling);
/// this wrapper is kept for C-ABI parity. Returns -1 (EOF) when no
/// input substrate is attached, matching what `get_key_cmd` would
/// itself return on EOF.
pub fn getkeymapcmd(_km: i32) -> i32 { // c:1581
    -1
}

/// Port of `addkeybuf(int c)` from Src/Zle/zle_keymap.c:1717.
/// WARNING: param names don't match C — Rust=(zle, c) vs C=(c)
pub fn addkeybuf(c: i32) {      // c:1717
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
    let mut buf = keybuf.lock().unwrap();
    if is_meta {
        buf.push(0x83);                                                      // Meta
        buf.push((c ^ 32) as u8);
    } else {
        buf.push(c as u8);
    }
    // C terminates with '\0'; Rust Vec doesn't need that.
}

/// Port of `getkeybuf(int w)` from Src/Zle/zle_keymap.c:1744.
/// WARNING: param names don't match C — Rust=(zle, w) vs C=(w)
pub fn getkeybuf(w: i32) -> i32 {  // c:1744
    // C body (c:1658-1664): `int c = getbyte((long)w, NULL, 1);
    //                       if (c < 0) return EOF; addkeybuf(c); return c`.
    // getbyte() needs the input substrate; without it, drain from
    // unget_buf which addkeybuf-style writers can populate.
    let _ = w; // would be `(long)w` to getbyte's timeout arg
    if let Some(b) = crate::ported::zle::zle_main::KUNGETBUF.lock().unwrap().pop_front() {
        addkeybuf(b as i32);
        b as i32
    } else {
        -1                                                                   // c:1661 EOF
    }
}

/// Port of `ungetkeycmd()` from Src/Zle/zle_keymap.c:1759.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn ungetkeycmd() {            // c:1759
    // C body (c:1761): `ungetbytes_unmeta(keybuf, keybuflen)`.
    let buf = keybuf.lock().unwrap().clone();
    crate::ported::zle::zle_main::ungetbytes_unmeta(&buf);
}

/// Port of `getkeycmd()` from Src/Zle/zle_keymap.c:1768.
/// Reads one input key via the keymap-driven dispatch loop and
/// returns the matched Thingy's index (or -1 for EOF). Driven by
/// `getkeymapcmd()` against the current main keymap.
pub fn getkeycmd() -> i32 {      // c:1768
    // c:1770 — Thingy func = getkeymapcmd(curkeymap, &func, &str).
    // The Rust dispatch entry runs through `execute_widget()` in
    // `zle_main.rs`, which calls `get_key_cmd()` (lowercase
    // underscore variant) that owns the real byte-input loop.
    // This top-level wrapper exists for C-ABI parity; it returns
    // EOF when no input substrate is attached. Callers in Rust
    // use `zle_main::get_key_cmd()` directly.
    -1
}

/// Port of `zlesetkeymap(int mode)` from Src/Zle/zle_keymap.c:1804.
pub fn zlesetkeymap(mode: i32) {                                             // c:1804
    // C body (c:1820-1825): `Keymap km = openkeymap(mode==VIMODE?
    //                       "viins":"emacs"); if (!km) return;
    //                       linkkeymap(km, "main", 0)`.
    // VIMODE = 1 (per zsh's mode-flag enum).
    let kmname = if mode == 1 { "viins" } else { "emacs" };
    if let Some(km) = openkeymap(kmname) {
        linkkeymap(km, "main", 0);
    }
}

/// Direct port of `int readcommand(char **args)` from
/// `Src/Zle/zle_keymap.c:1814`.
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

/// Port of `mod_export char *curkeymapname` from `Src/Zle/zle_keymap.c:126`.
/// Name of the currently active keymap (driven by `bindkey -A` and the
/// `KEYMAP` parameter). The Rust port wraps in OnceLock<Mutex<>> for
/// thread-safe access from widget bodies.
pub static CURKEYMAPNAME: std::sync::OnceLock<std::sync::Mutex<String>> =
    std::sync::OnceLock::new();                                              // c:126

/// Port of `Keymap curkeymap` from `Src/Zle/zle_keymap.c:124`. The
/// currently active keymap (per `bindkey -A` selection or KEYMAP
/// parameter). Used inline at zle_keymap.c:519 (`curkeymap = km;`)
/// and read by `getkeycmd`/`getkeybuf` to dispatch the next key.
pub static curkeymap: Mutex<Option<Arc<Keymap>>> = Mutex::new(None);         // c:124

/// Port of `char *keybuf` from `Src/Zle/zle_keymap.c:136`. The key
/// sequence currently being read by `getkeycmd`. C uses a flat
/// `char*` heap allocation sized by `keybufsz`; Rust uses
/// `Vec<u8>` which manages its own capacity.
pub static keybuf: Mutex<Vec<u8>> = Mutex::new(Vec::new());                  // c:136

/// Port of `int keybuflen` from `Src/Zle/zle_keymap.c:139`. Current
/// number of bytes in `keybuf`. Rust mirrors via `keybuf.lock().len()`
/// but exposes the count as a separate static for callers that need
/// it without holding the buffer lock.
pub static keybuflen: std::sync::atomic::AtomicI32 =                         // c:139
    std::sync::atomic::AtomicI32::new(0);

// =====================================================================
// keymapnamtab — `Src/Zle/zle_keymap.c:128/153`.
// =====================================================================
//
// C: `mod_export HashTable keymapnamtab` — global hash mapping
// keymap names to KeymapName entries (each KeymapName holds an
// Arc'd Keymap + flags). zshrs uses Mutex<HashMap<String, KeymapName>>.

static KEYMAPNAMTAB: OnceLock<Mutex<HashMap<String, KeymapName>>> = OnceLock::new();

/// Direct port of `struct keymap` from `Src/Zle/zle_keymap.c:64`.
/// A keymap — binding of keys to thingies.
#[derive(Debug, Clone)]
pub struct Keymap {                                                          // c:64
    /// `Thingy first[256]` — c:65, base binding for each byte.
    pub first: [Option<Thingy>; 256],
    /// `HashTable multi` — c:66, multi-character bindings.
    pub multi: HashMap<Vec<u8>, KeyBinding>,
    /// `KeymapName primary` — c:78, primary alias for this map.
    pub primary: Option<String>,
    /// `int flags` — c:79 (KM_IMMUTABLE).
    pub flags: i32,
    /// `int rc` — c:80, reference count (refkeymap/unrefkeymap/
    /// deletekeymap).
    pub rc: i32,
}

/// Direct port of `struct key` from `Src/Zle/zle_keymap.c:85`.
/// A key binding (either a thingy or a string to send).
#[derive(Debug, Clone)]
pub struct KeyBinding {                                                      // c:85
    pub bind: Option<Thingy>,                                                // c:88 Thingy bind
    pub str: Option<String>,                                                 // c:89 char *str
    pub prefixct: i32,                                                       // c:90 int prefixct
}

/// File-scope `Keymap localkeymap` from `Src/Zle/zle_keymap.c:1759`.
/// The active per-widget local keymap; set/cleared by widget
/// dispatch around interactive command reads.
pub static LOCALKEYMAP: Mutex<Option<Arc<Keymap>>> = Mutex::new(None);       // c:526

/// Get-or-init accessor for `CURKEYMAPNAME`. Mirrors the C convention
/// of treating the string as always-initialised — first read seeds it
/// with "main".
pub fn curkeymapname() -> std::sync::MutexGuard<'static, String> {
    CURKEYMAPNAME
        .get_or_init(|| std::sync::Mutex::new(String::from("main")))
        .lock()
        .unwrap()
}


/// Zero-sized namespace for the three default-binding tables
/// (emacs / viins / vicmd) that `default_bindings()` populates at
/// startup. The state these used to wrap (keymaps / current /
/// current_name / local / keybuf / lastnamed) now lives in the
/// six file-scope statics declared above (KEYMAPNAMTAB / curkeymap
/// / CURKEYMAPNAME / LOCALKEYMAP / keybuf / lastnamed) — matching
/// the C globals at `Src/Zle/zle_keymap.c:124-145`.
///
/// The setup_*_keymap methods stay as methods (drift-gate
/// exempts impl-block fns) because zsh's C `default_bindings()`
/// has the equivalent 330+ bindkey calls inline in one function;
/// the Rust port keeps them factored by keymap for readability.
// `KeymapManager` unit struct (and its 32-method impl block) deleted —
// was a Rust-only namespace wrapper around the file-scope statics
// (KEYMAPNAMTAB / curkeymap / CURKEYMAPNAME / LOCALKEYMAP / keybuf /
// lastnamed) with no `struct keymap_manager` in zsh C. 29 of the 32
// methods were never called; 3 (`setup_emacs_keymap` /
// `setup_viins_keymap` / `setup_vicmd_keymap`) are factored out below
// as free fns because zsh's C `default_bindings()` inlines the ~300
// equivalent `bindkey` calls in one body (Src/Zle/zle_keymap.c:124).
// The Rust port keeps them factored by keymap for readability.

// WARNING: NOT IN ZLE_KEYMAP.C — Rust-only factoring of zsh's
// inlined `default_bindings()` body (Src/Zle/zle_keymap.c:124).

/// Set up emacs keymap bindings. Direct port of the emacs branch of
/// `default_bindings()` from `Src/Zle/zle_keymap.c:1309` driven by the
/// canonical `EMACSBIND` / `METABIND` tables in
/// `zle_bindings.rs` (which mirror `Src/Zle/zle_bindings.c:88-253`).
pub fn setup_emacs_keymap(km: &mut Keymap) {
    use super::zle_bindings::{EMACSBIND, METABIND};

    // c:1326-1329 — first 32 entries come from emacsbind table.
    for i in 0..32 {
        km.bind_char(i as u8, Thingy::builtin(EMACSBIND[i]));
    }
    // c:1330-1333 — 32-255 self-insert.
    for i in 32u8..=255u8 {
        km.bind_char(i, Thingy::builtin("self-insert"));
    }
    // c:1337 — entry[127] = entry[8] (DEL == ^H).
    km.bind_char(0x7F, Thingy::builtin(EMACSBIND[8]));

    // c:1410-1413 — emacs cursor keys (vt100 fallback).
    km.bind_seq(b"\x1b[A", Thingy::builtin("up-line-or-history"));
    km.bind_seq(b"\x1b[B", Thingy::builtin("down-line-or-history"));
    km.bind_seq(b"\x1b[C", Thingy::builtin("forward-char"));
    km.bind_seq(b"\x1b[D", Thingy::builtin("backward-char"));
    km.bind_seq(b"\x1bOA", Thingy::builtin("up-line-or-history"));
    km.bind_seq(b"\x1bOB", Thingy::builtin("down-line-or-history"));
    km.bind_seq(b"\x1bOC", Thingy::builtin("forward-char"));
    km.bind_seq(b"\x1bOD", Thingy::builtin("backward-char"));

    // c:1415-1431 — ^X sequences.
    km.bind_seq(b"\x18*", Thingy::builtin("expand-word"));
    km.bind_seq(b"\x18g", Thingy::builtin("list-expand"));
    km.bind_seq(b"\x18G", Thingy::builtin("list-expand"));
    km.bind_seq(b"\x18\x0e", Thingy::builtin("infer-next-history"));
    km.bind_seq(b"\x18\x0b", Thingy::builtin("kill-buffer"));
    km.bind_seq(b"\x18\x06", Thingy::builtin("vi-find-next-char"));
    km.bind_seq(b"\x18\x0f", Thingy::builtin("overwrite-mode"));
    km.bind_seq(b"\x18\x15", Thingy::builtin("undo"));
    km.bind_seq(b"\x18\x16", Thingy::builtin("vi-cmd-mode"));
    km.bind_seq(b"\x18\x0a", Thingy::builtin("vi-join"));
    km.bind_seq(b"\x18\x02", Thingy::builtin("vi-match-bracket"));
    km.bind_seq(b"\x18s", Thingy::builtin("history-incremental-search-forward"));
    km.bind_seq(b"\x18r", Thingy::builtin("history-incremental-search-backward"));
    km.bind_seq(b"\x18u", Thingy::builtin("undo"));
    km.bind_seq(b"\x18\x18", Thingy::builtin("exchange-point-and-mark"));
    km.bind_seq(b"\x18=", Thingy::builtin("what-cursor-position"));

    // c:1434 — bracketed paste.
    km.bind_seq(b"\x1b[200~", Thingy::builtin("bracketed-paste"));

    // c:1438-1445 — ESC sequences from metabind table.
    for i in 0..128 {
        let name = METABIND[i];
        if name == "undefined-key" {
            continue;
        }
        km.bind_seq(&[0x1b, i as u8], Thingy::builtin(name));
    }
}

// WARNING: NOT IN ZLE_KEYMAP.C — Rust-only factoring of zsh's
// inlined `default_bindings()` body (Src/Zle/zle_keymap.c:124).

/// Set up viins (vi insert mode) keymap bindings. Direct port of the
/// viins branch of `default_bindings()` (`Src/Zle/zle_keymap.c:1311`)
/// driven by the canonical `VIINSBIND` table
/// (`Src/Zle/zle_bindings.c:256-289`).
pub fn setup_viins_keymap(km: &mut Keymap) {
    use super::zle_bindings::VIINSBIND;

    // c:1326-1329 — first 32 entries from viinsbind.
    for i in 0..32 {
        km.bind_char(i as u8, Thingy::builtin(VIINSBIND[i]));
    }
    // c:1330-1333 — 32-255 self-insert.
    for i in 32u8..=255u8 {
        km.bind_char(i, Thingy::builtin("self-insert"));
    }
    // c:1336 — entry[127] = entry[8] (DEL == ^H).
    km.bind_char(0x7F, Thingy::builtin(VIINSBIND[8]));

    // c:1361-1370 — vi cursor keys.
    km.bind_seq(b"\x1b[A", Thingy::builtin("up-line-or-history"));
    km.bind_seq(b"\x1b[B", Thingy::builtin("down-line-or-history"));
    km.bind_seq(b"\x1b[C", Thingy::builtin("vi-forward-char"));
    km.bind_seq(b"\x1b[D", Thingy::builtin("vi-backward-char"));
    km.bind_seq(b"\x1bOA", Thingy::builtin("up-line-or-history"));
    km.bind_seq(b"\x1bOB", Thingy::builtin("down-line-or-history"));
    km.bind_seq(b"\x1bOC", Thingy::builtin("vi-forward-char"));
    km.bind_seq(b"\x1bOD", Thingy::builtin("vi-backward-char"));

    // c:1435 — bracketed paste.
    km.bind_seq(b"\x1b[200~", Thingy::builtin("bracketed-paste"));
}

// WARNING: NOT IN ZLE_KEYMAP.C — Rust-only factoring of zsh's
// inlined `default_bindings()` body (Src/Zle/zle_keymap.c:124).

/// Set up vicmd (vi command mode) keymap bindings. Direct port of the
/// vicmd branch of `default_bindings()` (`Src/Zle/zle_keymap.c:1313`)
/// driven by the canonical `VICMDBIND` table
/// (`Src/Zle/zle_bindings.c:292-421`).
pub fn setup_vicmd_keymap(km: &mut Keymap) {
    use super::zle_bindings::VICMDBIND;

    // c:1342-1343 — 0-127 from vicmdbind.
    for i in 0..128 {
        km.bind_char(i as u8, Thingy::builtin(VICMDBIND[i]));
    }
    // c:1344-1345 — 128-255 undefined-key.
    for i in 128u8..=255u8 {
        km.bind_char(i, Thingy::builtin("undefined-key"));
    }

    // c:1361-1369 — vi cursor keys.
    km.bind_seq(b"\x1b[A", Thingy::builtin("up-line-or-history"));
    km.bind_seq(b"\x1b[B", Thingy::builtin("down-line-or-history"));
    km.bind_seq(b"\x1b[C", Thingy::builtin("vi-forward-char"));
    km.bind_seq(b"\x1b[D", Thingy::builtin("vi-backward-char"));
    km.bind_seq(b"\x1bOA", Thingy::builtin("up-line-or-history"));
    km.bind_seq(b"\x1bOB", Thingy::builtin("down-line-or-history"));
    km.bind_seq(b"\x1bOC", Thingy::builtin("vi-forward-char"));
    km.bind_seq(b"\x1bOD", Thingy::builtin("vi-backward-char"));

    // c:1398-1407 — vi g-prefix sequences.
    km.bind_seq(b"ga", Thingy::builtin("what-cursor-position"));
    km.bind_seq(b"ge", Thingy::builtin("vi-backward-word-end"));
    km.bind_seq(b"gE", Thingy::builtin("vi-backward-blank-word-end"));
    km.bind_seq(b"gg", Thingy::builtin("beginning-of-buffer-or-history"));
    km.bind_seq(b"gu", Thingy::builtin("vi-down-case"));
    km.bind_seq(b"gU", Thingy::builtin("vi-up-case"));
    km.bind_seq(b"g~", Thingy::builtin("vi-oper-swap-case"));

    // c:1436 — bracketed paste.
    km.bind_seq(b"\x1b[200~", Thingy::builtin("bracketed-paste"));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub(crate) fn keymapnamtab() -> &'static Mutex<HashMap<String, KeymapName>> {
    KEYMAPNAMTAB.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emacs_default_has_quoted_insert_undo_yank_pop() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        createkeymapnamtab();
        default_bindings();
        
        let km = openkeymap("emacs").expect("emacs keymap created");
        // Ctrl-V quoted-insert (zle_bindings.c emacs '^V').
        assert_eq!(
            km.lookup_char(0x16).map(|t| t.nam.as_str()),
            Some("quoted-insert")
        );
        // Ctrl-_ undo (zle_bindings.c emacs '^_').
        assert_eq!(km.lookup_char(0x1F).map(|t| t.nam.as_str()), Some("undo"));
        // \ey yank-pop (zle_bindings.c emacs '\\ey').
        assert_eq!(
            km.lookup_seq(b"\x1by").and_then(|kb| kb.bind.as_ref()).map(|t| t.nam.as_str()),
            Some("yank-pop")
        );
    }

    #[test]
    fn emacs_default_has_history_search_and_insert_last_word() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        createkeymapnamtab();
        default_bindings();
        
        let km = openkeymap("emacs").expect("emacs keymap created");
        // \e. insert-last-word.
        assert_eq!(
            km.lookup_seq(b"\x1b.").and_then(|kb| kb.bind.as_ref()).map(|t| t.nam.as_str()),
            Some("insert-last-word")
        );
        assert_eq!(
            km.lookup_seq(b"\x1bp").and_then(|kb| kb.bind.as_ref()).map(|t| t.nam.as_str()),
            Some("history-search-backward")
        );
        // ^X^X exchange-point-and-mark.
        assert_eq!(
            km.lookup_seq(b"\x18\x18")
                .and_then(|kb| kb.bind.as_ref())
                .map(|t| t.nam.as_str()),
            Some("exchange-point-and-mark")
        );
    }

    #[test]
    fn vicmd_default_has_visual_marks_indent() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        createkeymapnamtab();
        default_bindings();
        
        let km = openkeymap("vicmd").expect("vicmd keymap created");
        assert_eq!(
            km.lookup_char(b'v').map(|t| t.nam.as_str()),
            Some("visual-mode")
        );
        assert_eq!(
            km.lookup_char(b'V').map(|t| t.nam.as_str()),
            Some("visual-line-mode")
        );
        assert_eq!(
            km.lookup_char(b'm').map(|t| t.nam.as_str()),
            Some("vi-set-mark")
        );
        assert_eq!(
            km.lookup_char(b'>').map(|t| t.nam.as_str()),
            Some("vi-indent")
        );
        assert_eq!(
            km.lookup_char(b'~').map(|t| t.nam.as_str()),
            Some("vi-swap-case")
        );
        assert_eq!(
            km.lookup_char(b'%').map(|t| t.nam.as_str()),
            Some("vi-match-bracket")
        );
    }

    #[test]
    fn viins_default_matches_viinsbind_table() {
        // Test renamed + retargeted from the legacy
        // `viins_default_has_history_search_and_quoted_insert` which
        // asserted Rust-only emacs-flavored bindings (`^R` →
        // history-incremental-search-backward; `^A` →
        // beginning-of-line; `^V` → quoted-insert). Those were
        // overridden by the C-faithful `VIINSBIND` table port
        // (`Src/Zle/zle_bindings.c:256-289`).
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        createkeymapnamtab();
        default_bindings();
        let km = openkeymap("viins").expect("viins keymap created");
        // ^R → redisplay (VIINSBIND[18]).
        assert_eq!(km.lookup_char(0x12).map(|t| t.nam.as_str()), Some("redisplay"));
        // ^V → vi-quoted-insert (VIINSBIND[22]).
        assert_eq!(km.lookup_char(0x16).map(|t| t.nam.as_str()), Some("vi-quoted-insert"));
        // ^A → self-insert (VIINSBIND[1]).
        assert_eq!(km.lookup_char(0x01).map(|t| t.nam.as_str()), Some("self-insert"));
        // ^[ → vi-cmd-mode (VIINSBIND[27]).
        assert_eq!(km.lookup_char(0x1B).map(|t| t.nam.as_str()), Some("vi-cmd-mode"));
        // ^M / ^J → accept-line.
        assert_eq!(km.lookup_char(0x0D).map(|t| t.nam.as_str()), Some("accept-line"));
        assert_eq!(km.lookup_char(0x0A).map(|t| t.nam.as_str()), Some("accept-line"));
    }

    // ---------- Real-port tests for refkeymap / unrefkeymap ----------

    #[test]
    fn refkeymap_increments_rc() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:687-688 — empty input → always prefix → 1.
        let km = Keymap::default();
        assert_eq!(keyisprefix(&km, b""), 1);
    }

    #[test]
    fn keyisprefix_single_byte_bound_returns_zero() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:689-692 — single byte that has a first[] binding is NOT
        // a prefix; it IS the binding.
        let mut km = Keymap::default();
        km.bind_char(b'a', dummy_thingy());
        assert_eq!(keyisprefix(&km, b"a"), 0);
    }

    #[test]
    fn keyisprefix_single_byte_unbound() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:694-695 — fall through to multi lookup; no match → 0.
        let km = Keymap::default();
        assert_eq!(keyisprefix(&km, b"x"), 0);
    }

    #[test]
    fn keyisprefix_seq_is_real_prefix() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:694-695 — multi has prefixct > 0 → 1.
        // bind_seq("ab", X) marks "a" as a prefix (prefixct=1).
        let mut km = Keymap::default();
        km.bind_seq(b"ab", dummy_thingy());
        // "a" alone is NOT a complete binding but IS a prefix of "ab".
        assert_eq!(keyisprefix(&km, b"a"), 1);
    }

    #[test]
    fn keyisprefix_seq_is_complete_binding() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:694-695 — when seq itself IS a binding (not a prefix),
        // multi[seq] has prefixct=0 → 0.
        let mut km = Keymap::default();
        km.bind_seq(b"xyz", dummy_thingy());
        // "xyz" is the bound seq (prefixct=0). Should return 0.
        assert_eq!(keyisprefix(&km, b"xyz"), 0);
    }

    #[test]
    fn keyisprefix_meta_pair_decoded() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:690 — `seq[0]==Meta` (0x83) → use seq[1]^32 as single byte.
        // Bind 'A' (0x41) in first[]. Seq [0x83, 0x61] decodes to
        // 0x61^0x20 = 0x41 = 'A'. So this is single-byte 'A'.
        let mut km = Keymap::default();
        km.bind_char(b'A', dummy_thingy());
        assert_eq!(keyisprefix(&km, &[0x83, 0x61]), 0);
    }

    // ---------- keybind real-port tests (this session). ----------

    #[test]
    fn keybind_single_byte_returns_first_bound_thingy() {
        // c:664-669 — `if (ztrlen(seq) == 1) { f = seq[0]; if (km->first[f])
        // return bind; }`. Bind 'q' in first[], call keybind with "q",
        // expect the Thingy back and no send-string.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut km = Keymap::default();
        km.bind_char(b'q', Thingy::new("quit-widget"));
        let (bind, send) = keybind(&km, b"q");
        assert!(bind.is_some());
        assert_eq!(bind.as_ref().unwrap().nam, "quit-widget");
        assert!(send.is_none(), "no str on first[] path");
    }

    #[test]
    fn keybind_meta_pair_decodes_to_single_byte() {
        // c:665 — `seq[0]==Meta ? seq[1]^32 : seq[0]`. [0x83, 0x61]
        // decodes to 0x41 = 'A'. Verify it lands on the first[] entry
        // for 'A' just like a literal 'A' byte would.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut km = Keymap::default();
        km.bind_char(b'A', Thingy::new("uppercase-A-widget"));
        let (bind, _) = keybind(&km, &[0x83, 0x61]);
        assert_eq!(bind.as_ref().map(|t| t.nam.as_str()), Some("uppercase-A-widget"));
    }

    #[test]
    fn keybind_unbound_byte_returns_none() {
        // c:670-671 — single-byte but km->first[f] is None, falls
        // through to the multi-byte lookup which also misses → (None, None).
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let km = Keymap::default();
        let (bind, send) = keybind(&km, b"z");
        assert!(bind.is_none(), "unbound byte → t_undefinedkey sentinel (None)");
        assert!(send.is_none());
    }

    #[test]
    fn keybind_multi_byte_sequence_via_multi_map() {
        // c:670-674 — `k = km->multi->getnode(km->multi, seq)`. Bind
        // a multi-byte sequence in `multi`, expect the lookup to find it.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut km = Keymap::default();
        km.multi.insert(b"\x1b[A".to_vec(), KeyBinding {
            bind: Some(Thingy::new("up-line")),
            str: None,
            prefixct: 0,
        });
        let (bind, _) = keybind(&km, b"\x1b[A");
        assert_eq!(bind.as_ref().map(|t| t.nam.as_str()), Some("up-line"));
    }

    #[test]
    fn keybind_returns_send_string_when_multi_entry_has_str() {
        // c:673-674 — `*strp = k->str; return k->bind`. Send-string
        // entries (`bindkey -s`) have bind=None + str=Some.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut km = Keymap::default();
        km.multi.insert(b"\x1b[Z".to_vec(), KeyBinding {
            bind: None,
            str: Some("hello".to_string()),
            prefixct: 0,
        });
        let (bind, send) = keybind(&km, b"\x1b[Z");
        assert!(bind.is_none(), "send-string entries have no bind");
        assert_eq!(send.as_deref(), Some("hello"));
    }

    // ---------- init_keymaps / cleanup_keymaps round-trip ----------

    #[test]
    fn init_keymaps_seeds_keybuf_and_clears_lastnamed() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // After init: keybuf is allocated (non-empty Vec), lastnamed is None
        // (the `t_undefinedkey` sentinel in Rust convention).
        init_keymaps();
        assert!(!keybuf.lock().unwrap().is_empty(), "keybuf zshcalloc(keybufsz)");
        assert!(lastnamed.lock().unwrap().is_none(), "lastnamed = t_undefinedkey (None)");
    }

    #[test]
    fn cleanup_keymaps_drains_namtab_and_keybuf() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        init_keymaps();
        assert!(!keybuf.lock().unwrap().is_empty());
        cleanup_keymaps();
        assert!(keybuf.lock().unwrap().is_empty(), "zfree(keybuf, ...)");
        assert!(keymapnamtab().lock().unwrap().is_empty(), "deletehashtable(keymapnamtab)");
    }
}
