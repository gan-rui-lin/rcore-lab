//! LoongArch64 QEMU virt machine board configuration.
//!
//! Board constants (CLOCK_FREQ, MEMORY_END, etc.) are provided by the arch crate.
//! This module provides device-level wiring: block device type alias and
//! minimal device_init / irq_handler stubs.

/// Block device implementation on this board (VirtIO-blk over PCI).
pub type BlockDeviceImpl = crate::drivers::block::VirtIOPCIBlock;

/// Initialize board-level devices.
///
/// For LoongArch, the PCI block device is discovered lazily on first access
/// (via the `BLOCK_DEVICE` lazy_static), so there is nothing to do here
/// beyond logging.
pub fn device_init() {
    info!("[board] LoongArch64 QEMU virt: device_init (PCI block device will be probed lazily)");
}

/// Dispatch an external interrupt.
///
/// LoongArch interrupt routing is not yet fully implemented; for now we
/// only log a warning.  The block device uses polling, so missing IRQs
/// are not fatal.
#[allow(dead_code)]
pub fn irq_handler() {
    warn!("[board] LoongArch64 IRQ handler: external interrupt received but not yet dispatched");
}
