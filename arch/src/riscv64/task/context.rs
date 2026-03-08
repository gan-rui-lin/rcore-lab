//! Implementation of [`TaskContext`] for RISC-V 64.

/// Kernel-level task context saved/restored across context switches.
///
/// Layout must match `switch.S` exactly:
/// - offset  0: `ra` (return address — where execution resumes)
/// - offset  8: `sp` (kernel stack pointer)
/// - offset 16..112: `s0`..`s11` (callee-saved registers)
#[repr(C)]
pub struct TaskContext {
    /// Return address after task switching.
    ra: usize,
    /// Stack pointer.
    sp: usize,
    /// Callee-saved registers s0-s11.
    s: [usize; 12],
}

impl TaskContext {
    /// Create an all-zero task context (used as a placeholder).
    pub fn zero_init() -> Self {
        Self {
            ra: 0,
            sp: 0,
            s: [0; 12],
        }
    }

    /// Create a task context that will jump to `trap_return` when
    /// switched to for the first time.
    ///
    /// `kstack_ptr` is the top of this task's kernel stack.
    /// `trap_return_addr` is the address of the arch-level `trap_return`
    /// function (typically `trap_return as usize`).
    pub fn goto_trap_return(kstack_ptr: usize, trap_return_addr: usize) -> Self {
        Self {
            ra: trap_return_addr,
            sp: kstack_ptr,
            s: [0; 12],
        }
    }
}
