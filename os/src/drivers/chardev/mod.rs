mod ns16550a;

use alloc::sync::Arc;
use arch::DeviceKind;
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

fn uart_base() -> usize {
    arch::platform_config()
        .device(DeviceKind::Uart)
        .and_then(|device| device.mmio_base())
        .expect("UART MMIO base missing from platform config")
}

lazy_static! {
    /// Global UART device instance.
    pub static ref UART: Arc<NS16550a> = Arc::new(NS16550a::new(uart_base()));
}
