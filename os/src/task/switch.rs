//! Context switch re-export for the current arch.
#[cfg(target_arch = "riscv64")]
pub use crate::arch::__switch;
