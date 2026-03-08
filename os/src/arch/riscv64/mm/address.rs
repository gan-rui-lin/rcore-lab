//! Implementation of physical and virtual address and page number.
use super::PageTableEntry;
use crate::config::{PAGE_SIZE, PAGE_SIZE_BITS};
use core::fmt::{self, Debug, Formatter};

const PA_WIDTH_SV39: usize = 56;
const VA_WIDTH_SV39: usize = 39;
const PPN_WIDTH_SV39: usize = PA_WIDTH_SV39 - PAGE_SIZE_BITS;
const VPN_WIDTH_SV39: usize = VA_WIDTH_SV39 - PAGE_SIZE_BITS;

/// Definitions
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PhysAddr(pub usize);

/// Virtual Address
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
///virtual address
pub struct VirtAddr(pub usize);

/// Physical Page Number PPN
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
///phiscal page number
pub struct PhysPageNum(pub usize);

/// Virtual Page Number VPN
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct VirtPageNum(pub usize);

/// Debugging

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

/// T: {PhysAddr, VirtAddr, PhysPageNum, VirtPageNum}
/// T -> usize: T.0
/// usize -> T: usize.into()

impl From<usize> for PhysAddr {
    fn from(v: usize) -> Self {
        Self(v & ((1 << PA_WIDTH_SV39) - 1))
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
/// virtual address impl
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

impl PhysAddr {
    /// Get the immutable reference of physical address
    pub fn get_ref<T>(&self) -> &'static T {
        unsafe { (self.0 as *const T).as_ref().unwrap() }
    }
    /// Get the mutable reference of physical address
    pub fn get_mut<T>(&self) -> &'static mut T {
        unsafe { (self.0 as *mut T).as_mut().unwrap() }
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
    /// Get the reference of page table(array of ptes)
    pub fn get_pte_array(&self) -> &'static mut [PageTableEntry] {
        let pa: PhysAddr = (*self).into();
        unsafe { core::slice::from_raw_parts_mut(pa.0 as *mut PageTableEntry, 512) }
    }
    /// Get the reference of page(array of bytes)
    pub fn get_bytes_array(&self) -> &'static mut [u8] {
        let pa: PhysAddr = (*self).into();
        unsafe { core::slice::from_raw_parts_mut(pa.0 as *mut u8, 4096) }
    }
}

/// A range of VPNs
#[derive(Copy, Clone, Debug)]
pub struct VPNRange {
    start: VirtPageNum,
    end: VirtPageNum,
}

impl VPNRange {
    /// Create a new VPNRange from start and end VPNs
    pub fn new(start: VirtPageNum, end: VirtPageNum) -> Self {
        assert!(start <= end);
        Self { start, end }
    }
    /// Get start VPN
    pub fn get_start(&self) -> VirtPageNum {
        self.start
    }
    /// Get end VPN
    pub fn get_end(&self) -> VirtPageNum {
        self.end
    }
}

/// Required by `VPNRange` for step-by-one iteration
pub trait StepByOne {
    /// Increment by one step
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
