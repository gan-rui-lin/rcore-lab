//! Global logger

use log::{Level, LevelFilter, Log, Metadata, Record};

/// a simple logger
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
pub fn init() {
    static LOGGER: SimpleLogger = SimpleLogger;
    log::set_logger(&LOGGER).unwrap();
    let level = match option_env!("LOG") {
        Some("OFF") => Some(LevelFilter::Off),
        Some("NONE") => Some(LevelFilter::Off),
        Some("ERROR") => Some(LevelFilter::Error),
        Some("WARN") => Some(LevelFilter::Warn),
        Some("INFO") => Some(LevelFilter::Info),
        Some("DEBUG") => Some(LevelFilter::Debug),
        Some("TRACE") => Some(LevelFilter::Trace),
        Some(_) => Some(LevelFilter::Info),
        None => Some(LevelFilter::Off),
    };
    if let Some(level) = level {
        log::set_max_level(level);
        return;
    }
}
