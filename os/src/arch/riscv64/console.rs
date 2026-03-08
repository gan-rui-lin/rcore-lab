/// Console backend for RISC-V via SBI.

pub(crate) fn console_putchar(ch: usize) {
    crate::sbi::console_putchar(ch);
}

#[allow(dead_code)]
pub(crate) fn console_getchar() -> Option<u8> {
    let ch = crate::sbi::console_getchar();
    if ch == 0 { None } else { Some(ch as u8) }
}

#[allow(dead_code)]
pub(crate) fn console_init() {
    // SBI console does not require explicit init.
}
