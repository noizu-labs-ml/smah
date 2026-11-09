//! `tracing`-based logging with per-run files, size rotation, and global
//! secret redaction (T-001 fix).
//!
//! Layout per run:
//!
//! - **File layer** — everything (TRACE and up) goes to
//!   `~/.smah/logs/smah.<YYYYmmdd_HHMMSS>.log` (local time), rotated by size:
//!   at 1 MB the current file shifts to `.1`/`.2` and at most 3 files are
//!   kept per run-stamp.
//! - **Console layer** — WARN and up to **stderr only**, ANSI-styled iff
//!   stderr is a TTY.
//!
//! Both layers write through the [`Redactor`] registry before any byte hits
//! disk or the console: substrings registered via [`register_secret`] are
//! rewritten to `***`. This is the structural fix for T-001 (provider API
//! keys reaching the DEBUG log through the settings save-dump).
//!
//! ```no_run
//! smah_core::logging::init().expect("logging init");
//! smah_core::logging::register_secret("sk-live-...");
//! let dump = "vsn: 0.0.1\nproviders:\n  anthropic:\n    settings:\n      api_key: ***\n";
//! tracing::debug!("Settings YAML: {}", dump); // key never reaches the file
//! ```

use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::layer::{Layer as _, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

/// Replacement written over every registered secret.
pub const REDACTION_MASK: &str = "***";

/// Rotate the active file once it reaches this size.
const MAX_FILE_BYTES: u64 = 1_000_000;

/// Rotated generations kept besides the active file (3 files total).
const ROTATIONS_KEPT: u32 = 2;

type Registry = Vec<(String, &'static str)>;

/// Global secret registry: `(needle, replacement)` pairs, read on every
/// write, written only by [`register_secret`]/[`register_replacement`].
fn registry() -> &'static RwLock<Registry> {
    static REGISTRY: OnceLock<RwLock<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Register a secret string; every future write rewrites it to `***`.
/// Empty or whitespace-only secrets are ignored. Secrets already written to
/// past log files are unaffected — registration must precede the calls that
/// may log them (it does: settings resolution registers keys before the
/// first dump).
pub fn register_secret(secret: impl Into<String>) {
    register_replacement(secret, REDACTION_MASK);
}

/// Register a secret with a custom replacement string.
pub fn register_replacement(secret: impl Into<String>, replacement: &'static str) {
    let secret = secret.into();
    if secret.trim().is_empty() {
        return;
    }
    if let Ok(mut reg) = registry().write() {
        if !reg.iter().any(|(needle, _)| needle == &secret) {
            reg.push((secret, replacement));
        }
    }
}

/// Rewrite every registered substring in `input` (secrets → `***`).
pub fn redact(input: &str) -> String {
    match registry().read() {
        Ok(reg) => {
            let mut out = input.to_string();
            for (needle, replacement) in reg.iter() {
                out = out.replace(needle, replacement);
            }
            out
        }
        Err(_) => input.to_string(),
    }
}

/// Compute the per-run log file name: `smah.<YYYYmmdd_HHMMSS>.log` (local
/// time).
fn log_file_name() -> String {
    format!("smah.{}.log", chrono::Local::now().format("%Y%m%d_%H%M%S"))
}

/// Default logs directory: `~/.smah/logs`.
pub fn default_logs_dir() -> PathBuf {
    let mut path = home::home_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push(".smah");
    path.push("logs");
    path
}

/// Initialize logging against the default `~/.smah/logs` directory.
///
/// Returns the log file path. If a global subscriber is already set (double
/// init in tests), the layers are not re-installed but the path is still
/// returned.
pub fn init() -> io::Result<PathBuf> {
    init_with_dir(&default_logs_dir())
}

/// Initialize logging with an explicit logs directory (used by tests and the
/// future `--log-dir` override). See [`init`] for behavior.
pub fn init_with_dir(dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(log_file_name());
    let rotating = Arc::new(RotatingFile::new(path.clone()));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(FileMaker { file: rotating });

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_ansi(io::stderr().is_terminal())
        .with_writer(StderrMaker)
        .with_filter(LevelFilter::WARN);

    let _ = tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .try_init();
    Ok(path)
}

/// Size-rotating log file: `smah.<ts>.log` → `.1` → `.2` (oldest dropped).
/// Every write passes through [`redact`] before hitting the disk.
struct RotatingFile {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

impl RotatingFile {
    fn new(path: PathBuf) -> Self {
        RotatingFile {
            path,
            file: Mutex::new(None),
        }
    }

    fn rotated_path(&self, generation: u32) -> PathBuf {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "smah.log".to_string());
        self.path.with_file_name(format!("{name}.{generation}"))
    }

    /// Shift current → `.1` → `.2`, dropping beyond [`ROTATIONS_KEPT`], then
    /// reopen a fresh active file.
    fn rotate(&self) -> io::Result<()> {
        for generation in (1..ROTATIONS_KEPT).rev() {
            let from = self.rotated_path(generation);
            let to = self.rotated_path(generation + 1);
            if from.exists() {
                let _ = fs::rename(&from, &to);
            }
        }
        if self.path.exists() {
            let _ = fs::rename(&self.path, self.rotated_path(1));
        }
        self.reopen()
    }

    fn reopen(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        *self.lock() = Some(file);
        Ok(())
    }

    /// Open the active file if it is not open yet.
    fn ensure_open(&self) -> io::Result<()> {
        if self.lock().is_none() {
            self.reopen()?;
        }
        Ok(())
    }

    /// Reopen under rotation iff the active file has reached
    /// [`MAX_FILE_BYTES`].
    fn rotate_if_needed(&self) -> io::Result<()> {
        let size = fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if size >= MAX_FILE_BYTES {
            self.rotate()?;
        }
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<File>> {
        self.file.lock().expect("rotating log file lock")
    }

    /// The single disk-bound write path: ensure open, rotate on size,
    /// redact, write. Returns `buf.len()` (the lossy-redacted copy is what
    /// lands on disk).
    fn write_redacted(&self, buf: &[u8]) -> io::Result<usize> {
        self.ensure_open()?;
        self.rotate_if_needed()?;
        let mut guard = self.lock();
        if let Some(file) = guard.as_mut() {
            let text = String::from_utf8_lossy(buf);
            let clean = redact(&text);
            file.write_all(clean.as_bytes())?;
        }
        Ok(buf.len())
    }

    fn flush_disk(&self) -> io::Result<()> {
        if let Some(file) = self.lock().as_mut() {
            file.flush()?;
        }
        Ok(())
    }
}

/// [`MakeWriter`] handing out handles to the shared [`RotatingFile`].
#[derive(Clone)]
struct FileMaker {
    file: Arc<RotatingFile>,
}

struct FileWriter(Arc<RotatingFile>);

impl Write for FileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_redacted(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush_disk()
    }
}

impl<'a> MakeWriter<'a> for FileMaker {
    type Writer = FileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        FileWriter(self.file.clone())
    }
}

/// [`MakeWriter`] for stderr, redacting before bytes reach the console.
#[derive(Clone, Copy, Default)]
struct StderrMaker;

struct RedactingStderr(io::StderrLock<'static>);

impl Write for RedactingStderr {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let clean = redact(&text);
        self.0.write_all(clean.as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<'a> MakeWriter<'a> for StderrMaker {
    type Writer = RedactingStderr;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingStderr(io::stderr().lock())
    }
}
