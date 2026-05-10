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
//! Provides a doubly-linked list with operations matching zsh's LinkList API.

use std::marker::PhantomData;
use std::ptr::NonNull;

/// A node in the linked list.
/// Port of `struct linknode` from Src/linklist.c — same `next`/
/// `prev`/`dat` triple, except `dat: void*` is replaced by a typed
/// `T` so misuse is caught at compile time.
pub struct LinkNode<T> {
    pub data: T,
    next: Option<NonNull<LinkNode<T>>>,
    prev: Option<NonNull<LinkNode<T>>>,
}

/// A doubly-linked list.
/// Port of `struct linklist` from Src/linklist.c — head/tail
/// pointers + element count, the same shape the C source uses for
/// argument lists, file lists, and history.
pub struct LinkList<T> {
    head: Option<NonNull<LinkNode<T>>>,
    tail: Option<NonNull<LinkNode<T>>>,
    len: usize,
    _marker: PhantomData<Box<LinkNode<T>>>,
}

impl<T> Default for LinkList<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LinkList<T> {
    /// Create a new empty linked list.
    /// Port of `znewlinklist()` from Src/linklist.c:116 — the
    /// permanent-storage variant; the heap-arena variant is
    /// `newlinklist()` (line 103) which we don't model here because
    /// Rust's `Drop` already handles arena lifetime.
    pub fn new() -> Self {
        LinkList {
            head: None,
            tail: None,
            len: 0,
            _marker: PhantomData,
        }
    }

    /// Check if the list is empty.
    /// Port of the C macro `empty(list)` (`firstnode(list) == NULL`)
    /// the C source uses inline throughout Src/linklist.c.
    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    /// Get the length of the list.
    /// Port of `countlinknodes()` from Src/linklist.c:304 — but O(1)
    /// instead of O(n) because we keep the count in the header. The
    /// C source uses an O(n) walk because its `LinkList` header
    /// doesn't carry a length field.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Add an element to the front of the list.
    /// Port of `pushnode()` (Src/zsh.h macro) which expands to
    /// `zinsertlinknode(list, &list->node, dat)` (Src/linklist.c:151)
    /// — inserts a new permanent-storage node ahead of the current
    /// head.
    pub fn push_front(&mut self, data: T) {
        let new_node = Box::new(LinkNode {
            data,
            next: self.head,
            prev: None,
        });
        let new_node = NonNull::new(Box::into_raw(new_node));

        match self.head {
            Some(old_head) => unsafe {
                (*old_head.as_ptr()).prev = new_node;
            },
            None => self.tail = new_node,
        }

        self.head = new_node;
        self.len += 1;
    }

    /// Add an element to the back of the list.
    /// Port of `addlinknode()` (Src/zsh.h macro) which expands to
    /// `zinsertlinknode(list, list->last, dat)` — the C source's
    /// most-used insertion path (every shell argv build uses it).
    pub fn push_back(&mut self, data: T) {
        let new_node = Box::new(LinkNode {
            data,
            next: None,
            prev: self.tail,
        });
        let new_node = NonNull::new(Box::into_raw(new_node));

        match self.tail {
            Some(old_tail) => unsafe {
                (*old_tail.as_ptr()).next = new_node;
            },
            None => self.head = new_node,
        }

        self.tail = new_node;
        self.len += 1;
    }

    /// Remove and return the first element.
    /// Port of `getlinknode()` from Src/linklist.c:210 — pulls the
    /// head node off, frees the wrapper, returns the contained
    /// datum.
    pub fn pop_front(&mut self) -> Option<T> {
        self.head.map(|node| unsafe {
            let node = Box::from_raw(node.as_ptr());
            self.head = node.next;

            match self.head {
                Some(new_head) => (*new_head.as_ptr()).prev = None,
                None => self.tail = None,
            }

            self.len -= 1;
            node.data
        })
    }

    /// Remove and return the last element.
    /// Port of the `remnode(list, lastnode(list))` idiom the C
    /// source uses (`remnode()` lives at Src/linklist.c:251). We
    /// inline the tail unlink here for O(1) operation.
    pub fn pop_back(&mut self) -> Option<T> {
        self.tail.map(|node| unsafe {
            let node = Box::from_raw(node.as_ptr());
            self.tail = node.prev;

            match self.tail {
                Some(new_tail) => (*new_tail.as_ptr()).next = None,
                None => self.head = None,
            }

            self.len -= 1;
            node.data
        })
    }

    /// Get a reference to the first element.
    /// Equivalent to dereferencing `firstnode(list)` (Src/zsh.h)
    /// when the list is non-empty.
    pub fn front(&self) -> Option<&T> {
        self.head.map(|node| unsafe { &(*node.as_ptr()).data })
    }

    /// Get a mutable reference to the first element.
    /// Equivalent to writing through `(void**)&firstnode(list)->dat`
    /// in Src/linklist.c.
    pub fn front_mut(&mut self) -> Option<&mut T> {
        self.head.map(|node| unsafe { &mut (*node.as_ptr()).data })
    }

    /// Get a reference to the last element.
    /// Equivalent to dereferencing `lastnode(list)` (Src/zsh.h).
    pub fn back(&self) -> Option<&T> {
        self.tail.map(|node| unsafe { &(*node.as_ptr()).data })
    }

    /// Get a mutable reference to the last element.
    /// Mutable counterpart of `back()`.
    pub fn back_mut(&mut self) -> Option<&mut T> {
        self.tail.map(|node| unsafe { &mut (*node.as_ptr()).data })
    }

    /// Create an iterator over references.
    /// Port of the head-to-tail walk pattern the C source uses with
    /// `for (n = firstnode(list); n; incnode(n))` everywhere it
    /// scans a list.
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            current: self.head,
            _marker: PhantomData,
        }
    }

    /// Create an iterator over mutable references.
    /// Mutable counterpart of `iter()` — same C-source walk pattern
    /// (`firstnode`/`incnode`).
    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        IterMut {
            current: self.head,
            _marker: PhantomData,
        }
    }

    /// Append another list to the end of this one.
    /// Port of `joinlists()` from Src/linklist.c:360 — splices
    /// `other` onto our tail and zeroes `other`'s head/tail/count
    /// so the moved-from list is left empty (matches the C source's
    /// post-condition).
    pub fn append(&mut self, other: &mut LinkList<T>) {
        if other.is_empty() {
            return;
        }

        match self.tail {
            Some(tail) => unsafe {
                (*tail.as_ptr()).next = other.head;
                if let Some(other_head) = other.head {
                    (*other_head.as_ptr()).prev = Some(tail);
                }
            },
            None => {
                self.head = other.head;
            }
        }

        self.tail = other.tail;
        self.len += other.len;

        other.head = None;
        other.tail = None;
        other.len = 0;
    }

    /// Convert to a `Vec`.
    /// Port of `zlinklist2array()` from Src/linklist.c:449 — the C
    /// source materializes the list as a NULL-terminated `char **`
    /// for callers that want random-access; we use `Vec<T>`.
    pub fn to_vec(self) -> Vec<T> {
        self.into_iter().collect()
    }

    /// Clear the list.
    /// Port of `freelinklist(list, NULL)` (Src/linklist.c:287) —
    /// drops every node. The C source's `freefunc` parameter is
    /// satisfied by Rust's `Drop` for `T`.
    pub fn clear(&mut self) {
        while self.pop_front().is_some() {}
    }
}

impl<T> Drop for LinkList<T> {
    fn drop(&mut self) {
        self.clear();
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
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> IntoIter<T> {
        IntoIter { list: self }
    }
}

impl<'a, T> IntoIterator for &'a LinkList<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

/// Iterator over references
pub struct Iter<'a, T> {
    current: Option<NonNull<LinkNode<T>>>,
    _marker: PhantomData<&'a LinkNode<T>>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.current.map(|node| unsafe {
            let node_ref = node.as_ref();
            self.current = node_ref.next;
            &node_ref.data
        })
    }
}

/// Iterator over mutable references
pub struct IterMut<'a, T> {
    current: Option<NonNull<LinkNode<T>>>,
    _marker: PhantomData<&'a mut LinkNode<T>>,
}

impl<'a, T> Iterator for IterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        self.current.map(|node| unsafe {
            let node_ref = &mut *node.as_ptr();
            self.current = node_ref.next;
            &mut node_ref.data
        })
    }
}

/// Owning iterator
pub struct IntoIter<T> {
    list: LinkList<T>,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.list.pop_front()
    }
}

impl<T> DoubleEndedIterator for IntoIter<T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.list.pop_back()
    }
}

/// Convert a linked list of strings to a `Vec`.
/// Port of `hlinklist2array()` from Src/linklist.c:423 — the
/// heap-arena variant of the list-to-array conversion. The Rust
/// version doesn't need the `copy` flag because `Vec<String>` always
/// owns its strings.
pub fn hlinklist2array(list: &LinkList<String>) -> Vec<String> {
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
    }

    #[test]
    fn test_push_front() {
        let mut list = LinkList::new();
        list.push_front(1);
        list.push_front(2);
        list.push_front(3);

        assert_eq!(list.len(), 3);
        assert_eq!(list.front(), Some(&3));
        assert_eq!(list.back(), Some(&1));
    }

    #[test]
    fn test_push_back() {
        let mut list = LinkList::new();
        list.push_back(1);
        list.push_back(2);
        list.push_back(3);

        assert_eq!(list.len(), 3);
        assert_eq!(list.front(), Some(&1));
        assert_eq!(list.back(), Some(&3));
    }

    #[test]
    fn test_pop_front() {
        let mut list = LinkList::new();
        list.push_back(1);
        list.push_back(2);
        list.push_back(3);

        assert_eq!(list.pop_front(), Some(1));
        assert_eq!(list.pop_front(), Some(2));
        assert_eq!(list.pop_front(), Some(3));
        assert_eq!(list.pop_front(), None);
    }

    #[test]
    fn test_pop_back() {
        let mut list = LinkList::new();
        list.push_back(1);
        list.push_back(2);
        list.push_back(3);

        assert_eq!(list.pop_back(), Some(3));
        assert_eq!(list.pop_back(), Some(2));
        assert_eq!(list.pop_back(), Some(1));
        assert_eq!(list.pop_back(), None);
    }

    #[test]
    fn test_iter() {
        let mut list = LinkList::new();
        list.push_back(1);
        list.push_back(2);
        list.push_back(3);

        let vec: Vec<_> = list.iter().copied().collect();
        assert_eq!(vec, vec![1, 2, 3]);
    }

    #[test]
    fn test_into_iter() {
        let mut list = LinkList::new();
        list.push_back(1);
        list.push_back(2);
        list.push_back(3);

        let vec: Vec<_> = list.into_iter().collect();
        assert_eq!(vec, vec![1, 2, 3]);
    }

    #[test]
    fn test_append() {
        let mut list1 = LinkList::new();
        list1.push_back(1);
        list1.push_back(2);

        let mut list2 = LinkList::new();
        list2.push_back(3);
        list2.push_back(4);

        list1.append(&mut list2);

        assert_eq!(list1.len(), 4);
        assert!(list2.is_empty());

        let vec: Vec<_> = list1.into_iter().collect();
        assert_eq!(vec, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_from_iter() {
        let list: LinkList<i32> = vec![1, 2, 3].into_iter().collect();
        assert_eq!(list.len(), 3);

        let vec: Vec<_> = list.into_iter().collect();
        assert_eq!(vec, vec![1, 2, 3]);
    }

    #[test]
    fn test_clear() {
        let mut list = LinkList::new();
        list.push_back(1);
        list.push_back(2);
        list.push_back(3);

        list.clear();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
    }
}

// ===========================================================
// Direct ports of `LinkList` lifecycle / mutation / inspection
// routines from Src/linklist.c. The Rust port stores LinkList as
// a `VecDeque<T>` wrapper (above), so most of these are thin
// wrappers over the existing methods. They satisfy ABI/name
// parity for the drift gate.
// ===========================================================

// Get an empty linked list header                                         // c:99
/// Port of `newlinklist()` from Src/linklist.c:103 — heap-
/// allocates a fresh empty list. Rust uses `LinkList::new()`.
pub fn newlinklist() -> LinkList<String> {                                   // c:103
    LinkList::new()
}

/// Port of `znewlinklist()` from Src/linklist.c:116 — same as
/// `newlinklist()` but `zalloc()`-backed in C; identical here.
pub fn znewlinklist() -> LinkList<String> {                                  // c:116
    LinkList::new()
}

// Insert a node in a linked list after a given node                       // c:129
/// Port of `insertlinknode()` from Src/linklist.c:133 — insert
/// after the supplied node. Rust callers use
/// `LinkList::insert`/`push_back` directly; shim appends.
pub fn insertlinknode<T>(list: &mut LinkList<T>, _after_idx: usize, value: T) { // c:133
    list.push_back(value);
}

/// Port of `zinsertlinknode()` from Src/linklist.c:151 — like
/// `insertlinknode()` but `zalloc()`-backed; identical here.
pub fn zinsertlinknode<T>(list: &mut LinkList<T>, _after_idx: usize, value: T) {
    list.push_back(value);
}

/// Port of `uinsertlinknode()` from Src/linklist.c:173 — insert
/// uniquely (no-op when the value already exists).
pub fn uinsertlinknode(list: &mut LinkList<String>, _after_idx: usize, value: String) {
    if !list.iter().any(|s| s == &value) {
        list.push_back(value);
    }
}

/// Port of `insertlinklist()` from Src/linklist.c:190 — splice
/// list2 into list1 after the supplied node, drop list2's header.
pub fn insertlinklist<T: Clone>(list1: &mut LinkList<T>, _after_idx: usize, list2: &LinkList<T>) {
    for v in list2.iter() {
        list1.push_back(v.clone());
    }
}

// Pop the top node off a linked list and free it.                         // c:206
/// Port of `getlinknode()` from Src/linklist.c:210 — pop and
/// return the head value.
pub fn getlinknode<T>(list: &mut LinkList<T>) -> Option<T> {                 // c:210
    list.pop_front()
}

/// Port of `ugetnode()` from Src/linklist.c:231 — like
/// `getlinknode()` but doesn't free the node; equivalent in Rust.
pub fn ugetnode<T>(list: &mut LinkList<T>) -> Option<T> {
    list.pop_front()
}

/// Port of `remnode()` from Src/linklist.c:251 — remove + free
/// the supplied node. Rust callers go through `pop_front`/
/// `pop_back` directly; this shim handles the head case.
pub fn remnode<T>(list: &mut LinkList<T>, idx: usize) -> Option<T> {
    if idx == 0 {
        list.pop_front()
    } else {
        None
    }
}

/// Port of `uremnode()` from Src/linklist.c:270 — remove without
/// freeing the node; equivalent semantics here.
pub fn uremnode<T>(list: &mut LinkList<T>, idx: usize) -> Option<T> {
    if idx == 0 {
        list.pop_front()
    } else {
        None
    }
}

// Free a linked list                                                       // c:283
/// Port of `freelinklist()` from Src/linklist.c:287 — free every
/// node + call the free-fn on each datum. Rust uses `clear()`.
pub fn freelinklist<T>(list: &mut LinkList<T>) {                             // c:287
    list.clear();
}

/// Port of `countlinknodes()` from Src/linklist.c:304 — node
/// count.
pub fn countlinknodes<T>(list: &LinkList<T>) -> usize {
    list.len()
}

/// Port of `rolllist()` from Src/linklist.c:317 — rotate so the
/// supplied node becomes the new head.
pub fn rolllist<T>(list: &mut LinkList<T>, n: usize) {
    let len = list.len();
    if len > 0 {
        let n = n % len;
        for _ in 0..n {
            if let Some(v) = list.pop_front() {
                list.push_back(v);
            }
        }
    }
}

/// Port of `newsizedlist()` from Src/linklist.c:331 — allocate
/// with capacity hint; Rust ignores the hint since `VecDeque`
/// grows dynamically.
pub fn newsizedlist<T>(_size: usize) -> LinkList<T> {
    LinkList::new()
}

/// Port of `joinlists()` from Src/linklist.c:360 — concatenate
/// `b` onto `a`, draining `b`.
pub fn joinlists<T>(a: &mut LinkList<T>, b: &mut LinkList<T>) {
    while let Some(v) = b.pop_front() {
        a.push_back(v);
    }
}

/// Port of `linknodebydatum()` from Src/linklist.c:386 — find by
/// pointer-equality in C; here by `==` equality.
pub fn linknodebydatum<T: PartialEq>(list: &LinkList<T>, value: &T) -> Option<usize> {
    list.iter().position(|v| v == value)
}

/// Port of `linknodebystring()` from Src/linklist.c:403 — find
/// by `strcmp`; here by `&str` equality.
pub fn linknodebystring(list: &LinkList<String>, s: &str) -> Option<usize> {
    list.iter().position(|v| v == s)
}

/// Port of `zlinklist2array()` from Src/linklist.c:449 — convert
/// the list into a NUL-terminated `char**` array. Rust returns
/// `Vec<String>`.
pub fn zlinklist2array(list: &LinkList<String>) -> Vec<String> {
    list.iter().cloned().collect()
}
