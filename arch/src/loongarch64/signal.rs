#![allow(missing_docs)]

/// Floating-point register save area for LoongArch64.
///
/// Note: the original code named this `RiscvFpRegs` by mistake.
/// The canonical name is now `FpRegs`; a type alias is kept for
/// backward compatibility.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FpRegs {
    pub f: [u64; 32],
    pub fcsr: u32,
    pub _pad: u32,
}

/// Backward-compatible alias.
pub type RiscvFpRegs = FpRegs;

/// Machine context saved/restored across signal delivery.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MContext {
    pub gregs: [usize; 32],
    pub fpregs: FpRegs,
}
