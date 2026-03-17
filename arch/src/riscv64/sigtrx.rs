//! RISC-V signal-return trampoline stub.

#![allow(missing_docs)]

#[naked]
#[no_mangle]
#[link_section = ".sigtrx.sigreturn"]
unsafe extern "C" fn _sigreturn() -> ! {
    core::arch::asm!(
        "
            li  a7, 139
            ecall
        ",
        options(noreturn)
    )
}

pub fn sigreturn_trampoline_addr() -> usize {
    _sigreturn as usize
}

pub fn sigreturn_trampoline_offset() -> usize {
    sigreturn_trampoline_addr() & (crate::PAGE_SIZE - 1)
}
