use core::sync::atomic::{AtomicBool, Ordering};

use alloc::{collections::linked_list::LinkedList, sync::Arc};
use safa_abi::poll::PollEvents;

use crate::{
    arch::without_interrupts,
    drivers::vfs::FSResult,
    memory::paging::PAGE_SIZE,
    process::{
        poll::{self, PollID},
        resources::Resource,
    },
    scheduler::wait_queue::WaitQueue,
    sockets::{SockConnID, Socket, SocketError},
    thread,
    utils::locks::Mutex,
};

const MAX_STREAM_SIZE: usize = PAGE_SIZE;
/// A stream socket connection.
pub struct SocketStreamConn {
    server_buf: Mutex<heapless::Vec<u8, MAX_STREAM_SIZE>>,
    client_buf: Mutex<heapless::Vec<u8, MAX_STREAM_SIZE>>,
}

impl SocketStreamConn {
    pub const fn new() -> Self {
        Self {
            server_buf: Mutex::new(heapless::Vec::new()),
            client_buf: Mutex::new(heapless::Vec::new()),
        }
    }

    fn read_inner<const IS_SERVER: bool>(&self, buf: &mut [u8]) -> Result<usize, SocketError> {
        let mut to_read_from = if IS_SERVER {
            self.server_buf.lock()
        } else {
            self.client_buf.lock()
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

    fn write_inner<const IS_SERVER: bool>(&self, buf: &[u8]) -> Result<usize, SocketError> {
        let len = buf.len().min(MAX_STREAM_SIZE);

        let mut to_write_to = if IS_SERVER {
            self.server_buf.lock()
        } else {
            self.client_buf.lock()
        };

        to_write_to
            .extend_from_slice(&buf[..len])
            .map_err(|()| SocketError::WouldBlockFull)?;
        Ok(len)
    }
}

/// A sequenced packet socket connection.
pub struct SocketSeqPacketConn {
    inner: SocketStreamConn,
    server_packets: Mutex<LinkedList<usize>>,
    client_packets: Mutex<LinkedList<usize>>,
}

impl SocketSeqPacketConn {
    pub const fn new() -> Self {
        Self {
            inner: SocketStreamConn::new(),
            server_packets: Mutex::new(LinkedList::new()),
            client_packets: Mutex::new(LinkedList::new()),
        }
    }

    fn read_inner<const IS_SERVER: bool>(&self, buf: &mut [u8]) -> Result<usize, SocketError> {
        let mut to_read_from = if IS_SERVER {
            self.server_packets.lock()
        } else {
            self.client_packets.lock()
        };

        let Some(msg_len) = to_read_from.pop_front() else {
            return Err(SocketError::WouldBlockEmpty);
        };

        let amount = buf.len().min(msg_len);
        self.inner.read_inner::<IS_SERVER>(&mut buf[..amount])
    }

    fn write_inner<const IS_SERVER: bool>(&self, buf: &[u8]) -> Result<usize, SocketError> {
        if buf.len() == 0 {
            return Ok(0);
        }

        let mut to_write_to = if IS_SERVER {
            self.server_packets.lock()
        } else {
            self.client_packets.lock()
        };

        let len = self.inner.write_inner::<IS_SERVER>(buf)?;
        to_write_to.push_back(len);
        Ok(len)
    }
}

/// A trait for generic socket connections.
pub trait GenericSockConnTrait {
    fn new() -> Self
    where
        Self: Sized;

    /// A Write operation
    fn write<const TARGETS_SERVER: bool>(&self, buf: &[u8]) -> Result<usize, SocketError>;
    /// A Read operation
    fn read<const IS_SERVER: bool>(&self, buf: &mut [u8]) -> Result<usize, SocketError>;
}

impl GenericSockConnTrait for SocketStreamConn {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self::new()
    }

    fn read<const IS_SERVER: bool>(&self, buf: &mut [u8]) -> Result<usize, SocketError> {
        self.read_inner::<IS_SERVER>(buf)
    }

    fn write<const TARGETS_SERVER: bool>(&self, buf: &[u8]) -> Result<usize, SocketError> {
        self.write_inner::<TARGETS_SERVER>(buf)
    }
}

impl GenericSockConnTrait for SocketSeqPacketConn {
    fn new() -> Self {
        Self::new()
    }

    fn read<const IS_SERVER: bool>(&self, buf: &mut [u8]) -> Result<usize, SocketError> {
        self.read_inner::<IS_SERVER>(buf)
    }

    fn write<const TARGETS_SERVER: bool>(&self, buf: &[u8]) -> Result<usize, SocketError> {
        self.write_inner::<TARGETS_SERVER>(buf)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SocketWaitReason {
    ServSockFull(usize),
    ClientSockFull(usize),
    ServSockEmpty,
    ClientSockEmpty,
}

struct Waiter {
    server_sock_len: usize,
    client_sock_len: usize,
    conn_dropped: bool,
    wait_queue: WaitQueue<2, SocketWaitReason>,
}

impl Waiter {
    const fn new() -> Self {
        Self {
            server_sock_len: 0,
            client_sock_len: 0,
            conn_dropped: false,
            wait_queue: WaitQueue::new(),
        }
    }
}

/// Represents a generic socket connection.
pub(super) struct GenericSockConn<T: GenericSockConnTrait> {
    inner_conn: T,
    wait_queue: Mutex<Waiter>,
}

impl<T: GenericSockConnTrait> GenericSockConn<T> {
    pub(super) fn new() -> Self {
        Self {
            inner_conn: T::new(),
            wait_queue: Mutex::new(Waiter::new()),
        }
    }

    pub(super) fn read<const IS_SERVER: bool>(
        &self,
        buf: &mut [u8],
        can_block: bool,
    ) -> Result<usize, SocketError> {
        let mut waiter = self.wait_queue.lock();
        let conn_dropped = waiter.conn_dropped;
        let update = if IS_SERVER {
            &mut waiter.server_sock_len
        } else {
            &mut waiter.client_sock_len
        };

        let results = self.inner_conn.read::<IS_SERVER>(buf);
        match results {
            Ok(r) => {
                let last = *update;
                *update -= r;
                let current = *update;

                let ava = MAX_STREAM_SIZE - current;
                waiter
                    .wait_queue
                    .wake_on_condition(|reason| match (reason, IS_SERVER) {
                        (SocketWaitReason::ServSockFull(need), true)
                        | (SocketWaitReason::ClientSockFull(need), false)
                            if *need <= ava =>
                        {
                            true
                        }
                        _ => false,
                    });

                let id = if IS_SERVER {
                    self.server_poll_id()
                } else {
                    self.client_poll_id()
                };

                let events_add = if last >= MAX_STREAM_SIZE {
                    PollEvents::CAN_WRITE
                } else {
                    PollEvents::NONE
                };

                let events_remove = if current == 0 {
                    PollEvents::DATA_AVAILABLE
                } else {
                    PollEvents::NONE
                };

                if !events_add.is_empty() || !events_remove.is_empty() {
                    poll::broadcast_events(id, events_add, events_remove);
                }
                Ok(r)
            }
            Err(SocketError::WouldBlockEmpty) if can_block => {
                if conn_dropped {
                    return Err(SocketError::ConnectionClosed);
                }

                without_interrupts(move || {
                    thread::current().sleep_in_queue(
                        &mut waiter.wait_queue,
                        if IS_SERVER {
                            SocketWaitReason::ServSockEmpty
                        } else {
                            SocketWaitReason::ClientSockEmpty
                        },
                    );
                    drop(waiter);
                    thread::current::yield_now();
                });

                self.read::<IS_SERVER>(buf, can_block)
            }
            Err(e) => Err(e),
        }
    }

    pub(super) fn write<const TARGETS_SERVER: bool>(
        &self,
        buf: &[u8],
        can_block: bool,
    ) -> Result<usize, SocketError> {
        let amount = buf.len().min(MAX_STREAM_SIZE);
        let buf = &buf[..amount];
        let mut waiter = self.wait_queue.lock();

        let conn_dropped = waiter.conn_dropped;
        if conn_dropped {
            return Err(SocketError::ConnectionClosed);
        }

        let update = if TARGETS_SERVER {
            &mut waiter.server_sock_len
        } else {
            &mut waiter.client_sock_len
        };

        let results = self.inner_conn.write::<TARGETS_SERVER>(buf);
        match results {
            Ok(len) => {
                let last = *update;
                *update += len;
                let current = *update;

                let (reason, poll_id) = if TARGETS_SERVER {
                    (SocketWaitReason::ServSockEmpty, self.server_poll_id())
                } else {
                    (SocketWaitReason::ClientSockEmpty, self.client_poll_id())
                };

                let events_add = if last == 0 {
                    PollEvents::DATA_AVAILABLE
                } else {
                    PollEvents::NONE
                };

                let events_remove = if current >= MAX_STREAM_SIZE {
                    PollEvents::CAN_WRITE
                } else {
                    PollEvents::NONE
                };

                if !events_add.is_empty() || !events_remove.is_empty() {
                    poll::broadcast_events(poll_id, events_add, events_remove);
                }

                waiter.wait_queue.wake_equals(&reason);
                Ok(len)
            }
            Err(SocketError::WouldBlockFull) if can_block => {
                if conn_dropped {
                    return Err(SocketError::ConnectionClosed);
                }

                without_interrupts(|| {
                    thread::current().sleep_in_queue(
                        &mut waiter.wait_queue,
                        if TARGETS_SERVER {
                            SocketWaitReason::ServSockFull(amount)
                        } else {
                            SocketWaitReason::ClientSockFull(amount)
                        },
                    );

                    drop(waiter);
                    thread::current::yield_now();
                });
                self.write::<TARGETS_SERVER>(buf, can_block)
            }
            Err(e) => Err(e),
        }
    }

    pub(super) fn server_poll_id(&self) -> PollID {
        PollID::from_ptr(self as *const Self)
    }

    pub(super) fn client_poll_id(&self) -> PollID {
        let ptr = self as *const Self as usize;
        // size_of::<Self> is bigger than 1 so this is ok.
        PollID::from_usize(ptr + 1)
    }

    pub(super) fn mark_dropped(&self) {
        let mut waiter = self.wait_queue.lock();
        waiter.conn_dropped = true;
        waiter.wait_queue.wake_all();
        poll::broadcast_events(
            self.client_poll_id(),
            PollEvents::DISCONNECTED,
            PollEvents::NONE,
        );
        poll::broadcast_events(
            self.server_poll_id(),
            PollEvents::DISCONNECTED,
            PollEvents::NONE,
        );
    }
}

impl<T: GenericSockConnTrait> Drop for GenericSockConn<T> {
    fn drop(&mut self) {
        poll::stop_tracking_id(self.client_poll_id());
        poll::stop_tracking_id(self.server_poll_id());
    }
}

/// All possible socket connections.
#[derive(Clone)]
pub(super) enum SocketConn {
    Stream(Arc<GenericSockConn<SocketStreamConn>>),
    SeqPacket(Arc<GenericSockConn<SocketSeqPacketConn>>),
}

impl SocketConn {
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
    ) -> Result<usize, SocketError> {
        match self {
            Self::SeqPacket(seq) => seq.write::<TARGETS_SERVER>(buf, can_block),
            Self::Stream(s) => s.write::<TARGETS_SERVER>(buf, can_block),
        }
    }

    fn server_poll_id(&self) -> PollID {
        match self {
            Self::SeqPacket(sqp) => sqp.server_poll_id(),
            Self::Stream(st) => st.server_poll_id(),
        }
    }

    fn client_poll_id(&self) -> PollID {
        match self {
            Self::SeqPacket(sqp) => sqp.client_poll_id(),
            Self::Stream(st) => st.client_poll_id(),
        }
    }
}

/// The server's side of the socket connection
///
/// Once dropped the connection is removed, the client may still read until there are no more data to read
pub struct SocketServerConn {
    inner: SocketConn,
    id: SockConnID,
    socket: Arc<Socket>,
    can_block: AtomicBool,
}

impl SocketServerConn {
    pub(super) fn new(
        inner: SocketConn,
        id: SockConnID,
        socket: Arc<Socket>,
        can_block: bool,
    ) -> Self {
        Self {
            inner,
            id,
            socket,
            can_block: AtomicBool::new(can_block),
        }
    }

    /// Handles a `SysIOCommand` operation
    pub fn handle_command(&self, cmd: u16, arg: u64) -> FSResult<()> {
        const SET_BLOCKING: u16 = 0;
        match cmd {
            SET_BLOCKING => {
                let can_block = arg != 0;
                self.set_can_block(can_block);
                Ok(())
            }
            _ => Err(crate::drivers::vfs::FSError::InvalidCmd),
        }
    }

    pub fn can_block(&self) -> bool {
        self.can_block.load(Ordering::Acquire)
    }

    /// Sets can_block to value `new_value`
    pub fn set_can_block(&self, new_value: bool) {
        match self.can_block.compare_exchange(
            !new_value,
            new_value,
            Ordering::Acquire,
            Ordering::Acquire,
        ) {
            Ok(v) => assert_ne!(v, new_value),
            Err(v) => assert_eq!(v, new_value),
        }
    }
    /// Reads `buf.len()` or less data from the server's buffer
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, SocketError> {
        self.inner.read::<true>(buf, self.can_block())
    }

    /// Writes `buf.len()` or less data to the client's buffer
    pub fn write(&self, buf: &[u8]) -> Result<usize, SocketError> {
        self.inner.write::<false>(buf, self.can_block())
    }
}

/// The client's side of the socket connection
///
/// Once dropped the connection is removed, the server may still read until there are no data to read
pub struct SocketClientConn {
    inner: SocketConn,
    id: SockConnID,
    socket: Arc<Socket>,
    can_block: AtomicBool,
}

impl SocketClientConn {
    pub(super) fn new(
        inner: SocketConn,
        id: SockConnID,
        socket: Arc<Socket>,
        can_block: bool,
    ) -> Self {
        Self {
            inner,
            id,
            socket,
            can_block: AtomicBool::new(can_block),
        }
    }

    /// Handles a `SysIOCommand` operation
    pub fn handle_command(&self, cmd: u16, arg: u64) -> FSResult<()> {
        const SET_BLOCKING: u16 = 0;
        match cmd {
            SET_BLOCKING => {
                let can_block = arg != 0;
                self.set_can_block(can_block);
                Ok(())
            }
            _ => Err(crate::drivers::vfs::FSError::InvalidCmd),
        }
    }
    pub fn can_block(&self) -> bool {
        self.can_block.load(Ordering::Acquire)
    }

    /// Sets can_block to value `new_value`
    pub fn set_can_block(&self, new_value: bool) {
        match self.can_block.compare_exchange(
            !new_value,
            new_value,
            Ordering::Acquire,
            Ordering::Acquire,
        ) {
            Ok(v) => assert_ne!(v, new_value),
            Err(v) => assert_eq!(v, new_value),
        }
    }

    /// Reads `buf.len()` or less data from the client's buffer
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, SocketError> {
        self.inner.read::<false>(buf, self.can_block())
    }

    /// Writes `buf.len()` data to the server's buffer
    pub fn write(&self, buf: &[u8]) -> Result<usize, SocketError> {
        self.inner.write::<true>(buf, self.can_block())
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

impl Resource for SocketClientConn {
    fn read(
        &self,
        off: crate::drivers::vfs::SeekOffset,
        buf: &mut [u8],
    ) -> Result<usize, safa_abi::errors::ErrorStatus> {
        _ = off;
        let am = self.read(buf)?;
        Ok(am)
    }
    fn write(
        &self,
        off: crate::drivers::vfs::SeekOffset,
        buf: &[u8],
    ) -> Result<usize, safa_abi::errors::ErrorStatus> {
        _ = off;
        let am = self.write(buf)?;
        Ok(am)
    }
    fn send_command(&self, cmd: u16, arg: u64) -> Result<(), safa_abi::errors::ErrorStatus> {
        self.handle_command(cmd, arg)?;
        Ok(())
    }
    fn address_space_generic(&self) -> bool {
        false
    }
    fn poll_id(&self) -> Option<PollID> {
        Some(self.inner.client_poll_id())
    }
}

impl Resource for SocketServerConn {
    fn read(
        &self,
        off: crate::drivers::vfs::SeekOffset,
        buf: &mut [u8],
    ) -> Result<usize, safa_abi::errors::ErrorStatus> {
        _ = off;
        let am = self.read(buf)?;
        Ok(am)
    }
    fn write(
        &self,
        off: crate::drivers::vfs::SeekOffset,
        buf: &[u8],
    ) -> Result<usize, safa_abi::errors::ErrorStatus> {
        _ = off;
        let am = self.write(buf)?;
        Ok(am)
    }
    fn send_command(&self, cmd: u16, arg: u64) -> Result<(), safa_abi::errors::ErrorStatus> {
        self.handle_command(cmd, arg)?;
        Ok(())
    }
    fn address_space_generic(&self) -> bool {
        false
    }
    fn poll_id(&self) -> Option<PollID> {
        Some(self.inner.server_poll_id())
    }
}
