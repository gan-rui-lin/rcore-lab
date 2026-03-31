//! Implementation of [`MapArea`] and [`MemorySet`].
#[cfg(target_arch = "loongarch64")]
use super::PhysAddr;
use super::{frame_alloc, FrameTracker};
use super::{PTEFlags, PageTable, PageTableEntry};
use super::{PhysPageNum, VirtAddr, VirtPageNum};
use super::{StepByOne, VPNRange};
use crate::config::USER_STACK_TOP;
#[allow(unused_imports)]
use crate::config::{MEMORY_END, MMIO, PAGE_SIZE, USER_STACK_SIZE};
use crate::sync::UPIntrFreeCell;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use lazy_static::*;

extern "C" {
    #[cfg(not(target_arch = "loongarch64"))]
    fn stext();
    #[cfg(not(target_arch = "loongarch64"))]
    fn etext();
    #[cfg(not(target_arch = "loongarch64"))]
    fn srodata();
    #[cfg(not(target_arch = "loongarch64"))]
    fn erodata();
    #[cfg(not(target_arch = "loongarch64"))]
    fn sdata();
    #[cfg(not(target_arch = "loongarch64"))]
    fn edata();
    #[cfg(not(target_arch = "loongarch64"))]
    fn sbss_with_stack();
    #[cfg(not(target_arch = "loongarch64"))]
    fn ebss();
    #[cfg(not(target_arch = "loongarch64"))]
    fn ekernel();
}

lazy_static! {
    /// The kernel's initial memory mapping(kernel address space)
    pub static ref KERNEL_SPACE: Arc<UPIntrFreeCell<MemorySet>> =
        Arc::new(unsafe { UPIntrFreeCell::new(MemorySet::new_kernel()) });
}

/// the kernel token
pub fn kernel_token() -> usize {
    KERNEL_SPACE.exclusive_access().token()
}

/// Flush TLB on the current hart (architecture-specific).
fn flush_tlb() {
    #[cfg(target_arch = "riscv64")]
    unsafe { core::arch::asm!("sfence.vma") }
    #[cfg(target_arch = "loongarch64")]
    unsafe { core::arch::asm!("dbar 0; invtlb 0x00, $r0, $r0") }
}

/// address space
pub struct MemorySet {
    page_table: PageTable,
    areas: Vec<MapArea>,
}

/// Result classification for `mprotect()`-style permission changes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ProtectError {
    /// The requested range is not fully mapped.
    Unmapped,
    /// The underlying mapping does not permit the requested access.
    AccessDenied,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum MapAreaKind {
    Private,
    Shared,
}

impl MemorySet {
    #[cfg(target_arch = "riscv64")]
    #[inline]
    fn rv_strip_high_alias(addr: usize) -> usize {
        if addr >= arch::VIRT_ADDR_START {
            addr & !arch::VIRT_ADDR_START
        } else {
            addr
        }
    }

    /// Create a new empty `MemorySet`.
    pub fn new_bare() -> Self {
        Self {
            page_table: PageTable::new(),
            areas: Vec::new(),
        }
    }
    /// Get the page table token
    pub fn token(&self) -> usize {
        self.page_table.token()
    }
    /// Assume that no conflicts.
    pub fn insert_framed_area(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission,
    ) {
        self.push(
            MapArea::new(start_va, end_va, MapType::Framed, permission),
            None,
        );
    }
    /// Insert a framed user area whose pages stay shared across `fork()`.
    pub fn insert_shared_framed_area(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission,
    ) {
        self.push(
            MapArea::new(start_va, end_va, MapType::Framed, permission).with_shared_frames(),
            None,
        );
    }
    /// Insert a user mmap region with bookkeeping metadata.
    pub fn insert_mmap_area(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission,
        meta: MmapMeta,
    ) {
        let area = if meta.shared {
            MapArea::new_with_kind(
                start_va,
                end_va,
                MapType::Framed,
                permission,
                MapAreaKind::Shared,
            )
            .with_mmap_meta(meta)
            .with_shared_frames()
        } else {
            MapArea::new(start_va, end_va, MapType::Framed, permission).with_mmap_meta(meta)
        };
        if meta.shared {
            self.push(area, None);
        } else {
            self.push(area, None);
        }
    }
    /// Insert a shared framed area (for SysV SHM)
    /// Maps pre-allocated frames to a virtual address range
    pub fn insert_shm_area(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission,
        frames: Vec<Arc<FrameTracker>>,
    ) -> bool {
        let mut map_area = MapArea::new_with_kind(start_va, end_va, MapType::Framed, permission, MapAreaKind::Shared);
        let expected = map_area.vpn_range.get_end().0.saturating_sub(map_area.vpn_range.get_start().0);
        if expected != frames.len() {
            return false;
        }
        for (idx, vpn) in map_area.vpn_range.into_iter().enumerate() {
            let ppn = frames[idx].ppn;
            // Map the frame directly to the page table
            // Convert MapPermission to PTEFlags
            let mut flags = PTEFlags::V;
            if permission.contains(MapPermission::R) {
                flags |= PTEFlags::R;
            }
            if permission.contains(MapPermission::W) {
                flags |= PTEFlags::W;
            }
            if permission.contains(MapPermission::X) {
                flags |= PTEFlags::X;
            }
            if permission.contains(MapPermission::U) {
                flags |= PTEFlags::U;
            }
            self.page_table.map(vpn, ppn, flags);
            map_area.data_frames.insert(vpn, frames[idx].clone());
        }
        self.areas.push(map_area);
        true
    }
    /// remove a area
    pub fn remove_area_with_start_vpn(&mut self, start_vpn: VirtPageNum) {
        if let Some((idx, area)) = self
            .areas
            .iter_mut()
            .enumerate()
            .find(|(_, area)| area.vpn_range.get_start() == start_vpn)
        {
            area.unmap(&mut self.page_table);
            self.areas.remove(idx);
        }
    }
    /// Unmap a user range `[start, end)`, keeping unaffected portions intact.
    pub fn unmap_range(&mut self, start: VirtAddr, end: VirtAddr) {
        let start_vpn = start.floor();
        let end_vpn = end.ceil();
        if start_vpn >= end_vpn {
            return;
        }
        let mut idx = 0usize;
        while idx < self.areas.len() {
            let area_start = self.areas[idx].vpn_range.get_start();
            let area_end = self.areas[idx].vpn_range.get_end();
            if area_end <= start_vpn || area_start >= end_vpn {
                idx += 1;
                continue;
            }
            if area_start >= start_vpn && area_end <= end_vpn {
                // Entire area within unmap range: remove completely
                self.areas[idx].unmap(&mut self.page_table);
                self.areas.remove(idx);
            } else {
                // Partial overlap: unmap overlapping pages and split/shrink the area.
                let overlap_start = area_start.max(start_vpn);
                let overlap_end = area_end.min(end_vpn);
                // Unmap overlapping VPNs
                let mut vpn = overlap_start;
                while vpn < overlap_end {
                    if self.areas[idx].data_frames.contains_key(&vpn) {
                        self.areas[idx].unmap_one(&mut self.page_table, vpn);
                    } else if self.page_table.translate(vpn).map_or(false, |pte| pte.is_valid()) {
                        self.page_table.unmap(vpn);
                    }
                    vpn.step();
                }
                // Keep the non-overlapping portion as a valid area.
                // IMPORTANT: also unmap any orphaned PTEs outside the overlap
                // that will no longer be tracked by this area, to prevent ghost
                // PTEs from triggering COW in future MAP_FIXED mmaps.
                if overlap_start == area_start {
                    // Overlap at the start: shrink area to [overlap_end, area_end)
                    self.areas[idx].vpn_range = VPNRange::new(overlap_end, area_end);
                    self.areas[idx].start_va = VirtAddr::from(overlap_end);
                    idx += 1;
                } else if overlap_end == area_end {
                    // Overlap at the end: shrink area to [area_start, overlap_start)
                    self.areas[idx].vpn_range = VPNRange::new(area_start, overlap_start);
                    idx += 1;
                } else {
                    // Overlap in the middle: keep [area_start, overlap_start)
                    // Unmap orphaned tail [overlap_end, area_end) PTEs
                    let mut tail_vpn = overlap_end;
                    while tail_vpn < area_end {
                        if self.areas[idx].data_frames.remove(&tail_vpn).is_some() {
                            // frame dropped, unmap PTE
                            if self.page_table.translate(tail_vpn).map_or(false, |pte| pte.is_valid()) {
                                self.page_table.unmap(tail_vpn);
                            }
                        } else if self.page_table.translate(tail_vpn).map_or(false, |pte| pte.is_valid()) {
                            self.page_table.unmap(tail_vpn);
                        }
                        tail_vpn.step();
                    }
                    self.areas[idx].vpn_range = VPNRange::new(area_start, overlap_start);
                    idx += 1;
                }
            }
        }
    }
    /// Add a new MapArea into this MemorySet.
    /// Assuming that there are no conflicts in the virtual address
    /// space.
    fn push(&mut self, mut map_area: MapArea, data: Option<&[u8]>) {
        map_area.map(&mut self.page_table);
        if let Some(data) = data {
            map_area.copy_data(&mut self.page_table, data);
        }
        self.areas.push(map_area);
    }
    #[allow(dead_code)]
    fn push_shared_from(&mut self, area: &MapArea, src_page_table: &PageTable) {
        let mut new_area = MapArea::from_another(area);
        new_area.map_shared_from(&mut self.page_table, area, src_page_table);
        self.areas.push(new_area);
    }
    /// Mention that trampoline is not collected by areas.
    #[cfg(not(target_arch = "loongarch64"))]
    fn map_identical_area(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        map_perm: MapPermission,
    ) {
        let pte_flags = PTEFlags::from_bits(map_perm.bits).unwrap();
        for vpn in VPNRange::new(start_va.floor(), end_va.ceil()) {
            self.page_table.map(vpn, PhysPageNum(vpn.0), pte_flags);
        }
    }
    #[cfg(target_arch = "riscv64")]
    fn install_riscv_high_linear_root(&mut self) {
        // Keep runtime kernel page table compatible with boot-time high-half
        // direct map by copying root 1GiB leaf entries.
        let dst_root = PhysPageNum(self.page_table.token() & ((1usize << 44) - 1));
        let src_root = PhysPageNum(arch::kernel_page_table_token() & ((1usize << 44) - 1));
        let dst = dst_root.get_pte_array();
        let src = src_root.get_pte_array();
        for idx in [0x100usize, 0x101, 0x102] {
            if !dst[idx].is_valid() && src[idx].is_valid() {
                dst[idx] = src[idx];
            }
        }
    }
    /// Compatibility no-op: user return no longer depends on trampoline mapping.
    fn map_trampoline(&mut self) {}
    /// Without kernel stacks.
    pub fn new_kernel() -> Self {
        let mut memory_set = Self::new_bare();
        // map trampoline
        memory_set.map_trampoline();
        #[cfg(not(target_arch = "loongarch64"))]
        {
            #[cfg(target_arch = "riscv64")]
            let fix = |addr: usize| Self::rv_strip_high_alias(addr);
            #[cfg(not(target_arch = "riscv64"))]
            let fix = |addr: usize| addr;

            // map kernel sections
            info!(".text [{:#x}, {:#x})", stext as usize, etext as usize);
            info!(".rodata [{:#x}, {:#x})", srodata as usize, erodata as usize);
            info!(".data [{:#x}, {:#x})", sdata as usize, edata as usize);
            info!(
                ".bss [{:#x}, {:#x})",
                sbss_with_stack as usize, ebss as usize
            );
            info!("mapping .text section");
            memory_set.map_identical_area(
                fix(stext as usize).into(),
                fix(etext as usize).into(),
                MapPermission::R | MapPermission::X,
            );
            info!("mapping .rodata section");
            memory_set.map_identical_area(
                fix(srodata as usize).into(),
                fix(erodata as usize).into(),
                MapPermission::R,
            );
            info!("mapping .data section");
            memory_set.map_identical_area(
                fix(sdata as usize).into(),
                fix(edata as usize).into(),
                MapPermission::R | MapPermission::W,
            );
            info!("mapping .bss section");
            memory_set.map_identical_area(
                fix(sbss_with_stack as usize).into(),
                fix(ebss as usize).into(),
                MapPermission::R | MapPermission::W,
            );
            info!("mapping physical memory");
            memory_set.map_identical_area(
                fix(ekernel as usize).into(),
                MEMORY_END.into(),
                MapPermission::R | MapPermission::W,
            );
            info!("mapping memory-mapped registers");
            for pair in MMIO {
                memory_set.map_identical_area(
                    (*pair).0.into(),
                    ((*pair).0 + (*pair).1).into(),
                    MapPermission::R | MapPermission::W,
                );
            }
            #[cfg(target_arch = "riscv64")]
            memory_set.install_riscv_high_linear_root();
        }
        memory_set
    }

    fn scan_elf_meta(elf: &xmas_elf::ElfFile, elf_data: &[u8]) -> (bool, usize) {
        let ph_count = elf.header.pt2.ph_count();
        let mut has_interp = false;
        let mut min_load_vaddr = usize::MAX;
        for i in 0..ph_count {
            let ph = elf.program_header(i).unwrap();
            let ph_type = ph.get_type().unwrap();
            if ph_type == xmas_elf::program::Type::Interp {
                has_interp = true;
                let interp_start = ph.offset() as usize;
                let interp_end = interp_start + ph.file_size() as usize;
                if interp_end < elf_data.len() {
                    let interp_bytes = &elf_data[interp_start..interp_end];
                    if let Ok(interp_str) = core::str::from_utf8(interp_bytes) {
                        info!(
                            "[ELF] Found PT_INTERP: {}",
                            interp_str.trim_end_matches('\0')
                        );
                    }
                }
            } else if ph_type == xmas_elf::program::Type::Load {
                let vaddr = ph.virtual_addr() as usize;
                if vaddr < min_load_vaddr {
                    min_load_vaddr = vaddr;
                }
            }
        }
        (has_interp, min_load_vaddr)
    }

    fn map_load_segments(
        memory_set: &mut MemorySet,
        elf: &xmas_elf::ElfFile,
        load_base: usize,
    ) -> VirtPageNum {
        let ph_count = elf.header.pt2.ph_count();
        let mut max_end_vpn = VirtPageNum(0);
        for i in 0..ph_count {
            let ph = elf.program_header(i).unwrap();
            if ph.get_type().unwrap() == xmas_elf::program::Type::Load {
                let start_va = VirtAddr::from(load_base + ph.virtual_addr() as usize);
                let end_va =
                    VirtAddr::from(load_base + (ph.virtual_addr() + ph.mem_size()) as usize);
                #[cfg(target_arch = "loongarch64")]
                info!(
                    "[ELF] PH_LOAD: vaddr={:#x} memsz={:#x} filesz={:#x} start={:#x} end={:#x}",
                    ph.virtual_addr(),
                    ph.mem_size(),
                    ph.file_size(),
                    start_va.0,
                    end_va.0
                );
                let mut map_perm = MapPermission::U;
                let ph_flags = ph.flags();
                if ph_flags.is_read() {
                    map_perm |= MapPermission::R;
                }
                if ph_flags.is_write() {
                    map_perm |= MapPermission::W;
                }
                if ph_flags.is_execute() {
                    map_perm |= MapPermission::X;
                }
                let map_area = MapArea::new(start_va, end_va, MapType::Framed, map_perm);
                max_end_vpn = map_area.vpn_range.get_end();
                let file_data = &elf.input[ph.offset() as usize..(ph.offset() + ph.file_size()) as usize];
                memory_set.push(map_area, Some(file_data));
            }
        }
        max_end_vpn
    }

    fn scan_tls_info(elf: &xmas_elf::ElfFile, load_base: usize) -> Option<crate::task::TlsInfo> {
        let ph_count = elf.header.pt2.ph_count();
        let mut tls_info = None;
        info!("[ELF] Scanning {} program headers for PT_TLS", ph_count);
        for i in 0..ph_count {
            let ph = elf.program_header(i).unwrap();
            let ph_type = ph.get_type().unwrap();
            trace!(
                "[ELF] PH {}: type={:?}, vaddr={:#x}, filesz={:#x}, memsz={:#x}",
                i,
                ph_type,
                ph.virtual_addr(),
                ph.file_size(),
                ph.mem_size()
            );

            if ph_type == xmas_elf::program::Type::Tls {
                tls_info = Some(crate::task::TlsInfo {
                    vaddr: load_base + ph.virtual_addr() as usize,
                    file_offset: ph.offset() as usize,
                    filesz: ph.file_size() as usize,
                    memsz: ph.mem_size() as usize,
                    align: ph.align() as usize,
                });
                info!(
                    "[ELF] Found PT_TLS: vaddr={:#x}, filesz={:#x}, memsz={:#x}, align={:#x}",
                    ph.virtual_addr(),
                    ph.file_size(),
                    ph.mem_size(),
                    ph.align()
                );
            }
        }
        tls_info
    }

    fn prepare_auxv_info(elf: &xmas_elf::ElfFile, load_base: usize) -> crate::task::AuxvInfo {
        let elf_header = elf.header;
        let ph_count = elf_header.pt2.ph_count();
        let phdr_addr = if ph_count > 0 {
            let ph_offset = elf_header.pt2.ph_offset() as usize;
            let mut found = None;
            for i in 0..ph_count {
                let ph = elf.program_header(i).unwrap();
                if ph.get_type().unwrap() != xmas_elf::program::Type::Load {
                    continue;
                }
                let file_offset = ph.offset() as usize;
                let file_end = file_offset + ph.file_size() as usize;
                if ph_offset >= file_offset && ph_offset < file_end {
                    let vaddr = ph.virtual_addr() as usize;
                    found = Some(load_base + vaddr + (ph_offset - file_offset));
                    break;
                }
            }
            found.unwrap_or(0)
        } else {
            0
        };

        let auxv_info = crate::task::AuxvInfo {
            phdr_addr,
            phent_size: elf_header.pt2.ph_entry_size() as usize,
            phnum: ph_count as usize,
            entry: load_base + elf.header.pt2.entry_point() as usize,
        };

        if phdr_addr == 0 {
            info!("[ELF] Warning: AT_PHDR set to 0 (program headers not accessible)");
        }
        info!(
            "[ELF] Auxv: phdr={:#x}, phent={}, phnum={}, entry={:#x}",
            auxv_info.phdr_addr, auxv_info.phent_size, auxv_info.phnum, auxv_info.entry
        );
        if let Some(first_load) = (0..ph_count)
            .filter_map(|i| elf.program_header(i).ok())
            .find(|ph| ph.get_type().ok() == Some(xmas_elf::program::Type::Load))
        {
            info!(
                "[ELF] First PT_LOAD: vaddr={:#x}, offset={:#x}, filesz={:#x}",
                first_load.virtual_addr(),
                first_load.offset(),
                first_load.file_size()
            );
        }

        auxv_info
    }

    fn map_user_stack_and_trap(
        memory_set: &mut MemorySet,
        max_end_vpn: VirtPageNum,
    ) -> (usize, usize) {
        let max_end_va: VirtAddr = max_end_vpn.into();
        let heap_bottom: usize = max_end_va.into();
        let (user_stack_bottom, user_stack_top) =
            (USER_STACK_TOP - USER_STACK_SIZE, USER_STACK_TOP);
        memory_set.push(
            MapArea::new(
                user_stack_bottom.into(),
                user_stack_top.into(),
                MapType::Framed,
                MapPermission::R | MapPermission::W | MapPermission::U,
            ),
            None,
        );
        #[cfg(target_arch = "loongarch64")]
        {
            let tramp_base = user_stack_bottom.saturating_sub(PAGE_SIZE);
            let tramp_addr = arch::sigtrx::sigreturn_trampoline_addr();
            let tramp_page = tramp_addr & !(PAGE_SIZE - 1);
            info!(
                "[sigtrx_map] tramp_base={:#x} tramp_addr={:#x} tramp_page={:#x} offset={:#x}",
                tramp_base,
                tramp_addr,
                tramp_page,
                tramp_addr & (PAGE_SIZE - 1)
            );
            let tramp_bytes =
                unsafe { core::slice::from_raw_parts(tramp_page as *const u8, PAGE_SIZE) };
            memory_set.push(
                MapArea::new(
                    tramp_base.into(),
                    (tramp_base + PAGE_SIZE).into(),
                    MapType::Framed,
                    MapPermission::R | MapPermission::W | MapPermission::X | MapPermission::U,
                ),
                Some(tramp_bytes),
            );
        }
        #[cfg(target_arch = "riscv64")]
        {
            let tramp_base = arch::SIG_RETURN_ADDR;
            let tramp_addr = arch::sigtrx::sigreturn_trampoline_addr();
            let tramp_page = tramp_addr & !(PAGE_SIZE - 1);
            let tramp_bytes =
                unsafe { core::slice::from_raw_parts(tramp_page as *const u8, PAGE_SIZE) };
            memory_set.push(
                MapArea::new(
                    tramp_base.into(),
                    (tramp_base + PAGE_SIZE).into(),
                    MapType::Framed,
                    MapPermission::R | MapPermission::X | MapPermission::U,
                ),
                Some(tramp_bytes),
            );
        }
        if let Some(pte) = memory_set
            .page_table
            .translate(VirtAddr::from(user_stack_bottom).floor())
        {
            trace!(
                "[stack_map] bottom={:#x} top={:#x} pte_bits={:#x}",
                user_stack_bottom,
                user_stack_top,
                pte.bits
            );
        }
        memory_set.push(
            MapArea::new(
                heap_bottom.into(),
                heap_bottom.into(),
                MapType::Framed,
                MapPermission::R | MapPermission::W | MapPermission::U,
            ),
            None,
        );
        (heap_bottom, user_stack_top)
    }

    fn align_up(value: usize, align: usize) -> usize {
        (value + align - 1) & !(align - 1)
    }

    /// Include ELF segments and user stack,
    /// also returns heap_bottom, user_sp_base, entry point, TLS info (if present), and auxv info.
    /// 不处理解释器，只加载主程序 ELF
    pub fn from_elf(
        elf_data: &[u8],
    ) -> (
        Self,
        usize,
        usize,
        usize,
        Option<crate::task::TlsInfo>,
        crate::task::AuxvInfo,
    ) {
        let mut memory_set = Self::new_bare();
        // map trampoline
        memory_set.map_trampoline();
        // map program headers of elf, with U flag
        let elf = xmas_elf::ElfFile::new(elf_data).unwrap();
        let elf_header = elf.header;
        let magic = elf_header.pt1.magic;
        assert_eq!(magic, [0x7f, 0x45, 0x4c, 0x46], "invalid elf!");
        let _ph_count = elf_header.pt2.ph_count();
        let elf_type = elf_header.pt2.type_().as_type();
        let (has_interp, min_load_vaddr) = Self::scan_elf_meta(&elf, elf_data);
        if has_interp {
            warn!(
                "called from_elf but with interpreter, you should call from_elf_with_interp instead"
            );
        }
        let load_base = if elf_type == xmas_elf::header::Type::SharedObject && !has_interp {
            0x4000_0000usize
        } else {
            0
        };
        info!(
            "[ELF] type={:?} has_interp={} min_load_vaddr={:#x} load_base={:#x}",
            elf_type, has_interp, min_load_vaddr, load_base
        );
        let max_end_vpn = Self::map_load_segments(&mut memory_set, &elf, load_base);

        let tls_info = Self::scan_tls_info(&elf, load_base);
        if tls_info.is_none() {
            info!("[ELF] No PT_TLS segment found");
        }
        if !has_interp {
            info!("[ELF] No PT_INTERP (statically linked)");
        }

        let auxv_info = Self::prepare_auxv_info(&elf, load_base);

        let (heap_bottom, user_stack_top) =
            Self::map_user_stack_and_trap(&mut memory_set, max_end_vpn);
        let entry_point = load_base + elf.header.pt2.entry_point() as usize;
        (
            memory_set,
            heap_bottom,
            user_stack_top,
            entry_point,
            tls_info,
            auxv_info,
        )
    }

    /// Load main ELF plus interpreter ELF (PT_INTERP) into one address space.
    /// Returns main entry/auxv info and interpreter base/entry for initial jump.
    pub fn from_elf_with_interp(
        elf_data: &[u8],
        interp_data: &[u8],
    ) -> (
        Self,
        usize,
        usize,
        usize,
        Option<crate::task::TlsInfo>,
        crate::task::AuxvInfo,
        usize,
        usize,
    ) {
        let mut memory_set = Self::new_bare();
        memory_set.map_trampoline();

        let elf = xmas_elf::ElfFile::new(elf_data).unwrap();
        let elf_header = elf.header;
        let magic = elf_header.pt1.magic;
        assert_eq!(magic, [0x7f, 0x45, 0x4c, 0x46], "invalid elf!");
        let _ph_count = elf_header.pt2.ph_count();
        let elf_type = elf_header.pt2.type_().as_type();
        let (has_interp, min_load_vaddr) = Self::scan_elf_meta(&elf, elf_data);
        // PIE executables (SharedObject with interp, min_load_vaddr=0) need
        // a non-zero load_base so they don't start at VA 0x0.
        let load_base = if elf_type == xmas_elf::header::Type::SharedObject && min_load_vaddr == 0 {
            0x4000_0000usize
        } else {
            0
        };
        info!(
            "[ELF] type={:?} has_interp={} min_load_vaddr={:#x} load_base={:#x}",
            elf_type, has_interp, min_load_vaddr, load_base
        );

        let mut max_end_vpn = Self::map_load_segments(&mut memory_set, &elf, load_base);

        let tls_info = Self::scan_tls_info(&elf, load_base);
        if tls_info.is_none() {
            info!("[ELF] No PT_TLS segment found");
        }
        if !has_interp {
            info!("[ELF] No PT_INTERP (statically linked)");
        }

        let auxv_info = Self::prepare_auxv_info(&elf, load_base);

        let max_end_va: VirtAddr = max_end_vpn.into();
        let mut interp_base = max_end_va.into();
        interp_base = Self::align_up(interp_base, PAGE_SIZE);

        // 映射 解释器 ELF，注意解释器 ELF 也可能有 PT_TLS 和 PT_INTERP，但我们不处理解释器的解释器（递归加载），只加载第一层解释器
        let interp_elf = xmas_elf::ElfFile::new(interp_data).unwrap();
        let interp_max_end_vpn = Self::map_load_segments(&mut memory_set, &interp_elf, interp_base);
        if interp_max_end_vpn > max_end_vpn {
            max_end_vpn = interp_max_end_vpn;
        }

        let (heap_bottom, user_stack_top) =
            Self::map_user_stack_and_trap(&mut memory_set, max_end_vpn);

        // 静态链接的 ELF 入口点是主程序的 entry point，动态链接的 ELF 入口点是解释器的 entry point，loader 先跳转到解释器入口，由解释器负责加载主程序并跳转到主程序入口
        let entry_point = load_base + elf.header.pt2.entry_point() as usize;
        let interp_entry = interp_base + interp_elf.header.pt2.entry_point() as usize;
        (
            memory_set,
            heap_bottom,
            user_stack_top,
            entry_point,
            tls_info,
            auxv_info,
            interp_base,
            interp_entry,
        )
    }

    /// COW fork: create a new address space sharing physical frames with the parent.
    /// Writable pages are marked read-only in BOTH parent and child; actual copying
    /// is deferred to the page-fault handler (`handle_cow_fault`).
    pub fn from_existed_user(parent_space: &mut Self) -> Self {
        trace!("memory_set: COW clone user space start");
        let mut child = Self::new_bare();
        child.map_trampoline();
        debug!("[kernel] COW clone areas len {}", parent_space.areas.len());
        for (idx, area) in parent_space.areas.iter_mut().enumerate() {
            // Create a new MapArea with the same permissions and VPN range
            let mut new_area = MapArea::from_another(area);
            let is_shared = area.kind == MapAreaKind::Shared;
            for vpn in area.vpn_range {
                let src_pte = match parent_space.page_table.translate(vpn) {
                    Some(pte) if pte.is_valid() => pte,
                    _ => continue,
                };
                let src_ppn = src_pte.ppn();
                let src_flags = src_pte.flags();
                let is_writable = area.map_perm.contains(MapPermission::W) && !is_shared;
                // Share the frame: clone the Arc
                let shared_frame = if let Some(frame_arc) = area.data_frames.get(&vpn) {
                    frame_arc.clone()
                } else {
                    // Page not tracked by data_frames (e.g. identity-mapped kernel area
                    // should not appear here, but handle gracefully).
                    // Allocate a fresh frame and copy.
                    let frame = frame_alloc().unwrap();
                    frame.ppn.get_bytes_array().copy_from_slice(src_ppn.get_bytes_array());
                    let arc = Arc::new(frame);
                    // For writable pages, also need to remove W from parent
                    if is_writable {
                        let ro_flags = src_flags & !PTEFlags::W;
                        parent_space.page_table.change_pte_flags(vpn, ro_flags);
                    }
                    // Map child with same flags as parent (possibly already RO)
                    let child_flags = if is_writable {
                        src_flags & !PTEFlags::W
                    } else {
                        src_flags
                    };
                    child.page_table.map(vpn, arc.ppn, child_flags);
                    new_area.data_frames.insert(vpn, arc);
                    continue;
                };

                if is_shared {
                    // SysV SHM semantics: keep parent/child mappings writable-shared.
                    let mut shared_flags = src_flags;
                    if area.map_perm.contains(MapPermission::W) {
                        shared_flags |= PTEFlags::W;
                        if !src_flags.contains(PTEFlags::W) {
                            parent_space
                                .page_table
                                .change_pte_flags(vpn, shared_flags);
                        }
                    } else {
                        shared_flags &= !PTEFlags::W;
                    }
                    child.page_table.map(vpn, shared_frame.ppn, shared_flags);
                    new_area.data_frames.insert(vpn, shared_frame);
                    continue;
                }

                if is_writable && src_flags.contains(PTEFlags::W) {
                    // Remove W from parent's PTE (defer copy to fault handler)
                    let ro_flags = src_flags & !PTEFlags::W;
                    parent_space.page_table.change_pte_flags(vpn, ro_flags);
                }

                // Map child page with read-only flags (if originally writable)
                let child_flags = if is_writable {
                    src_flags & !PTEFlags::W
                } else {
                    src_flags
                };
                child.page_table.map(vpn, shared_frame.ppn, child_flags);
                new_area.data_frames.insert(vpn, shared_frame);
            }
            child.areas.push(new_area);
            debug!("[kernel] COW clone area {} done", idx);
        }
        // Flush parent's TLB since we removed W bits from its PTEs
        flush_tlb();
        trace!("memory_set: COW clone user space done");
        child
    }

    /// Best-effort synchronization for clone(CLONE_VM|CLONE_VFORK):
    /// copy writable user pages from `src` back into `self`.
    pub fn sync_user_writable_from(&mut self, src: &Self) -> usize {
        let mut copied_pages = 0usize;
        for area in src.areas.iter() {
            if !area.map_perm.contains(MapPermission::U) || !area.map_perm.contains(MapPermission::W) {
                continue;
            }
            for vpn in area.vpn_range {
                let Some(src_pte) = src.page_table.translate(vpn) else {
                    continue;
                };
                let Some(dst_pte) = self.page_table.translate(vpn) else {
                    continue;
                };
                if !src_pte.is_valid() || !dst_pte.is_valid() {
                    continue;
                }
                let src_ppn = src_pte.ppn();
                let dst_ppn = dst_pte.ppn();
                if src_ppn == dst_ppn {
                    continue;
                }
                dst_ppn
                    .get_bytes_array()
                    .copy_from_slice(src_ppn.get_bytes_array());
                copied_pages += 1;
            }
        }
        copied_pages
    }

    /// Handle a COW page fault at `addr`.
    /// Returns `true` if the fault was a COW fault and was resolved,
    /// `false` if it's a genuine page fault (caller should send SIGSEGV).
    pub fn handle_cow_fault(&mut self, addr: usize) -> bool {
        let fault_vpn = VirtAddr::from(addr).floor();

        // Find the MapArea containing this VPN
        let area = match self.areas.iter_mut().find(|a| {
            a.vpn_range.get_start() <= fault_vpn && fault_vpn < a.vpn_range.get_end()
        }) {
            Some(a) => a,
            None => return false,
        };

        // Check: area should be writable but PTE is read-only
        if !area.map_perm.contains(MapPermission::W) {
            return false;
        }

        let pte = match self.page_table.translate(fault_vpn) {
            Some(pte) if pte.is_valid() && !pte.writable() => pte,
            _ => return false,
        };

        if area.kind == MapAreaKind::Shared {
            // Shared memory must not be privatized by COW fault handling.
            // If a SHM PTE becomes read-only (e.g., due to legacy state),
            // restore writability directly instead of allocating a private page.
            let shared_flags = pte.flags() | PTEFlags::W;
            self.page_table.change_pte_flags(fault_vpn, shared_flags);
            flush_tlb();
            return true;
        }

        let old_ppn = pte.ppn();
        let old_flags = pte.flags();

        // Build the writable flags
        let new_flags = old_flags | PTEFlags::W;

        // Check if we're the sole owner of this frame
        let frame_arc = match area.data_frames.get(&fault_vpn) {
            Some(arc) => arc,
            None => return false,
        };

        if Arc::strong_count(frame_arc) == 1 {
            // Sole owner: just make it writable again, no copy needed
            self.page_table.change_pte_flags(fault_vpn, new_flags);
            flush_tlb();
            trace!(
                "[cow] sole-owner vpn={:#x} ppn={:#x}",
                fault_vpn.0, old_ppn.0
            );
            return true;
        }

        // Shared: allocate new frame, copy data, remap
        let new_frame = match frame_alloc() {
            Some(f) => f,
            None => {
                error!("[cow] frame_alloc failed for vpn={:#x}", fault_vpn.0);
                return false;
            }
        };
        let new_ppn = new_frame.ppn;
        new_ppn.get_bytes_array().copy_from_slice(old_ppn.get_bytes_array());

        // Replace the Arc (drops our reference to the shared frame)
        let new_arc = Arc::new(new_frame);
        area.data_frames.insert(fault_vpn, new_arc.clone());

        // Remap to new frame with write permission
        self.page_table.map(fault_vpn, new_ppn, new_flags);
        flush_tlb();
        trace!(
            "[cow] copied vpn={:#x} old_ppn={:#x} new_ppn={:#x}",
            fault_vpn.0, old_ppn.0, new_ppn.0
        );
        true
    }

    /// Debug helper to dump user area ranges.
    pub fn debug_area_ranges(&self) {
        debug!("[kernel] user areas: {}", self.areas.len());
        for (idx, area) in self.areas.iter().enumerate() {
            debug!(
                "[kernel] area {} {:?} {:?} perm={:#x}",
                idx,
                area.vpn_range.get_start(),
                area.vpn_range.get_end(),
                area.map_perm.bits
            );
        }
    }

    /// Render a minimal `/proc/<pid>/maps` view for user-space probes.
    pub fn render_proc_maps(
        &self,
        process_name: &str,
        heap_bottom: usize,
        program_brk: usize,
    ) -> String {
        let mut out = String::new();
        let mut emitted = 0usize;

        for area in self.areas.iter() {
            let start: usize = area.start_va.into();
            let end: usize = area.vpn_range.get_end().into();
            if start >= end {
                continue;
            }

            let mut perms = ['-'; 4];
            if area.map_perm.contains(MapPermission::R) {
                perms[0] = 'r';
            }
            if area.map_perm.contains(MapPermission::W) {
                perms[1] = 'w';
            }
            if area.map_perm.contains(MapPermission::X) {
                perms[2] = 'x';
            }
            perms[3] = if area.mmap_meta.is_some() { 's' } else { 'p' };

            let mut label = "";
            if heap_bottom >= start && heap_bottom < end && program_brk >= heap_bottom {
                label = " [heap]";
            } else if area.map_perm.contains(MapPermission::X) && emitted == 0 {
                label = " /";
            }

            if label == " /" {
                out.push_str(&format!(
                    "{start:016x}-{end:016x} {} 00000000 00:00 0 /{process_name}\n",
                    perms.iter().collect::<String>(),
                ));
            } else {
                out.push_str(&format!(
                    "{start:016x}-{end:016x} {} 00000000 00:00 0{label}\n",
                    perms.iter().collect::<String>(),
                ));
            }
            emitted += 1;
        }

        if out.is_empty() {
            out.push_str("0000000000010000-0000000000011000 r-xp 00000000 00:00 0 /unknown\n");
        }

        out
    }
    /// Change page table by writing satp CSR Register.
    pub fn activate(&self) {
        arch::init_kernel_page_table(self.page_table.token());
    }
    /// Translate a virtual page number to a page table entry
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.page_table.translate(vpn)
    }

    /// Check whether every page in `[start, start + len)` belongs to a user
    /// area that is writable in VMA permission and currently mapped as a user page.
    pub fn is_user_range_writable(&self, start: usize, len: usize) -> bool {
        if len == 0 {
            return true;
        }
        let Some(end) = start.checked_add(len) else {
            return false;
        };
        let mut va = start;
        while va < end {
            let vpn = VirtAddr::from(va).floor();
            let in_writable_user_area = self.areas.iter().any(|area| {
                area.vpn_range.get_start() <= vpn
                    && vpn < area.vpn_range.get_end()
                    && area.map_perm.contains(MapPermission::U)
                    && area.map_perm.contains(MapPermission::W)
            });
            if !in_writable_user_area {
                return false;
            }
            let Some(pte) = self.page_table.translate(vpn) else {
                return false;
            };
            let flags = pte.flags();
            if !pte.is_valid() || !flags.contains(PTEFlags::U) {
                return false;
            }
            let next_page = ((va / PAGE_SIZE) + 1) * PAGE_SIZE;
            va = next_page.max(va + 1);
        }
        true
    }

    /// Count mapped areas that overlap with [start, end).
    pub fn overlap_count(&self, start: VirtAddr, end: VirtAddr) -> usize {
        let start_vpn = start.floor();
        let end_vpn = end.ceil();
        self.areas
            .iter()
            .filter(|area| {
                let area_start = area.vpn_range.get_start();
                let area_end = area.vpn_range.get_end();
                area_start < end_vpn && area_end > start_vpn
            })
            .count()
    }

    /// Get overlapping area ranges for [start, end).
    pub fn overlap_ranges(&self, start: VirtAddr, end: VirtAddr) -> Vec<(VirtAddr, VirtAddr)> {
        let start_vpn = start.floor();
        let end_vpn = end.ceil();
        self.areas
            .iter()
            .filter_map(|area| {
                let area_start = area.vpn_range.get_start();
                let area_end = area.vpn_range.get_end();
                if area_start < end_vpn && area_end > start_vpn {
                    Some((area_start.into(), area_end.into()))
                } else {
                    None
                }
            })
            .collect()
    }

    ///Remove all `MapArea`
    pub fn recycle_data_pages(&mut self) {
        self.areas.clear();
    }

    /// shrink the area to new_end
    pub fn shrink_to(&mut self, start: VirtAddr, new_end: VirtAddr) -> bool {
        if let Some(area) = self
            .areas
            .iter_mut()
            .find(|area| area.vpn_range.get_start() == start.floor())
        {
            area.shrink_to(&mut self.page_table, new_end.ceil());
            true
        } else {
            false
        }
    }

    /// append the area to new_end
    pub fn append_to(&mut self, start: VirtAddr, new_end: VirtAddr) -> bool {
        if let Some(area) = self
            .areas
            .iter_mut()
            .find(|area| area.vpn_range.get_start() == start.floor())
        {
            area.append_to(&mut self.page_table, new_end.ceil());
            true
        } else {
            false
        }
    }

    /// COW remap: replace the mapping of `vpn` with `new_frame` and `new_flags`.
    /// The caller has already copied the page content into new_frame.
    pub fn remap_cow(&mut self, vpn: VirtPageNum, new_frame: FrameTracker, new_flags: PTEFlags) {
        // Update page table entry to point to new frame
        self.page_table.map(vpn, new_frame.ppn, new_flags);
        let new_arc = Arc::new(new_frame);
        // Store the frame tracker so it doesn't get freed
        // Find the area containing this VPN and add the frame
        for area in self.areas.iter_mut() {
            if area.vpn_range.get_start() <= vpn && vpn < area.vpn_range.get_end() {
                area.data_frames.insert(vpn, new_arc);
                return;
            }
        }
        // If no area found, leak to prevent dealloc
        core::mem::forget(new_arc);
    }

    /// Change memory protection for a region
    /// Returns true on success, false if region not found or invalid
    pub fn change_protection(
        &mut self,
        start: VirtAddr,
        end: VirtAddr,
        new_perm: MapPermission,
    ) -> Result<(), ProtectError> {
        let start_vpn = start.floor();
        let end_vpn = end.ceil();
        if start_vpn >= end_vpn {
            return Ok(());
        }

        let mut cursor = start_vpn;
        while cursor < end_vpn {
            let mut next_end: Option<VirtPageNum> = None;
            for area in self.areas.iter() {
                let area_start = area.vpn_range.get_start();
                let area_end = area.vpn_range.get_end();
                if area_start <= cursor && area_end > cursor {
                    next_end = Some(match next_end {
                        Some(cur_end) => cur_end.max(area_end),
                        None => area_end,
                    });
                }
            }
            let Some(covered_end) = next_end else {
                return Err(ProtectError::Unmapped);
            };
            cursor = covered_end.min(end_vpn);
        }

        for area in self.areas.iter() {
            let area_start = area.vpn_range.get_start();
            let area_end = area.vpn_range.get_end();
            if area_start < end_vpn && area_end > start_vpn {
                if let Some(meta) = area.mmap_meta {
                    if meta.file_backed
                        && meta.shared
                        && new_perm.contains(MapPermission::W)
                        && !meta.file_writable
                    {
                        return Err(ProtectError::AccessDenied);
                    }
                }
            }
        }

        let mut idx = 0usize;
        while idx < self.areas.len() {
            let area_start = self.areas[idx].vpn_range.get_start();
            let area_end = self.areas[idx].vpn_range.get_end();
            if area_end <= start_vpn || area_start >= end_vpn {
                idx += 1;
                continue;
            }

            let modify_start = area_start.max(start_vpn);
            let modify_end = area_end.min(end_vpn);
            if modify_start == area_start && modify_end == area_end {
                self.areas[idx].apply_perm(&mut self.page_table, new_perm);
                idx += 1;
                continue;
            }

            if modify_start == area_start {
                let right = {
                    let area = &mut self.areas[idx];
                    area.split_off(modify_end)
                };
                self.areas[idx].apply_perm(&mut self.page_table, new_perm);
                self.areas.insert(idx + 1, right);
                idx += 2;
                continue;
            }

            if modify_end == area_end {
                let mut middle = {
                    let area = &mut self.areas[idx];
                    area.split_off(modify_start)
                };
                middle.apply_perm(&mut self.page_table, new_perm);
                self.areas.insert(idx + 1, middle);
                idx += 2;
                continue;
            }

            let (mut middle, right) = {
                let area = &mut self.areas[idx];
                let mut middle = area.split_off(modify_start);
                let right = middle.split_off(modify_end);
                (middle, right)
            };
            middle.apply_perm(&mut self.page_table, new_perm);
            self.areas.insert(idx + 1, middle);
            self.areas.insert(idx + 2, right);
            idx += 3;
        }

        Ok(())
    }
}
/// map area structure, controls a contiguous piece of virtual memory
pub struct MapArea {
    start_va: VirtAddr,
    vpn_range: VPNRange,
    data_frames: BTreeMap<VirtPageNum, Arc<FrameTracker>>,
    map_type: MapType,
    map_perm: MapPermission,
    shared_frames: bool,
    mmap_meta: Option<MmapMeta>,
    kind: MapAreaKind,
}

impl MapArea {
    pub fn new(
        start_va: VirtAddr,
        end_va: VirtAddr,
        map_type: MapType,
        map_perm: MapPermission,
    ) -> Self {
        Self::new_with_kind(start_va, end_va, map_type, map_perm, MapAreaKind::Private)
    }
    fn new_with_kind(
        start_va: VirtAddr,
        end_va: VirtAddr,
        map_type: MapType,
        map_perm: MapPermission,
        kind: MapAreaKind,
    ) -> Self {
        let start_vpn: VirtPageNum = start_va.floor();
        let end_vpn: VirtPageNum = end_va.ceil();
        Self {
            start_va,
            vpn_range: VPNRange::new(start_vpn, end_vpn),
            data_frames: BTreeMap::new(),
            map_type,
            map_perm,
            shared_frames: false,
            mmap_meta: None,
            kind,
        }
    }
    pub fn from_another(another: &Self) -> Self {
        Self {
            start_va: another.start_va,
            vpn_range: VPNRange::new(another.vpn_range.get_start(), another.vpn_range.get_end()),
            data_frames: BTreeMap::new(),
            map_type: another.map_type,
            map_perm: another.map_perm,
            shared_frames: another.shared_frames,
            mmap_meta: another.mmap_meta,
            kind: another.kind,
        }
    }
    fn with_shared_frames(mut self) -> Self {
        self.shared_frames = true;
        self
    }
    fn with_mmap_meta(mut self, meta: MmapMeta) -> Self {
        self.mmap_meta = Some(meta);
        self
    }
    #[allow(dead_code)]
    fn shares_frames(&self) -> bool {
        self.shared_frames
    }
    fn split_off(&mut self, split_vpn: VirtPageNum) -> Self {
        let start_vpn = self.vpn_range.get_start();
        let end_vpn = self.vpn_range.get_end();
        assert!(split_vpn >= start_vpn && split_vpn <= end_vpn);
        let data_frames = if self.map_type == MapType::Framed {
            self.data_frames.split_off(&split_vpn)
        } else {
            BTreeMap::new()
        };
        let new_area = Self {
            start_va: VirtAddr::from(split_vpn),
            vpn_range: VPNRange::new(split_vpn, end_vpn),
            data_frames,
            map_type: self.map_type,
            map_perm: self.map_perm,
            shared_frames: self.shared_frames,
            mmap_meta: self.mmap_meta,
            kind: self.kind,
        };
        self.vpn_range = VPNRange::new(start_vpn, split_vpn);
        new_area
    }
    fn apply_perm(&mut self, page_table: &mut PageTable, new_perm: MapPermission) {
        let flags = map_perm_to_pte_flags(new_perm);
        for vpn in self.vpn_range {
            page_table.change_pte_flags(vpn, flags);
        }
        self.map_perm = new_perm;
        if let Some(pte) = page_table.translate(self.vpn_range.get_start()) {
            let area_start_addr: usize = VirtAddr::from(self.vpn_range.get_start()).into();
            let area_end_addr: usize = VirtAddr::from(self.vpn_range.get_end()).into();
            trace!(
                "[mprotect] area={:#x}-{:#x} perm_bits={:#x} pte_bits={:#x}",
                area_start_addr,
                area_end_addr,
                new_perm.bits,
                pte.bits
            );
        }
    }
    pub fn map_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        // If already mapped (shared page between adjacent LOAD segments),
        // COW: allocate new frame, copy old page content, remap with merged permissions.
        // This preserves text data from LOAD1 while allowing LOAD2 to write data.
        if let Some(pte) = page_table.translate(vpn) {
            if pte.is_valid() {
                let old_ppn = pte.ppn();
                let new_frame = frame_alloc().unwrap();
                let new_ppn = new_frame.ppn;
                // Copy old page content to new frame
                new_ppn.get_bytes_array().copy_from_slice(old_ppn.get_bytes_array());
                self.data_frames.insert(vpn, Arc::new(new_frame));
                let new_flags = PTEFlags::from_bits(self.map_perm.bits).unwrap();
                let merged = PTEFlags::from_bits(pte.flags().bits() | new_flags.bits()).unwrap();
                page_table.unmap(vpn);
                page_table.map(vpn, new_ppn, merged);
                return;
            }
        }
        let frame = match frame_alloc() {
            Some(f) => f,
            None => {
                error!("[map_one] frame_alloc OOM for vpn={:#x}", vpn.0);
                return;
            }
        };
        let ppn: PhysPageNum = frame.ppn;
        // Zero the frame -- anonymous mmap and BSS require zero-initialized pages.
        ppn.get_bytes_array().fill(0);
        self.data_frames.insert(vpn, Arc::new(frame));
        let pte_flags = PTEFlags::from_bits(self.map_perm.bits).unwrap();
        page_table.map(vpn, ppn, pte_flags);
    }
    #[allow(dead_code)]
    fn map_shared_from(
        &mut self,
        page_table: &mut PageTable,
        parent: &Self,
        src_page_table: &PageTable,
    ) {
        assert_eq!(self.map_type, MapType::Framed);
        assert_eq!(parent.map_type, MapType::Framed);
        for vpn in self.vpn_range {
            let frame = parent
                .data_frames
                .get(&vpn)
                .unwrap_or_else(|| panic!("shared map missing frame for vpn {:?}", vpn))
                .clone();
            let pte_flags = src_page_table
                .translate(vpn)
                .map(|pte| pte.flags())
                .unwrap_or_else(|| PTEFlags::from_bits(self.map_perm.bits).unwrap());
            page_table.map(vpn, frame.ppn, pte_flags);
            self.data_frames.insert(vpn, frame);
        }
    }
    pub fn unmap_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        if self.map_type == MapType::Framed {
            self.data_frames.remove(&vpn);
        }
        let _ = page_table.unmap(vpn);
    }
    pub fn map(&mut self, page_table: &mut PageTable) {
        for (_i, vpn) in self.vpn_range.into_iter().enumerate() {
            self.map_one(page_table, vpn);
            #[cfg(target_arch = "loongarch64")]
            if _i == 0 {
                if let Some(pte) = page_table.translate(vpn) {
                    info!(
                        "[ELF] map_one first vpn: vpn={:#x} pte_bits={:#x}",
                        vpn.0, pte.bits
                    );
                } else {
                    info!("[ELF] map_one first vpn: vpn={:#x} pte=None", vpn.0);
                }
            }
        }
    }
    pub fn unmap(&mut self, page_table: &mut PageTable) {
        for vpn in self.vpn_range {
            self.unmap_one(page_table, vpn);
        }
    }
    #[allow(unused)]
    pub fn shrink_to(&mut self, page_table: &mut PageTable, new_end: VirtPageNum) {
        for vpn in VPNRange::new(new_end, self.vpn_range.get_end()) {
            self.unmap_one(page_table, vpn)
        }
        self.vpn_range = VPNRange::new(self.vpn_range.get_start(), new_end);
    }
    #[allow(unused)]
    pub fn append_to(&mut self, page_table: &mut PageTable, new_end: VirtPageNum) {
        for vpn in VPNRange::new(self.vpn_range.get_end(), new_end) {
            self.map_one(page_table, vpn)
        }
        self.vpn_range = VPNRange::new(self.vpn_range.get_start(), new_end);
    }
    /// data: start-aligned but maybe with shorter length
    /// assume that all frames were cleared before
    pub fn copy_data(&mut self, page_table: &mut PageTable, data: &[u8]) {
        let mut data_offset: usize = 0;
        let mut current_vpn = self.vpn_range.get_start();
        let data_len = data.len();
        let mut first_page = true;
        while data_offset < data_len {
            let dst_offset = if first_page {
                self.start_va.page_offset()
            } else {
                0
            };
            let copy_len = (PAGE_SIZE - dst_offset).min(data_len - data_offset);
            let pte = page_table.translate(current_vpn).unwrap();
            #[cfg(target_arch = "loongarch64")]
            if first_page {
                let pa = pte.ppn().0 * PAGE_SIZE;
                info!(
                    "[ELF] copy_data first page: vpn={:#x} pte_bits={:#x} pa={:#x} data_len={:#x} start_va={:#x}",
                    current_vpn.0,
                    pte.bits,
                    pa,
                    data_len
                    ,self.start_va.0
                );
                assert!(
                    pte.bits != 0,
                    "[ELF] copy_data unmapped vpn: vpn={:#x}",
                    current_vpn.0
                );
                assert!(
                    pa < MEMORY_END,
                    "[ELF] copy_data pa out of RAM: pa={:#x} memory_end={:#x}",
                    pa,
                    MEMORY_END
                );
            }
            let page_bytes = pte.ppn().get_bytes_array();
            let dst = &mut page_bytes[dst_offset..dst_offset + copy_len];
            let src = &data[data_offset..data_offset + copy_len];
            dst.copy_from_slice(src);
            data_offset += copy_len;
            current_vpn.step();
            first_page = false;
        }
    }
}

/// Minimal mmap provenance used by `mprotect()` compatibility checks.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MmapMeta {
    /// Whether the mapping was created with `MAP_SHARED`.
    pub shared: bool,
    /// Whether the mapping is backed by a file instead of anonymous memory.
    pub file_backed: bool,
    /// Whether the originating file descriptor allowed writes.
    pub file_writable: bool,
}

fn map_perm_to_pte_flags(map_perm: MapPermission) -> PTEFlags {
    let mut flags = PTEFlags::V;
    if map_perm.contains(MapPermission::R) {
        flags |= PTEFlags::R;
    }
    if map_perm.contains(MapPermission::W) {
        flags |= PTEFlags::W;
    }
    if map_perm.contains(MapPermission::X) {
        flags |= PTEFlags::X;
    }
    if map_perm.contains(MapPermission::U) {
        flags |= PTEFlags::U;
    }
    flags
}

#[allow(dead_code)]
#[derive(Copy, Clone, PartialEq, Debug)]
/// map type for memory set: identical or framed
pub enum MapType {
    Identical,
    Framed,
}
bitflags! {
    /// map permission corresponding to that in pte: `R W X U`
    pub struct MapPermission: u8 {
        ///Readable
        const R = 1 << 1;
        ///Writable
        const W = 1 << 2;
        ///Excutable
        const X = 1 << 3;
        ///Accessible in U mode
        const U = 1 << 4;
    }
}

/// remap test in kernel space
#[allow(unused)]

pub fn remap_test() {
    #[cfg(not(target_arch = "loongarch64"))]
    {
        let mut kernel_space = KERNEL_SPACE.exclusive_access();
        let mid_text: VirtAddr = ((stext as usize + etext as usize) / 2).into();
        let mid_rodata: VirtAddr = ((srodata as usize + erodata as usize) / 2).into();
        let mid_data: VirtAddr = ((sdata as usize + edata as usize) / 2).into();
        assert!(!kernel_space
            .page_table
            .translate(mid_text.floor())
            .unwrap()
            .writable(),);
        assert!(!kernel_space
            .page_table
            .translate(mid_rodata.floor())
            .unwrap()
            .writable(),);
        assert!(!kernel_space
            .page_table
            .translate(mid_data.floor())
            .unwrap()
            .executable(),);
    }

    println!("remap_test passed!");
}
