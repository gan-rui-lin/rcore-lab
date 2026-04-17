//!Stdin & Stdout
use super::File;
use super::PollEvents;
use crate::mm::UserBuffer;
use core::sync::atomic::{AtomicU64, Ordering};
/// Read a character from the SBI console, returning the raw usize value.
/// 0 means no character available.
fn console_getchar() -> usize {
    match arch::console_getchar() {
        Some(ch) => ch as usize,
        None => 0,
    }
}
use crate::task::{has_pending_unmasked_signal, suspend_current_and_run_next};

static URANDOM_SEED: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);

/// /dev/null device: reads return 0 (EOF), writes succeed silently
pub struct DevNull {
    readable: bool,
    writable: bool,
    path: &'static str,
}

/// /dev/zero device: reads return zero bytes, writes succeed silently
pub struct DevZero {
    readable: bool,
    writable: bool,
    path: &'static str,
}

/// /dev/urandom and /dev/random device: reads return pseudorandom bytes
pub struct DevUrandom {
    readable: bool,
    writable: bool,
    path: &'static str,
}

impl DevNull {
    /// Create a `/dev/null` handle with the requested access mode.
    pub fn new(readable: bool, writable: bool, path: &'static str) -> Self {
        Self {
            readable,
            writable,
            path,
        }
    }
}

impl DevZero {
    /// Create a `/dev/zero` handle with the requested access mode.
    pub fn new(readable: bool, writable: bool, path: &'static str) -> Self {
        Self {
            readable,
            writable,
            path,
        }
    }
}

impl DevUrandom {
    /// Create a `/dev/urandom` handle with the requested access mode.
    pub fn new(readable: bool, writable: bool, path: &'static str) -> Self {
        Self {
            readable,
            writable,
            path,
        }
    }
}

/// stdin file for getting chars from console
pub struct Stdin;

/// stdout file for putting chars to console
pub struct Stdout;

impl File for Stdin {
    fn readable(&self) -> bool {
        true
    }
    fn writable(&self) -> bool {
        false
    }
    fn read(&self, mut user_buf: UserBuffer) -> usize {
        if user_buf.len() == 0 {
            return 0;
        }

        let mut total = 0usize;
        let mut got_first_byte = false;
        for buffer in user_buf.buffers.iter_mut() {
            for byte in buffer.iter_mut() {
                loop {
                    let c = console_getchar();
                    if c == 0 {
                        if !got_first_byte {
                            if has_pending_unmasked_signal(true) {
                                return usize::MAX; // EINTR sentinel
                            }
                            suspend_current_and_run_next();
                            continue;
                        }
                        return total;
                    }
                    *byte = c as u8;
                    total += 1;
                    got_first_byte = true;
                    break;
                }
            }
        }
        total
    }
    fn write(&self, _user_buf: UserBuffer) -> usize {
        panic!("Cannot write to stdin!");
    }

    fn poll(&self, events: PollEvents) -> PollEvents {
        if !events.contains(PollEvents::POLLIN) {
            return PollEvents::empty();
        }
        loop {
            if console_getchar() == usize::MAX {
                // no char available, suspend and run next task
                suspend_current_and_run_next();
            } else {
                // char available, put it back to console buffer and return POLLIN
                // since we have no way to put back the char to console buffer, we just return POLLIN and let the caller read it again.
                break;
            }
        }
        return PollEvents::POLLIN;
    }
}

impl File for Stdout {
    fn readable(&self) -> bool {
        false
    }
    fn writable(&self) -> bool {
        true
    }
    fn read(&self, _user_buf: UserBuffer) -> usize {
        panic!("Cannot read from stdout!");
    }
    fn write(&self, user_buf: UserBuffer) -> usize {
        for buffer in user_buf.buffers.iter() {
            let text = alloc::string::String::from_utf8_lossy(*buffer);
            print!("{}", text);
        }
        user_buf.len()
    }

    fn poll(&self, events: PollEvents) -> PollEvents {
        if events.contains(PollEvents::POLLOUT) {
            return PollEvents::POLLOUT;
        }
        return PollEvents::empty();
    }
}

impl File for DevNull {
    fn readable(&self) -> bool {
        self.readable
    }
    fn writable(&self) -> bool {
        self.writable
    }
    fn read(&self, _user_buf: UserBuffer) -> usize { 0 }
    fn write(&self, user_buf: UserBuffer) -> usize { user_buf.len() }
    fn path(&self) -> Option<&str> {
        Some(self.path)
    }
}

impl File for DevZero {
    fn readable(&self) -> bool {
        self.readable
    }
    fn writable(&self) -> bool {
        self.writable
    }
    fn read(&self, mut user_buf: UserBuffer) -> usize {
        let mut total = 0;
        for buffer in user_buf.buffers.iter_mut() {
            buffer.fill(0);
            total += buffer.len();
        }
        total
    }
    fn write(&self, user_buf: UserBuffer) -> usize { user_buf.len() }
    fn path(&self) -> Option<&str> {
        Some(self.path)
    }
}

impl File for DevUrandom {
    fn readable(&self) -> bool {
        self.readable
    }
    fn writable(&self) -> bool {
        self.writable
    }
    fn read(&self, mut user_buf: UserBuffer) -> usize {
        let mut total = 0usize;
        let mut seed = URANDOM_SEED.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
            ^ (crate::timer::get_time_us() as u64);
        for buffer in user_buf.buffers.iter_mut() {
            for byte in buffer.iter_mut() {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                *byte = (seed >> 56) as u8;
            }
            total += buffer.len();
        }
        URANDOM_SEED.store(seed, Ordering::Relaxed);
        total
    }
    fn write(&self, user_buf: UserBuffer) -> usize {
        user_buf.len()
    }
    fn path(&self) -> Option<&str> {
        Some(self.path)
    }
}
