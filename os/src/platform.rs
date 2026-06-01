//! Platform-level IRQ and device dispatch glue.

/// External interrupt controller operations for the active board.
pub trait IrqController {
    /// Initialize interrupt routing for the board.
    fn init();
    /// Claim the next pending IRQ, if any.
    fn claim() -> Option<u32>;
    /// Complete a previously claimed IRQ.
    fn complete(irq: u32);
}

/// Device initialization and IRQ dispatch operations for the active board.
pub trait BoardDevices {
    /// Initialize board-level devices.
    fn init_devices();
    /// Dispatch an IRQ number to the corresponding device.
    fn dispatch_irq(irq: u32);
}

/// Initialize platform interrupt routing and board devices.
pub fn platform_init() {
    <crate::board::PlatformIrqController as IrqController>::init();
    <crate::board::PlatformBoardDevices as BoardDevices>::init_devices();
}

/// Handle one external interrupt through the active platform interfaces.
pub fn handle_external_irq() {
    let Some(irq) = <crate::board::PlatformIrqController as IrqController>::claim() else {
        return;
    };
    <crate::board::PlatformBoardDevices as BoardDevices>::dispatch_irq(irq);
    <crate::board::PlatformIrqController as IrqController>::complete(irq);
}
