//! Typed views deserialized FROM the raw config Value (plan §B/§D).
//!
//! Views never own the document: they are projections used for
//! `is_configured()` checks and key resolution. Unknown keys are simply not
//! modeled here — they live on untouched in the root
//! [`serde_yaml_ng::Mapping`], which is what gets saved.

use serde::Deserialize;
use serde_yaml_ng::{Mapping, Value};

use super::{env_chain, strip_indirection};

/// A configured model entry (Python `Model` parity for the fields M1 needs).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ModelView {
    /// Catalog id (`anthropic.claude-haiku-5-5` minus provider prefix).
    pub name: Option<String>,
    /// Provider-side model id sent to the API.
    pub model: Option<String>,
    /// Description (unused by M1 checks).
    pub description: Option<String>,
    /// Enabled flag; `None` when the key is absent.
    pub enabled: Option<bool>,
}

impl ModelView {
    /// A model is configured when it has a catalog name and a provider model
    /// id (Python `Model.is_configured`).
    pub fn is_configured(&self) -> bool {
        present(&self.name) && present(&self.model)
    }
}

/// A provider block (Python `Provider` parity for the fields M1 needs).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProviderView {
    /// Provider id (the mapping key, e.g. `anthropic`); filled by
    /// [`InferenceView::provider_list`] — `None` for bare deserialization.
    #[serde(skip)]
    pub identifier: Option<String>,
    /// Display name.
    pub name: Option<String>,
    /// Description (unused by M1 checks).
    pub description: Option<String>,
    /// Enabled flag.
    pub enabled: Option<bool>,
    /// Section schema version.
    pub vsn: Option<String>,
    /// Provider settings mapping (holds `api_key`).
    pub settings: Option<Mapping>,
    /// Model entries in file order.
    #[serde(default)]
    pub models: Vec<ModelView>,
}

impl ProviderView {
    /// Whether the block is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled == Some(true)
    }

    fn is_openai(&self) -> bool {
        self.identifier.as_deref() == Some("openai")
    }

    fn provider_id(&self) -> String {
        self.identifier.clone().unwrap_or_default()
    }

    /// Configured = named, enabled, and every enabled model configured
    /// (Python `Provider.is_configured`, with the enabled check made strict
    /// per plan §G M1: "name+enabled+configured models").
    pub fn is_configured(&self) -> bool {
        if !present(&self.name) || self.enabled != Some(true) {
            return false;
        }
        self.models
            .iter()
            .filter(|m| m.enabled == Some(true))
            .all(|m| m.is_configured())
    }

    /// API-key resolution with an injectable env lookup. `cli_key` is the
    /// `--api-key` flag and only applies to the openai block (Python
    /// `Provider.api_key` parity; enforced by the caller).
    ///
    /// A `settings.api_key` starting with `$` is an indirection: the
    /// variable name is looked up, and when unset the provider env chain
    /// (`SMAH_<PROVIDER>_API_KEY` → `<PROVIDER>_API_KEY`) applies. A literal
    /// value is returned as-is. No `api_key` at all → env chain.
    pub fn api_key(
        &self,
        cli_key: Option<&str>,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Option<String> {
        let from_settings = self
            .settings
            .as_ref()
            .and_then(|s| s.get(Value::String("api_key".to_string())))
            .and_then(|v| v.as_str())
            .filter(|k| !k.is_empty())
            .map(str::to_string);

        let candidate = match cli_key {
            // Only the openai block accepts the CLI key (caller scopes it).
            Some(cli) if self.is_openai() => Some(cli.to_string()).or(from_settings),
            _ => from_settings,
        };

        match candidate {
            Some(key) if key.starts_with('$') => {
                let name = strip_indirection(&key);
                lookup(name).or_else(|| env_chain(&self.provider_id(), lookup))
            }
            Some(key) => Some(key),
            None => env_chain(&self.provider_id(), lookup),
        }
    }
}

/// The `inference` section (Python `Inference` parity for the fields M1
/// needs).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct InferenceView {
    /// Section schema version.
    pub vsn: Option<String>,
    /// Global instructions string.
    pub instructions: Option<String>,
    /// Model picker preferences (task → ordered candidate ids).
    pub model_picker: Option<Value>,
    /// Raw provider mapping, order-preserving.
    #[serde(default)]
    pub providers: Mapping,
}

impl InferenceView {
    /// Providers as `(id, view)` pairs in file order.
    pub fn provider_list(&self) -> Vec<(String, ProviderView)> {
        self.providers
            .iter()
            .filter_map(|(key, value)| {
                let id = key.as_str()?.to_string();
                let mut view: ProviderView = serde_yaml_ng::from_value(value.clone()).ok()?;
                view.identifier = Some(id.clone());
                Some((id, view))
            })
            .collect()
    }

    /// Configured = at least one provider AND every enabled provider
    /// configured (Python `Inference.is_configured`).
    pub fn is_configured(&self) -> bool {
        let providers = self.provider_list();
        if providers.is_empty() {
            return false;
        }
        providers
            .iter()
            .all(|(_, p)| !p.is_enabled() || p.is_configured())
    }
}

/// The `user` section (Python `User` parity).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UserView {
    /// Section schema version.
    pub vsn: Option<String>,
    /// User's name.
    pub name: Option<String>,
    /// System-administration experience level.
    pub system_admin_experience: Option<String>,
    /// User's role.
    pub role: Option<String>,
    /// Free-text about section.
    pub about: Option<String>,
}

impl UserView {
    /// Configured = all four fields present and non-empty (Python `User.
    /// is_configured`).
    pub fn is_configured(&self) -> bool {
        present(&self.name)
            && present(&self.system_admin_experience)
            && present(&self.role)
            && present(&self.about)
    }
}

/// The `system` section. M1 models only what the config stores; the live
/// stats shape (cpu/memory/disk from sysinfo) arrives with the system
/// module.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SystemView {
    /// Section schema version.
    pub vsn: Option<String>,
    /// Login shell.
    pub shell: Option<String>,
    /// Raw operating-system block (kind/name/version/release).
    pub operating_system: Option<Value>,
}

impl SystemView {
    /// Configured = the OS block names a known `kind` (Python `System.
    /// is_configured` delegates to `OperatingSystem.is_configured`).
    pub fn is_configured(&self) -> bool {
        self.operating_system
            .as_ref()
            .and_then(|os| os.get(Value::String("kind".to_string())))
            .and_then(|kind| kind.as_str())
            .map(|kind| !kind.is_empty())
            .unwrap_or(false)
    }
}

/// Python string truthiness: `Some` and non-empty.
fn present(value: &Option<String>) -> bool {
    value.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
}
