use core::ops::Deref;

use alloc::sync::Arc;

use crate::{
    process::resources::{Resource, generic_clone_impl},
    sockets::{
        SOCKET_ABSTRACT_BINDINGS, SockID, Socket, SocketDomain, SocketError, SocketKind,
        conn::{SocketClientConn, SocketServerConn},
        remove_socket,
    },
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

    /// Configures the listen queue to be able to hold `backlog` connection requests
    pub fn configure_listen_queue(&self, backlog: usize) -> Result<(), SocketError> {
        self.connection_state()
            .map(|state| state.configure_listen_queue(backlog))
            .ok_or(SocketError::OperationNotSupported)
    }

    /// As the server, accept a connection from the listening Queue
    pub fn accept(&self) -> Result<SocketServerConn, SocketError> {
        let conn_state = self
            .connection_state()
            .ok_or(SocketError::OperationNotSupported)?;
        conn_state.accept(&self.reference)
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
        self.reference
            .connection_state()
            .ok_or(SocketError::ConnectionRefused)?
            .connect(can_block)
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
