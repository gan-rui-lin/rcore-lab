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
pub mod sigtrx;
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
pub use entry::kernel_page_table_token;
pub use entry::switch_to_kernel_page_table;
pub use entry::VIRT_ADDR_START;

// ---------------------------------------------------------------------------
// Interrupt control
// ---------------------------------------------------------------------------
pub use interrupt::{disable_interrupts, enable_interrupts, enable_supervisor_external, interrupts_enabled};

// ---------------------------------------------------------------------------
// Memory management
// ---------------------------------------------------------------------------
pub use mm::{PhysAddr, PhysPageNum, VirtAddr, VirtPageNum, VPNRange, StepByOne};
pub use mm::{PageTable, PageTableEntry, PTEFlags};
pub use mm::{
    translated_byte_buffer, translated_byte_buffer_checked, translated_ref, translated_refmut,
    translated_str, translated_str_checked,
};
pub use mm::{UserBuffer, UserBufferIterator};
pub use mm::{PAGE_SIZE, PAGE_SIZE_BITS};
pub use mm::activate_page_table;
pub use mm::init_kernel_page_table;

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

#[inline]
pub unsafe fn switch_to_task(
    idle_task_cx_ptr: *mut TaskContext,
    next_task_cx_ptr: *const TaskContext,
    _pt_token: usize,
) {
    __switch(idle_task_cx_ptr, next_task_cx_ptr);
}

#[inline]
pub unsafe fn switch_to_idle(switched_task_cx_ptr: *mut TaskContext, idle_task_cx_ptr: *mut TaskContext) {
    __switch(switched_task_cx_ptr, idle_task_cx_ptr);
}

// ---------------------------------------------------------------------------
// Timer
// ---------------------------------------------------------------------------
pub use timer::{get_time, get_time_ms, get_time_us, set_next_trigger};

// ---------------------------------------------------------------------------
// Trap handling
// ---------------------------------------------------------------------------
pub use trap::TrapContext;
pub use trap::init as trap_init;
pub use trap::enable_timer_interrupt as trap_enable_timer_interrupt;
pub use trap::enter_user_and_trap;

/// Fixed virtual base where the RISC-V signal-return trampoline page is mapped.
pub const SIG_RETURN_ADDR: usize = 0xFFFF_FFC1_0000_0000;

/// Canonicalize a kernel text/function address to the high-half alias.
#[inline]
pub fn kernel_text_addr(addr: usize) -> usize {
    if addr >= VIRT_ADDR_START {
        addr
    } else {
        addr | VIRT_ADDR_START
    }
}
