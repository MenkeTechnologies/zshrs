//! Rust-only utility (NOT a port — lives outside `src/ported/` by design).
//!
//! C's `Keymap` is one heap object that every `KeymapName` aliasing it
//! points at, so `km->primary = kmn` (`Src/Zle/zle_keymap.c:213`, `:237`,
//! `:256`) is a single store that every alias sees at once. The port holds
//! keymaps as shared-immutable `Arc<Keymap>` and mutates one by
//! clone-modify-propagate (the same shape `bin_bindkey_bind` uses for a
//! binding change): clone the keymap, change the field, then re-point every
//! name that held the old `Arc` at the new one, so the alias set survives.

use std::sync::Arc;

use crate::ported::hashtable::hashtable_nodes;
use crate::ported::zle::zle_keymap::{Keymap, KeymapName};

/// Store `primary` as the primary name of keymap `km` — C's
/// `km->primary = primary` — for every name in `tab` that aliases `km`.
/// Returns the keymap as it now stands (the old `Arc` when nothing changed).
pub fn set_keymap_primary(
    tab: &mut hashtable_nodes<KeymapName>,
    km: &Arc<Keymap>,
    primary: Option<String>,
) -> Arc<Keymap> {
    if km.primary == primary {
        return km.clone();
    }
    let mut updated = (**km).clone();
    updated.primary = primary;
    let updated = Arc::new(updated);
    for (_, kmn) in tab.iter_mut() {
        if Arc::ptr_eq(&kmn.keymap, km) {
            kmn.keymap = updated.clone();
        }
    }
    updated
}
