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

/// Record every slot of a freshly-written `0..len` array that the literal
/// did NOT name as a hole.
///
/// An explicit-index array literal is sparse in bash: `a=([2]=x [5]=y)`
/// holds indices {2,5}, not a dense 0..5 with empties. The dense `Vec`
/// underneath has already been written by the caller's setter; this marks
/// the slots between the named ones, which is what `${a[@]}`, `${!a[@]}`
/// and `${#a[@]}` consult.
///
/// Shared by all three literal paths — the `a=(…)` builtin, `declare -a`'s
/// bracket form, and `declare -a`'s Marker-triad form — so they cannot
/// drift apart the way the first two did: the triad form had no marking at
/// all, and `declare -a a=([5]=x [10]=y)` came out dense with eleven
/// elements where bash reports two.
pub fn note_holes_outside(name: &str, explicit: &BTreeSet<usize>, len: usize) {
    for i in 0..len {
        if !explicit.contains(&i) {
            note_unset(name, i);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dense(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Each test runs on its own thread, so the thread-local table starts
    /// empty; the expected values are bash 3.2+/5.x output for the same ops.
    #[test]
    fn subscript_past_end_leaves_padding_holes() {
        // a=(x y z); a[5]=q  →  ${#a[@]}=4  ${!a[@]}=0 1 2 5
        note_subscript_set("a", 3, 5);
        assert_eq!(holes("a"), BTreeSet::from([3, 4]));
        assert_eq!(live_len("a", 6), 4);
        assert_eq!(live_indices("a", 6), vec![0, 1, 2, 5]);
        assert_eq!(
            compact("a", dense(&["x", "y", "z", "", "", "q"])),
            dense(&["x", "y", "z", "q"])
        );
    }

    #[test]
    fn unset_then_reassign_fills_the_hole() {
        // a=(x y z); a[5]=q; unset 'a[1]'  →  3 / 0 2 5 / x z q
        note_subscript_set("a", 3, 5);
        note_unset("a", 1);
        assert_eq!(live_len("a", 6), 3);
        assert_eq!(live_indices("a", 6), vec![0, 2, 5]);
        assert_eq!(
            compact("a", dense(&["x", "", "z", "", "", "q"])),
            dense(&["x", "z", "q"])
        );
        // a[1]=r inside the dense length: only index 1 becomes live.
        note_subscript_set("a", 6, 1);
        assert_eq!(live_indices("a", 6), vec![0, 1, 2, 5]);
        // a[3]=s and a[4]=t fill the remaining padding, so the side table
        // entry disappears and the array is dense again.
        note_subscript_set("a", 6, 3);
        note_subscript_set("a", 6, 4);
        assert!(!has_holes("a"));
        assert_eq!(live_len("a", 6), 6);
    }

    #[test]
    fn appending_at_the_end_records_no_hole() {
        note_subscript_set("a", 3, 3);
        assert!(!has_holes("a"));
        assert_eq!(live_indices("a", 4), vec![0, 1, 2, 3]);
    }

    #[test]
    fn explicit_index_literal_is_sparse() {
        // b=([2]=x [5]=y)  →  ${#b[@]}=2  ${!b[@]}=2 5
        note_holes_outside("b", &BTreeSet::from([2, 5]), 6);
        assert_eq!(live_len("b", 6), 2);
        assert_eq!(live_indices("b", 6), vec![2, 5]);
        assert_eq!(
            compact("b", dense(&["", "", "x", "", "", "y"])),
            dense(&["x", "y"])
        );
    }

    #[test]
    fn unset_past_the_end_does_not_change_the_count() {
        // unset 'a[9]' on a 6-slot array: bash still reports 4 elements.
        note_subscript_set("a", 3, 5);
        note_unset("a", 9);
        assert_eq!(live_len("a", 6), 4);
        assert_eq!(live_indices("a", 6), vec![0, 1, 2, 5]);
        // A later a[9]=v pads 6..8 and makes 9 live again.
        note_subscript_set("a", 6, 9);
        assert_eq!(live_indices("a", 10), vec![0, 1, 2, 5, 9]);
    }

    #[test]
    fn truncate_drops_trailing_holes_and_clears_when_empty() {
        note_subscript_set("a", 0, 6);
        assert_eq!(holes("a"), (0..6).collect::<BTreeSet<usize>>());
        truncate("a", 3);
        assert_eq!(holes("a"), BTreeSet::from([0, 1, 2]));
        truncate("a", 0);
        assert!(!has_holes("a"));
        truncate("never-tracked", 0);
        assert!(!has_holes("never-tracked"));
    }

    #[test]
    fn tracking_is_per_name_and_clear_resets_one_array() {
        note_unset("a", 0);
        note_unset("b", 1);
        clear("a");
        assert!(!has_holes("a"));
        assert_eq!(holes("b"), BTreeSet::from([1]));
        // Untracked names are dense passthroughs.
        assert_eq!(live_len("c", 3), 3);
        assert_eq!(compact("c", dense(&["p", "q"])), dense(&["p", "q"]));
    }
}
