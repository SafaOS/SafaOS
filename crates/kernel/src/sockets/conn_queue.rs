use alloc::sync::Arc;
use hashbrown::HashMap;

use crate::{
    sockets::{
        ListenQueue, ListenRequest, SockConnID, Socket, SocketError, SocketKind,
        conn::{
            GenericSockConn, GenericSockConnTrait, SocketClientConn, SocketConn,
            SocketSeqPacketConn, SocketServerConn, SocketStreamConn,
        },
        listener::ListenerWaitReason,
    },
    utils::locks::RwLock,
};

pub struct GenericSockConnQueue<T: GenericSockConnTrait> {
    connections: HashMap<SockConnID, Arc<GenericSockConn<T>>>,
    next_conn_id: SockConnID,
}

impl<T: GenericSockConnTrait> GenericSockConnQueue<T> {
    fn new() -> Self {
        Self {
            connections: HashMap::new(),
            next_conn_id: 0,
        }
    }

    fn connect(&mut self) -> (Arc<GenericSockConn<T>>, SockConnID) {
        let id = self.next_conn_id;
        let connection = Arc::new(GenericSockConn::new());
        self.connections.insert(id, connection.clone());
        self.next_conn_id += 1;
        (connection, id)
    }

    fn remove_connection(&mut self, conn_id: SockConnID) {
        if let Some(r) = self.connections.remove(&conn_id) {
            r.mark_dropped()
        } else {
            // Connection is Already removed
        };
    }

    fn drop_all_connections(&mut self) {
        for (_, conn) in self.connections.iter() {
            conn.mark_dropped();
        }

        self.connections.clear();
    }
}

/// Represents all possible socket connection queues.
pub enum SocketConnQueue {
    Stream(RwLock<GenericSockConnQueue<SocketStreamConn>>),
    SeqPacket(RwLock<GenericSockConnQueue<SocketSeqPacketConn>>),
}

impl SocketConnQueue {
    pub fn new_stream() -> Self {
        Self::Stream(RwLock::new(GenericSockConnQueue::new()))
    }

    pub fn new_seq_packet() -> Self {
        Self::SeqPacket(RwLock::new(GenericSockConnQueue::new()))
    }

    pub(super) fn connect(&self) -> (SocketConn, SockConnID) {
        match self {
            Self::Stream(s) => {
                let (conn, key) = s.write().connect();
                (SocketConn::Stream(conn), key)
            }
            Self::SeqPacket(seq) => {
                let (conn, key) = seq.write().connect();
                (SocketConn::SeqPacket(conn), key)
            }
        }
    }

    pub(super) fn remove_connection(&self, id: SockConnID) {
        match self {
            Self::Stream(s) => s.write().remove_connection(id),
            Self::SeqPacket(seq) => seq.write().remove_connection(id),
        }
    }

    pub(super) fn drop_all_connections(&self) {
        match self {
            Self::SeqPacket(seq) => seq.write().drop_all_connections(),
            Self::Stream(s) => s.write().drop_all_connections(),
        }
    }
}

impl Drop for SocketConnQueue {
    fn drop(&mut self) {
        self.drop_all_connections();
    }
}

/// Representation of the inner state of a connection-oriented socket.
pub(super) struct ConnOrientedSocket {
    pub(super) conn_queue: SocketConnQueue,
    listen_queue: ListenQueue,
}

impl ConnOrientedSocket {
    /// Creates a new connection-oriented socket state based on the provided socket kind, panicks if the kind is connectionless.
    pub fn new(kind: SocketKind) -> Self {
        let listen_queue = ListenQueue::new(0);
        let conn_queue = match kind {
            SocketKind::Stream => SocketConnQueue::new_stream(),
            SocketKind::SeqPacket => SocketConnQueue::new_seq_packet(),
            SocketKind::Datagram => unreachable!("Datagram sockets are connection less"),
        };
        Self {
            conn_queue,
            listen_queue,
        }
    }

    pub const fn ty(&self) -> SocketKind {
        match &self.conn_queue {
            SocketConnQueue::Stream(_) => SocketKind::Stream,
            SocketConnQueue::SeqPacket(_) => SocketKind::SeqPacket,
        }
    }

    pub fn on_drop(&self) {
        self.conn_queue.drop_all_connections();
        self.listen_queue.on_drop();
    }

    /// Creates a new socket connection returning both directions
    fn create_connection(
        &self,
        client_can_block: bool,
        socket_ref: Arc<Socket>,
    ) -> (SocketServerConn, SocketClientConn) {
        let server_can_block = socket_ref.can_block();
        let (inner, id) = self.conn_queue.connect();
        (
            SocketServerConn::new(inner.clone(), id, socket_ref.clone(), server_can_block),
            SocketClientConn::new(inner, id, socket_ref, client_can_block),
        )
    }

    /// Configures the listen queue to be able to hold `backlog` connection requests
    pub fn configure_listen_queue(&self, backlog: usize) {
        self.listen_queue.set_backlog(backlog);
    }

    /// As the server, accept a connection from the listening Queue
    pub fn accept(&self, socket_ref: &Arc<Socket>) -> Result<SocketServerConn, SocketError> {
        let server_can_block = socket_ref.can_block();

        match self
            .listen_queue
            .accept_one(|can_block| self.create_connection(can_block, socket_ref.clone()))
        {
            Ok(server_conn) => Ok(server_conn),
            Err(pending_wait) => {
                // Once a connection is available
                if server_can_block {
                    pending_wait.enter_wait(ListenerWaitReason::ServerSleeping)?;
                    self.accept(socket_ref)
                } else {
                    Err(SocketError::WouldBlockNoConnectionRequests)
                }
            }
        }
    }

    /// As a client connect with the server
    /// returns an Error if the server dropped the socket while we were trying to connect
    pub fn connect(&self, can_block: bool) -> Result<SocketClientConn, SocketError> {
        if !self.listen_queue.can_hold_one() {
            return Err(SocketError::ConnectionRefused);
        }

        // Create stuff in the higher half
        let request = ListenRequest::new(can_block);
        self.listen_queue.wait_for_accept(request.as_non_null())?;

        request.take().ok_or(SocketError::ConnectionRefused)
    }
}
