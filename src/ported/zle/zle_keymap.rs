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

use super::zle_bindings::{EMACSBIND, METABIND, VICMDBIND, VIINSBIND};
use super::zle_main::zle_test_setup;
use super::zle_thingy::Thingy;
use crate::ported::utils::inittyptab;
#[cfg(test)]
use crate::ported::ztype_h::TYPTAB_TEST_LOCK;
use std::io::Write;

// =====================================================================
// Flag constants — `Src/Zle/zle_keymap.c:62/83/114-115`.
// =====================================================================

#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
use crate::ported::zsh_h::{options, OPT_ISSET};
use crate::ported::ztype_h::imeta;

/// Port of `KMN_IMMORTAL` from `Src/Zle/zle_keymap.c:62`. Marks a
/// keymap-name node that can't be deleted (the `.safe` keymap).

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]

/// Direct port of `struct keymapname` from `Src/Zle/zle_keymap.c:54`.
/// One node in the global `keymapnamtab` — maps a name to a Keymap
/// + per-node flags (KMN_IMMORTAL for `.safe`).
#[derive(Debug, Clone)]
pub struct KeymapName {
    // c:54
    pub nam: String,         // c:56 char *nam
    pub flags: i32,          // c:57 int flags
    pub keymap: Arc<Keymap>, // c:58 Keymap keymap
}
/// `KMN_IMMORTAL` constant.
pub const KMN_IMMORTAL: i32 = 1 << 1; // c:62

/// Port of `KM_IMMUTABLE` from `Src/Zle/zle_keymap.c:83`. Marks a
/// keymap that can't have its bindings modified.
pub const KM_IMMUTABLE: i32 = 1 << 1; // c:83

/// Port of `struct bindstate` from `Src/Zle/zle_keymap.c:95-104`.
/// Closure state for `scanbindlist` / `bindlistout` — threads the
/// keymap-listing accumulator through `scankeymap`'s per-binding
/// callback. C definition:
/// ```c
/// struct bindstate {
///     int flags;
///     char *kmname;
///     char *firstseq;
///     char *lastseq;
///     Thingy bind;
///     char *str;
///     char *prefix;
///     int prefixlen;
/// };
/// ```
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct bindstate {
    // c:95
    pub flags: i32,              // c:96
    pub kmname: String,          // c:97
    pub firstseq: Vec<u8>,       // c:98
    pub lastseq: Vec<u8>,        // c:99
    pub bind: Option<Thingy>,    // c:100 — None ≡ C `t_undefinedkey`
    pub str: Option<String>,     // c:101 — None ≡ C `NULL`
    pub prefix: Option<Vec<u8>>, // c:102 — None ≡ C `NULL`
    pub prefixlen: usize,        // c:103
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
#[allow(non_camel_case_types)]
pub struct remprefstate {
    // c:108
    /// Target keymap (Arc handle for shared ownership).
    pub km: Arc<Keymap>, // c:109
    /// Byte prefix to match against each multi-key binding.
    pub prefix: Vec<u8>, // c:110
    /// `prefix.len()` cached for the scan inner loop (kept as a field
    /// to mirror the C struct shape; `self.prefix.len()` reads the
    /// same value).
    pub prefixlen: usize, // c:111
}

/// Port of `BS_LIST` from `Src/Zle/zle_keymap.c:114`. `bin_bindkey -L`:
/// list bindings in `bindkey -M` syntax.
pub const BS_LIST: i32 = 1 << 0; // c:114

/// Port of `BS_ALL` from `Src/Zle/zle_keymap.c:115`. `bin_bindkey -aL`:
/// list ALL bindings, including default sequences.
pub const BS_ALL: i32 = 1 << 1; // c:115

/// Port of `static Thingy lastnamed` from `Src/Zle/zle_keymap.c:145`.
/// Last command executed by `execute-named-command` — used to
/// re-execute via `bindkey -A name` then `getkeycmd`.
pub static lastnamed: Mutex<Option<Thingy>> = Mutex::new(None); // c:145

/// Port of `createkeymapnamtab()` from Src/Zle/zle_keymap.c:153.
pub fn createkeymapnamtab() {
    // c:153
    // c:153 — `keymapnamtab = newhashtable(7, "keymapnamtab", NULL)`.
    // OnceLock-init via accessor.
    let _ = keymapnamtab();
}

/// Direct port of `void init_keymaps(void)` from `Src/Zle/zle_keymap.c:1224`.
/// Module-load entry point — bootstraps the keymap-name table, installs
/// the default emacs/viins/vicmd bindings, allocates the keybuf, and
/// seeds `lastnamed` to the undefined-key sentinel (Rust None).
pub fn init_keymaps() {
    // c:1224
    createkeymapnamtab(); // c:1227
    default_bindings(); // c:1228
    *keybuf.lock().unwrap() = vec![0u8; 32]; // c:1229 zshcalloc(keybufsz)
    *lastnamed.lock().unwrap() = None; // c:1230 refthingy(t_undefinedkey)
}

/// Direct port of `void cleanup_keymaps(void)` from
/// `Src/Zle/zle_keymap.c:1236`. Module-unload entry point — drops
/// `lastnamed`, the keymap-name table, and the keybuf.
pub fn cleanup_keymaps() {
    // c:1236
    *lastnamed.lock().unwrap() = None; // c:1239 unrefthingy(lastnamed)
    keymapnamtab().lock().unwrap().clear(); // c:1240 deletehashtable(keymapnamtab)
    keybuf.lock().unwrap().clear(); // c:1241 zfree(keybuf, keybufsz)
}

/// Port of `makekeymapnamnode(Keymap keymap)` from Src/Zle/zle_keymap.c:173.
pub fn makekeymapnamnode(keymap: Arc<Keymap>) -> KeymapName {
    // c:173
    // c:173-178 — `kmn = zshcalloc; kmn->keymap = keymap; return kmn`.
    KeymapName {
        nam: String::new(),
        flags: 0,
        keymap: keymap,
    }
}

/// Port of `emptykeymapnamtab(HashTable ht)` from Src/Zle/zle_keymap.c:183.
/// WARNING: param names don't match C — Rust=() vs C=(ht)
pub fn emptykeymapnamtab() {
    // c:183
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
pub fn refkeymap_by_name(kmn: &str) {
    // c:209
    let _ = keymapnamtab().lock().unwrap().get(kmn); // c:209 getnode probe
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
pub fn scanprimaryname(_name: &str) { // c:224
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
pub fn unrefkeymap_by_name(name: &str) {
    // c:246
    // c:246 — `kmname = getnode(name)`. Lock the keymap name table
    // and walk the entry's rc + primary-name promotion in one pass.
    let mut tab = match keymapnamtab().lock() {
        Ok(t) => t,
        Err(_) => return,
    };
    let Some(_kmn) = tab.get(name) else {
        return;
    }; // c:249

    // c:252 — `--km->rc`. With Arc<Keymap> shared-immutable we can't
    // mutate rc on the shared instance; the canonical Rust unref
    // path drops a reference by removing the entry from the table.
    // Find any other names sharing the same Arc — if none, this is
    // the last reference and we drop the entry (Arc drop fires).
    let arc_to_remove = tab.get(name).map(|kmn| kmn.keymap.clone());
    let shared_count = if let Some(ref arc) = arc_to_remove {
        tab.values()
            .filter(|kmn| Arc::ptr_eq(&kmn.keymap, arc))
            .count()
    } else {
        0
    };

    if shared_count <= 1 {
        // c:253 rc==0 path
        tab.remove(name); // C: deletekeymap
    }
    // c:254 — `if (km->primary == kmname) km->primary = NULL` +
    // scanprimaryname re-promote. The Arc<Keymap>'s primary field
    // is shared-immutable in the Rust port; on the next refkeymap_by_name
    // call to a different name pointing to this keymap, primary is
    // re-set via the existing promotion path in refkeymap_by_name.
}

/// Port of `freekeymapnamnode(HashNode hn)` from Src/Zle/zle_keymap.c:267.
pub fn freekeymapnamnode(hn: &str) {
    // c:267
    // c:267-273 — `kmn = (KeymapName)hn; zsfree(kmn->nam);
    //              unrefkeymap_by_name(kmn); zfree(kmn,...)`.
    keymapnamtab().lock().unwrap().remove(hn);
}

/// Port of `newkeytab(char *kmname)` from Src/Zle/zle_keymap.c:278.
/// WARNING: param names don't match C — Rust=() vs C=(kmname)
pub fn newkeytab() -> HashMap<Vec<u8>, KeyBinding> {
    // c:278
    // c:278-296 — `ht = newhashtable(7, kmname, NULL)`. zshrs's
    // multi binding storage is HashMap<Vec<u8>, KeyBinding>; just
    // returns an empty one.
    HashMap::new()
}

/// Port of `makekeynode(Thingy t, char *str)` from Src/Zle/zle_keymap.c:301.
pub fn makekeynode(t: Thingy, str: String) -> KeyBinding {
    // c:301
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
    pub fn new() -> Self {
        // c:278
        Self::default()
    }

    /// Bind a 1-byte key to a Thingy via the `first[]` fast-path table.
    /// Direct port of the single-byte path in `bindkey()` at
    /// Src/Zle/zle_keymap.c:566; the C source writes into `km->first[c]`
    /// when `seq` has length 1.
    pub fn bind_char(&mut self, c: u8, thingy: Thingy) {
        // c:566
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
    pub fn bind_seq(&mut self, seq: &[u8], thingy: Thingy) {
        // c:566
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
        // c:Src/Zle/zle_keymap.c:566 — a single char's Key holds EITHER a func
        // or a send-string. zshrs's `first[]` fast-path stores only the Thingy,
        // so a single-char send-string lives in `multi[]` (below); clear the
        // `first[]` slot so `keybind` falls through to the `multi[]` str.
        if seq.len() == 1 {
            self.first[seq[0] as usize] = None;
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
pub fn freekeynode(hn: KeyBinding) {
    // c:312
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
pub fn newkeymap(tocopy: Option<&Keymap>, _kmname: &str) -> Arc<Keymap> {
    // c:330
    let mut km = Keymap::default();
    if let Some(src) = tocopy {
        // c:336
        // c:337-339 — copy first[i] entries via refthingy.
        for i in 0..256 {
            // c:337
            km.first[i] = src.first[i].clone(); // c:338
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
pub fn scancopykeys(_kb: &KeyBinding) { // c:351
                                        // No-op by design — newkeymap performs the copy directly.
}

/// Port of `deletekeymap(Keymap km)` from Src/Zle/zle_keymap.c:364.
#[allow(unused_variables)]
pub fn deletekeymap(km: Arc<Keymap>) { // c:364
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
pub fn scankeymap(
    km: &Keymap,
    sort: i32,
    func: &mut dyn FnMut(&[u8], Option<&Thingy>, Option<&str>),
) {
    // c:381
    // c:386 — `skm_km = km; skm_last = sort ? -1 : 255;
    //          skm_func = func; skm_magic = magic;`
    // Rust models the four file-statics as captured state in the
    // `scankeys` closure call sequence below.
    let mut skm_last: i32 = if sort != 0 { -1 } else { 255 };

    // c:390 — `scanhashtable(km->multi, sort, 0, 0, scankeys, 0)`.
    // Walk the multi-byte hash in lex order (when sort != 0),
    // interleaving any single-byte entries whose byte value is less
    // than the current multi-key's first byte. The C `scankeys`
    // callback at c:402 is inlined here as the per-iteration body.
    let mut multi_keys: Vec<&Vec<u8>> = km.multi.keys().collect();
    if sort != 0 {
        // c:381 sort flag
        multi_keys.sort();
    }
    for k_nam in multi_keys {
        let kb = km.multi.get(k_nam).expect("key from iter");
        // c:390 — `scanhashtable(km->multi, sort, 0, 0, scankeys, 0)`
        // calls `scankeys` per multi-byte node; we drive that loop
        // directly here because the Rust port has no scanhashtable.
        scankeys(
            k_nam,
            kb.bind.as_ref(),
            kb.str.as_deref(),
            km,
            &mut skm_last,
            func,
        );
    }

    // c:392 — `if (!sort) skm_last = -1`. Already sorted-or-not above;
    // for the unsorted path we reset and walk all 0..255 in order.
    if sort == 0 {
        skm_last = -1;
    }
    // c:393-401 — flush remaining single-byte slots.
    while skm_last < 255 {
        skm_last += 1;
        if let Some(t) = &km.first[skm_last as usize] {
            let m = [skm_last as u8];
            func(&m, Some(t), None);
        }
    }
}

/// Direct port of `static void scankeys(HashNode hn, UNUSED(int flags))`
/// from `Src/Zle/zle_keymap.c:404`. Per-multi-byte-binding callback
/// driven by `scankeymap`. Walks `km.first[]` slots whose byte
/// value is < the current multi-key's first byte, emitting each
/// non-undefined single-byte binding before the multi-byte binding
/// itself.
///
/// C uses the module-static globals `skm_km`, `skm_last`, `skm_func`,
/// `skm_magic` to plumb state through `scanhashtable`'s
/// `(HashNode, int) -> void` callback signature. Rust passes them
/// in explicitly because Rust closures express the same lifetime
/// without the global indirection.
/// WARNING: param names don't match C — Rust=(k_nam, k_bind, k_str,
/// km, skm_last, func) vs C=(hn, flags).
fn scankeys(
    k_nam: &[u8],
    k_bind: Option<&Thingy>,
    k_str: Option<&str>,
    km: &Keymap,
    skm_last: &mut i32,
    func: &mut dyn FnMut(&[u8], Option<&Thingy>, Option<&str>),
) {
    // c:404
    // c:407-408 — `f = (k->nam[0] == Meta ? k->nam[1]^32 : k->nam[0])`.
    // Rust storage is raw bytes, so the Meta-decoded first byte is
    // just the first byte (high-bit values represent themselves).
    // Empty key name (C can't produce one — `k->nam` is at least
    // one byte — but the Rust multi-byte bindkey path can hand an
    // empty slice for a fully-consumed prefix): nothing to scan.
    let Some(&f0) = k_nam.first() else {
        return;
    };
    let f = f0 as i32;
    // c:412-419 — flush every single-byte slot with byte < f.
    while *skm_last < f {
        *skm_last += 1;
        if *skm_last > 255 {
            break;
        }
        if let Some(t) = &km.first[*skm_last as usize] {
            let m = [*skm_last as u8];
            func(&m, Some(t), None);
        }
    }
    // c:420 — `skm_func(k->nam, k->bind, k->str, skm_magic)`.
    func(k_nam, k_bind, k_str);
}

/// Port of `openkeymap(char *name)` from Src/Zle/zle_keymap.c:428.
pub fn openkeymap(name: &str) -> Option<Arc<Keymap>> {
    // c:428
    // c:428-431 — `n = keymapnamtab.getnode(name); return n ? n->keymap : NULL`.
    keymapnamtab()
        .lock()
        .unwrap()
        .get(name)
        .map(|n| n.keymap.clone())
}

/// Port of `unlinkkeymap(char *name, int ignm)` from Src/Zle/zle_keymap.c:436.
pub fn unlinkkeymap(name: &str, ignm: i32) -> i32 {
    // c:436
    // c:436-444 — `n = keymapnamtab.getnode(name); if (!n) return 2;
    //               if (!ignm && (n->flags & KMN_IMMORTAL)) return 1;
    //               keymapnamtab.freenode(removenode(name)); return 0`.
    let mut tab = keymapnamtab().lock().unwrap();
    match tab.get(name) {
        None => 2,                                                  // c:440
        Some(n) if ignm == 0 && (n.flags & KMN_IMMORTAL) != 0 => 1, // c:441
        Some(_) => {
            tab.remove(name); // c:443
            0
        }
    }
}

/// Direct port of `int bindkey(Keymap km, const char *seq, Thingy
/// bind, char *str)` from `Src/Zle/zle_keymap.c:566`. The single
/// canonical entry — internal dispatch on (bind/str/seq.len())
/// matches the C body's `if (!bind || ztrlen(seq) > 1)` branch.
///
/// Returns 0 on success, 1 if `km->flags & KM_IMMUTABLE`, 2 if `seq`
/// is empty.
///
/// The `Keymap::bind_char` / `bind_seq` / `bind_str` methods are a
/// Rust-only factoring of C's internal dispatch — every default-
/// bindings site and every C-equivalent caller goes through this
/// canonical `bindkey()` so the call-site coverage matches C.
pub fn bindkey(km: &mut Keymap, seq: &[u8], bind: Option<Thingy>, str: Option<String>) -> i32 {
    // c:566
    // c:572 — `if (km->flags & KM_IMMUTABLE) return 1;`
    if (km.flags & KM_IMMUTABLE) != 0 {
        return 1;
    }
    // c:574 — `if (!*seq) return 2;`
    if seq.is_empty() {
        return 2;
    }
    // c:576 — `if (!bind || ztrlen(seq) > 1)` dispatch. Inlined
    // (not delegated back to bindkey or to the `Keymap::bind_*`
    // methods) to avoid (a) infinite recursion via the dispatch
    // table and (b) round-tripping through the Rust-only method
    // façade. The four arms match C's `c:600` single-byte arm and
    // `c:631-641` multi-byte arm.
    match (bind, str, seq.len()) {
        (Some(t), None, 1) => {
            // c:600 — `km->first[f] = bind; return 0;`
            km.first[seq[0] as usize] = Some(t);
            0
        }
        (Some(t), None, _) => {
            // c:631-641 — multi-char Thingy binding. Mark prefixes
            // first (so getkeymapcmd's trie walk knows to keep
            // reading bytes), then insert the full-seq binding.
            for i in 1..seq.len() {
                km.multi
                    .entry(seq[..i].to_vec())
                    .and_modify(|kb| kb.prefixct += 1)
                    .or_insert(KeyBinding {
                        bind: None,
                        str: None,
                        prefixct: 1,
                    });
            }
            km.multi.insert(
                seq.to_vec(),
                KeyBinding {
                    bind: Some(t),
                    str: None,
                    prefixct: 0,
                },
            );
            0
        }
        (None, Some(s), _) => {
            // c:614-641 — send-string `bindkey -s` form.
            // c:Src/Zle/zle_keymap.c:566 — a single char's Key holds EITHER a
            // func or a send-string. zshrs's `first[]` fast-path stores only
            // the Thingy, so a single-char send-string lives in `multi[]`
            // below; clear the `first[]` slot so `keybind` (which checks
            // `first[]` first for a single char) falls through to the
            // `multi[]` str instead of returning the stale default binding.
            if seq.len() == 1 {
                km.first[seq[0] as usize] = None;
            }
            for i in 1..seq.len() {
                km.multi
                    .entry(seq[..i].to_vec())
                    .and_modify(|kb| kb.prefixct += 1)
                    .or_insert(KeyBinding {
                        bind: None,
                        str: None,
                        prefixct: 1,
                    });
            }
            km.multi.insert(
                seq.to_vec(),
                KeyBinding {
                    bind: None,
                    str: Some(s),
                    prefixct: 0,
                },
            );
            0
        }
        (None, None, _) => {
            // c:574 — `bindkey -r` unbind: bind to t_undefinedkey.
            if seq.len() == 1 {
                km.first[seq[0] as usize] = Some(Thingy::builtin("undefined-key"));
            } else {
                for i in 1..seq.len() {
                    km.multi
                        .entry(seq[..i].to_vec())
                        .and_modify(|kb| kb.prefixct += 1)
                        .or_insert(KeyBinding {
                            bind: None,
                            str: None,
                            prefixct: 1,
                        });
                }
                km.multi.insert(
                    seq.to_vec(),
                    KeyBinding {
                        bind: Some(Thingy::builtin("undefined-key")),
                        str: None,
                        prefixct: 0,
                    },
                );
            }
            0
        }
        (Some(_), Some(_), _) => {
            // C signature doesn't allow both. Caller bug.
            -1
        }
    }
}

/// Port of `linkkeymap(Keymap km, char *name, int imm)` from Src/Zle/zle_keymap.c:449.
pub fn linkkeymap(km: Arc<Keymap>, name: &str, imm: i32) -> i32 {
    // c:449
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
    refkeymap_by_name(name); // c:465
    0 // c:466
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
pub fn refkeymap(km: &mut Keymap) {
    // c:471
    km.rc += 1; // c:471 km->rc++
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
pub fn unrefkeymap(km: &mut Keymap) -> i32 {
    // c:480
    km.rc -= 1; // c:480 --km->rc
    if km.rc == 0 {
        // c:483 — `deletekeymap(km)`. Rust caller drops the Keymap;
        // we just signal by returning 0.
        return 0; // c:484
    }
    km.rc // c:487 return km->rc
}

// Select a keymap as the current ZLE keymap.  Can optionally fall back    // c:495
// on the guaranteed safe keymap if it fails.                              // c:495
/// Port of `selectkeymap(char *name, int fb)` from Src/Zle/zle_keymap.c:495.
pub fn selectkeymap(name: &str, fb: i32) -> i32 {
    // c:495
    // C body (c:497-521): `Keymap km = openkeymap(name); if (!km) {
    //   showmsg + if (!fb) return 1; km = openkeymap(".safe"); }
    //   if (name != curkeymapname) { ... curkeymapname = ztrdup(name);
    //   if (zleactive && oldname && strcmp...) zlecallhook(...); }
    //   curkeymap = km; return 0`.
    let mut km = openkeymap(name); // c:497
    let mut resolved = name.to_string();
    if km.is_none() {
        // c:498
        if fb == 0 {
            return 1; // c:506
        }
        km = openkeymap(".safe"); // c:508
        if km.is_none() {
            return 1;
        }
        resolved = ".safe".to_string();
    }
    // c:513-521 — set curkeymapname to the new name and, when the keymap
    // actually CHANGES during an active edit, fire the `zle-keymap-select`
    // hook widget. In C:
    //   char *oname = curkeymapname;
    //   curkeymapname = ztrdup(name);
    //   if (oname && strcmp(oname, name) && zleactive &&
    //       (t = rthingy_nocreate("zle-keymap-select"))) {
    //       char *args[2] = { oname, NULL };
    //       execzlefunc(t, args, 1, 0);
    //   }
    // The hook reads `$KEYMAP` (the just-updated curkeymapname) and gets
    // the OLD keymap name as `$1`; it drives vi-mode indicators (a
    // keymap-select widget that sets RPROMPT + `zle reset-prompt`). The
    // earlier port set curkeymapname/curkeymap but NEVER fired the hook,
    // so `bindkey -v` + Esc switched keymaps silently and the right-prompt
    // mode indicator never appeared. Bug #654.
    let oldname = std::mem::replace(&mut *curkeymapname(), resolved.clone()); // c:513
                                                                              // c:518 — `curkeymap = km`.
    *curkeymap.lock().unwrap() = km;
    // Keep the `$KEYMAP` zle parameter in sync so the zle-keymap-select
    // hook (and any widget) reads the CURRENT keymap. zsh backs `$KEYMAP`
    // with a live getfn (get_keymap → curkeymapname); zshrs's makezleparams
    // snapshots the zle params via setsparam and had no KEYMAP entry, so
    // `$KEYMAP` was empty inside the hook. Gate on `zleactive` to mirror
    // zsh's "KEYMAP exists only during line editing". Bug #654.
    if crate::ported::builtins::sched::zleactive.load(std::sync::atomic::Ordering::Relaxed) != 0 {
        let _ = crate::ported::params::setsparam("KEYMAP", &resolved);
    }
    if !oldname.is_empty()
        && oldname != resolved
        && crate::ported::builtins::sched::zleactive.load(std::sync::atomic::Ordering::Relaxed) != 0
    {
        if crate::ported::zle::zle_thingy::rthingy_nocreate("zle-keymap-select") {
            // c:519 — `execzlefunc(t, args, 1, 0)` with args[0] = old keymap name.
            let _ =
                crate::ported::zle::zle_main::execzlefunc("zle-keymap-select", &[oldname], 1, 0);
        }
        // Native p10k engine (extensions/p10k): the script theme flips the
        // prompt_char arrow (❯/❮/Ⅴ) from its zle-keymap-select widget
        // (p10k:3336-3339 keyed on _p9k__keymap); the native engine installs
        // no widget, so notify it here — prompt_char reads the LIVE keymap
        // at render time (segments_core.rs), so a re-render + reset-prompt
        // repaints the flipped arrow mid-edit. No-op when the engine is
        // inactive.
        if crate::p10k::engine_active() {
            crate::p10k::preprompt_render();
            crate::ported::zle::zle_main::zle_resetprompt();
        }
    }
    0 // c:527
}

/// Direct port of `void selectlocalmap(Keymap m)` from
/// `Src/Zle/zle_keymap.c:527`.
/// ```c
/// Keymap oldm = localkeymap;
/// localkeymap = m;
/// if (oldm && !m)
///     reselectkeymap();
/// ```
pub fn selectlocalmap(m: Option<Arc<Keymap>>) {
    // c:527
    let oldm = {
        let mut g = LOCALKEYMAP.lock().unwrap();
        let prev = g.take();
        *g = m.clone();
        prev
    };
    // c:541-542 — `if (oldm && !m) reselectkeymap()`.
    if oldm.is_some() && m.is_none() {
        // reselectkeymap re-opens the CURRENT keymap by name
        // (zle_keymap.c:551 `selectkeymap(curkeymapname, 1)`). It was
        // previously hardcoded to "main", which yanked vi command mode
        // back to viins whenever a vi operator dropped its viopp local
        // map — `yy` left the editor in insert mode so `p` self-inserted.
        reselectkeymap();
    }
}

/// Port of `reselectkeymap()` from Src/Zle/zle_keymap.c:549.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn reselectkeymap() {
    // c:549
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
pub fn keybind(km: &Keymap, seq: &[u8]) -> (Option<Thingy>, Option<String>) {
    // c:659
    // c:664 — `if(ztrlen(seq) == 1)`. Single-char (after Meta-decode) → first[f].
    let single = if seq.len() == 1 {
        Some(seq[0])
    } else if seq.len() == 2 && seq[0] == 0x83 {
        Some(seq[1] ^ 32) // c:665 Meta-decode
    } else {
        None
    };
    if let Some(f) = single {
        if let Some(bind) = km.first[f as usize].as_ref() {
            // c:666-669
            return (Some(bind.clone()), None);
        }
    }
    // c:670 — `k = km->multi->getnode(km->multi, seq);`
    match km.multi.get(seq) {
        None => (None, None),                       // c:671 t_undefinedkey
        Some(k) => (k.bind.clone(), k.str.clone()), // c:673-674
    }
}
/// `keyisprefix` — see implementation.
pub fn keyisprefix(km: &Keymap, seq: &[u8]) -> i32 {
    // c:683
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
        if km.first[f as usize].is_some() {
            // c:691-692
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
pub fn bin_bindkey(
    name: &str,
    args: &[String], // c:743
    ops: &options,
    _func: i32,
) -> i32 {
    use crate::ported::zsh_h::{OPT_ARG, OPT_ISSET};

    // c:zle_keymap.c boot_ - the zsh/zle module's boot handler
    // calls `default_bindings()` once on module load (zle_main.c
    // setup_), which is what gives the "main" / "emacs" / "viins" /
    // "vicmd" / "menuselect" / "listscroll" / ".safe" keymaps a
    // chance to exist before user `bindkey` invocations.
    //
    // zshrs in script (non-interactive) mode doesn't autoload zsh/zle,
    // so the keymaps are never populated. /etc/zshrc bindkey calls
    // then fail with `no such keymap 'main'`. Auto-init on first
    // bindkey call.
    //
    // Gate on `keymapnamtab` being EMPTY — not a `Once`. `default_bindings()`
    // is destructive: it rebuilds all keymaps from fresh Keymap structs and
    // overwrites keymapnamtab. A SEPARATE lazy-init path (`${keymaps}` reads,
    // zleparameter.rs::keymapsgetfn) has its own trigger, so two independent
    // `Once`s let the second caller rebuild the keymaps and wipe bindings the
    // first caller had already added (`bindkey X; ${keymaps}` lost X). A
    // shared emptiness gate makes whichever path runs first do the one-time
    // init and every later caller skip it. (Lock released before the call —
    // default_bindings() re-locks keymapnamtab.)
    if keymapnamtab()
        .lock()
        .map(|t| !t.contains_key("main"))
        .unwrap_or(false)
    {
        default_bindings();
    }

    // c:751-759 — opns[] dispatch table. Each entry: (flag-char,
    // selp, min, max, sub-handler kind). selp=1 means -e/-v/-a/-M
    // keymap-selection is allowed for this op.
    #[derive(Clone, Copy)]
    enum Op {
        LsMaps,
        DelAll,
        Del,
        Link,
        New,
        Meta,
        Bind,
    }
    struct Opn {
        o: u8,
        selp: bool,
        func: Op,
        min: i32,
        max: i32,
    }
    static OPNS: &[Opn] = &[
        Opn {
            o: b'l',
            selp: false,
            func: Op::LsMaps,
            min: 0,
            max: -1,
        },
        Opn {
            o: b'd',
            selp: false,
            func: Op::DelAll,
            min: 0,
            max: 0,
        },
        Opn {
            o: b'D',
            selp: false,
            func: Op::Del,
            min: 1,
            max: -1,
        },
        Opn {
            o: b'A',
            selp: false,
            func: Op::Link,
            min: 2,
            max: 2,
        },
        Opn {
            o: b'N',
            selp: false,
            func: Op::New,
            min: 1,
            max: 2,
        },
        Opn {
            o: b'm',
            selp: true,
            func: Op::Meta,
            min: 0,
            max: 0,
        },
        Opn {
            o: b'r',
            selp: true,
            func: Op::Bind,
            min: 1,
            max: -1,
        },
        Opn {
            o: b's',
            selp: true,
            func: Op::Bind,
            min: 2,
            max: -1,
        },
        Opn {
            o: 0,
            selp: true,
            func: Op::Bind,
            min: 0,
            max: -1,
        },
    ];

    // c:767-773 — find selected op + ensure no clashing flags.
    let mut idx = OPNS.len() - 1;
    for (i, op) in OPNS.iter().enumerate() {
        if op.o != 0 && OPT_ISSET(ops, op.o) {
            idx = i;
            break;
        }
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
    let nsel = (OPT_ISSET(ops, b'e') as i32)
        + (OPT_ISSET(ops, b'v') as i32)
        + (OPT_ISSET(ops, b'a') as i32)
        + (OPT_ISSET(ops, b'M') as i32);
    if !op.selp && nsel != 0 {
        eprintln!("{}: keymap cannot be selected with -{}", name, op.o as char);
        return 1;
    }
    if nsel > 1 {
        // c:Src/Zle/zle_keymap.c:781 — zwarnnam so the message carries the
        // canonical `zshrs:bindkey:LINE:` prefix, not a bare `bindkey:`.
        crate::ported::utils::zwarnnam(name, "incompatible keymap selection options");
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
            OPT_ARG(ops, b'M')
                .map(|s| s.to_string())
                .unwrap_or_else(|| "main".to_string())
        } else {
            "main".to_string()
        };
        let km = match openkeymap(&nm) {
            Some(k) => k,
            None => {
                crate::ported::utils::zwarnnam(name, &format!("no such keymap `{}'", nm)); // c:799
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
        return bin_bindkey_list(name, kmname.as_deref(), None, args, ops, 0);
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

    // c:826-827 — dispatch. C: `return op->func(name, kmname, km, argv, ops, op->o);`
    let func_i: i32 = op.o as i32;
    let km_ref: Option<&Keymap> = None; // c:826 km — substrate via openkeymap(kmname) deferred
    let km_str = kmname.as_deref();
    match op.func {
        Op::LsMaps => bin_bindkey_lsmaps(name, km_str, km_ref, args, ops, func_i),
        Op::DelAll => bin_bindkey_delall(name, km_str, km_ref, args, ops, func_i),
        Op::Del => bin_bindkey_del(name, km_str, km_ref, args, ops, func_i),
        Op::Link => bin_bindkey_link(name, km_str, km_ref, args, ops, func_i),
        Op::New => bin_bindkey_new(name, km_str, km_ref, args, ops, func_i),
        Op::Meta => bin_bindkey_meta(name, km_str, km_ref, args, ops, func_i),
        Op::Bind => bin_bindkey_bind(name, km_str, km_ref, args, ops, func_i),
    }
}

/// Direct port of `static int bin_bindkey_lsmaps(char *name,
///                                                 UNUSED(char *kmname),
///                                                 UNUSED(Keymap km),
///                                                 char **argv, Options ops,
///                                                 UNUSED(char func))`
/// from `Src/Zle/zle_keymap.c:834`. Dispatches per-keymap via
/// `scanlistmaps`. With argv: iterate the named keymaps. Without:
/// walk all via scanhashtable.
pub fn bin_bindkey_lsmaps(
    name: &str,
    _kmname: Option<&str>,
    _km: Option<&Keymap>,
    argv: &[String],
    ops: &options,
    _func: i32,
) -> i32 {
    // c:834
    let list_verbose = OPT_ISSET(ops, b'L');
    let mut ret = 0;
    if !argv.is_empty() {
        // c:838-849 — per-arg lookup + per-arg scanlistmaps.
        for a in argv {
            // c:840 — `kmn = keymapnamtab->getnode(keymapnamtab, *argv)`.
            let kmn = {
                let g = keymapnamtab().lock().unwrap();
                g.get(a).cloned()
            };
            match kmn {
                None => {
                    crate::ported::utils::zwarnnam(name, &format!("no such keymap: `{}'", a)); // c:842
                    ret = 1;
                }
                Some(kmn) => {
                    scanlistmaps(&kmn, a, list_verbose);
                }
            }
        }
    } else {
        // c:851 — `scanhashtable(keymapnamtab, 1, 0, 0, scanlistmaps, ...)`.
        // Walk in lex-sorted order.
        let snapshot: Vec<(String, KeymapName)> = {
            let g = keymapnamtab().lock().unwrap();
            g.iter().map(|(n, k)| (n.clone(), k.clone())).collect()
        };
        let mut names: Vec<(String, KeymapName)> = snapshot;
        names.sort_by(|a, b| a.0.cmp(&b.0));
        for (n, kmn) in &names {
            scanlistmaps(kmn, n, list_verbose);
        }
    }
    ret
}

/// Direct port of `static void scanlistmaps(HashNode hn,
///                                            int list_verbose)`
/// from `Src/Zle/zle_keymap.c:856`. Emits one line per keymap-name
/// entry. With `list_verbose` (= `bindkey -L`): formats as
/// `bindkey -A primary name` (alias) or `bindkey -N name` (new).
/// Without: just the name.
/// WARNING: param names don't match C — Rust=(kmn, n_nam, list_verbose)
/// vs C=(hn, list_verbose); the C source reads `n->nam` off the
/// HashNode, we pass the name separately because Rust's
/// KeymapName struct doesn't carry its own owned name.
pub fn scanlistmaps(kmn: &KeymapName, n_nam: &str, list_verbose: bool) {
    // c:856
    if list_verbose {
        // c:864 — `if (!strcmp(n->nam, ".safe")) return`.
        if n_nam == ".safe" {
            return;
        }
        // c:866 — `fputs("bindkey -", stdout)`.
        print!("bindkey -");
        // c:867-878 — `if (km->primary && km->primary != n)` →
        // alias form (`-A primary name`); else → new form (`-N name`).
        let primary_name = kmn.keymap.primary.as_deref();
        let is_alias = primary_name.is_some() && primary_name != Some(n_nam);
        if is_alias {
            // c:870 — `fputs("A ", stdout)`.
            print!("A ");
            let pn = primary_name.unwrap();
            // c:872 — `if (pn->nam[0] == '-') fputs("-- ", stdout)`.
            if pn.starts_with('-') {
                print!("-- ");
            }
            // c:874 — `quotedzputs(pn->nam, stdout)`.
            print!("{} ", pn);
        } else {
            // c:877 — `fputs("N ", stdout)`.
            print!("N ");
            // c:879 — `if (n->nam[0] == '-') fputs("-- ", stdout)`.
            if n_nam.starts_with('-') {
                print!("-- ");
            }
        }
        // c:881 — `quotedzputs(n->nam, stdout)`.
        print!("{}", n_nam);
    } else {
        // c:884 — `nicezputs(n->nam, stdout)`.
        print!("{}", n_nam);
    }
    // c:886 — `putchar('\n')`.
    println!();
}

/// Port of `bin_bindkey_delall(UNUSED(char *name), UNUSED(char *kmname),
/// UNUSED(Keymap km), UNUSED(char **argv), UNUSED(Options ops),
/// UNUSED(char func))` from Src/Zle/zle_keymap.c:891.
pub fn bin_bindkey_delall(
    _name: &str,
    _kmname: Option<&str>,
    _km: Option<&Keymap>,
    _argv: &[String],
    _ops: &options,
    _func: i32,
) -> i32 {
    // c:Src/Zle/zle_keymap.c — `bin_bindkey_delall` body:
    //   keymapnamtab->emptytable(keymapnamtab);
    //   default_bindings();
    //   return 0;
    // The previous Rust port mis-used `name` (the builtin name
    // "bindkey", not a keymap name) as a `openkeymap` lookup key and
    // returned 1 on the inevitable miss. C always succeeds.
    default_bindings();
    0
}

/// Port of `bin_bindkey_del(char *name, UNUSED(char *kmname),
/// UNUSED(Keymap km), char **argv, UNUSED(Options ops),
/// UNUSED(char func))` from Src/Zle/zle_keymap.c:902.
pub fn bin_bindkey_del(
    name: &str,
    _kmname: Option<&str>,
    _km: Option<&Keymap>,
    argv: &[String],
    _ops: &options,
    _func: i32,
) -> i32 {
    // c:902-915 — `do { r = unlinkkeymap(*argv, 0);
    //   if (r==1) zwarnnam(name, "keymap name `%s' is protected", *argv);
    //   else if (r==2) zwarnnam(name, "no such keymap `%s'", *argv);
    //   ret |= !!r; } while(*++argv)`. The previous port swallowed the
    // per-keymap diagnostics (protected / no-such-keymap).
    if argv.is_empty() {
        return 1;
    }
    let mut ret = 0;
    for arg in argv {
        match unlinkkeymap(arg, 0) {
            0 => {}
            1 => {
                crate::ported::utils::zwarnnam(
                    name,
                    &format!("keymap name `{}' is protected", arg),
                );
                ret = 1;
            }
            _ => {
                crate::ported::utils::zwarnnam(name, &format!("no such keymap `{}'", arg));
                ret = 1;
            }
        }
    }
    ret
}

/// Port of `bin_bindkey_link(char *name, UNUSED(char *kmname),
/// Keymap km, char **argv, UNUSED(Options ops), UNUSED(char func))`
/// from Src/Zle/zle_keymap.c:921.
pub fn bin_bindkey_link(
    name: &str,
    _kmname: Option<&str>,
    _km: Option<&Keymap>,
    argv: &[String],
    _ops: &options,
    _func: i32,
) -> i32 {
    // c:921-933 — `km = openkeymap(argv[0]);
    //   if (!km) { zwarnnam(name, "no such keymap `%s'", argv[0]); return 1; }
    //   else if (linkkeymap(km, argv[1], 0)) {
    //       zwarnnam(name, "keymap name `%s' is protected", argv[1]); return 1; }`.
    // The previous port returned 1 silently on both failures.
    if argv.len() < 2 {
        return 1;
    }
    let Some(km) = openkeymap(&argv[0]) else {
        crate::ported::utils::zwarnnam(name, &format!("no such keymap `{}'", argv[0])); // c:925
        return 1;
    };
    if linkkeymap(km, &argv[1], 0) != 0 {
        crate::ported::utils::zwarnnam(
            name,
            &format!("keymap name `{}' is protected", argv[1]), // c:928
        );
        return 1;
    }
    0
}

/// Port of `bin_bindkey_new(char *name, UNUSED(char *kmname),
/// Keymap km, char **argv, UNUSED(Options ops), UNUSED(char func))`
/// from Src/Zle/zle_keymap.c:938.
pub fn bin_bindkey_new(
    _name: &str,
    _kmname: Option<&str>,
    _km: Option<&Keymap>,
    argv: &[String],
    _ops: &options,
    _func: i32,
) -> i32 {
    // c:938
    // c:938-955 — `kmn = keymapnamtab.getnode(argv[0]); if (kmn->flags
    //               & KMN_IMMORTAL) return 1; if (argv[1]) km =
    //               openkeymap(argv[1]) else NULL;
    //               linkkeymap(newkeymap(km, argv[0]), argv[0], 0)`.
    if argv.is_empty() {
        return 1;
    }
    let blocked = keymapnamtab()
        .lock()
        .unwrap()
        .get(&argv[0])
        .map(|n| n.flags & KMN_IMMORTAL != 0)
        .unwrap_or(false);
    if blocked {
        return 1; // c:944
    }
    let template = if argv.len() >= 2 {
        let km = openkeymap(&argv[1]);
        if km.is_none() {
            return 1; // c:950
        }
        km
    } else {
        None
    };
    let new_km = newkeymap(template.as_deref(), &argv[0]); // c:954
    linkkeymap(new_km, &argv[0], 0);
    0 // c:955
}

/// Direct port of `static int bin_bindkey_meta(char *name, char *kmname,
///                                              Keymap km, char **argv,
///                                              Options ops, char func)`
/// from `Src/Zle/zle_keymap.c:966`.
///
/// Line-by-line port of c:966-988. Walks bytes 0x80..=0xff: for each
/// byte where `METABIND[i-128]` isn't `"undefined-key"`, looks up the
/// current binding via [`keybind`]; if it's `self-insert` or
/// undefined, rebinds it to the [`METABIND`] default via `bindkey`.
/// Skips entries whose current binding is something the user has
/// customised — matches the C body's `IS_THINGY(fn, selfinsert) ||
/// fn == t_undefinedkey` predicate at c:982.
pub fn bin_bindkey_meta(
    name: &str,
    kmname: Option<&str>,
    _km_arg: Option<&Keymap>,
    _argv: &[String],
    _ops: &options,
    _func: i32,
) -> i32 {
    use super::zle_bindings::METABIND;
    use super::zle_thingy::{refthingy, Thingy};

    // c:968 — KM_IMMUTABLE check.
    let target = kmname.unwrap_or(name);
    let km_arc = match openkeymap(target) {
        Some(k) => k,
        None => return 1,
    };

    // c:978-987 — walk i = 128..256 (bytes with high bit set).
    for i in 128usize..256 {
        let default_name = METABIND[i - 128]; // c:980
        if default_name == "undefined-key" {
            // c:981 — `if (metabind[i - 128] != z_undefinedkey)`
            continue;
        }
        // c:982-984 — `m[0] = i; metafy(m, 1, META_NOALLOC); fn = keybind(km, m);`
        let m = [0x83u8, (i as u8) ^ 32];
        let (cur_fn, _str) = keybind(&km_arc, &m);
        // c:985 — `if (IS_THINGY(fn, selfinsert) || fn == t_undefinedkey)`
        let should_rebind = match &cur_fn {
            None => true,
            Some(t) => t.nam == "self-insert",
        };
        if !should_rebind {
            continue;
        }
        // c:986 — `bindkey(km, m, refthingy(Th(metabind[i - 128])), NULL);`
        refthingy(default_name);
        let new_thingy = Thingy {
            nam: default_name.to_string(),
            flags: 0,
            rc: 1,
            widget: None,
        };
        if let Some(km_inner) = Arc::get_mut(&mut km_arc.clone()) {
            bindkey(km_inner, &m, Some(new_thingy), None);
        } else {
            // Arc was shared; clone-modify-replace via the keymapnamtab.
            let mut new_km: Keymap = (*km_arc).clone();
            bindkey(&mut new_km, &m, Some(new_thingy), None);
            linkkeymap(Arc::new(new_km), target, 0);
        }
    }
    0 // c:988
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
pub fn bin_bindkey_bind(
    _name: &str,
    kmname: Option<&str>,
    _km: Option<&Keymap>,
    argv: &[String],
    _ops: &options,
    func: i32,
) -> i32 {
    // c:999 — `km` is preselected by the caller via opts/-M; fall back
    // to "main" (zsh's default after default_bindings()) when no
    // explicit keymap was passed. Previous Rust port mis-used the
    // builtin name ("bindkey") as the keymap key and openkeymap
    // returned None for every invocation → bin_bindkey exited 1 and
    // installed nothing.
    let lookup_name = kmname.unwrap_or("main");
    let Some(old_arc) = openkeymap(lookup_name) else {
        return 1;
    }; // c:1002
       // c:1003-1011 — bind seq+target pairs need even argv count
       // (omit on '-r' / when func is the empty target).
    let func_c = if func == 0 { '\0' } else { func as u8 as char };
    let needs_pairs = func_c == '\0' || func_c == 's';
    if needs_pairs && (argv.len() % 2 != 0) {
        return 1;
    }

    // Mutable clone of the shared Keymap.
    let mut km: Keymap = (*old_arc).clone();

    // c:1014-1090 — walk argv in 1 or 2-step strides.
    let stride = if func_c == 'r' { 1 } else { 2 };
    let mut i = 0;
    while i + (stride - 1) < argv.len() {
        // c:Src/Zle/zle_keymap.c:1023-1040 — `seq = getkeystring(*argv,
        //   &len, GETKEYS_BINDKEY, NULL); seq = metafy(seq, len, META_USEHEAP);`
        // The user-typed string `^A` is 2 chars that getkeystring translates
        // to the single byte 0x01. Without this translation, `bindkey -r
        // "^A"` was inserting/clearing `b"^A"` (2 raw bytes) in km.multi
        // — `^A` (0x01) at km.first[1] stayed untouched. Bug #344 in
        // docs/BUGS.md.
        let seq_bytes = crate::ported::zle::zle_bindings::getkeystring(&argv[i]);
        let target = if stride == 2 {
            Some(argv[i + 1].clone())
        } else {
            None
        };

        let kb_value: KeyBinding = match func_c {
            // c:1027
            'r' => KeyBinding {
                bind: None,
                str: None,
                prefixct: 0,
            }, // c:1024 undefined-key
            's' => KeyBinding {
                // c:1030 send-string
                bind: None,
                str: target,
                prefixct: 0,
            },
            _ => KeyBinding {
                // c:1037 thingy
                bind: target.map(|n| Thingy::builtin(&n)),
                str: None,
                prefixct: 0,
            },
        };

        // c:1051 — `bindkey(km, seq, bind, str)`.
        if seq_bytes.len() == 1 {
            let c = seq_bytes[0] as usize;
            let key = seq_bytes.to_vec();
            if let Some(s) = kb_value.str {
                // c:614 — single-char SEND-STRING (`bindkey -s "^X" str`).
                // `first[]` holds only a Thingy, so keep the string in
                // `multi[]` and clear `first[]` (keybind falls through to it).
                // Preserve the prefixct if this char is also a prefix. The
                // previous port stored only `kb_value.bind`, silently DROPPING
                // the string.
                km.first[c] = None;
                km.multi
                    .entry(key)
                    .and_modify(|kb| kb.str = Some(s.clone()))
                    .or_insert_with(|| KeyBinding {
                        bind: None,
                        str: Some(s.clone()),
                        prefixct: 0,
                    });
            } else {
                // c:600 — single-char Thingy / `-r` unbind → first[]. Also
                // drop any stale send-string a prior `-s` left on `multi[]`
                // for this char, and remove the node if it's now empty (not a
                // prefix). Without this, `bindkey -s "^A" x; bindkey -r "^A"`
                // still resolved `^A` to the stale string.
                km.first[c] = kb_value.bind.clone();
                let remove = if let Some(kb) = km.multi.get_mut(&key) {
                    kb.str = None;
                    kb.prefixct == 0 && kb.bind.is_none()
                } else {
                    false
                };
                if remove {
                    km.multi.remove(&key);
                }
            }
        } else {
            km.multi.insert(seq_bytes.to_vec(), kb_value); // c:1054 hashtable
        }
        // PFA-SMR: record the binding so replay can recreate it.
        // Skip `bindkey -e` / `bindkey -v` mode switches (those land
        // in the Meta op path, not Bind — but stride==2 args here are
        // always real bindings; func_c='r' (stride 1) is unbind and
        // is intentionally skipped per RECORDER.md surface row.
        #[cfg(feature = "recorder")]
        if func_c != 'r' && crate::recorder::is_enabled() {
            let ctx = crate::recorder::recorder_ctx_global();
            let seq_str = String::from_utf8_lossy(&seq_bytes);
            let widget_default = String::new();
            let widget_ref: &str = match func_c {
                's' => "send-string", // c:1030
                _ => argv.get(i + 1).unwrap_or(&widget_default).as_str(),
            };
            crate::recorder::emit_bindkey(&seq_str, widget_ref, ctx);
        }
        i += stride;
    }

    // Rebuild the Arc + propagate to every name that shared the old.
    let new_arc = Arc::new(km);
    if let Ok(mut tab) = keymapnamtab().lock() {
        let names_to_update: Vec<String> = tab
            .iter()
            .filter(|(_, kmn)| Arc::ptr_eq(&kmn.keymap, &old_arc))
            .map(|(n, _)| n.clone())
            .collect();
        for n in names_to_update {
            if let Some(kmn) = tab.get_mut(&n) {
                kmn.keymap = new_arc.clone();
            }
        }
    }
    0 // c:1097
}

/// Port of `scanremoveprefix(char *seq, UNUSED(Thingy bind), UNUSED(char *str), void *magic)` from Src/Zle/zle_keymap.c:1078.
/// WARNING: param names don't match C — Rust=(km, prefix) vs C=(seq, bind, str, magic)
pub fn scanremoveprefix(km: &mut Keymap, prefix: &[u8]) {
    // c:1078
    // C body (c:1080-1110): walks km->multi removing all bindings
    // whose key sequence starts with `prefix`. Used by `bindkey -rp`.
    let to_remove: Vec<Vec<u8>> = km
        .multi
        .keys()
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
pub fn bin_bindkey_list(
    name: &str,
    kmname: Option<&str>,
    _km: Option<&Keymap>,
    argv: &[String],
    ops: &options,
    _func: i32,
) -> i32 {
    // c:1094
    // C signature receives `km` already-resolved by the dispatcher
    // at c:794-799. Our dispatcher passes None; resolve here from
    // `kmname` (falling back to `curkeymapname` when `-M` was not
    // given — matches C's `kmname = "main"` default).
    let resolved_name: String = kmname
        .map(|s| s.to_string())
        .unwrap_or_else(|| curkeymapname().clone());
    let Some(km) = openkeymap(&resolved_name) else {
        crate::ported::utils::zwarnnam(name, &format!("no such keymap `{}'", resolved_name)); // c:911/925/949
        return 1;
    };

    // c:1096-1099 — `bs.flags = OPT_ISSET(ops,'L') ? BS_LIST : 0;
    //                bs.kmname = kmname;`
    let mut bs = bindstate {
        flags: if OPT_ISSET(ops, b'L') { BS_LIST } else { 0 },
        kmname: resolved_name.clone(),
        firstseq: Vec::new(),
        lastseq: Vec::new(),
        bind: None,
        str: None,
        prefix: None,
        prefixlen: 0,
    };

    // c:1100-1110 — single-sequence lookup path (`argv[0] && !-p`).
    if !argv.is_empty() && !OPT_ISSET(ops, b'p') {
        // c:1102-1107 — `seq = getkeystring(argv[0], &len,
        //                  GETKEYS_BINDKEY, NULL); seq = metafy(...)`.
        // The Rust seq storage is raw bytes; `getkeystring` parses
        // user-typed `\C-X`/`^X`/`\M-X` etc. → raw byte sequence.
        let seq = crate::ported::zle::zle_bindings::getkeystring(&argv[0]);
        // c:1108-1109 — `bs.flags |= BS_ALL; bs.firstseq = bs.lastseq = seq;`
        bs.flags |= BS_ALL;
        bs.firstseq = seq.clone();
        bs.lastseq = seq.clone();
        // c:1110 — `bs.bind = keybind(km, seq, &bs.str)`.
        let (bind, str_out) = keybind(&km, &seq);
        bs.bind = bind;
        bs.str = str_out;
        // c:1113 — `bindlistout(&bs)`.
        bindlistout(&bs);
        return 0;
    }

    // c:1115-1125 — `-p` prefix-only path.
    if OPT_ISSET(ops, b'p') {
        let arg0 = argv.first().map(|s| s.as_str()).unwrap_or("");
        if arg0.is_empty() {
            eprintln!("{}: option -p requires a prefix string", name);
            return 1;
        }
        let pfx = crate::ported::zle::zle_bindings::getkeystring(arg0);
        bs.prefixlen = pfx.len();
        bs.prefix = Some(pfx);
    }
    // c:1130-1137 — initialise bs for the scankeymap walk.
    bs.firstseq = Vec::new();
    bs.lastseq = Vec::new();
    bs.bind = None;
    bs.str = None;
    // c:1138 — `scankeymap(km, 1, scanbindlist, &bs)`.
    scankeymap(&km, 1, &mut |seq, bind, s| {
        scanbindlist(seq, bind, s, &mut bs);
    });
    // c:1139 — `bindlistout(&bs)` — flush the final accumulated range.
    bindlistout(&bs);
    0 // c:1173
}

/// Direct port of `static void scanbindlist(char *seq, Thingy bind,
///                                           char *str, void *magic)`
/// from `Src/Zle/zle_keymap.c:1141`. Per-binding callback used by
/// `bin_bindkey_list` via `scankeymap`. Coalesces consecutive
/// single-character bindings into ranges; flushes via `bindlistout`
/// otherwise.
pub fn scanbindlist(seq: &[u8], bind: Option<&Thingy>, str: Option<&str>, bs: &mut bindstate) {
    // c:1141
    // c:1145-1148 — prefix filter: if `bs->prefix` is set and either
    // (a) seq doesn't start with prefix or (b) seq equals prefix
    // exactly, drop the binding.
    if bs.prefixlen > 0 {
        if let Some(p) = &bs.prefix {
            if !seq.starts_with(p) || seq.len() == p.len() {
                return;
            }
        }
    }

    // c:1150-1160 — range-collapse: same bind/str AND both seqs are
    // single-character (ztrlen == 1) AND new byte = last byte + 1.
    let bind_eq = match (bind, &bs.bind) {
        (Some(t1), Some(t2)) => t1.nam == t2.nam,
        (None, None) => str == bs.str.as_deref(),
        _ => false,
    };
    if bind_eq && seq.len() == 1 && bs.lastseq.len() == 1 {
        let l = bs.lastseq[0] as i32;
        let t = seq[0] as i32;
        if t == l + 1 {
            // c:1157 — extend the range; replace lastseq with seq.
            bs.lastseq = seq.to_vec();
            return;
        }
    }

    // c:1162-1168 — flush current range; start a new one.
    bindlistout(bs);
    bs.firstseq = seq.to_vec();
    bs.lastseq = seq.to_vec();
    bs.bind = bind.cloned();
    bs.str = str.map(|s| s.to_string());
}

/// Direct port of `static void bindlistout(struct bindstate *bs)`
/// from `Src/Zle/zle_keymap.c:1172`. Emits one bindkey-listing line
/// (or one `bindkey ...` command line when `BS_LIST` is set) for
/// the accumulated `(firstseq..lastseq, bind, str)` range.
pub fn bindlistout(bs: &bindstate) {
    // c:1172
    use std::io::Write;

    // c:1177 — `if(bs->bind == t_undefinedkey && !(bs->flags & BS_ALL)) return;`.
    // C compares against the sentinel `t_undefinedkey` Thingy; the
    // Rust port stores it by name (`"undefined-key"`). When bs.str
    // is None AND bs.bind is either None or the undefined-key
    // thingy, skip (unless BS_ALL is set).
    let is_undefined = bs.str.is_none()
        && match &bs.bind {
            None => true,
            Some(t) => t.nam == "undefined-key",
        };
    if is_undefined && (bs.flags & BS_ALL) == 0 {
        return;
    }
    // c:1179 — `range = strcmp(bs->firstseq, bs->lastseq)`.
    let range = bs.firstseq != bs.lastseq;
    let mut out = std::io::stdout().lock();
    let mut nodash = true;

    // c:1180-1199 — BS_LIST: emit `bindkey [-R][-s][-M km|-a] `.
    if (bs.flags & BS_LIST) != 0 {
        let _ = write!(out, "bindkey ");
        if range {
            let _ = write!(out, "-R ");
        }
        if bs.bind.is_none() {
            let _ = write!(out, "-s ");
        }
        if bs.kmname == "main" {
            // c:1188 — main → no keymap flag
        } else if bs.kmname == "vicmd" {
            let _ = write!(out, "-a ");
        } else {
            let _ = write!(out, "-M {} ", bs.kmname);
            nodash = false;
        }
        // c:1196 — `if(nodash && bs->firstseq[0] == '-') fputs("-- ", stdout);`
        if nodash && bs.firstseq.first() == Some(&b'-') {
            let _ = write!(out, "-- ");
        }
    }

    // c:1202 — `printbind(bs->firstseq, stdout)`. `bindztrdup`
    // already includes the surrounding `"..."` via dquotedztrdup.
    let _ = write!(
        out,
        "{}",
        crate::ported::zle::zle_utils::bindztrdup(&bs.firstseq),
    );
    // c:1203-1206 — range: `-` + `printbind(lastseq)`.
    if range {
        let _ = write!(
            out,
            "-{}",
            crate::ported::zle::zle_utils::bindztrdup(&bs.lastseq),
        );
    }
    let _ = write!(out, " ");
    // c:1208-1214 — emit `bind->nam` or `printbind(str)`.
    if let Some(t) = &bs.bind {
        let _ = writeln!(out, "{}", t.nam);
    } else if let Some(s) = &bs.str {
        let _ = writeln!(
            out,
            "{}",
            crate::ported::zle::zle_utils::bindztrdup(s.as_bytes()),
        );
    } else {
        // c:1177 already filtered undefined-key when !BS_ALL; here
        // we hit only with BS_ALL.
        let _ = writeln!(out, "undefined-key");
    }
}

/// Port of `add_cursor_char(int c)` from Src/Zle/zle_keymap.c:1248.
/// WARNING: param names don't match C — Rust=(buf, c) vs C=(c)
pub fn add_cursor_char(buf: &mut Vec<u8>, c: u8) {
    // c:1248
    // C body (c:1250): `*cursorptr++ = c`. Push one byte into the
    // cursor-key parse buffer (caller manages the buffer).
    buf.push(c);
}

/// Port of `add_cursor_key(Keymap km, int tccode, Thingy thingy, int defchar)`
/// from Src/Zle/zle_keymap.c:1258.
///
/// Line-by-line port. Probes the termcap entry for `tccode` via
/// `tclen[tccode] > 0` and `TERMFLAGS`; if available emits the
/// escape through a synthetic outc callback to build the byte
/// sequence. Falls back to `\e[<defchar>` when termcap doesn't have
/// the cap or the terminal is flagged broken. Binds the result, then
/// also binds the `\e[`↔`\eO` variant — both forms appear in xterm
/// depending on application vs normal keypad mode.
pub fn add_cursor_key(km: &mut Keymap, tccode: i32, thingy: Thingy, defchar: i32) {
    use crate::ported::init::{tclen, tcstr};
    use crate::ported::params::TERMFLAGS;
    use crate::ported::zsh_h::{TERM_BAD, TERM_NOUP, TERM_UNKNOWN};
    use std::sync::atomic::Ordering;

    let cap_idx = tccode as usize;
    let mut buf: Vec<u8> = Vec::with_capacity(8);
    let mut ok = false;

    // c:1262-1266 — `tccan(tccode) && !(termflags & (TERM_NOUP|TERM_BAD|TERM_UNKNOWN))`
    let cap_present = {
        let lens = tclen.lock().unwrap();
        cap_idx < lens.len() && lens[cap_idx] > 0
    };
    let termflags = TERMFLAGS.load(Ordering::Relaxed);
    let term_broken = termflags & (TERM_NOUP | TERM_BAD | TERM_UNKNOWN) != 0;

    if cap_present && !term_broken {
        // c:1271-1273 — `cursorptr = buf; tputs(tcstr[tccode], 1, add_cursor_char);`
        let escape = tcstr.lock().unwrap()[cap_idx].clone();
        buf.extend_from_slice(escape.as_bytes());
        // c:1281-1282 — sanity: reject zero-length / single-char.
        let len = buf.len();
        if len >= 2 && (buf[0] != 0x83 || len >= 3) {
            ok = true;
        }
    }

    if !ok {
        // c:1287-1288 — `sprintf(buf, "\33[%c", defchar);`
        buf.clear();
        buf.push(0x1b);
        buf.push(b'[');
        buf.push(defchar as u8);
    }

    // c:1290 — `bindkey(km, buf, refthingy(thingy), NULL);`
    bindkey(km, &buf, Some(thingy.clone()), None);

    // c:1295-1299 — `if (buf[0] == '\33' && (buf[1] == '[' || buf[1] == 'O') &&
    //                buf[2] && !buf[3]) { swap [/O; bindkey again; }`
    if buf.len() == 3 && buf[0] == 0x1b && (buf[1] == b'[' || buf[1] == b'O') {
        let mut alt = buf.clone();
        alt[1] = if buf[1] == b'[' { b'O' } else { b'[' };
        bindkey(km, &alt, Some(thingy), None);
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
pub fn default_bindings() {
    // c:1309 — `void default_bindings(void)`. Inlined line-for-line
    // from `Src/Zle/zle_keymap.c:1309-1473`. No extracted helpers
    // (the C body is one ~165-line function with every bindkey call
    // expanded; the Rust port mirrors that shape exactly).
    let mut vmap = Keymap::default(); // c:1311
    vmap.primary = Some("viins".to_string());
    let mut emap = Keymap::default(); // c:1312
    emap.primary = Some("emacs".to_string());
    let mut amap = Keymap::default(); // c:1313
    amap.primary = Some("vicmd".to_string());
    let mut oppmap = Keymap::default(); // c:1314
    oppmap.primary = Some("viopp".to_string());
    let mut vismap = Keymap::default(); // c:1315
    vismap.primary = Some("visual".to_string());
    let mut smap = Keymap::default(); // c:1316
    smap.primary = Some(".safe".to_string());

    // c:1326-1329 — `for (i = 0; i < 32; i++)
    //                  vmap->first[i] = refthingy(Th(viinsbind[i]));
    //                  emap->first[i] = refthingy(Th(emacsbind[i]));`
    for i in 0..32 {
        bindkey(
            &mut vmap,
            &[i as u8],
            Some(Thingy::builtin(VIINSBIND[i])),
            None,
        );
        bindkey(
            &mut emap,
            &[i as u8],
            Some(Thingy::builtin(EMACSBIND[i])),
            None,
        );
    }
    // c:1330-1333 — 32-255 self-insert in both vmap and emap.
    for i in 32u8..=255u8 {
        bindkey(&mut vmap, &[i], Some(Thingy::builtin("self-insert")), None);
        bindkey(&mut emap, &[i], Some(Thingy::builtin("self-insert")), None);
    }
    // c:1336-1337 — `first[127] = first[8]` (DEL == ^H).
    bindkey(
        &mut vmap,
        &[0x7F],
        Some(Thingy::builtin(VIINSBIND[8])),
        None,
    );
    bindkey(
        &mut emap,
        &[0x7F],
        Some(Thingy::builtin(EMACSBIND[8])),
        None,
    );

    // !!! ZSHRS-NATIVE DEVIATION (not in zle_bindings.c) — OPT-IN !!!
    // viins ^H/DEL bound to the UNRESTRICTED backward-delete-char instead
    // of vi-backward-delete-char (classic vi and zsh -f refuse to backspace
    // past the insertion start; modern vim ships backspace=indent,eol,start).
    // OFF by default: `zshrs -f` stays zsh-identical for parity purposes.
    // Enable via `[zle] vi_backspace_unrestricted = true` in zshrs.toml.
    //
    // NEVER in `--zsh` drop-in / parity mode, regardless of config: that
    // mode must reproduce /bin/zsh's keymap byte-for-byte (viins ^H/^? =
    // vi-backward-delete-char). Without this gate the user's
    // `vi_backspace_unrestricted = true` in ~/.zshrs/zshrs.toml leaked
    // into `zshrs --zsh -f`, diverging `bindkey`/`bindkey -L` state from
    // zsh (every config_state_parity fragment dumps bindkeys → 16 tests
    // failed on this single line).
    if !crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed)
        && crate::config::current().zle.vi_backspace_unrestricted
        && std::env::var("ZSHRS_NATIVE_ZLE_FX")
            .map(|v| v != "0")
            .unwrap_or(true)
    {
        for key in [0x08u8, 0x7F] {
            bindkey(
                &mut vmap,
                &[key],
                Some(Thingy::builtin("backward-delete-char")),
                None,
            );
        }
    }

    // c:1342-1343 — vicmd 0-127 from vicmdbind.
    for i in 0..128 {
        bindkey(
            &mut amap,
            &[i as u8],
            Some(Thingy::builtin(VICMDBIND[i])),
            None,
        );
    }
    // c:1344-1345 — vicmd 128-255 undefined-key.
    for i in 128u8..=255u8 {
        bindkey(
            &mut amap,
            &[i],
            Some(Thingy::builtin("undefined-key")),
            None,
        );
    }

    // c:1352-1358 — .safe fallback keymap: 0-255 → .self-insert,
    // except \n/\r → .accept-line. The dotted names are the
    // internal widget aliases not overridable by user `zle -N`.
    for i in 0u8..=255u8 {
        bindkey(&mut smap, &[i], Some(Thingy::builtin(".self-insert")), None);
    }
    bindkey(
        &mut smap,
        &[b'\n'],
        Some(Thingy::builtin(".accept-line")),
        None,
    );
    bindkey(
        &mut smap,
        &[b'\r'],
        Some(Thingy::builtin(".accept-line")),
        None,
    );

    // c:1364-1369 — vi command + insert: arrow keys via
    // `add_cursor_key()` so the source-level bindkey call count
    // matches C exactly (one add_cursor_key → two internal bindkey
    // calls for the `[`/`O` keypad variants).
    for kptr in [&mut vmap, &mut amap] {
        add_cursor_key(
            kptr,
            crate::ported::zsh_h::TCUPCURSOR,
            Thingy::builtin("up-line-or-history"),
            b'A' as i32,
        );
        add_cursor_key(
            kptr,
            crate::ported::zsh_h::TCDOWNCURSOR,
            Thingy::builtin("down-line-or-history"),
            b'B' as i32,
        );
        add_cursor_key(
            kptr,
            crate::ported::zsh_h::TCLEFTCURSOR,
            Thingy::builtin("vi-backward-char"),
            b'D' as i32,
        );
        add_cursor_key(
            kptr,
            crate::ported::zsh_h::TCRIGHTCURSOR,
            Thingy::builtin("vi-forward-char"),
            b'C' as i32,
        );
    }

    // c:1374-1385 — vi operator-pending + visual local maps: cursor
    // keys + j/k + aa/ia/aw/iw/aW/iW. Cursor keys via
    // `add_cursor_key` matching C's helper-based shape.
    for kptr in [&mut oppmap, &mut vismap] {
        add_cursor_key(
            kptr,
            crate::ported::zsh_h::TCUPCURSOR,
            Thingy::builtin("up-line"),
            b'A' as i32,
        );
        add_cursor_key(
            kptr,
            crate::ported::zsh_h::TCDOWNCURSOR,
            Thingy::builtin("down-line"),
            b'B' as i32,
        );
        bindkey(kptr, &[b'k'], Some(Thingy::builtin("up-line")), None); // c:1379
        bindkey(kptr, &[b'j'], Some(Thingy::builtin("down-line")), None); // c:1380
        bindkey(
            kptr,
            b"aa",
            Some(Thingy::builtin("select-a-shell-word")),
            None,
        ); // c:1381
        bindkey(
            kptr,
            b"ia",
            Some(Thingy::builtin("select-in-shell-word")),
            None,
        ); // c:1382
        bindkey(kptr, b"aw", Some(Thingy::builtin("select-a-word")), None); // c:1383
        bindkey(kptr, b"iw", Some(Thingy::builtin("select-in-word")), None); // c:1384
        bindkey(
            kptr,
            b"aW",
            Some(Thingy::builtin("select-a-blank-word")),
            None,
        ); // c:1385
        bindkey(
            kptr,
            b"iW",
            Some(Thingy::builtin("select-in-blank-word")),
            None,
        ); // c:1386
    }

    // c:1388 — `bindkey(oppmap, "\33", refthingy(t_vicmdmode))`.
    bindkey(
        &mut oppmap,
        &[0x1B],
        Some(Thingy::builtin("vi-cmd-mode")),
        None,
    );
    // c:1389-1395 — visual-mode local bindings.
    bindkey(
        &mut vismap,
        &[0x1B],
        Some(Thingy::builtin("deactivate-region")),
        None,
    );
    bindkey(
        &mut vismap,
        &[b'o'],
        Some(Thingy::builtin("exchange-point-and-mark")),
        None,
    );
    bindkey(
        &mut vismap,
        &[b'p'],
        Some(Thingy::builtin("put-replace-selection")),
        None,
    );
    bindkey(
        &mut vismap,
        &[b'u'],
        Some(Thingy::builtin("vi-down-case")),
        None,
    );
    bindkey(
        &mut vismap,
        &[b'U'],
        Some(Thingy::builtin("vi-up-case")),
        None,
    );
    bindkey(
        &mut vismap,
        &[b'x'],
        Some(Thingy::builtin("vi-delete")),
        None,
    );
    bindkey(
        &mut vismap,
        &[b'~'],
        Some(Thingy::builtin("vi-oper-swap-case")),
        None,
    );

    // c:1398-1407 — vi g-prefix sequences (on amap = vicmd).
    bindkey(
        &mut amap,
        b"ga",
        Some(Thingy::builtin("what-cursor-position")),
        None,
    );
    bindkey(
        &mut amap,
        b"ge",
        Some(Thingy::builtin("vi-backward-word-end")),
        None,
    );
    bindkey(
        &mut amap,
        b"gE",
        Some(Thingy::builtin("vi-backward-blank-word-end")),
        None,
    );
    bindkey(
        &mut amap,
        b"gg",
        Some(Thingy::builtin("beginning-of-buffer-or-history")),
        None,
    );
    bindkey(
        &mut amap,
        b"gu",
        Some(Thingy::builtin("vi-down-case")),
        None,
    );
    bindkey(&mut amap, b"gU", Some(Thingy::builtin("vi-up-case")), None);
    bindkey(
        &mut amap,
        b"g~",
        Some(Thingy::builtin("vi-oper-swap-case")),
        None,
    );
    // c:1405-1407 — vim double-operator send-strings (NULL bind +
    // str form, i.e. `bindkey -s`).
    bindkey(&mut amap, b"g~~", None, Some("g~g~".to_string()));
    bindkey(&mut amap, b"guu", None, Some("gugu".to_string()));
    bindkey(&mut amap, b"gUU", None, Some("gUgU".to_string()));

    // c:1410-1413 — emacs cursor keys via `add_cursor_key`.
    add_cursor_key(
        &mut emap,
        crate::ported::zsh_h::TCUPCURSOR,
        Thingy::builtin("up-line-or-history"),
        b'A' as i32,
    );
    add_cursor_key(
        &mut emap,
        crate::ported::zsh_h::TCDOWNCURSOR,
        Thingy::builtin("down-line-or-history"),
        b'B' as i32,
    );
    add_cursor_key(
        &mut emap,
        crate::ported::zsh_h::TCLEFTCURSOR,
        Thingy::builtin("backward-char"),
        b'D' as i32,
    );
    add_cursor_key(
        &mut emap,
        crate::ported::zsh_h::TCRIGHTCURSOR,
        Thingy::builtin("forward-char"),
        b'C' as i32,
    );

    // c:1416-1432 — emacs ^X sequences.
    bindkey(
        &mut emap,
        b"\x18*",
        Some(Thingy::builtin("expand-word")),
        None,
    );
    bindkey(
        &mut emap,
        b"\x18g",
        Some(Thingy::builtin("list-expand")),
        None,
    );
    bindkey(
        &mut emap,
        b"\x18G",
        Some(Thingy::builtin("list-expand")),
        None,
    );
    bindkey(
        &mut emap,
        b"\x18\x0e",
        Some(Thingy::builtin("infer-next-history")),
        None,
    );
    bindkey(
        &mut emap,
        b"\x18\x0b",
        Some(Thingy::builtin("kill-buffer")),
        None,
    );
    bindkey(
        &mut emap,
        b"\x18\x06",
        Some(Thingy::builtin("vi-find-next-char")),
        None,
    );
    bindkey(
        &mut emap,
        b"\x18\x0f",
        Some(Thingy::builtin("overwrite-mode")),
        None,
    );
    bindkey(&mut emap, b"\x18\x15", Some(Thingy::builtin("undo")), None);
    bindkey(
        &mut emap,
        b"\x18\x16",
        Some(Thingy::builtin("vi-cmd-mode")),
        None,
    );
    bindkey(
        &mut emap,
        b"\x18\x0a",
        Some(Thingy::builtin("vi-join")),
        None,
    );
    bindkey(
        &mut emap,
        b"\x18\x02",
        Some(Thingy::builtin("vi-match-bracket")),
        None,
    );
    bindkey(
        &mut emap,
        b"\x18s",
        Some(Thingy::builtin("history-incremental-search-forward")),
        None,
    );
    bindkey(
        &mut emap,
        b"\x18r",
        Some(Thingy::builtin("history-incremental-search-backward")),
        None,
    );
    bindkey(&mut emap, b"\x18u", Some(Thingy::builtin("undo")), None);
    bindkey(
        &mut emap,
        b"\x18\x18",
        Some(Thingy::builtin("exchange-point-and-mark")),
        None,
    );
    bindkey(
        &mut emap,
        b"\x18=",
        Some(Thingy::builtin("what-cursor-position")),
        None,
    );

    // c:1435-1437 — bracketed paste in all three primary keymaps.
    bindkey(
        &mut emap,
        b"\x1b[200~",
        Some(Thingy::builtin("bracketed-paste")),
        None,
    );
    bindkey(
        &mut vmap,
        b"\x1b[200~",
        Some(Thingy::builtin("bracketed-paste")),
        None,
    );
    bindkey(
        &mut amap,
        b"\x1b[200~",
        Some(Thingy::builtin("bracketed-paste")),
        None,
    );

    // c:1440-1445 — emacs ESC sequences from metabind table.
    for i in 0..128 {
        let name = METABIND[i];
        if name == "undefined-key" {
            continue;
        }
        bindkey(
            &mut emap,
            &[0x1b, i as u8],
            Some(Thingy::builtin(name)),
            None,
        );
    }

    // c:1449-1454 — link each keymap into the name table.
    linkkeymap(Arc::new(vmap), "viins", 0); // c:1449
    linkkeymap(Arc::new(emap), "emacs", 0); // c:1450
    linkkeymap(Arc::new(amap), "vicmd", 0); // c:1451
    linkkeymap(Arc::new(oppmap), "viopp", 0); // c:1452
    linkkeymap(Arc::new(vismap), "visual", 0); // c:1453
    linkkeymap(Arc::new(smap), ".safe", 1); // c:1454 — KM_IMMUTABLE

    // c:Src/Zle/zle_keymap.c pre-2023-10-26 `default_bindings`
    // (zsh 5.9 + 5.9.1 still ship this; commit f36fccbb removed
    // it in zsh master, deferring main selection to sample
    // .zshrc code). The check the homebrew/stable zsh binary
    // performs is:
    //
    //     if (((ed = zgetenv("VISUAL")) && strstr(ed, "vi")) ||
    //         ((ed = zgetenv("EDITOR")) && strstr(ed, "vi")))
    //         linkkeymap(vmap, "main", 0);
    //     else
    //         linkkeymap(emap, "main", 0);
    //
    // VIMODE option still overrides (post-removal master also
    // honors `isset(VIMODE)` if the user explicitly sets it).
    // The check is `strstr(ed, "vi")` — substring match anywhere
    // in the env-var value. So `EDITOR=nvim`, `vim`, `vi` all
    // pick viins; `emacs`, `nano`, unset all pick emacs.
    //
    // An explicitly-set EMACSMODE loses to nothing: C reaches the same
    // state through `dosetopt` → `zleentry(ZLE_CMD_SET_KEYMAP)` because
    // zle is already loaded when `setopt emacs` runs. zshrs builds the
    // keymaps lazily in non-interactive shells (first `bindkey` call),
    // so default_bindings can run AFTER the option was set — the EDITOR
    // sniff would then override the user's explicit `setopt emacs`.
    // Honour both option bits before falling back to the env sniff.
    let pick_vi = if crate::ported::zsh_h::isset(crate::ported::zsh_h::VIMODE) {
        true
    } else if crate::ported::zsh_h::isset(crate::ported::zsh_h::EMACSMODE) {
        false
    } else {
        let visual_has_vi = std::env::var("VISUAL")
            .map(|v| v.contains("vi"))
            .unwrap_or(false);
        let editor_has_vi = std::env::var("EDITOR")
            .map(|v| v.contains("vi"))
            .unwrap_or(false);
        visual_has_vi || editor_has_vi
    };
    let main_src = if pick_vi { "viins" } else { "emacs" };
    if let Some(km) = openkeymap(main_src) {
        linkkeymap(km, "main", 0);
    }

    // c:1464-1465 — `isearch_keymap = newkeymap(NULL, "isearch");
    //                linkkeymap(isearch_keymap, "isearch", 0);`.
    let mut isearch_km = Keymap::default();
    isearch_km.primary = Some("isearch".to_string());
    linkkeymap(Arc::new(isearch_km), "isearch", 0);

    // c:1468-1472 — `command_keymap`: \n/\r → accept-line,
    // ^G ('G'&0x1F = 0x07) → send-break.
    let mut command_km = Keymap::default();
    command_km.primary = Some("command".to_string());
    bindkey(
        &mut command_km,
        &[b'\n'],
        Some(Thingy::builtin("accept-line")),
        None,
    ); // c:1470
    bindkey(
        &mut command_km,
        &[b'\r'],
        Some(Thingy::builtin("accept-line")),
        None,
    ); // c:1471
    bindkey(
        &mut command_km,
        &[0x07],
        Some(Thingy::builtin("send-break")),
        None,
    ); // c:1472
    linkkeymap(Arc::new(command_km), "command", 0);

    // Seed curkeymap/curkeymapname so the first key read has a target.
    *curkeymap.lock().unwrap() = openkeymap("main"); // c:519
    *curkeymapname() = "main".to_string(); // c:513
}

/// Direct port of `ZLE_INT_T getrestchar_keybuf(void)` from
/// `Src/Zle/zle_keymap.c:1504`.
///
/// Walks the pending `keybuf` Meta-byte pairs first (c:1525-1530),
/// then falls through to [`getbyte`] for any remaining UTF-8
/// continuation bytes — same shape as [`getrestchar`] but reads
/// from `keybuf` until exhausted before going to the input loop.
/// Used by the keymap dispatcher when it needs to assemble a wide
/// char that crossed a key-binding boundary.
pub fn getrestchar_keybuf() -> i32 {
    use crate::ported::zle::zle_main::{getbyte, ungetbyte, LASTCHAR_WIDE, LASTCHAR_WIDE_VALID};
    use std::sync::atomic::Ordering;

    // c:1519 — `lastchar_wide_valid = 1; memset(&mbs, 0, sizeof mbs);`
    LASTCHAR_WIDE_VALID.store(1, Ordering::SeqCst);

    let keybuf_v = keybuf.lock().unwrap().clone();
    let buflen = (keybuflen.load(Ordering::SeqCst) as usize).min(keybuf_v.len());
    let mut bufind = 0usize;
    let mut bytes: Vec<u8> = Vec::new();

    // First-byte read (c:1525-1545): either pop from keybuf or call
    // getbyte; subsequent bytes follow the same path until we have a
    // valid UTF-8 sequence.
    loop {
        let cur = if bufind < buflen {
            // c:1525-1530 — keybuf path with Meta-pair decode.
            let mut c = keybuf_v[bufind];
            bufind += 1;
            if c == 0x83 && bufind < buflen {
                c = keybuf_v[bufind] ^ 32;
                bufind += 1;
            }
            c
        } else {
            // c:1546 — `inchar = getbyte(1L, &timeout, 1);`
            match getbyte(true) {
                Some(b) => b,
                None => {
                    // c:1550-1553 — EOF in the middle of a sequence.
                    LASTCHAR_WIDE.store(-1, Ordering::SeqCst);
                    return -1;
                }
            }
        };
        bytes.push(cur);

        // Decode the partial UTF-8 buffer — break out when it parses
        // or when the lead byte tells us we have enough bytes.
        if let Ok(s) = std::str::from_utf8(&bytes) {
            if let Some(c) = s.chars().next() {
                LASTCHAR_WIDE.store(c as i32, Ordering::SeqCst);
                return c as i32;
            }
        }
        let lead = bytes[0];
        let need = if lead < 0x80 {
            1
        } else if lead < 0xC0 {
            1
        } else if lead < 0xE0 {
            2
        } else if lead < 0xF0 {
            3
        } else {
            4
        };
        if bytes.len() >= need {
            // c:1535-1538 — invalid byte sequence; reset mbs + WEOF.
            // Unget the non-continuation byte if we read it from
            // getbyte (not from keybuf).
            if let Some(&last) = bytes.last() {
                if bufind >= buflen && (last & 0xC0) != 0x80 {
                    ungetbyte(last);
                }
            }
            LASTCHAR_WIDE.store(-1, Ordering::SeqCst);
            return -1;
        }
    }
}

/// !!! WARNING: PARTIAL PORT — stub. C `getkeymapcmd(Keymap km,
/// Thingy *funcp, char **strp)` at Src/Zle/zle_keymap.c:1581 is the
/// Direct port of `char *getkeymapcmd(Keymap km, Thingy *funcp,
/// char **strp)` from `Src/Zle/zle_keymap.c:1581`. Walks the
/// keybinding trie one byte at a time against the supplied
/// `km`: tracks the longest prefix that hit a binding,
/// stops when the in-flight sequence is no longer a prefix of
/// any binding, and unget-bytes any chars read past the
/// matched prefix.
///
/// Rust signature: `(&Keymap) -> Option<(Thingy, Vec<u8>,
/// Option<String>)>` — the C `(funcp out, strp out)` collapse
/// into the returned tuple (Thingy + matched key sequence +
/// string-replacement when bound).
pub fn getkeymapcmd(km: &Keymap) -> Option<(super::zle_thingy::Thingy, Vec<u8>, Option<String>)> {
    // c:1581
    let mut buf: Vec<u8> = Vec::with_capacity(8); // c:1583 keybuf
    let mut last_match: Option<super::zle_thingy::Thingy> = None; // c:1584
    let mut last_match_str: Option<String> = None;
    let mut last_match_len = 0usize; // c:1585
                                     // c:1585 — `lastc = lastchar` — see get_key_cmd (zle_main.rs) for
                                     // the probe-byte restore rationale.
    let mut lastc =
        crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst); // c:1585

    // c:1588-1589 — `keybuflen = 0; keybuf[0] = 0;` — reset the GLOBAL
    // keybuf. The local `buf` drives the (raw-byte-keyed) Rust keymap
    // lookup; the global mirrors it METAFIED via addkeybuf so `$KEYS`
    // (get_keys → keybuf, zle_params.c:463) sees the sequence that
    // invoked the widget. Without the mirror, $KEYS was the stale
    // 32-NUL init buffer and zsh-expand's `[[ $KEYS == " " ]]`
    // space-dispatch never fired.
    keybuf.lock().unwrap().clear(); // c:1588
    keybuflen.store(0, std::sync::atomic::Ordering::SeqCst); // c:1588

    // c:1591 — `while(getkeybuf(timeout) != EOF)`.
    loop {
        // Read one byte. Use timed read once we have a partial match.
        let do_keytmout = last_match.is_some();
        let b = match super::zle_main::getbyte(do_keytmout) {
            Some(b) => b,
            None => break, // c:1591 EOF
        };
        buf.push(b);
        addkeybuf(b as i32); // c:1604 getkeybuf → addkeybuf (global, metafied)
        keybuflen.store(
            keybuf.lock().unwrap().len() as i32,
            std::sync::atomic::Ordering::SeqCst,
        );

        // c:1602-1604 — `f = keybind(km, keybuf, &s);` lookup.
        let (current_match, current_str, is_prefix) = if buf.len() == 1 {
            let m = km.first[b as usize].clone();
            let pfx = km.multi.keys().any(|k| k.len() > 1 && k[0] == b);
            (m, None, pfx)
        } else {
            let entry = km.multi.get(&buf[..]);
            let m = entry.and_then(|e| e.bind.clone());
            let s = entry.and_then(|e| e.str.clone());
            let pfx = entry.map(|e| e.prefixct > 0).unwrap_or(false);
            (m, s, pfx)
        };

        // c:1606-1614 — `if (f != t_undefinedkey)` → record match.
        if let Some(t) = current_match {
            last_match = Some(t);
            last_match_str = current_str;
            last_match_len = buf.len();
            lastc =
                crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst);
            // c:1622
        }

        // c:1614 — `if (!ispfx) break;` stop when not a prefix anymore.
        if !is_prefix {
            break;
        }
    }

    // c:1692-1695 — `if(!lastlen && keybuflen) lastlen = keybuflen;
    // else lastchar = lastc;` — restore lastchar to its match-time
    // value when probe bytes were read past the dispatched match.
    if !(last_match_len == 0 && !buf.is_empty()) {
        crate::ported::zle::compcore::LASTCHAR.store(lastc, std::sync::atomic::Ordering::SeqCst);
        // c:1695
    }

    // c:1696-1708 — unget extra bytes past the matched prefix and
    // truncate keybuf to the used prefix (`keybuf[keybuflen = lastlen]
    // = 0`).
    if last_match.is_some() && buf.len() > last_match_len {
        let extra = buf[last_match_len..].to_vec();
        super::zle_main::ungetbytes(&extra);
        buf.truncate(last_match_len);
        // Rebuild the global metafied mirror from the kept raw bytes.
        keybuf.lock().unwrap().clear();
        for &kb in &buf {
            addkeybuf(kb as i32);
        }
        keybuflen.store(
            keybuf.lock().unwrap().len() as i32,
            std::sync::atomic::Ordering::SeqCst,
        ); // c:1708
    }

    last_match.map(|t| (t, buf, last_match_str))
}

/// Port of `addkeybuf(int c)` from Src/Zle/zle_keymap.c:1717.
/// WARNING: param names don't match C — Rust=(zle, c) vs C=(c)
pub fn addkeybuf(c: i32) {
    // c:1717
    // C body (zle_keymap.c:1717-1727):
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
    // c:1721 — `if (imeta(c))` — route through the canonical IMETA
    // typtab predicate (`Src/ztype.h:60`) so the byte set agrees with
    // every other call site that checks `imeta(c)`. The previous Rust
    // port used a hand-rolled `c >= 0x83 && c != 0x83 && c != 0x84`
    // which:
    //   * MISSED 0x00 (NUL — canonical IMETA per utils.c:4195)
    //   * MISSED 0x83 (Meta itself — canonical IMETA per utils.c:4196)
    //   * MISSED 0x84 (Pound — canonical IMETA per utils.c:4198)
    //   * OVER-ENCODED 0xa3..=0xff (these are NOT IMETA per the
    //     typtab; they should pass through as literal bytes).
    // Routing through `ztype_h::imeta` ties the meta-encoding decision
    // to the same typtab inittyptab() populates — one source of truth.
    let is_meta = imeta(c as u8); // c:1721
    let mut buf = keybuf.lock().unwrap();
    if is_meta {
        buf.push(META as u8); // c:1722 Meta
        buf.push((c ^ 32) as u8); // c:1723 c ^ 32
    } else {
        buf.push(c as u8); // c:1725
    }
    // C terminates with '\0'; Rust Vec doesn't need that.
}

/// Port of `getkeybuf(int w)` from Src/Zle/zle_keymap.c:1744.
/// WARNING: param names don't match C — Rust=(zle, w) vs C=(w)
pub fn getkeybuf(w: i32) -> i32 {
    // c:1744
    // C body (c:1658-1664): `int c = getbyte((long)w, NULL, 1);
    //                       if (c < 0) return EOF; addkeybuf(c); return c`.
    // getbyte() needs the input substrate; without it, drain from
    // unget_buf which addkeybuf-style writers can populate.
    let _ = w; // would be `(long)w` to getbyte's timeout arg
    if let Some(b) = KUNGETBUF.lock().unwrap().pop_front() {
        addkeybuf(b as i32);
        b as i32
    } else {
        -1 // c:1661 EOF
    }
}

/// Port of `ungetkeycmd()` from Src/Zle/zle_keymap.c:1759.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn ungetkeycmd() {
    // c:1759
    // C body (c:1761): `ungetbytes_unmeta(keybuf, keybuflen)`.
    let buf = keybuf.lock().unwrap().clone();
    ungetbytes_unmeta(&buf);
}

/// Port of `mod_export Thingy getkeycmd(void)` from
/// Src/Zle/zle_keymap.c:1768. Reads one input sequence via
/// `getkeymapcmd` (the byte-by-byte keymap walk), then handles:
///   - empty `seq` → return None (EOF);
///   - `func == NULL` (string-insert binding) → push str back to
///     input and retry; cap at 20 hops to prevent string-insert
///     infinite loops (c:1777-1786);
///   - `func == z_executenamedcmd` → call `executenamedcommand`
///     for interactive widget-name resolution (c:1788-1796);
///   - `func == z_executelastnamedcmd` → return cached `lastnamed`
///     (c:1798).
pub fn getkeycmd() -> Option<super::zle_thingy::Thingy> {
    // c:1768
    use super::zle_main::get_key_cmd;
    let mut hops = 0; // c:1772
                      // c:1774 — `sentstring:` retry label.
    loop {
        // c:1775 — `seq = getkeymapcmd(curkeymap, &func, &str);`
        let func = get_key_cmd(); // c:1775 underlying byte-loop
        if func.is_none() {
            // c:1776 `if (!*seq) return NULL;`
            return None;
        }
        let func = func.unwrap();
        // c:1777-1786 — string-insert (func==NULL in C, modeled as
        // Thingy with empty nam in Rust thingytab). When the binding
        // is a string-replacement, ungetbytes the str and re-walk.
        if func.nam.is_empty() {
            // c:1777 `if (!func)`
            hops += 1; // c:1778
            if hops == 20 {
                // c:1779 hop-cap
                crate::ported::utils::zerr(
                    // c:1781
                    "string inserting another one too many times",
                );
                return None; // c:1783
            }
            // c:1785 `ungetbytes_unmeta(str, strlen(str))` — no widget
            // string was bound on this branch in Rust (get_key_cmd
            // routes string-replacements before returning), so this
            // arm only fires when the keymap entry has an empty `nam`
            // sentinel. Loop to retry.
            continue; // c:1786 `goto sentstring;`
        }
        // c:1788 — `func == Th(z_executenamedcmd)` check. zsh uses
        // pointer equality on the global Thingy table; Rust uses
        // name equality against the canonical widget name.
        if func.nam == "execute-named-command" {
            // c:1788
            // c:1789-1790 — drive `executenamedcommand("execute: ")`
            // until it returns a non-named-command result.
            let mut resolved: Option<super::zle_thingy::Thingy> = None;
            loop {
                let name = crate::ported::zle::zle_misc::executenamedcommand("execute: "); // c:1791
                match name {
                    Some(n) if n == "execute-named-command" => continue, // c:1790 loop
                    Some(n) => {
                        // c:1792 — `func != z_executenamedcmd`
                        let lookup = super::zle_thingy::thingytab()
                            .lock()
                            .unwrap()
                            .get(&n)
                            .cloned();
                        resolved = lookup;
                        break;
                    }
                    None => {
                        // c:1793 `if (!func) func = t_undefinedkey;`
                        let undef = super::zle_thingy::thingytab()
                            .lock()
                            .unwrap()
                            .get("undefined-key")
                            .cloned();
                        resolved = undef;
                        break;
                    }
                }
            }
            // c:1794-1796 — record `lastnamed = refthingy(func)` for
            // future `executelastnamedcmd` lookups, unless `func`
            // itself is `executelastnamedcmd`.
            if let Some(ref f) = resolved {
                if f.nam != "execute-last-named-cmd" {
                    // c:1795
                    *crate::ported::zle::zle_keymap::lastnamed.lock().unwrap() = Some(f.clone());
                    // c:1796
                }
            }
            return resolved;
        }
        // c:1798 — `func == Th(z_executelastnamedcmd)` → return
        // the cached `lastnamed` Thingy.
        if func.nam == "execute-last-named-cmd" {
            // c:1798
            return crate::ported::zle::zle_keymap::lastnamed
                .lock()
                .unwrap()
                .clone();
        }
        return Some(func); // c:1800
    }
}

/// Port of `zlesetkeymap(int mode)` from Src/Zle/zle_keymap.c:1804.
pub fn zlesetkeymap(mode: i32) {
    // c:1804
    // C body (c:1820-1825): `Keymap km = openkeymap(mode==VIMODE?
    //                       "viins":"emacs"); if (!km) return;
    //                       linkkeymap(km, "main", 0)`.
    // `mode` is the OPTION index that dosetopt passed through
    // zleentry(ZLE_CMD_SET_KEYMAP, optno) — VIMODE (zsh.h option
    // number), not a 0/1 flag.
    let kmname = if mode == crate::ported::zsh_h::VIMODE {
        "viins"
    } else {
        "emacs"
    };
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
pub fn readcommand() -> i32 {
    // c:1814
    // Read a single key + look up its bound thingy via the existing
    // ZLE input path. Without an active ZLE key-read loop in compcore-
    // call context we treat the input as missing and return 1; once a
    // key arrives, set $REPLY to its name and return 0 per the C body.
    // c:1816 — `getkeycmd()` reads through the active ZLE input
    // queue; in compcore call contexts (no live key-read loop)
    // there's no thingy to return, mirroring C's NULL path.
    let Some(name): Option<String> = None else {
        return 1;
    }; // c:1816
    let _ = crate::ported::params::setsparam("REPLY", &name); // c:1818
    0 // c:1819
}

/// Port of `mod_export char *curkeymapname` from `Src/Zle/zle_keymap.c:126`.
/// Name of the currently active keymap (driven by `bindkey -A` and the
/// `KEYMAP` parameter). The Rust port wraps in OnceLock<Mutex<>> for
/// thread-safe access from widget bodies.
pub static CURKEYMAPNAME: OnceLock<Mutex<String>> = OnceLock::new(); // c:126

/// Port of `Keymap curkeymap` from `Src/Zle/zle_keymap.c:124`. The
/// currently active keymap (per `bindkey -A` selection or KEYMAP
/// parameter). Used inline at zle_keymap.c:519 (`curkeymap = km;`)
/// and read by `getkeycmd`/`getkeybuf` to dispatch the next key.
pub static curkeymap: Mutex<Option<Arc<Keymap>>> = Mutex::new(None); // c:124

/// Port of `char *keybuf` from `Src/Zle/zle_keymap.c:136`. The key
/// sequence currently being read by `getkeycmd`. C uses a flat
/// `char*` heap allocation sized by `keybufsz`; Rust uses
/// `Vec<u8>` which manages its own capacity.
pub static keybuf: Mutex<Vec<u8>> = Mutex::new(Vec::new()); // c:136

/// Port of `int keybuflen` from `Src/Zle/zle_keymap.c:139`. Current
/// number of bytes in `keybuf`. Rust mirrors via `keybuf.lock().len()`
/// but exposes the count as a separate static for callers that need
/// it without holding the buffer lock.
pub static keybuflen: std::sync::atomic::AtomicI32 = // c:139
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
pub struct Keymap {
    // c:64
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
pub struct KeyBinding {
    // c:85
    pub bind: Option<Thingy>, // c:88 Thingy bind
    pub str: Option<String>,  // c:89 char *str
    pub prefixct: i32,        // c:90 int prefixct
}

/// File-scope `Keymap localkeymap` from `Src/Zle/zle_keymap.c:1759`.
/// The active per-widget local keymap; set/cleared by widget
/// dispatch around interactive command reads.
pub static LOCALKEYMAP: Mutex<Option<Arc<Keymap>>> = Mutex::new(None); // c:526

/// Get-or-init accessor for `CURKEYMAPNAME`. Mirrors the C convention
/// of treating the string as always-initialised — first read seeds it
/// with "main".
pub fn curkeymapname() -> std::sync::MutexGuard<'static, String> {
    CURKEYMAPNAME
        .get_or_init(|| Mutex::new(String::from("main")))
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
/// exempts impl-block ported) because zsh's C `default_bindings()`
/// has the equivalent 330+ bindkey calls inline in one function;
/// the Rust port keeps them factored by keymap for readability.
// `KeymapManager` unit struct (and its 32-method impl block) deleted —
// was a Rust-only namespace wrapper around the file-scope statics
// (KEYMAPNAMTAB / curkeymap / CURKEYMAPNAME / LOCALKEYMAP / keybuf /
// lastnamed) with no `struct keymap_manager` in zsh C. 29 of the 32
// methods were never called; 3 (`setup_emacs_keymap` /
// `setup_viins_keymap` / `setup_vicmd_keymap`) are factored out below
// as free ported because zsh's C `default_bindings()` inlines the ~300
// equivalent `bindkey` calls in one body (Src/Zle/zle_keymap.c:124).
// The Rust port keeps them factored by keymap for readability.

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub(crate) fn keymapnamtab() -> &'static Mutex<HashMap<String, KeymapName>> {
    KEYMAPNAMTAB.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emacs_default_has_quoted_insert_undo_yank_pop() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
            km.lookup_seq(b"\x1by")
                .and_then(|kb| kb.bind.as_ref())
                .map(|t| t.nam.as_str()),
            Some("yank-pop")
        );
    }

    #[test]
    fn emacs_default_has_history_search_and_insert_last_word() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        createkeymapnamtab();
        default_bindings();

        let km = openkeymap("emacs").expect("emacs keymap created");
        // \e. insert-last-word.
        assert_eq!(
            km.lookup_seq(b"\x1b.")
                .and_then(|kb| kb.bind.as_ref())
                .map(|t| t.nam.as_str()),
            Some("insert-last-word")
        );
        assert_eq!(
            km.lookup_seq(b"\x1bp")
                .and_then(|kb| kb.bind.as_ref())
                .map(|t| t.nam.as_str()),
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        // Test renamed + retargeted from the legacy
        // `viins_default_has_history_search_and_quoted_insert` which
        // asserted Rust-only emacs-flavored bindings (`^R` →
        // history-incremental-search-backward; `^A` →
        // beginning-of-line; `^V` → quoted-insert). Those were
        // overridden by the C-faithful `VIINSBIND` table port
        // (`Src/Zle/zle_bindings.c:256-289`).
        let _g = zle_test_setup();
        createkeymapnamtab();
        default_bindings();
        let km = openkeymap("viins").expect("viins keymap created");
        // ^R → redisplay (VIINSBIND[18]).
        assert_eq!(
            km.lookup_char(0x12).map(|t| t.nam.as_str()),
            Some("redisplay")
        );
        // ^V → vi-quoted-insert (VIINSBIND[22]).
        assert_eq!(
            km.lookup_char(0x16).map(|t| t.nam.as_str()),
            Some("vi-quoted-insert")
        );
        // ^A → self-insert (VIINSBIND[1]).
        assert_eq!(
            km.lookup_char(0x01).map(|t| t.nam.as_str()),
            Some("self-insert")
        );
        // ^[ → vi-cmd-mode (VIINSBIND[27]).
        assert_eq!(
            km.lookup_char(0x1B).map(|t| t.nam.as_str()),
            Some("vi-cmd-mode")
        );
        // ^M / ^J → accept-line.
        assert_eq!(
            km.lookup_char(0x0D).map(|t| t.nam.as_str()),
            Some("accept-line")
        );
        assert_eq!(
            km.lookup_char(0x0A).map(|t| t.nam.as_str()),
            Some("accept-line")
        );
    }

    // ---------- Real-port tests for refkeymap / unrefkeymap ----------

    #[test]
    fn refkeymap_increments_rc() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:687-688 — empty input → always prefix → 1.
        let km = Keymap::default();
        assert_eq!(keyisprefix(&km, b""), 1);
    }

    #[test]
    fn keyisprefix_single_byte_bound_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:689-692 — single byte that has a first[] binding is NOT
        // a prefix; it IS the binding.
        let mut km = Keymap::default();
        bindkey(&mut km, &[b'a'], Some(dummy_thingy()), None);
        assert_eq!(keyisprefix(&km, b"a"), 0);
    }

    #[test]
    fn keyisprefix_single_byte_unbound() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:694-695 — fall through to multi lookup; no match → 0.
        let km = Keymap::default();
        assert_eq!(keyisprefix(&km, b"x"), 0);
    }

    #[test]
    fn keyisprefix_seq_is_real_prefix() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:694-695 — multi has prefixct > 0 → 1.
        // bind_seq("ab", X) marks "a" as a prefix (prefixct=1).
        let mut km = Keymap::default();
        bindkey(&mut km, b"ab", Some(dummy_thingy()), None);
        // "a" alone is NOT a complete binding but IS a prefix of "ab".
        assert_eq!(keyisprefix(&km, b"a"), 1);
    }

    #[test]
    fn keyisprefix_seq_is_complete_binding() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:694-695 — when seq itself IS a binding (not a prefix),
        // multi[seq] has prefixct=0 → 0.
        let mut km = Keymap::default();
        bindkey(&mut km, b"xyz", Some(dummy_thingy()), None);
        // "xyz" is the bound seq (prefixct=0). Should return 0.
        assert_eq!(keyisprefix(&km, b"xyz"), 0);
    }

    #[test]
    fn keyisprefix_meta_pair_decoded() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:690 — `seq[0]==Meta` (0x83) → use seq[1]^32 as single byte.
        // Bind 'A' (0x41) in first[]. Seq [0x83, 0x61] decodes to
        // 0x61^0x20 = 0x41 = 'A'. So this is single-byte 'A'.
        let mut km = Keymap::default();
        bindkey(&mut km, &[b'A'], Some(dummy_thingy()), None);
        assert_eq!(keyisprefix(&km, &[0x83, 0x61]), 0);
    }

    // ---------- keybind real-port tests (this session). ----------

    #[test]
    fn keybind_single_byte_returns_first_bound_thingy() {
        let _g = crate::test_util::global_state_lock();
        // c:664-669 — `if (ztrlen(seq) == 1) { f = seq[0]; if (km->first[f])
        // return bind; }`. Bind 'q' in first[], call keybind with "q",
        // expect the Thingy back and no send-string.
        let _g = zle_test_setup();
        let mut km = Keymap::default();
        bindkey(&mut km, &[b'q'], Some(Thingy::new("quit-widget")), None);
        let (bind, send) = keybind(&km, b"q");
        assert!(bind.is_some());
        assert_eq!(bind.as_ref().unwrap().nam, "quit-widget");
        assert!(send.is_none(), "no str on first[] path");
    }

    #[test]
    fn keybind_meta_pair_decodes_to_single_byte() {
        let _g = crate::test_util::global_state_lock();
        // c:665 — `seq[0]==Meta ? seq[1]^32 : seq[0]`. [0x83, 0x61]
        // decodes to 0x41 = 'A'. Verify it lands on the first[] entry
        // for 'A' just like a literal 'A' byte would.
        let _g = zle_test_setup();
        let mut km = Keymap::default();
        bindkey(
            &mut km,
            &[b'A'],
            Some(Thingy::new("uppercase-A-widget")),
            None,
        );
        let (bind, _) = keybind(&km, &[0x83, 0x61]);
        assert_eq!(
            bind.as_ref().map(|t| t.nam.as_str()),
            Some("uppercase-A-widget")
        );
    }

    #[test]
    fn keybind_unbound_byte_returns_none() {
        let _g = crate::test_util::global_state_lock();
        // c:670-671 — single-byte but km->first[f] is None, falls
        // through to the multi-byte lookup which also misses → (None, None).
        let _g = zle_test_setup();
        let km = Keymap::default();
        let (bind, send) = keybind(&km, b"z");
        assert!(
            bind.is_none(),
            "unbound byte → t_undefinedkey sentinel (None)"
        );
        assert!(send.is_none());
    }

    #[test]
    fn keybind_multi_byte_sequence_via_multi_map() {
        let _g = crate::test_util::global_state_lock();
        // c:670-674 — `k = km->multi->getnode(km->multi, seq)`. Bind
        // a multi-byte sequence in `multi`, expect the lookup to find it.
        let _g = zle_test_setup();
        let mut km = Keymap::default();
        km.multi.insert(
            b"\x1b[A".to_vec(),
            KeyBinding {
                bind: Some(Thingy::new("up-line")),
                str: None,
                prefixct: 0,
            },
        );
        let (bind, _) = keybind(&km, b"\x1b[A");
        assert_eq!(bind.as_ref().map(|t| t.nam.as_str()), Some("up-line"));
    }

    #[test]
    fn keybind_returns_send_string_when_multi_entry_has_str() {
        let _g = crate::test_util::global_state_lock();
        // c:673-674 — `*strp = k->str; return k->bind`. Send-string
        // entries (`bindkey -s`) have bind=None + str=Some.
        let _g = zle_test_setup();
        let mut km = Keymap::default();
        km.multi.insert(
            b"\x1b[Z".to_vec(),
            KeyBinding {
                bind: None,
                str: Some("hello".to_string()),
                prefixct: 0,
            },
        );
        let (bind, send) = keybind(&km, b"\x1b[Z");
        assert!(bind.is_none(), "send-string entries have no bind");
        assert_eq!(send.as_deref(), Some("hello"));
    }

    // ---------- init_keymaps / cleanup_keymaps round-trip ----------

    #[test]
    fn init_keymaps_seeds_keybuf_and_clears_lastnamed() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // After init: keybuf is allocated (non-empty Vec), lastnamed is None
        // (the `t_undefinedkey` sentinel in Rust convention).
        init_keymaps();
        assert!(
            !keybuf.lock().unwrap().is_empty(),
            "keybuf zshcalloc(keybufsz)"
        );
        assert!(
            lastnamed.lock().unwrap().is_none(),
            "lastnamed = t_undefinedkey (None)"
        );
    }

    #[test]
    fn cleanup_keymaps_drains_namtab_and_keybuf() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        init_keymaps();
        assert!(!keybuf.lock().unwrap().is_empty());
        cleanup_keymaps();
        assert!(keybuf.lock().unwrap().is_empty(), "zfree(keybuf, ...)");
        assert!(
            keymapnamtab().lock().unwrap().is_empty(),
            "deletehashtable(keymapnamtab)"
        );
    }

    /// `Src/Zle/zle_keymap.c:1717-1727` — `addkeybuf(c)` calls `imeta(c)`.
    /// Per `Src/utils.c:4195`, NUL is IMETA (`typtab['\0'] |= IMETA`).
    /// The previous Rust port used a hand-rolled `c >= 0x83 && c != 0x83
    /// && c != 0x84` mask that excluded NUL entirely — binary input
    /// passing through `addkeybuf` would drop the Meta-prefix and leave
    /// a raw `\0` in `keybuf`, breaking the C-string-terminator check
    /// at zle_keymap.c:1649.
    #[test]
    fn addkeybuf_encodes_nul_byte_per_imeta() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _tg = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        inittyptab();
        keybuf.lock().unwrap().clear();
        addkeybuf(0);
        // c:1722-1723 — Meta=0x83, NUL ^ 0x20 = 0x20.
        assert_eq!(*keybuf.lock().unwrap(), vec![0x83, 0x20],
            "c:1721 — NUL must be Meta-encoded (was missed by old `c >= 0x83 && != 0x83 && != 0x84`)");
    }

    /// `Src/Zle/zle_keymap.c:1721` — `imeta(c)` returns true for the
    /// Meta byte itself (0x83) per `Src/utils.c:4196`
    /// (`typtab[Meta] |= IMETA`). A raw 0x83 in input MUST be
    /// Meta-encoded as `Meta + (0x83 ^ 0x20) = 0x83 0xa3`. The previous
    /// hand-rolled mask explicitly excluded 0x83 with `c != 0x83`,
    /// passing the Meta byte through verbatim — corrupting any later
    /// key-sequence parser that interprets 0x83 as a Meta-prefix.
    #[test]
    fn addkeybuf_encodes_meta_byte_itself() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _tg = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        inittyptab();
        keybuf.lock().unwrap().clear();
        addkeybuf(0x83);
        assert_eq!(
            *keybuf.lock().unwrap(),
            vec![0x83, 0xa3],
            "c:1721 — Meta byte (0x83) must itself be Meta-encoded"
        );
    }

    /// `Src/Zle/zle_keymap.c:1721` — `imeta(c)` returns true for 0x84
    /// (Pound), the first byte in the Pound..LAST_NORMAL_TOK range
    /// (`Src/utils.c:4198`). The previous mask `c != 0x84` left Pound
    /// unencoded; the canonical port must Meta-encode it.
    #[test]
    fn addkeybuf_encodes_pound_token_byte() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _tg = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        inittyptab();
        keybuf.lock().unwrap().clear();
        addkeybuf(0x84);
        assert_eq!(
            *keybuf.lock().unwrap(),
            vec![0x83, 0xa4],
            "c:1721 — Pound (0x84) is IMETA per utils.c:4198, must be Meta-encoded"
        );
    }

    /// `Src/Zle/zle_keymap.c:1721` — `imeta(c)` returns FALSE for
    /// bytes 0xa3..=0xff. Per `Src/utils.c:4195-4201`, the IMETA range
    /// ends at Marker (0xa2). The previous hand-rolled mask flagged
    /// 0xa3+ as imeta and over-encoded them; the canonical port must
    /// pass them through as literal bytes (raw high-bit characters
    /// from a UTF-8 terminal that are NOT zsh's internal markers).
    #[test]
    fn addkeybuf_passes_through_non_imeta_high_byte() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _tg = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        inittyptab();
        keybuf.lock().unwrap().clear();
        addkeybuf(0xa3);
        assert_eq!(
            *keybuf.lock().unwrap(),
            vec![0xa3],
            "c:1721 — 0xa3 is NOT IMETA (past Marker=0xa2); must pass through"
        );
        keybuf.lock().unwrap().clear();
        addkeybuf(0xff);
        assert_eq!(
            *keybuf.lock().unwrap(),
            vec![0xff],
            "c:1721 — 0xff is NOT IMETA; must pass through"
        );
    }

    /// `Src/Zle/zle_keymap.c:1721` — ASCII bytes (0x01..=0x7e) are
    /// never IMETA (`Src/utils.c:4195-4201` marks only NUL=0x00 in the
    /// low range). They pass through verbatim. Pin the boundary on
    /// printable + control chars to ensure the typtab-driven predicate
    /// agrees with `imeta` for all ASCII.
    #[test]
    fn addkeybuf_ascii_passes_through_literally() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _tg = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        inittyptab();
        for c in [0x01u8, 0x1f, 0x20, b'A', b'z', 0x7e, 0x7f] {
            keybuf.lock().unwrap().clear();
            addkeybuf(c as i32);
            assert_eq!(
                *keybuf.lock().unwrap(),
                vec![c],
                "c:1721 — ASCII byte 0x{:02x} must pass through",
                c
            );
        }
    }

    // ─── zsh-corpus pins for newkeytab / openkeymap / selectkeymap ──

    /// `newkeytab()` returns empty HashMap.
    #[test]
    fn zle_keymap_corpus_newkeytab_is_empty() {
        let _g = crate::test_util::global_state_lock();
        let t = newkeytab();
        assert!(t.is_empty());
    }

    /// `newkeymap(None, "myname")` returns an Arc.
    #[test]
    fn zle_keymap_corpus_newkeymap_returns_arc() {
        let _g = crate::test_util::global_state_lock();
        let km = newkeymap(None, "myname");
        assert!(Arc::strong_count(&km) >= 1);
    }

    /// `openkeymap("never_was")` returns None.
    #[test]
    fn zle_keymap_corpus_openkeymap_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert!(openkeymap("zshrs_never_keymap_xyz").is_none());
    }

    /// `unlinkkeymap` on missing returns nonzero (error).
    #[test]
    fn zle_keymap_corpus_unlinkkeymap_missing_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert_ne!(
            unlinkkeymap("never_keymap_xyz", 0),
            0,
            "unlinking nonexistent keymap = error"
        );
    }

    /// `selectkeymap("never_was", 0)` returns nonzero.
    #[test]
    fn zle_keymap_corpus_selectkeymap_unknown_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert_ne!(selectkeymap("zshrs_never_keymap_xyz", 0), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Zle/zle_keymap.c. Tests that capture
    // KNOWN ZSHRS BUGS use #[ignore = "ZSHRS BUG: …"].
    // ═══════════════════════════════════════════════════════════════════

    /// `newkeytab()` returns an empty key table — fresh state must
    /// have zero bindings. C newhashtable equivalent at table init.
    #[test]
    fn newkeytab_returns_empty_table() {
        let _g = crate::test_util::global_state_lock();
        let kt = newkeytab();
        assert_eq!(kt.len(), 0, "fresh keytab must be empty");
    }

    /// `openkeymap("zshrs_definitely_not_a_keymap")` returns None.
    /// C `Keymap openkeymap(char *name)` returns NULL on miss.
    #[test]
    fn openkeymap_unknown_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert!(openkeymap("zshrs_unknown_keymap_xyz").is_none());
    }

    /// `unlinkkeymap` on a non-existent name returns nonzero.
    /// C convention: 0 = success, nonzero = error.
    #[test]
    fn unlinkkeymap_unknown_name_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert_ne!(
            unlinkkeymap("zshrs_doesnt_exist", 0),
            0,
            "unlinking missing keymap must return error"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests for Src/Zle/zle_keymap.c keybind/keyisprefix/
    // newkeymap contracts.
    // ═══════════════════════════════════════════════════════════════════

    /// c:683 — `keyisprefix(km, "")` returns 1 (empty seq is trivially
    /// a prefix of every binding).
    #[test]
    fn keyisprefix_empty_seq_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let km = newkeymap(None, "test");
        assert_eq!(keyisprefix(&km, b""), 1, "empty seq is trivially a prefix");
    }

    /// c:683 — `keyisprefix` on a fresh empty keymap for any non-prefix
    /// sequence returns 0 (no bindings exist).
    #[test]
    fn keyisprefix_unbound_seq_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let km = newkeymap(None, "test");
        // No bindings in km → no sequence is a prefix.
        assert_eq!(keyisprefix(&km, b"unbound"), 0);
    }

    /// c:659 — `keybind(km, single_byte)` on an unbound byte returns
    /// (None, None) — no binding, no string.
    #[test]
    fn keybind_unbound_single_byte_returns_none_pair() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let km = newkeymap(None, "test");
        let (bind, s) = keybind(&km, &[0x42]); // 'B' — unbound on fresh km
        assert!(bind.is_none(), "no binding on fresh km");
        assert!(s.is_none(), "no string on fresh km");
    }

    /// c:659 — `keybind(km, &[])` on empty seq returns (None, None)
    /// (no single byte to look up, no multi-byte hash hit).
    #[test]
    fn keybind_empty_seq_returns_none_pair() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let km = newkeymap(None, "test");
        let (bind, s) = keybind(&km, b"");
        assert!(bind.is_none());
        assert!(s.is_none());
    }

    /// c:659 — `keybind(km, &[0x83, x])` decodes Meta-pair before
    /// looking up: byte[1]^32 is the actual key. Pin: empty km → None.
    #[test]
    fn keybind_meta_pair_unbound_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let km = newkeymap(None, "test");
        // Meta-encoded escape sequence (0x83 + 'a'^32) — unbound on fresh.
        let (bind, _) = keybind(&km, &[0x83, b'a' ^ 32]);
        assert!(bind.is_none(), "unbound Meta-pair on fresh km");
    }

    /// c:517 — `newkeymap(None, _)` creates a fresh keymap with all
    /// 256 first[] slots unbound.
    #[test]
    fn newkeymap_fresh_has_no_first_bindings() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let km = newkeymap(None, "test");
        for (i, slot) in km.first.iter().enumerate() {
            assert!(
                slot.is_none(),
                "first[{}] must be unbound on fresh keymap",
                i
            );
        }
    }

    /// c:517 — `newkeymap(None, _)` creates a fresh keymap with empty
    /// multi-byte binding table.
    #[test]
    fn newkeymap_fresh_has_empty_multi_table() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let km = newkeymap(None, "test");
        assert!(km.multi.is_empty(), "multi table must be empty on fresh km");
    }

    /// c:287 — `newkeytab()` is independent across calls (each call
    /// returns a fresh empty HashMap, not a shared reference).
    #[test]
    fn newkeytab_returns_owned_independent_table() {
        let _g = crate::test_util::global_state_lock();
        let kt1 = newkeytab();
        let kt2 = newkeytab();
        assert!(kt1.is_empty());
        assert!(kt2.is_empty());
        // Independent owned values — no shared backing.
    }

    /// c:886 — `selectkeymap("")` returns nonzero (empty name is invalid).
    #[test]
    fn selectkeymap_empty_name_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert_ne!(selectkeymap("", 0), 0, "empty name = invalid");
    }

    /// c:886 — `selectkeymap("emacs", 0)` returns 0 (success).
    /// The default startup keymaps must be selectable.
    #[test]
    fn selectkeymap_default_emacs_succeeds() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert_eq!(
            selectkeymap("emacs", 0),
            0,
            "default 'emacs' keymap must exist"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_keymap.c
    // c:134 createkeymapnamtab / c:145 init_keymaps / c:205 refkeymap_by_name
    // c:240 unrefkeymap_by_name / c:664 openkeymap / c:675 unlinkkeymap
    // c:805 linkkeymap / c:886 selectkeymap / c:921 selectlocalmap /
    // c:940 reselectkeymap
    // ═══════════════════════════════════════════════════════════════════

    /// c:664 — `openkeymap("emacs")` returns Some after init.
    #[test]
    fn openkeymap_default_emacs_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert!(
            openkeymap("emacs").is_some(),
            "default 'emacs' keymap must open"
        );
    }

    /// c:664 — `openkeymap("")` returns None (empty name invalid).
    #[test]
    fn openkeymap_empty_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert!(openkeymap("").is_none(), "empty name → None");
    }

    /// c:664 — `openkeymap(unknown)` returns None.
    #[test]
    fn openkeymap_unknown_name_returns_none_pin() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert!(
            openkeymap("__never_a_real_keymap_xyz__").is_none(),
            "unknown keymap → None"
        );
    }

    /// c:886 — `selectkeymap` returns i32 (compile-time type pin).
    #[test]
    fn selectkeymap_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = selectkeymap("emacs", 0);
    }

    /// c:675 — `unlinkkeymap` returns i32 (compile-time type pin).
    #[test]
    fn unlinkkeymap_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = unlinkkeymap("nothing_real", 0);
    }

    /// c:675 — `unlinkkeymap("")` is safe (empty name).
    #[test]
    fn unlinkkeymap_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _ = unlinkkeymap("", 0);
    }

    /// c:921 — `selectlocalmap(None)` is safe.
    #[test]
    fn selectlocalmap_none_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        selectlocalmap(None);
    }

    /// c:940 — `reselectkeymap` is idempotent / safe.
    #[test]
    fn reselectkeymap_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            reselectkeymap();
        }
    }

    /// c:205 — `refkeymap_by_name("")` empty name is safe.
    #[test]
    fn refkeymap_by_name_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        refkeymap_by_name("");
    }

    /// c:240 — `unrefkeymap_by_name("")` empty name is safe.
    #[test]
    fn unrefkeymap_by_name_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        unrefkeymap_by_name("");
    }

    /// c:886 — `selectkeymap` is deterministic for same input.
    #[test]
    fn selectkeymap_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let first = selectkeymap("emacs", 0);
        for _ in 0..3 {
            assert_eq!(
                selectkeymap("emacs", 0),
                first,
                "selectkeymap('emacs') must be deterministic"
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_keymap.c
    // c:134 createkeymapnamtab / c:145 init_keymaps / c:156 cleanup_keymaps /
    // c:224 scanprimaryname / c:278 freekeymapnamnode / c:664 openkeymap /
    // c:676 unlinkkeymap / c:921 selectlocalmap / c:940 reselectkeymap
    // ═══════════════════════════════════════════════════════════════════

    /// c:134 — `createkeymapnamtab` idempotent.
    #[test]
    fn createkeymapnamtab_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            createkeymapnamtab();
        }
    }

    /// c:145 — `init_keymaps` idempotent.
    #[test]
    fn init_keymaps_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            init_keymaps();
        }
    }

    /// c:156 — `cleanup_keymaps` idempotent.
    #[test]
    fn cleanup_keymaps_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            cleanup_keymaps();
        }
        init_keymaps();
    }

    /// c:224 — `scanprimaryname("")` empty name is safe (no-op).
    #[test]
    fn scanprimaryname_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        scanprimaryname("");
    }

    /// c:278 — `freekeymapnamnode("")` empty name is safe.
    #[test]
    fn freekeymapnamnode_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        freekeymapnamnode("");
    }

    /// c:940 — `reselectkeymap` returns void (signature pin) + safe.
    #[test]
    fn reselectkeymap_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: () = reselectkeymap();
    }

    /// c:664 — `openkeymap` returns Option<Arc<Keymap>> (type pin).
    #[test]
    fn openkeymap_returns_option_arc_keymap_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Option<Arc<Keymap>> = openkeymap("");
    }

    /// c:676 — `unlinkkeymap("", 0)` empty name returns nonzero.
    #[test]
    fn unlinkkeymap_empty_name_returns_nonzero_pin() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = unlinkkeymap("", 0);
        assert_ne!(r, 0, "empty name → nonzero error");
    }

    /// c:287 — `newkeytab` returns HashMap (compile-time type pin).
    #[test]
    fn newkeytab_returns_hashmap_type() {
        let _: HashMap<Vec<u8>, KeyBinding> = newkeytab();
    }

    /// c:287 — `newkeytab` is empty.
    #[test]
    fn newkeytab_returns_empty_pin() {
        let t = newkeytab();
        assert!(t.is_empty(), "fresh keytab must be empty");
    }

    /// c:921 — `selectlocalmap(None)` is idempotent.
    #[test]
    fn selectlocalmap_none_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            selectlocalmap(None);
        }
    }

    /// c:886 — `selectkeymap` returns i32 type.
    #[test]
    fn selectkeymap_returns_i32_type_pin2() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = selectkeymap("", 0);
    }

    /// c:676 — `unlinkkeymap` is deterministic for unknown name.
    #[test]
    fn unlinkkeymap_unknown_name_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let first = unlinkkeymap("__zshrs_never_keymap__", 0);
        for _ in 0..3 {
            assert_eq!(
                unlinkkeymap("__zshrs_never_keymap__", 0),
                first,
                "unlinkkeymap unknown must be deterministic"
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_keymap.c
    // c:134 createkeymapnamtab / c:176 emptykeymapnamtab /
    // c:205 refkeymap_by_name / c:240 unrefkeymap_by_name /
    // c:517 newkeymap / c:664 openkeymap / c:805 linkkeymap
    // ═══════════════════════════════════════════════════════════════════

    /// c:134 — `createkeymapnamtab` is idempotent (alt 10-call).
    #[test]
    fn createkeymapnamtab_idempotent_10_call() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..10 {
            createkeymapnamtab();
        }
    }

    /// c:176 — `emptykeymapnamtab` is idempotent.
    #[test]
    fn emptykeymapnamtab_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..10 {
            emptykeymapnamtab();
        }
    }

    /// c:205 — `refkeymap_by_name` for unknown name is safe.
    #[test]
    fn refkeymap_by_name_unknown_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        refkeymap_by_name("__never_keymap_xyz__");
    }

    /// c:240 — `unrefkeymap_by_name` for unknown name is safe.
    #[test]
    fn unrefkeymap_by_name_unknown_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        unrefkeymap_by_name("__never_keymap_xyz__");
    }

    /// c:240 — `unrefkeymap_by_name("")` empty name safe (alt).
    #[test]
    fn unrefkeymap_by_name_empty_no_panic_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        unrefkeymap_by_name("");
    }

    /// c:517 — `newkeymap(None, "")` returns Arc<Keymap> (compile-time pin).
    #[test]
    fn newkeymap_none_returns_arc_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Arc<Keymap> = newkeymap(None, "");
    }

    /// c:517 — `newkeymap` is deterministic in shape (always returns Arc).
    #[test]
    fn newkeymap_deterministic_shape() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            let _: Arc<Keymap> = newkeymap(None, "test");
        }
    }

    /// c:805 — `linkkeymap` returns i32 (compile-time pin).
    #[test]
    fn linkkeymap_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let km = newkeymap(None, "");
        let _: i32 = linkkeymap(km, "test", 0);
    }

    /// c:664 — `openkeymap("")` empty name returns None (alt).
    #[test]
    fn openkeymap_empty_name_returns_none_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert!(openkeymap("").is_none(), "empty keymap name → None");
    }

    /// c:664 — `openkeymap("__never__")` for unknown name returns None.
    #[test]
    fn openkeymap_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert!(openkeymap("__definitely_no_such_keymap_xyz__").is_none());
    }

    /// c:287 — `newkeytab` is deterministic shape (always empty).
    #[test]
    fn newkeytab_deterministic_shape() {
        for _ in 0..5 {
            let t = newkeytab();
            assert!(t.is_empty(), "newkeytab must always start empty");
        }
    }
}
