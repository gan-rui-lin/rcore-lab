#![allow(missing_docs)]

/// LoongArch64 kernel virtual address start (direct-mapped window).
pub const VIRT_ADDR_START: usize = 0x9000_0000_0000_0000;

/// UART MMIO base in direct-mapped window.
pub const UART_BASE: usize = 0x1fe0_01e0 | VIRT_ADDR_START;

/// Fixed virtual address where the signal-return trampoline is mapped.
pub const SIG_RETURN_ADDR: usize = 0x40_0000_0000;

/// Size of the trap frame structure in bytes.
pub const TRAPFRAME_SIZE: usize = core::mem::size_of::<super::context::TrapFrame>();

/// Page size in bytes (4 KiB).
pub const PAGE_SIZE: usize = 0x1000;

/// log2(PAGE_SIZE).
pub const PAGE_SIZE_BITS: usize = 12;
