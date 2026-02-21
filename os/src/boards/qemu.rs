/// QEMU virt machine clock frequency (Hz).
pub const CLOCK_FREQ: usize = 12500000;
/// QEMU virt machine physical memory end.
pub const MEMORY_END: usize = 0x8800_0000;

/// QEMU virt machine MMIO ranges.
pub const MMIO: &[(usize, usize)] = &[
    (0x0010_0000, 0x00_2000), // VIRT_TEST/RTC  in virt machine
    (0x2000000, 0x10000),
    (0xc000000, 0x210000), // VIRT_PLIC in virt machine
    (0x10000000, 0x9000),  // VIRT_UART0 with GPU  in virt machine
];

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

/// Initialize board-level devices and PLIC routing.
pub fn device_init() {
    #[allow(unused)]
    use riscv::register::sie;
    let mut plic = unsafe { PLIC::new(VIRT_PLIC) }; // 创建 PLIC 访问器
    let hart_id: usize = 0; // 单核环境使用 hart 0
    let supervisor = IntrTargetPriority::Supervisor;
    let machine = IntrTargetPriority::Machine;
    plic.set_threshold(hart_id, supervisor, 0); // S 态阈值设为 0，放开所有优先级
    plic.set_threshold(hart_id, machine, 1); // M 态阈值提高，避免接管这些中断
    // Enable IRQs for devices that are actually attached.
    for intr_src_id in [VIRTIO_BLK_IRQ, VIRT_UART_IRQ] {
        plic.enable(hart_id, supervisor, intr_src_id as usize); // 让该 IRQ 送达 S 态
        plic.set_priority(intr_src_id as usize, 1); // 设置 IRQ 优先级
    }
    // unsafe {
    //     sie::set_sext(); // 开启 S 态外部中断
    // }
}

/// Dispatch a PLIC interrupt to the corresponding device handler.
pub fn irq_handler() {
    let mut plic = unsafe { PLIC::new(VIRT_PLIC) }; // 创建 PLIC 访问器，处理中断
    let intr_src_id = plic.claim(0, IntrTargetPriority::Supervisor); // 领取 hart 0 的挂起 IRQ
    match intr_src_id { // 分发到具体设备处理函数
        5 => KEYBOARD_DEVICE.handle_irq(), // 键盘输入
        6 => MOUSE_DEVICE.handle_irq(), // 鼠标输入
        VIRTIO_BLK_IRQ => BLOCK_DEVICE.handle_irq(), // virtio 块设备
        VIRT_UART_IRQ => UART.handle_irq(), // 串口控制台
        _ => panic!("unsupported IRQ {}", intr_src_id), // 未知 IRQ
    }
    plic.complete(0, IntrTargetPriority::Supervisor, intr_src_id); // 完成中断，重新使能该 IRQ
}
