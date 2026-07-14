use core::{
    net::{Ipv4Addr, SocketAddrV4},
    num::NonZero,
    sync::atomic::{AtomicU8, Ordering},
};

use alloc::{
    boxed::Box,
    collections::linked_list::LinkedList,
    sync::{Arc, Weak},
};
use safa_abi::{
    errors::{ErrorStatus, IntoErr},
    poll::PollEvents,
    sockets::SockMsgFlags,
};

use crate::{
    debug,
    memory::vmm::VMMAlloc,
    net::{
        self,
        icmp::{EchoIcmpPacket, ICMPType, IcmpPacket},
        ipv4::{DEFAULT_TTL, PageIPv4Packet},
    },
    process::poll::{self, PollID},
    scheduler::wait_queue::WaitQueue,
    sockets::{OwnedSocketAddr, Socket, SocketAddrRef, SocketError},
    syscalls::ffi::SyscallFFI,
    utils::locks::{Mutex, RwLock},
    warn,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpState {
    Disconnected,
    Connected { ip: Ipv4Addr, port: u16 },
}

#[derive(Debug)]
pub struct Message {
    src_ip: Ipv4Addr,
    src_port: u16,
    payload: Box<[u8], VMMAlloc>,
}

#[derive(Debug)]
pub struct BindInfo {
    ip: Ipv4Addr,
    port: NonZero<u16>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatagramProtocol {
    UDP,
    /// An ICMP datagram socket, contains the ID of data sent using this socket, custom IDs aren't supported yet.
    ICMP(u16),
}

pub struct UdpSocket {
    protocol: DatagramProtocol,
    timeout_info: RwLock<TimeoutInfo>,
    error: Mutex<Option<SocketError>>,
    ttl: AtomicU8,
    state: RwLock<UdpState>,
    messages: Mutex<LinkedList<Message>>,
    binded_to: RwLock<Option<BindInfo>>,
    wait_queue: Mutex<WaitQueue<1, ()>>,
    weak: Weak<Self>,
}

impl UdpSocket {
    #[inline]
    pub fn last_error(&self) -> Option<SocketError> {
        // TODO: Is the atomic operation correct?
        *self.error.lock()
    }

    #[inline]
    pub fn take_error(&self) -> Option<SocketError> {
        self.error.lock().take()
    }

    #[inline]
    pub fn set_error(&self, error: SocketError) {
        let mut wait_queue = self.wait_queue.lock();
        *self.error.lock() = Some(error);
        wait_queue.wake_all();
    }

    pub fn create(can_block: bool, protocol: DatagramProtocol) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            error: Mutex::new(None),
            protocol,
            ttl: AtomicU8::new(DEFAULT_TTL),
            timeout_info: RwLock::new(TimeoutInfo {
                read_timeout: None,
                write_timeout: None,
                can_block,
            }),
            state: RwLock::new(UdpState::Disconnected),
            messages: Mutex::new(LinkedList::new()),
            binded_to: RwLock::new(None),
            wait_queue: Mutex::new(WaitQueue::new()),
            weak: weak.clone(),
        })
    }

    pub fn write(&self, from_addr: Ipv4Addr, from_port: u16, data: &[u8]) -> usize {
        let mut messages = self.messages.lock();
        let data_size_was = messages.len();
        messages.push_back(Message {
            src_ip: from_addr,
            src_port: from_port,
            payload: data
                .to_vec_in(VMMAlloc::new(&"sockets::udp::Message"))
                .into_boxed_slice(),
        });

        self.wait_queue.lock().wake_n_on_condition(|()| true, 1);
        let events_add = if data_size_was == 0 {
            PollEvents::DATA_AVAILABLE
        } else {
            PollEvents::NONE
        };

        poll::broadcast_events(self.poll_id(), events_add, PollEvents::NONE);
        data.len()
    }

    #[inline]
    pub fn this(&self) -> Arc<Self> {
        self.weak
            .upgrade()
            .expect("Upgrading socket's weak should never fail if we have a reference")
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
            super::SocketOpt::SockError => return Err(ErrorStatus::InvalidCommand),
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
            super::SocketOpt::SockError => {
                let r = <&mut u16>::make(to_usr_ptr.cast())?;
                let err = self.take_error();
                *r = err.map(|e| e.into_err() as u16).unwrap_or(0);
            }
            super::SocketOpt::IpBroadcast => {
                // TODO: broadcast permissions...
                let r = <&mut bool>::make(to_usr_ptr.cast())?;
                *r = true;
            }
        }

        Ok(())
    }

    fn accept(&self) -> Result<alloc::sync::Arc<dyn Socket>, super::SocketError> {
        Err(SocketError::OperationNotSupported)
    }

    fn listen(&self, backlog: usize) -> Result<(), SocketError> {
        _ = backlog;
        Err(SocketError::OperationNotSupported)
    }

    fn connect(&self, addr: super::SocketAddrRef) -> Result<(), SocketError> {
        if self.protocol != DatagramProtocol::UDP {
            return Err(SocketError::OperationNotSupported);
        }

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
        if self.protocol != DatagramProtocol::UDP {
            return Err(SocketError::OperationNotSupported);
        }

        match addr {
            SocketAddrRef::Ip { addr, port } => {
                let mut binded_to = self.binded_to.write();
                let binded_port = crate::net::udp::bind_socket(NonZero::new(port), self.this())
                    .map_err(|()| SocketError::AddressInUse)?;
                *binded_to = Some(BindInfo {
                    ip: addr,
                    port: binded_port,
                });
                debug!(UdpSocket, "Binded to {}:{}", addr, binded_port);
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
        crate::net::manager::send_ipv4_packet(
            binded_to.ip,
            &mut PageIPv4Packet::new_udp(
                buf,
                binded_to.port.get(),
                dst_port,
                dst_addr,
                self.ttl.load(Ordering::Acquire),
            ),
        )?;

        // TODO: truncate the buffer if it's too large...
        Ok(buf.len())
    }

    fn send_to(
        &self,
        buf: &[u8],
        flags: safa_abi::sockets::SockMsgFlags,
        addr: SocketAddrRef,
    ) -> Result<usize, SocketError> {
        let addr = match addr {
            SocketAddrRef::Ip { addr, port } => SocketAddrV4::new(addr, port),
            _ => return Err(SocketError::OperationNotSupported),
        };
        let dst_addr = *addr.ip();
        let dst_port = addr.port();

        match self.protocol {
            DatagramProtocol::UDP => {
                _ = flags;
                let binded_to = self.binded_to.read();
                let binded_to = binded_to.as_ref().ok_or(SocketError::NotBound)?;
                // TODO: verify destination address and port before doing allocations
                // TODO: don't do allocations at all and do write timeout
                crate::net::manager::send_ipv4_packet(
                    binded_to.ip,
                    &mut PageIPv4Packet::new_udp(
                        buf,
                        binded_to.port.get(),
                        dst_port,
                        dst_addr,
                        self.ttl.load(Ordering::Acquire),
                    ),
                )?;

                // TODO: truncate the buffer if it's too large...
                Ok(buf.len())
            }
            DatagramProtocol::ICMP(id) => {
                let packet = IcmpPacket::new(buf).ok_or(SocketError::InvalidArgument)?;
                let ty = packet.header().ty();

                match ty {
                    ICMPType::ECHO_REQUEST => {
                        let echo_packet =
                            EchoIcmpPacket::new(packet).ok_or(SocketError::InvalidArgument)?;

                        net::icmp::send_echo_request(
                            dst_addr,
                            self.this(),
                            echo_packet,
                            Some(id),
                            self.ttl.load(Ordering::Acquire),
                        )
                        .map_err(|e| e.into())
                        .map(|()| buf.len())
                    }
                    o => {
                        warn!(UdpSocket, "Attempt to send unknown ICMP type: {o:?}");
                        Err(SocketError::InvalidArgument)
                    }
                }
            }
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
        let message = messages.front();
        let Some(message) = message else {
            if can_block {
                // error wasn't handled
                if let Some(err) = self.last_error() {
                    return Err(err);
                }

                let pending_wait = self.wait_queue.prepare_wait();
                drop(messages);
                // FIXME: retrying doesn't respect timeout...
                pending_wait.enter_wait((), timeout_info.read_timeout)?;
                if let Some(err) = self.last_error() {
                    return Err(err);
                }

                return self.recv_from(buf, flags);
            } else {
                return Err(SocketError::WouldBlockEmpty);
            }
        };

        let size = message.payload.len().min(buf.len());
        buf[..size].copy_from_slice(&message.payload[..size]);

        let src_ip = message.src_ip;
        let src_port = message.src_port;
        if !flags.contains(SockMsgFlags::PEEK) {
            messages.pop_front();
        }

        let len_is = messages.len();
        if len_is == 0 {
            // Everything removed...
            poll::broadcast_events(self.poll_id(), PollEvents::NONE, PollEvents::DATA_AVAILABLE);
        }

        Ok((
            size,
            Some(OwnedSocketAddr::Ip {
                addr: src_ip,
                port: src_port,
            }),
        ))
    }

    fn receive(&self, buf: &mut [u8], flags: SockMsgFlags) -> Result<usize, SocketError> {
        self.recv_from(buf, flags).map(|(size, _)| size)
    }

    fn on_close(&self) {
        match self.protocol {
            DatagramProtocol::UDP => {
                let _b = self.binded_to.write().take();
                drop(_b);
            }
            DatagramProtocol::ICMP(_) => {
                net::icmp::cleanup_icmp_socket(&self.this());
            }
        }
    }
}
