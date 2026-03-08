#![allow(missing_docs)]

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RiscvFpRegs {
    pub f: [u64; 32],
    pub fcsr: u32,
    pub _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MContext {
    pub gregs: [usize; 32],
    pub fpregs: RiscvFpRegs,
}
