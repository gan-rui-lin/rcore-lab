//! Console backend for RISC-V via SBI.

/// Write a single character to the SBI console.
pub fn console_putchar(ch: usize) {
    super::sbi::console_putchar(ch);
}

/// Read a single character from the SBI console.
/// Returns `None` if no character is available.
#[allow(dead_code)]
pub fn console_getchar() -> Option<u8> {
    let ch = super::sbi::console_getchar();
    if ch == 0 {
        None
    } else {
        Some(ch as u8)
    }
}

/// SBI console does not require explicit initialization.
#[allow(dead_code)]
pub fn console_init() {}
