//! epoll instance file descriptor implementation.
//!
//! Provides `EpollFile` which implements the `File` trait and stores a map of
//! watched (fd → EpollEvent).  The busy-poll approach used in `sys_epoll_wait`
//! is intentionally simple but sufficient for LTP tests using pipes / timerfd.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::mm::UserBuffer;
use crate::sync::UPIntrFreeCell;

use super::{EpollEvent, File};

// ── public constants ─────────────────────────────────────────────────────────

/// Close-on-exec flag for `epoll_create1`.
pub const EPOLL_CLOEXEC: i32 = 0x8_0000;

/// epoll_ctl: register a new file descriptor with the epoll instance.
pub const EPOLL_CTL_ADD: i32 = 1;
/// epoll_ctl: remove a file descriptor from the epoll instance.
pub const EPOLL_CTL_DEL: i32 = 2;
/// epoll_ctl: change the event associated with a monitored file descriptor.
pub const EPOLL_CTL_MOD: i32 = 3;

/// Maximum epoll nesting depth allowed by the kernel.
pub const EPOLL_MAX_DEPTH: usize = 5;

// ── EpollFile ─────────────────────────────────────────────────────────────────

/// An epoll instance file descriptor.  Stores the set of monitored (fd, event) pairs.
pub struct EpollFile {
    cloexec: bool,
    /// Nesting depth: 0 = only non-epoll fds registered, N = max chain length below us.
    depth: UPIntrFreeCell<usize>,
    registered: UPIntrFreeCell<BTreeMap<i32, EpollEvent>>,
}

impl EpollFile {
    /// Create a new EpollFile.  `cloexec` sets the `FD_CLOEXEC` flag on the fd.
    pub fn new(cloexec: bool) -> Self {
        Self {
            cloexec,
            depth: unsafe { UPIntrFreeCell::new(0) },
            registered: unsafe { UPIntrFreeCell::new(BTreeMap::new()) },
        }
    }
}

// ── File trait ────────────────────────────────────────────────────────────────

impl File for EpollFile {
    fn readable(&self) -> bool { false }
    fn writable(&self) -> bool { false }
    fn read(&self, _buf: UserBuffer) -> usize { 0 }
    fn write(&self, _buf: UserBuffer) -> usize { 0 }

    fn fd_flags(&self) -> u32 {
        if self.cloexec { 1 } else { 0 }
    }

    fn is_epoll_file(&self) -> bool { true }

    fn epoll_depth(&self) -> usize {
        *self.depth.exclusive_access()
    }

    fn set_epoll_depth(&self, depth: usize) {
        *self.depth.exclusive_access() = depth;
    }

    fn epoll_ctl_inner(&self, op: i32, fd: i32, event: EpollEvent) -> isize {
        let mut reg = self.registered.exclusive_access();
        match op {
            EPOLL_CTL_ADD => {
                if reg.contains_key(&fd) {
                    return -17; // EEXIST
                }
                reg.insert(fd, event);
                0
            }
            EPOLL_CTL_MOD => {
                if !reg.contains_key(&fd) {
                    return -2; // ENOENT
                }
                reg.insert(fd, event);
                0
            }
            EPOLL_CTL_DEL => {
                if reg.remove(&fd).is_none() {
                    return -2; // ENOENT
                }
                0
            }
            _ => -22, // EINVAL
        }
    }

    fn epoll_get_registered(&self) -> Option<Vec<(i32, EpollEvent)>> {
        Some(
            self.registered
                .exclusive_access()
                .iter()
                .map(|(&k, &v)| (k, v))
                .collect(),
        )
    }
}
