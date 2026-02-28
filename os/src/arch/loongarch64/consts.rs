//! LoongArch64 architecture constants

/// LoongArch DMW1 virtual address start (cached kernel space)
/// DMW1: 0x9000_0000_0000_0000 ~ 0x9FFF_FFFF_FFFF_FFFF
pub const VIRT_ADDR_START: usize = 0x9000_0000_0000_0000;

/// Page size (4KB)
pub const PAGE_SIZE: usize = 4096;
pub const PAGE_SIZE_BITS: usize = 12;

/// User address space
/// User space: 0x0000_0000_0000_0000 ~ 0x0000_007F_FFFF_FFFF (512GB)
pub const USER_STACK_TOP: usize = 0x0000_0040_0000_0000;

/// Trampoline page (mapped at highest user address)
pub const TRAMPOLINE: usize = USER_STACK_TOP - PAGE_SIZE;

/// Trap context base (one page below trampoline)
pub const TRAP_CONTEXT_BASE: usize = TRAMPOLINE - PAGE_SIZE;

/// Kernel space memory end (512MB after kernel start)
pub const MEMORY_END: usize = VIRT_ADDR_START + 0x2000_0000;

/// Signal return trampoline address
pub const SIG_RETURN_ADDR: usize = 0x40_0000_0000;
