//! Kernel object logging.
//!
//! The logger is intentionally small: it writes directly to the kernel serial
//! console, uses an atomic runtime threshold, and formats messages without heap
//! allocation.

use core::fmt::{self, Write};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::kernel_lowlevel::serial::Serial;

include!("log_logic_shared.rs");

#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warning = 2,
    Err = 3,
    Fatal = 4,
}

impl LogLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warning => "warning",
            LogLevel::Err => "err",
            LogLevel::Fatal => "fatal",
        }
    }

    const fn tag(self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warning => "WARNING",
            LogLevel::Err => "ERR",
            LogLevel::Fatal => "FATAL",
        }
    }
}

static LOG_LEVEL: AtomicUsize = AtomicUsize::new(LogLevel::Info as usize);

fn level_from_raw(value: usize) -> LogLevel {
    smros_log_level_from_raw_body!(
        value,
        LogLevel::Debug as usize,
        LogLevel::Debug,
        LogLevel::Warning as usize,
        LogLevel::Warning,
        LogLevel::Err as usize,
        LogLevel::Err,
        LogLevel::Fatal as usize,
        LogLevel::Fatal,
        LogLevel::Info
    )
}

fn should_log_at(level: LogLevel, threshold: LogLevel) -> bool {
    smros_log_should_log_body!(level as usize, threshold as usize)
}

struct SerialLogWriter {
    serial: Serial,
}

impl SerialLogWriter {
    fn new() -> Self {
        Self {
            serial: Serial::active(),
        }
    }

    fn write_prefix(&mut self, level: LogLevel, target: &str) {
        self.serial.write_str("[");
        self.serial.write_str(level.tag());
        self.serial.write_str("][KOBJ");
        if !target.is_empty() {
            self.serial.write_str(":");
            self.serial.write_str(target);
        }
        self.serial.write_str("] ");
    }
}

impl Write for SerialLogWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.serial.write_str(s);
        Ok(())
    }
}

pub fn set_level(level: LogLevel) {
    LOG_LEVEL.store(level as usize, Ordering::Release);
}

pub fn level() -> LogLevel {
    level_from_raw(LOG_LEVEL.load(Ordering::Acquire))
}

pub fn level_from_str(value: &str) -> Option<LogLevel> {
    smros_log_level_from_match_flags_body!(
        value.eq_ignore_ascii_case("debug"),
        value.eq_ignore_ascii_case("info"),
        value.eq_ignore_ascii_case("warning"),
        value.eq_ignore_ascii_case("warn"),
        value.eq_ignore_ascii_case("err"),
        value.eq_ignore_ascii_case("error"),
        value.eq_ignore_ascii_case("fatal"),
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warning,
        LogLevel::Err,
        LogLevel::Fatal
    )
}

pub fn should_log(level: LogLevel) -> bool {
    should_log_at(level, self::level())
}

pub fn log(level: LogLevel, target: &str, message: &str) {
    log_fmt(level, target, format_args!("{}", message));
}

pub fn debug(target: &str, message: &str) {
    log(LogLevel::Debug, target, message);
}

pub fn info(target: &str, message: &str) {
    log(LogLevel::Info, target, message);
}

pub fn warning(target: &str, message: &str) {
    log(LogLevel::Warning, target, message);
}

pub fn err(target: &str, message: &str) {
    log(LogLevel::Err, target, message);
}

pub fn fatal(target: &str, message: &str) {
    log(LogLevel::Fatal, target, message);
}

pub fn log_fmt(level: LogLevel, target: &str, args: fmt::Arguments<'_>) {
    if !should_log(level) {
        return;
    }

    let mut writer = SerialLogWriter::new();
    writer.write_prefix(level, target);
    let _ = writer.write_fmt(args);
    writer.serial.write_str("\n");
}

#[macro_export]
macro_rules! kobj_log {
    ($level:expr, $target:expr, $($arg:tt)+) => {{
        $crate::kernel_objects::log::log_fmt($level, $target, format_args!($($arg)+));
    }};
}

#[macro_export]
macro_rules! kobj_debug {
    ($target:expr, $($arg:tt)+) => {{
        $crate::kobj_log!($crate::kernel_objects::log::LogLevel::Debug, $target, $($arg)+);
    }};
}

#[macro_export]
macro_rules! kobj_info {
    ($target:expr, $($arg:tt)+) => {{
        $crate::kobj_log!($crate::kernel_objects::log::LogLevel::Info, $target, $($arg)+);
    }};
}

#[macro_export]
macro_rules! kobj_warning {
    ($target:expr, $($arg:tt)+) => {{
        $crate::kobj_log!($crate::kernel_objects::log::LogLevel::Warning, $target, $($arg)+);
    }};
}

#[macro_export]
macro_rules! kobj_err {
    ($target:expr, $($arg:tt)+) => {{
        $crate::kobj_log!($crate::kernel_objects::log::LogLevel::Err, $target, $($arg)+);
    }};
}

#[macro_export]
macro_rules! kobj_fatal {
    ($target:expr, $($arg:tt)+) => {{
        $crate::kobj_log!($crate::kernel_objects::log::LogLevel::Fatal, $target, $($arg)+);
    }};
}
