//! Network system call implementations.

use alloc::sync::Arc;
use alloc::vec;
use core::sync::atomic::{AtomicBool, AtomicU16};
use log::warn;
use smoltcp::iface::SocketHandle;
use smoltcp::socket::{tcp, udp};
use smoltcp::wire::{IpAddress, IpEndpoint, IpListenEndpoint, Ipv4Address};

use crate::mm::{translated_byte_buffer, translated_refmut};
use crate::task::{current_process, current_user_token, has_pending_unmasked_signal, suspend_current_and_run_next};

use super::socket_file::{SocketFile, SocketType};
use super::{alloc_ephemeral_port, poll_net, NET_STACK};

// Address family
const AF_INET: usize = 2;

// Socket types
const SOCK_STREAM: usize = 1;
const SOCK_DGRAM: usize = 2;
const SOCK_NONBLOCK: usize = 0o4000;
const SOCK_CLOEXEC: usize = 0o2000000;

// Socket options
const SOL_SOCKET: usize = 1;
const IPPROTO_TCP: usize = 6;
const SO_REUSEADDR: usize = 2;
const SO_ERROR: usize = 4;
const SO_KEEPALIVE: usize = 9;
const SO_SNDBUF: usize = 7;
const SO_RCVBUF: usize = 8;
const SO_RCVTIMEO: usize = 20;
const SO_SNDTIMEO: usize = 21;
const TCP_NODELAY: usize = 1;

// Errno values
const EINTR: isize = -4;
const EBADF: isize = -9;
const EINVAL: isize = -22;
const ENOTSOCK: isize = -88;
const EAFNOSUPPORT: isize = -97;
const EADDRINUSE: isize = -98;
const ENOTCONN: isize = -107;
const ECONNREFUSED: isize = -111;
const EFAULT: isize = -14;
const EMFILE: isize = -24;
const EMSGSIZE: isize = -90;
const EOPNOTSUPP: isize = -95;

/// Linux sockaddr_in structure (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct SockAddrIn {
    sin_family: u16,
    sin_port: u16,   // big-endian
    sin_addr: u32,   // big-endian
    sin_zero: [u8; 8],
}

/// Convert user-space sockaddr to smoltcp IpEndpoint.
fn read_sockaddr(addr_ptr: *const u8, addr_len: usize, token: usize) -> Option<IpEndpoint> {
    if addr_ptr.is_null() || addr_len < core::mem::size_of::<SockAddrIn>() {
        return None;
    }
    let bufs = translated_byte_buffer(token, addr_ptr, core::mem::size_of::<SockAddrIn>());
    let mut raw = [0u8; 16];
    let mut offset = 0;
    for buf in bufs.iter() {
        let n = buf.len().min(16 - offset);
        raw[offset..offset + n].copy_from_slice(&buf[..n]);
        offset += n;
    }
    let family = u16::from_ne_bytes([raw[0], raw[1]]);
    if family != AF_INET as u16 {
        return None;
    }
    let port = u16::from_be_bytes([raw[2], raw[3]]);
    let ip = Ipv4Address::new(raw[4], raw[5], raw[6], raw[7]);
    Some(IpEndpoint::new(IpAddress::Ipv4(ip), port))
}

/// Write smoltcp IpEndpoint back to user-space sockaddr.
fn write_sockaddr(ep: &IpEndpoint, addr_ptr: *mut u8, addrlen_ptr: *mut u32, token: usize) {
    if addr_ptr.is_null() {
        return;
    }
    let mut raw = [0u8; 16];
    raw[0] = AF_INET as u8;
    raw[1] = 0;
    let port_bytes = ep.port.to_be_bytes();
    raw[2] = port_bytes[0];
    raw[3] = port_bytes[1];
    let IpAddress::Ipv4(ipv4) = ep.addr;
    let octets = ipv4.as_bytes();
    raw[4] = octets[0];
    raw[5] = octets[1];
    raw[6] = octets[2];
    raw[7] = octets[3];

    let bufs = translated_byte_buffer(token, addr_ptr, 16);
    let mut offset = 0;
    for buf in bufs.iter() {
        let n = buf.len().min(16 - offset);
        // Safety: translated_byte_buffer gives mutable kernel-mapped slices
        let dst = unsafe { core::slice::from_raw_parts_mut(buf.as_ptr() as *mut u8, n) };
        dst.copy_from_slice(&raw[offset..offset + n]);
        offset += n;
    }

    if !addrlen_ptr.is_null() {
        let len_ref = translated_refmut(token, addrlen_ptr);
        *len_ref = 16;
    }
}

/// Helper: get socket handle and type from fd.
fn get_socket_info(fd: usize) -> Result<(SocketHandle, SocketType), isize> {
    let process = current_process();
    let inner = process.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return Err(EBADF);
    }
    match inner.fd_table[fd].as_ref() {
        Some(file) => match file.as_socket() {
            Some((handle, sock_type)) => Ok((handle, sock_type)),
            None => Err(ENOTSOCK),
        },
        None => Err(EBADF),
    }
}

/// Helper: get socket file's bound_port and listening state.
fn get_socket_extra(fd: usize) -> Result<(SocketHandle, SocketType, u16, bool), isize> {
    let process = current_process();
    let inner = process.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return Err(EBADF);
    }
    match inner.fd_table[fd].as_ref() {
        Some(file) => match file.as_socket() {
            Some((handle, sock_type)) => {
                let bound_port = file.bound_port();
                let listening = file.is_listening();
                Ok((handle, sock_type, bound_port, listening))
            }
            None => Err(ENOTSOCK),
        },
        None => Err(EBADF),
    }
}

// ============================================================
// System call implementations
// ============================================================

/// sys_socket(domain, type, protocol) -> fd
pub fn sys_socket(domain: usize, sock_type: usize, _protocol: usize) -> isize {
    if domain != AF_INET {
        return EAFNOSUPPORT;
    }

    let base_type = sock_type & 0xFF;
    let nonblock = (sock_type & SOCK_NONBLOCK) != 0;
    let cloexec = (sock_type & SOCK_CLOEXEC) != 0;

    let st = match base_type {
        SOCK_STREAM => SocketType::Tcp,
        SOCK_DGRAM => SocketType::Udp,
        _ => return EINVAL,
    };

    let handle = {
        let mut net = NET_STACK.exclusive_access();
        let stack = match net.as_mut() {
            Some(s) => s,
            None => return EINVAL,
        };
        match st {
            SocketType::Tcp => {
                let rx_buf = tcp::SocketBuffer::new(vec![0u8; 65536]);
                let tx_buf = tcp::SocketBuffer::new(vec![0u8; 65536]);
                let socket = tcp::Socket::new(rx_buf, tx_buf);
                stack.sockets.add(socket)
            }
            SocketType::Udp => {
                let rx_buf = udp::PacketBuffer::new(
                    vec![udp::PacketMetadata::EMPTY; 32],
                    vec![0u8; 65536],
                );
                let tx_buf = udp::PacketBuffer::new(
                    vec![udp::PacketMetadata::EMPTY; 32],
                    vec![0u8; 65536],
                );
                let socket = udp::Socket::new(rx_buf, tx_buf);
                stack.sockets.add(socket)
            }
        }
    };

    let mut socket_file = SocketFile::new(handle, st);
    socket_file.nonblock = nonblock;
    socket_file.cloexec = cloexec;

    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    let fd = match inner.alloc_fd() {
        Some(fd) => fd,
        None => return EMFILE,
    };
    inner.fd_table[fd] = Some(Arc::new(socket_file));
    fd as isize
}

/// Convert IpEndpoint to IpListenEndpoint, treating 0.0.0.0 and 127.0.0.1 as wildcard.
fn endpoint_to_listen(ep: &IpEndpoint) -> IpListenEndpoint {
    let addr = match ep.addr {
        IpAddress::Ipv4(v4) => {
            let bytes = v4.as_bytes();
            // 0.0.0.0 = INADDR_ANY, 127.x.x.x = loopback → treat as wildcard
            if (bytes[0] == 0 && bytes[1] == 0 && bytes[2] == 0 && bytes[3] == 0)
                || bytes[0] == 127
            {
                None
            } else {
                Some(ep.addr)
            }
        }
    };
    IpListenEndpoint {
        addr,
        port: ep.port,
    }
}

/// sys_bind(fd, addr, addrlen) -> 0
pub fn sys_bind(fd: usize, addr: *const u8, addr_len: usize) -> isize {
    let token = current_user_token();
    let ep = match read_sockaddr(addr, addr_len, token) {
        Some(ep) => ep,
        None => return EINVAL,
    };

    let (handle, sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    let listen_ep = endpoint_to_listen(&ep);

    let mut net = NET_STACK.exclusive_access();
    let stack = match net.as_mut() {
        Some(s) => s,
        None => return EINVAL,
    };

    match sock_type {
        SocketType::Tcp => {
            // For TCP, store the bound port for later use in listen()
            let port = if listen_ep.port == 0 {
                alloc_ephemeral_port()
            } else {
                listen_ep.port
            };
            // Store via File trait's set_bound_port
            drop(net);
            let process = current_process();
            let inner = process.inner_exclusive_access();
            if let Some(file) = &inner.fd_table[fd] {
                file.set_bound_port(port);
            }
            0
        }
        SocketType::Udp => {
            let socket = stack.sockets.get_mut::<udp::Socket>(handle);
            // port=0 means kernel should auto-assign an ephemeral port
            let bind_ep = if listen_ep.port == 0 {
                IpListenEndpoint {
                    addr: listen_ep.addr,
                    port: alloc_ephemeral_port(),
                }
            } else {
                listen_ep
            };
            match socket.bind(bind_ep) {
                Ok(()) => 0,
                Err(e) => {
                    warn!("[net] UDP bind failed: {:?}", e);
                    EADDRINUSE
                }
            }
        }
    }
}

/// sys_listen(fd, _backlog) -> 0
pub fn sys_listen(fd: usize, _backlog: usize) -> isize {
    let (handle, sock_type, bound_port, _listening) = match get_socket_extra(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    if sock_type != SocketType::Tcp {
        return EOPNOTSUPP;
    }

    let port = if bound_port == 0 {
        alloc_ephemeral_port()
    } else {
        bound_port
    };

    let mut net = NET_STACK.exclusive_access();
    let stack = match net.as_mut() {
        Some(s) => s,
        None => return EINVAL,
    };

    let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
    let listen_ep = IpListenEndpoint {
        addr: None,
        port,
    };
    if let Err(e) = socket.listen(listen_ep) {
        warn!("[net] TCP listen failed: {:?}", e);
        return EINVAL;
    }
    drop(net);

    // Update the socket file's state
    let process = current_process();
    let inner = process.inner_exclusive_access();
    if let Some(file) = &inner.fd_table[fd] {
        file.set_bound_port(port);
        file.set_listening(true);
    }
    0
}

/// sys_accept(listen_fd, addr, addrlen) -> new_fd
pub fn sys_accept(listen_fd: usize, addr: *mut u8, addr_len: *mut u32) -> isize {
    let token = current_user_token();
    let (listen_handle, sock_type, bound_port, _listening) = match get_socket_extra(listen_fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    if sock_type != SocketType::Tcp {
        return EOPNOTSUPP;
    }
    if !_listening || bound_port == 0 {
        return EINVAL;
    }

    // The listen socket is already in LISTEN state from sys_listen().
    // smoltcp's model: the listen socket itself transitions to ESTABLISHED when
    // a SYN arrives. After accept, we need to move the connection to a new socket
    // and put the original back to LISTEN.

    // Wait for the listening socket to become active (SYN received → ESTABLISHED)
    loop {
        poll_net();
        let mut net = NET_STACK.exclusive_access();
        let stack = match net.as_mut() {
            Some(s) => s,
            None => return EINVAL,
        };
        let socket = stack.sockets.get_mut::<tcp::Socket>(listen_handle);

        if socket.is_active() {
            // Connection established! Get remote endpoint.
            let remote_ep = socket.remote_endpoint();
            let _local_ep = socket.local_endpoint();

            // Write peer address to user space
            if let Some(ep) = remote_ep {
                if !addr.is_null() {
                    write_sockaddr(&ep, addr, addr_len, token);
                }
            }

            // Create a NEW socket for the accepted connection by swapping:
            // 1. Create a fresh TCP socket
            // 2. Put it in LISTEN state on the same port
            // 3. The old handle (now ESTABLISHED) becomes the accepted fd
            let new_listen_rx = tcp::SocketBuffer::new(vec![0u8; 65536]);
            let new_listen_tx = tcp::SocketBuffer::new(vec![0u8; 65536]);
            let mut new_listen_sock = tcp::Socket::new(new_listen_rx, new_listen_tx);
            let _ = new_listen_sock.listen(IpListenEndpoint {
                addr: None,
                port: bound_port,
            });
            let new_listen_handle = stack.sockets.add(new_listen_sock);

            drop(net);

            // Swap: the accepted connection keeps listen_handle,
            // but update the listen fd to point to new_listen_handle
            let accepted_file = Arc::new(SocketFile::new(listen_handle, SocketType::Tcp));

            // Update listen fd to new listen socket
            let process = current_process();
            let mut inner = process.inner_exclusive_access();
            // Mark old SocketFile as transferred so Drop doesn't destroy the socket
            if let Some(old_file) = &inner.fd_table[listen_fd] {
                old_file.mark_transferred();
            }
            // Replace the listen socket with the new one
            let new_listen_file = {
                let mut sf = SocketFile::new(new_listen_handle, SocketType::Tcp);
                sf.cloexec = false;
                sf.bound_port = AtomicU16::new(bound_port);
                sf.listening = AtomicBool::new(true);
                Arc::new(sf)
            };
            inner.fd_table[listen_fd] = Some(new_listen_file);

            // Allocate fd for the accepted connection
            let new_fd = match inner.alloc_fd() {
                Some(fd) => fd,
                None => return EMFILE,
            };
            inner.fd_table[new_fd] = Some(accepted_file);
            return new_fd as isize;
        }

        drop(net);
        suspend_current_and_run_next();
        // Check for pending signals -> EINTR so SIGALRM can be delivered
        if has_pending_unmasked_signal(false) {
            return EINTR;
        }
    }
}

/// sys_connect(fd, addr, addrlen) -> 0
pub fn sys_connect(fd: usize, addr: *const u8, addr_len: usize) -> isize {
    let token = current_user_token();
    let remote = match read_sockaddr(addr, addr_len, token) {
        Some(ep) => ep,
        None => return EINVAL,
    };

    let (handle, sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    match sock_type {
        SocketType::Tcp => {
            let local_port = alloc_ephemeral_port();
            let is_loopback = match remote.addr {
                IpAddress::Ipv4(v4) => {
                    let b = v4.as_bytes();
                    b[0] == 127 || (b[0] == 0 && b[1] == 0 && b[2] == 0 && b[3] == 0)
                }
            };
            // Rewrite 0.0.0.0 to 127.0.0.1 (INADDR_ANY means localhost for connect)
            let connect_remote = if remote.addr == IpAddress::v4(0, 0, 0, 0) {
                IpEndpoint::new(IpAddress::v4(127, 0, 0, 1), remote.port)
            } else {
                remote
            };
            {
                poll_net();
                let mut net = NET_STACK.exclusive_access();
                let stack = match net.as_mut() {
                    Some(s) => s,
                    None => return EINVAL,
                };
                let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
                // Use loopback interface context for 127.x.x.x and 0.0.0.0
                let cx = if is_loopback {
                    stack.lo_iface.context()
                } else {
                    stack.iface.context()
                };
                if let Err(e) = socket.connect(cx, connect_remote, local_port) {
                    warn!("[net] TCP connect failed: {:?}", e);
                    return ECONNREFUSED;
                }
            }

            // Block until connected
            loop {
                poll_net();
                let mut net = NET_STACK.exclusive_access();
                let stack = match net.as_mut() {
                    Some(s) => s,
                    None => return EINVAL,
                };
                let socket = stack.sockets.get_mut::<tcp::Socket>(handle);

                match socket.state() {
                    tcp::State::Established => return 0,
                    tcp::State::Closed => return ECONNREFUSED,
                    _ => {}
                }

                drop(net);
                suspend_current_and_run_next();
                if has_pending_unmasked_signal(false) {
                    return EINTR;
                }
            }
        }
        SocketType::Udp => {
            // UDP connect stores the default destination for write()/send()
            let process = current_process();
            let inner = process.inner_exclusive_access();
            if fd < inner.fd_table.len() {
                if let Some(ref file) = inner.fd_table[fd] {
                    file.set_connected_remote(remote);
                }
            }
            // Also set smoltcp-level remote filter so that this connected
            // socket won't steal packets destined for other sockets on the
            // same port (e.g. iperf3 parallel UDP streams).
            {
                let mut net = NET_STACK.exclusive_access();
                if let Some(ref mut ns) = *net {
                    let sock = ns.sockets.get_mut::<smoltcp::socket::udp::Socket>(handle);
                    sock.set_remote_endpoint(Some(remote));
                }
            }
            0
        }
    }
}

/// sys_getsockname(fd, addr, addrlen) -> 0
pub fn sys_getsockname(fd: usize, addr: *mut u8, addr_len: *mut u32) -> isize {
    let token = current_user_token();
    let (handle, sock_type, bound_port, _listening) = match get_socket_extra(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    let mut net = NET_STACK.exclusive_access();
    let stack = match net.as_mut() {
        Some(s) => s,
        None => return EINVAL,
    };

    let ep = match sock_type {
        SocketType::Tcp => {
            let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
            socket.local_endpoint().unwrap_or_else(|| {
                // Not connected yet, return the bound port
                IpEndpoint::new(IpAddress::v4(0, 0, 0, 0), bound_port)
            })
        }
        SocketType::Udp => {
            let socket = stack.sockets.get_mut::<udp::Socket>(handle);
            let listen_ep = socket.endpoint();
            IpEndpoint::new(
                listen_ep.addr.unwrap_or(IpAddress::v4(0, 0, 0, 0)),
                listen_ep.port,
            )
        }
    };
    drop(net);

    write_sockaddr(&ep, addr, addr_len, token);
    0
}

/// sys_getpeername(fd, addr, addrlen) -> 0
pub fn sys_getpeername(fd: usize, addr: *mut u8, addr_len: *mut u32) -> isize {
    let token = current_user_token();
    let (handle, sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    match sock_type {
        SocketType::Tcp => {
            let mut net = NET_STACK.exclusive_access();
            let stack = match net.as_mut() {
                Some(s) => s,
                None => return EINVAL,
            };
            let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
            let ep = match socket.remote_endpoint() {
                Some(ep) => ep,
                None => return ENOTCONN,
            };
            drop(net);
            write_sockaddr(&ep, addr, addr_len, token);
            0
        }
        SocketType::Udp => {
            // Return the connected remote endpoint if set
            let process = current_process();
            let inner = process.inner_exclusive_access();
            if fd < inner.fd_table.len() {
                if let Some(ref file) = inner.fd_table[fd] {
                    if let Some(ep) = file.get_connected_remote() {
                        drop(inner);
                        write_sockaddr(&ep, addr, addr_len, token);
                        return 0;
                    }
                }
            }
            ENOTCONN
        }
    }
}

/// sys_sendto(fd, buf, len, flags, dest_addr, addrlen) -> bytes_sent
pub fn sys_sendto(
    fd: usize,
    buf: *const u8,
    len: usize,
    _flags: usize,
    dest_addr: *const u8,
    addr_len: usize,
) -> isize {
    let token = current_user_token();
    let (handle, sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    // Validate user buffer pointer
    if len > 0 && (buf as usize) >= 0x4000_0000_0000 {
        return EFAULT;
    }

    // Read user data into kernel buffer
    let user_bufs = translated_byte_buffer(token, buf, len);
    let mut data = vec![0u8; len];
    let mut offset = 0;
    for ubuf in user_bufs.iter() {
        let n = ubuf.len().min(len - offset);
        data[offset..offset + n].copy_from_slice(&ubuf[..n]);
        offset += n;
    }

    match sock_type {
        SocketType::Tcp => loop {
            poll_net();
            let mut net = NET_STACK.exclusive_access();
            let stack = match net.as_mut() {
                Some(s) => s,
                None => return EINVAL,
            };
            let socket = stack.sockets.get_mut::<tcp::Socket>(handle);

            if !socket.may_send() {
                use smoltcp::socket::tcp::State;
                let state = socket.state();
                // Closed/non-established socket: EPIPE (not connected)
                // CloseWait/LastAck/etc: return 0 (connection closed after established)
                if matches!(state, State::Closed | State::Listen | State::SynSent | State::SynReceived) {
                    const EPIPE: isize = -32;
                    return EPIPE;
                }
                return 0;
            }
            if socket.can_send() {
                match socket.send_slice(&data) {
                    Ok(n) => {
                        // Flush through loopback so peer can receive immediately
                        let now = super::smoltcp_now();
                        for _ in 0..4 {
                            stack.lo_iface.poll(now, &mut stack.lo_device, &mut stack.sockets);
                        }
                        stack.iface.poll(now, &mut stack.device, &mut stack.sockets);
                        return n as isize;
                    }
                    Err(_) => return ENOTCONN,
                }
            }

            drop(net);
            suspend_current_and_run_next();
            if has_pending_unmasked_signal(false) {
                return EINTR;
            }
        },
        SocketType::Udp => {
            // Validate dest_addr pointer and addr_len
            if (addr_len as isize) < 0 {
                return EINVAL;
            }
            if !dest_addr.is_null() && (dest_addr as usize) >= 0x4000_0000_0000 {
                return EFAULT;
            }
            let dest = match read_sockaddr(dest_addr, addr_len, token) {
                Some(ep) => ep,
                None => return EINVAL,
            };
            // EMSGSIZE: UDP datagram too large
            if len > 65535 {
                return EMSGSIZE;
            }

            let is_loopback = match dest.addr {
                IpAddress::Ipv4(v4) => v4.as_bytes()[0] == 127,
            };

            poll_net();
            let mut net = NET_STACK.exclusive_access();
            let stack = match net.as_mut() {
                Some(s) => s,
                None => return EINVAL,
            };

            // Auto-bind sender to ephemeral port if not yet bound
            {
                let socket = stack.sockets.get_mut::<udp::Socket>(handle);
                if !socket.is_open() {
                    let local_port = alloc_ephemeral_port();
                    if let Err(e) = socket.bind(IpListenEndpoint {
                        addr: None,
                        port: local_port,
                    }) {
                        warn!("[net] UDP auto-bind failed: {:?}", e);
                        return EINVAL;
                    }
                }
            }

            if is_loopback {
                // Loopback: inject directly into target socket's rx buffer
                let sender_port = stack
                    .sockets
                    .get_mut::<udp::Socket>(handle)
                    .endpoint()
                    .port;
                let sender_meta = udp::UdpMetadata {
                    endpoint: IpEndpoint::new(IpAddress::v4(127, 0, 0, 1), sender_port),
                    meta: Default::default(),
                };
                let target_port = dest.port;
                super::loopback_udp_inject(stack, handle, target_port, &data, sender_meta);
                data.len() as isize
            } else {
                // Normal send via smoltcp network stack
                let socket = stack.sockets.get_mut::<udp::Socket>(handle);
                match socket.send_slice(&data, dest) {
                    Ok(()) => data.len() as isize,
                    Err(e) => {
                        warn!("[net] UDP sendto failed: {:?}", e);
                        EINVAL
                    }
                }
            }
        }
    }
}

/// sys_recvfrom(fd, buf, len, flags, src_addr, addrlen) -> bytes_received
pub fn sys_recvfrom(
    fd: usize,
    buf: *mut u8,
    len: usize,
    _flags: usize,
    src_addr: *mut u8,
    addr_len: *mut u32,
) -> isize {
    let token = current_user_token();
    let (handle, sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    match sock_type {
        SocketType::Tcp => loop {
            poll_net();
            let mut net = NET_STACK.exclusive_access();
            let stack = match net.as_mut() {
                Some(s) => s,
                None => return EINVAL,
            };
            let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
            let state = socket.state();

            if socket.can_recv() {
                let mut tmp = vec![0u8; len];
                match socket.recv_slice(&mut tmp) {
                    Ok(n) => {
                        let pid = current_process().getpid();
                        trace!("[net] recvfrom TCP fd={} pid={} got {} bytes state={:?}", fd, pid, n, state);
                        // Write back to user buffer
                        let user_bufs = translated_byte_buffer(token, buf, n);
                        let mut off = 0;
                        for ubuf in user_bufs.iter() {
                            let copy = ubuf.len().min(n - off);
                            let dst = unsafe {
                                core::slice::from_raw_parts_mut(ubuf.as_ptr() as *mut u8, copy)
                            };
                            dst.copy_from_slice(&tmp[off..off + copy]);
                            off += copy;
                        }
                        return n as isize;
                    }
                    Err(_) => return ENOTCONN,
                }
            }

            if !socket.may_recv() {
                let pid = current_process().getpid();
                info!("[net] recvfrom TCP fd={} pid={} EOF state={:?}", fd, pid, state);
                return 0; // EOF
            }

            drop(net);
            suspend_current_and_run_next();
            if has_pending_unmasked_signal(false) {
                return EINTR;
            }
        },
        SocketType::Udp => loop {
            poll_net();
            let mut net = NET_STACK.exclusive_access();
            let stack = match net.as_mut() {
                Some(s) => s,
                None => return EINVAL,
            };
            let socket = stack.sockets.get_mut::<udp::Socket>(handle);

            if socket.can_recv() {
                let mut tmp = vec![0u8; len];
                match socket.recv_slice(&mut tmp) {
                    Ok((n, endpoint)) => {
                        // Write data to user buffer
                        let user_bufs = translated_byte_buffer(token, buf, n);
                        let mut off = 0;
                        for ubuf in user_bufs.iter() {
                            let copy = ubuf.len().min(n - off);
                            let dst = unsafe {
                                core::slice::from_raw_parts_mut(ubuf.as_ptr() as *mut u8, copy)
                            };
                            dst.copy_from_slice(&tmp[off..off + copy]);
                            off += copy;
                        }
                        // Write source address
                        write_sockaddr(&endpoint.endpoint, src_addr, addr_len, token);
                        return n as isize;
                    }
                    Err(_) => return EINVAL,
                }
            }

            drop(net);
            suspend_current_and_run_next();
            if has_pending_unmasked_signal(false) {
                return EINTR;
            }
        },
    }
}

/// sys_setsockopt(fd, level, optname, optval, optlen) -> 0
pub fn sys_setsockopt(
    fd: usize,
    level: usize,
    optname: usize,
    _optval: *const u8,
    _optlen: usize,
) -> isize {
    let (_handle, _sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    // Stub: accept common options silently
    match (level, optname) {
        (SOL_SOCKET, SO_REUSEADDR) => 0,
        (SOL_SOCKET, SO_KEEPALIVE) => 0,
        (SOL_SOCKET, SO_SNDBUF) => 0,
        (SOL_SOCKET, SO_RCVBUF) => 0,
        (SOL_SOCKET, SO_RCVTIMEO) => 0,
        (SOL_SOCKET, SO_SNDTIMEO) => 0,
        (IPPROTO_TCP, TCP_NODELAY) => 0,
        _ => {
            warn!(
                "[net] setsockopt: unsupported level={} optname={}",
                level, optname
            );
            0 // Return success to avoid breaking applications
        }
    }
}

/// sys_getsockopt(fd, level, optname, optval, optlen) -> 0
pub fn sys_getsockopt(
    fd: usize,
    level: usize,
    optname: usize,
    optval: *mut u8,
    optlen: *mut u32,
) -> isize {
    let (_handle, _sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    let token = current_user_token();

    // Helper to write a u32 value to user optval/optlen
    let write_u32 = |val: u32| {
        if !optval.is_null() && !optlen.is_null() {
            let bufs = translated_byte_buffer(token, optval, 4);
            let bytes = val.to_ne_bytes();
            if let Some(buf) = bufs.first() {
                let dst =
                    unsafe { core::slice::from_raw_parts_mut(buf.as_ptr() as *mut u8, 4) };
                dst.copy_from_slice(&bytes);
            }
            let len_ref = translated_refmut(token, optlen);
            *len_ref = 4;
        }
    };

    // Return default values for common options
    match (level, optname) {
        (SOL_SOCKET, SO_ERROR) => { write_u32(0); 0 }
        (SOL_SOCKET, SO_SNDBUF) => { write_u32(65536); 0 }
        (SOL_SOCKET, SO_RCVBUF) => { write_u32(65536); 0 }
        (SOL_SOCKET, SO_REUSEADDR) => { write_u32(1); 0 }
        (SOL_SOCKET, SO_KEEPALIVE) => { write_u32(0); 0 }
        (IPPROTO_TCP, TCP_NODELAY) => { write_u32(0); 0 }
        // TCP_MAXSEG (2): return default MSS for loopback
        (IPPROTO_TCP, 2) => { write_u32(65495); 0 }
        // TCP_INFO (11): not supported, return silently
        (IPPROTO_TCP, 11) => { write_u32(0); 0 }
        _ => {
            warn!(
                "[net] getsockopt: unsupported level={} optname={}",
                level, optname
            );
            0
        }
    }
}

/// sys_shutdown(fd, how) -> 0
pub fn sys_shutdown_socket(fd: usize, how: i32) -> isize {
    let (handle, sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    let mut net = NET_STACK.exclusive_access();
    let stack = match net.as_mut() {
        Some(s) => s,
        None => return EINVAL,
    };

    match sock_type {
        SocketType::Tcp => {
            let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
            let old_state = socket.state();
            socket.close();
            // Flush FIN through loopback immediately
            let now = super::smoltcp_now();
            for _ in 0..8 {
                stack.lo_iface.poll(now, &mut stack.lo_device, &mut stack.sockets);
            }
            stack.iface.poll(now, &mut stack.device, &mut stack.sockets);
            let new_state = stack.sockets.get_mut::<tcp::Socket>(handle).state();
            let pid = current_process().getpid();
            info!("[net] shutdown TCP fd={} pid={} how={} state {:?} -> {:?}", fd, pid, how, old_state, new_state);
            0
        }
        SocketType::Udp => {
            let socket = stack.sockets.get_mut::<udp::Socket>(handle);
            socket.close();
            0
        }
    }
}

/// sys_socketpair - not supported
pub fn sys_socketpair() -> isize {
    warn!("[net] socketpair: not implemented");
    EOPNOTSUPP
}

/// sys_sendmsg - not implemented
pub fn sys_sendmsg() -> isize {
    warn!("[net] sendmsg: not implemented");
    EOPNOTSUPP
}

/// sys_recvmsg - not implemented
pub fn sys_recvmsg() -> isize {
    warn!("[net] recvmsg: not implemented");
    EOPNOTSUPP
}
