//! LoongArch64 board-level constants.

#![allow(missing_docs)]

/// Timer / stable counter frequency (Hz).
pub const CLOCK_FREQ: usize = 12_500_000;

/// Upper bound of physical memory on QEMU virt machine.
/// QEMU loongarch64 virt RAM base is 0x9000_0000, and run-la.sh defaults to -m 4G.
/// Therefore RAM end is 0x9000_0000 + 0x1_0000_0000 = 0x1_9000_0000.
pub const MEMORY_END: usize = 0x1_9000_0000;

/// MMIO regions (base, length) that need identity-mapping.
pub const MMIO: &[(usize, usize)] = &[];
