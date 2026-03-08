/// Console backend for LoongArch64 via UART MMIO.

const VIRT_ADDR_START: usize = 0x9000_0000_0000_0000;
const UART_BASE: usize = 0x1fe0_01e0 | VIRT_ADDR_START;
const UART_THR: usize = UART_BASE + 0x0;
const UART_RBR: usize = UART_BASE + 0x0;
const UART_IER: usize = UART_BASE + 0x1;
const UART_MCR: usize = UART_BASE + 0x4;
const UART_LSR: usize = UART_BASE + 0x5;

pub(crate) fn console_putchar(ch: usize) {
    unsafe {
        while core::ptr::read_volatile(UART_LSR as *const u8) & 0x20 == 0 {
            core::hint::spin_loop();
        }
        core::ptr::write_volatile(UART_THR as *mut u8, ch as u8);
    }
}

#[allow(dead_code)]
pub(crate) fn console_getchar() -> Option<u8> {
    unsafe {
        if core::ptr::read_volatile(UART_LSR as *const u8) & 0x01 != 0 {
            Some(core::ptr::read_volatile(UART_RBR as *const u8))
        } else {
            None
        }
    }
}

#[allow(dead_code)]
pub(crate) fn console_init() {
    unsafe {
        core::ptr::write_volatile(UART_MCR as *mut u8, 0x0b);
        core::ptr::write_volatile(UART_IER as *mut u8, 0x01);
    }
}
