//! eventfd(2) implementation.
//!
//! An eventfd is a file descriptor that holds a 64-bit unsigned counter.
//! Reads are atomic: they return the counter and reset it to 0 (or decrement
//! by 1 with EFD_SEMAPHORE).  Writes add to the counter.  The fd becomes
//! readable (POLLIN) when the counter > 0, and always writable (POLLOUT)
//! unless the counter would overflow UINT64_MAX-1.

use alloc::sync::Arc;

use crate::fs::{File, PollEvents};
use crate::mm::UserBuffer;
use crate::sync::UPIntrFreeCell;
use crate::task::{has_pending_unmasked_signal, suspend_current_and_run_next};

// ── flags ────────────────────────────────────────────────────────────────────

/// Create fd with `O_NONBLOCK` set.
pub const EFD_NONBLOCK: i32 = 0x800;
/// Create fd with `FD_CLOEXEC` set.
pub const EFD_CLOEXEC: i32 = 0x8_0000;
/// Semaphore semantics: read decrements by 1 instead of clearing.
pub const EFD_SEMAPHORE: i32 = 0x1;

// ── EventFdFile ───────────────────────────────────────────────────────────────

struct Inner {
    counter: u64,
}

/// An eventfd file descriptor.
pub struct EventFdFile {
    nonblock: bool,
    cloexec: bool,
    semaphore: bool,
    inner: UPIntrFreeCell<Inner>,
}

impl EventFdFile {
    /// Create a new EventFdFile with the given initial counter value and flags.
    pub fn new(initval: u64, flags: i32) -> Arc<Self> {
        Arc::new(Self {
            nonblock: (flags & EFD_NONBLOCK) != 0,
            cloexec: (flags & EFD_CLOEXEC) != 0,
            semaphore: (flags & EFD_SEMAPHORE) != 0,
            inner: unsafe { UPIntrFreeCell::new(Inner { counter: initval }) },
        })
    }
}

impl File for EventFdFile {
    fn readable(&self) -> bool { true }
    fn writable(&self) -> bool { true }

    /// Read the counter (8 bytes).
    /// - Normal mode: returns counter, resets to 0.
    /// - Semaphore mode: returns 1, decrements counter by 1.
    /// - Blocks if counter == 0 and EFD_NONBLOCK is not set.
    fn read(&self, mut user_buf: UserBuffer) -> usize {
        // eventfd read requires at least 8 bytes
        if user_buf.len() < 8 {
            return usize::MAX; // EINVAL sentinel (callers check for this)
        }
        loop {
            let mut inner = self.inner.exclusive_access();
            if inner.counter > 0 {
                let val: u64 = if self.semaphore {
                    inner.counter -= 1;
                    1u64
                } else {
                    let v = inner.counter;
                    inner.counter = 0;
                    v
                };
                drop(inner);
                let bytes = val.to_ne_bytes();
                for (dst, &src) in user_buf.buffers[0].iter_mut().zip(bytes.iter()) {
                    *dst = src;
                }
                return 8;
            }
            drop(inner);
            if self.nonblock {
                return usize::MAX; // EAGAIN sentinel
            }
            if has_pending_unmasked_signal(true) {
                return usize::MAX; // EINTR
            }
            suspend_current_and_run_next();
        }
    }

    /// Write to the counter (8 bytes).
    /// Adds the written value to the counter.
    fn write(&self, user_buf: UserBuffer) -> usize {
        if user_buf.len() < 8 {
            return 0; // EINVAL
        }
        let mut bytes = [0u8; 8];
        for (dst, &src) in bytes.iter_mut().zip(user_buf.buffers[0].iter()) {
            *dst = src;
        }
        let add_val = u64::from_ne_bytes(bytes);
        if add_val == u64::MAX {
            return 0; // EINVAL — cannot add UINT64_MAX
        }
        loop {
            let mut inner = self.inner.exclusive_access();
            // Would overflow: UINT64_MAX - 1 is the max allowed counter value
            if inner.counter.checked_add(add_val).map_or(false, |v| v < u64::MAX) {
                inner.counter += add_val;
                return 8;
            }
            drop(inner);
            if self.nonblock {
                return 0; // EAGAIN
            }
            if has_pending_unmasked_signal(true) {
                return 0; // EINTR
            }
            suspend_current_and_run_next();
        }
    }

    fn poll(&self, events: PollEvents) -> PollEvents {
        let inner = self.inner.exclusive_access();
        let mut revents = PollEvents::empty();
        if events.contains(PollEvents::POLLIN) && inner.counter > 0 {
            revents |= PollEvents::POLLIN;
        }
        // Always writable unless counter would overflow (simplified: always report POLLOUT)
        if events.contains(PollEvents::POLLOUT) {
            revents |= PollEvents::POLLOUT;
        }
        revents
    }

    fn fd_flags(&self) -> u32 {
        if self.cloexec { 1 } else { 0 }
    }

    fn status_flags(&self) -> u32 {
        if self.nonblock { 0x800 } else { 0 }
    }
}
