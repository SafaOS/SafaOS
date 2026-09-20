use core::alloc::Layout;
use core::cmp::Ordering;
use core::ptr::NonNull;

use alloc::alloc::AllocError;
use alloc::alloc::Allocator;
use alloc::alloc::Global;

use crate::collections::QueryFor;

use super::raw;

#[derive(Debug)]
pub(super) struct KeyValue<K, V> {
    pub(super) key: K,
    pub(super) value: V,
}

impl<Q: QueryFor<K>, K, V> QueryFor<KeyValue<K, V>> for Q {
    fn compare(&self, key: &KeyValue<K, V>) -> Ordering {
        self.compare(&key.key)
    }
}

/// A key-value node in the RBTree.
pub(super) type RBNode<K, V> = raw::Node<KeyValue<K, V>>;
/// A pointer to an [`RBNode`].
pub(super) type RBNodePtr<K, V> = NonNull<RBNode<K, V>>;

/// A red-black tree that stores key-value pairs.
/// Provides O(log n) insert, delete, and lookup operations with low memory overhead.
pub struct RBTree<K, V, A: Allocator = Global> {
    pub(super) raw: raw::RawRBTree<KeyValue<K, V>>,
    len: usize,
    alloc: A,
}

impl<K, V, A: Allocator> Drop for RBTree<K, V, A> {
    fn drop(&mut self) {
        self.clear();
    }
}

/// Represents an entry in the RBTree, either vacant or occupied.
///
/// WIP
pub(super) enum RBTreeEntry<'a, K, V, A: Allocator = Global> {
    Vacant {
        tree: &'a mut RBTree<K, V, A>,
        key: K,
        parent: Option<RBNodePtr<K, V>>,
        direction: raw::Direction,
    },
    Occupied {
        node: &'a mut RBNode<K, V>,
    },
}

impl<'a, K, V, A: Allocator> RBTreeEntry<'a, K, V, A> {
    /// Replaces the value in the entry with `value` or allocates a new node if the entry is vacant and sets it to `value`.
    ///
    /// returns the old value, if any.
    fn try_replace(self, value: V) -> Result<Option<V>, AllocError> {
        self.replace_or_alloc(value, |_, _, old| old)
    }

    #[inline(always)]
    /// Replaces the value in the entry, or allocates a new node if the entry is vacant and sets it to the given value.
    ///
    /// calls `f` with the node pointer, the parent pointer the node was inserted at, the old value.
    pub(super) fn replace_or_alloc<
        F: FnOnce(RBNodePtr<K, V>, Option<(RBNodePtr<K, V>, raw::Direction)>, Option<V>) -> R,
        R,
    >(
        self,
        value: V,
        f: F,
    ) -> Result<R, AllocError> {
        match self {
            Self::Vacant {
                tree,
                key,
                parent,
                direction,
            } => unsafe {
                let new_node = tree.alloc_node(key, value)?;
                tree.raw.insert_at_node(new_node, parent, direction);
                Ok(f(new_node, parent.map(|p| (p, direction)), None))
            },
            Self::Occupied { node } => unsafe {
                let node_ptr = NonNull::from_mut(node);
                let old_m = &mut node.value_mut().value;
                let old = core::mem::replace(old_m, value);
                Ok(f(node_ptr, None, Some(old)))
            },
        }
    }
}

impl<K, V> RBTree<K, V> {
    #[inline]
    /// Creates a new `RBTree` with the default allocator.
    pub const fn new() -> Self {
        Self {
            raw: raw::RawRBTree::new(),
            len: 0,
            alloc: Global,
        }
    }
}

impl<K, V, A: Allocator> RBTree<K, V, A> {
    #[inline]
    /// Creates a new `RBTree` with the given allocator.
    pub const fn new_in(alloc: A) -> Self {
        Self {
            raw: raw::RawRBTree::new(),
            len: 0,
            alloc,
        }
    }

    /// Returns a reference to the raw `RawRBTree` backing this `RBTree`.
    pub(super) const fn raw(&self) -> &raw::RawRBTree<KeyValue<K, V>> {
        &self.raw
    }

    fn alloc_node(
        &mut self,
        key: K,
        value: V,
    ) -> Result<NonNull<raw::Node<KeyValue<K, V>>>, AllocError> {
        let key_value = KeyValue { key, value };
        let node = self
            .alloc
            .allocate(Layout::new::<raw::Node<KeyValue<K, V>>>())?
            .cast();

        unsafe { core::ptr::write(node.as_ptr(), raw::Node::new(key_value)) };
        self.len += 1;
        Ok(node)
    }

    pub(super) unsafe fn dealloc_node(&mut self, node: NonNull<raw::Node<KeyValue<K, V>>>) {
        unsafe {
            self.alloc
                .deallocate(node.cast(), Layout::new::<raw::Node<KeyValue<K, V>>>());

            self.len -= 1;
        }
    }

    #[inline]
    fn clean_recursive(&mut self, node: NonNull<raw::Node<KeyValue<K, V>>>) {
        if let Some(left) = unsafe { node.as_ref().left } {
            self.clean_recursive(left);
        }
        if let Some(right) = unsafe { node.as_ref().right } {
            self.clean_recursive(right);
        }

        let value = unsafe { core::ptr::read(node.as_ref().value()) };
        unsafe { self.dealloc_node(node) };
        drop(value);
    }

    /// Cleans up the tree, deallocating all nodes and their values.
    pub fn clear(&mut self) {
        if let Some(root) = self.raw.root() {
            self.clean_recursive(root);
            self.mark_clear();
        }
    }

    #[inline]
    /// Marks the tree as cleared, without deallocating any nodes.
    pub(super) fn mark_clear(&mut self) {
        self.len = 0;
        self.raw.clear();
    }

    pub(super) fn entry(&mut self, key: K) -> RBTreeEntry<'_, K, V, A>
    where
        K: QueryFor<K>,
    {
        let (node, parent, direction) = self.raw.search(&key);

        if let Some(mut node) = node {
            RBTreeEntry::Occupied {
                node: unsafe { node.as_mut() },
            }
        } else {
            RBTreeEntry::Vacant {
                tree: self,
                key,
                parent,
                direction,
            }
        }
    }

    /// Inserts a key-value pair into the tree, returning the old value if it already existed, or an allocator error if allocation fails.
    #[inline]
    pub fn try_insert(&mut self, key: K, value: V) -> Result<Option<V>, AllocError>
    where
        K: QueryFor<K>,
    {
        let ent = self.entry(key);
        ent.try_replace(value)
    }

    #[inline]
    /// Returns the value associated with the given key, if it exists.
    pub fn get<Q: QueryFor<K>>(&self, key: &Q) -> Option<&V> {
        let (node, _, _) = self.raw.search(key);
        node.map(|n| unsafe { &n.as_ref().value().value })
    }

    #[inline]
    /// Returns the value associated with the given key, if it exists.
    pub fn get_mut<Q: QueryFor<K>>(&mut self, key: &Q) -> Option<&mut V> {
        let (node, _, _) = self.raw.search(key);
        node.map(|mut n| unsafe { &mut n.as_mut().value_mut().value })
    }

    /// Removes a key-value pair from the tree, returning the old value if it existed.
    pub fn remove<Q: QueryFor<K>>(&mut self, key: &Q) -> Option<V> {
        let (node, _, _) = self.raw.search(key);
        if let Some(node) = node {
            unsafe {
                let value = core::ptr::read(node.as_ref().value());
                self.raw.remove_node(node);
                // read the value before the removal
                self.dealloc_node(node);
                Some(value.value)
            }
        } else {
            None
        }
    }

    #[inline]
    /// Returns the number of key-value pairs in the tree.
    pub const fn len(&self) -> usize {
        self.len
    }

    #[inline]
    /// Returns whether the tree is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}
