//! A small file logger.
//!
//! A background app has no terminal to print to, so a diagnosable failure needs a file.
//! The pieces that can be wrong - the timestamp arithmetic, the line format, the size cap
//! that stops a long-running process filling a disk - are plain functions with tests; the
//! only part that touches global state is [`init`].

use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Default size at which the log is rotated.
pub const DEFAULT_MAX_BYTES: u64 = 256 * 1024;

/// An append-only log file with a size cap.
#[derive(Clone, Debug)]
pub struct Sink {
    path: PathBuf,
    max_bytes: u64,
}

impl Sink {
    /// A sink writing to `path`, rotating once it exceeds `max_bytes`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, max_bytes: u64) -> Self {
        Self {
            path: path.into(),
            max_bytes,
        }
    }

    /// The file being written.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one line, rotating first if the file has grown past the cap.
    ///
    /// Logging failures are swallowed: a reminder app that stops reminding because it
    /// could not write a log file would be worse than one with no log.
    pub fn write_line(&self, line: &str) {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        self.rotate_if_needed();
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{line}");
        }
    }

    /// Moves the current log aside, keeping exactly one previous generation.
    fn rotate_if_needed(&self) {
        let Ok(meta) = fs::metadata(&self.path) else {
            return;
        };
        if meta.len() <= self.max_bytes {
            return;
        }
        let previous = self.path.with_extension("log.1");
        let _ = fs::remove_file(&previous);
        let _ = fs::rename(&self.path, &previous);
    }
}

/// Formats one log line.
#[must_use]
pub fn format_line(unix_millis: u64, level: Level, target: &str, message: &str) -> String {
    let mut out = String::with_capacity(message.len() + 48);
    let _ = write!(
        out,
        "{} {:<5} {target}: {message}",
        format_timestamp(unix_millis),
        level
    );
    out
}

/// Formats a UTC timestamp as `2026-10-09 14:03:21.123`.
///
/// Spelled out rather than pulled from a date library: it is twenty lines of arithmetic,
/// it is exercised by tests, and a reminder app does not need a dependency to print a log
/// line.
#[must_use]
pub fn format_timestamp(unix_millis: u64) -> String {
    let total_secs = unix_millis / 1000;
    let millis = unix_millis % 1000;
    let days = i64::try_from(total_secs / 86_400).unwrap_or(0);
    let secs_of_day = total_secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{millis:03}",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    )
}

/// Days since the Unix epoch to a civil calendar date, by Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = u64::try_from(z - era * 146_097).unwrap_or(0);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = i64::try_from(year_of_era).unwrap_or(0) + era * 400;
    (year + i64::from(month <= 2), month, day)
}

/// The global logger: writes to a file, and to stderr when one is attached.
#[derive(Debug)]
pub struct FileLogger {
    sink: Sink,
    level: LevelFilter,
}

impl FileLogger {
    /// A logger writing records at or above `level` to `sink`.
    #[must_use]
    pub fn new(sink: Sink, level: LevelFilter) -> Self {
        Self { sink, level }
    }

    /// Renders a record, so the formatting is testable without a global logger.
    #[must_use]
    pub fn render(record: &Record<'_>) -> String {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or_default();
        format_line(
            millis,
            record.level(),
            record.target(),
            &record.args().to_string(),
        )
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = Self::render(record);
        self.sink.write_line(&line);
    }

    fn flush(&self) {}
}

/// Installs the global logger. Only the first call in a process takes effect.
///
/// # Errors
///
/// If a logger has already been installed.
pub fn init(sink: Sink, level: LevelFilter) -> Result<(), log::SetLoggerError> {
    log::set_boxed_logger(Box::new(FileLogger::new(sink, level)))?;
    log::set_max_level(level);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn timestamps_render_known_dates() {
        assert_eq!(format_timestamp(0), "1970-01-01 00:00:00.000");
        assert_eq!(
            format_timestamp(1_760_000_000_123),
            "2025-10-09 08:53:20.123"
        );
        assert_eq!(
            format_timestamp(951_782_400_000),
            "2000-02-29 00:00:00.000",
            "a leap day"
        );
        assert_eq!(
            format_timestamp(4_102_444_799_999),
            "2099-12-31 23:59:59.999"
        );
    }

    #[test]
    fn civil_dates_handle_month_and_year_boundaries() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        assert_eq!(civil_from_days(365), (1971, 1, 1));
    }

    #[test]
    fn lines_carry_the_level_and_the_module() {
        let line = format_line(
            0,
            Level::Warn,
            "eye_saver_platform::store",
            "could not write",
        );
        assert_eq!(
            line,
            "1970-01-01 00:00:00.000 WARN  eye_saver_platform::store: could not write"
        );
    }

    #[test]
    fn writing_creates_the_directory_and_appends() {
        let dir = TempDir::new("logger-append");
        let sink = Sink::new(
            dir.path().join("nested").join("eye-saver.log"),
            DEFAULT_MAX_BYTES,
        );
        sink.write_line("first");
        sink.write_line("second");
        let contents = fs::read_to_string(sink.path()).unwrap();
        assert_eq!(contents, "first\nsecond\n");
    }

    #[test]
    fn the_log_is_rotated_once_it_outgrows_the_cap() {
        let dir = TempDir::new("logger-rotate");
        let path = dir.path().join("eye-saver.log");
        let sink = Sink::new(&path, 16);
        sink.write_line("0123456789abcdefghij");
        assert!(path.exists());
        sink.write_line("after rotation");

        assert_eq!(fs::read_to_string(&path).unwrap(), "after rotation\n");
        let previous = path.with_extension("log.1");
        assert!(previous.exists(), "exactly one generation is kept");
        assert!(
            fs::read_to_string(&previous)
                .unwrap()
                .starts_with("0123456789")
        );
    }

    #[test]
    fn rotation_replaces_an_existing_previous_generation() {
        let dir = TempDir::new("logger-rotate-twice");
        let path = dir.path().join("eye-saver.log");
        fs::write(path.with_extension("log.1"), "ancient history").unwrap();
        let sink = Sink::new(&path, 4);
        sink.write_line("aaaaaaaaaa");
        sink.write_line("bbbb");
        assert_ne!(
            fs::read_to_string(path.with_extension("log.1")).unwrap(),
            "ancient history"
        );
    }

    #[test]
    fn an_unwritable_path_is_survived_silently() {
        // The parent exists as a file, so neither the directory nor the log can be created.
        let dir = TempDir::new("logger-unwritable");
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "not a directory").unwrap();
        let sink = Sink::new(blocker.join("eye-saver.log"), DEFAULT_MAX_BYTES);
        sink.write_line("this must not panic");
    }

    #[test]
    fn the_level_filter_is_respected_and_records_render() {
        let dir = TempDir::new("logger-level");
        let sink = Sink::new(dir.path().join("eye-saver.log"), DEFAULT_MAX_BYTES);
        let logger = FileLogger::new(sink.clone(), LevelFilter::Info);

        assert!(logger.enabled(&Metadata::builder().level(Level::Warn).build()));
        assert!(!logger.enabled(&Metadata::builder().level(Level::Debug).build()));

        logger.log(
            &Record::builder()
                .level(Level::Info)
                .target("t")
                .args(format_args!("kept"))
                .build(),
        );
        logger.log(
            &Record::builder()
                .level(Level::Trace)
                .target("t")
                .args(format_args!("dropped"))
                .build(),
        );
        logger.flush();

        let contents = fs::read_to_string(sink.path()).unwrap();
        assert!(contents.contains("t: kept"));
        assert!(!contents.contains("dropped"));
    }

    #[test]
    fn the_global_logger_is_installed_once() {
        let dir = TempDir::new("logger-init");
        let sink = Sink::new(dir.path().join("eye-saver.log"), DEFAULT_MAX_BYTES);
        assert!(init(sink.clone(), LevelFilter::Info).is_ok());
        log::info!("hello from the test");
        assert!(
            fs::read_to_string(sink.path())
                .unwrap()
                .contains("hello from the test")
        );
        assert!(
            init(sink, LevelFilter::Info).is_err(),
            "a second install is refused"
        );
    }
}
