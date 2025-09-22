use core::{mem::MaybeUninit, ptr::NonNull};

use alloc::boxed::Box;

use crate::{
    scheduler::wait_queue::WaitQueue,
    sockets::{
        SocketError,
        conn::{SocketClientConn, SocketServerConn},
    },
    thread::{self, ArcThread},
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

/// A queue of pending connections for a listening socket.
pub(super) struct ListenQueue {
    max: usize,
    socket_dropped: bool,
    wait_queue: WaitQueue<0, NonNull<ListenRequest>>,
    server_sleeping: Option<ArcThread>,
}

impl ListenQueue {
    pub const fn new(max: usize) -> Self {
        ListenQueue {
            max,
            socket_dropped: false,
            wait_queue: WaitQueue::new(),
            server_sleeping: None,
        }
    }

    pub fn len(&self) -> usize {
        self.wait_queue.len()
    }

    pub fn mark_server_sleeping(&mut self, thread: ArcThread) {
        self.server_sleeping = Some(thread);
    }

    pub fn can_hold_one(&self) -> bool {
        self.len() < self.max
    }

    pub fn accept_one(
        &mut self,
        create_connection: impl FnOnce(bool) -> (SocketServerConn, SocketClientConn),
    ) -> Option<SocketServerConn> {
        self.wait_queue.try_pop_one(|mut req_ptr| {
            let req = unsafe { req_ptr.as_mut() };
            let client_can_block = req.client_can_block;

            let (server_conn, client_conn) = create_connection(client_can_block);
            req.fill = MaybeUninit::new(client_conn);
            req.request_rejected = false;
            server_conn
        })
    }

    /// Push a new request onto the queue.
    ///
    /// Please drop any locks on self and then yield after calling this with interrupts disabled.
    pub fn push(&mut self, req: NonNull<ListenRequest>) -> Result<(), SocketError> {
        if self.len() >= self.max {
            return Err(SocketError::ConnectionRefused);
        }

        thread::current().sleep_in_queue(&mut self.wait_queue, req);
        if let Some(server) = self.server_sleeping.take() {
            server.wake_up();
        }
        // give a chance to drop this before yielding
        Ok(())
    }

    /// Call when the socket is dropped.
    pub fn on_drop(&mut self) {
        self.socket_dropped = true;
        self.wait_queue.wake_on_condition(|req| {
            unsafe { req.as_mut().request_rejected = true }
            true
        });
    }

    pub fn set_backlog(&mut self, backlog: usize) {
        self.max = backlog;
    }
}
