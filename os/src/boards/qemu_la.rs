//! LoongArch64 QEMU virt machine board configuration.
//!
//! Board constants (CLOCK_FREQ, MEMORY_END, etc.) are provided by the arch crate.
//! This module provides device-level wiring: block device type alias and
//! minimal device_init / irq_handler stubs.

/// Block device implementation on this board (VirtIO-blk over PCI).
pub type BlockDeviceImpl = crate::drivers::block::VirtIOPCIBlock;

use crate::platform::{BoardDevices, IrqController};

/// Stub IRQ controller for LoongArch64 QEMU virt.
pub struct QemuIrqController;

/// Device dispatcher for LoongArch64 QEMU virt.
pub struct QemuBoardDevices;

/// Active platform IRQ controller.
pub type PlatformIrqController = QemuIrqController;

/// Active platform device dispatcher.
pub type PlatformBoardDevices = QemuBoardDevices;

impl IrqController for QemuIrqController {
    fn init() {}

    fn claim() -> Option<u32> {
        warn!("[board] LoongArch64 IRQ handler: external interrupt received but not yet dispatched");
        None
    }

    fn complete(_irq: u32) {}
}

impl BoardDevices for QemuBoardDevices {
    fn init_devices() {
        info!("[board] LoongArch64 QEMU virt: device_init (PCI block device will be probed lazily)");
    }

    fn dispatch_irq(_irq: u32) {
        warn!("[board] LoongArch64 IRQ dispatch is not implemented");
    }
}

/// Initialize board-level devices.
///
/// For LoongArch, the PCI block device is discovered lazily on first access
/// (via the `BLOCK_DEVICE` lazy_static), so there is nothing to do here
/// beyond logging.
#[allow(dead_code)]
pub fn device_init() {
    crate::platform::platform_init();
}

/// Dispatch an external interrupt.
///
/// LoongArch interrupt routing is not yet fully implemented; for now we
/// only log a warning.  The block device uses polling, so missing IRQs
/// are not fatal.
#[allow(dead_code)]
pub fn irq_handler() {
    crate::platform::handle_external_irq();
}
