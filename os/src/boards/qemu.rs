// Board constants CLOCK_FREQ, MEMORY_END, MMIO are now provided by the arch crate.

/// Block device implementation on this board.
pub type BlockDeviceImpl = crate::drivers::block::VirtIOBlock;
/// UART device implementation on this board.
pub type CharDeviceImpl = crate::drivers::chardev::NS16550a<VIRT_UART>;

/// PLIC base address on QEMU virt machine.
pub const VIRT_PLIC: usize = 0xC00_0000;
/// UART base address on QEMU virt machine.
pub const VIRT_UART: usize = 0x1000_0000;
/// VirtIO block device base address on QEMU virt machine.
pub const VIRTIO_BLK: usize = 0x1000_1000;
/// VirtIO block device IRQ on QEMU virt machine.
pub const VIRTIO_BLK_IRQ: u32 = 1;
#[allow(unused)]
/// VirtIO net device IRQ on QEMU virt machine.
pub const VIRTIO_NET_IRQ: u32 = 2;
/// UART IRQ on QEMU virt machine.
pub const VIRT_UART_IRQ: u32 = 10;
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

/// IRQ controller implementation for RISC-V QEMU virt.
pub struct QemuIrqController;

/// Device dispatch implementation for RISC-V QEMU virt.
pub struct QemuBoardDevices;

/// Active platform IRQ controller.
pub type PlatformIrqController = QemuIrqController;

/// Active platform device dispatcher.
pub type PlatformBoardDevices = QemuBoardDevices;

impl IrqController for QemuIrqController {
    fn init() {
        let mut plic = unsafe { PLIC::new(VIRT_PLIC) };
        let hart_id: usize = 0;
        let supervisor = IntrTargetPriority::Supervisor;
        let machine = IntrTargetPriority::Machine;
        plic.set_threshold(hart_id, supervisor, 0);
        plic.set_threshold(hart_id, machine, 1);
        for intr_src_id in [VIRTIO_BLK_IRQ, VIRTIO_NET_IRQ, VIRT_UART_IRQ] {
            plic.enable(hart_id, supervisor, intr_src_id as usize);
            plic.set_priority(intr_src_id as usize, 1);
        }
        arch::enable_supervisor_external();
    }

    fn claim() -> Option<u32> {
        let mut plic = unsafe { PLIC::new(VIRT_PLIC) };
        let irq = plic.claim(0, IntrTargetPriority::Supervisor);
        if irq == 0 {
            None
        } else {
            Some(irq)
        }
    }

    fn complete(irq: u32) {
        let mut plic = unsafe { PLIC::new(VIRT_PLIC) };
        plic.complete(0, IntrTargetPriority::Supervisor, irq);
    }
}

impl BoardDevices for QemuBoardDevices {
    fn init_devices() {}

    fn dispatch_irq(irq: u32) {
        match irq {
            5 => KEYBOARD_DEVICE.handle_irq(),
            6 => MOUSE_DEVICE.handle_irq(),
            VIRTIO_BLK_IRQ => BLOCK_DEVICE.handle_irq(),
            VIRT_UART_IRQ => UART.handle_irq(),
            VIRTIO_NET_IRQ => crate::net::poll_net_if_available(),
            _ => panic!("unsupported IRQ {}", irq),
        }
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
