use alloc::{boxed::Box, sync::Arc};
use hashbrown::HashMap;
use lazy_static::lazy_static;
use safa_abi::errors::IntoErr;

use crate::{
    sockets::{
        conn::BlockingDatagramStream,
        conn_queue::ConnOrientedSocket,
        desc::{CliSocketDesc, ServerSocketDesc},
        listener::{ListenQueue, ListenRequest},
    },
    utils::{
        locks::{Mutex, RwLock},
        types::Name,
    },
};

pub mod conn;
mod conn_queue;
pub mod desc;
mod listener;

#[cfg(test)]
mod tests;

pub type SockID = u32;
pub type SockConnID = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketDomain {
    Unix,
    Net,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketError {
    /// Should Block because the Socket is empty and we are trying to read from it
    WouldBlockEmpty,
    /// Should Block because the Socket is full and we are trying to append to it
    WouldBlockFull,
    /// Should Block because of an attempted accept while there are no connection requests pending
    WouldBlockNoConnectionRequests,
    /// One side closed the connection
    ConnectionClosed,
    /// Connection refused for some reason
    ConnectionRefused,
    OperationNotSupported,
}

impl IntoErr for SocketError {
    fn into_err(self) -> safa_abi::errors::ErrorStatus {
        match self {
            Self::WouldBlockEmpty | Self::WouldBlockFull | Self::WouldBlockNoConnectionRequests => {
                safa_abi::errors::ErrorStatus::WouldBlock
            }
            Self::ConnectionRefused => safa_abi::errors::ErrorStatus::ConnectionRefused,
            Self::ConnectionClosed => safa_abi::errors::ErrorStatus::ConnectionClosed,
            Self::OperationNotSupported => safa_abi::errors::ErrorStatus::OperationNotSupported,
        }
    }
}

enum SocketState {
    ConnectionOriented(ConnOrientedSocket),
    Connectionless { stream: Box<BlockingDatagramStream> },
}

impl SocketState {
    pub fn on_drop(&self) {
        match self {
            SocketState::ConnectionOriented(state) => state.on_drop(),
            SocketState::Connectionless { stream } => stream.on_drop(),
        }
    }
}

/// Represents a Unix socket.
pub struct Socket {
    can_block: bool,
    sock_state: SocketState,
    domain: SocketDomain,
}

impl Socket {
    fn before_drop(&self) {
        self.sock_state.on_drop()
    }

    pub const fn can_block(&self) -> bool {
        self.can_block
    }

    fn connection_state(&self) -> Option<&ConnOrientedSocket> {
        match &self.sock_state {
            SocketState::ConnectionOriented(socket) => Some(socket),
            _ => None,
        }
    }

    pub const fn sock_type(&self) -> SocketKind {
        match self.sock_state {
            SocketState::ConnectionOriented(ref connection_state) => connection_state.ty(),
            SocketState::Connectionless { .. } => SocketKind::Datagram,
        }
    }

    pub const fn domain(&self) -> SocketDomain {
        self.domain
    }

    pub fn disconnect(&self, id: SockConnID) {
        self.connection_state()
            .map(|state| state.conn_queue.remove_connection(id));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketKind {
    SeqPacket,
    Stream,
    Datagram,
}

struct SockQueue {
    sockets: HashMap<SockID, Arc<Socket>>,
    next_id: SockID,
}

unsafe impl Send for SockQueue {}
unsafe impl Sync for SockQueue {}

impl SockQueue {
    fn new() -> Self {
        Self {
            sockets: HashMap::new(),
            next_id: 0,
        }
    }

    fn create(
        &mut self,
        domain: SocketDomain,
        kind: SocketKind,
        can_block: bool,
    ) -> ServerSocketDesc {
        let id = self.next_id;

        let sock = Socket {
            can_block,
            domain,
            sock_state: match kind {
                SocketKind::SeqPacket | SocketKind::Stream => {
                    SocketState::ConnectionOriented(ConnOrientedSocket::new(kind))
                }
                SocketKind::Datagram => SocketState::Connectionless {
                    stream: Box::new(BlockingDatagramStream::new()),
                },
            },
        };

        let reference = Arc::new(sock);
        self.sockets.insert(id, reference.clone());
        self.next_id += 1;
        ServerSocketDesc::new(reference, id)
    }

    fn remove_socket(&mut self, socket_id: SockID) -> bool {
        if let Some(s) = self.sockets.remove(&socket_id) {
            s.sock_state.on_drop();
            true
        } else {
            false
        }
    }
}
static SOCKET_ABSTRACT_BINDINGS: Mutex<heapless::FnvIndexMap<Name, SockID, 4096>> =
    Mutex::new(heapless::FnvIndexMap::new());

lazy_static! {
    static ref SOCKET_QUEUE: RwLock<SockQueue> = RwLock::new(SockQueue::new());
}

/// Creates a new socket returning a Server Descriptor
pub fn create_socket(domain: SocketDomain, kind: SocketKind, can_block: bool) -> ServerSocketDesc {
    SOCKET_QUEUE.write().create(domain, kind, can_block)
}

pub fn bind_abstract_socket(under_name: Name, id: SockID) {
    SOCKET_ABSTRACT_BINDINGS
        .lock()
        .insert(under_name, id)
        .expect("failed to bind socket");
}

pub fn get_abstract_binding(name: &Name) -> Option<SockID> {
    SOCKET_ABSTRACT_BINDINGS.lock().get(name).copied()
}

/// Removes a socket given it's ID
pub fn remove_socket(id: SockID) -> bool {
    SOCKET_QUEUE.write().remove_socket(id)
}

/// As the client, gets a new reference to the client Socket
pub fn get_client_socket(id: SockID) -> Option<CliSocketDesc> {
    SOCKET_QUEUE
        .read()
        .sockets
        .get(&id)
        .cloned()
        .map(|reference| CliSocketDesc::new(reference))
}
