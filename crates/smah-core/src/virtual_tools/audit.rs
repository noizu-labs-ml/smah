//! Append-only JSONL audit log (design decision D2): one line per directive
//! attempt — allowed or denied — at
//! `~/.local/state/smah/audit-virtual-tools.jsonl` (honors `XDG_STATE_HOME`).
//!
//! **Metadata only.** Contents, stdin, and prompts are never logged (secret
//! hygiene; the audit is a repudiation aid, not a transcript). The directory
//! is created 0700, the file 0600. No rotation in v1 (user-managed).
//!
//! Write failures are best-effort: a stderr warning, never a crash — an
//! unusable audit log must not make the tool surface unusable in v1 (the
//! trade-off is documented in the driver spec).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// One audit record (JSONL line).
#[derive(Debug, Clone, Serialize)]
pub struct AuditRecord {
    /// Local time with offset (repo convention).
    pub ts: String,
    /// Run id (uuid v4), shared by every record of one invocation.
    pub run: String,
    /// Tool name.
    pub tool: String,
    /// Directive verb (`read`/`write`/`append`).
    pub op: String,
    /// Path as the model requested it (never contents).
    pub path: String,
    /// Canonical path the operation resolved to, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
    /// Payload size in bytes when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<usize>,
    /// `allowed` or `denied`.
    pub decision: &'static str,
    /// Empty when allowed; the reason when denied.
    pub reason: String,
    /// Zero-based model turn the directive arrived in.
    pub turn: usize,
}

/// The open audit sink for one run.
/// Clone: cheap handle copy — every clone appends to the same file.
#[derive(Clone)]
pub struct Audit {
    path: PathBuf,
    disabled_reason: Option<String>,
}

impl Audit {
    /// Open (creating dirs/file with 0700/0600) at the default state path.
    pub fn open(run_id: &str) -> Self {
        Self::open_at(&state_dir().join("audit-virtual-tools.jsonl"), run_id)
    }

    /// Open at an explicit path (tests, `--state-dir` overrides later).
    pub fn open_at(path: &Path, _run_id: &str) -> Self {
        let mut disabled_reason = None;
        if let Some(parent) = path.parent() {
            match std::fs::create_dir_all(parent) {
                Err(err) => {
                    disabled_reason = Some(format!("cannot create {}: {}", parent.display(), err))
                }
                Ok(()) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Ok(meta) = std::fs::metadata(parent) {
                            let mut perms = meta.permissions();
                            perms.set_mode(0o700);
                            let _ = std::fs::set_permissions(parent, perms);
                        }
                    }
                }
            }
        }
        Audit {
            path: path.to_path_buf(),
            disabled_reason,
        }
    }

    /// Why the audit is disabled, if it is.
    pub fn disabled_reason(&self) -> Option<&str> {
        self.disabled_reason.as_deref()
    }

    /// Append one record. Returns `Err` with a short reason when the line
    /// could not be written (the caller decides how loud to be — the driver
    /// warns once on stderr).
    pub fn record(&self, rec: &AuditRecord) -> Result<(), String> {
        if self.disabled_reason.is_some() {
            return Err(self.disabled_reason.clone().unwrap());
        }
        let line = serde_json::to_string(rec).map_err(|err| err.to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|err| format!("{}: {}", self.path.display(), err))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = file.metadata() {
                let mut perms = meta.permissions();
                perms.set_mode(0o600);
                let _ = file.set_permissions(perms);
            }
        }
        writeln!(file, "{line}").map_err(|err| err.to_string())?;
        Ok(())
    }
}

/// `~/.local/state/smah` (honors `XDG_STATE_HOME`).
pub fn state_dir() -> PathBuf {
    let mut base = match std::env::var("XDG_STATE_HOME") {
        Ok(x) if !x.is_empty() => PathBuf::from(x),
        _ => {
            let mut h = home::home_dir().unwrap_or_else(|| PathBuf::from("."));
            h.push(".local");
            h.push("state");
            h
        }
    };
    base.push("smah");
    base
}

/// `~/.cache/smah` (honors `XDG_CACHE_HOME`) — for the regenerable transcript
/// scratch (design decision D5).
pub fn cache_dir() -> PathBuf {
    let mut base = match std::env::var("XDG_CACHE_HOME") {
        Ok(x) if !x.is_empty() => PathBuf::from(x),
        _ => {
            let mut h = home::home_dir().unwrap_or_else(|| PathBuf::from("."));
            h.push(".cache");
            h
        }
    };
    base.push("smah");
    base
}
