//! The main module and entrypoint
//!
//! Various facilities of the kernels are implemented as submodules. The most
//! important ones are:
//!
//! - [`trap`]: Handles all cases of switching from userspace to the kernel
//! - [`task`]: Task management
//! - [`syscall`]: System call handling and implementation
//! - [`mm`]: Address map using SV39
//! - [`sync`]: Wrap a static data structure inside it so that we are able to access it without any `unsafe`.
//! - [`fs`]: Separate user from file system with some structures
//!
//! The operating system also starts in this module. Kernel code starts
//! executing from `entry.asm`, after which [`rust_main()`] is called to
//! initialize various pieces of functionality. (See its source code for
//! details.)
//!
//! We then call [`task::run_tasks()`] and for the first time go to
//! userspace.

#![deny(missing_docs)]
#![deny(warnings)]
#![no_std]
#![no_main]
#![feature(panic_info_message)]
#![feature(alloc_error_handler)]

#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate log;

extern crate alloc;

#[macro_use]
mod console;
#[path = "boards/qemu.rs"]
mod board;
pub mod config;
/// Device drivers and device manager glue.
pub mod drivers;
pub mod fs;
pub mod lang_items;
pub mod logging;
pub mod mm;
pub mod sbi;
pub mod sync;
pub mod syscall;
pub mod task;
pub mod timer;
pub mod trap;

use lazy_static::lazy_static;
use sync::UPIntrFreeCell;

use core::arch::global_asm;

global_asm!(include_str!("entry.asm"));
/// clear BSS segment
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
/// the rust entry-point of os
pub fn rust_main() -> ! {
    clear_bss();
    // info!("[kernel] Hello, world!");
    logging::init();
    mm::init();
    mm::remap_test();
    trap::init();
    trap::enable_timer_interrupt();
    timer::set_next_trigger();
    board::device_init();
    #[cfg(feature = "ext4")]
    if fs::mount_ext4_auto() {
        info!("[kernel] ext4 mounted as root");
        fs::ensure_busybox_links();
    } else if fs::mount_fat32_auto() {
        info!("[kernel] fat32 mounted as root");
    } else {
        fs::mount_easyfs();
    }
    #[cfg(not(feature = "ext4"))]
    if fs::mount_fat32_auto() {
        info!("[kernel] fat32 mounted as root");
    } else {
        fs::mount_easyfs();
    }
    fs::mount_procfs();
    fs::ensure_basic_paths();
    fs::list_apps();
    task::add_initproc();
    *DEV_NON_BLOCKING_ACCESS.exclusive_access() = false;  // Disable non-blocking I/O to avoid Unsupported error
    task::run_tasks();
    panic!("Unreachable in rust_main!");
}
