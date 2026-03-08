//! RISC-V trap implementation is hosted under arch/.

#![allow(missing_docs)]

#[cfg(target_arch = "riscv64")]
#[doc(inline)]
pub use crate::arch::riscv64::trap::*;

#[cfg(target_arch = "loongarch64")]
#[doc(inline)]
pub use crate::arch::loongarch64::trap::*;
