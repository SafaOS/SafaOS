use core::{marker::PhantomData, ptr::NonNull};

use alloc::{boxed::Box, sync::Arc};

use crate::threading::{cpu_context::Thread, task::Task};

struct Node<T> {
    inner: T,
    next: Option<NonNull<Node<T>>>,
    prev: Option<NonNull<Node<T>>>,
}

#[derive(Debug)]
pub struct SchedulerQueue<T> {
    head: Option<NonNull<Node<T>>>,
    current: Option<NonNull<Node<T>>>,
    tail: Option<NonNull<Node<T>>>,
    len: usize,
}

impl<T> SchedulerQueue<T> {
    pub const fn new() -> Self {
        Self {
            head: None,
            current: None,
            tail: None,
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    /// Advances the current task pointer in a circular manner, returning a reference to the next node which is now the current node
    /// returns None only if the queue is empty.
    pub fn advance_circular(&mut self) -> Option<&T> {
        if let Some(current) = self.current.take() {
            let current_ref = unsafe { current.as_ref() };
            if let Some(next) = current_ref.next {
                self.current = Some(next);
            } else {
                self.current = self.head;
            }
            self.current()
        } else {
            None
        }
    }

    pub fn current(&self) -> Option<&T> {
        if let Some(current) = self.current {
            let current_ref = unsafe { current.as_ref() };
            Some(&current_ref.inner)
        } else {
            None
        }
    }

    pub fn push_back(&mut self, item: T) {
        let node = Box::new(Node {
            inner: item,
            next: None,
            prev: None,
        });

        let mut node_ptr = NonNull::new(Box::into_raw(node)).unwrap();
        let node_ref = unsafe { node_ptr.as_mut() };

        if let Some(mut tail) = self.tail {
            let tail_ref = unsafe { tail.as_mut() };
            debug_assert!(tail_ref.next.is_none());

            tail_ref.next = Some(node_ptr);
            node_ref.prev = Some(tail);
        } else {
            self.head = Some(node_ptr);
        }

        if self.current.is_none() {
            self.current = Some(node_ptr);
        }
        self.tail = Some(node_ptr);

        self.len += 1;
    }

    pub(super) fn iter<'a>(&'a self) -> SchedulerQueueIter<'a, T> {
        SchedulerQueueIter {
            queue: PhantomData,
            current: self.head.map(|h| unsafe { h.as_ref() }),
        }
    }

    unsafe fn remove_raw_inner(&mut self, mut node_ptr: NonNull<Node<T>>) -> Box<Node<T>> {
        unsafe {
            let node = node_ptr.as_mut();
            let prev_ptr = node.prev.take();
            let next_ptr = node.next.take();

            let prev = prev_ptr.map(|mut prev| prev.as_mut());
            let next = next_ptr.map(|mut next| next.as_mut());

            if let Some(prev) = prev {
                prev.next = next_ptr;
            } else {
                self.head = next_ptr;
            }

            if let Some(next) = next {
                next.prev = prev_ptr;
            } else {
                self.tail = prev_ptr;
            }

            if let Some(current) = self.current
                && current == node_ptr
            {
                self.current = next_ptr;
            }

            self.len -= 1;
            Box::from_non_null(node_ptr)
        }
    }

    pub fn remove_where<F>(&mut self, mut predicate: F) -> Option<T>
    where
        F: FnMut(&T) -> bool,
    {
        let mut current = self.head;
        while let Some(mut node_ptr) = current {
            let node = unsafe { node_ptr.as_mut() };
            current = node.next;
            if predicate(&node.inner) {
                return Some(unsafe { (*self.remove_raw_inner(node_ptr)).inner });
            }
        }
        None
    }
}

unsafe impl<T: Send> Send for SchedulerQueue<T> {}
unsafe impl<T: Sync> Sync for SchedulerQueue<T> {}

pub type ThreadQueue = SchedulerQueue<Arc<Thread>>;
pub type TaskQueue = SchedulerQueue<Arc<Task>>;

impl TaskQueue {}

pub(super) struct SchedulerQueueIter<'a, T: 'a> {
    queue: PhantomData<&'a SchedulerQueue<T>>,
    current: Option<&'a Node<T>>,
}

impl<'a, T: 'a> Iterator for SchedulerQueueIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current.take()?;
        self.current = current.next.map(|node| unsafe { node.as_ref() });
        Some(&current.inner)
    }
}
