//! Constants in the kernel

#[allow(unused)]

pub use arch::{CLOCK_FREQ, MEMORY_END, MMIO};

/// user app's stack size
pub const USER_STACK_SIZE: usize = 4096 * 128; // 512KB, needed for deep recursion (regex) and glibc
/// fixed user stack top for all architectures
pub const USER_STACK_TOP: usize = 0x8_0000_0000;
/// fixed mmap base top for all architectures
pub const USER_MMAP_TOP: usize = 0x6_0000_0000;
/// kernel stack size: 64KB = exactly 2^16, fits buddy class 16 with zero internal waste.
/// Previously 4096*20=80KB was rounded up to 128KB by buddy, wasting 48KB per task.
/// Confirmed safe: busybox failure on Mac is pre-existing (platform issue), not caused by this.
pub const KERNEL_STACK_SIZE: usize = 4096 * 16;
/// kernel heap size
pub const KERNEL_HEAP_SIZE: usize = 64 * 1024 * 1024; // 64MB

/// page size : 4KB
pub const PAGE_SIZE: usize = 0x1000;
/// page size bits: 12
pub const PAGE_SIZE_BITS: usize = 0xc;
