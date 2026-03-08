//! SBI call wrappers.

#![allow(missing_docs)]

#[cfg(target_arch = "riscv64")]
pub use crate::arch::riscv64::sbi::{console_getchar, console_putchar, set_timer, shutdown};

#[cfg(target_arch = "loongarch64")]
pub use crate::arch::loongarch64::sbi::{console_getchar, console_putchar, set_timer, shutdown};
