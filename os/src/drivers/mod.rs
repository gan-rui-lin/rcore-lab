/// Block device drivers.
pub mod block;
/// Bus and DMA helpers.
pub mod bus;
/// Character device drivers.
pub mod chardev;
/// Input device drivers.
pub mod input;
/// Network device drivers.
pub mod net;
/// Platform-level interrupt controller.
pub mod plic;

/// Global block device instance.
pub use block::BLOCK_DEVICE;
pub use bus::*;
pub use input::*;
