//! RISC-V MMU helpers.

pub mod address;
pub mod page_table;

use core::arch::asm;
use riscv::register::satp;

pub use address::{PhysAddr, PhysPageNum, StepByOne, VirtAddr, VirtPageNum, VPNRange};
pub use page_table::{
    translated_byte_buffer, translated_ref, translated_refmut, translated_str, PageTable,
    PageTableEntry, PTEFlags, UserBuffer, UserBufferIterator,
};

/// Change page table by writing satp CSR Register.
pub fn activate_page_table(token: usize) {
    unsafe {
        satp::write(token);
        asm!("sfence.vma");
    }
}
