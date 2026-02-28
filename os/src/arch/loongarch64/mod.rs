//! LoongArch64 architecture support for rcore-lab
//!
//! This module provides LoongArch64-specific implementations of:
//! - Boot sequence with DMW (Direct Memory Windows) initialization
//! - UART console driver
//! - Trap handling (exceptions and interrupts)
//! - Timer support
//! - Architecture constants

mod boot;
pub mod console;
pub mod consts;
pub mod context;
pub mod switch;
pub mod timer;
pub mod trap;

// Re-export important items
pub use console::{console_putchar, console_getchar, console_init};
pub use consts::*;
pub use context::TrapContext;
pub use timer::{get_time, get_time_ms, get_time_us, set_next_trigger};
pub use trap::{init as trap_init, enable_timer_interrupt, trap_return};

/// Shutdown the system (QEMU specific)
pub fn shutdown() -> ! {
    const HALT_ADDR: usize = 0x100E001C | VIRT_ADDR_START;
    unsafe {
        core::ptr::write_volatile(HALT_ADDR as *mut u8, 0x34);
    }
    loop {
        unsafe {
            core::arch::asm!("idle 0");
        }
    }
}
