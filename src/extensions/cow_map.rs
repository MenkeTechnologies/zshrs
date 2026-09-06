//! `cow_map` — copy-on-write `HashMap` wrapper for cheap subshell
//! snapshot/restore.
//!
//! !!! WARNING: RUST-ONLY HELPER !!!
//! C zsh has no counterpart. `$( … )` and `( … )` are forks there
//! (`c:Src/exec.c:4783` `getoutput` → `entersubsh`), so the child gets
//! the parent's whole address space by page-table copy and the kernel
//! does the copy-on-write. zshrs runs both forms IN PROCESS and has to
//! snapshot the mutable globals by hand, which turned "enter a
//! substitution" into O(total shell state). This type is the userspace
//! stand-in for the page-table trick: sharing until someone writes.
//!
//! Every read goes through `Deref` and touches the shared map. Every
//! `&mut` method goes through `DerefMut`, which calls `Arc::make_mut` —
//! a no-op when the map is unshared (the ordinary case), and a single
//! deep copy the first time a subshell body writes while the parent
//! still holds a snapshot. After that copy the subshell owns its map
//! and the parent's snapshot is frozen at the pre-write contents, which
//! is exactly what the fork gave C.
//!
//! `clone()` is deliberately NOT a deep copy — it is the snapshot
//! operation, so it must stay O(1). Independence is still guaranteed:
//! whichever side writes first is the one that pays for the split.

use std::collections::HashMap;
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// Copy-on-write associative store used for subshell snapshot/restore.
///
/// Drop-in for `HashMap<K, V>` at read and write call sites via
/// `Deref`/`DerefMut`; the difference is only visible in the cost of
/// `clone()`.
#[derive(Debug)]
pub struct CowHashMap<K, V> {
    inner: Arc<HashMap<K, V>>,
}

impl<K, V> CowHashMap<K, V> {
    /// Empty map, sharing nothing.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(HashMap::new()),
        }
    }

    /// True while another handle (a live subshell snapshot) shares this
    /// map, i.e. while the next write will pay for a deep copy.
    pub fn is_shared(&self) -> bool {
        Arc::strong_count(&self.inner) > 1
    }
}

impl<K, V> Default for CowHashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

/// O(1) — a refcount bump, NOT a deep copy. This is the snapshot
/// operation; see the module docs.
impl<K, V> Clone for CowHashMap<K, V> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<K, V> Deref for CowHashMap<K, V> {
    type Target = HashMap<K, V>;

    fn deref(&self) -> &HashMap<K, V> {
        &self.inner
    }
}

impl<K: Clone + Eq + Hash, V: Clone> DerefMut for CowHashMap<K, V> {
    /// Splits the map away from any snapshot sharing it, then hands out
    /// the `&mut`. A caller that takes `&mut` only to read still pays
    /// the split — that is a cost bug, never a correctness one.
    fn deref_mut(&mut self) -> &mut HashMap<K, V> {
        Arc::make_mut(&mut self.inner)
    }
}

impl<K, V> From<HashMap<K, V>> for CowHashMap<K, V> {
    fn from(map: HashMap<K, V>) -> Self {
        Self {
            inner: Arc::new(map),
        }
    }
}

impl<K: Clone + Eq + Hash, V: Clone> FromIterator<(K, V)> for CowHashMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self::from(HashMap::from_iter(iter))
    }
}

impl<'a, K, V> IntoIterator for &'a CowHashMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = std::collections::hash_map::Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

impl<K: Eq + Hash, V: PartialEq> PartialEq for CowHashMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        // Sharing the same allocation is equality without a walk.
        Arc::ptr_eq(&self.inner, &other.inner) || *self.inner == *other.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: a snapshot must not walk the map, and must not
    /// see writes made after it was taken.
    #[test]
    fn snapshot_is_shared_until_a_write_splits_it() {
        let mut live: CowHashMap<String, Vec<String>> = CowHashMap::new();
        live.insert("_comps".into(), vec!["git".into()]);

        let snap = live.clone();
        assert!(live.is_shared(), "clone must share, not copy");

        // Subshell body writes: the split happens here, not at clone.
        live.insert("added_in_subshell".into(), vec![]);
        assert!(!live.is_shared(), "make_mut must have split the map");

        assert!(snap.get("added_in_subshell").is_none());
        assert_eq!(snap.get("_comps").map(Vec::len), Some(1));
        assert_eq!(live.len(), 2);
    }

    /// Restoring the snapshot must undo the subshell's writes, which is
    /// what `$( … )` does at its tail.
    #[test]
    fn restoring_a_snapshot_drops_the_subshell_writes() {
        let mut live: CowHashMap<String, String> = CowHashMap::new();
        live.insert("keep".into(), "outer".into());
        let snap = live.clone();

        live.insert("keep".into(), "inner".into());
        live.insert("leaked".into(), "inner".into());
        live.remove("nothing");

        live = snap;
        assert_eq!(live.get("keep").map(String::as_str), Some("outer"));
        assert!(live.get("leaked").is_none());
    }

    /// Mutating through the snapshot handle must not reach back into the
    /// live map either — the split is symmetric.
    #[test]
    fn writing_through_the_snapshot_does_not_reach_the_live_map() {
        let mut live: CowHashMap<String, String> = CowHashMap::new();
        live.insert("k".into(), "live".into());
        let mut snap = live.clone();

        snap.insert("k".into(), "snap".into());

        assert_eq!(live.get("k").map(String::as_str), Some("live"));
        assert_eq!(snap.get("k").map(String::as_str), Some("snap"));
    }
}
