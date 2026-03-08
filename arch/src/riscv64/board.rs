//! QEMU virt board constants for RISC-V 64.

/// Timer clock frequency (Hz) — QEMU virt default.
pub const CLOCK_FREQ: usize = 12_500_000;

/// Upper bound of physical memory available to the kernel.
pub const MEMORY_END: usize = 0x8800_0000;

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
