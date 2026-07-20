//! Bash sparse-array support — a Rust-only extension with NO zsh C
//! counterpart.
//!
//! zsh (and zshrs) arrays are DENSE `Vec<String>`: `a[5]=q` on a 3-element
//! array pads indices 3,4 with empty strings, so `${#a[@]}` is 6. bash
//! arrays are SPARSE maps: the same op leaves indices {0,1,2,5}, so
//! `${#a[@]}` is 4, `${!a[@]}` is `0 1 2 5`, and `unset a[1]` removes index
//! 1 leaving {0,2}.
//!
//! Rather than change the dense `Vec<String>` representation the whole
//! param system (and `--zsh`, the primary target) depends on, this tracks
//! the "holes" — the indices that are padding artifacts or were `unset`,
//! and therefore NOT real elements — in a side table keyed by array name.
//! It is consulted only in `--bash` mode; `${#}`/`${!}`/`${@}` subtract the
//! holes, subscript-assign/append/full-reassign maintain them, and `unset
//! a[i]` marks a hole. Everything else keeps operating on the dense Vec.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

thread_local! {
    /// name → set of HOLE indices (padding / unset slots that are not real
    /// elements). Absent name ⇒ fully dense (no holes).
    static HOLES: RefCell<HashMap<String, BTreeSet<usize>>> = RefCell::new(HashMap::new());
}

/// Clear all hole tracking for `name` — the array became fully dense
/// (a plain `a=(...)` reassign, `unset a`, or a non-bash mutation).
pub fn clear(name: &str) {
    HOLES.with(|h| {
        h.borrow_mut().remove(name);
    });
}

/// Record that a subscript assignment set index `i`, padding the dense Vec
/// from `old_len` up to `i`. Indices `old_len..i` become holes; `i` itself
/// is a real element (removed from the hole set).
pub fn note_subscript_set(name: &str, old_len: usize, i: usize) {
    HOLES.with(|h| {
        let mut map = h.borrow_mut();
        let set = map.entry(name.to_string()).or_default();
        if i >= old_len {
            for k in old_len..i {
                set.insert(k);
            }
        }
        set.remove(&i);
        if set.is_empty() {
            map.remove(name);
        }
    });
}

/// Mark index `i` a hole (an `unset a[i]` that leaves a gap rather than a
/// dense empty). Returns nothing; callers keep the dense Vec's slot as "".
pub fn note_unset(name: &str, i: usize) {
    HOLES.with(|h| {
        h.borrow_mut()
            .entry(name.to_string())
            .or_default()
            .insert(i);
    });
}

/// Drop any hole indices >= `new_len` — the array shrank (truncation), so
/// trailing holes no longer exist.
pub fn truncate(name: &str, new_len: usize) {
    HOLES.with(|h| {
        let mut map = h.borrow_mut();
        if let Some(set) = map.get_mut(name) {
            set.retain(|&k| k < new_len);
            if set.is_empty() {
                map.remove(name);
            }
        }
    });
}

/// The hole indices for `name` (empty if dense).
pub fn holes(name: &str) -> BTreeSet<usize> {
    HOLES.with(|h| h.borrow().get(name).cloned().unwrap_or_default())
}

/// True if `name` has any tracked holes.
pub fn has_holes(name: &str) -> bool {
    HOLES.with(|h| h.borrow().get(name).map_or(false, |s| !s.is_empty()))
}

/// The live (non-hole) indices of a dense Vec of length `len` for `name`,
/// in ascending order. With no holes this is simply `0..len`.
pub fn live_indices(name: &str, len: usize) -> Vec<usize> {
    let hs = holes(name);
    (0..len).filter(|k| !hs.contains(k)).collect()
}

/// The live element COUNT for a dense Vec of length `dense_len` — `dense_len`
/// minus the holes below it. Inherently a no-op (`== dense_len`) outside
/// `--bash` mode, where no holes are ever recorded. Backs `${#a[@]}`.
pub fn live_len(name: &str, dense_len: usize) -> usize {
    HOLES.with(|h| match h.borrow().get(name) {
        Some(set) => dense_len - set.iter().filter(|&&k| k < dense_len).count(),
        None => dense_len,
    })
}

/// Filter a dense whole-array Vec down to its live elements, dropping the
/// hole slots (padding / `unset` gaps). Order-preserving. No-op outside
/// `--bash` (no holes tracked). Backs the `${a[@]}` / `${a[*]}` splat.
pub fn compact(name: &str, dense: Vec<String>) -> Vec<String> {
    let hs = holes(name);
    if hs.is_empty() {
        return dense;
    }
    dense
        .into_iter()
        .enumerate()
        .filter(|(k, _)| !hs.contains(k))
        .map(|(_, v)| v)
        .collect()
}
