#![allow(missing_docs)]

use core::ops::{Index, IndexMut};

use super::TrapFrameArgs;

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
    /// Token of kernel address space (kept for API compatibility).
    pub kernel_satp: usize,
    /// Kernel stack pointer of the current application.
    pub kernel_sp: usize,
    /// Virtual address of trap handler entry point in kernel.
    pub trap_handler: usize,
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

    /// Init the trap context of an application.
    pub fn app_init_context(
        entry: usize,
        sp: usize,
        kernel_satp: usize,
        kernel_sp: usize,
        trap_handler: usize,
    ) -> Self {
        let mut cx = Self::new();
        cx.sepc = entry;
        cx.kernel_satp = kernel_satp;
        cx.kernel_sp = kernel_sp;
        cx.trap_handler = trap_handler;
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
