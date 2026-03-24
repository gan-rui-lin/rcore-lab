#![allow(missing_docs)]

use core::fmt::{self, Debug, Formatter};

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::consts::{VIRT_ADDR_START, PAGE_SIZE, PAGE_SIZE_BITS};
use crate::pagetable;

// ---------------------------------------------------------------------------
// Frame allocation via crate_interface (decoupled from kernel crate)
// ---------------------------------------------------------------------------

/// Allocate a single physical page frame through the kernel callback.
fn arch_frame_alloc() -> PhysPageNum {
    let ppn_raw = pagetable::frame_alloc_persist();
    PhysPageNum(ppn_raw)
}

/// Deallocate a single physical page frame through the kernel callback.
fn arch_frame_dealloc(ppn: PhysPageNum) {
    pagetable::frame_dealloc_persist(ppn.0);
}

bitflags::bitflags! {
    /// page table entry flags (kept compatible with MapPermission bits)
    pub struct PTEFlags: u8 {
        const V = 1 << 0;
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
    }
}

/// LoongArch64 hardware PTE bit positions.
mod la_pte {
    pub const V:   usize = 1 << 0;   // Valid
    pub const D:   usize = 1 << 1;   // Dirty
    pub const PLV: usize = 0b11 << 2; // PLV=3 (user mode)
    pub const MAT: usize = 0b01 << 4; // Coherent Cacheable
    pub const P:   usize = 1 << 7;   // Present (physical page exists)
    pub const W:   usize = 1 << 8;   // Writable
    pub const NR:  usize = 1 << 11;  // Not Readable (unused, kept for reference)
    pub const NX:  usize = 1 << 12;  // Not eXecutable (unused, kept for reference)
}

/// page table entry structure
#[derive(Copy, Clone)]
#[repr(C)]
pub struct PageTableEntry {
    /// bits of page table entry
    pub bits: usize,
}

impl PageTableEntry {
    /// Create a new page table entry
    pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
        // Directory entry: only V flag -> store clean physical address
        // (lddir uses the raw value as base; extra flag bits corrupt it)
        if flags == PTEFlags::V {
            return PageTableEntry { bits: ppn.0 << 12 };
        }
        // Leaf PTE: translate software flags to hardware format
        let mut hw: usize = ppn.0 << 12;
        hw |= la_pte::V | la_pte::P | la_pte::MAT;
        if flags.contains(PTEFlags::W) {
            hw |= la_pte::W | la_pte::D;
        }
        if flags.contains(PTEFlags::U) {
            hw |= la_pte::PLV;
        }
        PageTableEntry { bits: hw }
    }
    /// Create an empty page table entry
    pub fn empty() -> Self {
        PageTableEntry { bits: 0 }
    }
    /// Get the physical page number from the page table entry
    pub fn ppn(&self) -> PhysPageNum {
        ((self.bits >> 12) & ((1usize << PPN_WIDTH_LA) - 1)).into()
    }
    /// Get the flags from the page table entry
    pub fn flags(&self) -> PTEFlags {
        let mut f = PTEFlags::empty();
        if self.bits != 0 { f |= PTEFlags::V; }
        f |= PTEFlags::R; // Always readable (NR not used)
        if self.bits & la_pte::W != 0  { f |= PTEFlags::W; }
        f |= PTEFlags::X; // Always executable (NX not used)
        if self.bits & la_pte::PLV == la_pte::PLV { f |= PTEFlags::U; }
        f
    }
    /// The page pointed by page table entry is valid?
    pub fn is_valid(&self) -> bool {
        self.bits != 0
    }
    /// The page pointed by page table entry is readable?
    pub fn readable(&self) -> bool { self.is_valid() }
    /// The page pointed by page table entry is writable?
    pub fn writable(&self) -> bool { self.bits & la_pte::W != 0 }
    /// The page pointed by page table entry is executable?
    pub fn executable(&self) -> bool { self.is_valid() }
}

/// page table structure
///
/// The `frames` vector tracks page-table node frames allocated through
/// [`ArchInterface::frame_alloc`].  When the PageTable is dropped, all
/// intermediate frames are released back via [`ArchInterface::frame_dealloc`].
pub struct PageTable {
    root_ppn: PhysPageNum,
    frames: Vec<PhysPageNum>,
}

impl Drop for PageTable {
    fn drop(&mut self) {
        for ppn in self.frames.drain(..) {
            arch_frame_dealloc(ppn);
        }
    }
}

impl PageTable {
    pub const PTE_NUM_IN_PAGE: usize = 512;
    pub const PAGE_SIZE: usize = PAGE_SIZE;

    /// Create a new page table
    pub fn new() -> Self {
        let frame = arch_frame_alloc();
        // Zero the root page-table page.
        let bytes = frame.get_bytes_array();
        bytes.fill(0);
        PageTable {
            root_ppn: frame,
            frames: vec![frame],
        }
    }
    /// Temporarily used to get arguments from user space.
    pub fn from_token(token: usize) -> Self {
        Self {
            root_ppn: PhysPageNum::from(token >> 12),
            frames: Vec::new(),
        }
    }
    /// Find PageTableEntry by VirtPageNum, create a frame for a 4KB page table if not exist
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
                let frame = arch_frame_alloc();
                // Zero the newly allocated page-table page.
                frame.get_bytes_array().fill(0);
                *pte = PageTableEntry::new(frame, PTEFlags::V);
                self.frames.push(frame);
            }
            ppn = pte.ppn();
        }
        result
    }
    /// Find PageTableEntry by VirtPageNum
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
    /// set the map between virtual page number and physical page number
    pub fn map(&mut self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        let pte = self.find_pte_create(vpn).unwrap();
        // Allow overwriting existing PTE (needed for MAP_FIXED).
        *pte = PageTableEntry::new(ppn, flags | PTEFlags::V);
    }
    /// remove the map between virtual page number and physical page number
    pub fn unmap(&mut self, vpn: VirtPageNum) {
        let pte = self.find_pte(vpn).unwrap();
        if !pte.is_valid() {
            return; // already unmapped, skip silently
        }
        *pte = PageTableEntry::empty();
    }
    /// change the flags of a page table entry
    pub fn change_pte_flags(&mut self, vpn: VirtPageNum, flags: PTEFlags) -> bool {
        if let Some(pte) = self.find_pte(vpn) {
            if pte.is_valid() {
                let ppn = pte.ppn();
                *pte = PageTableEntry::new(ppn, flags | PTEFlags::V);
                return true;
            }
        }
        false
    }
    /// get the page table entry from the virtual page number
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.find_pte(vpn).map(|pte| *pte)
    }
    /// get the physical address from the virtual address
    pub fn translate_va(&self, va: VirtAddr) -> Option<PhysAddr> {
        self.find_pte(va.clone().floor()).map(|pte| {
            let aligned_pa: PhysAddr = pte.ppn().into();
            let offset = va.page_offset();
            let aligned_pa_usize: usize = aligned_pa.into();
            (aligned_pa_usize + offset).into()
        })
    }
    /// get the token from the page table
    pub fn token(&self) -> usize {
        self.root_ppn.0 << 12
    }
}

/// Translate&Copy a ptr[u8] array with LENGTH len to a mutable u8 Vec through page table
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

/// Translate&Copy a ptr[u8] array end with `\0` to a `String` Vec through page table
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

/// Translate a ptr[u8] array through page table and return a reference of T
pub fn translated_ref<T>(token: usize, ptr: *const T) -> &'static T {
    let page_table = PageTable::from_token(token);
    page_table
        .translate_va(VirtAddr::from(ptr as usize))
        .unwrap()
        .get_ref()
}
/// Translate a ptr[u8] array through page table and return a mutable reference of T
pub fn translated_refmut<T>(token: usize, ptr: *mut T) -> &'static mut T {
    let page_table = PageTable::from_token(token);
    let va = ptr as usize;
    page_table
        .translate_va(VirtAddr::from(va))
        .unwrap()
        .get_mut()
}

/// An abstraction over a buffer passed from user space to kernel space
pub struct UserBuffer {
    /// A list of buffers
    pub buffers: Vec<&'static mut [u8]>,
}

impl UserBuffer {
    /// Construct UserBuffer
    pub fn new(buffers: Vec<&'static mut [u8]>) -> Self {
        Self { buffers }
    }
    /// Get the length of the buffer
    pub fn len(&self) -> usize {
        let mut total: usize = 0;
        for b in self.buffers.iter() {
            total += b.len();
        }
        total
    }
}

pub struct UserBufferIterator {
    buffers: Vec<&'static mut [u8]>,
    cur_buffer: usize,
    cur_pos: usize,
}

impl IntoIterator for UserBuffer {
    type Item = *mut u8;
    type IntoIter = UserBufferIterator;
    fn into_iter(self) -> Self::IntoIter {
        UserBufferIterator {
            buffers: self.buffers,
            cur_buffer: 0,
            cur_pos: 0,
        }
    }
}

impl Iterator for UserBufferIterator {
    type Item = *mut u8;
    fn next(&mut self) -> Option<Self::Item> {
        if self.cur_buffer >= self.buffers.len() {
            return None;
        }
        let result = self.buffers[self.cur_buffer][self.cur_pos..].as_mut_ptr();
        self.cur_pos += 1;
        if self.cur_pos >= self.buffers[self.cur_buffer].len() {
            self.cur_buffer += 1;
            self.cur_pos = 0;
        }
        Some(result)
    }
}

/// Implementation of physical and virtual address and page number.
const PA_WIDTH_LA: usize = 56;
const VA_WIDTH_LA: usize = 39;
const PPN_WIDTH_LA: usize = PA_WIDTH_LA - PAGE_SIZE_BITS;
const VPN_WIDTH_LA: usize = VA_WIDTH_LA - PAGE_SIZE_BITS;

#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PhysAddr(pub usize);

#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct VirtAddr(pub usize);

#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PhysPageNum(pub usize);

#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct VirtPageNum(pub usize);

impl Debug for VirtAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("VA:{:#x}", self.0))
    }
}
impl Debug for VirtPageNum {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("VPN:{:#x}", self.0))
    }
}
impl Debug for PhysAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("PA:{:#x}", self.0))
    }
}
impl Debug for PhysPageNum {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("PPN:{:#x}", self.0))
    }
}

impl From<usize> for PhysAddr {
    fn from(v: usize) -> Self {
        let masked = v & ((1 << PA_WIDTH_LA) - 1);
        let pa = masked & !VIRT_ADDR_START;
        Self(pa)
    }
}
impl From<usize> for PhysPageNum {
    fn from(v: usize) -> Self {
        Self(v & ((1 << PPN_WIDTH_LA) - 1))
    }
}
impl From<usize> for VirtAddr {
    fn from(v: usize) -> Self {
        Self(v)
    }
}
impl From<usize> for VirtPageNum {
    fn from(v: usize) -> Self {
        Self(v)
    }
}
impl From<PhysAddr> for usize {
    fn from(v: PhysAddr) -> Self {
        v.0
    }
}
impl From<PhysPageNum> for usize {
    fn from(v: PhysPageNum) -> Self {
        v.0
    }
}
impl From<VirtAddr> for usize {
    fn from(v: VirtAddr) -> Self {
        v.0
    }
}
impl From<VirtPageNum> for usize {
    fn from(v: VirtPageNum) -> Self {
        v.0
    }
}

impl VirtAddr {
    /// Get the (floor) virtual page number
    pub fn floor(&self) -> VirtPageNum {
        VirtPageNum(self.0 / PAGE_SIZE)
    }

    /// Get the (ceil) virtual page number
    pub fn ceil(&self) -> VirtPageNum {
        VirtPageNum((self.0 - 1 + PAGE_SIZE) / PAGE_SIZE)
    }

    /// Get the page offset of virtual address
    pub fn page_offset(&self) -> usize {
        self.0 & (PAGE_SIZE - 1)
    }

    /// Check if the virtual address is aligned by page size
    pub fn aligned(&self) -> bool {
        self.page_offset() == 0
    }
}

impl From<VirtAddr> for VirtPageNum {
    fn from(v: VirtAddr) -> Self {
        assert_eq!(v.page_offset(), 0);
        v.floor()
    }
}
impl From<VirtPageNum> for VirtAddr {
    fn from(v: VirtPageNum) -> Self {
        Self(v.0 << PAGE_SIZE_BITS)
    }
}

impl PhysAddr {
    /// Get the (floor) physical page number
    pub fn floor(&self) -> PhysPageNum {
        PhysPageNum(self.0 / PAGE_SIZE)
    }
    /// Get the (ceil) physical page number
    pub fn ceil(&self) -> PhysPageNum {
        PhysPageNum((self.0 - 1 + PAGE_SIZE) / PAGE_SIZE)
    }
    /// Get the page offset of physical address
    pub fn page_offset(&self) -> usize {
        self.0 & (PAGE_SIZE - 1)
    }
    /// Check if the physical address is aligned by page size
    pub fn aligned(&self) -> bool {
        self.page_offset() == 0
    }
    /// Get the immutable reference of physical address
    pub fn get_ref<T>(&self) -> &'static T {
        unsafe { ((self.0 | VIRT_ADDR_START) as *const T).as_ref().unwrap() }
    }
    /// Get the mutable reference of physical address
    pub fn get_mut<T>(&self) -> &'static mut T {
        unsafe { ((self.0 | VIRT_ADDR_START) as *mut T).as_mut().unwrap() }
    }
}

impl From<PhysAddr> for PhysPageNum {
    fn from(v: PhysAddr) -> Self {
        assert_eq!(v.page_offset(), 0);
        v.floor()
    }
}
impl From<PhysPageNum> for PhysAddr {
    fn from(v: PhysPageNum) -> Self {
        Self(v.0 << PAGE_SIZE_BITS)
    }
}

impl VirtPageNum {
    /// Get the indexes of the page table entry
    pub fn indexes(&self) -> [usize; 3] {
        let mut vpn = self.0;
        let mut idx = [0usize; 3];
        for i in (0..3).rev() {
            idx[i] = vpn & 511;
            vpn >>= 9;
        }
        idx
    }
}

impl PhysPageNum {
    /// Get the immutable reference of physical page number
    pub fn get_ref<T>(&self) -> &'static T {
        let pa: PhysAddr = (*self).into();
        pa.get_ref::<T>()
    }
    /// Get the mutable reference of physical page number
    pub fn get_mut<T>(&self) -> &'static mut T {
        let pa: PhysAddr = (*self).into();
        pa.get_mut::<T>()
    }
    /// Get the array of bytes through physical page number
    pub fn get_bytes_array(&self) -> &'static mut [u8] {
        let pa: PhysAddr = (*self).into();
        pa.get_mut::<[u8; PAGE_SIZE]>()
    }
    /// Get the array of page table entry through physical page number
    pub fn get_pte_array(&self) -> &'static mut [PageTableEntry] {
        let pa: PhysAddr = (*self).into();
        pa.get_mut::<[PageTableEntry; 512]>()
    }
}

pub trait StepByOne {
    fn step(&mut self);
}

impl StepByOne for VirtPageNum {
    fn step(&mut self) {
        self.0 += 1;
    }
}

#[derive(Clone, Copy)]
pub struct VPNRange {
    l: VirtPageNum,
    r: VirtPageNum,
}

impl VPNRange {
    pub fn new(l: VirtPageNum, r: VirtPageNum) -> Self {
        Self { l, r }
    }
    pub fn get_start(&self) -> VirtPageNum {
        self.l
    }
    pub fn get_end(&self) -> VirtPageNum {
        self.r
    }
}

impl IntoIterator for VPNRange {
    type Item = VirtPageNum;
    type IntoIter = VPNRangeIter;
    fn into_iter(self) -> Self::IntoIter {
        VPNRangeIter { current: self.l, end: self.r }
    }
}

pub struct VPNRangeIter {
    current: VirtPageNum,
    end: VirtPageNum,
}

impl Iterator for VPNRangeIter {
    type Item = VirtPageNum;
    fn next(&mut self) -> Option<Self::Item> {
        if self.current.0 >= self.end.0 {
            None
        } else {
            let t = self.current;
            self.current.step();
            Some(t)
        }
    }
}

/// Change page table by writing pgdl.
pub fn activate_page_table(token: usize) {
    use loongArch64::register::pgdl;

    pgdl::set_base(token);
    unsafe {
        core::arch::asm!("dbar 0; invtlb 0x00, $r0, $r0");
    }
}

/// Set the kernel page table in PGDH (for VA[47]=1 addresses).
/// This must be called once during init so that kernel-space virtual addresses
/// (e.g. kernel stacks at TRAMPOLINE region) can be resolved via the TLB.
pub fn init_kernel_page_table(token: usize) {
    use loongArch64::register::{pgdh, pgdl};

    pgdl::set_base(token);
    pgdh::set_base(token);
    unsafe {
        core::arch::asm!("dbar 0; invtlb 0x00, $r0, $r0");
    }
}
