//! Kernel entrypoint with architecture-specific wiring.
#![deny(missing_docs)]
#![deny(warnings)]
#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate log;

extern crate alloc;
extern crate arch;

#[macro_use]
mod console;

#[macro_use]
mod logging;

#[cfg_attr(target_arch = "riscv64", path = "boards/qemu.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "boards/qemu_la.rs")]
mod board;

pub mod config;
/// Device drivers and device manager glue.
pub mod drivers;
/// File system layer (shared across all architectures).
pub mod fs;
pub mod lang_items;
pub mod mm;
/// Network subsystem (smoltcp-based, loopback on all arches, external net on RISC-V).
pub mod net;
pub mod sync;
pub mod syscall;
pub mod task;
pub mod timer;
pub mod trap;

mod boot;

// ---------------------------------------------------------------------------
// DEV_NON_BLOCKING_ACCESS — lives in the kernel, not in the arch crate.
// Used by the virtio block driver to switch between polling / interrupt I/O.
// ---------------------------------------------------------------------------

#[cfg(target_arch = "riscv64")]
use sync::UPIntrFreeCell;

#[cfg(target_arch = "riscv64")]
lazy_static::lazy_static! {
    /// Switch between polling and interrupt-driven block I/O.
    pub static ref DEV_NON_BLOCKING_ACCESS: UPIntrFreeCell<bool> =
        unsafe { UPIntrFreeCell::new(false) };
}

// ---------------------------------------------------------------------------
// ArchInterface implementation — the arch crate calls back into the kernel
// through these methods.
// ---------------------------------------------------------------------------

struct ArchInterfaceImpl;

#[crate_interface::impl_interface]
impl arch::api::ArchInterface for ArchInterfaceImpl {
    fn init_allocator() {}

    fn init_logging() {}

    fn add_memory_region(_start: usize, _end: usize) {}

    fn main(_hartid: usize) {}

    fn prepare_drivers() {}

    fn kernel_interrupt(trap_type: arch::TrapType) {
        crate::trap::kernel_interrupt_dispatch(trap_type);
    }

    fn frame_alloc() -> usize {
        let frame = crate::mm::frame_alloc().unwrap();
        let ppn = frame.ppn.0;
        core::mem::forget(frame);
        ppn
    }

    fn frame_dealloc(ppn: usize) {
        crate::mm::frame_dealloc(arch::PhysPageNum(ppn));
    }

    fn kernel_page_table_token() -> usize {
        crate::mm::kernel_page_table_token_if_ready()
    }
}
