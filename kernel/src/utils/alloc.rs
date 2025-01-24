use core::marker::PhantomData;
use core::ops::RangeBounds;
use core::ptr::NonNull;
use core::str;

use crate::memory::page_allocator::{PageAlloc, GLOBAL_PAGE_ALLOCATOR};
use crate::memory::{align_up, paging::PAGE_SIZE};
use alloc::boxed::Box;
use alloc::str::pattern::{Pattern, ReverseSearcher};
use alloc::vec::{Drain, Vec};

pub struct PageVec<T> {
    inner: Vec<T, PageAlloc>,
}

impl<T> PageVec<T> {
    pub fn new() -> Self {
        Self {
            inner: Vec::new_in(&*GLOBAL_PAGE_ALLOCATOR),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Vec::with_capacity_in(capacity, &*GLOBAL_PAGE_ALLOCATOR),
        }
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn reserve(&mut self, additional: usize) {
        let additional = align_up(additional, PAGE_SIZE / core::mem::size_of::<T>());
        self.inner.reserve(additional);
    }

    pub fn extend_from_slice(&mut self, other: &[T])
    where
        T: Clone,
    {
        if self.inner.capacity() == self.inner.len() {
            self.reserve(other.len());
        }
        self.inner.extend_from_slice(other);
    }

    pub fn truncate(&mut self, len: usize) {
        self.inner.truncate(len);
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn drain<R: RangeBounds<usize>>(&mut self, range: R) -> Drain<'_, T, PageAlloc> {
        self.inner.drain(range)
    }
}

impl<T> core::ops::Deref for PageVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> From<Vec<T, PageAlloc>> for PageVec<T> {
    fn from(v: Vec<T, PageAlloc>) -> Self {
        Self { inner: v }
    }
}

pub struct PageString {
    pub inner: PageVec<u8>,
}

impl PageString {
    pub fn new() -> Self {
        Self {
            inner: PageVec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: PageVec::with_capacity(capacity),
        }
    }

    #[inline]
    pub fn push_str(&mut self, s: &str) {
        self.inner.extend_from_slice(s.as_bytes());
    }

    pub fn push_char(&mut self, c: char) {
        let mut dst = [0; 4];
        let fake_str = c.encode_utf8(&mut dst);
        self.push_str(fake_str);
    }

    pub fn pop(&mut self) -> Option<char> {
        let char = self.as_str().chars().next_back()?;
        self.inner.truncate(self.len() - char.len_utf8());
        Some(char)
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.inner) }
    }

    pub fn ends_with<P>(&self, other: P) -> bool
    where
        P: Pattern,
        for<'a> P::Searcher<'a>: ReverseSearcher<'a>,
    {
        self.as_str().ends_with(other)
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl serde_json::io::Write for PageVec<u8> {
    fn write(&mut self, buf: &[u8]) -> serde_json::io::Result<usize> {
        self.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> serde_json::io::Result<()> {
        Ok(())
    }

    fn write_all(&mut self, buf: &[u8]) -> serde_json::io::Result<()> {
        self.extend_from_slice(buf);
        Ok(())
    }
}
impl serde_json::io::Write for PageString {
    fn write(&mut self, buf: &[u8]) -> serde_json::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> serde_json::io::Result<()> {
        self.inner.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> serde_json::io::Result<()> {
        self.inner.write_all(buf)
    }
}

struct LinkedListNode<T> {
    inner: T,
    next: Option<NonNull<Self>>,
    prev: Option<NonNull<Self>>,
    marker: PhantomData<Box<Self>>,
}

/// An Iterator like LinkedList
pub struct LinkedList<T> {
    head: Option<NonNull<LinkedListNode<T>>>,
    tail: Option<NonNull<LinkedListNode<T>>>,
    current: Option<NonNull<LinkedListNode<T>>>,
    prev: Option<NonNull<LinkedListNode<T>>>,

    len: usize,
}

impl<T> LinkedList<T> {
    pub fn new() -> Self {
        Self {
            head: None,
            current: None,
            prev: None,
            tail: None,
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Pushes a value to the end of the list.
    pub fn push(&mut self, value: T) {
        let node = Box::new(LinkedListNode {
            inner: value,
            next: None,
            prev: None,
            marker: PhantomData,
        });
        let node_ptr = NonNull::from(Box::leak(node));
        unsafe {
            self.push_node(node_ptr);
        }
    }

    unsafe fn push_node(&mut self, node: NonNull<LinkedListNode<T>>) {
        if let Some(tail) = self.tail {
            (*tail.as_ptr()).next = Some(node);
            (*node.as_ptr()).prev = Some(tail);

            self.tail = Some(node);
        } else {
            // initializes the list if it is empty
            self.head = Some(node);
            self.tail = Some(node);
            self.current = Some(node);
        }
        self.len += 1;
    }

    unsafe fn remove_node(&mut self, node: NonNull<LinkedListNode<T>>) -> T {
        let next = (*node.as_ptr()).next;
        let prev = (*node.as_ptr()).prev;

        if let Some(next) = next {
            (*next.as_ptr()).prev = prev;
        }

        if let Some(prev) = prev {
            (*prev.as_ptr()).next = next;
        }

        if self.head.is_some_and(|head| head == node) {
            self.head = next;
        }

        if self.tail.is_some_and(|tail| tail == node) {
            self.tail = prev;
        }

        if self.current.is_some_and(|c| c == node) {
            self.current = prev;
        }

        self.len -= 1;
        let results = Box::from_non_null(node);
        results.inner
    }

    /// removes the first element where `condition` on returns true
    /// returns the removed element
    pub fn remove_where<C>(&mut self, condition: C) -> Option<T>
    where
        C: Fn(&mut T) -> bool,
    {
        let mut current_node = self.head;

        while let Some(node) = current_node {
            unsafe {
                if condition(&mut (*node.as_ptr()).inner) {
                    return Some(self.remove_node(node));
                }
                current_node = (*node.as_ptr()).next;
            }
        }
        None
    }

    pub fn next(&mut self) -> Option<&mut T> {
        let current = self.current?;
        unsafe {
            if let Some(node) = (*current.as_ptr()).next {
                self.current = Some(node);
                Some(&mut (*node.as_ptr()).inner)
            } else {
                None
            }
        }
    }
    /// same as `Self::next` but wraps around back to the start if it reaches the end
    /// returns `None` if the list is empty
    pub fn next_wrap(&mut self) -> Option<&mut T> {
        let current = self.current?;
        unsafe {
            if let Some(node) = (*current.as_ptr()).next {
                self.current = Some(node);
                Some(&mut (*current.as_ptr()).inner)
            } else {
                self.current = self.head;
                Some(&mut (*current.as_ptr()).inner)
            }
        }
    }

    #[allow(dead_code)]
    pub fn current(&self) -> Option<&T> {
        let current = self.current?;
        unsafe { Some(&(*current.as_ptr()).inner) }
    }

    pub fn last(&self) -> Option<&T> {
        let last = self.tail?;
        unsafe { Some(&(*last.as_ptr()).inner) }
    }

    #[allow(dead_code)]
    pub fn last_mut(&mut self) -> Option<&mut T> {
        let last = self.tail?;
        unsafe { Some(&mut (*last.as_ptr()).inner) }
    }

    pub fn current_mut(&mut self) -> Option<&mut T> {
        let current = self.current?;
        unsafe { Some(&mut (*current.as_ptr()).inner) }
    }

    /// returns an iterator that 'continues' the list which means calling `next` on the iterator
    /// would be the same as calling `next_wrap` on the list, this iterator muttates the list...
    pub fn continue_iter(&mut self) -> LinkedListContinue<T> {
        LinkedListContinue { list: self }
    }

    /// returns an iterator that acts like a clone of the list
    /// iterating over the list will yield the same values as iterating over the original list
    pub fn clone_iter(&self) -> LinkedListCloneIter<T> {
        let list = Self {
            head: self.head,
            tail: self.tail,
            current: self.head,
            prev: self.prev,
            len: self.len,
        };

        LinkedListCloneIter {
            list,
            marker: PhantomData,
        }
    }

    /// returns an iterator that acts like a clone of the list
    /// iterating over the list will yield the same values as iterating over the original list
    pub fn clone_iter_mut(&mut self) -> LinkedListCloneIterMut<T> {
        let list = Self {
            head: self.head,
            tail: self.tail,
            current: self.head,
            prev: self.prev,
            len: self.len,
        };

        LinkedListCloneIterMut {
            list,
            marker: PhantomData,
        }
    }
}

unsafe impl<T: Send> Send for LinkedList<T> {}
unsafe impl<T: Sync> Sync for LinkedList<T> {}

/// This `struct` is created by the [`LinkedList::clone_iter`] method
/// this does not muttate the original list it is a clone of the original list
pub struct LinkedListCloneIter<'a, T: 'a> {
    list: LinkedList<T>,
    marker: PhantomData<&'a LinkedList<T>>,
}

impl<'a, T> Iterator for LinkedListCloneIter<'a, T> {
    type Item = &'a T;
    fn next(&mut self) -> Option<Self::Item> {
        let it = self.list.current?;
        // TODO: this is a hack to prevent the iterator from being used after it has been finished
        if self.list.next().is_none() {
            self.list.current = None;
        }

        unsafe { Some(&(*it.as_ptr()).inner) }
    }
}

/// This `struct` is created by the [`LinkedList::clone_iter_mut`] method
/// this does not muttate the original list it is a clone of the original list
pub struct LinkedListCloneIterMut<'a, T: 'a> {
    list: LinkedList<T>,
    marker: PhantomData<&'a mut LinkedList<T>>,
}

impl<'a, T> Iterator for LinkedListCloneIterMut<'a, T> {
    type Item = &'a mut T;
    fn next(&mut self) -> Option<Self::Item> {
        let it = self.list.current?;
        // TODO: this is a hack to prevent the iterator from being used after it has been finished
        if self.list.next().is_none() {
            self.list.current = None;
        }

        unsafe { Some(&mut (*it.as_ptr()).inner) }
    }
}
/// This `struct` is created by the [`LinkedList::iter_mut`] method
/// it provides a wrap_around iterator over the elements of a `LinkedList`. which means it warps around
/// when it reaches the end of the list.to the head of the list for now
/// this muttates the original list
pub struct LinkedListContinue<'a, T: 'a> {
    list: &'a mut LinkedList<T>,
}

impl<'a, T> Iterator for LinkedListContinue<'a, T> {
    type Item = &'a mut T;
    fn next(&mut self) -> Option<Self::Item> {
        self.list.next_wrap();
        unsafe { Some(&mut (*self.list.current?.as_ptr()).inner) }
    }
}
