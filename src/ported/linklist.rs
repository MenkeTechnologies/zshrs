//! Linked list implementation for zshrs
//!
//! Direct port from zsh/Src/linklist.c
//!
//! Get an empty linked list header                                          // c:99
//! Insert a node in a linked list after a given node                       // c:129
//! Remove a node from a linked list                                        // c:247
//! Free a linked list                                                      // c:283
//! Count the number of nodes in a linked list                              // c:300
//!
//! Provides the canonical `LinkList<T>` used everywhere a C source line
//! takes a `LinkList`. Backed by `VecDeque<T>` so index-based access used
//! by `Src/subst.c` walks (`firstnode` / `nextnode` / `incnode` /
//! `getdata` / `setdata`) is O(1) — same big-O as C's pointer walk over
//! `linknode->next`.
//!
//! Mirrors `struct linklist` from `Src/zsh.h:563` — `first` / `last` /
//! `flags`. Rust folds `first`/`last` into the `VecDeque`'s head/tail
//! pointers; the `flags` field is preserved as `u32`. Subst.c sets
//! `LF_ARRAY` (`Src/subst.c:33`) on the flag word.

use std::collections::VecDeque;

// ===========================================================
// Free-fn ports of `Src/linklist.c` (functions, not macros).
// ===========================================================

// Get an empty linked list header                                         // c:116
/// Port of `newlinklist()` (`Src/linklist.c:103`).
pub fn newlinklist() -> LinkList<String> {
    // c:103
    LinkList::new()
}

impl<T> Default for LinkList<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LinkList<T> {
    // Get an empty linked list header                                        // c:99
    /// Port of `znewlinklist()` from Src/linklist.c:116 — heap-arena
    /// fresh empty list. Rust uses `LinkList::new()`.
    pub fn new() -> Self {
        // c:116
        LinkList {
            nodes: VecDeque::new(),
            flags: 0,
        }
    }

    /// Port of the C macro `empty(list)` (`Src/zsh.h:583`) —
    /// `firstnode(list) == NULL`.
    pub fn is_empty(&self) -> bool {
        // c:zsh.h:583
        self.nodes.is_empty()
    }

    // Count the number of nodes in a linked list                             // c:300
    /// Port of `countlinknodes(LinkList list)` from Src/linklist.c:304.
    pub fn len(&self) -> usize {
        // c:304
        self.nodes.len()
    }

    /// Push at the head. Port of the C macro `pushnode()` (`Src/zsh.h`).
    pub fn push_front(&mut self, data: T) {
        // c:151
        self.nodes.push_front(data);
    }

    /// Push at the tail. Port of `addlinknode()` (`Src/zsh.h`) /
    /// `zaddlinknode()` (`Src/linklist.c:151`).
    pub fn push_back(&mut self, data: T) {
        // c:151
        self.nodes.push_back(data);
    }

    /// Pop the head. Port of `getlinknode(LinkList list)` (`Src/linklist.c:210`).
    pub fn pop_front(&mut self) -> Option<T> {
        // c:210
        self.nodes.pop_front()
    }

    /// Pop the tail. Port of `remnode(list, lastnode(list))` idiom.
    pub fn pop_back(&mut self) -> Option<T> {
        // c:251
        self.nodes.pop_back()
    }

    /// Front-element ref, equivalent to `firstnode(list)->dat`
    /// (`Src/zsh.h:576,586`).
    pub fn front(&self) -> Option<&T> {
        self.nodes.front()
    }
    /// `front_mut` — see implementation.
    pub fn front_mut(&mut self) -> Option<&mut T> {
        self.nodes.front_mut()
    }

    /// Back-element ref, equivalent to `lastnode(list)->dat`
    /// (`Src/zsh.h:577,586`).
    pub fn back(&self) -> Option<&T> {
        self.nodes.back()
    }
    /// `back_mut` — see implementation.
    pub fn back_mut(&mut self) -> Option<&mut T> {
        self.nodes.back_mut()
    }
    /// `iter` — see implementation.
    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, T> {
        self.nodes.iter()
    }
    /// `iter_mut` — see implementation.
    pub fn iter_mut(&mut self) -> std::collections::vec_deque::IterMut<'_, T> {
        self.nodes.iter_mut()
    }

    /// Append `other` onto the tail; drains `other`. Port of
    /// `joinlists()` (`Src/linklist.c:360`).
    pub fn append(&mut self, other: &mut LinkList<T>) {
        // c:360
        self.nodes.append(&mut other.nodes);
    }

    /// Drop every node. Port of `freelinklist(list, NULL)`
    /// (`Src/linklist.c:287`).
    pub fn clear(&mut self) {
        // c:287
        self.nodes.clear();
    }
    /// `to_vec` — see implementation.
    pub fn to_vec(self) -> Vec<T>
    where
        T: Clone,
    {
        self.nodes.into_iter().collect()
    }

    // ===== C-macro accessors (Src/zsh.h:576-590) =====

    /// Port of `firstnode(X)` macro (`Src/zsh.h:576`) — head node
    /// handle. Rust uses `usize` indices since the `VecDeque` backing
    /// gives O(1) random access matching C's pointer walk.
    pub fn firstnode(&self) -> Option<usize> {
        // c:zsh.h:576
        if self.nodes.is_empty() {
            None
        } else {
            Some(0)
        }
    }

    /// Port of `lastnode(X)` macro (`Src/zsh.h:577`).
    pub fn lastnode(&self) -> Option<usize> {
        // c:zsh.h:577
        if self.nodes.is_empty() {
            None
        } else {
            Some(self.nodes.len() - 1)
        }
    }

    /// Port of `nextnode(X)` macro (`Src/zsh.h:588`).
    pub fn nextnode(&self, idx: usize) -> Option<usize> {
        // c:zsh.h:588
        if idx + 1 < self.nodes.len() {
            Some(idx + 1)
        } else {
            None
        }
    }

    /// Port of `prevnode(X)` macro (`Src/zsh.h:589`).
    pub fn prevnode(&self, idx: usize) -> Option<usize> {
        // c:zsh.h:589
        if idx > 0 && idx <= self.nodes.len() {
            Some(idx - 1)
        } else {
            None
        }
    }

    /// Port of `getdata(X)` macro (`Src/zsh.h:586`).
    pub fn getdata(&self, idx: usize) -> Option<&T> {
        // c:zsh.h:586
        self.nodes.get(idx)
    }

    /// Port of `setdata(X,Y)` macro (`Src/zsh.h:587`).
    pub fn setdata(&mut self, idx: usize, data: T) {
        // c:zsh.h:587
        if let Some(slot) = self.nodes.get_mut(idx) {
            *slot = data;
        }
    }

    /// Port of `empty(X)` macro (`Src/zsh.h:583`).
    pub fn empty(&self) -> bool {
        // c:zsh.h:583
        self.nodes.is_empty()
    }

    /// Port of `insertlinknode(list, after, dat)` macro
    /// (`Src/zsh.h:580`) and the function form (`Src/linklist.c:133`)
    /// — insert after the supplied node index, return the index of the
    /// inserted node.
    /// WARNING: param names don't match C — Rust=(after_idx, data) vs C=(list, node, dat)
    pub fn insertlinknode(&mut self, after_idx: usize, data: T) -> usize {
        // c:linklist.c:133
        let new_idx = after_idx + 1;
        if new_idx >= self.nodes.len() {
            self.nodes.push_back(data);
            self.nodes.len() - 1
        } else {
            self.nodes.insert(new_idx, data);
            new_idx
        }
    }

    /// Remove + free a node. Port of `remnode(LinkList list, LinkNode nd)` (`Src/linklist.c:251`).
    pub fn delete_node(&mut self, idx: usize) -> Option<T> {
        // c:251
        self.nodes.remove(idx)
    }

    /// Port of `pushlinknode(list, val)` head-insert helper.
    pub fn insert_at(&mut self, idx: usize, data: T) {
        if idx >= self.nodes.len() {
            self.nodes.push_back(data);
        } else {
            self.nodes.insert(idx, data);
        }
    }
}

impl<T: Clone> Clone for LinkList<T> {
    fn clone(&self) -> Self {
        LinkList {
            nodes: self.nodes.clone(),
            flags: self.flags,
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for LinkList<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkList")
            .field("nodes", &self.nodes)
            .field("flags", &self.flags)
            .finish()
    }
}

impl<T> FromIterator<T> for LinkList<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut list = LinkList::new();
        for item in iter {
            list.push_back(item);
        }
        list
    }
}

impl<T> IntoIterator for LinkList<T> {
    type Item = T;
    type IntoIter = std::collections::vec_deque::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.nodes.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a LinkList<T> {
    type Item = &'a T;
    type IntoIter = std::collections::vec_deque::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.nodes.iter()
    }
}

/// Port of `znewlinklist()` (`Src/linklist.c:116`).
pub fn znewlinklist() -> LinkList<String> {
    // c:116
    LinkList::new()
}

// Insert a node in a linked list after a given node                       // c:151
/// Port of `insertlinknode(LinkList list, LinkNode node, void *dat)` (`Src/linklist.c:133`).
pub fn insertlinknode<T>(list: &mut LinkList<T>, node: usize, dat: T) -> usize {
    // c:133
    list.insertlinknode(node, dat)
}

/// Port of `zinsertlinknode(LinkList list, LinkNode node, void *dat)` (`Src/linklist.c:151`).
pub fn zinsertlinknode<T>(list: &mut LinkList<T>, node: usize, dat: T) -> usize {
    list.insertlinknode(node, dat)
}

/// Port of `uinsertlinknode(LinkList list, LinkNode node, LinkNode new)` (`Src/linklist.c:173`).
pub fn uinsertlinknode(list: &mut LinkList<String>, node: usize, new: String) -> Option<usize> {
    if list.iter().any(|s| s == &new) {
        None
    } else {
        Some(list.insertlinknode(node, new))
    }
}

// Insert a list in another list                                           // c:210
/// Port of `insertlinklist(LinkList l, LinkNode where, LinkList x)` from
/// `Src/linklist.c:190`. **C semantics: `l` is the SOURCE list, `where`
/// is the position in DEST list `x`, and `x` is the DESTINATION**. All
/// nodes of `l` get spliced into `x` right after node `where` —
/// equivalent to inserting the contents of `l` between `where` and
/// `where->next` in `x`. Empty `l` is a no-op (c:194 `if (!firstnode(l))
/// return;`). Param names + positions match C exactly so callers
/// reading `insertlinklist(sub.in, lastnode(result->in), result->in)`
/// (the canonical zutil.c:1324 pattern) translate 1:1.
pub fn insertlinklist<T: Clone>(
    // c:190
    l: &LinkList<T>,
    where_idx: usize,
    x: &mut LinkList<T>,
) {
    if l.is_empty() {
        // c:194
        return;
    }
    let mut idx = where_idx;
    for v in l.iter() {
        // c:196 walk l, splice into x
        idx = x.insertlinknode(idx, v.clone());
    }
}

// Pop the top node off a linked list and free it.                         // c:210
/// Port of `getlinknode(LinkList list)` (`Src/linklist.c:210`).
pub fn getlinknode<T>(list: &mut LinkList<T>) -> Option<T> {
    // c:210
    list.pop_front()
}

// Pop the top node off a linked list without freeing it.                  // c:251
/// Port of `ugetnode(LinkList list)` (`Src/linklist.c:231`).
pub fn ugetnode<T>(list: &mut LinkList<T>) -> Option<T> {
    // c:231
    list.pop_front()
}

// Remove a node from a linked list                                        // c:270
/// Port of `remnode(LinkList list, LinkNode nd)` (`Src/linklist.c:251`).
pub fn remnode<T>(list: &mut LinkList<T>, nd: usize) -> Option<T> {
    // c:251
    list.delete_node(nd)
}

/// Port of `uremnode(LinkList list, LinkNode nd)` (`Src/linklist.c:270`).
pub fn uremnode<T>(list: &mut LinkList<T>, nd: usize) -> Option<T> {
    // c:270
    list.delete_node(nd)
}

// Free a linked list                                                       // c:304
/// Port of `freelinklist(LinkList list, FreeFunc freefunc)` (`Src/linklist.c:287`).
/// WARNING: param names don't match C — Rust=(list) vs C=(list, freefunc)
pub fn freelinklist<T>(list: &mut LinkList<T>) {
    // c:287
    list.clear();
}

// Count the number of nodes in a linked list                              // c:317
/// Port of `countlinknodes(LinkList list)` (`Src/linklist.c:304`).
pub fn countlinknodes<T>(list: &LinkList<T>) -> usize {
    // c:304
    list.len()
}

// Make specified node first, moving preceding nodes to end                // c:317
/// Port of `rolllist(LinkList l, LinkNode nd)` (`Src/linklist.c:317`).
pub fn rolllist<T>(l: &mut LinkList<T>, nd: usize) {
    // c:317
    let len = l.len();
    if len > 0 {
        let nd = nd % len;
        for _ in 0..nd {
            if let Some(v) = l.pop_front() {
                l.push_back(v);
            }
        }
    }
}

// Create linklist of specified size. node->dats are not initialized.      // c:331
/// Port of `newsizedlist(int size)` from `Src/linklist.c:331-348`.
///
/// C body allocates a header + `size` pre-linked placeholder nodes
/// with uninitialized data; the C `for` loop wires prev/next
/// pointers (c:339-341). Callers iterate and fill data into each
/// slot.
///
/// The previous Rust port returned an empty list (ignoring `size`),
/// so any caller expecting `size` placeholder slots would iterate
/// over nothing. Fix by pushing `size` default-constructed nodes.
pub fn newsizedlist<T: Default>(size: usize) -> LinkList<T> {
    // c:331
    let mut list = LinkList::new();
    for _ in 0..size {
        // c:339-341
        list.push_back(T::default());
    }
    list
}

/// Port of `joinlists(LinkList first, LinkList second)` (`Src/linklist.c:360`).
pub fn joinlists<T>(first: &mut LinkList<T>, second: &mut LinkList<T>) {
    // c:360
    first.append(second);
}

/// Port of `linknodebydatum(LinkList list, void *dat)` (`Src/linklist.c:386`).
pub fn linknodebydatum<T: PartialEq>(list: &LinkList<T>, dat: &T) -> Option<usize> {
    // c:386
    list.iter().position(|v| v == dat)
}

/// Port of `linknodebystring(LinkList list, char *dat)` (`Src/linklist.c:403`).
pub fn linknodebystring(list: &LinkList<String>, dat: &str) -> Option<usize> {
    // c:403
    list.iter().position(|v| v == dat)
}

/// Convert a linked list of strings to a `Vec`. Port of
/// `hlinklist2array()` (`Src/linklist.c:423`).
pub fn hlinklist2array(list: &LinkList<String>) -> Vec<String> {
    // c:423
    list.iter().cloned().collect()
}

/// Port of `zlinklist2array(LinkList list, int copy)` (`Src/linklist.c:449`).
/// WARNING: param names don't match C — Rust=(list) vs C=(list, copy)
pub fn zlinklist2array(list: &LinkList<String>) -> Vec<String> {
    // c:449
    list.iter().cloned().collect()
}

/// A doubly-ended list, port of `struct linklist` (`Src/zsh.h:563`).
/// `flags` carries `LF_ARRAY` and friends from `Src/subst.c:33`.
pub struct LinkList<T> {
    pub nodes: VecDeque<T>, // c:zsh.h:565,566
    pub flags: u32,         // c:zsh.h:567
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_list() {
        let _g = crate::test_util::global_state_lock();
        let list: LinkList<i32> = LinkList::new();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        assert_eq!(list.flags, 0);
    }

    /// Pin `newsizedlist(N)` to canonical C body at
    /// `Src/linklist.c:339-341`: pre-allocates N placeholder nodes
    /// with uninitialized data, ready for callers to fill in.
    /// The previous Rust port returned an empty list, ignoring `size`.
    #[test]
    fn newsizedlist_preallocates_n_slots() {
        let _g = crate::test_util::global_state_lock();
        let list: LinkList<i32> = newsizedlist(5);
        assert_eq!(
            list.len(),
            5,
            "c:339-341 — newsizedlist(5) must pre-allocate 5 nodes"
        );
        // Default-constructed i32 is 0; every slot ready for assign.
        for v in list.iter() {
            assert_eq!(*v, 0, "pre-allocated slots default to 0");
        }

        let zero_list: LinkList<String> = newsizedlist(0);
        assert_eq!(zero_list.len(), 0, "newsizedlist(0) is the same as new()");
    }

    #[test]
    fn test_push_front_back() {
        let _g = crate::test_util::global_state_lock();
        let mut list = LinkList::new();
        list.push_back(1);
        list.push_back(2);
        list.push_front(0);
        assert_eq!(list.front(), Some(&0));
        assert_eq!(list.back(), Some(&2));
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn test_pop_front_back() {
        let _g = crate::test_util::global_state_lock();
        let mut list = LinkList::new();
        list.push_back(1);
        list.push_back(2);
        list.push_back(3);
        assert_eq!(list.pop_front(), Some(1));
        assert_eq!(list.pop_back(), Some(3));
        assert_eq!(list.pop_front(), Some(2));
        assert_eq!(list.pop_front(), None);
    }

    #[test]
    fn test_iter() {
        let _g = crate::test_util::global_state_lock();
        let mut list = LinkList::new();
        list.push_back(1);
        list.push_back(2);
        list.push_back(3);
        let v: Vec<_> = list.iter().copied().collect();
        assert_eq!(v, vec![1, 2, 3]);
    }

    #[test]
    fn test_macro_methods() {
        let _g = crate::test_util::global_state_lock();
        let mut list: LinkList<String> = LinkList::new();
        list.push_back("a".to_string());
        list.push_back("b".to_string());
        list.push_back("c".to_string());

        assert_eq!(list.firstnode(), Some(0));
        assert_eq!(list.lastnode(), Some(2));
        assert_eq!(list.nextnode(0), Some(1));
        assert_eq!(list.nextnode(2), None);
        assert_eq!(list.getdata(1).map(String::as_str), Some("b"));
        list.setdata(1, "B".to_string());
        assert_eq!(list.getdata(1).map(String::as_str), Some("B"));
        let new_idx = list.insertlinknode(1, "X".to_string());
        assert_eq!(new_idx, 2);
        assert_eq!(list.getdata(2).map(String::as_str), Some("X"));
        assert_eq!(list.delete_node(2).as_deref(), Some("X"));
        assert_eq!(list.getdata(2).map(String::as_str), Some("c"));
    }

    #[test]
    fn test_append() {
        let _g = crate::test_util::global_state_lock();
        let mut a: LinkList<i32> = vec![1, 2].into_iter().collect();
        let mut b: LinkList<i32> = vec![3, 4].into_iter().collect();
        a.append(&mut b);
        assert!(b.is_empty());
        assert_eq!(a.to_vec(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_clear() {
        let _g = crate::test_util::global_state_lock();
        let mut list: LinkList<i32> = vec![1, 2, 3].into_iter().collect();
        list.clear();
        assert!(list.is_empty());
    }

    #[test]
    fn test_uinsertlinknode_dedups() {
        let _g = crate::test_util::global_state_lock();
        let mut list: LinkList<String> = LinkList::new();
        list.push_back("a".to_string());
        assert!(uinsertlinknode(&mut list, 0, "b".to_string()).is_some());
        assert!(uinsertlinknode(&mut list, 0, "a".to_string()).is_none());
        assert_eq!(list.len(), 2);
    }

    /// c:360 — `joinlists(first, second)` moves all of `second` onto
    /// the end of `first`, draining second. A regression where second
    /// isn't drained would let the caller iterate doubled entries.
    #[test]
    fn joinlists_drains_second_into_first() {
        let _g = crate::test_util::global_state_lock();
        let mut a: LinkList<i32> = vec![1, 2].into_iter().collect();
        let mut b: LinkList<i32> = vec![3, 4, 5].into_iter().collect();
        joinlists(&mut a, &mut b);
        assert_eq!(a.to_vec(), vec![1, 2, 3, 4, 5]);
        assert!(b.is_empty(), "second list must be drained after join");
    }

    /// c:360 — joining an empty `second` is a no-op. Catches a
    /// regression that adds phantom empty sentinels.
    #[test]
    fn joinlists_empty_second_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let mut a: LinkList<i32> = vec![1, 2].into_iter().collect();
        let mut b: LinkList<i32> = LinkList::new();
        joinlists(&mut a, &mut b);
        assert_eq!(a.to_vec(), vec![1, 2]);
        assert!(b.is_empty());
    }

    /// c:360 — joining INTO an empty `first` transfers second
    /// cleanly. The empty-head edge case in the C body has a
    /// dedicated branch — regression there would lose the data.
    #[test]
    fn joinlists_empty_first_receives_all_of_second() {
        let _g = crate::test_util::global_state_lock();
        let mut a: LinkList<i32> = LinkList::new();
        let mut b: LinkList<i32> = vec![1, 2, 3].into_iter().collect();
        joinlists(&mut a, &mut b);
        assert_eq!(a.to_vec(), vec![1, 2, 3]);
        assert!(b.is_empty());
    }

    /// c:386 — `linknodebydatum` returns Some(idx) for the first
    /// matching entry, None for miss. Used by `unhash -d` lookups.
    #[test]
    fn linknodebydatum_finds_first_match() {
        let _g = crate::test_util::global_state_lock();
        let list: LinkList<i32> = vec![10, 20, 30, 20].into_iter().collect();
        assert_eq!(
            linknodebydatum(&list, &20),
            Some(1),
            "must return FIRST match index"
        );
        assert_eq!(linknodebydatum(&list, &99), None);
    }

    /// c:403 — `linknodebystring` is the string-specialised variant.
    /// Verifies same FIRST-match contract for the alias-table walks.
    #[test]
    fn linknodebystring_finds_first_match() {
        let _g = crate::test_util::global_state_lock();
        let list: LinkList<String> = vec!["a".into(), "b".into(), "a".into()]
            .into_iter()
            .collect();
        assert_eq!(linknodebystring(&list, "a"), Some(0));
        assert_eq!(linknodebystring(&list, "b"), Some(1));
        assert_eq!(linknodebystring(&list, "x"), None);
    }

    /// c:423 — `hlinklist2array` flattens to Vec preserving order.
    /// Used by `${(@k)hash}` array materialisation.
    #[test]
    fn hlinklist2array_preserves_order() {
        let _g = crate::test_util::global_state_lock();
        let list: LinkList<String> = vec!["a".into(), "b".into(), "c".into()]
            .into_iter()
            .collect();
        let arr = hlinklist2array(&list);
        assert_eq!(arr, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    }

    /// `Src/linklist.c:302-311` — `countlinknodes(list)` walks the
    /// `next` chain incrementing a counter. Empty list → 0.
    #[test]
    fn countlinknodes_returns_len_for_arbitrary_lists() {
        let _g = crate::test_util::global_state_lock();
        let empty: LinkList<i32> = LinkList::new();
        assert_eq!(
            countlinknodes(&empty),
            0,
            "c:309 — empty list traversal yields 0"
        );
        let one: LinkList<i32> = vec![42].into_iter().collect();
        assert_eq!(countlinknodes(&one), 1);
        let many: LinkList<i32> = (0..100).collect();
        assert_eq!(countlinknodes(&many), 100);
    }

    /// `Src/linklist.c:316-325` — `rolllist(l, nd)` makes `nd` first,
    /// moving preceding nodes to end (circular rotation). The Rust
    /// port treats `nd` as a 0-indexed position to rotate to the
    /// front; rotate-by-0 is a no-op, rotate-by-N wraps via modulo.
    #[test]
    fn rolllist_rotates_to_index() {
        let _g = crate::test_util::global_state_lock();
        // c:319-324 — rotate so nd-th element becomes first.
        let mut list: LinkList<i32> = vec![10, 20, 30, 40].into_iter().collect();
        rolllist(&mut list, 2);
        assert_eq!(
            list.to_vec(),
            vec![30, 40, 10, 20],
            "c:321 — `list.first = nd` then preceding nodes append at end"
        );
    }

    /// c:316-325 — rolllist by 0 is the identity. Pin so an off-by-one
    /// regression doesn't silently rotate every caller by 1.
    #[test]
    fn rolllist_zero_index_is_identity() {
        let _g = crate::test_util::global_state_lock();
        let mut list: LinkList<i32> = vec![1, 2, 3].into_iter().collect();
        rolllist(&mut list, 0);
        assert_eq!(list.to_vec(), vec![1, 2, 3]);
    }

    /// c:316-325 — rolllist with index >= len wraps via modulo. Pins
    /// the implementation choice (C version is UB on out-of-range —
    /// Rust port chose modulo defensively).
    #[test]
    fn rolllist_wraps_index_modulo_length() {
        let _g = crate::test_util::global_state_lock();
        let mut list: LinkList<i32> = vec![1, 2, 3].into_iter().collect();
        // index 4 mod 3 == 1 → rotate by 1.
        rolllist(&mut list, 4);
        assert_eq!(list.to_vec(), vec![2, 3, 1]);
    }

    /// `Src/linklist.c:188-206` — `insertlinklist(l, where, x)` splices
    /// the contents of SOURCE list `l` into DESTINATION list `x` right
    /// after node `where`. Canonical caller pattern (per
    /// `Src/Modules/zutil.c:1324`):
    ///     `insertlinklist(sub.in, lastnode(result->in), result->in);`
    /// which appends every node from `sub.in` to the end of `result->in`.
    /// Pin C semantics: source unchanged, dest grows by source's length,
    /// inserted in the right span and in source order.
    #[test]
    fn insertlinklist_splices_source_into_dest_after_position() {
        let _g = crate::test_util::global_state_lock();
        // dest: [10, 20, 30], source: [A, B, C], where=0 (after first).
        // Expected: [10, A, B, C, 20, 30] — source appears AFTER 10.
        let source: LinkList<i32> = vec![100, 200, 300].into_iter().collect();
        let mut dest: LinkList<i32> = vec![10, 20, 30].into_iter().collect();
        insertlinklist(&source, 0, &mut dest);
        assert_eq!(
            dest.to_vec(),
            vec![10, 100, 200, 300, 20, 30],
            "c:194-202 — source spliced into dest after node 0"
        );
        assert_eq!(
            source.to_vec(),
            vec![100, 200, 300],
            "c:188-206 — source list is NOT modified (read-only)"
        );
    }

    /// `Src/linklist.c:193-194` — `if (!firstnode(l)) return;` — empty
    /// source is a no-op. Pins so a regression doesn't accidentally
    /// insert a phantom sentinel.
    #[test]
    fn insertlinklist_empty_source_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let source: LinkList<i32> = LinkList::new();
        let mut dest: LinkList<i32> = vec![1, 2, 3].into_iter().collect();
        insertlinklist(&source, 1, &mut dest);
        assert_eq!(
            dest.to_vec(),
            vec![1, 2, 3],
            "c:193-194 — empty l returns early; dest unchanged"
        );
    }

    /// `Src/linklist.c:188-206` — canonical zutil.c:1324 pattern:
    /// `insertlinklist(sub.in, lastnode(result->in), result->in)` —
    /// append entire source list at end of dest. The Rust port's
    /// `lastnode_index()` is `len()-1`; passing that as `where_idx`
    /// inserts after the last node, producing dest++source.
    #[test]
    fn insertlinklist_lastnode_append_pattern() {
        let _g = crate::test_util::global_state_lock();
        let source: LinkList<&str> = vec!["x", "y"].into_iter().collect();
        let mut dest: LinkList<&str> = vec!["a", "b", "c"].into_iter().collect();
        let last = dest.len() - 1;
        insertlinklist(&source, last, &mut dest);
        assert_eq!(
            dest.to_vec(),
            vec!["a", "b", "c", "x", "y"],
            "c:188-206 zutil.c:1324 — lastnode anchor → tail-append"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Round-7: LinkList edge cases — pop on empty, countlinknodes,
    // joinlists, linknodebydatum/string, ugetnode, hlinklist2array.
    // ═══════════════════════════════════════════════════════════════════

    /// `pop_front` / `pop_back` on empty list → None (no panic).
    #[test]
    fn linklist_pop_on_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let mut list: LinkList<i32> = LinkList::new();
        assert_eq!(list.pop_front(), None);
        assert_eq!(list.pop_back(), None);
    }

    /// `front` / `back` on empty list → None.
    #[test]
    fn linklist_front_back_on_empty_return_none() {
        let _g = crate::test_util::global_state_lock();
        let list: LinkList<i32> = LinkList::new();
        assert!(list.front().is_none());
        assert!(list.back().is_none());
    }

    /// `countlinknodes` on empty → 0.
    #[test]
    fn countlinknodes_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let list: LinkList<i32> = LinkList::new();
        assert_eq!(countlinknodes(&list), 0);
    }

    /// `countlinknodes` matches `len()`.
    #[test]
    fn countlinknodes_matches_len_for_populated_list() {
        let _g = crate::test_util::global_state_lock();
        let mut list = LinkList::new();
        for i in 0..7 {
            list.push_back(i);
        }
        assert_eq!(countlinknodes(&list), 7);
        assert_eq!(countlinknodes(&list), list.len());
    }

    /// `joinlists`: second appended to first; second empties.
    #[test]
    fn joinlists_appends_second_to_first_and_empties_second() {
        let _g = crate::test_util::global_state_lock();
        let mut first: LinkList<i32> = LinkList::new();
        first.push_back(1);
        first.push_back(2);
        let mut second: LinkList<i32> = LinkList::new();
        second.push_back(3);
        second.push_back(4);

        joinlists(&mut first, &mut second);

        assert_eq!(first.len(), 4, "first must absorb second's elements");
        assert!(second.is_empty(), "second must be emptied after joinlists");
    }

    /// `joinlists` with empty second → first unchanged.
    #[test]
    fn joinlists_with_empty_second_leaves_first_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let mut first: LinkList<i32> = LinkList::new();
        first.push_back(1);
        first.push_back(2);
        let len_before = first.len();
        let mut second: LinkList<i32> = LinkList::new();

        joinlists(&mut first, &mut second);
        assert_eq!(first.len(), len_before);
    }

    /// `linknodebydatum` finds the first matching element (Some(idx))
    /// or None if absent.
    #[test]
    fn linknodebydatum_finds_existing_element() {
        let _g = crate::test_util::global_state_lock();
        let mut list = LinkList::new();
        list.push_back(10);
        list.push_back(20);
        list.push_back(30);
        assert!(linknodebydatum(&list, &20).is_some());
        assert!(linknodebydatum(&list, &99).is_none());
    }

    /// `linknodebystring` — same as datum but for &str.
    #[test]
    fn linknodebystring_finds_existing_string() {
        let _g = crate::test_util::global_state_lock();
        let mut list: LinkList<String> = LinkList::new();
        list.push_back("alpha".into());
        list.push_back("beta".into());
        list.push_back("gamma".into());
        assert!(linknodebystring(&list, "beta").is_some());
        assert!(linknodebystring(&list, "delta").is_none());
    }

    /// `hlinklist2array` converts to Vec<String> preserving order.
    #[test]
    fn hlinklist2array_preserves_insertion_order() {
        let _g = crate::test_util::global_state_lock();
        let mut list: LinkList<String> = LinkList::new();
        list.push_back("x".into());
        list.push_back("y".into());
        list.push_back("z".into());
        assert_eq!(
            hlinklist2array(&list),
            vec!["x".to_string(), "y".into(), "z".into()]
        );
    }

    /// `hlinklist2array` on empty list returns empty Vec.
    #[test]
    fn hlinklist2array_on_empty_returns_empty_vec() {
        let _g = crate::test_util::global_state_lock();
        let list: LinkList<String> = LinkList::new();
        let v = hlinklist2array(&list);
        assert!(v.is_empty());
    }

    /// `freelinklist` clears the list to length 0.
    #[test]
    fn freelinklist_empties_the_list() {
        let _g = crate::test_util::global_state_lock();
        let mut list = LinkList::new();
        for i in 0..5 {
            list.push_back(i);
        }
        assert_eq!(list.len(), 5);
        freelinklist(&mut list);
        assert!(list.is_empty());
    }

    /// `getlinknode` removes and returns the first element (head pop).
    #[test]
    fn getlinknode_pops_head_returning_value() {
        let _g = crate::test_util::global_state_lock();
        let mut list = LinkList::new();
        list.push_back(1);
        list.push_back(2);
        list.push_back(3);
        assert_eq!(getlinknode(&mut list), Some(1));
        assert_eq!(list.len(), 2, "head removed → len down by 1");
        assert_eq!(getlinknode(&mut list), Some(2));
        assert_eq!(getlinknode(&mut list), Some(3));
        assert_eq!(getlinknode(&mut list), None, "empty → None");
    }

    // ─── zsh-corpus pins ────────────────────────────────────────────

    /// `newlinklist()` returns empty list with countlinknodes = 0.
    #[test]
    fn linklist_corpus_new_is_empty() {
        let l = newlinklist();
        assert_eq!(countlinknodes(&l), 0);
        assert!(l.is_empty());
    }

    /// Push many items, count matches.
    #[test]
    fn linklist_corpus_count_after_many_pushes() {
        let mut l = LinkList::<i32>::new();
        for i in 0..50 {
            l.push_back(i);
        }
        assert_eq!(countlinknodes(&l), 50);
        assert_eq!(l.len(), 50);
    }

    /// `joinlists` concatenates: first grows, second empties.
    #[test]
    fn linklist_corpus_joinlists_concatenates() {
        let mut a = LinkList::<i32>::new();
        let mut b = LinkList::<i32>::new();
        a.push_back(1);
        a.push_back(2);
        b.push_back(3);
        b.push_back(4);
        joinlists(&mut a, &mut b);
        assert_eq!(a.len(), 4, "first holds union");
        assert!(b.is_empty(), "second emptied");
    }

    /// `getlinknode` on empty returns None.
    #[test]
    fn linklist_corpus_getlinknode_empty_returns_none() {
        let mut l = LinkList::<i32>::new();
        assert_eq!(getlinknode(&mut l), None);
    }

    /// `getlinknode` empties list element by element until None.
    #[test]
    fn linklist_corpus_getlinknode_drains_in_fifo_order() {
        let mut l = LinkList::<&'static str>::new();
        l.push_back("a");
        l.push_back("b");
        l.push_back("c");
        assert_eq!(getlinknode(&mut l), Some("a"));
        assert_eq!(getlinknode(&mut l), Some("b"));
        assert_eq!(getlinknode(&mut l), Some("c"));
        assert_eq!(getlinknode(&mut l), None);
        assert!(l.is_empty());
    }

    /// `linknodebydatum` finds element by value.
    #[test]
    fn linklist_corpus_linknodebydatum_finds_value() {
        let mut l = LinkList::<i32>::new();
        l.push_back(10);
        l.push_back(20);
        l.push_back(30);
        assert_eq!(linknodebydatum(&l, &20), Some(1));
        assert_eq!(linknodebydatum(&l, &99), None);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/linklist.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:30 — `newlinklist()` returns empty list.
    #[test]
    fn newlinklist_returns_empty_pin() {
        let l = newlinklist();
        assert!(l.is_empty());
        assert_eq!(l.len(), 0);
    }

    /// c:285 — `znewlinklist()` returns empty (permanent alloc).
    #[test]
    fn znewlinklist_returns_empty_pin() {
        let l = znewlinklist();
        assert!(l.is_empty());
    }

    /// c:375 — `countlinknodes` returns 0 on empty list.
    #[test]
    fn countlinknodes_empty_is_zero_pin() {
        let l = LinkList::<i32>::new();
        assert_eq!(countlinknodes(&l), 0);
    }

    /// c:375 — `countlinknodes` matches len on populated list.
    #[test]
    fn countlinknodes_matches_length_pin() {
        let mut l = LinkList::<i32>::new();
        l.push_back(1);
        l.push_back(2);
        l.push_back(3);
        assert_eq!(countlinknodes(&l), 3);
    }

    /// c:340 — `getlinknode` empty → None.
    #[test]
    fn getlinknode_empty_returns_none_pin() {
        let mut l = LinkList::<i32>::new();
        assert!(getlinknode(&mut l).is_none());
    }

    /// c:347 — `ugetnode` empty → None.
    #[test]
    fn ugetnode_empty_returns_none_pin() {
        let mut l = LinkList::<i32>::new();
        assert!(ugetnode(&mut l).is_none());
    }

    /// c:368 — `freelinklist` clears list.
    #[test]
    fn freelinklist_clears_list_pin() {
        let mut l = LinkList::<i32>::new();
        l.push_back(1);
        l.push_back(2);
        freelinklist(&mut l);
        assert!(l.is_empty());
    }

    /// c:429 — `linknodebystring` finds string.
    #[test]
    fn linknodebystring_finds_string_pin() {
        let mut l = LinkList::<String>::new();
        l.push_back("alpha".to_string());
        l.push_back("beta".to_string());
        l.push_back("gamma".to_string());
        assert_eq!(linknodebystring(&l, "beta"), Some(1));
        assert_eq!(linknodebystring(&l, "delta"), None);
    }

    /// c:429 — finds first match on duplicates.
    #[test]
    fn linknodebystring_first_match_pin() {
        let mut l = LinkList::<String>::new();
        l.push_back("x".to_string());
        l.push_back("x".to_string());
        assert_eq!(linknodebystring(&l, "x"), Some(0));
    }

    /// c:436 — `hlinklist2array` preserves order.
    #[test]
    fn hlinklist2array_preserves_order_pin() {
        let mut l = LinkList::<String>::new();
        l.push_back("a".to_string());
        l.push_back("b".to_string());
        l.push_back("c".to_string());
        let v = hlinklist2array(&l);
        assert_eq!(v, vec!["a", "b", "c"]);
    }

    /// c:436 — empty list → empty Vec.
    #[test]
    fn hlinklist2array_empty_returns_empty_vec_pin() {
        let l = LinkList::<String>::new();
        let v = hlinklist2array(&l);
        assert!(v.is_empty());
    }

    /// c:406 — `newsizedlist(0)` returns empty list.
    #[test]
    fn newsizedlist_zero_returns_empty_pin() {
        let l: LinkList<i32> = newsizedlist(0);
        assert!(l.is_empty());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/linklist.c
    // c:173 uinsertlinknode / c:317 rolllist / c:331 newsizedlist /
    // c:403 linknodebystring / c:423 hlinklist2array / c:449 zlinklist2array
    // ═══════════════════════════════════════════════════════════════════

    /// c:173 — `uinsertlinknode` returns None when dup already in list.
    #[test]
    fn uinsertlinknode_dup_returns_none() {
        let mut l: LinkList<String> = LinkList::new();
        l.push_back("a".to_string());
        l.push_back("b".to_string());
        let r = uinsertlinknode(&mut l, 0, "a".to_string());
        assert_eq!(r, None, "dup must return None");
        assert_eq!(l.len(), 2, "list size unchanged");
    }

    /// c:173 — `uinsertlinknode` returns Some(idx) when value is new.
    #[test]
    fn uinsertlinknode_new_returns_some() {
        let mut l: LinkList<String> = LinkList::new();
        l.push_back("a".to_string());
        let r = uinsertlinknode(&mut l, 0, "b".to_string());
        assert!(r.is_some(), "new value must return Some");
        assert_eq!(l.len(), 2, "list size incremented");
    }

    /// c:317 — `rolllist` on empty list is safe (no panic, no-op).
    #[test]
    fn rolllist_empty_no_panic() {
        let mut l: LinkList<i32> = LinkList::new();
        rolllist(&mut l, 5);
        assert!(l.is_empty());
    }

    /// c:317 — `rolllist` by length is identity (full cycle).
    #[test]
    fn rolllist_full_length_is_identity() {
        let mut l: LinkList<i32> = LinkList::new();
        for i in 1..=5 {
            l.push_back(i);
        }
        let before: Vec<i32> = l.iter().copied().collect();
        rolllist(&mut l, 5); // full rotation
        let after: Vec<i32> = l.iter().copied().collect();
        assert_eq!(before, after, "full rotation is identity");
    }

    /// c:331 — `newsizedlist(N)` returns exactly N default-constructed nodes.
    #[test]
    fn newsizedlist_n_returns_n_default_nodes() {
        for n in [1usize, 3, 10, 100] {
            let l: LinkList<i32> = newsizedlist(n);
            assert_eq!(l.len(), n, "newsizedlist({}) must return {} nodes", n, n);
            for v in l.iter() {
                assert_eq!(*v, 0, "default i32 = 0");
            }
        }
    }

    /// c:403 — `linknodebystring` returns None for non-existent string.
    #[test]
    fn linknodebystring_missing_returns_none_pin() {
        let mut l: LinkList<String> = LinkList::new();
        l.push_back("apple".to_string());
        l.push_back("banana".to_string());
        assert_eq!(linknodebystring(&l, "cherry"), None);
    }

    /// c:403 — `linknodebystring` is case-sensitive.
    #[test]
    fn linknodebystring_case_sensitive() {
        let mut l: LinkList<String> = LinkList::new();
        l.push_back("APPLE".to_string());
        assert_eq!(linknodebystring(&l, "apple"), None,
            "case mismatch must miss");
        assert!(linknodebystring(&l, "APPLE").is_some(),
            "exact case match");
    }

    /// c:386 — `linknodebydatum` for absent value returns None.
    #[test]
    fn linknodebydatum_missing_returns_none_pin() {
        let mut l: LinkList<i32> = LinkList::new();
        for i in [1, 2, 3] {
            l.push_back(i);
        }
        assert_eq!(linknodebydatum(&l, &99), None);
    }

    /// c:449 — `zlinklist2array` agrees with `hlinklist2array` on
    /// the same input (both produce identical Vec).
    #[test]
    fn zlinklist2array_matches_hlinklist2array() {
        let mut l: LinkList<String> = LinkList::new();
        for s in ["a", "b", "c", "d"] {
            l.push_back(s.to_string());
        }
        let h = hlinklist2array(&l);
        let z = zlinklist2array(&l);
        assert_eq!(h, z, "h and z variants must agree");
    }

    /// c:386 + c:403 — both lookup fns are deterministic.
    #[test]
    fn lookup_fns_are_deterministic() {
        let mut l: LinkList<String> = LinkList::new();
        for s in ["a", "b", "c"] {
            l.push_back(s.to_string());
        }
        let a = linknodebystring(&l, "b");
        for _ in 0..5 {
            assert_eq!(linknodebystring(&l, "b"), a);
        }
        let mut ll: LinkList<i32> = LinkList::new();
        for i in [1, 2, 3] {
            ll.push_back(i);
        }
        let b = linknodebydatum(&ll, &2);
        for _ in 0..5 {
            assert_eq!(linknodebydatum(&ll, &2), b);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/linklist.c
    // c:292 insertlinknode / c:340 getlinknode / c:354 remnode /
    // c:368 freelinklist / c:375 countlinknodes / c:382 rolllist /
    // c:406 newsizedlist / c:417 joinlists
    // ═══════════════════════════════════════════════════════════════════

    /// c:340 — `getlinknode` returns Option<T> (compile-time type pin).
    #[test]
    fn getlinknode_returns_option_t_type() {
        let mut l: LinkList<i32> = LinkList::new();
        let _: Option<i32> = getlinknode(&mut l);
    }

    /// c:340 — `getlinknode` on empty list returns None.
    #[test]
    fn getlinknode_empty_returns_none() {
        let mut l: LinkList<i32> = LinkList::new();
        assert!(getlinknode(&mut l).is_none());
    }

    /// c:340 — `getlinknode` drains list to empty.
    #[test]
    fn getlinknode_drains_list_to_empty() {
        let mut l: LinkList<i32> = LinkList::new();
        for i in 1..=5 {
            l.push_back(i);
        }
        for _ in 0..5 {
            assert!(getlinknode(&mut l).is_some());
        }
        assert!(getlinknode(&mut l).is_none(), "after 5 pops → None");
        assert!(l.is_empty());
    }

    /// c:368 — `freelinklist` empties the list.
    #[test]
    fn freelinklist_empties_list() {
        let mut l: LinkList<i32> = LinkList::new();
        for i in 1..=10 {
            l.push_back(i);
        }
        freelinklist(&mut l);
        assert!(l.is_empty(), "freelinklist empties");
    }

    /// c:375 — `countlinknodes` returns usize.
    #[test]
    fn countlinknodes_returns_usize_type() {
        let l: LinkList<i32> = LinkList::new();
        let _: usize = countlinknodes(&l);
    }

    /// c:375 — `countlinknodes(empty)` returns 0.
    #[test]
    fn countlinknodes_empty_returns_zero_pin() {
        let l: LinkList<i32> = LinkList::new();
        assert_eq!(countlinknodes(&l), 0);
    }

    /// c:417 — `joinlists` is associative: (a+b)+c == a+(b+c) on len.
    #[test]
    fn joinlists_associative_on_length() {
        let make = |xs: &[i32]| -> LinkList<i32> {
            let mut l = LinkList::new();
            for &x in xs {
                l.push_back(x);
            }
            l
        };
        // (a + b) + c
        let mut a1 = make(&[1, 2]);
        let mut b1 = make(&[3, 4]);
        let mut c1 = make(&[5, 6]);
        joinlists(&mut a1, &mut b1);
        joinlists(&mut a1, &mut c1);

        // a + (b + c)
        let mut a2 = make(&[1, 2]);
        let mut b2 = make(&[3, 4]);
        let mut c2 = make(&[5, 6]);
        joinlists(&mut b2, &mut c2);
        joinlists(&mut a2, &mut b2);

        assert_eq!(a1.len(), a2.len(),
            "joinlists associative: (a+b)+c.len = a+(b+c).len");
    }

    /// c:382 — `rolllist` on 1-element list is no-op for any index.
    #[test]
    fn rolllist_single_element_is_noop() {
        let mut l: LinkList<i32> = LinkList::new();
        l.push_back(42);
        for n in [0usize, 1, 5, 100] {
            rolllist(&mut l, n);
            assert_eq!(l.len(), 1);
        }
    }

    /// c:406 — `newsizedlist::<i32>(N)` returns Vec of N zeros.
    #[test]
    fn newsizedlist_i32_fills_with_default_zero() {
        for n in [1usize, 3, 10] {
            let l: LinkList<i32> = newsizedlist(n);
            assert_eq!(l.len(), n);
            for v in l.iter() {
                assert_eq!(*v, 0, "i32 default = 0");
            }
        }
    }

    /// c:417 — `joinlists` with second containing one element appends.
    #[test]
    fn joinlists_single_elem_second_appends() {
        let mut a: LinkList<i32> = LinkList::new();
        a.push_back(1);
        a.push_back(2);
        let mut b: LinkList<i32> = LinkList::new();
        b.push_back(3);
        joinlists(&mut a, &mut b);
        assert_eq!(a.len(), 3);
        assert!(b.is_empty(), "second emptied after join");
    }
}
