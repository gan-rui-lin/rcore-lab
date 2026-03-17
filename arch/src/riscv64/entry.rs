//! RISC-V kernel entry and boot-time helpers.
//!
//! This module includes the assembly entry point (`_start`) and provides
//! low-level initialization routines.  The full `rust_main` boot sequence
//! lives in the kernel crate; here we only expose primitives the kernel
//! needs.

use core::arch::{asm, global_asm};
use riscv::register::satp;

use super::mm::{PageTableEntry, PAGE_SIZE_BITS};

global_asm!(include_str!("entry.asm"));

/// High-half direct-map base used by the boot page table.
pub const VIRT_ADDR_START: usize = 0xFFFF_FFC0_0000_0000;

const SATP_MODE_SV39: usize = 8usize << 60;

// PTE flags for a global 1 GiB kernel leaf mapping.
const PTE_V: usize = 1 << 0;
const PTE_R: usize = 1 << 1;
const PTE_W: usize = 1 << 2;
const PTE_X: usize = 1 << 3;
const PTE_G: usize = 1 << 5;
const PTE_A: usize = 1 << 6;
const PTE_D: usize = 1 << 7;
const BOOT_PTE_FLAGS: usize = PTE_V | PTE_R | PTE_W | PTE_X | PTE_G | PTE_A | PTE_D;

#[repr(align(4096))]
struct AlignedBootRoot([PageTableEntry; 512]);

const fn pte_1g(phys_base: usize, flags: usize) -> PageTableEntry {
    PageTableEntry {
        bits: ((phys_base >> PAGE_SIZE_BITS) << 10) | flags,
    }
}

#[link_section = ".data.prepage.entry"]
static BOOT_PAGE_TABLE: AlignedBootRoot = {
    let mut root = [PageTableEntry { bits: 0 }; 512];
    // Low half identity-map only kernel physical window (starts at 0x8000_0000).
    root[2] = pte_1g(0x8000_0000, BOOT_PTE_FLAGS);
    // High half linear-map (VIRT_ADDR_START | PA), rustoswhu-compatible layout.
    root[0x100] = pte_1g(0x0000_0000, BOOT_PTE_FLAGS);
    root[0x101] = pte_1g(0x4000_0000, BOOT_PTE_FLAGS);
    root[0x102] = pte_1g(0x8000_0000, BOOT_PTE_FLAGS);
    AlignedBootRoot(root)
};

/// Return the statically built RISC-V boot kernel page-table token.
pub fn kernel_page_table_token() -> usize {
    let root_pa = BOOT_PAGE_TABLE.0.as_ptr() as usize & !VIRT_ADDR_START;
    SATP_MODE_SV39 | (root_pa >> PAGE_SIZE_BITS)
}

/// Switch CPU to the statically built kernel Sv39 page table.
pub fn switch_to_kernel_page_table() {
    unsafe {
        satp::write(kernel_page_table_token());
        asm!("sfence.vma");
    }
}

/// Clear the BSS segment to zero.
///
/// # Safety
/// Must be called exactly once, before any BSS-resident statics are read.
pub fn clear_bss() {
    extern "C" {
        fn sbss();
        fn ebss();
    }
    unsafe {
        core::slice::from_raw_parts_mut(sbss as usize as *mut u8, ebss as usize - sbss as usize)
            .fill(0);
    }
}
