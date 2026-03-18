//! Implementation of [`TrapContext`] and [`KernelTrapContext`].

use core::ops::{Index, IndexMut};
use riscv::register::sstatus::{self, SPP, Sstatus};

use crate::TrapFrameArgs;

/// User-mode trap context.
///
/// Saved by `kernelvec/uservec` (in `trap.S`) and restored by `riscv_user_enter`.
/// The layout must match the assembly exactly.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct TrapContext {
    /// General-purpose registers x0-x31.
    ///
    /// NOTE: RV `trap.S` temporarily uses `x[0]` as an internal kernel stack
    /// scratch slot between `riscv_user_enter` and `uservec`, then clears it back
    /// to zero in `uservec`. Rust code must treat `x[0]` as architectural x0.
    pub x: [usize; 32],
    /// Supervisor Status Register.
    pub sstatus: Sstatus,
    /// Supervisor Exception Program Counter.
    pub sepc: usize,
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

    /// Set the global pointer (x3).
    pub fn set_gp(&mut self, gp: usize) {
        self.x[3] = gp;
    }

    /// Read the global pointer (x3).
    pub fn gp(&self) -> usize {
        self.x[3]
    }

    /// Read temporary register t0 (x5).
    pub fn t0(&self) -> usize {
        self.x[5]
    }

    /// Read temporary register t1 (x6).
    pub fn t1(&self) -> usize {
        self.x[6]
    }

    /// Construct the initial trap context for a user application.
    pub fn app_init_context(entry: usize, sp: usize) -> Self {
        let mut sstatus = sstatus::read();
        // Return to User mode after `sret`.
        sstatus.set_spp(SPP::User);
        let mut cx = Self {
            x: [0; 32],
            sstatus,
            sepc: entry,
        };
        cx.set_sp(sp);
        cx
    }

    #[inline]
    pub fn args(&self) -> [usize; 6] {
        [
            self.x[10],
            self.x[11],
            self.x[12],
            self.x[13],
            self.x[14],
            self.x[15],
        ]
    }

    pub fn write_ucontext_gregs(&self, out: &mut [usize; 32]) {
        out[0] = self.sepc;
        out[1..].copy_from_slice(&self.x[1..]);
    }

    pub fn restore_from_ucontext_gregs(&mut self, gregs: &[usize; 32]) {
        self.sepc = gregs[0];
        self.x[1..].copy_from_slice(&gregs[1..]);
        self.x[0] = 0;
    }
}

impl Index<TrapFrameArgs> for TrapContext {
    type Output = usize;

    fn index(&self, index: TrapFrameArgs) -> &Self::Output {
        match index {
            TrapFrameArgs::SEPC => &self.sepc,
            TrapFrameArgs::RA => &self.x[1],
            TrapFrameArgs::SP => &self.x[2],
            TrapFrameArgs::RET => &self.x[10],
            TrapFrameArgs::ARG0 => &self.x[10],
            TrapFrameArgs::ARG1 => &self.x[11],
            TrapFrameArgs::ARG2 => &self.x[12],
            TrapFrameArgs::TLS => &self.x[4],
            TrapFrameArgs::SYSCALL => &self.x[17],
        }
    }
}

impl IndexMut<TrapFrameArgs> for TrapContext {
    fn index_mut(&mut self, index: TrapFrameArgs) -> &mut Self::Output {
        match index {
            TrapFrameArgs::SEPC => &mut self.sepc,
            TrapFrameArgs::RA => &mut self.x[1],
            TrapFrameArgs::SP => &mut self.x[2],
            TrapFrameArgs::RET => &mut self.x[10],
            TrapFrameArgs::ARG0 => &mut self.x[10],
            TrapFrameArgs::ARG1 => &mut self.x[11],
            TrapFrameArgs::ARG2 => &mut self.x[12],
            TrapFrameArgs::TLS => &mut self.x[4],
            TrapFrameArgs::SYSCALL => &mut self.x[17],
        }
    }
}
