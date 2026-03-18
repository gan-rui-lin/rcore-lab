//! LoongArch64 architecture module for the `arch` crate.
//!
//! This module re-exports all public symbols that the kernel depends on
//! through a unified, architecture-agnostic interface.  The shared
//! `TrapFrameArgs`, `TrapType`, and `KContextArgs` enums live in
//! `arch/src/lib.rs` and are NOT duplicated here.

#![allow(missing_docs)]
#![allow(dead_code)]
#![allow(unused_imports)]

pub mod board;
pub mod console;
pub mod consts;
pub mod context;
pub mod entry;
pub mod kcontext;
pub mod page_table;
pub mod sbi;
pub mod sigtrx;
pub mod signal;
pub mod timer;
pub mod trap;
pub mod unaligned;

// ---------------------------------------------------------------------------
// Unified re-exports
// ---------------------------------------------------------------------------

pub use board::{CLOCK_FREQ, MEMORY_END, MMIO};
pub use console::{console_getchar, console_putchar};
pub use entry::clear_bss;
pub use context::TrapFrame;
pub use kcontext::{context_switch, context_switch_pt, read_current_tp, KContext};
pub use page_table::*; // PhysAddr, VirtAddr, PageTable, etc.
pub use page_table::init_kernel_page_table;
pub use sbi::shutdown;
pub use signal::{FpRegs, MContext};
pub use timer::{get_time, get_time_ms, get_time_us, set_next_trigger, init_timer, Time};
pub use trap::{trap_init, trap_enable_timer_interrupt};
pub use trap::{disable_irq as disable_interrupts, enable_irq as enable_interrupts};
pub use trap::{set_kernel_trap, set_kernel_user_rw_trap, set_trap_vector_base};
pub use trap::{try_read_user, try_write_user, enter_user_and_trap};
pub use trap::{disable_irq, enable_irq, enable_external_irq, init_interrupt};
pub use consts::*;

pub use sbi::set_timer;

// ---------------------------------------------------------------------------
// Type aliases matching the unified arch API
// ---------------------------------------------------------------------------

pub type TrapContext = TrapFrame;
pub type TaskContext = KContext;
pub use kcontext::context_switch as __switch;

#[inline]
pub unsafe fn switch_to_task(
    idle_task_cx_ptr: *mut TaskContext,
    next_task_cx_ptr: *const TaskContext,
    pt_token: usize,
) {
    context_switch_pt(idle_task_cx_ptr, next_task_cx_ptr, pt_token);
}

#[inline]
pub unsafe fn switch_to_idle(switched_task_cx_ptr: *mut TaskContext, idle_task_cx_ptr: *mut TaskContext) {
    context_switch(switched_task_cx_ptr, idle_task_cx_ptr);
}

// ---------------------------------------------------------------------------
// Stubs / compat
// ---------------------------------------------------------------------------

/// LoongArch doesn't have async device access support yet.
pub static DEV_NON_BLOCKING_ACCESS: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Check whether interrupts are currently enabled.
pub fn interrupts_enabled() -> bool {
    use loongArch64::register::prmd;
    prmd::read().pie()
}

/// Canonicalize a kernel text/function address.
///
/// LoongArch64 kernel code already runs in canonical high VA space, so this is
/// an identity mapping kept for cross-arch API symmetry.
#[inline]
pub fn kernel_text_addr(addr: usize) -> usize {
    addr
}
