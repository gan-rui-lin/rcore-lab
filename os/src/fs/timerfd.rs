//! timerfd: timer-backed file descriptor (timerfd_create/settime/gettime/read).
use super::{File, PollEvents};
use crate::mm::UserBuffer;
use crate::sync::UPIntrFreeCell;
use crate::task::{has_pending_unmasked_signal, suspend_current_and_run_next};
use crate::timer::get_time_us;

/// Sentinel returned from read() when nonblocking and timer hasn't fired yet.
/// sys_read converts this to EAGAIN.
pub const TIMERFD_EAGAIN: usize = usize::MAX - 1;

struct TimerFdInner {
    /// Absolute monotonic time (µs since boot) when timer next fires. 0 = disarmed.
    expiry_us: u64,
    /// Interval for periodic timers (µs). 0 = one-shot.
    interval_us: u64,
    /// Pending expirations accumulated but not yet consumed by read().
    expirations: u64,
}

/// File descriptor backed by a per-process timer (timerfd_create/settime/gettime).
pub struct TimerFdFile {
    /// clockid_t: 0=CLOCK_REALTIME, 1=CLOCK_MONOTONIC (both map to monotonic internally)
    pub clockid: i32,
    /// TFD_NONBLOCK
    pub nonblock: bool,
    /// TFD_CLOEXEC
    pub cloexec: bool,
    inner: UPIntrFreeCell<TimerFdInner>,
}

impl TimerFdFile {
    /// Create a new timerfd with the given clock and flags.
    pub fn new(clockid: i32, nonblock: bool, cloexec: bool) -> Self {
        Self {
            clockid,
            nonblock,
            cloexec,
            inner: unsafe {
                UPIntrFreeCell::new(TimerFdInner {
                    expiry_us: 0,
                    interval_us: 0,
                    expirations: 0,
                })
            },
        }
    }

    /// Arm the timer.  `expiry_us` is the absolute monotonic time when it first fires.
    /// `interval_us` is the repeat period (0 = one-shot).
    pub fn arm(&self, expiry_us: u64, interval_us: u64) {
        let mut inner = self.inner.exclusive_access();
        inner.expiry_us = expiry_us;
        inner.interval_us = interval_us;
        inner.expirations = 0;
    }

    /// Disarm the timer (expiry_us = 0).  Returns the remaining µs until it would have fired.
    pub fn disarm(&self) -> u64 {
        let mut inner = self.inner.exclusive_access();
        let now = get_time_us() as u64;
        let remaining = if inner.expiry_us > now {
            inner.expiry_us - now
        } else {
            0
        };
        inner.expiry_us = 0;
        inner.interval_us = 0;
        inner.expirations = 0;
        remaining
    }

    /// Sample the current timer state: (remaining_us_until_next_fire, interval_us).
    pub fn gettime(&self) -> (u64, u64) {
        let inner = self.inner.exclusive_access();
        let now = get_time_us() as u64;
        let remaining = if inner.expiry_us == 0 {
            0
        } else if inner.expiry_us > now {
            inner.expiry_us - now
        } else {
            0
        };
        (remaining, inner.interval_us)
    }

    /// Check for expirations without blocking.  Returns Some(count) if the timer fired,
    /// None otherwise (includes disarmed).
    fn check_expirations(inner: &mut TimerFdInner) -> Option<u64> {
        if inner.expiry_us == 0 {
            return if inner.expirations > 0 {
                Some(inner.expirations)
            } else {
                None
            };
        }
        let now = get_time_us() as u64;
        if now >= inner.expiry_us {
            // Count how many intervals have elapsed.
            let elapsed = now - inner.expiry_us;
            if inner.interval_us > 0 {
                let extra = elapsed / inner.interval_us;
                inner.expirations += 1 + extra;
                inner.expiry_us += (1 + extra) * inner.interval_us;
            } else {
                // One-shot: disarm.
                inner.expirations += 1;
                inner.expiry_us = 0;
            }
        }
        if inner.expirations > 0 {
            Some(inner.expirations)
        } else {
            None
        }
    }
}

impl File for TimerFdFile {
    fn readable(&self) -> bool {
        // Always nominally readable (actual blocking happens in read()).
        true
    }

    fn writable(&self) -> bool {
        false
    }

    fn poll(&self, events: PollEvents) -> PollEvents {
        let mut ready = PollEvents::empty();
        if events.contains(PollEvents::POLLIN) {
            let mut inner = self.inner.exclusive_access();
            if Self::check_expirations(&mut inner).is_some() {
                ready |= PollEvents::POLLIN;
            }
        }
        ready
    }

    fn read(&self, buf: UserBuffer) -> usize {
        // timerfd read: blocks until timer fires, then writes expiration count as u64.
        if buf.len() < 8 {
            // EINVAL: buffer too small
            return usize::MAX; // use EINTR sentinel, sys_read will return -EINTR
            // (ideally we'd return EINVAL, but that needs a new sentinel)
        }
        loop {
            {
                let mut inner = self.inner.exclusive_access();
                if let Some(exp) = Self::check_expirations(&mut inner) {
                    let count = exp;
                    inner.expirations = 0;
                    drop(inner);
                    // Write count as little-endian u64 to first 8 bytes of user buffer.
                    let bytes = count.to_ne_bytes();
                    let mut written = 0usize;
                    'outer: for slice in buf.buffers.iter() {
                        let dst = unsafe {
                            core::slice::from_raw_parts_mut(slice.as_ptr() as *mut u8, slice.len())
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
            // Timer hasn't fired yet.
            if self.nonblock {
                return TIMERFD_EAGAIN;
            }
            if has_pending_unmasked_signal(true) {
                return usize::MAX; // EINTR
            }
            suspend_current_and_run_next();
        }
    }

    fn write(&self, _buf: UserBuffer) -> usize {
        0 // timerfd is not writable
    }

    fn fd_flags(&self) -> u32 {
        if self.cloexec { 1 } else { 0 }
    }

    fn status_flags(&self) -> u32 {
        if self.nonblock { 0x800 } else { 0 } // O_NONBLOCK
    }

    fn is_timerfd(&self) -> bool { true }

    fn timerfd_arm(&self, expiry_us: u64, interval_us: u64) {
        self.arm(expiry_us, interval_us);
    }

    fn timerfd_disarm(&self) -> u64 {
        self.disarm()
    }

    fn timerfd_gettime(&self) -> Option<(u64, u64)> {
        Some(self.gettime())
    }

    fn timerfd_clockid(&self) -> i32 {
        self.clockid
    }
}
