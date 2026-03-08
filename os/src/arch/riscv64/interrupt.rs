//! RISC-V interrupt helpers.

use riscv::register::sstatus;
use riscv::register::sie;

#[inline]
pub fn interrupts_enabled() -> bool {
    sstatus::read().sie()
}

#[inline]
pub fn disable_interrupts() {
    unsafe {
        sstatus::clear_sie();
    }
}

#[inline]
pub fn enable_interrupts() {
    unsafe {
        sstatus::set_sie();
    }
}

#[inline]
pub fn enable_supervisor_external() {
    unsafe {
        sie::set_sext();
    }
}
