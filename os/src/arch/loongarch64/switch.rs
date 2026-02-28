//! Task context switching for LoongArch64
//! Based on OSKernel2025-rustoswhu/arch/src/loongarch64/kcontext.rs

use core::arch::asm;

/// Context Switch for LoongArch64
///
/// Save the context of current task and switch to new task.
/// This function matches the signature expected by rcore-lab's task module.
///
/// # Arguments
/// * `current_task_cx_ptr` - Pointer to current task's TaskContext
/// * `next_task_cx_ptr` - Pointer to next task's TaskContext
///
/// # Note
/// TaskContext structure (from task/context.rs):
/// ```
/// #[repr(C)]
/// pub struct TaskContext {
///     ra: usize,      // offset 0
///     sp: usize,      // offset 8
///     s: [usize; 12], // offset 16 (s0-s11)
/// }
/// ```
///
/// LoongArch register mapping:
/// - ra ($r1) - return address
/// - sp ($r3) - stack pointer
/// - s0-s8 ($r23-$r31) - callee-saved registers
/// - s9 ($r22) - callee-saved register
#[naked]
#[no_mangle]
pub unsafe extern "C" fn __switch(
    current_task_cx_ptr: *mut usize,
    next_task_cx_ptr: *const usize,
) {
    asm!(
        // Save current task context
        // Save ra at offset 0
        "st.d    $ra,  $a0, 0*8",
        // Save sp at offset 1
        "st.d    $sp,  $a0, 1*8",
        // Save s0-s8 (r23-r31) at offsets 2-10
        "st.d    $s0,  $a0, 2*8",
        "st.d    $s1,  $a0, 3*8",
        "st.d    $s2,  $a0, 4*8",
        "st.d    $s3,  $a0, 5*8",
        "st.d    $s4,  $a0, 6*8",
        "st.d    $s5,  $a0, 7*8",
        "st.d    $s6,  $a0, 8*8",
        "st.d    $s7,  $a0, 9*8",
        "st.d    $s8,  $a0, 10*8",
        // Save s9 (r22) at offset 11
        "st.d    $s9,  $a0, 11*8",
        // Note: tp ($r2) is not saved/restored in rcore-lab's context switch

        // Restore next task context
        // Restore ra from offset 0
        "ld.d    $ra,  $a1, 0*8",
        // Restore sp from offset 1
        "ld.d    $sp,  $a1, 1*8",
        // Restore s0-s8 from offsets 2-10
        "ld.d    $s0,  $a1, 2*8",
        "ld.d    $s1,  $a1, 3*8",
        "ld.d    $s2,  $a1, 4*8",
        "ld.d    $s3,  $a1, 5*8",
        "ld.d    $s4,  $a1, 6*8",
        "ld.d    $s5,  $a1, 7*8",
        "ld.d    $s6,  $a1, 8*8",
        "ld.d    $s7,  $a1, 9*8",
        "ld.d    $s8,  $a1, 10*8",
        // Restore s9 from offset 11
        "ld.d    $s9,  $a1, 11*8",

        // Return to restored ra
        "ret",
        options(noreturn)
    )
}
