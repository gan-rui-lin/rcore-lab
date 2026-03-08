//! RISC-V timer primitives.
//!
//! Only hardware-level time reading and timer programming live here.
//! Timer condition variables, the timer queue, and `check_timer()` belong
//! in the kernel crate (they depend on `TaskControlBlock`, `wakeup_task`,
//! and the kernel's synchronisation primitives).

use super::board::CLOCK_FREQ;
use riscv::register::time;

const TICKS_PER_SEC: usize = 100;
const MSEC_PER_SEC: usize = 1000;
const MICRO_PER_SEC: usize = 1_000_000;

/// Read the current `time` CSR (cycle counter).
pub fn get_time() -> usize {
    time::read()
}

/// Current wall-clock time in milliseconds (approximate).
pub fn get_time_ms() -> usize {
    time::read() / (CLOCK_FREQ / MSEC_PER_SEC)
}

/// Current wall-clock time in microseconds (approximate).
pub fn get_time_us() -> usize {
    time::read() * MICRO_PER_SEC / CLOCK_FREQ
}

/// Program the next timer interrupt to fire after one tick
/// (~10 ms at 100 Hz).
pub fn set_next_trigger() {
    super::sbi::set_timer(get_time() + CLOCK_FREQ / TICKS_PER_SEC);
}
