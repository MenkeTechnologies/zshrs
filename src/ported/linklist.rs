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

/// A doubly-ended list, port of `struct linklist` (`Src/zsh.h:563`).
/// `flags` carries `LF_ARRAY` and friends from `Src/subst.c:33`.
pub struct LinkList<T> {
    pub nodes: VecDeque<T>,                                                 // c:zsh.h:565,566
    pub flags: u32,                                                          // c:zsh.h:567
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
    pub fn new() -> Self {                                                      // c:116
        LinkList { nodes: VecDeque::new(), flags: 0 }
    }

    /// Port of the C macro `empty(list)` (`Src/zsh.h:583`) —
    /// `firstnode(list) == NULL`.
    pub fn is_empty(&self) -> bool {                                            // c:zsh.h:583
        self.nodes.is_empty()
    }

    // Count the number of nodes in a linked list                             // c:300
    /// Port of `countlinknodes(LinkList list)` from Src/linklist.c:304.
    pub fn len(&self) -> usize {                                                // c:304
        self.nodes.len()
    }

    /// Push at the head. Port of the C macro `pushnode()` (`Src/zsh.h`).
    pub fn push_front(&mut self, data: T) {                                     // c:151
        self.nodes.push_front(data);
    }

    /// Push at the tail. Port of `addlinknode()` (`Src/zsh.h`) /
    /// `zaddlinknode()` (`Src/linklist.c:151`).
    pub fn push_back(&mut self, data: T) {                                      // c:151
        self.nodes.push_back(data);
    }

    /// Pop the head. Port of `getlinknode(LinkList list)` (`Src/linklist.c:210`).
    pub fn pop_front(&mut self) -> Option<T> {                                  // c:210
        self.nodes.pop_front()
    }

    /// Pop the tail. Port of `remnode(list, lastnode(list))` idiom.
    pub fn pop_back(&mut self) -> Option<T> {                                   // c:251
        self.nodes.pop_back()
    }

    /// Front-element ref, equivalent to `firstnode(list)->dat`
    /// (`Src/zsh.h:576,586`).
    pub fn front(&self) -> Option<&T> {
        self.nodes.front()
    }

    pub fn front_mut(&mut self) -> Option<&mut T> {
        self.nodes.front_mut()
    }

    /// Back-element ref, equivalent to `lastnode(list)->dat`
    /// (`Src/zsh.h:577,586`).
    pub fn back(&self) -> Option<&T> {
        self.nodes.back()
    }

    pub fn back_mut(&mut self) -> Option<&mut T> {
        self.nodes.back_mut()
    }

    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, T> {
        self.nodes.iter()
    }

    pub fn iter_mut(&mut self) -> std::collections::vec_deque::IterMut<'_, T> {
        self.nodes.iter_mut()
    }

    /// Append `other` onto the tail; drains `other`. Port of
    /// `joinlists()` (`Src/linklist.c:360`).
    pub fn append(&mut self, other: &mut LinkList<T>) {                         // c:360
        self.nodes.append(&mut other.nodes);
    }

    /// Drop every node. Port of `freelinklist(list, NULL)`
    /// (`Src/linklist.c:287`).
    pub fn clear(&mut self) {                                                   // c:287
        self.nodes.clear();
    }

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
    pub fn firstnode(&self) -> Option<usize> {                                  // c:zsh.h:576
        if self.nodes.is_empty() { None } else { Some(0) }
    }

    /// Port of `lastnode(X)` macro (`Src/zsh.h:577`).
    pub fn lastnode(&self) -> Option<usize> {                                   // c:zsh.h:577
        if self.nodes.is_empty() { None } else { Some(self.nodes.len() - 1) }
    }

    /// Port of `nextnode(X)` macro (`Src/zsh.h:588`).
    pub fn nextnode(&self, idx: usize) -> Option<usize> {                       // c:zsh.h:588
        if idx + 1 < self.nodes.len() { Some(idx + 1) } else { None }
    }

    /// Port of `prevnode(X)` macro (`Src/zsh.h:589`).
    pub fn prevnode(&self, idx: usize) -> Option<usize> {                       // c:zsh.h:589
        if idx > 0 && idx <= self.nodes.len() { Some(idx - 1) } else { None }
    }

    /// Port of `getdata(X)` macro (`Src/zsh.h:586`).
    pub fn getdata(&self, idx: usize) -> Option<&T> {                           // c:zsh.h:586
        self.nodes.get(idx)
    }

    /// Port of `setdata(X,Y)` macro (`Src/zsh.h:587`).
    pub fn setdata(&mut self, idx: usize, data: T) {                            // c:zsh.h:587
        if let Some(slot) = self.nodes.get_mut(idx) {
            *slot = data;
        }
    }

    /// Port of `empty(X)` macro (`Src/zsh.h:583`).
    pub fn empty(&self) -> bool {                                               // c:zsh.h:583
        self.nodes.is_empty()
    }

    /// Port of `insertlinknode(list, after, dat)` macro
    /// (`Src/zsh.h:580`) and the function form (`Src/linklist.c:133`)
    /// — insert after the supplied node index, return the index of the
    /// inserted node.
    /// WARNING: param names don't match C — Rust=(after_idx, data) vs C=(list, node, dat)
    pub fn insertlinknode(&mut self, after_idx: usize, data: T) -> usize {      // c:linklist.c:133
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
    pub fn delete_node(&mut self, idx: usize) -> Option<T> {                    // c:251
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
        LinkList { nodes: self.nodes.clone(), flags: self.flags }
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

// ===========================================================
// Free-fn ports of `Src/linklist.c` (functions, not macros).
// ===========================================================

// Get an empty linked list header                                         // c:116
/// Port of `newlinklist()` (`Src/linklist.c:103`).
pub fn newlinklist() -> LinkList<String> {                                   // c:103
    LinkList::new()
}

/// Port of `znewlinklist()` (`Src/linklist.c:116`).
pub fn znewlinklist() -> LinkList<String> {                                  // c:116
    LinkList::new()
}

// Insert a node in a linked list after a given node                       // c:151
/// Port of `insertlinknode(LinkList list, LinkNode node, void *dat)` (`Src/linklist.c:133`).
pub fn insertlinknode<T>(list: &mut LinkList<T>, node: usize, dat: T) -> usize { // c:133
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
/// Port of `insertlinklist(LinkList l, LinkNode where, LinkList x)` (`Src/linklist.c:190`).
/// WARNING: param names don't match C — Rust=(l, after_idx, x) vs C=(l, where, x)
pub fn insertlinklist<T: Clone>(l: &mut LinkList<T>, after_idx: usize, x: &LinkList<T>) { // c:190
    let mut idx = after_idx;
    for v in x.iter() {
        idx = l.insertlinknode(idx, v.clone());
    }
}

// Pop the top node off a linked list and free it.                         // c:210
/// Port of `getlinknode(LinkList list)` (`Src/linklist.c:210`).
pub fn getlinknode<T>(list: &mut LinkList<T>) -> Option<T> {                 // c:210
    list.pop_front()
}

// Pop the top node off a linked list without freeing it.                  // c:251
/// Port of `ugetnode(LinkList list)` (`Src/linklist.c:231`).
pub fn ugetnode<T>(list: &mut LinkList<T>) -> Option<T> {                    // c:231
    list.pop_front()
}

// Remove a node from a linked list                                        // c:270
/// Port of `remnode(LinkList list, LinkNode nd)` (`Src/linklist.c:251`).
pub fn remnode<T>(list: &mut LinkList<T>, nd: usize) -> Option<T> {         // c:251
    list.delete_node(nd)
}

/// Port of `uremnode(LinkList list, LinkNode nd)` (`Src/linklist.c:270`).
pub fn uremnode<T>(list: &mut LinkList<T>, nd: usize) -> Option<T> {        // c:270
    list.delete_node(nd)
}

// Free a linked list                                                       // c:304
/// Port of `freelinklist(LinkList list, FreeFunc freefunc)` (`Src/linklist.c:287`).
/// WARNING: param names don't match C — Rust=(list) vs C=(list, freefunc)
pub fn freelinklist<T>(list: &mut LinkList<T>) {                             // c:287
    list.clear();
}

// Count the number of nodes in a linked list                              // c:317
/// Port of `countlinknodes(LinkList list)` (`Src/linklist.c:304`).
pub fn countlinknodes<T>(list: &LinkList<T>) -> usize {                      // c:304
    list.len()
}

// Make specified node first, moving preceding nodes to end                // c:317
/// Port of `rolllist(LinkList l, LinkNode nd)` (`Src/linklist.c:317`).
pub fn rolllist<T>(l: &mut LinkList<T>, nd: usize) {                       // c:317
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
/// Port of `newsizedlist(int size)` (`Src/linklist.c:331`).
#[allow(unused_variables)]
pub fn newsizedlist<T>(size: usize) -> LinkList<T> {                        // c:331
    LinkList::new()
}

/// Port of `joinlists(LinkList first, LinkList second)` (`Src/linklist.c:360`).
pub fn joinlists<T>(first: &mut LinkList<T>, second: &mut LinkList<T>) {              // c:360
    first.append(second);
}

/// Port of `linknodebydatum(LinkList list, void *dat)` (`Src/linklist.c:386`).
pub fn linknodebydatum<T: PartialEq>(list: &LinkList<T>, dat: &T) -> Option<usize> { // c:386
    list.iter().position(|v| v == dat)
}

/// Port of `linknodebystring(LinkList list, char *dat)` (`Src/linklist.c:403`).
pub fn linknodebystring(list: &LinkList<String>, dat: &str) -> Option<usize> { // c:403
    list.iter().position(|v| v == dat)
}

/// Convert a linked list of strings to a `Vec`. Port of
/// `hlinklist2array()` (`Src/linklist.c:423`).
pub fn hlinklist2array(list: &LinkList<String>) -> Vec<String> {                // c:423
    list.iter().cloned().collect()
}

/// Port of `zlinklist2array(LinkList list, int copy)` (`Src/linklist.c:449`).
/// WARNING: param names don't match C — Rust=(list) vs C=(list, copy)
pub fn zlinklist2array(list: &LinkList<String>) -> Vec<String> {             // c:449
    list.iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_list() {
        let list: LinkList<i32> = LinkList::new();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        assert_eq!(list.flags, 0);
    }

    #[test]
    fn test_push_front_back() {
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
        let mut list = LinkList::new();
        list.push_back(1);
        list.push_back(2);
        list.push_back(3);
        let v: Vec<_> = list.iter().copied().collect();
        assert_eq!(v, vec![1, 2, 3]);
    }

    #[test]
    fn test_macro_methods() {
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
        let mut a: LinkList<i32> = vec![1, 2].into_iter().collect();
        let mut b: LinkList<i32> = vec![3, 4].into_iter().collect();
        a.append(&mut b);
        assert!(b.is_empty());
        assert_eq!(a.to_vec(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_clear() {
        let mut list: LinkList<i32> = vec![1, 2, 3].into_iter().collect();
        list.clear();
        assert!(list.is_empty());
    }

    #[test]
    fn test_uinsertlinknode_dedups() {
        let mut list: LinkList<String> = LinkList::new();
        list.push_back("a".to_string());
        assert!(uinsertlinknode(&mut list, 0, "b".to_string()).is_some());
        assert!(uinsertlinknode(&mut list, 0, "a".to_string()).is_none());
        assert_eq!(list.len(), 2);
    }
}
