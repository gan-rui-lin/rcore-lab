//! LoongArch64 minimal kernel entry for early boot.
//!
//! NOTE: `rust_main` is intentionally left as a stub in the arch crate.
//! The actual kernel entry point lives in the `os` crate and is expected
//! to be provided at link time.  The arch crate only supplies the
//! assembly bootstrap (`entry.S`) and BSS clearing.

use core::arch::global_asm;

global_asm!(include_str!("entry.S"));

/// Clear BSS segment.
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
