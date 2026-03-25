//! LoongArch64 board-level constants.

#![allow(missing_docs)]

/// Timer / stable counter frequency (Hz).
pub const CLOCK_FREQ: usize = 12_500_000;

/// Upper bound of physical memory on QEMU virt machine.
/// QEMU loongarch64 virt RAM base is 0x9000_0000, aligned with run-la.sh -m 1G.
/// Therefore RAM end is 0x9000_0000 + 0x4000_0000 = 0xD000_0000 (1GB).
pub const MEMORY_END: usize = 0xD000_0000;

/// MMIO regions (base, length) that need identity-mapping.
pub const MMIO: &[(usize, usize)] = &[];
