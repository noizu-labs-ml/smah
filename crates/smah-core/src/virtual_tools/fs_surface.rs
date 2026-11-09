//! The restricted file surface: allowlist-root containment and directive
//! execution with plain `std::fs` (design decision D2). The model cannot run
//! code — it emits directives; this module is the only executor.
//!
//! Containment model:
//! 1. roots are canonicalized once per run (a symlinked root resolves to its
//!    target — the target is what is trusted);
//! 2. a requested path is rejected lexically first if it carries any `..`
//!    component; absolute paths are used as given, relative paths resolve
//!    against the **first declared root** (deterministic; the contract tells
//!    the model this);
//! 3. containment is a canonical-path *component* prefix against a canonical
//!    root — string prefixes do not count (`/root-evil` is not inside `/root`);
//! 4. `read` follows symlinks but the canonical target must stay in-root;
//!    `write`/`append` refuse a symlink as the final path component outright.
//!    All existing-component symlinks are collapsed by canonicalize, so
//!    symlink escape reduces to the final component. The canonicalize→open
//!    TOCTOU window is accepted for v1 (documented; `openat2 RESOLVE_BENEATH`
//!    is the noted future hardening).
//!
//! Outcome taxonomy:
//! - **Violation** aborts the run immediately (exit 3): path outside all
//!   roots, `..` traversal, final-component symlink on write/append,
//!   aggregate write cap breach. First violation stops everything — a
//!   directive outside the declared roots is a tool-design bug or an attack;
//!   both deserve a loud abort.
//! - **SoftError** is fed back to the model as a result line and the run
//!   continues: missing/oversize/directory read targets, per-directive write
//!   over [`crate::virtual_tools::WRITE_CONTENT_MAX`], write onto an existing
//!   directory, per-path I/O errors.

use std::path::{Component, Path, PathBuf};

use crate::virtual_tools::directives::Directive;
use crate::virtual_tools::{READ_FILE_MAX, RUN_AGGREGATE_WRITE_MAX, WRITE_CONTENT_MAX};

/// What happened when the driver executed one directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Performed; `resolved` is the canonical path, `bytes` the payload size.
    Allowed {
        /// Canonical path the operation landed on.
        resolved: PathBuf,
        /// Payload bytes (read size, or bytes written/appended).
        bytes: usize,
        /// For `read`: the file's text (UTF-8).
        text: Option<String>,
    },
    /// Refused but survivable; `detail` is one line and is fed back to the
    /// model so it can adapt.
    SoftError {
        /// One-line reason.
        detail: String,
    },
    /// Run-aborting refusal; `detail` names path + reason (stderr + audit).
    Violation {
        /// One-line reason (path + why).
        detail: String,
    },
}

/// The per-run file surface. `roots` are already canonicalized
/// ([`crate::virtual_tools::loader::resolve_roots`]).
pub struct FileSurface {
    roots: Vec<PathBuf>,
    aggregate_written: usize,
}

impl FileSurface {
    /// Build a surface over roots. Roots are canonicalized here (defensively —
    /// the loader already canonicalizes; a symlinked root resolves to its
    /// target, and a non-canonical path like macOS's `/var/...` tempdirs
    /// resolves to its real `/private/var/...` form so component containment
    /// stays exact).
    pub fn new(roots: Vec<PathBuf>) -> Self {
        let canonical = roots
            .into_iter()
            .map(|root| std::fs::canonicalize(&root).unwrap_or(root))
            .collect();
        FileSurface {
            roots: canonical,
            aggregate_written: 0,
        }
    }

    /// Roots, canonical.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Bytes written so far this run (for the aggregate cap).
    pub fn aggregate_written(&self) -> usize {
        self.aggregate_written
    }

    /// Execute one directive, auditing left to the caller (the driver audits
    /// every attempt, allowed or not).
    pub fn execute(&mut self, directive: &Directive) -> Outcome {
        let is_write = matches!(
            directive,
            Directive::Write { .. } | Directive::Append { .. }
        );
        let path_req = directive.path().to_string();

        // 1. Lexical `..` rejection before any filesystem call.
        if has_dotdot(&path_req) {
            return Outcome::Violation {
                detail: format!("path '{path_req}' contains '..' traversal"),
            };
        }

        // 2. Resolve: absolute as-is, relative against the first root.
        let candidate = match Path::new(&path_req) {
            p if p.is_absolute() => p.to_path_buf(),
            rel => match self.roots.first() {
                Some(first) => first.join(rel),
                None => {
                    return Outcome::Violation {
                        detail: format!(
                            "path '{path_req}' is relative but the tool declares no allow roots"
                        ),
                    }
                }
            },
        };

        // 3. Canonical resolution: existing final component via canonicalize;
        //    otherwise canonicalize the parent and join the (plain) final
        //    component.
        let resolved = match resolve_candidate(&candidate) {
            Ok(resolved) => resolved,
            Err(err) => {
                return Outcome::SoftError {
                    detail: format!("path '{path_req}' could not be resolved: {err}"),
                }
            }
        };

        // 4. Component-prefix containment against a canonical root.
        if !self.contained(&resolved) {
            return Outcome::Violation {
                detail: format!(
                    "path '{}' is outside the tool's allowed roots",
                    resolved.display()
                ),
            };
        }

        // 5. Write/append: the final component must not be a symlink.
        if is_write {
            if let Ok(meta) = std::fs::symlink_metadata(&candidate) {
                if meta.file_type().is_symlink() {
                    return Outcome::Violation {
                        detail: format!(
                            "write target '{path_req}' is a symlink (refusing; writes never follow a symlink at the final path component)"
                        ),
                    };
                }
            }
        }

        match directive {
            Directive::Read { path } => self.do_read(path),
            Directive::Write { path, content } => self.do_write(path, content, false),
            Directive::Append { path, content } => self.do_write(path, content, true),
            // Unreachable from the cli driver: its scanners only accept the
            // base verbs (extension directives are refused in
            // `driver::execute_directive` before the surface is touched).
            Directive::Extension { verb, .. } => Outcome::Violation {
                detail: format!("directive verb '{verb}' has no file operation"),
            },
        }
    }

    fn do_read(&mut self, req: &str) -> Outcome {
        let candidate = self.candidate_for(req);
        let resolved = match resolve_candidate(&candidate) {
            Ok(resolved) => resolved,
            Err(err) => {
                return Outcome::SoftError {
                    detail: format!("read '{req}': {err}"),
                }
            }
        };
        if !self.contained(&resolved) {
            return Outcome::Violation {
                detail: format!(
                    "read target '{}' is outside the tool's allowed roots",
                    resolved.display()
                ),
            };
        }
        let meta = match std::fs::symlink_metadata(&resolved) {
            Ok(meta) => meta,
            Err(_) => {
                return Outcome::SoftError {
                    detail: format!("read '{req}': file does not exist"),
                }
            }
        };
        if !meta.is_file() {
            return Outcome::SoftError {
                detail: format!("read '{req}': not a regular file"),
            };
        }
        if meta.len() > READ_FILE_MAX as u64 {
            return Outcome::SoftError {
                detail: format!(
                    "read '{req}': file is {} bytes, over the {} KiB read cap",
                    meta.len(),
                    READ_FILE_MAX / 1024
                ),
            };
        }
        match std::fs::read(&resolved) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => Outcome::Allowed {
                    resolved,
                    bytes: text.len(),
                    text: Some(text),
                },
                Err(_) => Outcome::SoftError {
                    detail: format!("read '{req}': file is not UTF-8 text"),
                },
            },
            Err(err) => Outcome::SoftError {
                detail: format!("read '{req}': {}", one_line(&err.to_string())),
            },
        }
    }

    fn do_write(&mut self, req: &str, content: &str, append: bool) -> Outcome {
        // Per-directive cap: soft reject, file untouched.
        if content.len() > WRITE_CONTENT_MAX {
            return Outcome::SoftError {
                detail: format!(
                    "write '{req}': payload is {} bytes, over the {} KiB per-write cap (retry with smaller content)",
                    content.len(),
                    WRITE_CONTENT_MAX / 1024
                ),
            };
        }
        // Aggregate cap: violation, abort the run.
        if self.aggregate_written + content.len() > RUN_AGGREGATE_WRITE_MAX {
            return Outcome::Violation {
                detail: format!(
                    "write '{req}' would exceed the {} KiB total write cap for this run",
                    RUN_AGGREGATE_WRITE_MAX / 1024
                ),
            };
        }
        let verb = if append { "append" } else { "write" };
        let candidate = self.candidate_for(req);
        let resolved = match resolve_candidate(&candidate) {
            Ok(resolved) => resolved,
            Err(err) => {
                return Outcome::SoftError {
                    detail: format!("{verb} '{req}': {err}"),
                }
            }
        };
        if !self.contained(&resolved) {
            return Outcome::Violation {
                detail: format!(
                    "{verb} target '{}' is outside the tool's allowed roots",
                    resolved.display()
                ),
            };
        }
        if resolved.is_dir() {
            return Outcome::SoftError {
                detail: format!("{verb} '{req}': target exists and is a directory"),
            };
        }
        // No mkdir -p: the parent must already exist (a new file under a
        // nonexistent subdirectory is rejected — a missing tree is a tool
        // bug the model should hear about, not a silent tree the driver
        // grows).
        let result = if append {
            std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&resolved)
                .and_then(|mut file| std::io::Write::write_all(&mut file, content.as_bytes()))
        } else {
            std::fs::write(&resolved, content.as_bytes())
        };
        match result {
            Ok(()) => {
                self.aggregate_written += content.len();
                Outcome::Allowed {
                    resolved,
                    bytes: content.len(),
                    text: None,
                }
            }
            Err(err) => Outcome::SoftError {
                detail: format!("{verb} '{req}': {}", one_line(&err.to_string())),
            },
        }
    }

    /// The pre-canonicalization candidate path for a request.
    fn candidate_for(&self, req: &str) -> PathBuf {
        let p = Path::new(req);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            match self.roots.first() {
                Some(first) => first.join(p),
                None => p.to_path_buf(),
            }
        }
    }

    /// Component-prefix containment: `resolved` must be `root` itself or a
    /// descendant of a canonical root.
    fn contained(&self, resolved: &Path) -> bool {
        self.roots.iter().any(|root| is_within(resolved, root))
    }
}

/// Reject any lexical `..` component in a requested path. Also rejects a
/// bare `.`-only path with no name.
fn has_dotdot(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|c| c == Component::ParentDir)
}

/// Canonical resolution of a candidate: if the final component exists,
/// canonicalize it; otherwise canonicalize the parent and join the final
/// component (which must therefore be plain — any symlink there is caught
/// by the write-path check, and by the containment check for reads since
/// canonicalization already collapsed it).
fn resolve_candidate(candidate: &Path) -> Result<PathBuf, String> {
    match std::fs::symlink_metadata(candidate) {
        Ok(_) => std::fs::canonicalize(candidate).map_err(|err| one_line(&err.to_string())),
        Err(_) => {
            let parent = candidate
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .ok_or_else(|| "path has no parent directory".to_string())?;
            let final_component = candidate
                .file_name()
                .ok_or_else(|| "path has no final component".to_string())?;
            let canonical_parent = std::fs::canonicalize(parent)
                .map_err(|err| format!("parent directory: {}", one_line(&err.to_string())))?;
            Ok(canonical_parent.join(final_component))
        }
    }
}

/// True when `path` is `root` itself or a descendant (component prefix, not
/// string prefix: `/root-evil` is not inside `/root`).
fn is_within(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
        && path
            .strip_prefix(root)
            .map(|rest| {
                rest.components().all(|c| {
                    matches!(
                        c,
                        Component::Normal(_) | Component::RootDir | Component::CurDir
                    )
                })
            })
            .unwrap_or(false)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
