use alloc::alloc::{AllocError, Allocator, Global};

use crate::collections::{QueryFor, raw::Direction};

use super::rbtree::{RBNodePtr, RBTree};
#[derive(Debug)]
struct SortedNode<K, V> {
    value: V,
    next: Option<SortedNodePtr<K, V>>,
    prev: Option<SortedNodePtr<K, V>>,
}

impl<K, V> SortedNode<K, V> {
    fn unlink<A: Allocator>(&self, tree: &mut LinkedRBTree<K, V, A>) {
        if let Some(mut prev) = self.prev {
            unsafe {
                prev.set_next(self.next);
            };
        } else {
            tree.head = self.next;
        }

        if let Some(mut next) = self.next {
            unsafe {
                next.set_prev(self.prev);
            };
        } else {
            tree.tail = self.prev;
        }
    }
}

#[derive(Debug)]
struct SortedNodePtr<K, V>(RBNodePtr<K, SortedNode<K, V>>);

unsafe impl<K: Send, V: Send> Send for SortedNodePtr<K, V> {}
unsafe impl<K: Sync, V: Sync> Sync for SortedNodePtr<K, V> {}

impl<K, V> Clone for SortedNodePtr<K, V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<K, V> Copy for SortedNodePtr<K, V> {}

impl<K, V> PartialEq for SortedNodePtr<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl<K, V> SortedNodePtr<K, V> {
    #[inline]
    unsafe fn raw<'a>(&self) -> &'a SortedNode<K, V> {
        unsafe { &self.0.as_ref().value().value }
    }

    #[inline]
    unsafe fn raw_mut<'a>(&mut self) -> &'a mut SortedNode<K, V> {
        unsafe { &mut self.0.as_mut().value_mut().value }
    }

    #[inline(always)]
    unsafe fn key<'a>(&self) -> &'a K
    where
        K: 'a,
        V: 'a,
    {
        let this = unsafe { self.0.as_ref() };
        &this.value().key
    }

    #[inline(always)]
    unsafe fn key_mut<'a>(&mut self) -> &'a mut K
    where
        K: 'a,
        V: 'a,
    {
        let this = unsafe { self.0.as_mut() };
        unsafe { &mut this.value_mut().key }
    }

    #[inline(always)]
    unsafe fn value<'a>(&self) -> &'a V
    where
        K: 'a,
        V: 'a,
    {
        let raw: &'a SortedNode<K, V> = unsafe { self.raw() };
        &raw.value
    }

    #[inline(always)]
    unsafe fn value_mut<'a>(&mut self) -> &'a mut V
    where
        K: 'a,
        V: 'a,
    {
        let raw: &'a mut SortedNode<K, V> = unsafe { self.raw_mut() };
        &mut raw.value
    }

    #[inline(always)]
    unsafe fn next(&self) -> Option<Self> {
        unsafe { self.raw().next }
    }

    #[inline(always)]
    unsafe fn prev(&self) -> Option<Self> {
        unsafe { self.raw().prev }
    }

    #[inline(always)]
    unsafe fn set_next(&mut self, next: Option<SortedNodePtr<K, V>>) {
        unsafe { self.raw_mut().next = next }
    }

    #[inline(always)]
    unsafe fn set_prev(&mut self, prev: Option<SortedNodePtr<K, V>>) {
        unsafe { self.raw_mut().prev = prev }
    }
}

/// A cursor to a node within a [`LinkedRBTree`].
pub struct Cursor<'a, K, V, A: Allocator = Global> {
    ptr: Option<SortedNodePtr<K, V>>,
    tree: &'a LinkedRBTree<K, V, A>,
}

impl<'a, K, V, A: Allocator> Cursor<'a, K, V, A> {
    #[inline(always)]
    /// Returns the key-value pair of the current node, if it exists.
    pub fn key_value(&self) -> Option<(&'a K, &'a V)> {
        unsafe { self.ptr.map(|n| (n.key(), n.value())) }
    }

    #[inline]
    /// Returns the key of the current node, if it exists.
    pub fn key(&self) -> Option<&'a K> {
        self.key_value().map(|(k, _)| k)
    }

    #[inline]
    /// Returns the value of the current node, if it exists.
    pub fn value(&self) -> Option<&'a V> {
        self.key_value().map(|(_, v)| v)
    }

    #[inline]
    /// Returns the key-value pair of the previous node, if it exists.
    pub fn peek_prev(&self) -> Option<(&'a K, &'a V)> {
        unsafe {
            self.ptr
                .and_then(|n| n.prev().map(|p| (p.key(), p.value())))
        }
    }

    #[inline]
    /// Returns the key-value pair of the next node, if it exists.
    pub fn peek_next(&self) -> Option<(&'a K, &'a V)> {
        unsafe {
            self.ptr
                .and_then(|n| n.next().map(|n| (n.key(), n.value())))
        }
    }

    /// Moves the cursor to the previous node, if it exists otherwise moves to a ghost node before the head.
    ///
    /// If the cursor is at a ghost node, it moves to the tail node.
    pub fn move_prev(&mut self) {
        if let Some(ptr) = self.ptr {
            if let Some(prev) = unsafe { ptr.prev() } {
                self.ptr = Some(prev);
            } else {
                self.ptr = None;
            }
        } else {
            self.ptr = self.tree.tail;
        }
    }

    /// Moves the cursor to the next node if it exists, otherwise moves to the head node.
    ///
    /// If the cursor is at a ghost node, it moves to the head node.
    pub fn move_next(&mut self) {
        if let Some(ptr) = self.ptr {
            if let Some(next) = unsafe { ptr.next() } {
                self.ptr = Some(next);
            } else {
                self.ptr = None;
            }
        } else {
            self.ptr = self.tree.head;
        }
    }
}

/// Read-only cursor is like a reference.
impl<'a, K, V> Clone for Cursor<'a, K, V> {
    fn clone(&self) -> Self {
        Self {
            ptr: self.ptr,
            tree: self.tree,
        }
    }
}
impl<'a, K, V> Copy for Cursor<'a, K, V> {}

/// A mutable cursor is like a reference, but allows modifying the tree.
///
/// Muttable version of [`Cursor`].
pub struct CursorMut<'a, K, V, A: Allocator = Global> {
    ptr: Option<SortedNodePtr<K, V>>,
    tree: &'a mut LinkedRBTree<K, V, A>,
}

impl<'a, K, V, A: Allocator> CursorMut<'a, K, V, A> {
    #[inline(always)]
    /// Returns the key-value pair of the current node, if it exists.
    pub fn key_value(&self) -> Option<(&'a K, &'a V)> {
        unsafe { self.ptr.map(|n| (n.key(), n.value())) }
    }

    #[inline(always)]
    /// Returns the mutable key-value pair of the current node, if it exists.
    ///
    /// # Safety: muttating a key is unsafe because it could break the tree's invariants.
    pub unsafe fn key_value_mut(&mut self) -> Option<(&'a mut K, &'a mut V)> {
        unsafe { self.ptr.as_mut().map(|n| (n.key_mut(), n.value_mut())) }
    }

    #[inline]
    /// Returns the mutable key of the current node, if it exists.
    ///
    /// # Safety: muttating a key is unsafe because it could break the tree's invariants.
    pub unsafe fn key_mut(&mut self) -> Option<&'a mut K> {
        unsafe { self.key_value_mut().map(|(k, _)| k) }
    }

    #[inline]
    /// Returns the mutable value of the current node, if it exists.
    pub fn value_mut(&mut self) -> Option<&'a mut V> {
        unsafe { self.key_value_mut().map(|(_, v)| v) }
    }

    #[inline]
    /// Returns the key of the current node, if it exists.
    pub fn key(&self) -> Option<&'a K> {
        self.key_value().map(|(k, _)| k)
    }

    #[inline]
    /// Returns the value of the current node, if it exists.
    pub fn value(&self) -> Option<&'a V> {
        self.key_value().map(|(_, v)| v)
    }

    #[inline]
    /// Returns the key-value pair of the previous node, if it exists.
    pub fn peek_prev(&self) -> Option<(&'a K, &'a V)> {
        unsafe {
            self.ptr
                .and_then(|n| n.prev().map(|p| (p.key(), p.value())))
        }
    }

    #[inline]
    /// Returns the key-value pair of the next node, if it exists.
    pub fn peek_next(&self) -> Option<(&'a K, &'a V)> {
        unsafe {
            self.ptr
                .and_then(|n| n.next().map(|n| (n.key(), n.value())))
        }
    }

    /// Completely removes the current node from the tree and returns its key-value pair.
    pub fn remove_inplace(&mut self) -> Option<(K, V)> {
        if let Some(ptr) = self.ptr {
            unsafe {
                let (k, v) = self.tree.tree.remove_node(ptr.0);
                v.unlink(self.tree);

                Some((k, v.value))
            }
        } else {
            None
        }
    }

    /// Moves the cursor to the previous node, if it exists otherwise moves to a ghost node before the head.
    ///
    /// If the cursor is at a ghost node, it moves to the tail node.
    pub fn move_prev(&mut self) {
        if let Some(ptr) = self.ptr {
            if let Some(prev) = unsafe { ptr.prev() } {
                self.ptr = Some(prev);
            } else {
                self.ptr = None;
            }
        } else {
            self.ptr = self.tree.tail;
        }
    }

    /// Moves the cursor to the next node if it exists, otherwise moves to the head node.
    ///
    /// If the cursor is at a ghost node, it moves to the head node.
    pub fn move_next(&mut self) {
        if let Some(ptr) = self.ptr {
            if let Some(next) = unsafe { ptr.next() } {
                self.ptr = Some(next);
            } else {
                self.ptr = None;
            }
        } else {
            self.ptr = self.tree.head;
        }
    }
}

/// An [`RBTree`] that maintains a sorted linked list of values, allowing for efficient range queries.
pub struct LinkedRBTree<K, V, A: Allocator = Global> {
    tree: RBTree<K, SortedNode<K, V>, A>,
    head: Option<SortedNodePtr<K, V>>,
    tail: Option<SortedNodePtr<K, V>>,
}

impl<K, V> LinkedRBTree<K, V> {
    #[inline(always)]
    /// Creates a new `LinkedRBTree` with the default allocator.
    pub const fn new() -> Self {
        Self {
            tree: RBTree::new(),
            head: None,
            tail: None,
        }
    }
}

impl<K, V, A: Allocator> LinkedRBTree<K, V, A> {
    pub const SIZE_OF_NODE: usize = RBTree::<K, SortedNode<K, V>, A>::SIZE_OF_NODE;

    /// Allocates a new `LinkedRBTree` with the given allocator.
    #[inline(always)]
    pub const fn new_in(alloc: A) -> Self {
        Self {
            tree: RBTree::new_in(alloc),
            head: None,
            tail: None,
        }
    }

    #[inline(always)]
    /// Returns the value associated with the given key, if it exists.
    pub fn get<Q: QueryFor<K>>(&self, key: &Q) -> Option<&V> {
        let node = self.tree.get(key)?;
        Some(&node.value)
    }

    /// Returns a cursor to the node with the given key, if it doesn't exist cursor points to a ghost node.
    pub fn cursor_to<Q: QueryFor<K>>(&self, key: &Q) -> Cursor<'_, K, V, A> {
        let (node, _, _) = self.tree.raw().search(key);
        Cursor {
            ptr: node.map(|n| SortedNodePtr(n)),
            tree: self,
        }
    }

    /// Returns a mutable cursor to the node with the given key, if it doesn't exist cursor points to a ghost node.
    pub fn cursor_mut_to<Q: QueryFor<K>>(&mut self, key: &Q) -> CursorMut<'_, K, V, A> {
        let (node, _, _) = self.tree.raw().search(key);
        CursorMut {
            ptr: node.map(|n| SortedNodePtr(n)),
            tree: self,
        }
    }

    /// Returns a cursor to the front of the tree's linked list.
    #[inline(always)]
    pub fn front_cursor(&self) -> Cursor<'_, K, V, A> {
        Cursor {
            ptr: self.head,
            tree: self,
        }
    }

    /// Returns a cursor to the back of the tree's linked list.
    #[inline(always)]
    pub fn back_cursor(&self) -> Cursor<'_, K, V, A> {
        Cursor {
            ptr: self.tail,
            tree: self,
        }
    }

    /// Returns a mutable cursor to the front of the tree's linked list.
    pub fn front_cursor_mut(&mut self) -> CursorMut<'_, K, V, A> {
        CursorMut {
            ptr: self.head,
            tree: self,
        }
    }

    /// Returns a mutable cursor to the back of the tree's linked list.
    #[inline(always)]
    pub fn back_cursor_mut(&mut self) -> CursorMut<'_, K, V, A> {
        CursorMut {
            ptr: self.tail,
            tree: self,
        }
    }

    #[inline(always)]
    /// Returns a mutable reference to the value associated with the given key, if it exists.
    pub fn get_mut<Q: QueryFor<K>>(&mut self, key: &Q) -> Option<&mut V> {
        let node = self.tree.get_mut(key)?;
        Some(&mut node.value)
    }

    /// Inserts a key-value pair into the tree, returning the old value if it already existed or an allocator error.
    pub fn try_insert(&mut self, key: K, value: V) -> Result<Option<V>, AllocError>
    where
        K: QueryFor<K>,
    {
        let was_empty = self.is_empty();
        let entry = self.tree.entry(key);

        entry.replace_or_alloc(
            SortedNode {
                value,
                next: None,
                prev: None,
            },
            |raw_node_ptr, parent, old| {
                {
                    let mut node_ptr = SortedNodePtr(raw_node_ptr);
                    let node_mut = unsafe { node_ptr.raw_mut() };

                    match parent.map(|(p, d)| (SortedNodePtr(p), d)) {
                        None if let Some(ref old) = old => {
                            debug_assert!(
                                !was_empty,
                                "inserting over node, so tree should not be empty"
                            );

                            node_mut.next = old.next;
                            node_mut.prev = old.prev;
                        }
                        None if was_empty => {
                            self.head = Some(node_ptr);
                            self.tail = Some(node_ptr);
                        }

                        None => unreachable!("Node should either have a parent, already exists or the tree should be empty"),

                        Some((mut parent, Direction::Left)) => {
                            // key is less than parent, insert before parent
                            let prev = unsafe { parent.prev() };
                            unsafe {
                                node_ptr.set_prev(prev);
                                node_ptr.set_next(Some(parent));
                                parent.set_prev(Some(node_ptr));
                            }
                            match prev {
                                None => {
                                    self.head = Some(node_ptr);
                                }
                                Some(mut p) => unsafe {
                                    p.set_next(Some(node_ptr));
                                },
                            }
                        }

                        Some((mut parent, Direction::Right)) => {
                            // key is greater than parent, insert after parent
                            let next = unsafe { parent.next() };
                            unsafe {
                                node_ptr.set_prev(Some(parent));
                                node_ptr.set_next(next);
                                parent.set_next(Some(node_ptr));
                            }
                            match next {
                                None => {
                                    self.tail = Some(node_ptr);
                                }
                                Some(mut p) => unsafe {
                                    p.set_prev(Some(node_ptr));
                                },
                            }
                        }
                    }
                }
                old.map(|o| o.value)
            },
        )
    }

    /// Removes a key-value pair from the tree, returning the value if it existed.
    pub fn remove<Q: QueryFor<K>>(&mut self, key: &Q) -> Option<(K, V)> {
        self.tree.remove(key).map(|(k, v)| {
            v.unlink(self);
            (k, v.value)
        })
    }

    #[inline(always)]
    /// Clears the tree, removing all key-value pairs.
    pub fn clear(&mut self) {
        let len = self.len();

        let mut current = self.head.take();
        let mut cleared = 0;
        while let Some(node) = current {
            current = unsafe { node.next() };

            let value = unsafe { core::ptr::read(node.0.as_ref().value()) };
            unsafe { self.tree.dealloc_node(node.0) };
            drop(value);
            cleared += 1;
        }

        debug_assert_eq!(
            cleared, len,
            "LinkedRBTree::clear: cleared {cleared} != len {len}"
        );

        self.head = None;
        self.tail = None;
        self.tree.mark_clear();
    }

    #[inline(always)]
    /// Returns the number of key-value pairs in the tree.
    pub const fn len(&self) -> usize {
        self.tree.len()
    }

    #[inline(always)]
    /// Returns whether the tree is empty.
    pub const fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }
}

impl<K, V, A: Allocator> Drop for LinkedRBTree<K, V, A> {
    fn drop(&mut self) {
        self.clear();
    }
}
