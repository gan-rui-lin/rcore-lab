//! RISC-V trap handling — architecture layer.
//!
//! This module provides the low-level trap setup, the `trap.S` trampoline
//! code, and the `trap_return` path.  The **dispatch** logic (syscall
//! routing, signal handling, scheduling) lives in the kernel crate's
//! `os/src/trap/mod.rs` and is invoked via the `TrapContext.trap_handler`
//! function pointer (for user-mode traps) or via the
//! [`ArchInterface::kernel_interrupt`] callback (for kernel-mode traps).

mod context;

use crate::api::ArchInterface;
use crate::TrapType;
use super::mm::address::PAGE_SIZE;
use core::arch::{asm, global_asm};
use riscv::register::{
    mtvec::TrapMode,
    scause::{self, Exception, Interrupt, Trap},
    sie, sscratch, sstatus, stval, stvec,
};

global_asm!(include_str!("trap.S"));

/// The virtual address of the trampoline page (highest page in the VA space).
pub const TRAMPOLINE: usize = usize::MAX - PAGE_SIZE + 1;

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize trap handling: point `stvec` at the kernel-mode trap entry.
pub fn init() {
    set_kernel_trap_entry();
}

/// Point `stvec` at `__alltraps_k` (via TRAMPOLINE offset) for traps
/// taken while in supervisor mode.
fn set_kernel_trap_entry() {
    extern "C" {
        fn __alltraps();
        fn __alltraps_k();
    }
    let __alltraps_k_va = __alltraps_k as usize - __alltraps as usize + TRAMPOLINE;
    unsafe {
        stvec::write(__alltraps_k_va, TrapMode::Direct);
        sscratch::write(trap_from_kernel as usize);
    }
}

/// Point `stvec` at the TRAMPOLINE page for traps from user mode.
fn set_user_trap_entry() {
    unsafe {
        stvec::write(TRAMPOLINE as usize, TrapMode::Direct);
    }
}

// ---------------------------------------------------------------------------
// Timer interrupt enable
// ---------------------------------------------------------------------------

/// Enable the supervisor timer interrupt (sets `sie.STIE`).
pub fn enable_timer_interrupt() {
    unsafe {
        sie::set_stimer();
    }
}

// ---------------------------------------------------------------------------
// Return to user space
// ---------------------------------------------------------------------------

/// Return to user mode.
///
/// This is a **parameterised** entry point: the caller (the kernel) must
/// supply the trap-context pointer and `satp` token, rather than the arch
/// layer querying kernel task state directly.
///
/// # Arguments
/// * `trap_cx_ptr` — virtual address of this task's `TrapContext` (in the
///   trampoline page).
/// * `user_satp` — `satp` token of the user address space.
pub fn trap_return(trap_cx_ptr: usize, user_satp: usize) -> ! {
    unsafe { sstatus::clear_sie(); }
    set_user_trap_entry();
    extern "C" {
        fn __alltraps();
        fn __restore();
    }
    let restore_va = __restore as usize - __alltraps as usize + TRAMPOLINE;
    unsafe {
        asm!(
            "fence.i",
            "jr {restore_va}",
            restore_va = in(reg) restore_va,
            in("a0") trap_cx_ptr,
            in("a1") user_satp,
            options(noreturn)
        );
    }
}

// ---------------------------------------------------------------------------
// Kernel-mode trap handler
// ---------------------------------------------------------------------------

/// Handle a trap taken while already in supervisor mode.
///
/// Classifies the hardware trap into a [`TrapType`] and dispatches it to
/// the kernel via [`ArchInterface::kernel_interrupt`].  Traps that the
/// kernel cannot handle cause a panic.
#[no_mangle]
fn trap_from_kernel(_trap_cx: &context::KernelTrapContext) {
    let scause = scause::read();
    let stval = stval::read();

    let trap_type = match scause.cause() {
        Trap::Interrupt(Interrupt::SupervisorExternal) => TrapType::SupervisorExternal,
        Trap::Interrupt(Interrupt::SupervisorTimer) => TrapType::Time,
        Trap::Exception(Exception::Breakpoint) => TrapType::Breakpoint,
        _ => {
            panic!(
                "Unsupported trap from kernel: {:?}, stval = {:#x}!",
                scause.cause(),
                stval
            );
        }
    };

    crate::api::ArchInterface::kernel_interrupt(trap_type);
}

// ---------------------------------------------------------------------------
// Re-exports
// ---------------------------------------------------------------------------

pub use context::TrapContext;
