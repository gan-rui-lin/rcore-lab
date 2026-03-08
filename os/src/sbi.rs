//! SBI call wrappers

#![allow(unused)]
#![allow(missing_docs)]

#[cfg(target_arch = "riscv64")]
mod riscv_sbi {
    use core::arch::asm;

    const SBI_SET_TIMER: usize = 0;
    const SBI_CONSOLE_PUTCHAR: usize = 1;
    const SBI_CONSOLE_GETCHAR: usize = 2;
    const SBI_SHUTDOWN: usize = 8;

    /// general sbi call
    #[inline(always)]
    fn sbi_call(which: usize, arg0: usize, arg1: usize, arg2: usize) -> usize {
        let mut ret;
        unsafe {
            asm!(
                "ecall",
                inlateout("x10") arg0 => ret,
                in("x11") arg1,
                in("x12") arg2,
                in("x16") 0,
                in("x17") which,
            );
        }
        ret
    }

    /// use sbi call to set timer
    pub fn set_timer(timer: usize) {
        sbi_call(SBI_SET_TIMER, timer, 0, 0);
    }

    /// use sbi call to putchar in console (qemu uart handler)
    pub fn console_putchar(c: usize) {
        sbi_call(SBI_CONSOLE_PUTCHAR, c, 0, 0);
    }

    /// use sbi call to getchar from console (qemu uart handler)
    pub fn console_getchar() -> usize {
        sbi_call(SBI_CONSOLE_GETCHAR, 0, 0, 0)
    }

    /// use sbi call to shutdown the kernel
    pub fn shutdown() -> ! {
        sbi_call(SBI_SHUTDOWN, 0, 0, 0);
        panic!("It should shutdown!");
    }
}

#[cfg(target_arch = "riscv64")]
pub use riscv_sbi::{console_getchar, console_putchar, set_timer, shutdown};

#[cfg(target_arch = "loongarch64")]
mod loongarch_stub {
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
}

#[cfg(target_arch = "loongarch64")]
pub use loongarch_stub::{console_getchar, console_putchar, set_timer, shutdown};
