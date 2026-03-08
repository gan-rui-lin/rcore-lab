//! LoongArch64 board-level constants.

#![allow(missing_docs)]

/// Timer / stable counter frequency (Hz).
pub const CLOCK_FREQ: usize = 12_500_000;

/// Upper bound of physical memory on QEMU virt machine.
pub const MEMORY_END: usize = 0xB000_0000;

/// MMIO regions (base, length) that need identity-mapping.
/// LoongArch QEMU virt currently has none that the kernel maps explicitly.
pub const MMIO: &[(usize, usize)] = &[];
