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
}
