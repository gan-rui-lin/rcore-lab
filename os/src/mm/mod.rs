//! Memory management implementation
//!
//! SV39 page-based virtual-memory architecture for RV64 systems, and
//! everything about memory management, like frame allocator, page table,
//! map area and memory set, is implemented here.
//!
//! Every task or process has a memory_set to control its virtual memory.

mod frame_allocator;
mod heap_allocator;
mod memory_set;
use core::sync::atomic::{AtomicBool, Ordering};

pub use arch::{PhysAddr, PhysPageNum, StepByOne, VirtAddr, VirtPageNum, VPNRange};
pub use arch::{PTEFlags, translated_byte_buffer, translated_ref, translated_refmut, translated_str};
pub use arch::{PageTable, PageTableEntry, UserBuffer, UserBufferIterator};
pub use frame_allocator::{
    frame_alloc, frame_alloc_more, frame_allocator_stats, frame_dealloc, FrameTracker,
};
pub use memory_set::remap_test;
pub use memory_set::{kernel_token, MapPermission, MemorySet, KERNEL_SPACE};

static KERNEL_PT_READY: AtomicBool = AtomicBool::new(false);

/// initiate heap allocator, frame allocator and kernel space
pub fn init() {
    heap_allocator::init_heap();
    frame_allocator::init_frame_allocator();
    KERNEL_SPACE.exclusive_access().activate();
    KERNEL_PT_READY.store(true, Ordering::Release);
}

/// Return kernel page-table token only when kernel mappings are ready.
pub fn kernel_page_table_token_if_ready() -> usize {
    if !KERNEL_PT_READY.load(Ordering::Acquire) {
        return 0;
    }
    kernel_token()
}
