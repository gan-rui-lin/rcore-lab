//! LoongArch64 board-level constants.

#![allow(missing_docs)]

use crate::platform::{
    DeviceDesc, DeviceKind, DeviceTransport, DmaMode, PlatformConfig,
};

/// Timer / stable counter frequency (Hz).
pub const CLOCK_FREQ: usize = 12_500_000;

/// Upper bound of physical memory on QEMU virt machine.
/// QEMU loongarch64 virt RAM base is 0x9000_0000, aligned with run-la.sh -m 1G.
/// Therefore RAM end is 0x9000_0000 + 0x4000_0000 = 0xD000_0000 (1GB).
pub const MEMORY_END: usize = 0xD000_0000;

/// MMIO regions (base, length) that need identity-mapping.
pub const MMIO: &[(usize, usize)] = &[];

const DMW_UC_BASE: usize = 0x8000_0000_0000_0000;

const DEVICES: &[DeviceDesc] = &[DeviceDesc {
    kind: DeviceKind::Block,
    transport: DeviceTransport::Pci,
    irq: None,
}];

static PLATFORM_CONFIG: PlatformConfig = PlatformConfig {
    memory_end: MEMORY_END,
    mmio_regions: MMIO,
    devices: DEVICES,
    dma_mode: DmaMode::DirectMapWindow {
        uncached_base: DMW_UC_BASE,
    },
};

pub fn platform_config() -> &'static PlatformConfig {
    &PLATFORM_CONFIG
}
