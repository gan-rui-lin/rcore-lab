//! Constants in the kernel

#[allow(unused)]
pub use arch::{CLOCK_FREQ, MEMORY_END, MMIO};

/// user app's stack size
pub const USER_STACK_SIZE: usize = 4096 * 512; // 2MB, gives enough headroom for deep LTP stacks
/// fixed user stack top for all architectures
pub const USER_STACK_TOP: usize = 0x8_0000_0000;
/// fixed mmap base top for all architectures
pub const USER_MMAP_TOP: usize = 0x6_0000_0000;
/// kernel stack size: 64KB. Fits buddy class 16 with zero internal waste.
/// The 65536-byte UDP recv buffer in socket_file.rs has been moved to heap
/// so 64KB kernel stack is safe.
pub const KERNEL_STACK_SIZE: usize = 4096 * 16;
/// kernel heap size
pub const KERNEL_HEAP_SIZE: usize = 128 * 1024 * 1024; // 128MB

/// page size : 4KB
pub const PAGE_SIZE: usize = 0x1000;
/// page size bits: 12
pub const PAGE_SIZE_BITS: usize = 0xc;
