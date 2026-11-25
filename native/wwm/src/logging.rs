//! File logger (GUI apps have no console). Writes to `wwm.log` beside the exe.
//!
//! Set `WWM_LOG=debug` (or trace/warn/error/off) to change the level.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

use log::{LevelFilter, Metadata, Record, SetLoggerError};

struct Logger {
    max_level: LevelFilter,
    file: Mutex<Option<File>>,
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.max_level
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let line = format!("[{}] {}\n", record.level(), record.args());
            if let Ok(mut guard) = self.file.lock() {
                if let Some(f) = guard.as_mut() {
                    let _ = f.write_all(line.as_bytes());
                }
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.file.lock() {
            if let Some(f) = guard.as_mut() {
                let _ = f.flush();
            }
        }
    }
}

pub fn init() -> Result<(), SetLoggerError> {
    let filter = match std::env::var("WWM_LOG").ok().as_deref() {
        Some("trace") => LevelFilter::Trace,
        Some("debug") => LevelFilter::Debug,
        Some("warn") => LevelFilter::Warn,
        Some("error") => LevelFilter::Error,
        Some("off") => LevelFilter::Off,
        _ => LevelFilter::Info,
    };

    let file = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("wwm.log")))
        .or_else(|| Some("wwm.log".into()))
        .and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());

    let logger: &'static Logger = Box::leak(Box::new(Logger {
        max_level: filter,
        file: Mutex::new(file),
    }));
    log::set_logger(logger)?;
    log::set_max_level(filter);
    Ok(())
}
