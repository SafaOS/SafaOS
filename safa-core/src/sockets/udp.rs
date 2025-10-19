use core::{
    net::Ipv4Addr,
    num::NonZero,
    sync::atomic::{AtomicU8, Ordering},
};

use alloc::{
    collections::linked_list::LinkedList,
    sync::{Arc, Weak},
};
use safa_abi::{errors::ErrorStatus, poll::PollEvents, sockets::SockMsgFlags};

use crate::{
    debug,
    net::ipv4::{DEFAULT_TTL, PageIPv4Packet},
    process::poll::{self, PollID},
    scheduler::wait_queue::WaitQueue,
    sockets::{OwnedSocketAddr, Socket, SocketAddrRef, SocketError, unix::Stream},
    syscalls::ffi::SyscallFFI,
    utils::locks::{Mutex, RwLock},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpState {
    Disconnected,
    Connected { ip: Ipv4Addr, port: u16 },
}

#[derive(Debug, Clone, Copy)]
pub struct Message {
    src_ip: Ipv4Addr,
    src_port: u16,
    size: usize,
}

pub struct Messages {
    messages: LinkedList<Message>,
    stream: Stream,
}

impl Messages {
    pub fn new() -> Self {
        Self {
            messages: LinkedList::new(),
            stream: Stream::new(),
        }
    }
}

#[derive(Debug)]
pub struct BindInfo {
    ip: Ipv4Addr,
    port: u16,
}

impl Drop for BindInfo {
    fn drop(&mut self) {
        assert!(crate::net::udp::remove_socket(self.port));
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TimeoutInfo {
    read_timeout: Option<NonZero<u64>>,
    write_timeout: Option<NonZero<u64>>,
    can_block: bool,
}

pub struct UdpSocket {
    timeout_info: RwLock<TimeoutInfo>,
    ttl: AtomicU8,
    state: RwLock<UdpState>,
    messages: Mutex<Messages>,
    binded_to: RwLock<Option<BindInfo>>,
    wait_queue: Mutex<WaitQueue<1, ()>>,
    weak: Weak<Self>,
}

impl UdpSocket {
    pub fn create(can_block: bool) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            ttl: AtomicU8::new(DEFAULT_TTL),
            timeout_info: RwLock::new(TimeoutInfo {
                read_timeout: None,
                write_timeout: None,
                can_block,
            }),
            state: RwLock::new(UdpState::Disconnected),
            messages: Mutex::new(Messages::new()),
            binded_to: RwLock::new(None),
            wait_queue: Mutex::new(WaitQueue::new()),
            weak: weak.clone(),
        })
    }

    pub fn write(
        &self,
        from_addr: Ipv4Addr,
        from_port: u16,
        data: &[u8],
    ) -> Result<usize, SocketError> {
        let mut messages = self.messages.lock();
        let (data_size, _, data_size_was) = messages.stream.write(data)?;

        messages.messages.push_back(Message {
            src_ip: from_addr,
            src_port: from_port,
            size: data_size,
        });

        self.wait_queue.lock().wake_n_on_condition(|()| true, 1);
        let events_add = if data_size_was == 0 {
            PollEvents::DATA_AVAILABLE
        } else {
            PollEvents::NONE
        };

        poll::broadcast_events(self.poll_id(), events_add, PollEvents::NONE);
        Ok(data_size)
    }
}

impl Socket for UdpSocket {
    fn set_sock_opt(&self, opt: super::SocketOpt, value: u64) -> Result<(), ErrorStatus> {
        match opt {
            super::SocketOpt::Blocking => self.timeout_info.write().can_block = value > 0,
            super::SocketOpt::IpTTL => self.ttl.store(value as u8, Ordering::Release),
            super::SocketOpt::ReadTimeout => {
                self.timeout_info.write().read_timeout = NonZero::new(value)
            }
            super::SocketOpt::WriteTimeout => {
                self.timeout_info.write().write_timeout = NonZero::new(value)
            }
            super::SocketOpt::SockError => todo!(),
            super::SocketOpt::IpBroadcast => {
                // TODO: broadcast permissions
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
            super::SocketOpt::IpTTL => {
                let r = <&mut u8>::make(to_usr_ptr.cast())?;
                let ttl = self.ttl.load(Ordering::Acquire);
                *r = ttl;
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
            super::SocketOpt::SockError => todo!(),
            super::SocketOpt::IpBroadcast => {
                // TODO: broadcast permissions...
                let r = <&mut bool>::make(to_usr_ptr.cast())?;
                *r = true;
            }
        }

        Ok(())
    }

    fn accept(&self) -> Result<alloc::sync::Arc<Self>, super::SocketError> {
        Err(SocketError::OperationNotSupported)
    }

    fn listen(&self, backlog: usize) -> Result<(), SocketError> {
        _ = backlog;
        Err(SocketError::OperationNotSupported)
    }

    fn connect(&self, addr: super::SocketAddrRef) -> Result<(), SocketError> {
        match addr {
            SocketAddrRef::Ip { addr, port } => {
                let mut state = self.state.write();
                *state = UdpState::Connected { ip: addr, port };
                Ok(())
            }
            SocketAddrRef::Abstract(_) => Err(SocketError::OperationNotSupported),
        }
    }

    fn bind(&self, addr: SocketAddrRef) -> Result<(), SocketError> {
        match addr {
            SocketAddrRef::Ip { addr, port } => {
                let mut binded_to = self.binded_to.write();
                crate::net::udp::bind_socket(
                    port,
                    self.weak
                        .upgrade()
                        .expect("Upgradng UdpSocket::Weak should never fail..."),
                )
                .map_err(|()| SocketError::AddressInUse)?;
                *binded_to = Some(BindInfo { ip: addr, port });
                debug!(UdpSocket, "Binded to {}:{}", addr, port);
                Ok(())
            }
            SocketAddrRef::Abstract(_) => Err(SocketError::OperationNotSupported),
        }
    }

    fn send(
        &self,
        buf: &[u8],
        flags: safa_abi::sockets::SockMsgFlags,
    ) -> Result<usize, SocketError> {
        _ = flags;
        let UdpState::Connected {
            ip: dst_addr,
            port: dst_port,
        } = *self.state.read()
        else {
            return Err(SocketError::ConnectionClosed);
        };

        let binded_to = self.binded_to.read();
        let binded_to = binded_to.as_ref().ok_or(SocketError::NotBound)?;
        // TODO: verify destination address and port before doing allocations
        // TODO: don't do allocations at all and do write timeout
        crate::net::manager::send_ipv4_packet(&mut PageIPv4Packet::new_udp(
            buf,
            binded_to.port,
            dst_port,
            dst_addr,
            self.ttl.load(Ordering::Acquire),
        ))?;

        // TODO: truncate the buffer if it's too large...
        Ok(buf.len())
    }

    fn send_to(
        &self,
        buf: &[u8],
        flags: safa_abi::sockets::SockMsgFlags,
        addr: SocketAddrRef,
    ) -> Result<usize, SocketError> {
        match addr {
            SocketAddrRef::Ip {
                addr: dst_addr,
                port: dst_port,
            } => {
                _ = flags;
                let binded_to = self.binded_to.read();
                let binded_to = binded_to.as_ref().ok_or(SocketError::NotBound)?;
                // TODO: verify destination address and port before doing allocations
                // TODO: don't do allocations at all and do write timeout
                crate::net::manager::send_ipv4_packet(&mut PageIPv4Packet::new_udp(
                    buf,
                    binded_to.port,
                    dst_port,
                    dst_addr,
                    self.ttl.load(Ordering::Acquire),
                ))?;

                // TODO: truncate the buffer if it's too large...
                Ok(buf.len())
            }
            SocketAddrRef::Abstract(_) => Err(SocketError::OperationNotSupported),
        }
    }

    fn poll_id(&self) -> crate::process::poll::PollID {
        PollID::from_ptr(self)
    }

    fn recv_from(
        &self,
        buf: &mut [u8],
        flags: safa_abi::sockets::SockMsgFlags,
    ) -> Result<(usize, Option<super::OwnedSocketAddr>), SocketError> {
        let timeout_info = *self.timeout_info.read();
        let can_block = !flags.contains(SockMsgFlags::DONT_WAIT) && timeout_info.can_block;

        let mut messages = self.messages.lock();
        let message = if flags.contains(SockMsgFlags::PEEK) {
            messages.messages.front().copied()
        } else {
            messages.messages.pop_front()
        };
        let Some(message) = message else {
            if can_block {
                let pending_wait = self.wait_queue.prepare_wait();
                drop(messages);
                // FIXME: retrying doesn't respect timeout...
                pending_wait.enter_wait((), timeout_info.read_timeout)?;
                return self.recv_from(buf, flags);
            } else {
                return Err(SocketError::WouldBlockEmpty);
            }
        };

        let size = message.size.min(buf.len());
        let (amount, len_is, _) = messages
            .stream
            .read(&mut buf[..size], flags.contains(SockMsgFlags::PEEK))
            .expect("Shouldn't return an error if there is messages...");

        if len_is == 0 {
            // Everything removed...
            poll::broadcast_events(self.poll_id(), PollEvents::NONE, PollEvents::DATA_AVAILABLE);
        }

        Ok((
            amount,
            Some(OwnedSocketAddr::Ip {
                addr: message.src_ip,
                port: message.src_port,
            }),
        ))
    }

    fn receive(&self, buf: &mut [u8], flags: SockMsgFlags) -> Result<usize, SocketError> {
        self.recv_from(buf, flags).map(|(size, _)| size)
    }

    fn on_close(&self) {
        let _b = self.binded_to.write().take();
        drop(_b);
    }
}
