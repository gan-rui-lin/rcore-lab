//! RISC-V kernel entry and boot-time helpers.
//!
//! This module includes the assembly entry point (`_start`) and provides
//! low-level initialization routines.  The full `rust_main` boot sequence
//! lives in the kernel crate; here we only expose primitives the kernel
//! needs.

use core::arch::global_asm;

global_asm!(include_str!("entry.asm"));

/// Clear the BSS segment to zero.
///
/// # Safety
/// Must be called exactly once, before any BSS-resident statics are read.
pub fn clear_bss() {
    extern "C" {
        fn sbss();
        fn ebss();
    }
    unsafe {
        core::slice::from_raw_parts_mut(sbss as usize as *mut u8, ebss as usize - sbss as usize)
            .fill(0);
    }
}
