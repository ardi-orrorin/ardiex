use anyhow::Result;
use env_logger::Builder;
use env_logger::Env;
use file_rotate::compression::Compression;
use file_rotate::suffix::{AppendTimestamp, DateFrom, FileLimit};
use file_rotate::{ContentLimit, FileRotate};
use log::LevelFilter;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Mutex;

const DEFAULT_MAX_LOG_FILE_SIZE_MB: u64 = 20;
const MAX_ROTATED_LOG_FILES: usize = 30;
const DATE_SUFFIX_PATTERN: &str = "%Y-%m-%d_%H-%M-%S";

struct TeeLogWriter<F: Write, O: Write> {
    file_writer: Mutex<F>,
    stdout: Mutex<O>,
}

impl<F: Write, O: Write> TeeLogWriter<F, O> {
    fn new(file_writer: F, stdout: O) -> Self {
        Self {
            file_writer: Mutex::new(file_writer),
            stdout: Mutex::new(stdout),
        }
    }
}

impl<F: Write, O: Write> Write for TeeLogWriter<F, O> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut file_writer = self
            .file_writer
            .lock()
            .map_err(|_| io::Error::other("file logger mutex poisoned"))?;
        file_writer.write_all(buf)?;

        let mut stdout = self
            .stdout
            .lock()
            .map_err(|_| io::Error::other("stdout logger mutex poisoned"))?;
        stdout.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut file_writer = self
            .file_writer
            .lock()
            .map_err(|_| io::Error::other("file logger mutex poisoned"))?;
        file_writer.flush()?;

        let mut stdout = self
            .stdout
            .lock()
            .map_err(|_| io::Error::other("stdout logger mutex poisoned"))?;
        stdout.flush()
    }
}

fn apply_local_time_format(builder: &mut Builder) {
    builder.format(|buf, record| {
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        writeln!(
            buf,
            "[{} {} {}] {}",
            ts,
            record.level(),
            record.target(),
            record.args()
        )
    });
}

pub fn init_file_logging_with_size(log_dir: &PathBuf, max_log_file_size_mb: u64) -> Result<()> {
    fs::create_dir_all(log_dir)?;

    let log_file = log_dir.join("ardiex.log");
    let size_mb = if max_log_file_size_mb == 0 {
        DEFAULT_MAX_LOG_FILE_SIZE_MB
    } else {
        max_log_file_size_mb
    };
    let max_bytes_u64 = size_mb
        .checked_mul(1024 * 1024)
        .ok_or_else(|| anyhow::anyhow!("max_log_file_size_mb is too large: {}", size_mb))?;
    let max_bytes: usize = max_bytes_u64.try_into().map_err(|_| {
        anyhow::anyhow!(
            "max_log_file_size_mb is too large for this platform: {}",
            size_mb
        )
    })?;

    let suffix = AppendTimestamp::with_format(
        DATE_SUFFIX_PATTERN,
        FileLimit::MaxFiles(MAX_ROTATED_LOG_FILES),
        DateFrom::Now,
    );
    let content_limit = ContentLimit::BytesSurpassed(max_bytes);

    #[cfg(unix)]
    let rotate = FileRotate::new(
        log_file.clone(),
        suffix,
        content_limit,
        Compression::OnRotate(1),
        None,
    );

    #[cfg(not(unix))]
    let rotate = FileRotate::new(
        log_file.clone(),
        suffix,
        content_limit,
        Compression::OnRotate(1),
    );

    let writer = TeeLogWriter::new(rotate, io::stdout());

    let mut builder = Builder::from_env(Env::default().default_filter_or("info"));
    builder
        .target(env_logger::Target::Pipe(Box::new(writer)))
        .filter_level(LevelFilter::Info);
    apply_local_time_format(&mut builder);
    builder.init();

    println!(
        "Logging to file: {:?} (max size: {} MB, rotate: gzip + date suffix {})",
        log_file, size_mb, DATE_SUFFIX_PATTERN
    );
    Ok(())
}

pub fn init_console_logging() {
    let mut builder = Builder::from_env(Env::default().default_filter_or("info"));
    apply_local_time_format(&mut builder);
    builder.init();
}

#[cfg(test)]
#[path = "tests/logger_tests.rs"]
mod tests;
