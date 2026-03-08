//! Console output for text mode.
use core::fmt::{self, Write};

#[cfg(target_arch = "riscv64")]
use crate::arch::riscv64::console::{
    console_getchar as raw_getchar,
    console_init as arch_console_init,
    console_putchar as raw_putchar,
};

#[cfg(target_arch = "loongarch64")]
use crate::arch::loongarch64::console::{
    console_getchar as raw_getchar,
    console_init as arch_console_init,
    console_putchar as raw_putchar,
};

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

/// Initialize the console device (arch-specific).
pub fn console_init() {
    arch_console_init();
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
