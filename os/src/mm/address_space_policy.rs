//! Architecture-specific address-space policy hooks.
//!
//! `MemorySet` owns VMA semantics. This module owns the small pieces of
//! architecture policy that decide where kernel/trampoline mappings live and
//! which debug checks are meaningful on a given MMU.

use super::memory_set::MapPermission;
use super::{PageTable, PageTableEntry, VirtAddr, VirtPageNum};
use alloc::vec::Vec;

#[cfg(target_arch = "riscv64")]
use super::PhysPageNum;

#[cfg(target_arch = "riscv64")]
extern "C" {
    fn stext();
    fn etext();
    fn srodata();
    fn erodata();
    fn sdata();
    fn edata();
    fn sbss_with_stack();
    fn ebss();
    fn ekernel();
}

/// One identity-mapped kernel/platform range.
pub struct IdenticalMapping {
    /// Human-readable mapping label for boot logs.
    pub label: &'static str,
    /// Start virtual/physical address.
    pub start: usize,
    /// End virtual/physical address.
    pub end: usize,
    /// Mapping permission.
    pub permission: MapPermission,
}

/// One user-visible signal trampoline mapping.
pub struct UserTrampolineMapping {
    /// User virtual base where the trampoline page is mapped.
    pub base: usize,
    /// Page bytes copied into the user mapping.
    pub bytes: &'static [u8],
    /// Mapping permission.
    pub permission: MapPermission,
}

#[cfg(target_arch = "riscv64")]
#[inline]
fn strip_high_alias(addr: usize) -> usize {
    if addr >= arch::VIRT_ADDR_START {
        addr & !arch::VIRT_ADDR_START
    } else {
        addr
    }
}

/// Kernel-space identity mappings required by this architecture.
#[cfg(target_arch = "riscv64")]
pub fn kernel_identical_mappings() -> Vec<IdenticalMapping> {
    let mut mappings = Vec::new();
    mappings.push(IdenticalMapping {
        label: ".text",
        start: strip_high_alias(stext as usize),
        end: strip_high_alias(etext as usize),
        permission: MapPermission::R | MapPermission::X,
    });
    mappings.push(IdenticalMapping {
        label: ".rodata",
        start: strip_high_alias(srodata as usize),
        end: strip_high_alias(erodata as usize),
        permission: MapPermission::R,
    });
    mappings.push(IdenticalMapping {
        label: ".data",
        start: strip_high_alias(sdata as usize),
        end: strip_high_alias(edata as usize),
        permission: MapPermission::R | MapPermission::W,
    });
    mappings.push(IdenticalMapping {
        label: ".bss",
        start: strip_high_alias(sbss_with_stack as usize),
        end: strip_high_alias(ebss as usize),
        permission: MapPermission::R | MapPermission::W,
    });
    mappings.push(IdenticalMapping {
        label: "physical memory",
        start: strip_high_alias(ekernel as usize),
        end: arch::platform_config().memory_end,
        permission: MapPermission::R | MapPermission::W,
    });
    for &(start, len) in arch::platform_config().mmio_regions {
        mappings.push(IdenticalMapping {
            label: "memory-mapped registers",
            start,
            end: start + len,
            permission: MapPermission::R | MapPermission::W,
        });
    }
    mappings
}

/// Kernel-space identity mappings required by this architecture.
#[cfg(not(target_arch = "riscv64"))]
pub fn kernel_identical_mappings() -> Vec<IdenticalMapping> {
    Vec::new()
}

/// Install any architecture-specific root-level kernel mappings.
#[cfg(target_arch = "riscv64")]
pub fn install_kernel_root_mappings(page_table: &mut PageTable) {
    // Keep runtime kernel page table compatible with boot-time high-half
    // direct map by copying root 1GiB leaf entries.
    let dst_root = PhysPageNum(page_table.token() & ((1usize << 44) - 1));
    let src_root = PhysPageNum(arch::kernel_page_table_token() & ((1usize << 44) - 1));
    let dst = dst_root.get_pte_array();
    let src = src_root.get_pte_array();
    for idx in [0x100usize, 0x101, 0x102] {
        if !dst[idx].is_valid() && src[idx].is_valid() {
            dst[idx] = src[idx];
        }
    }
}

/// Install any architecture-specific root-level kernel mappings.
#[cfg(not(target_arch = "riscv64"))]
pub fn install_kernel_root_mappings(_page_table: &mut PageTable) {}

/// Return the user signal trampoline mapping for the current architecture.
#[cfg(target_arch = "riscv64")]
pub fn user_trampoline_mapping(_user_stack_bottom: usize) -> Option<UserTrampolineMapping> {
    let tramp_base = arch::SIG_RETURN_ADDR;
    let tramp_addr = arch::sigtrx::sigreturn_trampoline_addr();
    let tramp_page = tramp_addr & !(arch::PAGE_SIZE - 1);
    let bytes = unsafe { core::slice::from_raw_parts(tramp_page as *const u8, arch::PAGE_SIZE) };
    Some(UserTrampolineMapping {
        base: tramp_base,
        bytes,
        permission: MapPermission::R | MapPermission::X | MapPermission::U,
    })
}

/// Return the user signal trampoline mapping for the current architecture.
#[cfg(target_arch = "loongarch64")]
pub fn user_trampoline_mapping(user_stack_bottom: usize) -> Option<UserTrampolineMapping> {
    let tramp_base = user_stack_bottom.saturating_sub(arch::PAGE_SIZE);
    let tramp_addr = arch::sigtrx::sigreturn_trampoline_addr();
    let tramp_page = tramp_addr & !(arch::PAGE_SIZE - 1);
    info!(
        "[sigtrx_map] tramp_base={:#x} tramp_addr={:#x} tramp_page={:#x} offset={:#x}",
        tramp_base,
        tramp_addr,
        tramp_page,
        tramp_addr & (arch::PAGE_SIZE - 1)
    );
    let bytes = unsafe { core::slice::from_raw_parts(tramp_page as *const u8, arch::PAGE_SIZE) };
    Some(UserTrampolineMapping {
        base: tramp_base,
        bytes,
        permission: MapPermission::R | MapPermission::W | MapPermission::X | MapPermission::U,
    })
}

/// Return the user signal trampoline mapping for the current architecture.
#[cfg(not(any(target_arch = "riscv64", target_arch = "loongarch64")))]
pub fn user_trampoline_mapping(_user_stack_bottom: usize) -> Option<UserTrampolineMapping> {
    None
}

/// Architecture-specific ELF load segment logging.
#[cfg(target_arch = "loongarch64")]
pub fn log_load_segment(
    vaddr: usize,
    memsz: usize,
    filesz: usize,
    start_va: VirtAddr,
    end_va: VirtAddr,
) {
    info!(
        "[ELF] PH_LOAD: vaddr={:#x} memsz={:#x} filesz={:#x} start={:#x} end={:#x}",
        vaddr, memsz, filesz, start_va.0, end_va.0
    );
}

/// Architecture-specific ELF load segment logging.
#[cfg(not(target_arch = "loongarch64"))]
pub fn log_load_segment(
    _vaddr: usize,
    _memsz: usize,
    _filesz: usize,
    _start_va: VirtAddr,
    _end_va: VirtAddr,
) {
}

/// Architecture-specific first-page map logging.
#[cfg(target_arch = "loongarch64")]
pub fn on_map_one_first_page(page_table: &PageTable, vpn: VirtPageNum) {
    if let Some(pte) = page_table.translate(vpn) {
        info!(
            "[ELF] map_one first vpn: vpn={:#x} pte_bits={:#x}",
            vpn.0, pte.bits
        );
    } else {
        info!("[ELF] map_one first vpn: vpn={:#x} pte=None", vpn.0);
    }
}

/// Architecture-specific first-page map logging.
#[cfg(not(target_arch = "loongarch64"))]
pub fn on_map_one_first_page(_page_table: &PageTable, _vpn: VirtPageNum) {}

/// Architecture-specific first copied ELF page validation.
#[cfg(target_arch = "loongarch64")]
pub fn validate_copy_data_first_page(
    vpn: VirtPageNum,
    pte: PageTableEntry,
    data_len: usize,
    start_va: VirtAddr,
) {
    let pa = pte.ppn().0 * arch::PAGE_SIZE;
    info!(
        "[ELF] copy_data first page: vpn={:#x} pte_bits={:#x} pa={:#x} data_len={:#x} start_va={:#x}",
        vpn.0,
        pte.bits,
        pa,
        data_len,
        start_va.0
    );
    assert!(
        pte.bits != 0,
        "[ELF] copy_data unmapped vpn: vpn={:#x}",
        vpn.0
    );
    assert!(
        pa < arch::platform_config().memory_end,
        "[ELF] copy_data pa out of RAM: pa={:#x} memory_end={:#x}",
        pa,
        arch::platform_config().memory_end
    );
}

/// Architecture-specific first copied ELF page validation.
#[cfg(not(target_arch = "loongarch64"))]
pub fn validate_copy_data_first_page(
    _vpn: VirtPageNum,
    _pte: PageTableEntry,
    _data_len: usize,
    _start_va: VirtAddr,
) {
}

/// Run architecture-specific remap invariants.
#[cfg(target_arch = "riscv64")]
pub fn remap_test(page_table: &PageTable) {
    let mid_text: VirtAddr = ((stext as usize + etext as usize) / 2).into();
    let mid_rodata: VirtAddr = ((srodata as usize + erodata as usize) / 2).into();
    let mid_data: VirtAddr = ((sdata as usize + edata as usize) / 2).into();
    assert!(!page_table.translate(mid_text.floor()).unwrap().writable());
    assert!(!page_table.translate(mid_rodata.floor()).unwrap().writable());
    assert!(!page_table.translate(mid_data.floor()).unwrap().executable());
}

/// Run architecture-specific remap invariants.
#[cfg(not(target_arch = "riscv64"))]
pub fn remap_test(_page_table: &PageTable) {}
