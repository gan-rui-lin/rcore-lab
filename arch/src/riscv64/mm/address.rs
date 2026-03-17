//! Implementation of physical and virtual address and page number.

use super::page_table::PageTableEntry;
use crate::VIRT_ADDR_START;

/// Page size: 4 KiB.
pub const PAGE_SIZE: usize = 0x1000;
/// log2(PAGE_SIZE) = 12.
pub const PAGE_SIZE_BITS: usize = 0xc;

const PA_WIDTH_SV39: usize = 56;
const VA_WIDTH_SV39: usize = 39;
const PPN_WIDTH_SV39: usize = PA_WIDTH_SV39 - PAGE_SIZE_BITS;
const VPN_WIDTH_SV39: usize = VA_WIDTH_SV39 - PAGE_SIZE_BITS;

/// Physical Address.
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PhysAddr(pub usize);

/// Virtual Address.
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct VirtAddr(pub usize);

/// Physical Page Number (PPN).
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PhysPageNum(pub usize);

/// Virtual Page Number (VPN).
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct VirtPageNum(pub usize);

// ---------------------------------------------------------------------------
// Debug formatting
// ---------------------------------------------------------------------------

use core::fmt::{self, Debug, Formatter};

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

// ---------------------------------------------------------------------------
// Conversions: usize <-> address / page-number types
// ---------------------------------------------------------------------------

impl From<usize> for PhysAddr {
    fn from(v: usize) -> Self {
        let masked = v & ((1 << PA_WIDTH_SV39) - 1);
        Self(masked & !VIRT_ADDR_START)
    }
}
impl From<usize> for PhysPageNum {
    fn from(v: usize) -> Self {
        Self(v & ((1 << PPN_WIDTH_SV39) - 1))
    }
}
impl From<usize> for VirtAddr {
    fn from(v: usize) -> Self {
        Self(v & ((1 << VA_WIDTH_SV39) - 1))
    }
}
impl From<usize> for VirtPageNum {
    fn from(v: usize) -> Self {
        Self(v & ((1 << VPN_WIDTH_SV39) - 1))
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
        // Sign-extend virtual addresses for SV39.
        if v.0 >= (1 << (VA_WIDTH_SV39 - 1)) {
            v.0 | (!((1 << VA_WIDTH_SV39) - 1))
        } else {
            v.0
        }
    }
}
impl From<VirtPageNum> for usize {
    fn from(v: VirtPageNum) -> Self {
        v.0
    }
}

// ---------------------------------------------------------------------------
// VirtAddr helpers
// ---------------------------------------------------------------------------

impl VirtAddr {
    /// Get the floor virtual page number.
    pub fn floor(&self) -> VirtPageNum {
        VirtPageNum(self.0 / PAGE_SIZE)
    }

    /// Get the ceil virtual page number.
    pub fn ceil(&self) -> VirtPageNum {
        VirtPageNum((self.0 - 1 + PAGE_SIZE) / PAGE_SIZE)
    }

    /// Get the page offset of this virtual address.
    pub fn page_offset(&self) -> usize {
        self.0 & (PAGE_SIZE - 1)
    }

    /// Check if the virtual address is page-aligned.
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

// ---------------------------------------------------------------------------
// PhysAddr helpers
// ---------------------------------------------------------------------------

impl PhysAddr {
    /// Get the floor physical page number.
    pub fn floor(&self) -> PhysPageNum {
        PhysPageNum(self.0 / PAGE_SIZE)
    }
    /// Get the ceil physical page number.
    pub fn ceil(&self) -> PhysPageNum {
        PhysPageNum((self.0 - 1 + PAGE_SIZE) / PAGE_SIZE)
    }
    /// Get the page offset of this physical address.
    pub fn page_offset(&self) -> usize {
        self.0 & (PAGE_SIZE - 1)
    }
    /// Check if the physical address is page-aligned.
    pub fn aligned(&self) -> bool {
        self.page_offset() == 0
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

// ---------------------------------------------------------------------------
// VirtPageNum — page-table index decomposition
// ---------------------------------------------------------------------------

impl VirtPageNum {
    /// Decompose the VPN into three 9-bit indices for the SV39 page table.
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

// ---------------------------------------------------------------------------
// Direct memory access helpers (identity-mapped / physical addresses)
// ---------------------------------------------------------------------------

impl PhysAddr {
    /// Get an immutable reference to a value at this physical address.
    pub fn get_ref<T>(&self) -> &'static T {
        unsafe { ((self.0 | VIRT_ADDR_START) as *const T).as_ref().unwrap() }
    }
    /// Get a mutable reference to a value at this physical address.
    pub fn get_mut<T>(&self) -> &'static mut T {
        unsafe { ((self.0 | VIRT_ADDR_START) as *mut T).as_mut().unwrap() }
    }
}

impl PhysPageNum {
    /// Get an immutable reference to a value at the start of this page.
    pub fn get_ref<T>(&self) -> &'static T {
        let pa: PhysAddr = (*self).into();
        pa.get_ref::<T>()
    }
    /// Get a mutable reference to a value at the start of this page.
    pub fn get_mut<T>(&self) -> &'static mut T {
        let pa: PhysAddr = (*self).into();
        pa.get_mut::<T>()
    }
    /// Interpret this physical page as a page-table node (512 PTEs).
    pub fn get_pte_array(&self) -> &'static mut [PageTableEntry] {
        let pa: PhysAddr = (*self).into();
        unsafe {
            core::slice::from_raw_parts_mut((pa.0 | VIRT_ADDR_START) as *mut PageTableEntry, 512)
        }
    }
    /// Interpret this physical page as a raw byte array (4096 bytes).
    pub fn get_bytes_array(&self) -> &'static mut [u8] {
        let pa: PhysAddr = (*self).into();
        unsafe { core::slice::from_raw_parts_mut((pa.0 | VIRT_ADDR_START) as *mut u8, 4096) }
    }
}

// ---------------------------------------------------------------------------
// VPNRange — iterator over a contiguous range of virtual pages
// ---------------------------------------------------------------------------

/// A contiguous range of virtual page numbers `[start, end)`.
#[derive(Copy, Clone, Debug)]
pub struct VPNRange {
    start: VirtPageNum,
    end: VirtPageNum,
}

impl VPNRange {
    /// Create a new VPNRange.  Panics if `start > end`.
    pub fn new(start: VirtPageNum, end: VirtPageNum) -> Self {
        assert!(start <= end);
        Self { start, end }
    }
    /// Get the start VPN.
    pub fn get_start(&self) -> VirtPageNum {
        self.start
    }
    /// Get the (exclusive) end VPN.
    pub fn get_end(&self) -> VirtPageNum {
        self.end
    }
}

/// Required by [`VPNRange`] for step-by-one iteration.
pub trait StepByOne {
    /// Increment by one step.
    fn step(&mut self);
}

impl StepByOne for VirtPageNum {
    fn step(&mut self) {
        self.0 += 1;
    }
}

impl StepByOne for PhysPageNum {
    fn step(&mut self) {
        self.0 += 1;
    }
}

impl Iterator for VPNRange {
    type Item = VirtPageNum;
    fn next(&mut self) -> Option<Self::Item> {
        if self.start >= self.end {
            None
        } else {
            let t = self.start;
            self.start.step();
            Some(t)
        }
    }
}
