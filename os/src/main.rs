//! Kernel entrypoint with architecture-specific wiring.
#![deny(missing_docs)]
#![deny(warnings)]
#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

#[cfg(target_arch = "riscv64")]
#[macro_use]
extern crate bitflags;
#[cfg(target_arch = "riscv64")]
#[macro_use]
extern crate log;

#[cfg(target_arch = "riscv64")]
extern crate alloc;

#[macro_use]
mod console;

#[cfg(target_arch = "riscv64")]
#[macro_use]
mod logging;

#[cfg(target_arch = "riscv64")]
#[path = "boards/qemu.rs"]
mod board;

#[cfg(target_arch = "riscv64")]
pub mod config;
/// Device drivers and device manager glue.
#[cfg(target_arch = "riscv64")]
pub mod drivers;
#[cfg(target_arch = "riscv64")]
pub mod fs;
pub mod lang_items;
#[cfg(target_arch = "riscv64")]
pub mod mm;
#[cfg(target_arch = "riscv64")]
/// Network subsystem (smoltcp-based TCP/IP stack).
pub mod net;
pub mod sbi;
#[cfg(target_arch = "riscv64")]
pub mod sync;
#[cfg(target_arch = "riscv64")]
pub mod syscall;
#[cfg(target_arch = "riscv64")]
pub mod task;
#[cfg(target_arch = "riscv64")]
pub mod timer;
#[cfg(target_arch = "riscv64")]
pub mod trap;

mod arch;

#[cfg(target_arch = "riscv64")]
pub use arch::riscv64::DEV_NON_BLOCKING_ACCESS;
