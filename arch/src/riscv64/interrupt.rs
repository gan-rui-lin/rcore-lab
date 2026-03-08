//! RISC-V interrupt helpers.

use riscv::register::sie;
use riscv::register::sstatus;

/// Check whether supervisor-level interrupts are currently enabled.
#[inline]
pub fn interrupts_enabled() -> bool {
    sstatus::read().sie()
}

/// Disable supervisor-level interrupts.
#[inline]
pub fn disable_interrupts() {
    unsafe {
        sstatus::clear_sie();
    }
}

/// Enable supervisor-level interrupts.
#[inline]
pub fn enable_interrupts() {
    unsafe {
        sstatus::set_sie();
    }
}

/// Enable supervisor-level external interrupts (PLIC).
#[inline]
pub fn enable_supervisor_external() {
    unsafe {
        sie::set_sext();
    }
}
