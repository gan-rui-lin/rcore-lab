//! RISC-V 64-bit architecture module.
//!
//! This is the top-level module for the `riscv64` target.  It aggregates
//! all architecture-specific sub-modules and re-exports the symbols that
//! the kernel expects to find via `arch::*`.

pub mod board;
pub mod console;
pub mod entry;
pub mod interrupt;
pub mod mm;
pub mod sbi;
pub mod signal;
pub mod task;
pub mod timer;
pub mod trap;

// ---------------------------------------------------------------------------
// Board constants
// ---------------------------------------------------------------------------
pub use board::{CLOCK_FREQ, MEMORY_END, MMIO};
pub use board::{VIRT_PLIC, VIRT_UART, VIRTIO_BLK};

// ---------------------------------------------------------------------------
// Console I/O
// ---------------------------------------------------------------------------
pub use console::{console_getchar, console_putchar};

// ---------------------------------------------------------------------------
// Entry / boot helpers
// ---------------------------------------------------------------------------
pub use entry::clear_bss;

// ---------------------------------------------------------------------------
// Interrupt control
// ---------------------------------------------------------------------------
pub use interrupt::{disable_interrupts, enable_interrupts, enable_supervisor_external, interrupts_enabled};

// ---------------------------------------------------------------------------
// Memory management
// ---------------------------------------------------------------------------
pub use mm::{PhysAddr, PhysPageNum, VirtAddr, VirtPageNum, VPNRange, StepByOne};
pub use mm::{PageTable, PageTableEntry, PTEFlags};
pub use mm::{translated_byte_buffer, translated_ref, translated_refmut, translated_str};
pub use mm::{UserBuffer, UserBufferIterator};
pub use mm::{PAGE_SIZE, PAGE_SIZE_BITS};
pub use mm::activate_page_table;

// ---------------------------------------------------------------------------
// SBI
// ---------------------------------------------------------------------------
pub use sbi::{set_timer, shutdown};

// ---------------------------------------------------------------------------
// Signal context
// ---------------------------------------------------------------------------
pub use signal::{FpRegs, RiscvFpRegs, MContext};

// ---------------------------------------------------------------------------
// Task context & switch
// ---------------------------------------------------------------------------
pub use task::{TaskContext, __switch};

// ---------------------------------------------------------------------------
// Timer
// ---------------------------------------------------------------------------
pub use timer::{get_time, get_time_ms, get_time_us, set_next_trigger};

// ---------------------------------------------------------------------------
// Trap handling
// ---------------------------------------------------------------------------
pub use trap::TrapContext;
pub use trap::TRAMPOLINE;
pub use trap::init as trap_init;
pub use trap::enable_timer_interrupt as trap_enable_timer_interrupt;
pub use trap::trap_return;
