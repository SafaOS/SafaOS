use alloc::sync::Arc;
use hashbrown::HashMap;
use lazy_static::lazy_static;
use safa_abi::errors::IntoErr;

use crate::{
    sockets::{
        conn_queue::SocketConnQueue,
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
}

impl IntoErr for SocketError {
    fn into_err(self) -> safa_abi::errors::ErrorStatus {
        match self {
            Self::WouldBlockEmpty | Self::WouldBlockFull | Self::WouldBlockNoConnectionRequests => {
                safa_abi::errors::ErrorStatus::WouldBlock
            }
            Self::ConnectionRefused => safa_abi::errors::ErrorStatus::ConnectionRefused,
            Self::ConnectionClosed => safa_abi::errors::ErrorStatus::ConnectionClosed,
        }
    }
}

pub struct Socket {
    can_block: bool,
    sock_queue: SocketConnQueue,
    listen_queue: Mutex<ListenQueue>,
}

impl Socket {
    fn before_drop(&self) {
        let mut listen_queue = self.listen_queue.lock();

        // Stop all the existing connections
        self.sock_queue.drop_all_connections();
        listen_queue.on_drop();
    }

    pub const fn can_block(&self) -> bool {
        self.can_block
    }

    pub const fn sock_type(&self) -> SocketKind {
        match self.sock_queue {
            SocketConnQueue::SeqPacket(_) => SocketKind::SeqPacket,
            SocketConnQueue::Stream(_) => SocketKind::Stream,
        }
    }

    pub const fn domain(&self) -> SocketDomain {
        SocketDomain::Unix
    }

    pub fn disconnect(&self, id: SockConnID) {
        self.sock_queue.remove_connection(id);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketKind {
    SeqPacket,
    Stream,
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
        _ = domain;

        let id = self.next_id;
        let sock = Socket {
            listen_queue: Mutex::new(ListenQueue::new(0)),
            can_block,
            sock_queue: match kind {
                SocketKind::SeqPacket => SocketConnQueue::new_seq_packet(),
                SocketKind::Stream => SocketConnQueue::new_stream(),
            },
        };

        let reference = Arc::new(sock);
        self.sockets.insert(id, reference.clone());
        self.next_id += 1;
        ServerSocketDesc::new(reference, id)
    }

    fn remove_socket(&mut self, socket_id: SockID) -> bool {
        if let Some(s) = self.sockets.remove(&socket_id) {
            s.sock_queue.drop_all_connections();
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
