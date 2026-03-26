//!Stdin & Stdout
use super::PollEvents;
use super::File;
use crate::mm::UserBuffer;
/// Read a character from the SBI console, returning the raw usize value.
/// 0 means no character available.
fn console_getchar() -> usize {
    match arch::console_getchar() {
        Some(ch) => ch as usize,
        None => 0,
    }
}
use crate::task::{has_pending_unmasked_signal, suspend_current_and_run_next};

/// /dev/null device: reads return 0 (EOF), writes succeed silently
pub struct DevNull;

/// /dev/zero device: reads return zero bytes, writes succeed silently
pub struct DevZero;

/// /dev/urandom device: reads return pseudo-random bytes, writes succeed silently
pub struct DevUrandom;

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
        assert_eq!(user_buf.len(), 1);
        // busy loop
        let mut c: usize;
        loop {
            c = console_getchar();
            if c == 0 {
                if has_pending_unmasked_signal(true) {
                    return usize::MAX; // EINTR sentinel
                }
                suspend_current_and_run_next();
                continue;
            } else {
                break;
            }
        }
        let ch = c as u8;
        unsafe {
            user_buf.buffers[0].as_mut_ptr().write_volatile(ch);
        }
        1
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
            }
            else {
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
            print!("{}", core::str::from_utf8(*buffer).unwrap());
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
    fn readable(&self) -> bool { true }
    fn writable(&self) -> bool { true }
    fn read(&self, _user_buf: UserBuffer) -> usize { 0 }
    fn write(&self, user_buf: UserBuffer) -> usize { user_buf.len() }
    fn path(&self) -> Option<&str> { Some("/dev/null") }
}

impl File for DevZero {
    fn readable(&self) -> bool { true }
    fn writable(&self) -> bool { true }
    fn read(&self, mut user_buf: UserBuffer) -> usize {
        let mut total = 0;
        for buffer in user_buf.buffers.iter_mut() {
            buffer.fill(0);
            total += buffer.len();
        }
        total
    }
    fn write(&self, user_buf: UserBuffer) -> usize { user_buf.len() }
    fn path(&self) -> Option<&str> { Some("/dev/zero") }
}

impl File for DevUrandom {
    fn readable(&self) -> bool { true }
    fn writable(&self) -> bool { true }
    fn read(&self, mut user_buf: UserBuffer) -> usize {
        use core::sync::atomic::{AtomicU64, Ordering};
        static STATE: AtomicU64 = AtomicU64::new(0xdead_beef_cafe_babe);
        let mut s = STATE.load(Ordering::Relaxed)
            .wrapping_add(crate::timer::get_time_us() as u64);
        let mut total = 0;
        for buffer in user_buf.buffers.iter_mut() {
            for byte in buffer.iter_mut() {
                s ^= s >> 12;
                s ^= s << 25;
                s ^= s >> 27;
                *byte = s.wrapping_mul(0x2545_F491_4F6C_DD1D) as u8;
                total += 1;
            }
        }
        STATE.store(s, Ordering::Relaxed);
        total
    }
    fn write(&self, user_buf: UserBuffer) -> usize { user_buf.len() }
    fn path(&self) -> Option<&str> { Some("/dev/urandom") }
}
