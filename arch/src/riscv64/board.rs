//! QEMU virt board constants for RISC-V 64.

use crate::platform::{
    DeviceDesc, DeviceKind, DeviceTransport, DmaMode, PlatformConfig,
};

/// Timer clock frequency (Hz) — QEMU virt: aclint-mtimer @ 10MHz.
pub const CLOCK_FREQ: usize = 10_000_000;

/// Upper bound of physical memory available to the kernel.
/// 从 0x8000_0000 开始，到 0xC000_0000 结束，大小为 1GB。
pub const MEMORY_END: usize = 0xC000_0000;

/// Memory-mapped I/O regions that the kernel must identity-map.
pub const MMIO: &[(usize, usize)] = &[
    (0x0010_0000, 0x00_2000), // QEMU virt test device
    (0x200_0000, 0x1_0000),   // CLINT
    (0xC00_0000, 0x21_0000),  // PLIC
    (0x1000_0000, 0x9000),    // UART + VirtIO
];

/// Platform-Level Interrupt Controller base address.
pub const VIRT_PLIC: usize = 0xC00_0000;

/// UART (16550-compatible) base address.
pub const VIRT_UART: usize = 0x1000_0000;

/// VirtIO block device base address.
pub const VIRTIO_BLK: usize = 0x1000_1000;

/// VirtIO net device IRQ on QEMU virt machine.
pub const VIRTIO_NET_IRQ: u32 = 2;

/// UART IRQ on QEMU virt machine.
pub const VIRT_UART_IRQ: u32 = 10;

const DEVICES: &[DeviceDesc] = &[
    DeviceDesc {
        kind: DeviceKind::Test,
        transport: DeviceTransport::Mmio {
            base: 0x0010_0000,
            size: 0x2000,
        },
        irq: None,
    },
    DeviceDesc {
        kind: DeviceKind::Plic,
        transport: DeviceTransport::Mmio {
            base: VIRT_PLIC,
            size: 0x21_0000,
        },
        irq: None,
    },
    DeviceDesc {
        kind: DeviceKind::Uart,
        transport: DeviceTransport::Mmio {
            base: VIRT_UART,
            size: 0x100,
        },
        irq: Some(VIRT_UART_IRQ),
    },
    DeviceDesc {
        kind: DeviceKind::Block,
        transport: DeviceTransport::Mmio {
            base: VIRTIO_BLK,
            size: 0x1000,
        },
        irq: Some(1),
    },
    DeviceDesc {
        kind: DeviceKind::Net,
        transport: DeviceTransport::Mmio {
            base: VIRTIO_BLK + 0x1000,
            size: 0x1000,
        },
        irq: Some(VIRTIO_NET_IRQ),
    },
    DeviceDesc {
        kind: DeviceKind::InputKeyboard,
        transport: DeviceTransport::Mmio {
            base: VIRTIO_BLK + 0x4000,
            size: 0x1000,
        },
        irq: Some(5),
    },
    DeviceDesc {
        kind: DeviceKind::InputMouse,
        transport: DeviceTransport::Mmio {
            base: VIRTIO_BLK + 0x5000,
            size: 0x1000,
        },
        irq: Some(6),
    },
];

static PLATFORM_CONFIG: PlatformConfig = PlatformConfig {
    memory_end: MEMORY_END,
    mmio_regions: MMIO,
    devices: DEVICES,
    dma_mode: DmaMode::KernelPageTableTranslate,
};

/// Return the static platform description for the current target.
pub fn platform_config() -> &'static PlatformConfig {
    &PLATFORM_CONFIG
}
