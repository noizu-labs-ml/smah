//! Tool-file loading: parse, validate, and discover
//! `~/.config/smah/virtual-tools/*.md` (design decision D1).
//!
//! One file per tool; the filename stem **is** the tool name, so a
//! `--emulate-cli <tool>` invocation resolves to exactly one file with no
//! directory scan and no name/stem drift.
//!
//! Strictness rules (all rejections are [`SmahError::ToolFileInvalid`] on
//! direct invoke, and a stderr-listed `invalid` row in `smah tools list`):
//! - unknown frontmatter keys make the file invalid (a `constrants:` typo
//!   must not silently drop a security-relevant constraint);
//! - `name` must equal the stem, match [`crate::virtual_tools::is_valid_name`],
//!   and not be in [`crate::virtual_tools::RESERVED_NAMES`];
//! - `driver` (optional) names the presentation driver: the canonical set is
//!   `cli | tui | web | api | soap | mcp | grpc | vfs | db` and `"emulated"`
//!   is a permanent alias of `"cli"` (see [`crate::virtual_tools::drivers`]).
//!   Unknown drivers reject the file — they must not silently run as the cli
//!   driver — and [`ToolDef::driver`] stores the canonical name;
//! - `interface` (optional) is the driver-specific contract artifact —
//!   inline text or a path/`$VAR` reference, driver's choice. The loader
//!   only checks that it is a string; each driver validates its own content;
//! - `allow_roots` entries (`~/...` or whole-value `$VAR`) must resolve to an
//!   existing directory at invoke time;
//! - the markdown body (the tool's persona) is required and non-empty.

use std::path::{Path, PathBuf};

use serde_yaml_ng::Value;

use crate::errors::SmahError;
use crate::virtual_tools::drivers::{canonical, DRIVER_NAMES};
use crate::virtual_tools::{is_valid_name, RESERVED_NAMES};

/// A parsed, valid tool definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDef {
    /// Tool name (== filename stem).
    pub name: String,
    /// One-line description, shown by `smah tools list`.
    pub description: String,
    /// `"<provider>.<model-id>"` or a bare model id; `None` = settings default.
    pub model: Option<String>,
    /// Canonical driver name (`"cli"` for every legacy file — `"emulated"`
    /// loads as `"cli"`; the other family members land with their lanes).
    pub driver: String,
    /// Driver-specific contract artifact (the optional `interface:`
    /// frontmatter key): inline text or a path/`$VAR` reference, whichever
    /// the owning driver defines. `None` when absent; the loader does not
    /// interpret it.
    pub interface: Option<String>,
    /// Author-supplied hard rules, injected under a `Hard constraints:` heading.
    pub constraints: Vec<String>,
    /// Raw `allow_roots` entries (`~/...`, `$VAR`, absolute, relative) —
    /// expanded and canonicalized at invoke time by [`resolve_roots`].
    pub allow_roots: Vec<String>,
    /// The markdown body: the tool's persona/system prompt.
    pub body: String,
    /// Where the file lives (for error messages, cache keys, audit).
    pub path: PathBuf,
}

/// A file present in the tools dir that failed validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidTool {
    /// File path.
    pub path: PathBuf,
    /// Filename stem (best-effort name for display).
    pub name: String,
    /// One-line rejection reason.
    pub reason: String,
}

/// Frontmatter keys the loader accepts. Anything else rejects the file.
pub const KNOWN_KEYS: [&str; 7] = [
    "name",
    "description",
    "model",
    "driver",
    "interface",
    "constraints",
    "allow_roots",
];

/// The tools directory: `SMAH_VIRTUAL_TOOLS_DIR` (test/harness seam) >
/// `$XDG_CONFIG_HOME/smah/virtual-tools` > `~/.config/smah/virtual-tools`.
pub fn tools_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SMAH_VIRTUAL_TOOLS_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let mut base = match std::env::var("XDG_CONFIG_HOME") {
        Ok(x) if !x.is_empty() => PathBuf::from(x),
        _ => {
            let mut h = home::home_dir().unwrap_or_else(|| PathBuf::from("."));
            h.push(".config");
            h
        }
    };
    base.push("smah");
    base.push("virtual-tools");
    base
}

/// Discover every `.md` file in `dir`, sorted by stem. Missing dir ⇒ empty.
/// Returns valid defs and invalid files (each invalid file appears exactly
/// once, with its reason).
pub fn discover_in(dir: &Path) -> (Vec<ToolDef>, Vec<InvalidTool>) {
    let mut stems: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|x| x == "md").unwrap_or(false))
                .filter_map(|e| {
                    e.path()
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    stems.sort();
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for stem in stems {
        let path = dir.join(format!("{stem}.md"));
        match load_from(&path) {
            Ok(def) => valid.push(def),
            Err(reason) => invalid.push(InvalidTool {
                path,
                name: stem,
                reason,
            }),
        }
    }
    (valid, invalid)
}

/// Discover in the default [`tools_dir`].
pub fn discover() -> (Vec<ToolDef>, Vec<InvalidTool>) {
    discover_in(&tools_dir())
}

/// Load `<tools_dir>/<tool>.md` — the direct-invoke resolution path. A
/// missing file is [`SmahError::ToolNotFound`] (exit 2); a present-but-
/// invalid file is [`SmahError::ToolFileInvalid`] (exit 3) with the reason.
pub fn load_by_name(name: &str) -> Result<ToolDef, SmahError> {
    let dir = tools_dir();
    let path = dir.join(format!("{name}.md"));
    if !path.is_file() {
        let (valid, _) = discover_in(&dir);
        let mut names: Vec<String> = valid.into_iter().map(|t| t.name).collect();
        names.sort();
        let installed = if names.is_empty() {
            "(none)".to_string()
        } else {
            names.join(", ")
        };
        return Err(SmahError::ToolNotFound {
            name: name.to_string(),
            installed,
        });
    }
    load_from(&path).map_err(|reason| SmahError::ToolFileInvalid {
        path: path.display().to_string(),
        detail: reason,
    })
}

/// Parse and validate one tool file. `Err(reason)` is a one-line rejection.
pub fn load_from(path: &Path) -> Result<ToolDef, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("unreadable: {}", one_line(&err.to_string())))?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let (frontmatter, body) = split_frontmatter(&raw, path)?;

    // Strict mapping: unknown keys reject (with the key named).
    let Value::Mapping(map) = frontmatter else {
        return Err("frontmatter is not a key/value mapping".to_string());
    };
    for key in map.keys() {
        let key = key.as_str().unwrap_or("<non-string key>");
        if !KNOWN_KEYS.contains(&key) {
            return Err(format!("unknown frontmatter key '{key}'"));
        }
    }

    let get_str = |key: &str| -> Option<Result<String, String>> {
        map.get(Value::String(key.to_string())).map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("'{key}' must be a string"))
        })
    };
    let get_list = |key: &str| -> Option<Result<Vec<String>, String>> {
        map.get(Value::String(key.to_string())).map(|v| {
            v.as_sequence()
                .map(|seq| {
                    seq.iter()
                        .map(|item| {
                            item.as_str()
                                .map(str::to_string)
                                .ok_or_else(|| format!("'{key}' entries must be strings"))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap_or_else(|| Err(format!("'{key}' must be a list")))
        })
    };

    let name = match get_str("name") {
        Some(Ok(n)) => n,
        Some(Err(err)) => return Err(err),
        None => return Err("missing required key 'name'".to_string()),
    };
    if name != stem {
        return Err(format!(
            "frontmatter name '{name}' does not match the filename stem '{stem}'"
        ));
    }
    if !is_valid_name(&name) {
        return Err(format!(
            "name '{name}' is invalid: lowercase letters/digits/'-'/'_', starting with a letter, at most 64 characters"
        ));
    }
    if RESERVED_NAMES.contains(&name.as_str()) {
        return Err(format!(
            "name '{name}' is reserved (it would collide with a smah command)"
        ));
    }

    let description = match get_str("description") {
        Some(Ok(d)) => d,
        Some(Err(err)) => return Err(err),
        None => return Err("missing required key 'description'".to_string()),
    };
    if description.trim().is_empty() {
        return Err("'description' must be non-empty".to_string());
    }

    let model = match get_str("model") {
        Some(Ok(m)) if m.trim().is_empty() => {
            return Err("'model' must be non-empty when present".to_string())
        }
        Some(Ok(m)) => Some(m),
        Some(Err(err)) => return Err(err),
        None => None,
    };

    // Driver taxonomy: any family name (or the "emulated" alias) is
    // accepted and stored canonically; unknown drivers reject so a typo or
    // a not-yet-landed driver never silently runs as cli.
    let driver = match get_str("driver") {
        Some(Ok(d)) => d,
        Some(Err(err)) => return Err(err),
        None => "cli".to_string(),
    };
    let driver = canonical(&driver).ok_or_else(|| {
        format!(
            "unknown driver '{driver}': known drivers are {} ('emulated' is an alias of 'cli')",
            DRIVER_NAMES.join(", ")
        )
    })?;

    let interface = match get_str("interface") {
        Some(Ok(text)) => Some(text),
        Some(Err(err)) => return Err(err),
        None => None,
    };

    let constraints = match get_list("constraints") {
        Some(Ok(list)) => list,
        Some(Err(err)) => return Err(err),
        None => Vec::new(),
    };
    let allow_roots = match get_list("allow_roots") {
        Some(Ok(list)) => list,
        Some(Err(err)) => return Err(err),
        None => Vec::new(),
    };

    let body = body.trim().to_string();
    if body.is_empty() {
        return Err("the markdown body (the tool's prompt) is required and non-empty".to_string());
    }

    Ok(ToolDef {
        name,
        description,
        model,
        driver: driver.to_string(),
        interface,
        constraints,
        allow_roots,
        body,
        path: path.to_path_buf(),
    })
}

/// Split a leading `---`-fenced YAML frontmatter from the markdown body.
/// Missing fence or unterminated frontmatter is an error naming the file.
fn split_frontmatter(raw: &str, path: &Path) -> Result<(Value, String), String> {
    let nometa = format!("{}: ", path.display());
    let mut lines = raw.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(format!("{nometa}must start with a '---' frontmatter fence"));
    }
    let mut front = Vec::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        front.push(line);
    }
    if !closed {
        return Err(format!("{nometa}frontmatter fence is never closed"));
    }
    // The body is everything after the closing fence line (byte-offset walk,
    // so a '---' inside the body is left alone).
    let body = {
        let mut consumed = 0usize;
        for (idx, line) in raw.lines().enumerate() {
            consumed += line.len() + 1;
            if idx > 0 && line.trim_end() == "---" {
                break;
            }
        }
        if consumed >= raw.len() {
            String::new()
        } else {
            raw[consumed..].to_string()
        }
    };
    let value: Value = serde_yaml_ng::from_str(&front.join("\n")).map_err(|err| {
        format!(
            "{nometa}frontmatter YAML did not parse: {}",
            one_line(&err.to_string())
        )
    })?;
    Ok((value, body))
}

/// Collapse whitespace to one line (error `<detail>` is one line).
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Expand `allow_roots` entries and canonicalize each into a trusted root.
///
/// - `~/...` expands via the home directory;
/// - a whole-value `$VAR` (or `${VAR}`) resolves through the environment —
///   same convention as settings `$ENV` API keys, no mid-string interpolation;
/// - every root must exist and be a directory, then is canonicalized (a
///   symlinked root resolves to its target — the target is what is trusted).
///
/// A failure is a [`SmahError::ToolFileInvalid`] ("invalid at load-for-invoke").
pub fn resolve_roots(tool: &ToolDef) -> Result<Vec<PathBuf>, SmahError> {
    let mut roots = Vec::new();
    for raw in &tool.allow_roots {
        let expanded = expand_root(raw).map_err(|err| SmahError::ToolFileInvalid {
            path: tool.path.display().to_string(),
            detail: format!("allow_roots entry '{raw}': {err}"),
        })?;
        let path = PathBuf::from(&expanded);
        if !path.is_dir() {
            return Err(SmahError::ToolFileInvalid {
                path: tool.path.display().to_string(),
                detail: format!("allow_roots entry '{raw}' is not an existing directory"),
            });
        }
        let canonical = std::fs::canonicalize(&path).map_err(|err| SmahError::ToolFileInvalid {
            path: tool.path.display().to_string(),
            detail: format!("allow_roots entry '{raw}' could not be resolved: {err}"),
        })?;
        roots.push(canonical);
    }
    Ok(roots)
}

/// Expand one root entry: `~/...` (home) or whole-value `$VAR` / `${VAR}`
/// indirection. Relative entries pass through (resolved against the process
/// CWD at invoke time). `$` mid-string is an error (no interpolation).
fn expand_root(raw: &str) -> Result<String, String> {
    if let Some(rest) = raw.strip_prefix("~/") {
        let home =
            home::home_dir().ok_or_else(|| "cannot resolve '~': no home directory".to_string())?;
        return Ok(home.join(rest).display().to_string());
    }
    if raw == "~" {
        return home::home_dir()
            .map(|h| h.display().to_string())
            .ok_or_else(|| "cannot resolve '~': no home directory".to_string());
    }
    if let Some(var) = raw.strip_prefix('$') {
        let var = var.trim_start_matches('{').trim_end_matches('}');
        return std::env::var(var).map_err(|_| format!("environment variable '{var}' is not set"));
    }
    if raw.contains('$') {
        return Err("'$' is only allowed as a whole-value '$VAR' indirection".to_string());
    }
    Ok(raw.to_string())
}
