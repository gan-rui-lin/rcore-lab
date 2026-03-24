//! Socket file descriptor: implements the File trait for network sockets.

use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use log::warn;
use smoltcp::iface::SocketHandle;
use smoltcp::socket::{tcp, udp};
use smoltcp::wire::IpEndpoint;

use crate::fs::{File, PollEvents};
use crate::mm::UserBuffer;
use crate::task::suspend_current_and_run_next;

use super::{poll_net, NET_STACK};

/// Socket protocol type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SocketType {
    /// TCP stream socket.
    Tcp,
    /// UDP datagram socket.
    Udp,
}

/// A network socket wrapped as a file descriptor.
pub struct SocketFile {
    /// The smoltcp socket handle.
    pub handle: SocketHandle,
    /// The socket protocol type (TCP or UDP).
    pub sock_type: SocketType,
    /// Whether the socket is non-blocking (SOCK_NONBLOCK).
    pub nonblock: bool,
    /// Whether FD_CLOEXEC is set (SOCK_CLOEXEC).
    pub cloexec: bool,
    /// Bound local port (for TCP bind → listen → accept flow). Atomic for interior mutability.
    pub bound_port: AtomicU16,
    /// Whether TCP socket is in listening state.
    pub listening: AtomicBool,
    /// If true, Drop will NOT abort/remove the socket (ownership transferred to another fd).
    pub transferred: AtomicBool,
    /// UDP connected remote endpoint (set by connect() for write()/send() support).
    pub connected_remote: spin::Mutex<Option<IpEndpoint>>,
}

impl SocketFile {
    /// Create a new SocketFile with the given handle and type.
    pub fn new(handle: SocketHandle, sock_type: SocketType) -> Self {
        Self {
            handle,
            sock_type,
            nonblock: false,
            cloexec: false,
            bound_port: AtomicU16::new(0),
            listening: AtomicBool::new(false),
            transferred: AtomicBool::new(false),
            connected_remote: spin::Mutex::new(None),
        }
    }
}

impl File for SocketFile {
    fn readable(&self) -> bool {
        true
    }

    fn writable(&self) -> bool {
        true
    }

    fn read(&self, user_buf: UserBuffer) -> usize {
        match self.sock_type {
            SocketType::Tcp => self.tcp_read(user_buf),
            SocketType::Udp => self.udp_read(user_buf),
        }
    }

    fn write(&self, user_buf: UserBuffer) -> usize {
        match self.sock_type {
            SocketType::Tcp => self.tcp_write(user_buf),
            SocketType::Udp => self.udp_write(user_buf),
        }
    }

    fn poll(&self, events: PollEvents) -> PollEvents {
        poll_net();
        let mut net = NET_STACK.exclusive_access();
        let stack = match net.as_mut() {
            Some(s) => s,
            None => return PollEvents::empty(),
        };
        let mut result = PollEvents::empty();

        match self.sock_type {
            SocketType::Tcp => {
                let socket = stack.sockets.get_mut::<tcp::Socket>(self.handle);
                let state = socket.state();
                use smoltcp::socket::tcp::State;
                // A socket is "was connected" if it's in any state past SynSent
                let was_connected = !matches!(state, State::Closed | State::Listen | State::SynSent | State::SynReceived);
                // POLLIN: data available OR peer sent FIN on an established connection
                if events.contains(PollEvents::POLLIN) && (socket.can_recv() || (was_connected && !socket.may_recv())) {
                    result |= PollEvents::POLLIN;
                }
                if events.contains(PollEvents::POLLOUT) && socket.can_send() {
                    result |= PollEvents::POLLOUT;
                }
                // POLLHUP: connection fully closed after being established
                if !socket.is_open() && was_connected {
                    result |= PollEvents::POLLHUP;
                }
            }
            SocketType::Udp => {
                let socket = stack.sockets.get_mut::<udp::Socket>(self.handle);
                if events.contains(PollEvents::POLLIN) && socket.can_recv() {
                    result |= PollEvents::POLLIN;
                }
                if events.contains(PollEvents::POLLOUT) && socket.can_send() {
                    result |= PollEvents::POLLOUT;
                }
            }
        }
        result
    }

    fn as_socket(&self) -> Option<(SocketHandle, SocketType)> {
        Some((self.handle, self.sock_type))
    }

    fn set_connected_remote(&self, addr: IpEndpoint) {
        *self.connected_remote.lock() = Some(addr);
    }

    fn get_connected_remote(&self) -> Option<IpEndpoint> {
        *self.connected_remote.lock()
    }

    fn fd_flags(&self) -> u32 {
        if self.cloexec { 1 } else { 0 } // FD_CLOEXEC = 1
    }

    fn status_flags(&self) -> u32 {
        let mut flags = 0b10u32; // O_RDWR = 2
        if self.nonblock {
            flags |= 0o4000; // O_NONBLOCK
        }
        flags
    }

    fn bound_port(&self) -> u16 {
        self.bound_port.load(Ordering::Relaxed)
    }

    fn set_bound_port(&self, port: u16) {
        self.bound_port.store(port, Ordering::Relaxed);
    }

    fn is_listening(&self) -> bool {
        self.listening.load(Ordering::Relaxed)
    }

    fn set_listening(&self, listening: bool) {
        self.listening.store(listening, Ordering::Relaxed);
    }

    fn mark_transferred(&self) {
        self.transferred.store(true, Ordering::Relaxed);
    }
}

impl SocketFile {
    fn tcp_read(&self, mut user_buf: UserBuffer) -> usize {
        loop {
            poll_net();
            let mut net = NET_STACK.exclusive_access();
            let stack = match net.as_mut() {
                Some(s) => s,
                None => return 0,
            };
            let socket = stack.sockets.get_mut::<tcp::Socket>(self.handle);

            if socket.can_recv() {
                let mut total = 0;
                for slice in user_buf.buffers.iter_mut() {
                    match socket.recv_slice(slice) {
                        Ok(n) if n > 0 => total += n,
                        _ => break,
                    }
                }
                if total > 0 {
                    return total;
                }
            }

            // EOF: remote closed and no more data
            if !socket.may_recv() {
                return 0;
            }

            if self.nonblock {
                return usize::MAX; // -EAGAIN encoded
            }

            drop(net);
            suspend_current_and_run_next();
        }
    }

    fn tcp_write(&self, user_buf: UserBuffer) -> usize {
        loop {
            poll_net();
            let mut net = NET_STACK.exclusive_access();
            let stack = match net.as_mut() {
                Some(s) => s,
                None => return 0,
            };
            let socket = stack.sockets.get_mut::<tcp::Socket>(self.handle);

            if !socket.may_send() {
                return 0; // Connection closed
            }

            if socket.can_send() {
                let mut total = 0;
                for slice in user_buf.buffers.iter() {
                    match socket.send_slice(slice) {
                        Ok(n) if n > 0 => total += n,
                        _ => break,
                    }
                }
                if total > 0 {
                    // Flush through loopback so peer can receive immediately
                    let now = super::smoltcp_now();
                    for _ in 0..4 {
                        stack.lo_iface.poll(now, &mut stack.lo_device, &mut stack.sockets);
                    }
                    stack.poll_external(now);
                    return total;
                }
            }

            if self.nonblock {
                return usize::MAX;
            }

            drop(net);
            suspend_current_and_run_next();
        }
    }

    fn udp_read(&self, mut user_buf: UserBuffer) -> usize {
        loop {
            poll_net();
            let mut net = NET_STACK.exclusive_access();
            let stack = match net.as_mut() {
                Some(s) => s,
                None => return 0,
            };
            let socket = stack.sockets.get_mut::<udp::Socket>(self.handle);

            if socket.can_recv() {
                // Collect into a contiguous buffer first
                let mut tmp = [0u8; 65536];
                match socket.recv_slice(&mut tmp) {
                    Ok((n, _endpoint)) => {
                        // Copy to user buffer
                        let mut offset = 0;
                        for slice in user_buf.buffers.iter_mut() {
                            let remain = n - offset;
                            if remain == 0 {
                                break;
                            }
                            let copy_len = slice.len().min(remain);
                            slice[..copy_len].copy_from_slice(&tmp[offset..offset + copy_len]);
                            offset += copy_len;
                        }
                        return offset;
                    }
                    Err(_) => {}
                }
            }

            if self.nonblock {
                return usize::MAX;
            }

            drop(net);
            suspend_current_and_run_next();
        }
    }

    fn udp_write(&self, user_buf: UserBuffer) -> usize {
        use smoltcp::wire::{IpAddress, IpEndpoint};
        use smoltcp::socket::udp;

        let remote = match *self.connected_remote.lock() {
            Some(ep) => ep,
            None => {
                warn!("[net] UDP write: no connected remote, use sendto");
                return 0;
            }
        };
        let mut data = alloc::vec::Vec::new();
        for buf in user_buf.buffers.iter() {
            data.extend_from_slice(buf);
        }
        if data.is_empty() {
            return 0;
        }
        let is_loopback = match remote.addr {
            IpAddress::Ipv4(v4) => v4.as_bytes()[0] == 127,
        };

        poll_net();
        let mut net = NET_STACK.exclusive_access();
        let stack = match net.as_mut() {
            Some(s) => s,
            None => return 0,
        };

        // Auto-bind if not yet bound
        {
            let socket = stack.sockets.get_mut::<udp::Socket>(self.handle);
            if !socket.is_open() {
                let local_port = super::alloc_ephemeral_port();
                let _ = socket.bind(smoltcp::wire::IpListenEndpoint {
                    addr: None,
                    port: local_port,
                });
            }
        }

        if is_loopback {
            // Loopback: inject directly into target socket's rx buffer
            let sender_port = stack
                .sockets
                .get_mut::<udp::Socket>(self.handle)
                .endpoint()
                .port;
            let sender_meta = udp::UdpMetadata {
                endpoint: IpEndpoint::new(IpAddress::v4(127, 0, 0, 1), sender_port),
                meta: Default::default(),
            };
            let target_port = remote.port;
            super::loopback_udp_inject(stack, self.handle, target_port, &data, sender_meta);
            data.len()
        } else {
            let socket = stack.sockets.get_mut::<udp::Socket>(self.handle);
            let _ = socket.send_slice(&data, remote);
            drop(net);
            poll_net();
            data.len()
        }
    }
}

impl Drop for SocketFile {
    fn drop(&mut self) {
        // If ownership was transferred (e.g., accept swapped the handle), don't clean up
        if self.transferred.load(Ordering::Relaxed) {
            return;
        }
        let mut net = NET_STACK.exclusive_access();
        if let Some(ref mut stack) = *net {
            match self.sock_type {
                SocketType::Tcp => {
                    let socket = stack.sockets.get_mut::<tcp::Socket>(self.handle);
                    // Use close() instead of abort() to send FIN gracefully.
                    // This ensures pending TX data (like iperf IPERF_DONE state)
                    // reaches the peer before the connection is torn down.
                    socket.close();
                    // Flush the FIN through loopback immediately.
                    let now = super::smoltcp_now();
                    stack.lo_iface.poll(now, &mut stack.lo_device, &mut stack.sockets);
                    stack.poll_external(now);
                    // Don't remove the socket yet - smoltcp needs it for FIN handshake.
                    // It will be cleaned up when the socket enters Closed state.
                    // For simplicity, remove it anyway (smoltcp handles orphan FINs).
                    stack.sockets.remove(self.handle);
                }
                SocketType::Udp => {
                    let socket = stack.sockets.get_mut::<udp::Socket>(self.handle);
                    socket.close();
                    stack.sockets.remove(self.handle);
                }
            }
        }
    }
}
