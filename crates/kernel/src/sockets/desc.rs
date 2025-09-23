use core::ops::Deref;

use alloc::sync::Arc;

use crate::{
    arch::without_interrupts,
    process::resources::{Resource, generic_clone_impl},
    sockets::{
        ListenRequest, SOCKET_ABSTRACT_BINDINGS, SockID, Socket, SocketDomain, SocketError,
        SocketKind,
        conn::{SocketClientConn, SocketServerConn},
        remove_socket,
    },
    thread,
};

/// A reference to the socket from the Server, Only one can exist
///
/// Once dropped the socket is removed
pub struct ServerSocketDesc {
    pub(super) reference: Arc<Socket>,
    pub id: SockID,
}

impl Deref for ServerSocketDesc {
    type Target = Socket;
    fn deref(&self) -> &Self::Target {
        &*self.reference
    }
}

impl Drop for ServerSocketDesc {
    fn drop(&mut self) {
        // Drops all connections and safely informs the listeners that the socket is gone.
        self.before_drop();
        // Remove the socket from the Queue
        remove_socket(self.id);

        // Remove the socket from the bindings
        SOCKET_ABSTRACT_BINDINGS
            .lock()
            .retain(|_, id| *id != self.id);
    }
}

impl ServerSocketDesc {
    pub(super) fn new(reference: Arc<Socket>, id: SockID) -> Self {
        Self { reference, id }
    }

    /// Creates a new socket connection returning both directions
    fn create_connection(&self, client_can_block: bool) -> (SocketServerConn, SocketClientConn) {
        let (inner, id) = self.sock_queue.connect();
        (
            SocketServerConn::new(inner.clone(), id, self.reference.clone(), self.can_block),
            SocketClientConn::new(inner, id, self.reference.clone(), client_can_block),
        )
    }

    /// Configures the listen queue to be able to hold `backlog` connection requests
    pub fn configure_listen_queue(&self, backlog: usize) {
        self.listen_queue.lock().set_backlog(backlog);
    }

    /// As the server, accept a connection from the listening Queue
    pub fn accept(&self) -> Result<SocketServerConn, SocketError> {
        let mut listen_queue = self.listen_queue.lock();
        if let Some(server_conn) =
            listen_queue.accept_one(|can_block| self.create_connection(can_block))
        {
            Ok(server_conn)
        } else {
            // Once a connection is available
            if self.can_block {
                let thread = thread::current();
                without_interrupts(|| {
                    listen_queue.mark_server_sleeping(thread.clone());
                    drop(listen_queue);

                    thread.temp_block_forever();
                    thread::current::yield_now();
                });

                self.accept()
            } else {
                Err(SocketError::WouldBlockNoConnectionRequests)
            }
        }
    }
}

impl Resource for ServerSocketDesc {
    fn address_space_generic(&self) -> bool {
        false
    }
}

/// A client's socket reference descriptor
/// Multiple clients may exists but only one server can exist
pub struct CliSocketDesc {
    reference: Arc<Socket>,
}

impl Deref for CliSocketDesc {
    type Target = Socket;
    fn deref(&self) -> &Self::Target {
        &*self.reference
    }
}

impl CliSocketDesc {
    /// Create a new client socket descriptor
    pub(super) fn new(reference: Arc<Socket>) -> Self {
        Self { reference }
    }

    /// As a client connect with the server
    /// returns an Error if the server dropped the socket while we were trying to connect
    pub fn connect(&self, can_block: bool) -> Result<SocketClientConn, SocketError> {
        let mut queue = self.listen_queue.lock();
        if !queue.can_hold_one() {
            return Err(SocketError::ConnectionRefused);
        }

        // Create stuff in the higher half
        let request = ListenRequest::new(can_block);
        without_interrupts(|| {
            queue.push(request.as_non_null())?;
            drop(queue);
            thread::current::yield_now();
            Ok(())
        })?;

        request.take().ok_or(SocketError::ConnectionRefused)
    }
}

/// A socket descriptor
#[derive(Debug, Clone, Copy)]
pub struct SocketDesc {
    pub domain: SocketDomain,
    pub kind: SocketKind,
    pub can_block: bool,
}

impl Resource for SocketDesc {
    fn try_clone_into_node(
        &self,
        is_global: bool,
    ) -> Result<crate::process::resources::ResourceNodeRef, safa_abi::errors::ErrorStatus> {
        generic_clone_impl(self, is_global)
    }
    fn address_space_generic(&self) -> bool {
        true
    }
}
