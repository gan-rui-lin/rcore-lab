//! LoongArch64 timer support
//!
//! Provides timer initialization and time-related functions.

#![allow(dead_code)]

use loongArch64::register::{
    ecfg::{self, LineBasedInterrupt},
    tcfg,
};
use loongArch64::time::{get_timer_freq, Time};

/// Timer frequency (Hz)
/// For QEMU LoongArch, this is typically 100MHz
static mut TIMER_FREQ: usize = 0;

/// Ticks per timer interrupt
/// We aim for 100Hz (10ms per tick) like RISC-V
const TICKS_PER_SEC: usize = 100;

/// Initialize the timer
pub fn init() {
    unsafe {
        TIMER_FREQ = get_timer_freq();

        // Calculate ticks for desired frequency
        // Align to 4-byte boundary
        let ticks = ((TIMER_FREQ / TICKS_PER_SEC) + 3) & !3;

        // Configure timer
        tcfg::set_periodic(true);      // Periodic mode
        tcfg::set_init_val(ticks);     // Set interval
        tcfg::set_en(true);            // Enable timer

        // Enable interrupts
        let interrupts = LineBasedInterrupt::TIMER
            | LineBasedInterrupt::SWI0
            | LineBasedInterrupt::SWI1
            | LineBasedInterrupt::HWI0;
        ecfg::set_lie(interrupts);
    }

    info!("[kernel] LoongArch64 timer initialized at {} Hz", unsafe { TIMER_FREQ });
}

/// Get current time in clock cycles
#[inline]
pub fn get_time() -> usize {
    Time::read()
}

/// Get current time in microseconds
pub fn get_time_us() -> usize {
    let cycles = get_time();
    unsafe {
        if TIMER_FREQ == 0 {
            0
        } else {
            cycles * 1_000_000 / TIMER_FREQ
        }
    }
}

/// Get current time in milliseconds
pub fn get_time_ms() -> usize {
    get_time_us() / 1000
}

/// Set next timer trigger (called after each timer interrupt)
pub fn set_next_trigger() {
    // For LoongArch, the timer automatically reloads in periodic mode
    // So we don't need to manually set the next trigger like RISC-V SBI
    // Just clear the interrupt flag (done in trap handler via ticlr)
}

/// Get timer frequency
pub fn get_freq() -> usize {
    unsafe { TIMER_FREQ }
}
