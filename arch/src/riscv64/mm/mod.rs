//! RISC-V MMU helpers.

pub mod address;
pub mod page_table;

use core::arch::asm;
use riscv::register::satp;

pub use address::{PhysAddr, PhysPageNum, StepByOne, VirtAddr, VirtPageNum, VPNRange};
pub use address::{PAGE_SIZE, PAGE_SIZE_BITS};
pub use page_table::{
    translated_byte_buffer, translated_byte_buffer_checked, translated_ref, translated_refmut,
    translated_str, PageTable, PageTableEntry, PTEFlags, UserBuffer, UserBufferIterator,
};

/// Switch to the page table identified by the given `satp` token and
/// flush the TLB.
pub fn activate_page_table(token: usize) {
    unsafe {
        satp::write(token);
        asm!("sfence.vma");
    }
}

/// On RISC-V, init_kernel_page_table is identical to activate_page_table.
/// On LoongArch64, this also sets PGDH for kernel-space VA[47]=1 addresses.
pub fn init_kernel_page_table(token: usize) {
    activate_page_table(token);
}
