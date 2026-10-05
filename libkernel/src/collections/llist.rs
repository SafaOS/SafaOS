use core::ptr::NonNull;

#[derive(Debug, Clone)]
#[repr(C)]
/// Represents a [`LList`] Node
pub struct LListNode<T> {
    next: Option<NonNull<Self>>,
    prev: Option<NonNull<Self>>,
    pub data: T,
}

impl<T> LListNode<T> {
    pub const fn new(data: T) -> Self {
        Self {
            data,
            next: None,
            prev: None,
        }
    }

    #[inline(always)]
    /// Returns the next item in the list
    pub fn peek_next(&self) -> Option<NonNull<Self>> {
        self.next
    }

    #[inline(always)]
    /// Returns the previous item in the list.
    pub fn peek_prev(&self) -> Option<NonNull<Self>> {
        self.prev
    }

    #[inline(always)]
    /// Sets self.next to ptr
    pub fn set_next(&mut self, ptr: NonNull<Self>) {
        self.next = Some(ptr);
    }

    #[inline(always)]
    /// Sets self.prev to ptr
    pub fn set_prev(&mut self, ptr: NonNull<Self>) {
        self.prev = Some(ptr);
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
/// A double ended linked list that isn't stored anywhere
///
/// All usage of it is unsafe
pub struct LList<T> {
    head: Option<NonNull<LListNode<T>>>,
    tail: Option<NonNull<LListNode<T>>>,
}

impl<T> LList<T> {
    pub const fn new() -> Self {
        Self {
            head: None,
            tail: None,
        }
    }

    /// Adds a node to the queue given its pointer.
    pub const unsafe fn push_ptr_back(&mut self, mut node: NonNull<LListNode<T>>) {
        if let Some(mut tail) = self.tail {
            unsafe {
                tail.as_mut().next = Some(node);
                node.as_mut().prev = Some(tail);
            }
        } else {
            self.head = Some(node);
        }
        self.tail = Some(node);
    }

    /// Adds a node to the queue given its pointer.
    pub const unsafe fn push_ptr_front(&mut self, mut node: NonNull<LListNode<T>>) {
        if let Some(mut head) = self.head {
            unsafe {
                head.as_mut().prev = Some(node);
                node.as_mut().next = Some(head);
            }
        } else {
            self.tail = Some(node);
        }
        self.head = Some(node);
    }

    /// Pops a node from the queue and returns its pointer.
    pub const unsafe fn pop_ptr_front(&mut self) -> Option<NonNull<LListNode<T>>> {
        if let Some(mut head) = self.head {
            unsafe {
                self.head = head.as_mut().next;
                if let Some(mut next) = self.head {
                    next.as_mut().prev = None;
                } else {
                    self.tail = None;
                }
            }
            Some(head)
        } else {
            None
        }
    }

    /// Removes a node given its pointer.
    ///
    /// node has to be in the list
    pub const unsafe fn remove_ptr(&mut self, mut node: NonNull<LListNode<T>>) {
        unsafe {
            if let Some(mut prev) = node.as_mut().prev {
                prev.as_mut().next = node.as_mut().next;
            } else {
                self.head = node.as_mut().next;
            }

            if let Some(mut next) = node.as_mut().next {
                next.as_mut().prev = node.as_mut().prev;
            } else {
                self.tail = node.as_mut().prev;
            }

            node.as_mut().prev = None;
            node.as_mut().next = None;
        }
    }

    #[inline(always)]
    /// Peeks at the head of the queue and returns its pointer.
    pub const fn peek_head(&self) -> Option<NonNull<LListNode<T>>> {
        self.head
    }

    #[inline(always)]
    /// Peeks at the tail of the queue and returns its pointer.
    pub const fn peek_tail(&self) -> Option<NonNull<LListNode<T>>> {
        self.tail
    }
}
