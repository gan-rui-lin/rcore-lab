//! Kernel entrypoint with architecture-specific wiring.
#![deny(missing_docs)]
#![deny(warnings)]
#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![cfg_attr(target_arch = "loongarch64", feature(naked_functions))]
#![cfg_attr(target_arch = "loongarch64", feature(asm_const))]

#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
#[macro_use]
extern crate bitflags;
#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
#[macro_use]
extern crate log;

#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
extern crate alloc;

#[macro_use]
mod console;

#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
#[macro_use]
mod logging;

#[cfg(target_arch = "riscv64")]
#[path = "boards/qemu.rs"]
mod board;

#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
pub mod config;
/// Device drivers and device manager glue.
#[cfg(target_arch = "riscv64")]
pub mod drivers;
#[cfg(target_arch = "riscv64")]
pub mod fs;
#[cfg(target_arch = "loongarch64")]
#[path = "fs_stub.rs"]
pub mod fs;
pub mod lang_items;
#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
pub mod mm;
#[cfg(target_arch = "riscv64")]
/// Network subsystem (smoltcp-based TCP/IP stack).
pub mod net;
#[cfg(target_arch = "loongarch64")]
#[path = "net_stub.rs"]
pub mod net;
pub mod sbi;
#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
pub mod sync;
#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
pub mod syscall;
#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
pub mod task;
#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
pub mod timer;
#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
pub mod trap;

mod arch;

#[cfg(target_arch = "riscv64")]
pub use arch::riscv64::DEV_NON_BLOCKING_ACCESS;
