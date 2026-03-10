#![allow(missing_docs)]

use super::consts::VIRT_ADDR_START;
use super::page_table::{PageTable, PageTableEntry, PTEFlags, PhysAddr, PhysPageNum};

#[naked]
#[no_mangle]
#[link_section = ".sigtrx.sigreturn"]
unsafe extern "C" fn _sigreturn() -> ! {
    core::arch::asm!(
        "
            li.d  $a7, 139
            syscall  0
        ",
        options(noreturn)
    )
}

#[link_section = ".data.prepage.trx1"]
static mut TRX_STEP: [[PageTableEntry; PageTable::PTE_NUM_IN_PAGE]; 2] =
    [[PageTableEntry { bits: 0 }; PageTable::PTE_NUM_IN_PAGE]; 2];

pub fn init() {
    unsafe {
        // NOTE: uses .floor() -- consider page-aligning in .ld instead
        let sig_pa = PhysAddr(_sigreturn as usize & !VIRT_ADDR_START);
        TRX_STEP[0][0] = PageTableEntry::new(
            sig_pa.floor(),
            PTEFlags::V | PTEFlags::R | PTEFlags::X | PTEFlags::U,
        );
        let table_pa = PhysAddr(TRX_STEP.as_ptr() as usize & !VIRT_ADDR_START);
        TRX_STEP[1][0] = PageTableEntry::new(table_pa.floor(), PTEFlags::V);
    }
}

pub fn sigreturn_trampoline_addr() -> usize {
    _sigreturn as usize
}

pub fn sigreturn_trampoline_offset() -> usize {
    sigreturn_trampoline_addr() & (PageTable::PAGE_SIZE - 1)
}

pub fn get_trx_mapping() -> usize {
    unsafe { (TRX_STEP.as_ptr() as usize + PageTable::PAGE_SIZE) & !VIRT_ADDR_START }
}
