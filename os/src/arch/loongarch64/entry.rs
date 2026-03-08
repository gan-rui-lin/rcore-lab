//! LoongArch64 minimal kernel entry for early boot.

use core::arch::global_asm;

global_asm!(include_str!("entry.S"));

/// Clear BSS segment.
fn clear_bss() {
    extern "C" {
        fn sbss();
        fn ebss();
    }
    unsafe {
        core::slice::from_raw_parts_mut(sbss as usize as *mut u8, ebss as usize - sbss as usize)
            .fill(0);
    }
}

#[no_mangle]
/// The Rust entry-point of the LoongArch64 kernel.
pub fn rust_main() -> ! {
    clear_bss();
    crate::arch::loongarch64::console::console_init();
    println!("[kernel] loongarch64 boot");
    loop {
        core::hint::spin_loop();
    }
}
