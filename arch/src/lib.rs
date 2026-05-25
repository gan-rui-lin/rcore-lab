//! Architecture abstraction layer for rcore-lab.
//!
//! This crate provides a hardware abstraction layer (HAL) that isolates
//! architecture-specific code from the kernel.  The kernel imports
//! everything through `arch::*` and never needs `#[cfg(target_arch)]`.
//!
//! Each architecture sub-module must export the **same** set of public
//! symbols listed in `current_arch` re-export below.  At link time only
//! one sub-module is compiled thanks to `#[cfg_attr(..., path = ...)]`.
//!
//! The kernel must implement [`api::ArchInterface`] via `crate_interface`
//! so that the arch layer can call back for frame allocation and
//! interrupt dispatch without introducing circular dependencies.

#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_const)]

extern crate alloc;

#[macro_use]
extern crate log;

#[macro_use]
extern crate bitflags;

pub mod api;
pub mod pagetable;
pub mod platform;

pub use platform::*;

// ---------------------------------------------------------------------------
// Shared types used by both architectures and the kernel
// ---------------------------------------------------------------------------

/// Indices for accessing [`TrapContext`] registers in an
/// architecture-agnostic way via `Index` / `IndexMut`.
#[derive(Debug, Clone, Copy)]
pub enum TrapFrameArgs {
    /// Exception / interrupt program counter.
    SEPC,
    /// Return address register.
    RA,
    /// Stack pointer.
    SP,
    /// Syscall return value.
    RET,
    /// First syscall / function argument.
    ARG0,
    /// Second argument.
    ARG1,
    /// Third argument.
    ARG2,
    /// Thread-local storage pointer.
    TLS,
    /// Syscall number register.
    SYSCALL,
}

/// Architecture-independent trap classification.
///
/// Each architecture maps its hardware-specific exception / interrupt
/// codes to one of these variants.
#[derive(Debug, Clone, Copy)]
pub enum TrapType {
    Breakpoint,
    UserEnvCall,
    Time,
    Unknown,
    SupervisorExternal,
    StorePageFault(usize),
    LoadPageFault(usize),
    InstructionPageFault(usize),
    IllegalInstruction(usize),
}

/// Indices for accessing [`TaskContext`] (kernel context) fields.
#[derive(Debug, Clone, Copy)]
pub enum KContextArgs {
    /// Kernel stack pointer.
    KSP,
    /// Kernel thread pointer.
    KTP,
    /// Kernel program counter (return address after context switch).
    KPC,
}

// ---------------------------------------------------------------------------
// Architecture module selection – only one is compiled per target.
// ---------------------------------------------------------------------------

#[cfg_attr(target_arch = "riscv64", path = "riscv64/mod.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "loongarch64/mod.rs")]
mod current_arch;

pub use current_arch::*;
