use core::{
    mem::MaybeUninit,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::boxed::Box;

use crate::{
    scheduler::wait_queue::{PendingWait, WaitQueue},
    sockets::{
        SocketError,
        conn::{SocketClientConn, SocketServerConn},
    },
    utils::locks::Mutex,
};

/// A request for a new connection.
pub(super) struct ListenRequest {
    fill: MaybeUninit<SocketClientConn>,
    request_rejected: bool,
    client_can_block: bool,
}

impl ListenRequest {
    pub fn new(client_can_block: bool) -> Box<Self> {
        Box::new(ListenRequest {
            fill: MaybeUninit::uninit(),
            request_rejected: true, /* assume request is rejected by default */
            client_can_block,
        })
    }

    pub fn take(self) -> Option<SocketClientConn> {
        if self.request_rejected {
            None
        } else {
            // Safety: The request is not rejected, so the fill is initialized, we assume the request is rejected by default.
            unsafe { Some(self.fill.assume_init()) }
        }
    }

    pub fn as_non_null(&self) -> NonNull<ListenRequest> {
        NonNull::from(self)
    }
}

pub(super) enum ListenerWaitReason {
    ServerSleeping,
    ListenRequest(NonNull<ListenRequest>),
}

/// A queue of pending connections for a listening socket.
pub(super) struct ListenQueue {
    max: AtomicUsize,
    requests_count: AtomicUsize,
    wait_queue: Mutex<WaitQueue<1, ListenerWaitReason>>,
}

impl ListenQueue {
    pub const fn new(max: usize) -> Self {
        ListenQueue {
            max: AtomicUsize::new(max),
            requests_count: AtomicUsize::new(0),
            wait_queue: Mutex::new(WaitQueue::new()),
        }
    }

    pub fn can_hold_one(&self) -> bool {
        self.requests_count.load(Ordering::Acquire) < self.max.load(Ordering::Acquire)
    }

    /// Attempts to accept one connection if it fails returns a [`PendingWait`] so the server can sleep otherwise returns a connection.
    pub fn accept_one(
        &self,
        create_connection: impl Fn(bool) -> (SocketServerConn, SocketClientConn),
    ) -> Result<SocketServerConn, PendingWait<'_, 1, ListenerWaitReason, ()>> {
        let mut pending_wait = self.wait_queue.prepare_wait();
        match pending_wait
            .wait_queue_mut()
            .try_pop_one(|reason| match reason {
                ListenerWaitReason::ListenRequest(req_ptr) => {
                    let req = unsafe { req_ptr.as_mut() };
                    let client_can_block = req.client_can_block;

                    let (server_conn, client_conn) = create_connection(client_can_block);
                    req.fill = MaybeUninit::new(client_conn);
                    req.request_rejected = false;
                    Some(server_conn)
                }
                ListenerWaitReason::ServerSleeping => None,
            }) {
            Some(r) => Ok(r),
            None => Err(pending_wait),
        }
    }

    /// Push a new request onto the queue.
    ///
    /// Please drop any locks on self before calling this.
    pub fn wait_for_accept(&self, req: NonNull<ListenRequest>) -> Result<(), SocketError> {
        if !self.can_hold_one() {
            return Err(SocketError::ConnectionRefused);
        }

        let mut pending_wait = self.wait_queue.prepare_wait();
        pending_wait
            .wait_queue_mut()
            .wake_on_condition(|reason| matches!(reason, ListenerWaitReason::ServerSleeping));

        Ok(pending_wait.enter_wait(ListenerWaitReason::ListenRequest(req))?)
    }

    /// Call when the socket is dropped.
    pub fn on_drop(&self) {
        self.wait_queue.lock().wake_on_condition(|req| {
            match req {
                ListenerWaitReason::ListenRequest(req) => unsafe {
                    req.as_mut().request_rejected = true
                },
                ListenerWaitReason::ServerSleeping => {}
            }
            true
        });
    }

    pub fn set_backlog(&self, backlog: usize) {
        self.max.store(backlog, Ordering::Release);
    }
}
