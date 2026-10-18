//! Credential resolution mirroring the Python seams
//! (`smah/providers/base.py` + `openai_provider.py`).
//!
//! Precedence for a provider present in config (plan §D parity, executed by
//! [`smah_core::settings::views::ProviderView::api_key`]):
//!
//! 1. the OpenAI `--api-key` CLI flag (folds into the `openai` block only),
//! 2. the block's `settings.api_key` — a `$VAR` value resolves through the
//!    environment and falls through the env chain when unset,
//! 3. the env chain `SMAH_<PROVIDER>_API_KEY` → `<PROVIDER>_API_KEY`.
//!
//! **OpenAI special case:** with no `openai` block in the config there is no
//! key at all — not even from the environment (Python
//! `OpenAIProvider.resolve_api_key` returns early when the block is absent).
//! Every other provider falls to the env chain when its block is absent.
//!
//! A provider block that exists but cannot be deserialized (wrong types,
//! wrong shape) is a **malformed settings block**: exit 3 via
//! [`SmahError::ConfigUnreadable`], never a silent fall-through to env.
//!
//! Every resolved key is registered with [`smah_core::logging::register_secret`]
//! the moment it is handled, so no later log line can carry it.

use serde_yaml_ng::Value;
use smah_core::errors::SmahError;
use smah_core::logging;
use smah_core::settings::views::ProviderView;
use smah_core::settings::{env_chain, Settings};

/// Resolve the API key for `key` against the real environment.
pub fn resolve_api_key(
    settings: &Settings,
    key: &str,
    cli_key: Option<&str>,
) -> Result<Option<String>, SmahError> {
    resolve_api_key_with(settings, key, cli_key, &|name| std::env::var(name).ok())
}

/// Resolve the API key with an injectable env lookup (testable).
pub fn resolve_api_key_with(
    settings: &Settings,
    key: &str,
    cli_key: Option<&str>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<String>, SmahError> {
    match provider_block(settings, key) {
        // No block: OpenAI has no key at all; everyone else falls to env.
        None => {
            if key == "openai" {
                Ok(None)
            } else {
                Ok(env_chain(key, lookup))
            }
        }
        Some(block) => {
            let mut view: ProviderView =
                serde_yaml_ng::from_value(block).map_err(|err| SmahError::ConfigUnreadable {
                    path: settings.path().display().to_string(),
                    detail: one_line(&format!("the '{key}' provider block is malformed: {err}")),
                })?;
            view.identifier = Some(key.to_string());
            let api_key = view.api_key(cli_key, lookup);
            if let Some(resolved) = &api_key {
                logging::register_secret(resolved);
            }
            Ok(api_key)
        }
    }
}

/// The raw provider block from `inference.providers.<key>`, if present.
///
/// A YAML `null` block counts as absent (Python `config_data or {}` parity:
/// an empty block behaves like no block).
fn provider_block(settings: &Settings, key: &str) -> Option<Value> {
    let block = settings
        .value()
        .get(Value::String("inference".to_string()))?
        .get(Value::String("providers".to_string()))?
        .get(Value::String(key.to_string()))?
        .clone();
    match block {
        Value::Null => None,
        other => Some(other),
    }
}

/// Collapse whitespace to a single line (§4: `<detail>` is one line).
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
