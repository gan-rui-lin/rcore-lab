//! RISC-V kernel entry and initialization flow.

use crate::sync::UPIntrFreeCell;
use core::arch::global_asm;
use lazy_static::lazy_static;

global_asm!(include_str!("entry.asm"));

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

lazy_static! {
    /// Switch between polling and interrupt-driven block I/O.
    pub static ref DEV_NON_BLOCKING_ACCESS: UPIntrFreeCell<bool> =
        unsafe { UPIntrFreeCell::new(false) };
}

#[no_mangle]
/// The Rust entry-point of the kernel.
pub fn rust_main() -> ! {
    clear_bss();
    crate::logging::init();
    crate::mm::init();
    crate::mm::remap_test();
    crate::arch::trap_init();
    crate::arch::trap_enable_timer_interrupt();
    crate::timer::set_next_trigger();
    crate::board::device_init();
    #[cfg(feature = "ext4")]
    if crate::fs::mount_ext4_auto() {
        info!("[kernel] ext4 mounted as root");
        crate::fs::ensure_busybox_links();
    } else if crate::fs::mount_fat32_auto() {
        info!("[kernel] fat32 mounted as root");
    } else {
        crate::fs::mount_easyfs();
    }
    #[cfg(not(feature = "ext4"))]
    if crate::fs::mount_fat32_auto() {
        info!("[kernel] fat32 mounted as root");
    } else {
        crate::fs::mount_easyfs();
    }
    crate::fs::mount_procfs();
    crate::fs::ensure_basic_paths();
    crate::fs::list_apps();
    crate::net::init();
    crate::task::add_initproc();
    *DEV_NON_BLOCKING_ACCESS.exclusive_access() = false;
    crate::task::run_tasks();
    panic!("Unreachable in rust_main!");
}
