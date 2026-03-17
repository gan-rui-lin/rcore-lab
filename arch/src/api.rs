//! Kernel ↔ arch callback interface.
//!
//! The arch crate sometimes needs services that only the kernel can
//! provide (frame allocation, interrupt dispatch).  To avoid a circular
//! crate dependency the kernel implements [`ArchInterface`] and
//! registers the implementation with the `#[crate_interface::impl_interface]`
//! attribute.  The arch crate invokes it through
//! `crate::api::ArchInterface::method_name()`.

use crate::TrapType;

/// Trait that the kernel must implement to service arch-layer callbacks.
#[crate_interface::def_interface]
pub trait ArchInterface {
    /// Init allocator in kernel.
    fn init_allocator();
    /// Init logging in kernel.
    fn init_logging();
    /// Add a memory region into frame allocator.
    fn add_memory_region(start: usize, end: usize);
    /// Kernel main entry.
    fn main(hartid: usize);
    /// Prepare platform drivers.
    fn prepare_drivers();

    /// Dispatch a user-mode trap / interrupt to the kernel.
    ///
    /// Called from the architecture's trap entry after saving registers
    /// and classifying the event into a [`TrapType`].
    fn kernel_interrupt(trap_type: TrapType);

    /// Allocate one physical page frame.
    ///
    /// Returns the raw physical page number (`PhysPageNum.0`).
    /// Used internally by [`PageTable`](crate::PageTable) when mapping
    /// pages that require intermediate page-table nodes.
    fn frame_alloc() -> usize;

    /// Free one physical page frame previously obtained from
    /// [`frame_alloc`](ArchInterface::frame_alloc).
    fn frame_dealloc(ppn: usize);

    /// Return kernel page-table token if it is ready, otherwise 0.
    ///
    /// This allows arch page-table code to share kernel mappings into
    /// per-process page tables without depending on kernel internals.
    fn kernel_page_table_token() -> usize;
}
