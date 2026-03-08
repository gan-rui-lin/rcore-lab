//! Kernel boot sequence.
//!
//! `rust_main` is the first Rust function called from the assembly entry
//! point (`_start` in `arch/src/riscv64/entry.asm` or the LoongArch64
//! equivalent).  It initializes all kernel subsystems and enters the
//! scheduler loop.

/// The Rust entry-point of the kernel (RISC-V 64).
#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn rust_main() -> ! {
    arch::clear_bss();
    crate::logging::init();
    crate::mm::init();
    crate::mm::remap_test();
    arch::trap_init();
    arch::trap_enable_timer_interrupt();
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
    #[cfg(target_arch = "riscv64")]
    {
        *crate::DEV_NON_BLOCKING_ACCESS.exclusive_access() = false;
    }
    crate::task::run_tasks();
    panic!("Unreachable in rust_main!");
}

/// The Rust entry-point of the kernel (LoongArch64).
#[cfg(target_arch = "loongarch64")]
#[no_mangle]
pub fn rust_main() -> ! {
    arch::clear_bss();
    crate::logging::init();
    crate::mm::init();
    arch::trap_init();
    arch::trap_enable_timer_interrupt();
    crate::timer::set_next_trigger();
    crate::task::add_initproc();
    crate::task::run_tasks();
    panic!("Unreachable in rust_main!");
}
