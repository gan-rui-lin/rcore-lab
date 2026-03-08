//! VirtIO HAL implementation for LoongArch64.
//!
//! This implements the `virtio_drivers::Hal` trait (v0.7.1) needed by the
//! new PCI-capable virtio-drivers crate.  LoongArch uses the DMW (Direct
//! Mapping Window) to access physical memory / MMIO, so the phys-to-virt
//! mapping is a simple offset.

use crate::mm::{
    FrameTracker, PhysAddr, PhysPageNum, frame_alloc_more,
    frame_dealloc,
};
use crate::sync::UPIntrFreeCell;
use alloc::vec::Vec;
use core::ptr::NonNull;
use lazy_static::*;
use virtio_drivers_new::{BufferDirection, Hal, PhysAddr as VirtioPhysAddr, PAGE_SIZE};

lazy_static! {
    static ref QUEUE_FRAMES: UPIntrFreeCell<Vec<FrameTracker>> =
        unsafe { UPIntrFreeCell::new(Vec::new()) };
}

/// HAL implementation for virtio-drivers v0.7.1 on LoongArch64.
pub struct VirtioHal;

/// LoongArch64 DMW uncached window base address.
/// Physical address `pa` maps to virtual address `pa | DMW_BASE`.
const DMW_BASE: usize = 0x8000_0000_0000_0000;

unsafe impl Hal for VirtioHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (VirtioPhysAddr, NonNull<u8>) {
        let trackers = frame_alloc_more(pages);
        let ppn_base = trackers.as_ref().unwrap().last().unwrap().ppn;
        QUEUE_FRAMES
            .exclusive_access()
            .append(&mut trackers.unwrap());
        let pa: PhysAddr = ppn_base.into();
        let vaddr = pa.0 | DMW_BASE;
        // Zero the allocated pages
        unsafe {
            core::ptr::write_bytes(vaddr as *mut u8, 0, pages * PAGE_SIZE);
        }
        let vaddr_ptr = NonNull::new(vaddr as *mut u8).unwrap();
        (pa.0, vaddr_ptr)
    }

    unsafe fn dma_dealloc(pa: VirtioPhysAddr, _vaddr: NonNull<u8>, pages: usize) -> i32 {
        let pa = PhysAddr::from(pa);
        let ppn_base: PhysPageNum = pa.into();
        for i in 0..pages {
            frame_dealloc(PhysPageNum(ppn_base.0 + i));
        }
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: VirtioPhysAddr, _size: usize) -> NonNull<u8> {
        let vaddr = paddr | DMW_BASE;
        NonNull::new(vaddr as *mut u8).unwrap()
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> VirtioPhysAddr {
        // LoongArch with DMW: virtual address = physical address | DMW_BASE
        // So physical = virtual & !DMW_BASE
        let vaddr = buffer.as_ptr() as *const u8 as usize;
        vaddr & !DMW_BASE
    }

    unsafe fn unshare(
        _paddr: VirtioPhysAddr,
        _buffer: NonNull<[u8]>,
        _direction: BufferDirection,
    ) {
        // No-op: identity mapping, no bounce buffers needed.
    }
}
