//! Wrap `switch.S` as a Rust-callable function.

use super::context::TaskContext;
use core::arch::global_asm;

global_asm!(include_str!("switch.S"));

extern "C" {
    /// Perform a context switch from the current task to the next task.
    ///
    /// Saves callee-saved registers into `*current_task_cx_ptr` and
    /// restores them from `*next_task_cx_ptr`, then returns (via the
    /// restored `ra`).
    pub fn __switch(current_task_cx_ptr: *mut TaskContext, next_task_cx_ptr: *const TaskContext);
}
