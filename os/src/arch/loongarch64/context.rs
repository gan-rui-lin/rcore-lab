//! LoongArch64 Trap Context
//!
//! Trap context structure for saving/restoring registers during traps.

/// Trap context structure for LoongArch64
/// Contains all general-purpose registers and exception-related CSRs
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct TrapContext {
    /// General-Purpose Registers $r0-$r31
    /// Note: $r0 is always zero, $r3 is sp, $r2 is tp
    pub x: [usize; 32],
    /// PRMD: Pre-exception Mode Information
    /// bit 1:0 - PLV (Privilege Level): 0=kernel, 3=user
    /// bit 2 - PIE (Previous Interrupt Enable)
    /// bit 3 - PWE (Previous Watch Enable)
    pub prmd: usize,
    /// ERA: Exception Return Address
    pub era: usize,
    /// PGDL: Page Global Directory (page table base)
    pub kernel_satp: usize,
    /// Kernel stack pointer
    pub kernel_sp: usize,
    /// Trap handler entry point address
    pub trap_handler: usize,
}

/// Kernel trap context (for traps from kernel mode)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub(super) struct KernelTrapContext {
    /// General-Purpose Registers
    pub x: [usize; 32],
    /// PRMD register
    pub prmd: usize,
    /// ERA register
    pub era: usize,
}

impl TrapContext {
    /// Set stack pointer (LoongArch uses $r3 as sp)
    pub fn set_sp(&mut self, sp: usize) {
        self.x[3] = sp;
    }

    /// Initialize trap context for a new application
    pub fn app_init_context(
        entry: usize,
        sp: usize,
        kernel_satp: usize,
        kernel_sp: usize,
        trap_handler: usize,
    ) -> Self {
        let mut cx = Self {
            x: [0; 32],
            // PRMD = 0b0111
            // bit 1:0 = 11 (PLV=3, user mode)
            // bit 2 = 1 (PIE=1, interrupts enabled before exception)
            // bit 3 = 1 (PWE=1, watch enabled)
            prmd: 0b0111,
            era: entry,
            kernel_satp,
            kernel_sp,
            trap_handler,
        };
        cx.set_sp(sp);
        cx
    }

    /// Get syscall number (stored in $a7 = $r11)
    #[inline]
    pub fn syscall_number(&self) -> usize {
        self.x[11]
    }

    /// Get syscall arguments ($a0-$a5 = $r4-$r9)
    #[inline]
    pub fn syscall_args(&self) -> [usize; 6] {
        [
            self.x[4],  // $a0
            self.x[5],  // $a1
            self.x[6],  // $a2
            self.x[7],  // $a3
            self.x[8],  // $a4
            self.x[9],  // $a5
        ]
    }

    /// Set syscall return value (in $a0 = $r4)
    #[inline]
    pub fn set_ret(&mut self, ret: isize) {
        self.x[4] = ret as usize;
    }

    /// Advance PC past syscall instruction (syscall is 4 bytes)
    #[inline]
    pub fn syscall_ok(&mut self) {
        self.era += 4;
    }

    /// Get return address ($ra = $r1)
    #[inline]
    pub fn ra(&self) -> usize {
        self.x[1]
    }

    /// Get thread pointer ($tp = $r2)
    #[inline]
    pub fn tp(&self) -> usize {
        self.x[2]
    }

    /// Set thread pointer
    #[inline]
    pub fn set_tp(&mut self, tp: usize) {
        self.x[2] = tp;
    }

    /// Compatibility alias: sepc -> era (for RISC-V compatibility)
    #[inline]
    pub fn sepc(&self) -> usize {
        self.era
    }

    /// Compatibility alias: set sepc -> set era (for RISC-V compatibility)
    #[inline]
    pub fn set_sepc(&mut self, sepc: usize) {
        self.era = sepc;
    }
}
