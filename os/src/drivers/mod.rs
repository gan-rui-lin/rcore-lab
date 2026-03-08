/// Block device drivers.
pub mod block;
/// Bus and DMA helpers.
pub mod bus;

/// Character device drivers (RISC-V only).
#[cfg(target_arch = "riscv64")]
pub mod chardev;
/// Input device drivers (RISC-V only).
#[cfg(target_arch = "riscv64")]
pub mod input;
/// Network device drivers (RISC-V only).
#[cfg(target_arch = "riscv64")]
pub mod net;
/// Platform-level interrupt controller (RISC-V only).
#[cfg(target_arch = "riscv64")]
pub mod plic;

/// Global block device instance.
pub use block::BLOCK_DEVICE;
#[cfg(target_arch = "riscv64")]
pub use bus::*;
#[cfg(target_arch = "riscv64")]
pub use input::*;
