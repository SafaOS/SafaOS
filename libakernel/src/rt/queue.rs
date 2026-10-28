//! A queue for building intrusive futures' wait queues upon.
//!
//! This is heavily inspired by the intrisive futures crate.
//!
//! TODO: rework this, this is a prrof of concept for now.
use core::{
    cell::UnsafeCell,
    marker::{PhantomData, PhantomPinned},
    mem::offset_of,
    pin::Pin,
    ptr::NonNull,
    task::{self, Waker},
};

use crate::{
    collections::{LList, LListNode},
    sync::{IntSpinLock, SpinLockGuard},
};

#[derive(Debug, Default)]
#[repr(C)]
/// The state of which a [`WaitQueueNode`] is in.
pub enum NodeState<N = ()> {
    /// The node is newly created and not yet used.
    #[default]
    New,
    /// The node is waiting for a notification.
    Waiting(Waker),
    /// The node has got an unhandled notification.
    Notified(N),
    /// The node has acknowledged the notification.
    Ready,
}

#[derive(Debug)]
#[repr(C)]
/// A node in a [`WaitQueue`].
pub struct RawNode<N = ()> {
    list: LListNode<()>,
    pub state: NodeState<N>,
}

const _: () = assert!(offset_of!(RawNode<()>, list) == 0);

impl<N> RawNode<N> {
    pub const fn new() -> Self {
        Self {
            list: LListNode::new(()),
            state: NodeState::New,
        }
    }
}

unsafe impl<N: Send> Send for RawNode<N> {}
unsafe impl<N: Sync> Sync for RawNode<N> {}

#[derive(Debug)]
// A Generic [`WaitQueue`], takes a notification type.
pub struct WaitQueue<N = ()> {
    llist: IntSpinLock<LList<()>>,
    _p: PhantomData<RawNode<N>>,
}

impl<N> WaitQueue<N> {
    pub const fn new() -> Self {
        Self {
            llist: IntSpinLock::new(LList::new()),
            _p: PhantomData,
        }
    }

    #[inline(always)]
    pub fn lock<'a>(&'a self) -> LockedWaitQueue<'a, N> {
        LockedWaitQueue {
            _queue: self,
            llist: self.llist.lock(),
        }
    }
}

#[derive(Debug)]
pub struct LockedWaitQueue<'a, N> {
    _queue: &'a WaitQueue<N>,
    llist: SpinLockGuard<'a, LList<()>>,
}

impl<'a, N> LockedWaitQueue<'a, N> {
    pub unsafe fn remove_and_then<R, F: FnOnce(Option<N>, Self) -> R>(
        mut self,
        waiter: Pin<&mut Waiter<N>>,
        then: F,
    ) -> R {
        let waiter_mut = unsafe { waiter.get_unchecked_mut() };

        let list = &mut self.llist;

        let node = waiter_mut.node.get();
        let old = unsafe { core::mem::replace(&mut (*node).state, NodeState::Ready) };
        match old {
            NodeState::Waiting(w) => {
                unsafe { list.remove_ptr(NonNull::new_unchecked(node).cast()) };
                let r = then(None, self);
                drop(w);
                r
            }
            NodeState::Notified(n) => then(Some(n), self),
            _ => then(None, self),
        }
    }

    pub unsafe fn remove(self, waiter: Pin<&mut Waiter<N>>) {
        unsafe {
            self.remove_and_then(waiter, |_notif, _node| {
                drop(_node);
                drop(_notif);
            })
        }
    }

    /// Notifies the first node in the queue
    pub fn notify_one_with(mut self, notification: N) -> bool {
        unsafe {
            let list = &mut self.llist;
            let head = list.pop_ptr_front().map(|h| h.cast::<RawNode<N>>());
            if let Some(mut h) = head {
                let state = &mut h.as_mut().state;
                let NodeState::Waiting(w) =
                    core::mem::replace(state, NodeState::Notified(notification))
                else {
                    unreachable!("Shouldn't be called on a none waiting Node")
                };

                // Drop list first
                drop(self);
                w.wake();
                true
            } else {
                false
            }
        }
    }

    /// Polls the waiter for a notification
    pub unsafe fn poll(
        &mut self,
        waiter: Pin<&mut Waiter<N>>,
        cx: &mut task::Context,
    ) -> core::task::Poll<N> {
        let waiter_mut = unsafe { waiter.get_unchecked_mut() };

        let node = waiter_mut.node.get();
        let node_ptr = unsafe { NonNull::new_unchecked(node) };
        let llist = &mut self.llist;

        match unsafe { &(&*node).state } {
            NodeState::New => {
                unsafe { (*node).state = NodeState::Waiting(cx.waker().clone()) };

                let list_ptr = node_ptr.cast::<LListNode<()>>();
                unsafe { llist.push_ptr_back(list_ptr) };
                core::task::Poll::Pending
            }
            NodeState::Notified(_) => {
                let NodeState::Notified(n) =
                    core::mem::replace(unsafe { &mut (*node).state }, NodeState::Ready)
                else {
                    unreachable!()
                };

                // Shouldn't be in the list
                core::task::Poll::Ready(n)
            }
            NodeState::Waiting(waker) => {
                if !waker.will_wake(cx.waker()) {
                    unsafe { (*node).state = NodeState::Waiting(cx.waker().clone()) };
                }

                core::task::Poll::Pending
            }
            NodeState::Ready => unreachable!("Node shouldn't be ready before poll()"),
        }
    }
}

impl LockedWaitQueue<'_, ()> {
    #[inline(always)]
    /// Returns whether the queue has any waiters.
    pub fn is_empty(&self) -> bool {
        self.llist.is_empty()
    }

    /// Notifies the first node in the queue
    pub fn notify_one(self) -> bool {
        self.notify_one_with(())
    }

    pub unsafe fn enqueue(&mut self, waiter: Pin<&mut Waiter<()>>, cx: &mut task::Context) {
        let waiter_mut = unsafe { waiter.get_unchecked_mut() };

        let node = waiter_mut.node.get();
        let node_ptr = unsafe { NonNull::new_unchecked(node) };
        let llist = &mut self.llist;

        match unsafe { &(&*node).state } {
            NodeState::Waiting(waker) => {
                if !waker.will_wake(cx.waker()) {
                    unsafe { (*node).state = NodeState::Waiting(cx.waker().clone()) };
                }
            }
            NodeState::Ready | NodeState::Notified(()) => {
                unsafe { (*node).state = NodeState::Waiting(cx.waker().clone()) };

                let list_ptr = node_ptr.cast::<LListNode<()>>();
                unsafe { llist.push_ptr_front(list_ptr) };
            }
            NodeState::New => {
                unsafe { (*node).state = NodeState::Waiting(cx.waker().clone()) };

                let list_ptr = node_ptr.cast::<LListNode<()>>();
                unsafe { llist.push_ptr_back(list_ptr) };
            }
        }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct Waiter<N = ()> {
    node: UnsafeCell<RawNode<N>>,
    _pin: PhantomPinned,
}

impl<N> Waiter<N> {
    pub const fn new() -> Self {
        Waiter {
            node: UnsafeCell::new(RawNode::new()),
            _pin: PhantomPinned,
        }
    }
}
unsafe impl<'a, N: Send> Send for Waiter<N> {}
unsafe impl<'a, N> Sync for Waiter<N> {}
