//! Implementation of [`PageTableEntry`] and [`PageTable`].
//!
//! Frame allocation is delegated to the kernel via [`crate::pagetable`]
//! callbacks, eliminating the direct dependency on `os::mm::frame_alloc` /
//! `FrameTracker`.

#![allow(missing_docs)]

use super::address::{PhysAddr, PhysPageNum, StepByOne, VirtAddr, VirtPageNum};
use crate::pagetable;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

bitflags! {
    /// Page table entry flags (RISC-V Sv39).
    pub struct PTEFlags: u8 {
        const V = 1 << 0;
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
        const G = 1 << 5;
        const A = 1 << 6;
        const D = 1 << 7;
    }
}

/// A single page table entry.
#[derive(Copy, Clone)]
#[repr(C)]
pub struct PageTableEntry {
    /// Raw bits of the PTE.
    pub bits: usize,
}

impl PageTableEntry {
    /// Create a PTE that maps to `ppn` with the given `flags`.
    pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
        PageTableEntry {
            bits: ppn.0 << 10 | flags.bits as usize,
        }
    }
    /// Create an empty (invalid) PTE.
    pub fn empty() -> Self {
        PageTableEntry { bits: 0 }
    }
    /// Extract the physical page number.
    pub fn ppn(&self) -> PhysPageNum {
        (self.bits >> 10 & ((1usize << 44) - 1)).into()
    }
    /// Extract the flags.
    pub fn flags(&self) -> PTEFlags {
        PTEFlags::from_bits(self.bits as u8).unwrap()
    }
    /// Is this entry valid?
    pub fn is_valid(&self) -> bool {
        (self.flags() & PTEFlags::V) != PTEFlags::empty()
    }
    /// Is the mapped page readable?
    pub fn readable(&self) -> bool {
        (self.flags() & PTEFlags::R) != PTEFlags::empty()
    }
    /// Is the mapped page writable?
    pub fn writable(&self) -> bool {
        (self.flags() & PTEFlags::W) != PTEFlags::empty()
    }
    /// Is the mapped page executable?
    pub fn executable(&self) -> bool {
        (self.flags() & PTEFlags::X) != PTEFlags::empty()
    }
}

// ---------------------------------------------------------------------------
// Frame allocation helpers — delegate to the kernel via crate_interface
// ---------------------------------------------------------------------------

/// Allocate one physical frame via the kernel callback.
fn alloc_frame() -> PhysPageNum {
    let ppn_raw = pagetable::frame_alloc_persist();
    PhysPageNum(ppn_raw)
}

/// Deallocate one physical frame via the kernel callback.
fn dealloc_frame(ppn: PhysPageNum) {
    pagetable::frame_dealloc_persist(ppn.0);
}

/// Clone kernel root mappings into a freshly allocated user page-table root.
///
/// Returns early if kernel page table is not ready yet (token = 0).
fn clone_kernel_root_mappings(dst_root: PhysPageNum) {
    let Some(kernel_root_raw) = pagetable::kernel_root_ppn_if_ready((1usize << 44) - 1) else {
        warn!("kernel page table is not ready yet, skip cloning kernel root mappings");
        return;
    };
    let kernel_root = PhysPageNum::from(kernel_root_raw);
    let dst = dst_root.get_pte_array();
    let src = kernel_root.get_pte_array();
    // Keep user low half free (0..0x100) for process mappings, and only share
    // kernel high-half mappings (0x100..0x1ff), consistent with rustoswhu.
    for i in 0x100..512 {
        if !dst[i].is_valid() && src[i].is_valid() {
            dst[i] = src[i];
        }
    }
}

// ---------------------------------------------------------------------------
// PageTable
// ---------------------------------------------------------------------------

/// A three-level Sv39 page table.
///
/// Owns intermediate page-table nodes allocated through [`alloc_frame`].
/// When the `PageTable` is dropped, all owned frames are deallocated.
pub struct PageTable {
    root_ppn: PhysPageNum,
    /// Physical page numbers of all page-table nodes owned by this table
    /// (including the root).  Leaf data frames are NOT tracked here; the
    /// kernel's `MemorySet` / `MapArea` owns those.
    frames: Vec<PhysPageNum>,
}

impl Drop for PageTable {
    fn drop(&mut self) {
        for &ppn in self.frames.iter() {
            dealloc_frame(ppn);
        }
    }
}

/// Assume that it won't OOM when creating/mapping.
impl PageTable {
    /// Create a new, empty page table (allocates one root frame).
    pub fn new() -> Self {
        let root = alloc_frame();
        // Zero-fill the root page so every PTE starts as invalid.
        root.get_bytes_array().fill(0);
        // Share kernel mappings so trap entry can run without trampoline mapping.
        clone_kernel_root_mappings(root);
        PageTable {
            root_ppn: root,
            frames: vec![root],
        }
    }

    /// Create a *temporary* page table handle from a `satp` token.
    ///
    /// This variant does **not** own any frames (the `frames` vec is empty)
    /// and therefore will **not** deallocate anything on drop.  Use it only
    /// for translating addresses in an existing address space.
    pub fn from_token(satp: usize) -> Self {
        Self {
            root_ppn: PhysPageNum::from(satp & ((1usize << 44) - 1)),
            frames: Vec::new(),
        }
    }

    /// Walk the page table for `vpn`, creating intermediate nodes as needed.
    fn find_pte_create(&mut self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        for (i, idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[*idx];
            if i == 2 {
                result = Some(pte);
                break;
            }
            if !pte.is_valid() {
                let frame = alloc_frame();
                frame.get_bytes_array().fill(0);
                *pte = PageTableEntry::new(frame, PTEFlags::V);
                self.frames.push(frame);
            }
            ppn = pte.ppn();
        }
        result
    }

    /// Walk the page table for `vpn` (read-only — never allocates).
    fn find_pte(&self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        for (i, idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[*idx];
            if i == 2 {
                result = Some(pte);
                break;
            }
            if !pte.is_valid() {
                return None;
            }
            ppn = pte.ppn();
        }
        result
    }

    /// Walk page-table entries and return the first valid leaf entry.
    ///
    /// The returned level is 0/1/2 for Sv39 L2/L1/L0 respectively.
    fn find_leaf_pte(&self, vpn: VirtPageNum) -> Option<(PageTableEntry, usize)> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        for (level, idx) in idxs.iter().enumerate() {
            let pte = ppn.get_pte_array()[*idx];
            if !pte.is_valid() {
                return None;
            }
            if level == 2 || pte.readable() || pte.writable() || pte.executable() {
                return Some((pte, level));
            }
            ppn = pte.ppn();
        }
        None
    }

    /// Map `vpn` to `ppn` with the given `flags`.
    #[allow(unused)]
    pub fn map(&mut self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        let pte = self.find_pte_create(vpn).unwrap();
        // MAP_FIXED may remap already-mapped pages. Clear old mapping if needed.
        if pte.is_valid() {
            *pte = PageTableEntry::empty();
        }
        *pte = PageTableEntry::new(ppn, flags | PTEFlags::V);
    }

    /// Unmap `vpn`.
    #[allow(unused)]
    pub fn unmap(&mut self, vpn: VirtPageNum) {
        let pte = self.find_pte(vpn).unwrap();
        assert!(pte.is_valid(), "vpn {:?} is invalid before unmapping", vpn);
        *pte = PageTableEntry::empty();
    }

    /// Change the flags of an existing mapping.
    #[allow(unused)]
    pub fn change_pte_flags(&mut self, vpn: VirtPageNum, flags: PTEFlags) -> bool {
        if let Some(pte) = self.find_pte(vpn) {
            if pte.is_valid() {
                let ppn = pte.ppn();
                *pte = PageTableEntry::new(ppn, flags);
                return true;
            }
        }
        false
    }

    /// Translate `vpn` to a PTE (if mapped).
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.find_leaf_pte(vpn).map(|(pte, _)| pte)
    }

    /// Translate a virtual address to a physical address (if mapped).
    pub fn translate_va(&self, va: VirtAddr) -> Option<PhysAddr> {
        let vpn = va.floor();
        let va_usize: usize = va.into();
        self.find_leaf_pte(vpn).map(|(pte, level)| {
            let page_bits = match level {
                0 => 30, // 1 GiB leaf at root
                1 => 21, // 2 MiB leaf at middle level
                _ => 12, // 4 KiB leaf at last level
            };
            let page_mask = (1usize << page_bits) - 1;
            let pa_base = (pte.ppn().0 << 12) & !page_mask;
            (pa_base | (va_usize & page_mask)).into()
        })
    }

    /// Get the Sv39 `satp` token for this page table.
    pub fn token(&self) -> usize {
        8usize << 60 | self.root_ppn.0
    }
}

// ---------------------------------------------------------------------------
// Translation utilities (operate on *existing* page tables via token)
// ---------------------------------------------------------------------------

/// Translate a user-space byte buffer into a vector of kernel-accessible slices.
pub fn translated_byte_buffer(token: usize, ptr: *const u8, len: usize) -> Vec<&'static mut [u8]> {
    let page_table = PageTable::from_token(token);
    let mut start = ptr as usize;
    let end = start + len;
    let mut v = Vec::new();
    while start < end {
        let start_va = VirtAddr::from(start);
        let mut vpn = start_va.floor();
        let ppn = page_table.translate(vpn).unwrap().ppn();
        vpn.step();
        let mut end_va: VirtAddr = vpn.into();
        end_va = end_va.min(VirtAddr::from(end));
        if end_va.page_offset() == 0 {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..]);
        } else {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..end_va.page_offset()]);
        }
        start = end_va.into();
    }
    v
}

/// Translate a NUL-terminated user-space string into a `String`.
pub fn translated_str(token: usize, ptr: *const u8) -> String {
    let page_table = PageTable::from_token(token);
    let mut string = String::new();
    let mut va = ptr as usize;
    loop {
        let ch: u8 = *(page_table
            .translate_va(VirtAddr::from(va))
            .unwrap()
            .get_mut());
        if ch == 0 {
            break;
        }
        string.push(ch as char);
        va += 1;
    }
    string
}

/// Translate a user-space pointer and return an immutable reference.
#[allow(unused)]
pub fn translated_ref<T>(token: usize, ptr: *const T) -> &'static T {
    let page_table = PageTable::from_token(token);
    page_table
        .translate_va(VirtAddr::from(ptr as usize))
        .unwrap()
        .get_ref()
}

/// Translate a user-space pointer and return a mutable reference.
pub fn translated_refmut<T>(token: usize, ptr: *mut T) -> &'static mut T {
    let page_table = PageTable::from_token(token);
    let va = ptr as usize;
    page_table
        .translate_va(VirtAddr::from(va))
        .unwrap()
        .get_mut()
}

// ---------------------------------------------------------------------------
// UserBuffer
// ---------------------------------------------------------------------------

/// An abstraction over a buffer passed from user space to kernel space.
pub struct UserBuffer {
    /// A list of kernel-accessible slices that together form the buffer.
    pub buffers: Vec<&'static mut [u8]>,
}

impl UserBuffer {
    /// Construct a `UserBuffer` from pre-translated slices.
    pub fn new(buffers: Vec<&'static mut [u8]>) -> Self {
        Self { buffers }
    }
    /// Total byte length of the buffer.
    pub fn len(&self) -> usize {
        let mut total: usize = 0;
        for b in self.buffers.iter() {
            total += b.len();
        }
        total
    }
}

impl IntoIterator for UserBuffer {
    type Item = *mut u8;
    type IntoIter = UserBufferIterator;
    fn into_iter(self) -> Self::IntoIter {
        UserBufferIterator {
            buffers: self.buffers,
            current_buffer: 0,
            current_idx: 0,
        }
    }
}

/// Byte-level iterator over a [`UserBuffer`].
pub struct UserBufferIterator {
    buffers: Vec<&'static mut [u8]>,
    current_buffer: usize,
    current_idx: usize,
}

impl Iterator for UserBufferIterator {
    type Item = *mut u8;
    fn next(&mut self) -> Option<Self::Item> {
        if self.current_buffer >= self.buffers.len() {
            None
        } else {
            let r = &mut self.buffers[self.current_buffer][self.current_idx] as *mut _;
            if self.current_idx + 1 == self.buffers[self.current_buffer].len() {
                self.current_idx = 0;
                self.current_buffer += 1;
            } else {
                self.current_idx += 1;
            }
            Some(r)
        }
    }
}
