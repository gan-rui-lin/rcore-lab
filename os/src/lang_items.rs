//! The panic handler

use arch::shutdown;
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
    shutdown();
}
