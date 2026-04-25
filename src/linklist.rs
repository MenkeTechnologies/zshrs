//! Linked list implementation for zshrs
//!
//! Direct port from zsh/Src/linklist.c
//!
//! Provides a doubly-linked list with operations matching zsh's LinkList API.

use std::marker::PhantomData;
use std::ptr::NonNull;

/// A node in the linked list
pub struct LinkNode<T> {
    pub data: T,
    next: Option<NonNull<LinkNode<T>>>,
    prev: Option<NonNull<LinkNode<T>>>,
}

/// A doubly-linked list
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
    /// Create a new empty linked list
    pub fn new() -> Self {
        LinkList {
            head: None,
            tail: None,
            len: 0,
            _marker: PhantomData,
        }
    }

    /// Check if the list is empty
    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    /// Get the length of the list
    pub fn len(&self) -> usize {
        self.len
    }

    /// Add an element to the front of the list
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

    /// Add an element to the back of the list
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

    /// Remove and return the first element
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

    /// Remove and return the last element
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

    /// Get a reference to the first element
    pub fn front(&self) -> Option<&T> {
        self.head.map(|node| unsafe { &(*node.as_ptr()).data })
    }

    /// Get a mutable reference to the first element
    pub fn front_mut(&mut self) -> Option<&mut T> {
        self.head.map(|node| unsafe { &mut (*node.as_ptr()).data })
    }

    /// Get a reference to the last element
    pub fn back(&self) -> Option<&T> {
        self.tail.map(|node| unsafe { &(*node.as_ptr()).data })
    }

    /// Get a mutable reference to the last element
    pub fn back_mut(&mut self) -> Option<&mut T> {
        self.tail.map(|node| unsafe { &mut (*node.as_ptr()).data })
    }

    /// Create an iterator over references
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            current: self.head,
            _marker: PhantomData,
        }
    }

    /// Create an iterator over mutable references
    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        IterMut {
            current: self.head,
            _marker: PhantomData,
        }
    }

    /// Append another list to the end of this one
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

    /// Convert to a Vec
    pub fn to_vec(self) -> Vec<T> {
        self.into_iter().collect()
    }

    /// Clear the list
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

/// Convert a linked list of strings to a Vec
pub fn linklist_to_vec(list: &LinkList<String>) -> Vec<String> {
    list.iter().cloned().collect()
}

/// Convert a Vec to a linked list
pub fn vec_to_linklist<T>(vec: Vec<T>) -> LinkList<T> {
    vec.into_iter().collect()
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
