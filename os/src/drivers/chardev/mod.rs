mod ns16550a;

use crate::board::CharDeviceImpl;
use alloc::sync::Arc;
use lazy_static::*;
pub use ns16550a::NS16550a;

/// Character device interface.
pub trait CharDevice {
    /// Initialize the device.
    fn init(&self);
    /// Read a single byte from the device.
    fn read(&self) -> u8;
    /// Write a single byte to the device.
    fn write(&self, ch: u8);
    /// Handle device interrupt.
    fn handle_irq(&self);
}

lazy_static! {
    /// Global UART device instance.
    pub static ref UART: Arc<CharDeviceImpl> = Arc::new(CharDeviceImpl::new());
}
