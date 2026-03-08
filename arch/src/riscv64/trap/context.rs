//! Implementation of [`TrapContext`] and [`KernelTrapContext`].

use riscv::register::sstatus::{self, SPP, Sstatus};

/// User-mode trap context.
///
/// Saved by `__alltraps` (in `trap.S`) into the per-task trampoline page
/// and restored by `__restore`.  The layout must match the assembly exactly.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct TrapContext {
    /// General-purpose registers x0-x31.
    pub x: [usize; 32],
    /// Supervisor Status Register.
    pub sstatus: Sstatus,
    /// Supervisor Exception Program Counter.
    pub sepc: usize,
    /// Token (`satp` value) of the kernel address space.
    pub kernel_satp: usize,
    /// Kernel stack pointer for this task.
    pub kernel_sp: usize,
    /// Virtual address of the trap handler entry point in the kernel.
    pub trap_handler: usize,
}

/// Kernel-mode trap context.
///
/// Used by `__alltraps_k` / `__restore_k` for traps taken while already
/// in supervisor mode (e.g. timer interrupts during syscall processing).
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub(super) struct KernelTrapContext {
    /// General-purpose registers x0-x31.
    pub x: [usize; 32],
    /// Supervisor Status Register.
    pub sstatus: Sstatus,
    /// Supervisor Exception Program Counter.
    pub sepc: usize,
}

impl TrapContext {
    /// Set the stack pointer (x2).
    pub fn set_sp(&mut self, sp: usize) {
        self.x[2] = sp;
    }

    /// Construct the initial trap context for a user application.
    ///
    /// When `__restore` executes with this context it will:
    /// - switch to `kernel_satp`,
    /// - set the user PC to `entry`,
    /// - set the user stack pointer to `sp`,
    /// - and arrange for subsequent traps to jump to `trap_handler`.
    pub fn app_init_context(
        entry: usize,
        sp: usize,
        kernel_satp: usize,
        kernel_sp: usize,
        trap_handler: usize,
    ) -> Self {
        let mut sstatus = sstatus::read();
        // Return to User mode after `sret`.
        sstatus.set_spp(SPP::User);
        let mut cx = Self {
            x: [0; 32],
            sstatus,
            sepc: entry,
            kernel_satp,
            kernel_sp,
            trap_handler,
        };
        cx.set_sp(sp);
        cx
    }
}
