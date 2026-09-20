mod rbtree;

use core::{borrow::Borrow, cmp::Ordering};

pub use rbtree::*;

/// A query trait for searching a collection by key.
///
/// K is the raw value type stored in the tree.
pub trait QueryFor<K> {
    /// Match this against key, returning an [`Ordering`] indicating how the key compares to the value.
    fn compare(&self, key: &K) -> Ordering;
}

/// Satisfy the idiomatic Borrow<K> search for collections
impl<Q: Borrow<K>, K: Ord> QueryFor<K> for Q {
    fn compare(&self, key: &K) -> Ordering {
        self.borrow().cmp(key)
    }
}
