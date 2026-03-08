//! RISC-V signal context layouts.

/// Floating-point register state for signal frames.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FpRegs {
    /// f0-f31 double-precision registers.
    pub f: [u64; 32],
    /// Floating-point control and status register.
    pub fcsr: u32,
    /// Padding to maintain alignment.
    pub _pad: u32,
}

/// Backwards-compatible alias.
pub type RiscvFpRegs = FpRegs;

/// Machine context saved during signal delivery.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct MContext {
    /// General-purpose registers (x0-x31).
    pub gregs: [usize; 32],
    /// Floating-point register state.
    pub fpregs: FpRegs,
}
