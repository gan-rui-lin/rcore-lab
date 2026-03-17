//! Implementation of [`MapArea`] and [`MemorySet`].
use super::{frame_alloc, FrameTracker};
use super::{PTEFlags, PageTable, PageTableEntry};
use super::{PhysPageNum, VirtAddr, VirtPageNum};
use super::{StepByOne, VPNRange};
#[allow(unused_imports)]
use crate::config::{MEMORY_END, MMIO, PAGE_SIZE, USER_STACK_SIZE};
use crate::config::USER_STACK_TOP;
use crate::sync::UPIntrFreeCell;
use alloc::collections::BTreeMap;
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

/// address space
pub struct MemorySet {
    page_table: PageTable,
    areas: Vec<MapArea>,
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
        self.push(MapArea::new(start_va, end_va, permission), None);
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
                        info!("[ELF] Found PT_INTERP: {}", interp_str.trim_end_matches('\0'));
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
                let map_area = MapArea::new(start_va, end_va, map_perm);
                max_end_vpn = map_area.vpn_range.get_end();
                memory_set.push(
                    map_area,
                    Some(&elf.input[ph.offset() as usize..(ph.offset() + ph.file_size()) as usize]),
                );
            }
        }
        max_end_vpn
    }

    fn scan_tls_info(
        elf: &xmas_elf::ElfFile,
        load_base: usize,
    ) -> Option<crate::task::TlsInfo> {
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

    fn prepare_auxv_info(
        elf: &xmas_elf::ElfFile,
        load_base: usize,
    ) -> crate::task::AuxvInfo {
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
            auxv_info.phdr_addr,
            auxv_info.phent_size,
            auxv_info.phnum,
            auxv_info.entry
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
            let tramp_bytes = unsafe {
                core::slice::from_raw_parts(tramp_page as *const u8, PAGE_SIZE)
            };
            memory_set.push(
                MapArea::new(
                    tramp_base.into(),
                    (tramp_base + PAGE_SIZE).into(),
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
            let tramp_bytes = unsafe {
                core::slice::from_raw_parts(tramp_page as *const u8, PAGE_SIZE)
            };
            memory_set.push(
                MapArea::new(
                    tramp_base.into(),
                    (tramp_base + PAGE_SIZE).into(),
                    MapPermission::R | MapPermission::X | MapPermission::U,
                ),
                Some(tramp_bytes),
            );
        }
        if let Some(pte) = memory_set.page_table.translate(VirtAddr::from(user_stack_bottom).floor()) {
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
        elf_data: &[u8]
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
            elf_type,
            has_interp,
            min_load_vaddr,
            load_base
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
        let load_base = if elf_type == xmas_elf::header::Type::SharedObject && !has_interp {
            0x4000_0000usize
        } else {
            0
        };
        info!(
            "[ELF] type={:?} has_interp={} min_load_vaddr={:#x} load_base={:#x}",
            elf_type,
            has_interp,
            min_load_vaddr,
            load_base
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
    /// Create a new address space by copy code&data from a exited process's address space.
    pub fn from_existed_user(user_space: &Self) -> Self {
        // debug!("[kernel] clone user space start");
        trace!("memory_set: clone user space start");
        let mut memory_set = Self::new_bare();
        // map trampoline
        debug!("[kernel] clone: map_trampoline start");
        memory_set.map_trampoline();
        debug!("[kernel] clone: map_trampoline done");
        debug!("[kernel] clone areas len {}", user_space.areas.len());
        // copy data sections/trap_context/user_stack
        for (idx, area) in user_space.areas.iter().enumerate() {
            debug!("[kernel] clone area {}", idx);
            trace!(
                "memory_set: clone area {} start={:?} end={:?}",
                idx,
                area.vpn_range.get_start(),
                area.vpn_range.get_end()
            );
            let new_area = MapArea::from_another(area);
            memory_set.push(new_area, None);
            if area.map_perm.contains(MapPermission::U)
                && area.map_perm.contains(MapPermission::R)
                && area.map_perm.contains(MapPermission::W)
            {
                let start_vpn = area.vpn_range.get_start();
                let end_vpn = area.vpn_range.get_end();
                let start_addr: usize = VirtAddr::from(start_vpn).into();
                let end_addr: usize = VirtAddr::from(end_vpn).into();
                if let Some(pte) = memory_set
                    .page_table
                    .translate(VirtAddr::from(start_addr).floor())
                {
                    trace!(
                        "[clone_area] idx={} start={:#x} end={:#x} pte_bits={:#x}",
                        idx,
                        start_addr,
                        end_addr,
                        pte.bits
                    );
                }
            }
            // copy data from another space
            let mut pages_copied: usize = 0;
            for vpn in area.vpn_range {
                let src_pte = user_space.translate(vpn).unwrap();
                let src_ppn = src_pte.ppn();
                let dst_ppn = memory_set.translate(vpn).unwrap().ppn();
                dst_ppn
                    .get_bytes_array()
                    .copy_from_slice(src_ppn.get_bytes_array());
                memory_set
                    .page_table
                    .change_pte_flags(vpn, src_pte.flags());
                pages_copied += 1;
                if (pages_copied & 0x3ff) == 0 {
                    debug!("[kernel] area {} copied {} pages", idx, pages_copied);
                    trace!(
                        "memory_set: area {} copied {} pages",
                        idx,
                        pages_copied
                    );
                }
            }
            debug!("[kernel] clone area {} done ({} pages)", idx, pages_copied);
            trace!(
                "memory_set: clone area {} done ({} pages)",
                idx,
                pages_copied
            );
        }
        // debug!("[kernel] clone user space done");
        trace!("memory_set: clone user space done");
        memory_set
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
    /// Change page table by writing satp CSR Register.
    pub fn activate(&self) {
        arch::init_kernel_page_table(self.page_table.token());
    }
    /// Translate a virtual page number to a page table entry
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.page_table.translate(vpn)
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

    /// Change memory protection for a region
    /// Returns true on success, false if region not found or invalid
    pub fn change_protection(
        &mut self,
        start: VirtAddr,
        end: VirtAddr,
        new_perm: MapPermission,
    ) -> bool {
        let start_vpn = start.floor();
        let end_vpn = end.ceil();

        // Find overlapping areas and change their permissions
        let mut success = false;
        for area in self.areas.iter_mut() {
            let area_start = area.vpn_range.get_start();
            let area_end = area.vpn_range.get_end();

            // Check if this area overlaps with the requested range
            if area_start < end_vpn && area_end > start_vpn {
                // Calculate the actual range to modify within this area
                let modify_start = area_start.max(start_vpn);
                let modify_end = area_end.min(end_vpn);
                let area_start_addr: usize = VirtAddr::from(area_start).into();
                let area_end_addr: usize = VirtAddr::from(area_end).into();
                let modify_start_addr: usize = VirtAddr::from(modify_start).into();
                let modify_end_addr: usize = VirtAddr::from(modify_end).into();

                // Convert MapPermission to PTEFlags
                let mut flags = PTEFlags::V;
                if new_perm.contains(MapPermission::R) {
                    flags |= PTEFlags::R;
                }
                if new_perm.contains(MapPermission::W) {
                    flags |= PTEFlags::W;
                }
                if new_perm.contains(MapPermission::X) {
                    flags |= PTEFlags::X;
                }
                if new_perm.contains(MapPermission::U) {
                    flags |= PTEFlags::U;
                }

                // Update page table entries for this range
                for vpn in VPNRange::new(modify_start, modify_end) {
                    self.page_table.change_pte_flags(vpn, flags);
                }

                if let Some(pte) = self.page_table.translate(modify_start) {
                    trace!(
                        "[mprotect] area={:#x}-{:#x} modify={:#x}-{:#x} perm_bits={:#x} pte_bits={:#x}",
                        area_start_addr,
                        area_end_addr,
                        modify_start_addr,
                        modify_end_addr,
                        new_perm.bits,
                        pte.bits
                    );
                }

                // If the entire area is being modified, update the area's permission
                if modify_start == area_start && modify_end == area_end {
                    area.map_perm = new_perm;
                }

                success = true;
            }
        }

        success
    }
}
/// map area structure, controls a contiguous piece of virtual memory
pub struct MapArea {
    start_va: VirtAddr,
    vpn_range: VPNRange,
    data_frames: BTreeMap<VirtPageNum, FrameTracker>,
    map_perm: MapPermission,
}

impl MapArea {
    pub fn new(
        start_va: VirtAddr,
        end_va: VirtAddr,
        map_perm: MapPermission,
    ) -> Self {
        let start_vpn: VirtPageNum = start_va.floor();
        let end_vpn: VirtPageNum = end_va.ceil();
        Self {
            start_va,
            vpn_range: VPNRange::new(start_vpn, end_vpn),
            data_frames: BTreeMap::new(),
            map_perm,
        }
    }
    pub fn from_another(another: &Self) -> Self {
        Self {
            start_va: another.start_va,
            vpn_range: VPNRange::new(another.vpn_range.get_start(), another.vpn_range.get_end()),
            data_frames: BTreeMap::new(),
            map_perm: another.map_perm,
        }
    }
    pub fn map_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        let frame = frame_alloc().unwrap();
        let ppn: PhysPageNum = frame.ppn;
        self.data_frames.insert(vpn, frame);
        let pte_flags = PTEFlags::from_bits(self.map_perm.bits).unwrap();
        page_table.map(vpn, ppn, pte_flags);
    }
    pub fn unmap_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        self.data_frames.remove(&vpn);
        page_table.unmap(vpn);
    }
    pub fn map(&mut self, page_table: &mut PageTable) {
        for (_i, vpn) in self.vpn_range.into_iter().enumerate() {
            self.map_one(page_table, vpn);
            #[cfg(target_arch = "loongarch64")]
            if _i == 0 {
                if let Some(pte) = page_table.translate(vpn) {
                    info!(
                        "[ELF] map_one first vpn: vpn={:#x} pte_bits={:#x}",
                        vpn.0,
                        pte.bits
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
            let dst = &mut pte.ppn().get_bytes_array()[dst_offset..dst_offset + copy_len];
            let src = &data[data_offset..data_offset + copy_len];
            dst.copy_from_slice(src);
            data_offset += copy_len;
            current_vpn.step();
            first_page = false;
        }
    }
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
