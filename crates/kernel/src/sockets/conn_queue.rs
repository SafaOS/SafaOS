use alloc::sync::Arc;
use hashbrown::HashMap;

use crate::{
    sockets::{
        SockConnID,
        conn::{
            GenericSockConn, GenericSockConnTrait, SocketConn, SocketSeqPacketConn,
            SocketStreamConn,
        },
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
