#![allow(missing_docs)]

use core::ops::{Index, IndexMut};

use crate::TrapFrameArgs;

/// Saved registers when a trap (interrupt or exception) occurs.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct TrapFrame {
    /// General Registers
    pub x: [usize; 32],
    /// Pre-exception Mode information
    pub prmd: usize,
    /// Exception Return Address
    pub sepc: usize,
}

impl TrapFrame {
    #[inline]
    pub fn new() -> Self {
        Self {
            // bit 1:0 PLV
            // bit 2 PIE
            // bit 3 PWE
            prmd: 0b0111,
            ..Default::default()
        }
    }

    /// Put the stack pointer into x[3].
    pub fn set_sp(&mut self, sp: usize) {
        self.x[3] = sp;
    }

    /// Set the global pointer (no-op on LoongArch).
    pub fn set_gp(&mut self, _gp: usize) {}

    /// Read the global pointer (returns 0 on LoongArch).
    pub fn gp(&self) -> usize {
        0
    }

    /// Init the trap context of an application.
    pub fn app_init_context(
        entry: usize,
        sp: usize,
        _kernel_satp: usize,
        _kernel_sp: usize,
        _trap_handler: usize,
    ) -> Self {
        let mut cx = Self::new();
        cx.sepc = entry;
        cx.set_sp(sp);
        cx
    }

    #[inline]
    pub fn syscall_ok(&mut self) {
        self.sepc += 4;
    }

    #[inline]
    pub fn args(&self) -> [usize; 6] {
        [
            self.x[4],
            self.x[5],
            self.x[6],
            self.x[7],
            self.x[8],
            self.x[9],
        ]
    }

    pub fn write_ucontext_gregs(&self, out: &mut [usize; 32]) {
        out[0] = self.sepc;
        out[1..].copy_from_slice(&self.x[1..]);
    }

    pub fn restore_from_ucontext_gregs(&mut self, gregs: &[usize; 32]) {
        self.sepc = gregs[0];
        self.x[1..].copy_from_slice(&gregs[1..]);
    }
}

impl Index<TrapFrameArgs> for TrapFrame {
    type Output = usize;

    fn index(&self, index: TrapFrameArgs) -> &Self::Output {
        match index {
            TrapFrameArgs::SEPC => &self.sepc,
            TrapFrameArgs::RA => &self.x[1],
            TrapFrameArgs::SP => &self.x[3],
            TrapFrameArgs::RET => &self.x[4],
            TrapFrameArgs::ARG0 => &self.x[4],
            TrapFrameArgs::ARG1 => &self.x[5],
            TrapFrameArgs::ARG2 => &self.x[6],
            TrapFrameArgs::TLS => &self.x[2],
            TrapFrameArgs::SYSCALL => &self.x[11],
        }
    }
}

impl IndexMut<TrapFrameArgs> for TrapFrame {
    fn index_mut(&mut self, index: TrapFrameArgs) -> &mut Self::Output {
        match index {
            TrapFrameArgs::SEPC => &mut self.sepc,
            TrapFrameArgs::RA => &mut self.x[1],
            TrapFrameArgs::SP => &mut self.x[3],
            TrapFrameArgs::RET => &mut self.x[4],
            TrapFrameArgs::ARG0 => &mut self.x[4],
            TrapFrameArgs::ARG1 => &mut self.x[5],
            TrapFrameArgs::ARG2 => &mut self.x[6],
            TrapFrameArgs::TLS => &mut self.x[2],
            TrapFrameArgs::SYSCALL => &mut self.x[11],
        }
    }
}
