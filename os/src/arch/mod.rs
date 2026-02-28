//! Architecture-specific implementations
//!
//! This module provides an abstraction layer for different CPU architectures.
//! Currently supported:
//! - RISC-V 64 (riscv64gc)
//! - LoongArch 64

// LoongArch architecture
#[cfg(target_arch = "loongarch64")]
pub mod loongarch64;

#[cfg(target_arch = "loongarch64")]
pub use loongarch64::*;

// RISC-V architecture (using existing sbi module)
#[cfg(target_arch = "riscv64")]
pub use crate::sbi::shutdown;
