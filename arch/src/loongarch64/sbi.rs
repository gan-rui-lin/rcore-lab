//! LoongArch64 stubs for SBI-like helpers.

#![allow(missing_docs)]

const VIRT_ADDR_START: usize = 0x9000_0000_0000_0000;
const UART_BASE: usize = 0x1fe0_01e0 | VIRT_ADDR_START;
const UART_THR: usize = UART_BASE + 0x0;
const UART_LSR: usize = UART_BASE + 0x5;

pub fn set_timer(_timer: usize) {
    // LoongArch build currently does not use SBI timers.
}

pub fn console_putchar(c: usize) {
    unsafe {
        while core::ptr::read_volatile(UART_LSR as *const u8) & 0x20 == 0 {
            core::hint::spin_loop();
        }
        core::ptr::write_volatile(UART_THR as *mut u8, c as u8);
    }
}

pub fn console_getchar() -> usize {
    usize::MAX
}

pub fn shutdown() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
