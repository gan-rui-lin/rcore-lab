// Board constants CLOCK_FREQ, MEMORY_END, MMIO are now provided by the arch crate.

/// Block device implementation on this board.
pub type BlockDeviceImpl = crate::drivers::block::VirtIOBlock;
#[allow(unused)]
/// Default virtio-gpu horizontal resolution (pixels).
pub const VIRTGPU_XRES: u32 = 1280;
#[allow(unused)]
/// Default virtio-gpu vertical resolution (pixels).
pub const VIRTGPU_YRES: u32 = 800;

use crate::drivers::block::BLOCK_DEVICE;
use crate::drivers::chardev::{CharDevice, UART};
use crate::drivers::plic::{IntrTargetPriority, PLIC};
use crate::drivers::{KEYBOARD_DEVICE, MOUSE_DEVICE};
use crate::platform::{BoardDevices, IrqController};
use arch::DeviceKind;

/// IRQ controller implementation for RISC-V QEMU virt.
pub struct QemuIrqController;

/// Device dispatch implementation for RISC-V QEMU virt.
pub struct QemuBoardDevices;

/// Active platform IRQ controller.
pub type PlatformIrqController = QemuIrqController;

/// Active platform device dispatcher.
pub type PlatformBoardDevices = QemuBoardDevices;

fn plic_base() -> usize {
    arch::platform_config()
        .device(DeviceKind::Plic)
        .and_then(|device| device.mmio_base())
        .expect("PLIC MMIO base missing from platform config")
}

impl IrqController for QemuIrqController {
    fn init() {
        let mut plic = unsafe { PLIC::new(plic_base()) };
        let hart_id: usize = 0;
        let supervisor = IntrTargetPriority::Supervisor;
        let machine = IntrTargetPriority::Machine;
        plic.set_threshold(hart_id, supervisor, 0);
        plic.set_threshold(hart_id, machine, 1);
        for device in arch::platform_config().devices {
            if let Some(intr_src_id) = device.irq {
                plic.enable(hart_id, supervisor, intr_src_id as usize);
                plic.set_priority(intr_src_id as usize, 1);
            }
        }
        arch::enable_supervisor_external();
    }

    fn claim() -> Option<u32> {
        let mut plic = unsafe { PLIC::new(plic_base()) };
        let irq = plic.claim(0, IntrTargetPriority::Supervisor);
        if irq == 0 {
            None
        } else {
            Some(irq)
        }
    }

    fn complete(irq: u32) {
        let mut plic = unsafe { PLIC::new(plic_base()) };
        plic.complete(0, IntrTargetPriority::Supervisor, irq);
    }
}

impl BoardDevices for QemuBoardDevices {
    fn init_devices() {}

    fn dispatch_irq(irq: u32) {
        for device in arch::platform_config().devices {
            if device.irq != Some(irq) {
                continue;
            }
            match device.kind {
                DeviceKind::InputKeyboard => KEYBOARD_DEVICE.handle_irq(),
                DeviceKind::InputMouse => MOUSE_DEVICE.handle_irq(),
                DeviceKind::Block => BLOCK_DEVICE.handle_irq(),
                DeviceKind::Uart => UART.handle_irq(),
                DeviceKind::Net => crate::net::poll_net_if_available(),
                _ => panic!("unsupported IRQ device {:?} {}", device.kind, irq),
            }
            return;
        }
        panic!("unsupported IRQ {}", irq);
    }
}

/// Initialize board-level devices and PLIC routing.
#[allow(dead_code)]
pub fn device_init() {
    crate::platform::platform_init();
}

/// Dispatch a PLIC interrupt to the corresponding device handler.
#[allow(dead_code)]
pub fn irq_handler() {
    crate::platform::handle_external_irq();
}
