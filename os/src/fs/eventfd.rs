//! EventFd: a simple counter-based file descriptor for event notification.
use super::{File, PollEvents};
use crate::mm::UserBuffer;
use crate::sync::UPIntrFreeCell;
use crate::task::{has_pending_unmasked_signal, suspend_current_and_run_next};

/// Sentinel returned from read()/write() when non-blocking and would block.
pub const EVENTFD_EAGAIN: usize = usize::MAX - 1;

/// EFD_SEMAPHORE: read returns 1 and decrements counter by 1.
pub const EFD_SEMAPHORE: usize = 1;
/// EFD_CLOEXEC: set close-on-exec flag.
pub const EFD_CLOEXEC: usize = 0o2000000;
/// EFD_NONBLOCK: set non-blocking I/O.
pub const EFD_NONBLOCK: usize = 0x800;

struct EventFdInner {
    counter: u64,
}

/// File descriptor backed by an internal u64 counter.
pub struct EventFdFile {
    flags: usize,
    inner: UPIntrFreeCell<EventFdInner>,
}

impl EventFdFile {
    /// Create a new eventfd with the given initial value and flags.
    pub fn new(initval: u64, flags: usize) -> Self {
        Self {
            flags,
            inner: unsafe { UPIntrFreeCell::new(EventFdInner { counter: initval }) },
        }
    }

    #[inline]
    fn is_nonblock(&self) -> bool {
        (self.flags & EFD_NONBLOCK) != 0
    }

    #[inline]
    fn is_semaphore(&self) -> bool {
        (self.flags & EFD_SEMAPHORE) != 0
    }

    #[inline]
    fn is_cloexec(&self) -> bool {
        (self.flags & EFD_CLOEXEC) != 0
    }
}

impl File for EventFdFile {
    fn readable(&self) -> bool {
        true
    }

    fn writable(&self) -> bool {
        true
    }

    fn poll(&self, events: PollEvents) -> PollEvents {
        let inner = self.inner.exclusive_access();
        let mut ready = PollEvents::empty();
        if events.contains(PollEvents::POLLIN) && inner.counter > 0 {
            ready |= PollEvents::POLLIN;
        }
        if events.contains(PollEvents::POLLOUT) && inner.counter < u64::MAX - 1 {
            ready |= PollEvents::POLLOUT;
        }
        ready
    }

    fn read(&self, buf: UserBuffer) -> usize {
        if buf.len() < 8 {
            return usize::MAX; // EINVAL sentinel
        }
        loop {
            {
                let mut inner = self.inner.exclusive_access();
                if inner.counter > 0 {
                    let value = if self.is_semaphore() {
                        inner.counter -= 1;
                        1u64
                    } else {
                        let v = inner.counter;
                        inner.counter = 0;
                        v
                    };
                    drop(inner);
                    // Write value as little-endian u64 to user buffer.
                    let bytes = value.to_ne_bytes();
                    let mut written = 0usize;
                    'outer: for slice in buf.buffers.iter() {
                        let dst = unsafe {
                            core::slice::from_raw_parts_mut(
                                slice.as_ptr() as *mut u8,
                                slice.len(),
                            )
                        };
                        for byte in dst.iter_mut() {
                            if written >= 8 {
                                break 'outer;
                            }
                            *byte = bytes[written];
                            written += 1;
                        }
                        if written >= 8 {
                            break;
                        }
                    }
                    return 8;
                }
            }
            // Counter is zero, would block.
            if self.is_nonblock() {
                return EVENTFD_EAGAIN;
            }
            if has_pending_unmasked_signal(true) {
                return usize::MAX; // EINTR
            }
            suspend_current_and_run_next();
        }
    }

    fn write(&self, buf: UserBuffer) -> usize {
        if buf.len() < 8 {
            return usize::MAX; // EINVAL sentinel
        }
        // Read u64 value from user buffer.
        let mut bytes = [0u8; 8];
        let mut read_count = 0usize;
        'outer: for slice in buf.buffers.iter() {
            for &byte in slice.iter() {
                if read_count >= 8 {
                    break 'outer;
                }
                bytes[read_count] = byte;
                read_count += 1;
            }
            if read_count >= 8 {
                break;
            }
        }
        let value = u64::from_ne_bytes(bytes);
        if value == u64::MAX {
            return usize::MAX; // EINVAL
        }
        loop {
            {
                let mut inner = self.inner.exclusive_access();
                if inner.counter <= u64::MAX - 1 - value {
                    inner.counter += value;
                    return 8;
                }
            }
            // Would overflow, need to block.
            if self.is_nonblock() {
                return EVENTFD_EAGAIN;
            }
            if has_pending_unmasked_signal(true) {
                return usize::MAX; // EINTR
            }
            suspend_current_and_run_next();
        }
    }

    fn fd_flags(&self) -> u32 {
        if self.is_cloexec() {
            1
        } else {
            0
        }
    }

    fn status_flags(&self) -> u32 {
        if self.is_nonblock() {
            0x800
        } else {
            0
        }
    }
}
