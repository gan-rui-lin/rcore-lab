//!Stdin & Stdout
use super::PollEvents;
use super::File;
use crate::mm::UserBuffer;
use crate::sbi::console_getchar;
use crate::task::suspend_current_and_run_next;

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
