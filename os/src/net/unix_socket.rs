//! AF_UNIX (Unix Domain Socket) implementation.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicBool, Ordering};

use lazy_static::lazy_static;
use spin::Mutex;

use crate::fs::{File, PollEvents};
use crate::mm::UserBuffer;

/// Unix socket state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnixState {
    /// Socket is created but not bound yet.
    Unbound = 0,
    /// Socket is bound to an address.
    Bound = 1,
    /// Socket is listening for incoming connections.
    Listening = 2,
    /// Socket has an established peer connection.
    Connected = 3,
}

/// Unix domain socket file.
pub struct UnixSocketFile {
    /// SOCK_STREAM=1, SOCK_DGRAM=2, SOCK_SEQPACKET=5
    pub sock_type: u8,
    /// Whether O_NONBLOCK is enabled on this socket.
    pub nonblock: bool,
    /// Whether FD_CLOEXEC is enabled on this socket.
    pub cloexec: bool,
    /// Current socket state
    pub state: Mutex<UnixState>,
    /// Bound path. For abstract: starts with '\0'. Empty = unbound.
    pub bound_path: Mutex<String>,
    /// Receive buffer (data written by peer)
    pub rx_buf: Mutex<VecDeque<u8>>,
    /// For STREAM listening: pending incoming connections (server-side accepted sockets)
    pub backlog: Mutex<VecDeque<Arc<dyn File>>>,
    /// Peer socket (for connected pair), using dyn File to avoid downcast
    pub peer: Mutex<Option<Weak<dyn File>>>,
    /// Whether the peer has closed its end
    pub peer_closed: AtomicBool,
    /// Peer credentials: (pid, uid, gid) of the connecting process (for SO_PEERCRED)
    pub peer_cred: Mutex<(u32, u32, u32)>,
}

impl UnixSocketFile {
    /// Create a new AF_UNIX socket file.
    pub fn new(sock_type: u8, nonblock: bool, cloexec: bool) -> Arc<Self> {
        Arc::new(Self {
            sock_type,
            nonblock,
            cloexec,
            state: Mutex::new(UnixState::Unbound),
            bound_path: Mutex::new(String::new()),
            rx_buf: Mutex::new(VecDeque::new()),
            backlog: Mutex::new(VecDeque::new()),
            peer: Mutex::new(None),
            peer_closed: AtomicBool::new(false),
            peer_cred: Mutex::new((0, 0, 0)),
        })
    }

    /// Get current socket state.
    pub fn get_state(&self) -> UnixState {
        *self.state.lock()
    }

    /// Set bound path without registry insertion (caller handles registry).
    pub fn set_bound(&self, path: String) -> isize {
        let mut state = self.state.lock();
        if *state != UnixState::Unbound {
            return -22; // EINVAL: already bound
        }
        *self.bound_path.lock() = path;
        *state = UnixState::Bound;
        0
    }
}

// Global registry: path → bound/listening socket (Weak<dyn File>).
// Key = path for pathname sockets, "\0name" for abstract sockets.
// Using Weak so that when all fds to a socket are closed, the address becomes reusable.
lazy_static! {
    static ref UNIX_REGISTRY: Mutex<BTreeMap<String, Weak<dyn File>>> =
        Mutex::new(BTreeMap::new());
}

/// Register a unix socket in the global registry (for listen/connect).
/// Takes a Weak reference so closing all fds automatically frees the address.
pub fn unix_registry_insert(key: String, sock: Arc<dyn File>) {
    UNIX_REGISTRY.lock().insert(key, Arc::downgrade(&sock));
}

/// Remove a unix socket from the registry.
pub fn unix_registry_remove(key: &str) {
    UNIX_REGISTRY.lock().remove(key);
}

/// Check if a path is in the unix registry (path is in use by a live socket).
pub fn unix_registry_has(key: &str) -> bool {
    let mut map = UNIX_REGISTRY.lock();
    match map.get(key) {
        Some(weak) => {
            if weak.upgrade().is_some() {
                true
            } else {
                // Socket was closed; remove stale entry
                map.remove(key);
                false
            }
        }
        None => false,
    }
}

/// Look up a socket by path. Returns None if the socket was closed (Weak expired).
pub fn unix_registry_get(key: &str) -> Option<Arc<dyn File>> {
    let mut map = UNIX_REGISTRY.lock();
    match map.get(key) {
        Some(weak) => match weak.upgrade() {
            Some(arc) => Some(arc),
            None => {
                // Stale entry; remove it
                map.remove(key);
                None
            }
        },
        None => None,
    }
}

impl File for UnixSocketFile {
    fn readable(&self) -> bool {
        !self.rx_buf.lock().is_empty()
            || self.peer_closed.load(Ordering::Relaxed)
            || !self.backlog.lock().is_empty()
    }

    fn writable(&self) -> bool {
        match *self.state.lock() {
            UnixState::Connected => {
                if let Some(peer_weak) = self.peer.lock().as_ref() {
                    peer_weak.upgrade().is_some()
                } else {
                    false
                }
            }
            // DGRAM sockets: always writable if there's a bound peer target
            UnixState::Bound | UnixState::Listening => self.sock_type == 2,
            _ => false,
        }
    }

    fn read(&self, mut user_buf: UserBuffer) -> usize {
        let mut total = 0;
        let mut rx = self.rx_buf.lock();
        for slice in user_buf.buffers.iter_mut() {
            let n = slice.len().min(rx.len());
            if n == 0 {
                break;
            }
            for (i, b) in rx.drain(..n).enumerate() {
                slice[i] = b;
            }
            total += n;
        }
        total
    }

    fn write(&self, user_buf: UserBuffer) -> usize {
        let peer_arc = {
            let guard = self.peer.lock();
            guard.as_ref().and_then(|w| w.upgrade())
        };
        if let Some(peer) = peer_arc {
            let mut total = 0;
            for slice in user_buf.buffers.iter() {
                total += peer.unix_push_rx_bytes(slice);
            }
            total
        } else {
            0
        }
    }

    fn poll(&self, events: PollEvents) -> PollEvents {
        let mut ready = PollEvents::empty();
        if events.contains(PollEvents::POLLIN) && self.readable() {
            ready |= PollEvents::POLLIN;
        }
        if events.contains(PollEvents::POLLOUT) && self.writable() {
            ready |= PollEvents::POLLOUT;
        }
        ready
    }

    fn is_unix_socket(&self) -> bool {
        true
    }

    fn unix_socket_type(&self) -> u8 {
        self.sock_type
    }

    fn unix_bound_path(&self) -> Option<String> {
        let path = self.bound_path.lock();
        if path.is_empty() {
            None
        } else {
            Some(path.clone())
        }
    }

    fn unix_do_listen(&self, _backlog: usize) -> isize {
        if self.sock_type == 2 {
            return -95; // EOPNOTSUPP: SOCK_DGRAM doesn't listen
        }
        let mut state = self.state.lock();
        if *state != UnixState::Bound {
            return -22; // EINVAL: not bound
        }
        *state = UnixState::Listening;
        0
    }

    fn unix_do_accept(&self) -> Option<Arc<dyn File>> {
        if self.get_state() != UnixState::Listening {
            return None;
        }
        self.backlog.lock().pop_front()
    }

    fn unix_readable(&self) -> bool {
        self.readable()
    }

    fn unix_poll(&self, events: PollEvents) -> PollEvents {
        self.poll(events)
    }

    fn unix_push_rx_bytes(&self, data: &[u8]) -> usize {
        let mut rx = self.rx_buf.lock();
        rx.extend(data.iter().copied());
        data.len()
    }

    fn unix_push_backlog(&self, sock: Arc<dyn File>) {
        self.backlog.lock().push_back(sock);
    }

    fn unix_set_peer_dyn(&self, peer: Weak<dyn File>) {
        *self.peer.lock() = Some(peer);
        *self.state.lock() = UnixState::Connected;
    }

    fn unix_mark_peer_closed(&self) {
        self.peer_closed.store(true, Ordering::Relaxed);
        // Clear the peer reference
        *self.peer.lock() = None;
    }

    fn unix_get_state_u8(&self) -> u8 {
        self.get_state() as u8
    }

    fn unix_set_peer_cred(&self, pid: u32, uid: u32, gid: u32) {
        *self.peer_cred.lock() = (pid, uid, gid);
    }

    fn unix_get_peer_cred(&self) -> Option<(u32, u32, u32)> {
        Some(*self.peer_cred.lock())
    }

    fn unix_do_bind(&self, path: alloc::string::String, _is_abstract: bool) -> isize {
        self.set_bound(path)
    }

    fn unix_do_connect(&self, path: alloc::string::String, is_abstract: bool) -> isize {
        let registry_key = if is_abstract {
            let mut k = alloc::string::String::from("\0");
            k.push_str(&path);
            k
        } else {
            path.clone()
        };
        let server = match unix_registry_get(&registry_key) {
            Some(s) => s,
            None => return -111, // ECONNREFUSED
        };
        // Server must be listening (state == 2)
        if server.unix_get_state_u8() != 2 {
            return -111; // ECONNREFUSED
        }
        // Create a new socket representing the server side of this connection
        let server_side = UnixSocketFile::new(self.sock_type, false, false);
        // Link client ↔ server_side as peers
        // client.peer = Weak(server_side), server_side.peer = Weak(client_as_dyn)
        // We need Weak<dyn File> for client - that's done by the caller (syscall layer)
        // which calls unix_set_peer_dyn on the client socket.
        // Here we just set server_side.peer = Weak(self) — but we need Arc<Self>.
        // Instead: push server_side into server's backlog; let accept() return it.
        // The caller must then set client peer = Weak(server_side), server_side peer = Weak(client).
        // Push server_side to server backlog
        let server_side_arc: Arc<dyn File> = server_side;
        server.unix_push_backlog(server_side_arc.clone());
        // Return a special sentinel? No — we return the server_side index via a registry trick.
        // Actually: store server_side in a per-connect temporary slot.
        // Easier: caller (sys_connect) handles peer linking after calling unix_do_connect_raw.
        // This default impl can't do peer linking since it doesn't have Arc<Self>.
        // Return 0 to indicate the backlog push succeeded, syscall layer does the linking.
        let _ = server_side_arc; // moved into backlog
        -200 // sentinel: "backlog pushed, do peer linking at syscall layer"
    }

    fn unix_read(&self, buf: &mut [u8]) -> isize {
        let mut rx = self.rx_buf.lock();
        let n = buf.len().min(rx.len());
        for (i, b) in rx.drain(..n).enumerate() {
            buf[i] = b;
        }
        n as isize
    }

    fn unix_write(&self, buf: &[u8]) -> isize {
        let peer_arc = {
            let guard = self.peer.lock();
            guard.as_ref().and_then(|w| w.upgrade())
        };
        if let Some(peer) = peer_arc {
            peer.unix_push_rx_bytes(buf) as isize
        } else {
            -32 // EPIPE
        }
    }

    fn fd_flags(&self) -> u32 {
        if self.cloexec { 1 } else { 0 }
    }

    fn status_flags(&self) -> u32 {
        if self.nonblock { 0o4000 } else { 0 }
    }
}
