//! Constants in the kernel

#[allow(unused)]

pub use arch::{CLOCK_FREQ, MEMORY_END, MMIO};

/// user app's stack size
pub const USER_STACK_SIZE: usize = 4096 * 128; // 512KB, needed for deep recursion (regex) and glibc
/// fixed user stack top for all architectures
pub const USER_STACK_TOP: usize = 0x8_0000_0000;
/// fixed mmap base top for all architectures
pub const USER_MMAP_TOP: usize = 0x6_0000_0000;
/// kernel stack size
pub const KERNEL_STACK_SIZE: usize = 4096 * 20;
/// kernel heap size
pub const KERNEL_HEAP_SIZE: usize = 0x200_0000;

/// page size : 4KB
pub const PAGE_SIZE: usize = 0x1000;
/// page size bits: 12
pub const PAGE_SIZE_BITS: usize = 0xc;
