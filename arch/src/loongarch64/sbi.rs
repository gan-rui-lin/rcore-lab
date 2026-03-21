//! LoongArch64 stubs for SBI-like helpers.

#![allow(missing_docs)]

use crate::consts::{UART_BASE, VIRT_ADDR_START};

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
    
    // 往物理地址 0x100E001C 写入 0x34 来关闭 QEMU 模拟器
    const POWER_OFF_ADDR: usize = 0x100E001C;
    const POWER_OFF_VALUE: u8 = 0x34;
    // warn!("virt_addr_start: {:#x}", VIRT_ADDR_START);
    unsafe {
        core::ptr::write_volatile((POWER_OFF_ADDR | VIRT_ADDR_START) as *mut u8, POWER_OFF_VALUE);
    }
    warn!("Shutting down...");
    loop {
        core::hint::spin_loop();
    }
}
