/// Block device drivers.
pub mod block;
/// Bus and DMA helpers.
pub mod bus;

/// Character device drivers (RISC-V only).
#[cfg_attr(target_arch = "loongarch64", path = "chardev_stub.rs")]
pub mod chardev;
/// Input device drivers (RISC-V only).
#[cfg_attr(target_arch = "loongarch64", path = "input_stub.rs")]
pub mod input;
/// Network device drivers (RISC-V only).
#[cfg_attr(target_arch = "loongarch64", path = "net_stub.rs")]
pub mod net;
/// Platform-level interrupt controller (RISC-V only).
#[cfg_attr(target_arch = "loongarch64", path = "plic_stub.rs")]
pub mod plic;

/// Global block device instance.
pub use block::BLOCK_DEVICE;
pub use bus::*;
pub use input::*;
