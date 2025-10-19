use core::{cell::UnsafeCell, num::NonZero};

use alloc::{
    boxed::Box,
    collections::linked_list::LinkedList,
    sync::{Arc, Weak},
};
use hashbrown::HashMap;
use rustc_hash::FxBuildHasher;
use safa_abi::{errors::ErrorStatus, poll::PollEvents, sockets::SockMsgFlags};

use crate::{
    memory::{page_allocator::PageAlloc, paging::PAGE_SIZE},
    process::poll::{self, PollID},
    scheduler::wait_queue::WaitQueue,
    sockets::{Socket, SocketAddrRef, SocketError},
    syscalls::ffi::SyscallFFI,
    utils::{
        locks::{Mutex, RwLock},
        types::Name,
    },
};

const STREAM_SIZE: usize = (PAGE_SIZE * 2) - size_of::<heapless::Vec<u8, 0>>();
/// One side of a stream connection.
#[derive(Debug)]
pub(super) struct Stream {
    data: Box<heapless::Vec<u8, STREAM_SIZE>, PageAlloc>,
}

impl Stream {
    pub fn new() -> Self {
        Self {
            data: Box::new_in(heapless::Vec::new(), PageAlloc),
        }
    }

    pub fn read(
        &mut self,
        buf: &mut [u8],
        peek: bool,
    ) -> Result<(usize, usize, usize), SocketError> {
        let before_read_len = self.data.len();
        let size = buf.len().min(before_read_len);
        if size == 0 {
            return Err(SocketError::WouldBlockEmpty);
        }

        buf[..size].copy_from_slice(&self.data[..size]);
        if peek {
            return Ok((size, before_read_len, before_read_len));
        }

        let new_len = before_read_len - size;
        self.data.copy_within(size.., 0);
        self.data.truncate(new_len);

        Ok((size, new_len, before_read_len))
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<(usize, usize, usize), SocketError> {
        let len_before_write = self.data.len();
        let size = buf.len().min(self.data.capacity() - len_before_write);
        if size == 0 {
            return Err(SocketError::WouldBlockFull);
        }

        self.data
            .extend_from_slice(&buf[..size])
            .expect("Attempt to write too much data");
        Ok((size, self.data.len(), len_before_write))
    }
}

#[derive(Debug)]
struct SeqPacketStream {
    inner: Stream,
    messages: LinkedList<usize>,
}

impl SeqPacketStream {
    fn new() -> Self {
        Self {
            inner: Stream::new(),
            messages: LinkedList::new(),
        }
    }
    fn write(&mut self, buf: &[u8]) -> Result<(usize, usize, usize), SocketError> {
        let (wrote, len_now, len_before) = self.inner.write(buf)?;
        self.messages.push_back(wrote);
        Ok((wrote, len_now, len_before))
    }

    fn read(&mut self, buf: &mut [u8], peek: bool) -> Result<(usize, usize, usize), SocketError> {
        let message_len = if peek {
            self.messages.front().copied()
        } else {
            self.messages.pop_front()
        }
        .ok_or(SocketError::WouldBlockEmpty)?;

        let read_len = buf.len().min(message_len);
        self.inner.read(&mut buf[..read_len], peek)
    }
}

#[derive(Debug)]
enum Buffer {
    SeqPacket(SeqPacketStream),
    Stream(Stream),
}

impl Buffer {
    fn new(kind: LocalSocketKind) -> Self {
        match kind {
            LocalSocketKind::SeqPacket => Self::SeqPacket(SeqPacketStream::new()),
            LocalSocketKind::Stream => Self::Stream(Stream::new()),
        }
    }
    fn write(&mut self, buf: &[u8]) -> Result<(usize, usize, usize), SocketError> {
        match self {
            Self::SeqPacket(s) => s.write(buf),
            Self::Stream(s) => s.write(buf),
        }
    }

    fn read(&mut self, buf: &mut [u8], peek: bool) -> Result<(usize, usize, usize), SocketError> {
        match self {
            Self::SeqPacket(s) => s.read(buf, peek),
            Self::Stream(s) => s.read(buf, peek),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalSocketKind {
    Stream,
    SeqPacket,
}

enum Status {
    Disconnected,
    Listening {
        /// # Safety: Modifications MUST BE guarded by a lock on accept_queue
        current: UnsafeCell<usize>,
        max: usize,
        accept_queue: Mutex<WaitQueue<1, Arc<LocalSocket>>>,
    },
    Connected {
        other: Option<Arc<LocalSocket>>,
        buffer: Mutex<Buffer>,
    },
}

impl Drop for Status {
    fn drop(&mut self) {
        match self {
            // Close the connection from the other side
            Self::Connected {
                other: Some(other_socket),
                ..
            } => {
                let other_status = &mut *other_socket.status.write();
                match other_status {
                    Status::Connected { other: our_ref, .. } => {
                        *our_ref = None;
                        other_socket.on_other_disconnected();
                    }
                    _ => {}
                }
            }
            // Other status can drop themselves!
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitReason {
    WaitUntilCanWrite,
    WaitUntilNotEmpty,
    WaitingForAccepts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TimeoutInfo {
    read_timeout: Option<NonZero<u64>>,
    write_timeout: Option<NonZero<u64>>,
    can_block: bool,
}

pub struct LocalSocket {
    conn_accepted: UnsafeCell<bool>,
    timeout_info: RwLock<TimeoutInfo>,
    status: RwLock<Status>,
    wait_queue: Mutex<WaitQueue<1, WaitReason>>,
    kind: LocalSocketKind,
    binded_to: Mutex<Option<Arc<Name>>>,
    this: Weak<Self>,
}

impl Drop for LocalSocket {
    fn drop(&mut self) {
        self.unbind_mut();
        poll::stop_tracking_id(self.poll_id());
        crate::serial!("IT SHOULD BE DROPPED NOW\n");
    }
}

impl LocalSocket {
    fn create_inner(status: Status, kind: LocalSocketKind, can_block: bool) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            conn_accepted: UnsafeCell::new(false),
            timeout_info: RwLock::new(TimeoutInfo {
                read_timeout: None,
                write_timeout: None,
                can_block: can_block,
            }),
            status: RwLock::new(status),
            wait_queue: Mutex::new(WaitQueue::new()),
            kind,
            binded_to: Mutex::new(None),
            this: this.clone(),
        })
    }

    pub fn create(kind: LocalSocketKind, can_block: bool) -> Arc<Self> {
        Self::create_inner(Status::Disconnected, kind, can_block)
    }

    fn listen(&self, backlog: usize) -> Result<(), SocketError> {
        let mut status_guard = self.status.write();
        match &mut *status_guard {
            Status::Disconnected => {
                *status_guard = Status::Listening {
                    current: UnsafeCell::new(0),
                    max: backlog,
                    accept_queue: Mutex::new(WaitQueue::new()),
                };
                Ok(())
            }

            _ => Err(SocketError::OperationNotSupported),
        }
    }

    fn create_connected(connect_with: Arc<LocalSocket>, can_block: bool) -> Arc<Self> {
        let mut status_guard = connect_with.status.write();
        let kind = connect_with.kind;

        let this = Self::create_inner(
            Status::Connected {
                other: Some(connect_with.clone()),
                buffer: Mutex::new(Buffer::new(kind)),
            },
            kind,
            can_block,
        );

        *status_guard = Status::Connected {
            other: Some(this.clone()),
            buffer: Mutex::new(Buffer::new(kind)),
        };
        unsafe { *connect_with.conn_accepted.get() = true };
        this
    }

    fn try_accept_connection(&self) -> Result<Arc<LocalSocket>, SocketError> {
        match &*self.status.read() {
            Status::Listening { accept_queue, .. } => {
                let can_block = self.timeout_info.read().can_block;
                loop {
                    let mut queue_guard = accept_queue.lock();

                    if let Some(connection) = queue_guard.try_pop_one(|reason| Some(reason.clone()))
                    {
                        unsafe { *connection.conn_accepted.get() = true };
                        let new = Self::create_connected(connection, can_block);
                        break Ok(new);
                    } else if !can_block {
                        break Err(SocketError::WouldBlockNoConnectionRequests);
                    } else {
                        let pending_wait = self.wait_queue.prepare_wait();
                        poll::broadcast_events(
                            self.poll_id(),
                            PollEvents::CAN_WRITE,
                            PollEvents::DATA_AVAILABLE,
                        );
                        drop(queue_guard);
                        pending_wait.enter_wait(WaitReason::WaitingForAccepts, None)?;
                    }
                }
            }
            _ => Err(SocketError::OperationNotSupported),
        }
    }

    #[inline]
    pub fn poll_id(&self) -> PollID {
        PollID::from_ptr(self as *const LocalSocket)
    }

    fn unbind(&self) {
        let binded_to = self.binded_to.lock();
        if let Some(address) = &*binded_to {
            SOCKET_ABSTRACT_BINDINGS.write().remove(address);
        }
    }

    fn unbind_mut(&mut self) {
        if let Some(address) = &*self.binded_to.get_mut() {
            SOCKET_ABSTRACT_BINDINGS.write().remove(address);
        }
    }

    // Clean up all our references
    fn on_close(&self) {
        self.unbind();
        // To close or not to close,
        // 2 may close at this same time thats why we drop the write lock first before changing the other sides status
        let old_value = core::mem::replace(&mut *self.status.write(), Status::Disconnected);
        // Status should clean up after itself
        drop(old_value);
    }

    fn on_other_disconnected(&self) {
        self.wait_queue.lock().wake_all();
        poll::broadcast_events(self.poll_id(), PollEvents::DISCONNECTED, PollEvents::NONE);
    }

    pub fn write_connected(&self, buf: &[u8], can_block_mask: bool) -> Result<usize, SocketError> {
        let status = self.status.read();
        match &*status {
            Status::Connected {
                other: Some(other), ..
            } => other.write_self(buf, can_block_mask, *self.timeout_info.read()),
            Status::Disconnected
            | Status::Listening { .. }
            | Status::Connected { other: None, .. } => Err(SocketError::ConnectionClosed),
        }
    }

    fn write_self(
        &self,
        buf: &[u8],
        can_block_mask: bool,
        timeout_info: TimeoutInfo,
    ) -> Result<usize, SocketError> {
        let status_guard = self.status.read();
        let status = &*status_guard;
        let Status::Connected {
            buffer,
            other: Some(_),
            ..
        } = status
        else {
            return Err(SocketError::ConnectionClosed);
        };

        if buf.len() == 0 {
            return Ok(0);
        }

        let mut buffer = buffer.lock();

        let can_block = timeout_info.can_block && can_block_mask;

        match buffer.write(buf) {
            Ok((size, len_now, len_before)) => {
                let mut events_add = PollEvents::NONE;
                if len_before == 0 {
                    events_add = PollEvents::DATA_AVAILABLE;
                    debug_assert_ne!(len_now, 0);

                    self.wait_queue
                        .lock()
                        .wake_on_condition(|r| r == &WaitReason::WaitUntilNotEmpty);
                }

                let events_remove = if len_now >= STREAM_SIZE {
                    PollEvents::CAN_WRITE
                } else {
                    PollEvents::NONE
                };

                // We cannot drop the buffer guard before broadcasting events so that they stay in sync.
                poll::broadcast_events(self.poll_id(), events_add, events_remove);
                Ok(size)
            }
            Err(SocketError::WouldBlockFull) if can_block => {
                let pending_wait = self.wait_queue.prepare_wait();
                // We can drop the guard now, because when someone tries to notify the wait queue of new data they'll have to wait until we enter sleep
                drop(buffer);
                drop(status_guard);
                // FIXME: if more than one thread is waiting we should decrement the timeout before retrying...
                pending_wait
                    .enter_wait(WaitReason::WaitUntilCanWrite, timeout_info.write_timeout)?;

                self.write_self(buf, can_block_mask, timeout_info)
            }
            Err(SocketError::WouldBlockEmpty) => unreachable!(),
            Err(e) => Err(e),
        }
    }

    pub fn read_self(
        &self,
        buf: &mut [u8],
        just_peek: bool,
        can_block_mask: bool,
    ) -> Result<usize, SocketError> {
        if buf.len() == 0 {
            return Ok(0);
        }

        let status_guard = self.status.read();
        let status = &*status_guard;

        let closed;
        let buffer = match status {
            Status::Connected { buffer, other, .. } => {
                closed = other.is_none();
                buffer
            }
            Status::Disconnected | Status::Listening { .. } => {
                return Err(SocketError::ConnectionClosed);
            }
        };

        let mut buffer = buffer.lock();
        let timeout_info = *self.timeout_info.read();
        let can_block = can_block_mask && timeout_info.can_block;

        match buffer.read(buf, just_peek) {
            Ok((amount, len_now, len_before)) => {
                let mut events_add = PollEvents::NONE;
                if len_before >= STREAM_SIZE {
                    debug_assert!(len_now < STREAM_SIZE);
                    events_add = PollEvents::CAN_WRITE;

                    self.wait_queue
                        .lock()
                        .wake_on_condition(|r| r == &WaitReason::WaitUntilCanWrite);
                }

                let events_remove = if len_now == 0 {
                    PollEvents::DATA_AVAILABLE
                } else {
                    PollEvents::NONE
                };

                poll::broadcast_events(self.poll_id(), events_add, events_remove);
                Ok(amount)
            }
            Err(SocketError::WouldBlockEmpty) if closed => {
                drop(buffer);
                drop(status_guard);

                let mut status_mut = self.status.write();
                *status_mut = Status::Disconnected;

                Err(SocketError::ConnectionClosed)
            }
            Err(SocketError::WouldBlockEmpty) if can_block => {
                let pending_wait = self.wait_queue.prepare_wait();
                drop(buffer);
                drop(status_guard);
                // FIXME: retrying doesn't respect the timeout...
                pending_wait
                    .enter_wait(WaitReason::WaitUntilNotEmpty, timeout_info.read_timeout)?;

                self.read_self(buf, just_peek, can_block_mask)
            }
            Err(SocketError::WouldBlockFull) => unreachable!(),
            Err(e) => Err(e),
        }
    }

    pub fn try_connect_with(&self, other: Arc<LocalSocket>) -> Result<(), SocketError> {
        match &*other.status.read() {
            Status::Listening {
                current,
                max,
                accept_queue,
            } => {
                if other.kind != self.kind {
                    return Err(SocketError::TypeMismatch);
                }

                let max = *max;
                let pending_wait = accept_queue.prepare_wait();

                let curr = unsafe { &mut *current.get() };
                assert!(*curr <= max);
                if *curr == max {
                    return Err(SocketError::ConnectionRefused);
                }

                let events_add = if *curr == 0 {
                    PollEvents::DATA_AVAILABLE
                } else {
                    PollEvents::NONE
                };

                *curr += 1;

                let events_remove = if *curr == max {
                    PollEvents::CAN_WRITE
                } else {
                    PollEvents::NONE
                };

                other
                    .wait_queue
                    .lock()
                    .wake_on_condition(|r| r == &WaitReason::WaitingForAccepts);
                poll::broadcast_events(other.poll_id(), events_add, events_remove);

                unsafe { *self.conn_accepted.get() = false };
                let this = self
                    .this
                    .upgrade()
                    .expect("Upgrading LocalSocket::this should never fail");
                pending_wait.enter_wait(this, None)?;

                if !unsafe { *self.conn_accepted.get() } {
                    return Err(SocketError::ConnectionRefused);
                }
                Ok(())
            }
            _ => Err(SocketError::OperationNotSupported),
        }
    }
}

unsafe impl Send for LocalSocket {}
unsafe impl Sync for LocalSocket {}

static SOCKET_ABSTRACT_BINDINGS: RwLock<HashMap<Arc<Name>, Arc<LocalSocket>, FxBuildHasher>> =
    RwLock::new(HashMap::with_hasher(FxBuildHasher));

impl Socket for LocalSocket {
    fn accept(&self) -> Result<Arc<Self>, SocketError> {
        self.try_accept_connection()
    }

    fn set_sock_opt(&self, opt: super::SocketOpt, value: u64) -> Result<(), ErrorStatus> {
        match opt {
            super::SocketOpt::Blocking => self.timeout_info.write().can_block = value > 0,
            super::SocketOpt::ReadTimeout => {
                self.timeout_info.write().read_timeout = NonZero::new(value)
            }
            super::SocketOpt::WriteTimeout => {
                self.timeout_info.write().write_timeout = NonZero::new(value)
            }
            super::SocketOpt::SockError
            | super::SocketOpt::IpBroadcast
            | super::SocketOpt::IpTTL => {
                return Err(ErrorStatus::InvalidCommand);
            }
        }

        Ok(())
    }

    fn get_sock_opt(&self, opt: super::SocketOpt, to_usr_ptr: *mut ()) -> Result<(), ErrorStatus> {
        match opt {
            super::SocketOpt::Blocking => {
                let r = <&mut bool>::make(to_usr_ptr.cast())?;
                let blocking = self.timeout_info.read().can_block;
                *r = blocking;
            }
            super::SocketOpt::ReadTimeout => {
                let r = <&mut Option<NonZero<u64>>>::make(to_usr_ptr.cast())?;
                let timeout = self.timeout_info.read().read_timeout;
                *r = timeout;
            }
            super::SocketOpt::WriteTimeout => {
                let r = <&mut Option<NonZero<u64>>>::make(to_usr_ptr.cast())?;
                let timeout = self.timeout_info.read().write_timeout;
                *r = timeout;
            }
            super::SocketOpt::SockError
            | super::SocketOpt::IpBroadcast
            | super::SocketOpt::IpTTL => {
                return Err(ErrorStatus::InvalidCommand);
            }
        }

        Ok(())
    }

    fn connect(&self, addr: SocketAddrRef) -> Result<(), SocketError> {
        match addr {
            SocketAddrRef::Abstract(name) => {
                let name: Name = (*name)
                    .try_into()
                    .map_err(|()| SocketError::InvalidArgument)?;

                let socket_bindings = SOCKET_ABSTRACT_BINDINGS.read();
                if let Some(socket) = socket_bindings.get(&name) {
                    let socket = socket.clone();
                    drop(socket_bindings);

                    self.try_connect_with(socket)
                } else {
                    Err(SocketError::UnknownAddress)
                }
            }
            _ => Err(SocketError::InvalidArgument),
        }
    }

    fn bind(&self, addr: SocketAddrRef) -> Result<(), SocketError> {
        match addr {
            SocketAddrRef::Abstract(name) => {
                let mut our_name = self.binded_to.lock();
                if our_name.is_some() {
                    return Err(SocketError::AlreadyBinded);
                }

                let name: Name = (*name)
                    .try_into()
                    .map_err(|()| SocketError::InvalidArgument)?;

                let mut bindings = SOCKET_ABSTRACT_BINDINGS.write();
                if bindings.contains_key(&name) {
                    return Err(SocketError::AddressInUse);
                }

                let this = self
                    .this
                    .upgrade()
                    .expect("Upgrading LocalSocket::this should never fail");
                let arc_name = Arc::new(name);
                bindings.insert(arc_name.clone(), this);
                *our_name = Some(arc_name);
                Ok(())
            }
            _ => Err(SocketError::InvalidArgument),
        }
    }

    fn receive(
        &self,
        buf: &mut [u8],
        flags: safa_abi::sockets::SockMsgFlags,
    ) -> Result<usize, SocketError> {
        self.read_self(
            buf,
            flags.contains(SockMsgFlags::PEEK),
            !flags.contains(SockMsgFlags::DONT_WAIT),
        )
    }

    fn send_to(
        &self,
        buf: &[u8],
        flags: SockMsgFlags,
        addr: SocketAddrRef,
    ) -> Result<usize, SocketError> {
        _ = buf;
        _ = addr;
        _ = flags;
        Err(SocketError::OperationNotSupported)
    }

    fn send(&self, buf: &[u8], flags: SockMsgFlags) -> Result<usize, SocketError> {
        self.write_connected(buf, !flags.contains(SockMsgFlags::DONT_WAIT))
    }

    fn listen(&self, backlog: usize) -> Result<(), SocketError> {
        self.listen(backlog)
    }

    fn on_close(&self) {
        self.on_close()
    }

    fn poll_id(&self) -> PollID {
        self.poll_id()
    }
}
