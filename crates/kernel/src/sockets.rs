use core::{
    mem::MaybeUninit,
    ops::Deref,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use alloc::{boxed::Box, collections::linked_list::LinkedList, sync::Arc, vec::Vec};
use hashbrown::HashMap;
use lazy_static::lazy_static;

use crate::{
    memory::paging::PAGE_SIZE,
    process::current::kernel_thread_spawn,
    thread::{self, Tid},
    utils::{
        locks::{Mutex, RwLock},
        types::Name,
    },
};

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
    /// Attempted to write data that is too large
    TooLarge,
    /// One side closed the connection
    ConnectionClosed,
}

const MAX_STREAM_SIZE: usize = PAGE_SIZE;
struct SocketStreamConn {
    server_buf: heapless::Vec<u8, MAX_STREAM_SIZE>,
    client_buf: heapless::Vec<u8, MAX_STREAM_SIZE>,
}

impl SocketStreamConn {
    pub const fn new() -> Self {
        Self {
            server_buf: heapless::Vec::new(),
            client_buf: heapless::Vec::new(),
        }
    }

    fn read_inner<const IS_SERVER: bool>(&mut self, buf: &mut [u8]) -> Result<usize, SocketError> {
        let to_read_from = if IS_SERVER {
            &mut self.server_buf
        } else {
            &mut self.client_buf
        };
        let max_len = to_read_from.len();

        let read_len = max_len.min(buf.len());
        if read_len == 0 {
            return Err(SocketError::WouldBlockEmpty);
        }

        buf[..read_len].copy_from_slice(&to_read_from[..read_len]);

        to_read_from.copy_within(read_len.., 0);
        to_read_from.truncate(max_len - read_len);
        Ok(read_len)
    }

    fn write_inner<const IS_SERVER: bool>(&mut self, buf: &[u8]) -> Result<(), SocketError> {
        let to_write_to = if IS_SERVER {
            &mut self.server_buf
        } else {
            &mut self.client_buf
        };

        if buf.len() >= MAX_STREAM_SIZE {
            return Err(SocketError::TooLarge);
        }

        to_write_to
            .extend_from_slice(buf)
            .map_err(|()| SocketError::WouldBlockFull)
    }
}

struct SocketSeqPacketConn {
    inner: SocketStreamConn,
    server_packets: LinkedList<usize>,
    client_packets: LinkedList<usize>,
}

impl SocketSeqPacketConn {
    pub const fn new() -> Self {
        Self {
            inner: SocketStreamConn::new(),
            server_packets: LinkedList::new(),
            client_packets: LinkedList::new(),
        }
    }

    fn read_inner<const IS_SERVER: bool>(&mut self, buf: &mut [u8]) -> Result<usize, SocketError> {
        let to_read_from = if IS_SERVER {
            &mut self.server_packets
        } else {
            &mut self.client_packets
        };

        let Some(msg_len) = to_read_from.pop_front() else {
            return Err(SocketError::WouldBlockEmpty);
        };

        let amount = buf.len().min(msg_len);
        self.inner.read_inner::<IS_SERVER>(&mut buf[..amount])
    }

    fn write_inner<const IS_SERVER: bool>(&mut self, buf: &[u8]) -> Result<(), SocketError> {
        let to_write_to = if IS_SERVER {
            &mut self.server_packets
        } else {
            &mut self.client_packets
        };

        self.inner.write_inner::<IS_SERVER>(buf)?;
        to_write_to.push_back(buf.len());
        Ok(())
    }
}

pub type SockID = u32;
pub type SockConnID = u32;

trait GenericSockConnTrait {
    fn new() -> Self
    where
        Self: Sized;

    /// A Write operation
    fn write<const TARGETS_SERVER: bool>(&mut self, buf: &[u8]) -> Result<(), SocketError>;
    /// A Read operation
    fn read<const IS_SERVER: bool>(&mut self, buf: &mut [u8]) -> Result<usize, SocketError>;
}

impl GenericSockConnTrait for SocketStreamConn {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self::new()
    }

    fn read<const IS_SERVER: bool>(&mut self, buf: &mut [u8]) -> Result<usize, SocketError> {
        self.read_inner::<IS_SERVER>(buf)
    }

    fn write<const TARGETS_SERVER: bool>(&mut self, buf: &[u8]) -> Result<(), SocketError> {
        self.write_inner::<TARGETS_SERVER>(buf)
    }
}

impl GenericSockConnTrait for SocketSeqPacketConn {
    fn new() -> Self {
        Self::new()
    }

    fn read<const IS_SERVER: bool>(&mut self, buf: &mut [u8]) -> Result<usize, SocketError> {
        self.read_inner::<IS_SERVER>(buf)
    }

    fn write<const TARGETS_SERVER: bool>(&mut self, buf: &[u8]) -> Result<(), SocketError> {
        self.write_inner::<TARGETS_SERVER>(buf)
    }
}

struct GenericSockConn<T: GenericSockConnTrait> {
    inner_conn: Mutex<T>,
    available_server: AtomicUsize,
    available_cli: AtomicUsize,
    conn_dropped: AtomicBool,
}

impl<T: GenericSockConnTrait> GenericSockConn<T> {
    fn new() -> Self {
        Self {
            inner_conn: Mutex::new(T::new()),
            available_cli: AtomicUsize::new(0),
            available_server: AtomicUsize::new(0),
            conn_dropped: AtomicBool::new(false),
        }
    }

    fn read<const IS_SERVER: bool>(
        &self,
        buf: &mut [u8],
        can_block: bool,
    ) -> Result<usize, SocketError> {
        let update = if IS_SERVER {
            &self.available_server
        } else {
            &self.available_cli
        };

        let results = self.inner_conn.lock().read::<IS_SERVER>(buf);
        match results {
            Ok(r) => {
                update.fetch_sub(r, core::sync::atomic::Ordering::SeqCst);
                Ok(r)
            }
            Err(SocketError::WouldBlockEmpty) if can_block => {
                if self
                    .conn_dropped
                    .load(core::sync::atomic::Ordering::Acquire)
                {
                    return Err(SocketError::ConnectionClosed);
                }

                thread::current().wait_for_empty_socket(&update, &self.conn_dropped);
                self.read::<IS_SERVER>(buf, can_block)
            }
            Err(e) => Err(e),
        }
    }

    fn write<const TARGETS_SERVER: bool>(
        &self,
        buf: &[u8],
        can_block: bool,
    ) -> Result<(), SocketError> {
        if self
            .conn_dropped
            .load(core::sync::atomic::Ordering::Acquire)
        {
            return Err(SocketError::ConnectionClosed);
        }

        let update = if TARGETS_SERVER {
            &self.available_server
        } else {
            &self.available_cli
        };

        let results = self.inner_conn.lock().write::<TARGETS_SERVER>(buf);
        match results {
            Ok(()) => {
                update.fetch_add(buf.len(), core::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
            Err(SocketError::WouldBlockFull) if can_block => {
                if self
                    .conn_dropped
                    .load(core::sync::atomic::Ordering::Acquire)
                {
                    return Err(SocketError::ConnectionClosed);
                }

                unsafe {
                    thread::current().wait_for_full_socket(
                        &update,
                        &self.conn_dropped,
                        MAX_STREAM_SIZE,
                        buf.len(),
                    );
                }

                self.write::<TARGETS_SERVER>(buf, can_block)
            }
            Err(e) => Err(e),
        }
    }

    fn mark_dropped(&self) {
        self.conn_dropped
            .store(true, core::sync::atomic::Ordering::Release);
    }
}

struct GenericSockConnQueue<T: GenericSockConnTrait> {
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

#[derive(Clone)]
enum SocketConnState {
    Stream(Arc<GenericSockConn<SocketStreamConn>>),
    SeqPacket(Arc<GenericSockConn<SocketSeqPacketConn>>),
}

impl SocketConnState {
    fn read<const IS_SERVER: bool>(
        &self,
        buf: &mut [u8],
        can_block: bool,
    ) -> Result<usize, SocketError> {
        match self {
            Self::SeqPacket(seq) => seq.read::<IS_SERVER>(buf, can_block),
            Self::Stream(s) => s.read::<IS_SERVER>(buf, can_block),
        }
    }

    fn write<const TARGETS_SERVER: bool>(
        &self,
        buf: &[u8],
        can_block: bool,
    ) -> Result<(), SocketError> {
        match self {
            Self::SeqPacket(seq) => seq.write::<TARGETS_SERVER>(buf, can_block),
            Self::Stream(s) => s.write::<TARGETS_SERVER>(buf, can_block),
        }
    }
}

#[derive(Clone)]
struct SocketConn {
    state: SocketConnState,
    can_block: bool,
}

impl SocketConn {
    fn read<const IS_SERVER: bool>(&self, buf: &mut [u8]) -> Result<usize, SocketError> {
        self.state.read::<IS_SERVER>(buf, self.can_block)
    }

    fn write<const TARGETS_SERVER: bool>(&self, buf: &[u8]) -> Result<(), SocketError> {
        self.state.write::<TARGETS_SERVER>(buf, self.can_block)
    }
}

enum SocketType {
    Stream(RwLock<GenericSockConnQueue<SocketStreamConn>>),
    SeqPacket(RwLock<GenericSockConnQueue<SocketSeqPacketConn>>),
}

impl SocketType {
    fn connect(&self, can_block: bool) -> (SocketConn, SockConnID) {
        match self {
            Self::Stream(s) => {
                let (conn, key) = s.write().connect();
                (
                    SocketConn {
                        state: SocketConnState::Stream(conn),
                        can_block,
                    },
                    key,
                )
            }
            Self::SeqPacket(seq) => {
                let (conn, key) = seq.write().connect();
                (
                    SocketConn {
                        state: SocketConnState::SeqPacket(conn),
                        can_block,
                    },
                    key,
                )
            }
        }
    }

    fn remove_connection(&self, id: SockConnID) {
        match self {
            Self::Stream(s) => s.write().remove_connection(id),
            Self::SeqPacket(seq) => seq.write().remove_connection(id),
        }
    }

    fn drop_all_connections(&self) {
        match self {
            Self::SeqPacket(seq) => seq.write().drop_all_connections(),
            Self::Stream(s) => s.write().drop_all_connections(),
        }
    }
}

impl Drop for SocketType {
    fn drop(&mut self) {
        self.drop_all_connections();
    }
}

/// The server's side of the socket connection
///
/// Once dropped the connection is removed, the client may still read until there are no more data to read
pub struct SocketServerConn {
    inner: SocketConn,
    id: SockConnID,
    socket: Arc<Socket>,
}

impl SocketServerConn {
    /// Reads `buf.len()` or less data from the server's buffer
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, SocketError> {
        self.inner.read::<true>(buf)
    }

    /// Writes `buf.len()` data to the client's buffer
    pub fn write(&self, buf: &[u8]) -> Result<(), SocketError> {
        self.inner.write::<false>(buf)
    }
}

/// The client's side of the socket connection
///
/// Once dropped the connection is removed, the server may still read until there are no data to read
pub struct SocketClientConn {
    inner: SocketConn,
    id: SockConnID,
    socket: Arc<Socket>,
}

impl SocketClientConn {
    /// Reads `buf.len()` or less data from the client's buffer
    fn read(&self, buf: &mut [u8]) -> Result<usize, SocketError> {
        self.inner.read::<false>(buf)
    }

    /// Writes `buf.len()` data to the server's buffer
    fn write(&self, buf: &[u8]) -> Result<(), SocketError> {
        self.inner.write::<true>(buf)
    }
}

impl Drop for SocketClientConn {
    fn drop(&mut self) {
        self.socket.disconnect(self.id);
    }
}

impl Drop for SocketServerConn {
    fn drop(&mut self) {
        self.socket.disconnect(self.id);
    }
}

pub struct Socket {
    _domain: SocketDomain,
    can_block: bool,
    sock_type: SocketType,
    listen_queue: Mutex<Vec<*mut (MaybeUninit<SocketClientConn>, AtomicBool)>>,
    listen_queue_available: AtomicBool,

    socket_dropped: AtomicBool,
}

impl Socket {
    fn before_drop(&self) {
        self.socket_dropped.store(true, Ordering::Release);
        // Stop all the existing connections
        self.sock_type.drop_all_connections();

        let queue = self.listen_queue.lock();
        // Wake everyone waiting for connection
        for ptr in &**queue {
            unsafe { (**ptr).1.store(true, Ordering::Release) }
        }
    }

    pub fn disconnect(&self, id: SockConnID) {
        self.sock_type.remove_connection(id);
    }
}

#[derive(Debug, Clone, Copy)]
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
        let id = self.next_id;
        let sock = Socket {
            _domain: domain,
            can_block,
            sock_type: match kind {
                SocketKind::SeqPacket => {
                    SocketType::SeqPacket(RwLock::new(GenericSockConnQueue::new()))
                }
                SocketKind::Stream => SocketType::Stream(RwLock::new(GenericSockConnQueue::new())),
            },
            listen_queue: Mutex::new(Vec::new()),
            listen_queue_available: AtomicBool::new(false),
            socket_dropped: AtomicBool::new(false),
        };

        let reference = Arc::new(sock);
        self.sockets.insert(id, reference.clone());
        self.next_id += 1;
        ServerSocketDesc { reference, id }
    }

    fn remove_socket(&mut self, socket_id: SockID) -> bool {
        if let Some(s) = self.sockets.remove(&socket_id) {
            s.sock_type.drop_all_connections();
            true
        } else {
            false
        }
    }
}
static SOCKET_ABSTRACT_BINDINGS: Mutex<heapless::FnvIndexMap<Name, SockID, 32>> =
    Mutex::new(heapless::FnvIndexMap::new());

lazy_static! {
    static ref SOCKET_QUEUE: RwLock<SockQueue> = RwLock::new(SockQueue::new());
}

/// A reference to the socket from the Server, Only one can exist
///
/// Once dropped the socket is removed
pub struct ServerSocketDesc {
    reference: Arc<Socket>,
    id: SockID,
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
    /// Creates a new socket connection returning both directions
    fn create_connection(&self) -> (SocketServerConn, SocketClientConn) {
        let (inner, id) = self.sock_type.connect(self.can_block);
        (
            SocketServerConn {
                inner: inner.clone(),
                id,
                socket: self.reference.clone(),
            },
            SocketClientConn {
                inner,
                id,
                socket: self.reference.clone(),
            },
        )
    }

    /// As the server, accept a connection from the listening Queue
    pub fn accept(&self) -> Result<SocketServerConn, ()> {
        let mut listen_queue = self.listen_queue.lock();
        let Some(ptr) = listen_queue.pop() else {
            // Once a connection is available
            if self.can_block {
                self.listen_queue_available.store(false, Ordering::Release);
                drop(listen_queue);

                unsafe {
                    thread::current().wait_for_wake_signal(&self.listen_queue_available);
                }
                debug_assert!(!self.socket_dropped.load(Ordering::Acquire));
                return self.accept();
            } else {
                return Err(());
            }
        };

        let (server_conn, client_conn) = self.create_connection();
        unsafe {
            (*ptr).0 = MaybeUninit::new(client_conn);
            // Send wake up signal
            (*ptr).1.store(true, core::sync::atomic::Ordering::Release);
            Ok(server_conn)
        }
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
    /// As a client connect with the server
    /// returns an Error if the server dropped the socket while we were trying to connect
    pub fn connect(&self) -> Result<SocketClientConn, ()> {
        // Create stuff in the higher half
        let mut boxed = Box::new((MaybeUninit::uninit(), AtomicBool::new(false)));

        let mut queue = self.listen_queue.lock();
        queue.push(&mut *boxed);
        drop(queue);
        // Wake up the server waiting to accept
        self.listen_queue_available.store(true, Ordering::Release);

        unsafe {
            thread::current().wait_for_wake_signal(&boxed.1);
            if self.socket_dropped.load(Ordering::Acquire) {
                // We got wake signal because the socket was dropped by the server and therefore
                // We must return an error
                return Err(());
            }

            let conn = boxed.0.assume_init();
            Ok(conn)
        }
    }
}

/// Creates a new socket returning it's ID
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
        .map(|reference| CliSocketDesc { reference })
}

#[allow(unused)]
fn ipc_stream_test_inner() {
    static BINDED_SOCKET: Mutex<SockID> = Mutex::new(0);
    static SOCKET_DROPPED: AtomicBool = AtomicBool::new(false);

    static CLIENT_MSG: &[u8] = b"Hello from the other side!";
    static SERVER_MSG: &[u8] = b"Your message was received!";

    fn test_thread(_: Tid, (): &()) -> ! {
        let name: Name =
            Name::try_from("safa_core::sockets::test_socket").expect("test socket name too long");

        let sock_id = get_abstract_binding(&name).expect("socket was not binded");
        assert_eq!(*BINDED_SOCKET.lock(), sock_id);
        let sock = get_client_socket(sock_id).expect("socket binding not associated with any id");

        let connection = sock
            .connect()
            .expect("socket dropped while waiting for connection");

        connection.write(CLIENT_MSG).expect("client write failed");

        let mut data_buf = [0u8; SERVER_MSG.len()];
        connection
            .read(&mut data_buf[..])
            .expect("client read failed");

        assert_eq!(&data_buf[..], SERVER_MSG, "the server's message is wrong");
        drop(connection);

        drop(sock);
        SOCKET_DROPPED.store(true, core::sync::atomic::Ordering::Release);
        thread::current::exit(0);
    }

    let name: Name =
        Name::try_from("safa_core::sockets::test_socket").expect("test socket name too long");

    let sock_desc = create_socket(SocketDomain::Unix, SocketKind::Stream, true);
    let weak_sock = Arc::downgrade(&sock_desc.reference);

    *BINDED_SOCKET.lock() = sock_desc.id;
    bind_abstract_socket(name, sock_desc.id);

    // Spawn a second thread
    kernel_thread_spawn(test_thread, &(), None, None)
        .expect("failed to spawn the client thread for socket");

    let connection = sock_desc.accept().expect("socket blocked");
    let mut data_buf = [0u8; CLIENT_MSG.len()];

    connection
        .read(&mut data_buf[..])
        .expect("server read failed");
    assert_eq!(&data_buf[..], CLIENT_MSG, "The client's message is wrong");

    connection.write(SERVER_MSG).expect("server write failed");
    assert_eq!(
        connection.read(&mut []),
        Err(SocketError::ConnectionClosed),
        "Read didn't fail with connection closed, even after it was"
    );
    assert_eq!(
        connection.write(&[]),
        Err(SocketError::ConnectionClosed),
        "Write didn't fail with connection closed, even after it was"
    );

    drop(sock_desc);
    drop(connection);

    while !SOCKET_DROPPED.load(core::sync::atomic::Ordering::Acquire) {}
    assert!(
        weak_sock.strong_count() == 0,
        "The socket has a tailing reference, got {} references",
        weak_sock.strong_count()
    );
}

#[allow(unused)]
fn ipc_seqpacket_test_inner() {
    let name: Name =
        Name::try_from("safa_core::sockets::test_socket").expect("test socket name too long");

    static CLIENT_MSG0: &[u8] = b"Hello from the other side!";
    static CLIENT_MSG1: &[u8] = b"Reply if you received this message!";
    static SERVER_MSG: &[u8] = b"Your message was received!";
    static THREAD_EXIT: AtomicBool = AtomicBool::new(false);

    fn test_thread(_: Tid, (): &()) -> ! {
        {
            let name: Name = Name::try_from("safa_core::sockets::test_socket")
                .expect("test socket name too long");

            let sock_id = get_abstract_binding(&name).expect("Socket was not binded");
            let sock =
                get_client_socket(sock_id).expect("Socket binding not associated with any id");

            let connection = sock
                .connect()
                .expect("Socket dropped while waiting for connection");

            connection
                .write(CLIENT_MSG0)
                .expect("Client failed to write the first message");

            connection
                .write(CLIENT_MSG1)
                .expect("Client failed to write the second message");

            let mut read_buf = [0u8; SERVER_MSG.len()];
            connection
                .read(&mut read_buf[..])
                .expect("Client failed to read the server's message");

            assert_eq!(&read_buf[..], SERVER_MSG, "Server's message was wrong");
        }
        THREAD_EXIT.store(true, Ordering::Release);
        thread::current::exit(0);
    }

    let sock_desc = create_socket(SocketDomain::Unix, SocketKind::SeqPacket, true);
    bind_abstract_socket(name, sock_desc.id);

    // Spawn a second thread
    kernel_thread_spawn(test_thread, &(), None, None)
        .expect("failed to spawn the client thread for socket");

    let accepted = sock_desc.accept().expect("Socket is non-blocking?");
    // If we were a stream, we should have ended up reading both
    let mut msg_buf = [0u8; CLIENT_MSG0.len() + CLIENT_MSG1.len()];

    let read = accepted
        .read(&mut msg_buf)
        .expect("Server failed to read the first message");
    let msg0 = &msg_buf[..read];
    assert_eq!(msg0, CLIENT_MSG0, "Client's first message mismatch");

    let read = accepted
        .read(&mut msg_buf)
        .expect("Server failed to read the second message");
    let msg1 = &msg_buf[..read];
    assert_eq!(msg1, CLIENT_MSG1, "Client's second message mismatch");

    accepted
        .write(SERVER_MSG)
        .expect("Server failed to write a message");

    while !THREAD_EXIT.load(Ordering::Acquire) {}

    assert_eq!(
        accepted.write(SERVER_MSG),
        Err(SocketError::ConnectionClosed),
        "Write was successful even though connection should have been closed"
    );
    assert_eq!(
        accepted.read(&mut msg_buf),
        Err(SocketError::ConnectionClosed),
        "Read was successful even though connection should have been closed"
    );
}

// TODO: Tests are ordered alphatically so we want to run this last, in my framework some modules do have a priority over other tho so I could use that
#[test_case]
fn z_ipc0_stream() {
    ipc_stream_test_inner()
}

#[test_case]
fn z_ipc1_seqpacket() {
    ipc_seqpacket_test_inner();
}
