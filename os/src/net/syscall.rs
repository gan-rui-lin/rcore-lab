//! Network system call implementations.

use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use alloc::vec;
use core::sync::atomic::{AtomicBool, AtomicU16};
use lazy_static::lazy_static;
use log::warn;
use smoltcp::iface::SocketHandle;
use smoltcp::socket::{tcp, udp};
use smoltcp::wire::{IpAddress, IpEndpoint, IpListenEndpoint, Ipv4Address};
use spin::Mutex;

use crate::fs::{open_file, path_is_dir, OpenFlags};
use crate::mm::translated_refmut;
use crate::syscall::user_mem::{self, UserReadPolicy, UserWritePolicy};
use crate::task::{
    current_process, current_user_token, has_pending_unmasked_signal, suspend_current_and_run_next,
};

use super::socket_file::{SocketFile, SocketType};
use super::unix_socket::{
    unix_registry_get, unix_registry_has, unix_registry_insert, UnixSocketFile,
};
use super::{alloc_ephemeral_port, poll_net, with_net_stack_read, with_net_stack_write};

// Address family
const AF_UNIX: usize = 1;
const AF_INET: usize = 2;

// Socket types
const SOCK_STREAM: usize = 1;
const SOCK_DGRAM: usize = 2;
const SOCK_RAW: usize = 3;
const SOCK_SEQPACKET: usize = 5;
const SOCK_NONBLOCK: usize = 0o4000;
const SOCK_CLOEXEC: usize = 0o2000000;

// Socket options
const SOL_SOCKET: usize = 1;
const SOL_IP: usize = 0;
const IPPROTO_TCP: usize = 6;
const IPPROTO_UDP: usize = 17;
const SO_REUSEADDR: usize = 2;
const SO_ERROR: usize = 4;
const SO_KEEPALIVE: usize = 9;
const SO_OOBINLINE: usize = 10;
const SO_SNDBUF: usize = 7;
const SO_RCVBUF: usize = 8;
const SO_RCVTIMEO: usize = 20;
const SO_SNDTIMEO: usize = 21;
const TCP_NODELAY: usize = 1;
const MCAST_JOIN_GROUP: usize = 42;
const MCAST_LEAVE_GROUP: usize = 45;

// Errno values
const EINTR: isize = -4;
const EBADF: isize = -9;
const EINVAL: isize = -22;
const ENOTSOCK: isize = -88;
const EAFNOSUPPORT: isize = -97;
const EADDRINUSE: isize = -98;
const EADDRNOTAVAIL: isize = -99;
const EISCONN: isize = -106;
const ENOTCONN: isize = -107;
const ECONNREFUSED: isize = -111;
const EFAULT: isize = -14;
const EACCES: isize = -13;
const ENOTDIR: isize = -20;
const EMFILE: isize = -24;
const EMSGSIZE: isize = -90;
const EOPNOTSUPP: isize = -95;
const EPROTONOSUPPORT: isize = -93;
const ENOPROTOOPT: isize = -92;

/// Linux sockaddr_in structure (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct SockAddrIn {
    sin_family: u16,
    sin_port: u16, // big-endian
    sin_addr: u32, // big-endian
    sin_zero: [u8; 8],
}

/// Convert user-space sockaddr to smoltcp IpEndpoint.
fn read_sockaddr(addr_ptr: *const u8, addr_len: usize, token: usize) -> Option<IpEndpoint> {
    let size = core::mem::size_of::<SockAddrIn>();
    if addr_ptr.is_null() || addr_len < size {
        return None;
    }
    let mut raw = [0u8; 16];
    if user_mem::copy_from_user(token, addr_ptr, &mut raw, UserReadPolicy::StrictChecked).is_err() {
        return None;
    }
    let family = u16::from_ne_bytes([raw[0], raw[1]]);
    // Linux compat: some tests pass sin_family = 0 (AF_UNSPEC) for IPv4 bind.
    if family != AF_INET as u16 && family != 0 {
        return None;
    }
    let port = u16::from_be_bytes([raw[2], raw[3]]);
    let ip = Ipv4Address::new(raw[4], raw[5], raw[6], raw[7]);
    Some(IpEndpoint::new(IpAddress::Ipv4(ip), port))
}

/// Read IPv4 sockaddr with errno-precise validation for connect().
fn read_sockaddr_for_connect(
    addr_ptr: *const u8,
    addr_len: usize,
    token: usize,
) -> Result<IpEndpoint, isize> {
    if addr_ptr.is_null() {
        return Err(EFAULT);
    }
    if addr_len < core::mem::size_of::<SockAddrIn>() {
        return Err(EINVAL);
    }
    if (addr_ptr as usize) >= 0x4000_0000_0000 {
        return Err(EFAULT);
    }
    let mut raw = [0u8; 16];
    if user_mem::copy_from_user(token, addr_ptr, &mut raw, UserReadPolicy::StrictChecked).is_err() {
        return Err(EFAULT);
    }
    let family = u16::from_ne_bytes([raw[0], raw[1]]);
    if family != AF_INET as u16 && family != 0 {
        return Err(EAFNOSUPPORT);
    }
    let port = u16::from_be_bytes([raw[2], raw[3]]);
    let ip = Ipv4Address::new(raw[4], raw[5], raw[6], raw[7]);
    Ok(IpEndpoint::new(IpAddress::Ipv4(ip), port))
}

fn read_user_u32(token: usize, ptr: *const u32) -> Result<u32, isize> {
    user_mem::read_from_user(token, ptr, UserReadPolicy::StrictChecked)
}

fn write_user_u32(token: usize, ptr: *mut u32, val: u32) -> Result<(), isize> {
    user_mem::copy_to_user(
        token,
        ptr as *mut u8,
        &val.to_ne_bytes(),
        UserWritePolicy::DemandCowWithForkFallback,
    )
}

fn copy_to_user(token: usize, dst_ptr: *mut u8, src: &[u8]) -> Result<(), isize> {
    user_mem::copy_to_user(
        token,
        dst_ptr,
        src,
        UserWritePolicy::DemandCowWithForkFallback,
    )
}

/// Write smoltcp IpEndpoint back to user-space sockaddr.
fn write_sockaddr(
    ep: &IpEndpoint,
    addr_ptr: *mut u8,
    addrlen_ptr: *mut u32,
    token: usize,
) -> Result<(), isize> {
    // Linux accept()/getsockname()/getpeername() semantics:
    // - Both NULL is OK (caller doesn't want the address)
    // - Only one NULL is EFAULT (inconsistent state)
    if addr_ptr.is_null() && addrlen_ptr.is_null() {
        return Ok(()); // Caller doesn't want address info
    }
    if addr_ptr.is_null() || addrlen_ptr.is_null() {
        return Err(EFAULT); // Only one NULL is an error
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

    let user_len = read_user_u32(token, addrlen_ptr as *const u32)?;
    if (user_len as i32) < 0 {
        return Err(EINVAL);
    }
    let copy_len = core::cmp::min(raw.len(), user_len as usize);
    copy_to_user(token, addr_ptr, &raw[..copy_len])?;
    write_user_u32(token, addrlen_ptr, raw.len() as u32)?;
    Ok(())
}

/// Helper: get socket handle and type from fd.
fn get_socket_info(fd: usize) -> Result<(SocketHandle, SocketType), isize> {
    let process = current_process();
    match process.get_file(fd) {
        Some(file) => {
            if (file.status_flags() & OpenFlags::PATH.bits()) != 0 {
                return Err(EBADF);
            }
            match file.as_socket() {
                Some((handle, sock_type)) => Ok((handle, sock_type)),
                None => Err(ENOTSOCK),
            }
        }
        None => Err(EBADF),
    }
}

/// Helper: get socket file's bound_port and listening state.
fn get_socket_extra(fd: usize) -> Result<(SocketHandle, SocketType, u16, bool), isize> {
    let process = current_process();
    match process.get_file(fd) {
        Some(file) => {
            if (file.status_flags() & OpenFlags::PATH.bits()) != 0 {
                return Err(EBADF);
            }
            match file.as_socket() {
                Some((handle, sock_type)) => {
                    let bound_port = file.bound_port();
                    let listening = file.is_listening();
                    Ok((handle, sock_type, bound_port, listening))
                }
                None => Err(ENOTSOCK),
            }
        }
        None => Err(EBADF),
    }
}

lazy_static! {
    // Track sockets that joined MCAST group via MCAST_JOIN_GROUP.
    // Needed by accept02: accepted socket should not inherit this state.
    static ref MCAST_JOINED: Mutex<BTreeSet<SocketHandle>> = Mutex::new(BTreeSet::new());
}

fn mcast_mark_joined(handle: SocketHandle) {
    MCAST_JOINED.lock().insert(handle);
}

fn mcast_leave_group(handle: SocketHandle) -> bool {
    MCAST_JOINED.lock().remove(&handle)
}

fn mcast_transfer_membership(old_handle: SocketHandle, new_handle: SocketHandle) {
    let mut joined = MCAST_JOINED.lock();
    if joined.remove(&old_handle) {
        joined.insert(new_handle);
    }
}

fn normalize_abs_path(path: &str) -> alloc::string::String {
    let mut parts: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    for comp in path.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(comp),
        }
    }
    if parts.is_empty() {
        alloc::string::String::from("/")
    } else {
        alloc::format!("/{}", parts.join("/"))
    }
}

fn resolve_unix_bind_path(path: &str) -> alloc::string::String {
    if path.starts_with('/') {
        return normalize_abs_path(path);
    }
    let cwd = current_process().cwd();
    if cwd == "/" {
        normalize_abs_path(&alloc::format!("/{}", path))
    } else {
        normalize_abs_path(&alloc::format!("{}/{}", cwd.trim_end_matches('/'), path))
    }
}

// ============================================================
// System call implementations
// ============================================================

/// sys_socket(domain, type, protocol) -> fd
pub fn sys_socket(domain: usize, sock_type: usize, _protocol: usize) -> isize {
    let base_type = sock_type & 0xFF;
    let nonblock = (sock_type & SOCK_NONBLOCK) != 0;
    let cloexec = (sock_type & SOCK_CLOEXEC) != 0;

    // Handle AF_UNIX domain sockets
    if domain == AF_UNIX {
        let unix_type = match base_type {
            SOCK_STREAM | SOCK_DGRAM | SOCK_SEQPACKET => base_type as u8,
            _ => return EINVAL,
        };
        let sock = UnixSocketFile::new(unix_type, nonblock, cloexec);
        let process = current_process();
        let fd = match process.install_file(sock) {
            Some(fd) => fd,
            None => return EMFILE,
        };
        return fd as isize;
    }

    if domain != AF_INET {
        return EAFNOSUPPORT;
    }

    let st = match base_type {
        SOCK_STREAM => SocketType::Tcp,
        SOCK_DGRAM => SocketType::Udp,
        _ => return EINVAL,
    };

    let handle = match with_net_stack_write(|stack| match st {
        SocketType::Tcp => {
            let rx_buf = tcp::SocketBuffer::new(vec![0u8; 65536]);
            let tx_buf = tcp::SocketBuffer::new(vec![0u8; 65536]);
            let socket = tcp::Socket::new(rx_buf, tx_buf);
            stack.sockets.add(socket)
        }
        SocketType::Udp => {
            let rx_buf =
                udp::PacketBuffer::new(vec![udp::PacketMetadata::EMPTY; 32], vec![0u8; 65536]);
            let tx_buf =
                udp::PacketBuffer::new(vec![udp::PacketMetadata::EMPTY; 32], vec![0u8; 65536]);
            let socket = udp::Socket::new(rx_buf, tx_buf);
            stack.sockets.add(socket)
        }
    }) {
        Some(handle) => handle,
        None => return EINVAL,
    };

    let mut socket_file = SocketFile::new(handle, st);
    socket_file.nonblock = nonblock;
    socket_file.cloexec = cloexec;

    let process = current_process();
    let fd = match process.install_file(Arc::new(socket_file)) {
        Some(fd) => fd,
        None => return EMFILE,
    };
    fd as isize
}

/// Convert IpEndpoint to IpListenEndpoint, treating 0.0.0.0 and 127.0.0.1 as wildcard.
fn endpoint_to_listen(ep: &IpEndpoint) -> IpListenEndpoint {
    let addr = match ep.addr {
        IpAddress::Ipv4(v4) => {
            let bytes = v4.as_bytes();
            // 0.0.0.0 = INADDR_ANY, 127.x.x.x = loopback → treat as wildcard
            if (bytes[0] == 0 && bytes[1] == 0 && bytes[2] == 0 && bytes[3] == 0) || bytes[0] == 127
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

/// Read the address family (first 2 bytes) from a user sockaddr.
fn read_sockaddr_family(addr_ptr: *const u8, addr_len: usize, token: usize) -> Option<u16> {
    if addr_ptr.is_null() || addr_len < 2 {
        return None;
    }
    let mut raw = [0u8; 2];
    if user_mem::copy_from_user(token, addr_ptr, &mut raw, UserReadPolicy::StrictChecked).is_err() {
        return None;
    }
    Some(u16::from_ne_bytes(raw))
}

/// Read AF_UNIX sockaddr path from user memory.
/// Returns (path_string, is_abstract).
fn read_unix_sockaddr(
    addr_ptr: *const u8,
    addr_len: usize,
    token: usize,
) -> Option<(alloc::string::String, bool)> {
    if addr_ptr.is_null() || addr_len < 3 {
        return None;
    }
    // sockaddr_un: { sa_family: u16, sun_path: [u8; 108] }
    let max_len = addr_len.min(110);
    let mut raw = vec![0u8; max_len];
    if user_mem::copy_from_user(
        token,
        addr_ptr,
        raw.as_mut_slice(),
        UserReadPolicy::StrictChecked,
    )
    .is_err()
    {
        return None;
    }
    // raw[0..2] = family, raw[2..] = sun_path
    let path_bytes = &raw[2..];
    if path_bytes.is_empty() {
        return Some((alloc::string::String::new(), false));
    }
    if path_bytes[0] == 0 {
        // Abstract socket: name starts after the leading \0
        let name_end = path_bytes
            .iter()
            .position(|&b| b == 0 && path_bytes.iter().position(|&x| x == b).unwrap_or(0) != 0)
            .unwrap_or(path_bytes.len());
        let name = alloc::string::String::from_utf8_lossy(&path_bytes[1..name_end]).into_owned();
        Some((name, true))
    } else {
        // Pathname socket: null-terminated
        let end = path_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(path_bytes.len());
        let path = alloc::string::String::from_utf8_lossy(&path_bytes[..end]).into_owned();
        Some((path, false))
    }
}

/// Write AF_UNIX sockaddr to user memory.
fn write_unix_sockaddr(
    path: &str,
    addr_ptr: *mut u8,
    addrlen_ptr: *mut u32,
    token: usize,
) -> Result<(), isize> {
    if addr_ptr.is_null() || addrlen_ptr.is_null() {
        return Err(EFAULT);
    }
    let mut raw = [0u8; 110];
    raw[0] = AF_UNIX as u8;
    raw[1] = 0;

    let path_bytes = path.as_bytes();
    let copy_len = path_bytes.len().min(108);
    raw[2..2 + copy_len].copy_from_slice(&path_bytes[..copy_len]);

    // For pathname sockets, include trailing '\0' when there is room.
    let mut used_len = 2 + copy_len;
    if copy_len > 0 && path_bytes[0] != 0 && copy_len < 108 {
        raw[2 + copy_len] = 0;
        used_len += 1;
    }

    let user_len = read_user_u32(token, addrlen_ptr as *const u32)?;
    if (user_len as i32) < 0 {
        return Err(EINVAL);
    }
    let copy_to = core::cmp::min(used_len, user_len as usize);
    copy_to_user(token, addr_ptr, &raw[..copy_to])?;
    write_user_u32(token, addrlen_ptr, used_len as u32)?;
    Ok(())
}

/// sys_bind(fd, addr, addrlen) -> 0
pub fn sys_bind(fd: usize, addr: *const u8, addr_len: usize) -> isize {
    let token = current_user_token();

    // Read address family first to dispatch properly
    let family = match read_sockaddr_family(addr, addr_len, token) {
        Some(f) => f,
        None => return EINVAL,
    };

    // Check if fd is valid and get file
    {
        let process = current_process();
        let file = match process.get_file(fd) {
            Some(f) => f,
            None => return EBADF,
        };

        // Handle AF_UNIX sockets
        if file.is_unix_socket() {
            if family != AF_UNIX as u16 {
                return EINVAL; // Can't bind unix socket to non-unix addr
            }
            let (path, is_abstract) = match read_unix_sockaddr(addr, addr_len, token) {
                Some(r) => r,
                None => return EINVAL,
            };
            let registry_key = if is_abstract {
                let mut k = alloc::string::String::from("\0");
                k.push_str(&path);
                k
            } else {
                let resolved = resolve_unix_bind_path(&path);
                if let Some((parent, _)) = resolved.rsplit_once('/') {
                    let parent = if parent.is_empty() { "/" } else { parent };
                    if !path_is_dir(parent) {
                        return ENOTDIR;
                    }
                } else {
                    return ENOTDIR;
                }
                resolved
            };
            if unix_registry_has(&registry_key) {
                return EADDRINUSE;
            }
            if !is_abstract && open_file(registry_key.as_str(), OpenFlags::empty()).is_some() {
                return EADDRINUSE;
            }

            let ret = file.unix_do_bind(registry_key.clone(), is_abstract);
            if ret == 0 {
                if !is_abstract {
                    // Create a filesystem node so unlink() succeeds for pathname sockets.
                    let _ = open_file(registry_key.as_str(), OpenFlags::CREATE | OpenFlags::WRONLY);
                }
                unix_registry_insert(registry_key, file.clone());
            }
            return ret;
        }

        // For INET sockets bound to AF_UNIX addr → EAFNOSUPPORT
        if family == AF_UNIX as u16 {
            return EAFNOSUPPORT;
        }
    }

    // Original INET bind logic
    let ep = match read_sockaddr(addr, addr_len, token) {
        Some(ep) => ep,
        None => return EINVAL,
    };

    let (handle, sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    let listen_ep = endpoint_to_listen(&ep);

    // Non-local explicit bind address should fail with EADDRNOTAVAIL.
    if listen_ep.addr.is_some() {
        let IpAddress::Ipv4(v4) = ep.addr;
        let b = v4.as_bytes();
        let is_local = b == [10, 0, 2, 15] || b[0] == 127 || b == [0, 0, 0, 0];
        if !is_local {
            return EADDRNOTAVAIL;
        }
    }

    // Check privileged port access (ports < 1024 require root)
    if listen_ep.port > 0 && listen_ep.port < 1024 {
        let process = current_process();
        if process.effective_uid() != 0 {
            return EACCES;
        }
    }

    if with_net_stack_read(|_| ()).is_none() {
        return EINVAL;
    }

    match sock_type {
        SocketType::Tcp => {
            // For TCP, store the bound port for later use in listen()
            let port = if listen_ep.port == 0 {
                alloc_ephemeral_port()
            } else {
                listen_ep.port
            };
            // Store via File trait's set_bound_port
            let process = current_process();
            if let Some(file) = process.get_file(fd) {
                file.set_bound_port(port);
            }
            0
        }
        SocketType::Udp => {
            match with_net_stack_write(|stack| {
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
            }) {
                Some(ret) => ret,
                None => EINVAL,
            }
        }
    }
}

/// sys_listen(fd, _backlog) -> 0
pub fn sys_listen(fd: usize, _backlog: usize) -> isize {
    // Handle AF_UNIX sockets first (not tracked via smoltcp).
    {
        let process = current_process();
        if let Some(file) = process.get_file(fd) {
            if file.is_unix_socket() {
                let ret = file.unix_do_listen(_backlog);
                return ret;
            }
        }
    }

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

    let result = match with_net_stack_write(|stack| {
        let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
        let listen_ep = IpListenEndpoint { addr: None, port };
        if let Err(e) = socket.listen(listen_ep) {
            warn!("[net] TCP listen failed: {:?}", e);
            return EINVAL;
        }
        0
    }) {
        Some(ret) => ret,
        None => return EINVAL,
    };
    if result != 0 {
        return result;
    }

    // Update the socket file's state
    let process = current_process();
    if let Some(file) = process.get_file(fd) {
        file.set_bound_port(port);
        file.set_listening(true);
    }
    0
}

/// sys_accept(listen_fd, addr, addrlen, flags) -> new_fd
/// flags: SOCK_CLOEXEC=0o2000000, SOCK_NONBLOCK=0o4000
pub fn sys_accept(listen_fd: usize, addr: *mut u8, addr_len: *mut u32, flags: usize) -> isize {
    let token = current_user_token();

    // Handle AF_UNIX sockets (not tracked via smoltcp).
    {
        let process = current_process();
        if let Some(file) = process.get_file(listen_fd) {
            if file.is_unix_socket() {
                let listen_file = file.clone();
                let state = listen_file.unix_get_state_u8();
                if state != 2 {
                    // Not in listening state
                    return EINVAL;
                }
                // Block until a connection appears in the backlog.
                // sys_connect pushes a new socket into the backlog (for STREAM sockets)
                // or directly sets the peer (for DGRAM), so we poll the backlog.
                loop {
                    if let Some(accepted_sock) = listen_file.unix_do_accept() {
                        // Write the peer addr (empty for anonymous Unix sockets).
                        if !addr.is_null() && !addr_len.is_null() {
                            let _ = write_unix_sockaddr("", addr, addr_len, token);
                        }
                        // Allocate a new fd for the accepted socket.
                        let proc = current_process();
                        let new_fd = match proc.install_file(accepted_sock) {
                            Some(fd) => fd,
                            None => return EMFILE,
                        };
                        return new_fd as isize;
                    }
                    crate::task::suspend_current_and_run_next();
                    // Check if we should give up (e.g., pending signal).
                    if crate::task::has_pending_unmasked_signal(false) {
                        return EINTR;
                    }
                }
            }
        }
    }

    let (listen_handle, sock_type, bound_port, listening) = match get_socket_extra(listen_fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    if sock_type != SocketType::Tcp {
        return EOPNOTSUPP;
    }
    // Linux returns EINVAL for accept() on a TCP socket that has not entered
    // the listening state yet, instead of blocking forever.
    if bound_port == 0 || !listening {
        return EINVAL;
    }

    // The listen socket is already in LISTEN state from sys_listen().
    // smoltcp's model: the listen socket itself transitions to ESTABLISHED when
    // a SYN arrives. After accept, we need to move the connection to a new socket
    // and put the original back to LISTEN.

    // Wait for the listening socket to become active (SYN received → ESTABLISHED)
    loop {
        poll_net();
        enum AcceptDecision {
            Wait,
            Ready { new_listen_handle: SocketHandle },
            Error(isize),
        }

        let decision = match with_net_stack_write(|stack| {
            let socket = stack.sockets.get_mut::<tcp::Socket>(listen_handle);
            if !socket.is_active() {
                return AcceptDecision::Wait;
            }

            // Connection established! Get remote endpoint.
            let remote_ep = socket.remote_endpoint();
            let _local_ep = socket.local_endpoint();

            // Write peer address to user space
            if let Some(ep) = remote_ep {
                if !addr.is_null() {
                    if let Err(e) = write_sockaddr(&ep, addr, addr_len, token) {
                        return AcceptDecision::Error(e);
                    }
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
            // Preserve multicast membership on the listening socket only.
            mcast_transfer_membership(listen_handle, new_listen_handle);
            AcceptDecision::Ready { new_listen_handle }
        }) {
            Some(result) => result,
            None => return EINVAL,
        };

        match decision {
            AcceptDecision::Error(err) => return err,
            AcceptDecision::Ready { new_listen_handle } => {
                // Swap: the accepted connection keeps listen_handle,
                // but update the listen fd to point to new_listen_handle
                let accepted_file = {
                    let mut sf = SocketFile::new(listen_handle, SocketType::Tcp);
                    sf.cloexec = (flags & SOCK_CLOEXEC) != 0;
                    sf.nonblock = (flags & SOCK_NONBLOCK) != 0;
                    Arc::new(sf)
                };

                // Update listen fd to new listen socket
                let process = current_process();
                // Mark old SocketFile as transferred so Drop doesn't destroy the socket
                if let Some(old_file) = process.get_file(listen_fd) {
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
                process.install_file_at(listen_fd, new_listen_file);

                // Allocate fd for the accepted connection
                let new_fd = match process.install_file(accepted_file) {
                    Some(fd) => fd,
                    None => return EMFILE,
                };
                return new_fd as isize;
            }
            AcceptDecision::Wait => {}
        }
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

    // AF_UNIX connect path.
    {
        let process = current_process();
        let Some(file) = process.get_file(fd) else {
            return EBADF;
        };
        if (file.status_flags() & OpenFlags::PATH.bits()) != 0 {
            return EBADF;
        }
        if file.is_unix_socket() {
            let family = match read_sockaddr_family(addr, addr_len, token) {
                Some(f) => f,
                None => return EINVAL,
            };
            if family != AF_UNIX as u16 {
                return EINVAL;
            }
            let (path, is_abstract) = match read_unix_sockaddr(addr, addr_len, token) {
                Some(p) => p,
                None => return EINVAL,
            };
            let registry_key = if is_abstract {
                let mut k = alloc::string::String::from("\0");
                k.push_str(&path);
                k
            } else {
                resolve_unix_bind_path(&path)
            };
            let peer = match unix_registry_get(&registry_key) {
                Some(p) => p,
                None => return ECONNREFUSED,
            };
            let client: Arc<dyn crate::fs::File> = file.clone();
            let sock_type = file.unix_socket_type();
            if sock_type == 1 || sock_type == 5 {
                // SOCK_STREAM / SOCK_SEQPACKET: use backlog mechanism.
                // Server must be in listening state.
                if peer.unix_get_state_u8() != 2 {
                    return ECONNREFUSED;
                }
                // Create a new server-side socket to represent this connection.
                let nonblock = (file.status_flags() & 0o4000u32) != 0;
                let server_side = UnixSocketFile::new(sock_type, nonblock, false);
                let server_side_arc: Arc<dyn crate::fs::File> = server_side;
                // Record connecting process credentials (for SO_PEERCRED on accepted socket).
                let creds = process.credentials_snapshot();
                let (cred_pid, cred_uid, cred_gid) =
                    (process.pid.0 as u32, creds.real_uid, creds.real_gid);
                server_side_arc.unix_set_peer_cred(cred_pid, cred_uid, cred_gid);
                // Link peers: client ↔ server_side
                file.unix_set_peer_dyn(Arc::downgrade(&server_side_arc));
                server_side_arc.unix_set_peer_dyn(Arc::downgrade(&client));
                // Push server_side into the server's backlog so accept() can dequeue it.
                peer.unix_push_backlog(server_side_arc);
            } else {
                // SOCK_DGRAM: direct peer linking (no backlog).
                file.unix_set_peer_dyn(Arc::downgrade(&peer));
                peer.unix_set_peer_dyn(Arc::downgrade(&client));
            }
            return 0;
        }
    }

    let remote = match read_sockaddr_for_connect(addr, addr_len, token) {
        Ok(ep) => ep,
        Err(err) => return err,
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
                let connect_error = match with_net_stack_write(|stack| {
                    // Use loopback context for 127.x.x.x; external context otherwise
                    // (on LoongArch loopback-only mode, always uses loopback)
                    let cx = if is_loopback {
                        stack.lo_iface.context()
                    } else {
                        #[cfg(target_arch = "riscv64")]
                        {
                            stack.iface.context()
                        }
                        #[cfg(not(target_arch = "riscv64"))]
                        {
                            stack.lo_iface.context()
                        }
                    };
                    let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
                    socket.connect(cx, connect_remote, local_port).err()
                }) {
                    Some(err) => err,
                    None => return EINVAL,
                };
                if let Some(err) = connect_error {
                    warn!("[net] TCP connect failed: {:?}", err);
                    return match err {
                        tcp::ConnectError::InvalidState => EISCONN,
                        _ => ECONNREFUSED,
                    };
                }
            }

            // Block until connected, with retry on loopback RST.
            // glibc ld.so is slow; the server may not have called listen() yet.
            let mut retries_left: i32 = if is_loopback { 3 } else { 0 };
            loop {
                poll_net();
                enum ConnectDecision {
                    Established,
                    Closed,
                    Retried,
                    Pending,
                }

                let decision = match with_net_stack_write(|stack| {
                    let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
                    match socket.state() {
                        tcp::State::Established => ConnectDecision::Established,
                        tcp::State::Closed => {
                            if retries_left > 0 {
                                retries_left -= 1;
                                // Re-initiate connect with a new ephemeral port
                                let new_port = alloc_ephemeral_port();
                                let cx = stack.lo_iface.context();
                                let _ = socket.connect(cx, connect_remote, new_port);
                                ConnectDecision::Retried
                            } else {
                                ConnectDecision::Closed
                            }
                        }
                        _ => ConnectDecision::Pending,
                    }
                }) {
                    Some(result) => result,
                    None => return EINVAL,
                };

                match decision {
                    ConnectDecision::Established => return 0,
                    ConnectDecision::Closed => return ECONNREFUSED,
                    ConnectDecision::Retried => {
                        // Yield to let server process run
                        for _ in 0..5 {
                            suspend_current_and_run_next();
                        }
                        continue;
                    }
                    ConnectDecision::Pending => {}
                }

                suspend_current_and_run_next();
                if has_pending_unmasked_signal(false) {
                    return EINTR;
                }
            }
        }
        SocketType::Udp => {
            // UDP connect stores the default destination for write()/send()
            let process = current_process();
            if let Some(file) = process.get_file(fd) {
                file.set_connected_remote(remote);
            }
            // Also set smoltcp-level remote filter so that this connected
            // socket won't steal packets destined for other sockets on the
            // same port (e.g. iperf3 parallel UDP streams).
            {
                let _ = with_net_stack_write(|stack| {
                    let sock = stack.sockets.get_mut::<smoltcp::socket::udp::Socket>(handle);
                    sock.set_remote_endpoint(Some(remote));
                });
            }
            0
        }
    }
}

/// sys_getsockname(fd, addr, addrlen) -> 0
pub fn sys_getsockname(fd: usize, addr: *mut u8, addr_len: *mut u32) -> isize {
    let token = current_user_token();

    // AF_UNIX sockets are stored as generic File objects (not smoltcp sockets).
    {
        let process = current_process();
        let Some(file) = process.get_file(fd) else {
            return EBADF;
        };
        if (file.status_flags() & OpenFlags::PATH.bits()) != 0 {
            return EBADF;
        }
        if file.is_unix_socket() {
            let path = file
                .unix_bound_path()
                .unwrap_or_else(alloc::string::String::new);
            return match write_unix_sockaddr(&path, addr, addr_len, token) {
                Ok(()) => 0,
                Err(e) => e,
            };
        }
    }

    let (handle, sock_type, bound_port, _listening) = match get_socket_extra(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    let ep = match with_net_stack_write(|stack| match sock_type {
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
    }) {
        Some(ep) => ep,
        None => return EINVAL,
    };

    match write_sockaddr(&ep, addr, addr_len, token) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// sys_getpeername(fd, addr, addrlen) -> 0
pub fn sys_getpeername(fd: usize, addr: *mut u8, addr_len: *mut u32) -> isize {
    let token = current_user_token();

    // Handle AF_UNIX sockets (not tracked via smoltcp).
    {
        let process = current_process();
        let Some(file) = process.get_file(fd) else {
            return EBADF;
        };
        if (file.status_flags() & OpenFlags::PATH.bits()) != 0 {
            return EBADF;
        }
        if file.is_unix_socket() {
            let state = file.unix_get_state_u8();
            if state != 3 {
                // Not Connected
                return ENOTCONN;
            }
            // Connected: return the peer's AF_UNIX address (may be empty for anonymous sockets).
            let peer_path = alloc::string::String::new();
            return match write_unix_sockaddr(&peer_path, addr, addr_len, token) {
                Ok(()) => 0,
                Err(e) => e,
            };
        }
    }

    let (handle, sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    match sock_type {
        SocketType::Tcp => {
            let ep = match with_net_stack_write(|stack| {
                let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
                socket.remote_endpoint().ok_or(ENOTCONN)
            }) {
                Some(Ok(ep)) => ep,
                Some(Err(err)) => return err,
                None => return EINVAL,
            };
            match write_sockaddr(&ep, addr, addr_len, token) {
                Ok(()) => 0,
                Err(e) => e,
            }
        }
        SocketType::Udp => {
            // Return the connected remote endpoint if set
            let process = current_process();
            if let Some(file) = process.get_file(fd) {
                if let Some(ep) = file.get_connected_remote() {
                    return match write_sockaddr(&ep, addr, addr_len, token) {
                        Ok(()) => 0,
                        Err(e) => e,
                    };
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

    // Validate user buffer pointer
    if len > 0 && (buf as usize) >= 0x4000_0000_0000 {
        return EFAULT;
    }

    // Read user data into kernel buffer
    let mut data = vec![0u8; len];
    if user_mem::copy_from_user(token, buf, data.as_mut_slice(), UserReadPolicy::DemandPaged)
        .is_err()
    {
        return EFAULT;
    }

    // AF_UNIX sendto path.
    {
        let process = current_process();
        let Some(file) = process.get_file(fd) else {
            return EBADF;
        };
        if (file.status_flags() & OpenFlags::PATH.bits()) != 0 {
            return EBADF;
        }
        if file.is_unix_socket() {
            // If destination is provided, try direct pathname/abstract delivery first.
            if !dest_addr.is_null() {
                if let Some((path, is_abstract)) = read_unix_sockaddr(dest_addr, addr_len, token) {
                    let registry_key = if is_abstract {
                        let mut k = alloc::string::String::from("\0");
                        k.push_str(&path);
                        k
                    } else {
                        resolve_unix_bind_path(&path)
                    };
                    if let Some(target) = unix_registry_get(&registry_key) {
                        return target.unix_push_rx_bytes(&data) as isize;
                    }
                }
            }
            // Fallback to connected peer.
            let n = file.unix_write(&data);
            if n >= 0 {
                return n;
            }
            return ENOTCONN;
        }
    }

    let (handle, sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    match sock_type {
        SocketType::Tcp => loop {
            poll_net();
            let outcome = match with_net_stack_write(|stack| {
                let socket = stack.sockets.get_mut::<tcp::Socket>(handle);

                if !socket.may_send() {
                    use smoltcp::socket::tcp::State;
                    let state = socket.state();
                    // Closed/non-established socket: EPIPE (not connected)
                    // CloseWait/LastAck/etc: return 0 (connection closed after established)
                    if matches!(
                        state,
                        State::Closed | State::Listen | State::SynSent | State::SynReceived
                    ) {
                        const EPIPE: isize = -32;
                        return Some(EPIPE);
                    }
                    return Some(0);
                }
                if socket.can_send() {
                    match socket.send_slice(&data) {
                        Ok(n) => {
                            // Flush through loopback so peer can receive immediately
                            let now = super::smoltcp_now();
                            for _ in 0..4 {
                                stack
                                    .lo_iface
                                    .poll(now, &mut stack.lo_device, &mut stack.sockets);
                            }
                            stack.poll_external(now);
                            return Some(n as isize);
                        }
                        Err(_) => return Some(ENOTCONN),
                    }
                }

                None
            }) {
                Some(result) => result,
                None => return EINVAL,
            };
            if let Some(ret) = outcome {
                return ret;
            }
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
            let result = match with_net_stack_write(|stack| {
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
                    let sender_port = stack.sockets.get_mut::<udp::Socket>(handle).endpoint().port;
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
            }) {
                Some(ret) => ret,
                None => return EINVAL,
            };
            result
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

    // AF_UNIX recvfrom path.
    {
        let process = current_process();
        let Some(file) = process.get_file(fd) else {
            return EBADF;
        };
        if (file.status_flags() & OpenFlags::PATH.bits()) != 0 {
            return EBADF;
        }
        if file.is_unix_socket() {
            loop {
                let mut tmp = vec![0u8; len];
                let n = file.unix_read(&mut tmp);
                if n > 0 {
                    let n = n as usize;
                    if copy_to_user(token, buf, &tmp[..n]).is_err() {
                        return EFAULT;
                    }
                    // Return AF_UNIX family. For bind05 this is enough; sendto falls back to connected peer.
                    if !src_addr.is_null() {
                        if let Err(e) = write_unix_sockaddr("", src_addr, addr_len, token) {
                            return e;
                        }
                    }
                    return n as isize;
                }

                suspend_current_and_run_next();
                if has_pending_unmasked_signal(false) {
                    return EINTR;
                }
            }
        }
    }

    let (handle, sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    match sock_type {
        SocketType::Tcp => loop {
            poll_net();
            let outcome = match with_net_stack_write(|stack| {
                let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
                let state = socket.state();

                if socket.can_recv() {
                    let mut tmp = vec![0u8; len];
                    match socket.recv_slice(&mut tmp) {
                        Ok(n) => {
                            let pid = current_process().getpid();
                            trace!(
                                "[net] recvfrom TCP fd={} pid={} got {} bytes state={:?}",
                                fd,
                                pid,
                                n,
                                state
                            );
                            // Write back to user buffer
                            if copy_to_user(token, buf, &tmp[..n]).is_err() {
                                return Some(EFAULT);
                            }
                            return Some(n as isize);
                        }
                        Err(_) => return Some(ENOTCONN),
                    }
                }

                if !socket.may_recv() {
                    let pid = current_process().getpid();
                    info!(
                        "[net] recvfrom TCP fd={} pid={} EOF state={:?}",
                        fd, pid, state
                    );
                    return Some(0); // EOF
                }

                None
            }) {
                Some(result) => result,
                None => return EINVAL,
            };
            if let Some(ret) = outcome {
                return ret;
            }
            suspend_current_and_run_next();
            if has_pending_unmasked_signal(false) {
                return EINTR;
            }
        },
        SocketType::Udp => loop {
            poll_net();
            let outcome = match with_net_stack_write(|stack| {
                let socket = stack.sockets.get_mut::<udp::Socket>(handle);

                if socket.can_recv() {
                    let mut tmp = vec![0u8; len];
                    match socket.recv_slice(&mut tmp) {
                        Ok((n, endpoint)) => {
                            // Write data to user buffer
                            if copy_to_user(token, buf, &tmp[..n]).is_err() {
                                return Some(EFAULT);
                            }
                            // Write source address
                            if !src_addr.is_null() {
                                if let Err(e) =
                                    write_sockaddr(&endpoint.endpoint, src_addr, addr_len, token)
                                {
                                    return Some(e);
                                }
                            }
                            return Some(n as isize);
                        }
                        Err(_) => return Some(EINVAL),
                    }
                }

                None
            }) {
                Some(result) => result,
                None => return EINVAL,
            };
            if let Some(ret) = outcome {
                return ret;
            }
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
    optval: *const u8,
    optlen: usize,
) -> isize {
    let (handle, _sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    if optlen == 0 {
        return EINVAL;
    }
    if optval.is_null() {
        return EFAULT;
    }
    let token = current_user_token();
    let mut probe = [0u8; 1];
    if user_mem::copy_from_user(token, optval, &mut probe, UserReadPolicy::StrictChecked).is_err() {
        return EFAULT;
    }

    match (level, optname) {
        (SOL_IP, MCAST_JOIN_GROUP) => {
            mcast_mark_joined(handle);
            0
        }
        (SOL_IP, MCAST_LEAVE_GROUP) => {
            if mcast_leave_group(handle) {
                0
            } else {
                EADDRNOTAVAIL
            }
        }
        (SOL_SOCKET, SO_REUSEADDR) => 0,
        (SOL_SOCKET, SO_KEEPALIVE) => 0,
        (SOL_SOCKET, SO_SNDBUF) => 0,
        (SOL_SOCKET, SO_RCVBUF) => 0,
        (SOL_SOCKET, SO_RCVTIMEO) => 0,
        (SOL_SOCKET, SO_SNDTIMEO) => 0,
        (IPPROTO_TCP, TCP_NODELAY) => 0,
        (SOL_SOCKET, SO_OOBINLINE) => ENOPROTOOPT,
        (SOL_IP, _) | (IPPROTO_TCP, _) | (IPPROTO_UDP, _) => ENOPROTOOPT,
        (SOL_SOCKET, _) => 0,
        _ => {
            warn!(
                "[net] setsockopt: unsupported level={} optname={}",
                level, optname
            );
            ENOPROTOOPT
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
    let token = current_user_token();

    // optlen must be a valid non-null pointer.
    if optlen.is_null() {
        return EFAULT;
    }
    // optval must be non-null (we always write at least 4 bytes).
    if optval.is_null() {
        return EFAULT;
    }
    // Read *optlen to validate it. If the value is negative (high bit set), return EINVAL.
    // Must happen before socket type check per Linux semantics.
    let optlen_val = {
        let r = translated_refmut(token, optlen);
        *r
    };
    if (optlen_val as i32) < 0 {
        return EINVAL;
    }

    // Handle AF_UNIX sockets: limited support for SOL_SOCKET options.
    {
        let process = current_process();
        if let Some(file_cloned) = process.get_file(fd) {
            let file = &file_cloned;
            if file.is_unix_socket() {
                let unix_type = file.unix_socket_type() as u32;
                let write_u32 = |val: u32| -> isize {
                    if copy_to_user(token, optval, &val.to_ne_bytes()).is_err() {
                        return EFAULT;
                    }
                    let len_ref = translated_refmut(token, optlen);
                    *len_ref = 4;
                    0
                };
                if level == SOL_SOCKET {
                    const SO_TYPE: usize = 3;
                    const SO_DOMAIN: usize = 39;
                    const SO_PROTOCOL: usize = 38;
                    const SO_ERROR_OPT: usize = 4;
                    const SO_SNDBUF_OPT: usize = 7;
                    const SO_RCVBUF_OPT: usize = 8;
                    const SO_REUSEADDR_OPT: usize = 2;
                    const SO_KEEPALIVE_OPT: usize = 9;
                    const SO_PEERCRED: usize = 17;
                    match optname {
                        SO_TYPE => return write_u32(unix_type),
                        SO_DOMAIN => return write_u32(1), // AF_UNIX = 1
                        SO_PROTOCOL => return write_u32(0),
                        SO_ERROR_OPT | SO_SNDBUF_OPT | SO_RCVBUF_OPT | SO_REUSEADDR_OPT
                        | SO_KEEPALIVE_OPT => {
                            return write_u32(0);
                        }
                        SO_PEERCRED => {
                            // Write struct ucred { pid: u32, uid: u32, gid: u32 }
                            if (optlen_val as usize) < 12 {
                                return EINVAL;
                            }
                            let (p, u, g) = file.unix_get_peer_cred().unwrap_or((0, 0, 0));
                            let ucred_bytes: [u8; 12] = {
                                let mut b = [0u8; 12];
                                b[0..4].copy_from_slice(&p.to_ne_bytes());
                                b[4..8].copy_from_slice(&u.to_ne_bytes());
                                b[8..12].copy_from_slice(&g.to_ne_bytes());
                                b
                            };
                            if copy_to_user(token, optval, &ucred_bytes).is_err() {
                                return EFAULT;
                            }
                            let len_ref = translated_refmut(token, optlen);
                            *len_ref = 12;
                            return 0;
                        }
                        _ => return EOPNOTSUPP,
                    }
                }
                return EOPNOTSUPP;
            }
        }
    }

    let (_handle, _sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    // Helper to write a u32 value to user optval/optlen
    let write_u32 = |val: u32| -> isize {
        if copy_to_user(token, optval, &val.to_ne_bytes()).is_err() {
            return EFAULT;
        }
        let len_ref = translated_refmut(token, optlen);
        *len_ref = 4;
        0
    };

    // Validate level and return appropriate errors for invalid/unsupported options.
    // SOL_SOCKET=1, SOL_IP=IPPROTO_IP=0, IPPROTO_TCP=6, IPPROTO_UDP=17
    match level {
        SOL_SOCKET => {
            // Return default values for common socket options
            match optname {
                SO_ERROR => write_u32(0),
                SO_SNDBUF => write_u32(65536),
                SO_RCVBUF => write_u32(65536),
                SO_REUSEADDR => write_u32(1),
                SO_KEEPALIVE => write_u32(0),
                _ => {
                    warn!(
                        "[net] getsockopt SOL_SOCKET unsupported optname={}",
                        optname
                    );
                    EOPNOTSUPP
                }
            }
        }
        SOL_IP => {
            // IPPROTO_IP = SOL_IP = 0; unknown option names → ENOPROTOOPT
            warn!(
                "[net] getsockopt IPPROTO_IP unsupported optname={}",
                optname
            );
            ENOPROTOOPT
        }
        IPPROTO_TCP => {
            match optname {
                TCP_NODELAY => {
                    write_u32(0);
                    0
                }
                // TCP_MAXSEG (2): return default MSS
                2 => {
                    write_u32(65495);
                    0
                }
                // TCP_INFO (11): not supported, return 0
                11 => {
                    write_u32(0);
                    0
                }
                _ => {
                    warn!(
                        "[net] getsockopt IPPROTO_TCP unsupported optname={}",
                        optname
                    );
                    ENOPROTOOPT
                }
            }
        }
        IPPROTO_UDP => {
            // UDP has very limited getsockopt support; most options return EOPNOTSUPP
            warn!(
                "[net] getsockopt IPPROTO_UDP unsupported optname={}",
                optname
            );
            EOPNOTSUPP
        }
        _ => {
            warn!(
                "[net] getsockopt unsupported level={} optname={}",
                level, optname
            );
            EOPNOTSUPP
        }
    }
}

/// sys_shutdown(fd, how) -> 0
pub fn sys_shutdown_socket(fd: usize, how: i32) -> isize {
    let (handle, sock_type) = match get_socket_info(fd) {
        Ok(info) => info,
        Err(e) => return e,
    };

    match with_net_stack_write(|stack| match sock_type {
        SocketType::Tcp => {
            let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
            let old_state = socket.state();
            socket.close();
            // Flush FIN through loopback immediately
            let now = super::smoltcp_now();
            for _ in 0..8 {
                stack
                    .lo_iface
                    .poll(now, &mut stack.lo_device, &mut stack.sockets);
            }
            stack.poll_external(now);
            let new_state = stack.sockets.get_mut::<tcp::Socket>(handle).state();
            let pid = current_process().getpid();
            info!(
                "[net] shutdown TCP fd={} pid={} how={} state {:?} -> {:?}",
                fd, pid, how, old_state, new_state
            );
            0
        }
        SocketType::Udp => {
            let socket = stack.sockets.get_mut::<udp::Socket>(handle);
            socket.close();
            0
        }
    }) {
        Some(ret) => ret,
        None => EINVAL,
    }
}

/// sys_socketpair(domain, type, protocol, sv)
pub fn sys_socketpair(domain: usize, sock_type: usize, protocol: usize, sv: *mut i32) -> isize {
    let base_type = sock_type & 0xFF;
    let nonblock = (sock_type & SOCK_NONBLOCK) != 0;
    let cloexec = (sock_type & SOCK_CLOEXEC) != 0;

    // LTP expects errno distinctions for AF_INET socketpair attempts.
    if domain == AF_INET {
        return match base_type {
            SOCK_STREAM => {
                if protocol == 0 || protocol == IPPROTO_TCP {
                    EOPNOTSUPP
                } else {
                    EPROTONOSUPPORT
                }
            }
            SOCK_DGRAM => {
                if protocol == 0 || protocol == IPPROTO_UDP {
                    EOPNOTSUPP
                } else {
                    EPROTONOSUPPORT
                }
            }
            SOCK_RAW => EPROTONOSUPPORT,
            _ => EINVAL,
        };
    }

    if domain != AF_UNIX {
        return EAFNOSUPPORT;
    }
    if !matches!(base_type, SOCK_STREAM | SOCK_DGRAM | SOCK_SEQPACKET) {
        return EINVAL;
    }
    if protocol != 0 {
        return EPROTONOSUPPORT;
    }
    if sv.is_null() {
        return EFAULT;
    }

    let token = current_user_token();
    if !user_mem::ensure_user_readable(
        token,
        sv as *const u8,
        core::mem::size_of::<i32>() * 2,
        UserReadPolicy::StrictChecked,
    ) {
        return EFAULT;
    }

    let left = UnixSocketFile::new(base_type as u8, nonblock, cloexec);
    let right = UnixSocketFile::new(base_type as u8, nonblock, cloexec);
    let left_file: Arc<dyn crate::fs::File> = left.clone();
    let right_file: Arc<dyn crate::fs::File> = right.clone();
    left_file.unix_set_peer_dyn(Arc::downgrade(&right_file));
    right_file.unix_set_peer_dyn(Arc::downgrade(&left_file));

    let process = current_process();
    let fd0 = match process.install_file(left_file) {
        Some(fd) => fd,
        None => return EMFILE,
    };
    let fd1 = match process.install_file(right_file) {
        Some(fd) => fd,
        None => {
            process.take_fd(fd0);
            return EMFILE;
        }
    };

    *translated_refmut(token, sv) = fd0 as i32;
    *translated_refmut(token, unsafe { sv.add(1) }) = fd1 as i32;
    0
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
