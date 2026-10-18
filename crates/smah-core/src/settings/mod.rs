//! Value-centric settings: `~/.smah/config.yaml` load/save with order and
//! unknown-key parity, string-compare vsn gate, `$ENV_VAR` indirection, and
//! redacted display.
//!
//! The raw [`serde_yaml_ng::Mapping`] is the source of truth (plan §D):
//! typed views ([`views`]) are deserialized *from* the Value and unknown
//! keys survive a load/save cycle byte-for-byte — the Python rewrite drops
//! them; the Rust save deliberately does not. Key insertion order is
//! preserved by the mapping, so `vsn,user,system,inference` stays first as
//! Python's `save` emits it.
//!
//! Gate: `vsn <= "0.0.1"` as a **string compare** on an `Option<String>`
//! (Python parity; not semver). Missing or empty vsn is unsupported.

mod defaults;
pub mod views;

use std::fs;
use std::path::{Path, PathBuf};

use serde_yaml_ng::{Mapping, Value};

use crate::errors::SmahError;

pub use defaults::{inference_defaults, resolve_display_model, INFERENCE_DEFAULTS_YAML};

/// Current config schema version (Python parity: `Settings.CONFIG_VSN`).
pub const CONFIG_VSN: &str = "0.0.1";

/// Keys whose values are secrets for [`Settings::show_redacted`]
/// (plan §D). Matched case-insensitively against mapping keys.
pub const SECRET_KEYS: [&str; 6] = ["api_key", "apikey", "key", "token", "secret", "password"];

/// Mask substituted for literal secret values in redacted output.
const SECRET_MASK: &str = "••••••••";

/// Loaded settings: the raw Value plus its resolved path.
#[derive(Debug, Clone)]
pub struct Settings {
    path: PathBuf,
    value: Mapping,
    loaded: bool,
}

impl Settings {
    /// Default config path: `~/.smah/config.yaml`.
    pub fn default_path() -> PathBuf {
        let mut path = home::home_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push(".smah");
        path.push("config.yaml");
        path
    }

    /// Load from the default path (`--config` uses [`Settings::load_from`]).
    pub fn load() -> Result<Self, SmahError> {
        Self::load_from(&Self::default_path())
    }

    /// Load from an explicit path (the `--config` override).
    ///
    /// Missing file → an empty, not-loaded [`Settings`] (Python parity: the
    /// first-run wizard owns creation). Parse/structure failures →
    /// [`SmahError::ConfigUnreadable`]; gate rejection →
    /// [`SmahError::ConfigGate`] (both exit 3).
    pub fn load_from(path: &Path) -> Result<Self, SmahError> {
        let path_str = path.to_string_lossy().into_owned();
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Settings {
                    path: path.to_path_buf(),
                    value: Mapping::new(),
                    loaded: false,
                });
            }
            Err(err) => {
                return Err(SmahError::ConfigUnreadable {
                    path: path_str,
                    detail: err.to_string(),
                });
            }
        };

        let value: Value =
            serde_yaml_ng::from_str(&text).map_err(|err| SmahError::ConfigUnreadable {
                path: path_str.clone(),
                detail: one_line(&err.to_string()),
            })?;

        let mapping = match value {
            Value::Mapping(mapping) => mapping,
            other => {
                return Err(SmahError::ConfigUnreadable {
                    path: path_str,
                    detail: format!("config root must be a mapping, got {}", type_name(&other)),
                });
            }
        };

        let vsn = root_vsn(&mapping);
        if !Self::vsn_supported(&vsn) {
            return Err(SmahError::ConfigGate {
                path: path_str,
                detail: format!(
                    "config version {} is not supported by this version of SMAH",
                    vsn.unwrap_or_else(|| "<missing>".to_string())
                ),
            });
        }

        Ok(Settings {
            path: path.to_path_buf(),
            value: mapping,
            loaded: true,
        })
    }

    /// Version gate: string compare `vsn <= "0.0.1"`. `None` and empty are
    /// unsupported (Python `if not vsn` parity).
    pub fn vsn_supported(vsn: &Option<String>) -> bool {
        match vsn {
            Some(vsn) => !vsn.is_empty() && vsn.as_str() <= CONFIG_VSN,
            None => false,
        }
    }

    /// The file this settings object loads from / saves to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether a config file existed at load time.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// The raw root mapping (order-preserving). Read-only access for tests
    /// and callers that need keys the typed views do not model.
    pub fn value(&self) -> &Mapping {
        &self.value
    }

    /// Root `vsn` value.
    pub fn vsn(&self) -> Option<String> {
        root_vsn(&self.value)
    }

    /// Deserialize a typed view from a top-level section of the Value.
    pub fn view<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let section = self.value.get(Value::String(key.to_string()))?;
        serde_yaml_ng::from_value(section.clone()).ok()
    }

    /// Typed user view, deserialized from the Value.
    pub fn user(&self) -> Option<views::UserView> {
        self.view("user")
    }

    /// Typed system view, deserialized from the Value.
    pub fn system(&self) -> Option<views::SystemView> {
        self.view("system")
    }

    /// Typed inference view, deserialized from the Value.
    pub fn inference(&self) -> Option<views::InferenceView> {
        self.view("inference")
    }

    /// Fully configured = user AND system AND inference all configured
    /// (Python `Settings.is_configured` parity).
    pub fn is_configured(&self) -> bool {
        let user_ok = self.user().map(|u| u.is_configured()).unwrap_or(false);
        let system_ok = self.system().map(|s| s.is_configured()).unwrap_or(false);
        let inference_ok = self.inference().map(|i| i.is_configured()).unwrap_or(false);
        user_ok && system_ok && inference_ok
    }

    /// Save the Value back to [`Settings::path`]: insertion order preserved,
    /// unknown keys preserved, live system-stats snapshot refreshed first.
    pub fn save(&mut self) -> Result<(), SmahError> {
        refresh_system_stats(&mut self.value);
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| SmahError::Internal {
                detail: format!(
                    "failed to create config directory {}: {}",
                    parent.display(),
                    err
                ),
            })?;
        }
        let text =
            serde_yaml_ng::to_string(&Value::Mapping(self.value.clone())).map_err(|err| {
                SmahError::Internal {
                    detail: format!("failed to serialize config: {}", err),
                }
            })?;
        fs::write(&self.path, text).map_err(|err| SmahError::Internal {
            detail: format!("failed to write config {}: {}", self.path.display(), err),
        })?;
        Ok(())
    }

    /// The config re-dumped with every secret-bearing value masked:
    /// `$VAR` indirections → `<env:VAR>`, literal values → `••••••••`.
    pub fn show_redacted(&self) -> String {
        let redacted = redact_value(&Value::Mapping(self.value.clone()));
        serde_yaml_ng::to_string(&redacted).unwrap_or_default()
    }

    /// Resolve the API key for `provider_id` against the real environment.
    /// See [`Settings::provider_api_key_with`] for the precedence rules.
    pub fn provider_api_key(&self, provider_id: &str, cli_key: Option<&str>) -> Option<String> {
        self.provider_api_key_with(provider_id, cli_key, &|name| std::env::var(name).ok())
    }

    /// Resolve the API key with an injectable env lookup (testable).
    ///
    /// Precedence (plan §D, `provider.py:43-58` parity):
    /// 1. OpenAI only: the `--api-key` CLI flag folds into the openai block.
    /// 2. The block's `settings.api_key`; a `$VAR` value resolves through the
    ///    environment, falling through `SMAH_<PROVIDER>_API_KEY` then
    ///    `<PROVIDER>_API_KEY` when that variable is unset.
    /// 3. Otherwise the env chain `SMAH_<PROVIDER>_API_KEY` →
    ///    `<PROVIDER>_API_KEY`.
    ///
    /// **No config block ⇒ no key** (the OpenAI special case: an absent
    /// `openai` block yields `None` even with `OPENAI_API_KEY` exported,
    /// because resolution only ever runs for providers present in config).
    pub fn provider_api_key_with(
        &self,
        provider_id: &str,
        cli_key: Option<&str>,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Option<String> {
        let provider_value = self
            .value
            .get(Value::String("inference".to_string()))?
            .get(Value::String("providers".to_string()))?
            .get(Value::String(provider_id.to_string()))?;
        let mut view: views::ProviderView =
            serde_yaml_ng::from_value(provider_value.clone()).ok()?;
        view.identifier = Some(provider_id.to_string());
        view.api_key(cli_key, lookup)
    }
}

/// Live system-stats snapshot hook. **M1 stub**: the Value passes through
/// untouched; the real sysinfo refresh (cpu/memory/disk readings + shell,
/// Python `System.to_yaml({"stats": true})` shape) lands with the system
/// module. Kept as a named hook so `save()`'s contract does not change.
pub fn refresh_system_stats(_value: &mut Mapping) {}

/// Root `vsn` as a string, if present.
fn root_vsn(mapping: &Mapping) -> Option<String> {
    mapping
        .get(Value::String("vsn".to_string()))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// `$VAR` indirection strip, replicating Python
/// `k.lstrip("$").lstrip("{").rstrip("}")`: all leading `$`/`{` and all
/// trailing `}` characters removed.
pub fn strip_indirection(value: &str) -> &str {
    value.trim_start_matches(['$', '{']).trim_end_matches('}')
}

/// Provider env-var fallback chain: `SMAH_<PROVIDER>_API_KEY` →
/// `<PROVIDER>_API_KEY`.
pub fn env_chain(provider_id: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let prefix = provider_id.to_uppercase();
    lookup(&format!("SMAH_{prefix}_API_KEY")).or_else(|| lookup(&format!("{prefix}_API_KEY")))
}

/// Redacted scalar for a secret value: `$VAR` → `<env:VAR>`, literal → mask.
fn redact_secret(value: &str) -> String {
    if value.starts_with('$') {
        format!("<env:{}>", strip_indirection(value))
    } else {
        SECRET_MASK.to_string()
    }
}

/// Recursively mask secret-bearing mapping values (case-insensitive key
/// match against [`SECRET_KEYS`]); structure and order preserved.
fn redact_value(value: &Value) -> Value {
    match value {
        Value::Mapping(mapping) => Value::Mapping(
            mapping
                .iter()
                .map(|(key, val)| {
                    let is_secret = key
                        .as_str()
                        .map(|k| {
                            let lowered = k.to_ascii_lowercase();
                            SECRET_KEYS.contains(&lowered.as_str())
                        })
                        .unwrap_or(false);
                    let masked = match val.as_str() {
                        Some(s) if is_secret => Value::String(redact_secret(s)),
                        _ => redact_value(val),
                    };
                    (key.clone(), masked)
                })
                .collect(),
        ),
        Value::Sequence(seq) => Value::Sequence(seq.iter().map(redact_value).collect()),
        other => other.clone(),
    }
}

/// Collapse an error string to a single line (§4: `<detail>` is one line).
fn one_line(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
}

/// YAML type label for the "root must be a mapping" diagnostic.
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Sequence(_) => "a sequence",
        Value::Tagged(_) => "a tagged value",
        Value::Mapping(_) => "a mapping",
    }
}
