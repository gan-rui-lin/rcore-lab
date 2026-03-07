//! Console output for text mode.
use core::fmt::{self, Write};

#[cfg(target_arch = "riscv64")]
use crate::sbi::console_putchar as raw_putchar;

#[cfg(target_arch = "loongarch64")]
fn raw_putchar(ch: usize) {
    const VIRT_ADDR_START: usize = 0x9000_0000_0000_0000;
    const UART_BASE: usize = 0x1fe0_01e0 | VIRT_ADDR_START;
    const UART_THR: usize = UART_BASE + 0x0;
    const UART_LSR: usize = UART_BASE + 0x5;
    unsafe {
        while core::ptr::read_volatile(UART_LSR as *const u8) & 0x20 == 0 {
            core::hint::spin_loop();
        }
        core::ptr::write_volatile(UART_THR as *mut u8, ch as u8);
    }
}

#[cfg(target_arch = "loongarch64")]
#[allow(dead_code)]
fn raw_getchar() -> Option<u8> {
    const VIRT_ADDR_START: usize = 0x9000_0000_0000_0000;
    const UART_BASE: usize = 0x1fe0_01e0 | VIRT_ADDR_START;
    const UART_RBR: usize = UART_BASE + 0x0;
    const UART_LSR: usize = UART_BASE + 0x5;
    unsafe {
        if core::ptr::read_volatile(UART_LSR as *const u8) & 0x01 != 0 {
            Some(core::ptr::read_volatile(UART_RBR as *const u8))
        } else {
            None
        }
    }
}

#[cfg(target_arch = "riscv64")]
#[allow(dead_code)]
fn raw_getchar() -> Option<u8> {
    None
}

/// Initialize console for LoongArch64 (no-op on RISC-V).
#[allow(dead_code)]
pub fn console_init() {
    #[cfg(target_arch = "loongarch64")]
    unsafe {
        const VIRT_ADDR_START: usize = 0x9000_0000_0000_0000;
        const UART_BASE: usize = 0x1fe0_01e0 | VIRT_ADDR_START;
        const UART_IER: usize = UART_BASE + 0x1;
        const UART_MCR: usize = UART_BASE + 0x4;
        core::ptr::write_volatile(UART_MCR as *mut u8, 0x0b);
        core::ptr::write_volatile(UART_IER as *mut u8, 0x01);
    }
}

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            raw_putchar(c as usize);
        }
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}

/// Read a byte from console if available (LoongArch64 only).
#[allow(dead_code)]
pub fn getchar() -> Option<u8> {
    raw_getchar()
}

/// Print! to the host console using the format string and arguments.
#[macro_export]
macro_rules! print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!($fmt $(, $($arg)+)?))
    }
}

/// Println! to the host console using the format string and arguments.
#[macro_export]
macro_rules! println {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
    }
}
