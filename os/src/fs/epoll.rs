//! Epoll: I/O event notification facility.
use super::{File, PollEvents};
use crate::mm::UserBuffer;
use crate::sync::UPIntrFreeCell;
use alloc::vec::Vec;

/// An entry in the epoll interest list.
#[derive(Clone)]
pub struct EpollEntry {
    /// The monitored file descriptor.
    pub fd: usize,
    /// Requested events (EPOLLIN, EPOLLOUT, etc.).
    pub events: u32,
    /// User-supplied data returned with events.
    pub data: u64,
}

struct EpollInner {
    entries: Vec<EpollEntry>,
}

/// File descriptor for epoll instance.
pub struct EpollFile {
    inner: UPIntrFreeCell<EpollInner>,
    cloexec: bool,
}

impl EpollFile {
    /// Create a new epoll instance.
    pub fn new(cloexec: bool) -> Self {
        Self {
            inner: unsafe {
                UPIntrFreeCell::new(EpollInner {
                    entries: Vec::new(),
                })
            },
            cloexec,
        }
    }

    /// EPOLL_CTL_ADD: register a new fd.
    pub fn ctl_add(&self, fd: usize, events: u32, data: u64) -> bool {
        let mut inner = self.inner.exclusive_access();
        // Check for duplicate.
        if inner.entries.iter().any(|e| e.fd == fd) {
            return false; // EEXIST
        }
        inner.entries.push(EpollEntry { fd, events, data });
        true
    }

    /// EPOLL_CTL_MOD: modify an existing fd registration.
    pub fn ctl_mod(&self, fd: usize, events: u32, data: u64) -> bool {
        let mut inner = self.inner.exclusive_access();
        if let Some(entry) = inner.entries.iter_mut().find(|e| e.fd == fd) {
            entry.events = events;
            entry.data = data;
            true
        } else {
            false // ENOENT
        }
    }

    /// EPOLL_CTL_DEL: remove a fd from the interest list.
    pub fn ctl_del(&self, fd: usize) -> bool {
        let mut inner = self.inner.exclusive_access();
        let before = inner.entries.len();
        inner.entries.retain(|e| e.fd != fd);
        inner.entries.len() < before
    }

    /// Get a snapshot of all registered entries.
    pub fn get_entries(&self) -> Vec<EpollEntry> {
        let inner = self.inner.exclusive_access();
        inner.entries.clone()
    }
}

impl File for EpollFile {
    fn readable(&self) -> bool {
        true
    }

    fn writable(&self) -> bool {
        true
    }

    fn read(&self, _buf: UserBuffer) -> usize {
        0
    }

    fn write(&self, _buf: UserBuffer) -> usize {
        0
    }

    fn poll(&self, _events: PollEvents) -> PollEvents {
        // Epoll fd itself is not typically polled; return empty.
        PollEvents::empty()
    }

    fn fd_flags(&self) -> u32 {
        if self.cloexec {
            1
        } else {
            0
        }
    }

    fn status_flags(&self) -> u32 {
        0
    }
}
