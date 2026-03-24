//! Global logger with custom SYSCALL level

use core::sync::atomic::{AtomicU8, Ordering};
use log::{Level, LevelFilter, Log, Metadata, Record};

/// Custom log level for syscall tracing
/// 位于 INFO(3) 和 WARN(2) 之间，值为 25 (介于两者之间)
pub const SYSCALL_LEVEL: u8 = 25;

static CUSTOM_LOG_LEVEL: AtomicU8 = AtomicU8::new(0);

/// Check if syscall logging is enabled
#[inline]
pub fn syscall_enabled() -> bool {
    CUSTOM_LOG_LEVEL.load(Ordering::Relaxed) >= SYSCALL_LEVEL
}

/// syscall! macro for system call logging
/// 使用方法: syscall!("sys_write fd={} buf={:#x} len={}", fd, buf, len);
#[macro_export]
macro_rules! syscall {
    ($($arg:tt)*) => {
        if $crate::logging::syscall_enabled() {
            println!("\u{1B}[36m[SYSCALL] {}\u{1B}[0m", format_args!($($arg)*));
        }
    };
}

/// a simple logger
#[allow(dead_code)]
struct SimpleLogger;

impl Log for SimpleLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }
    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let color = match record.level() {
            Level::Error => 31, // Red
            Level::Warn => 93,  // BrightYellow
            Level::Info => 34,  // Blue
            Level::Debug => 32, // Green
            Level::Trace => 90, // BrightBlack
        };
        println!(
            "\u{1B}[{}m[{:>5}] {}\u{1B}[0m",
            color,
            record.level(),
            record.args(),
        );
    }
    fn flush(&self) {}
}

/// initiate logger
#[allow(dead_code)]
pub fn init() {
    static LOGGER: SimpleLogger = SimpleLogger;
    log::set_logger(&LOGGER).unwrap();

    let (level, custom_level) = match option_env!("LOG") {
        Some("OFF") | Some("NONE") => (LevelFilter::Off, 0),
        Some("ERROR") => (LevelFilter::Error, 10),
        Some("WARN") => (LevelFilter::Warn, 20),
        Some("SYSCALL") => (LevelFilter::Info, SYSCALL_LEVEL),  // 新增 SYSCALL 级别
        Some("INFO") => (LevelFilter::Info, 30),
        Some("DEBUG") => (LevelFilter::Debug, 40),
        Some("TRACE") => (LevelFilter::Trace, 50),
        Some(_) => (LevelFilter::Off, 0),
        None => (LevelFilter::Off, 0),
    };

    log::set_max_level(level);
    CUSTOM_LOG_LEVEL.store(custom_level, Ordering::Relaxed);
}
