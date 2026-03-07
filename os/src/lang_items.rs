//! The panic handler

#[cfg(target_arch = "riscv64")]
#[allow(unused_imports)]
use crate::sbi::shutdown;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    if let Some(location) = info.location() {
        println!(
            "[kernel] Panicked at {}:{}",
            location.file(),
            location.line()
        );
    } else {
        println!("[kernel] Panicked");
    }
    #[cfg(target_arch = "riscv64")]
    shutdown();
    #[cfg(target_arch = "loongarch64")]
    loop {
        core::hint::spin_loop();
    }
}
