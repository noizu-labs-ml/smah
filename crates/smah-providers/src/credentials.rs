//! Credential resolution mirroring the Python seams
//! (`smah/providers/base.py` + `openai_provider.py`).
//!
//! Precedence for a provider present in config (plan §D parity, executed by
//! [`smah_core::settings::views::ProviderView::api_key`]):
//!
//! 1. the OpenAI `--api-key` CLI flag (folds into the `openai` block only),
//! 2. the block's `settings.api_key` — a `$VAR` (legacy) or `env:VAR` value
//!    resolves through the environment and falls through the env chain when
//!    unset; a **secret reference** (`keychain:…`, `file:…#…`) resolves
//!    through the secret store ([RA-10 F2]) with the env chain taking
//!    priority over the stored key (an exported `SMAH_<P>_API_KEY` overrides
//!    the store — CI/one-off override semantics),
//! 3. the env chain `SMAH_<PROVIDER>_API_KEY` → `<PROVIDER>_API_KEY`.
//!
//! A provider block that exists but cannot be deserialized (wrong types,
//! wrong shape) is a **malformed settings block**: exit 3 via
//! [`SmahError::ConfigUnreadable`], never a silent fall-through to env.
//! A secret reference whose store errors is the same exit-3 class (the
//! config points somewhere unreachable); a missing store entry is simply
//! "no key" (exit 4 guidance downstream).
//!
//! A **literal** `api_key` (Python-era configs) still resolves, but `smah
//! doctor` warns and `smah config migrate-secrets` moves it into a store;
//! new writes never produce one (F2, owner decision 2026-11-10).
//!
//! **OpenAI special case:** with no `openai` block in the config there is no
//! key at all — not even from the environment (Python
//! `OpenAIProvider.resolve_api_key` returns early when the block is absent).
//! Every other provider falls to the env chain when its block is absent.
//!
//! Every resolved key — store-fetched, env-chain, or literal — is registered
//! with [`smah_core::logging::register_secret`] the moment it is handled, so
//! no later log line can carry it (F7 extended that to the no-block env
//! path, which the new no-config `smah ask` flow reaches).

use std::path::{Path, PathBuf};

use serde_yaml_ng::Value;
use smah_core::errors::SmahError;
use smah_core::logging;
use smah_core::secrets::{self, SecretRef, SecretStore};
use smah_core::settings::views::ProviderView;
use smah_core::settings::{env_chain, Settings};

/// Resolve the API key for `key` against the real environment and the real
/// secret stores.
pub fn resolve_api_key(
    settings: &Settings,
    key: &str,
    cli_key: Option<&str>,
) -> Result<Option<String>, SmahError> {
    resolve_api_key_with_store(
        settings,
        key,
        cli_key,
        &|name| std::env::var(name).ok(),
        &secrets::RuntimeStore,
    )
}

/// Resolve the API key with an injectable env lookup (testable).
pub fn resolve_api_key_with(
    settings: &Settings,
    key: &str,
    cli_key: Option<&str>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<String>, SmahError> {
    resolve_api_key_with_store(settings, key, cli_key, lookup, &secrets::RuntimeStore)
}

/// Resolve the API key with injectable env lookup **and** secret store —
/// the full test seam (tests inject a mock store; CI never touches a real
/// keychain).
pub fn resolve_api_key_with_store(
    settings: &Settings,
    key: &str,
    cli_key: Option<&str>,
    lookup: &dyn Fn(&str) -> Option<String>,
    store: &dyn SecretStore,
) -> Result<Option<String>, SmahError> {
    match provider_block(settings, key) {
        // No block: OpenAI has no key at all; everyone else falls to env.
        None => {
            if key == "openai" {
                return Ok(None);
            }
            let resolved = env_chain(key, lookup);
            if let Some(secret) = &resolved {
                logging::register_secret(secret.clone());
            }
            Ok(resolved)
        }
        Some(block) => {
            let mut view: ProviderView =
                serde_yaml_ng::from_value(block).map_err(|err| SmahError::ConfigUnreadable {
                    path: settings.path().display().to_string(),
                    detail: one_line(&format!("the '{key}' provider block is malformed: {err}")),
                })?;
            view.identifier = Some(key.to_string());
            let api_key = view.api_key(cli_key, lookup);
            // `$VAR` values are already resolved by the view; a reference is
            // handed back raw and resolved here.
            if let Some(raw) = api_key.as_ref() {
                if !raw.starts_with('$') {
                    if let Some(reference) = secrets::parse_secret_ref(raw, &config_dir(settings)) {
                        return resolve_reference(&reference, key, lookup, store);
                    }
                }
            }
            // Literal or legacy `$VAR`: register whatever resolved.
            if let Some(resolved) = &api_key {
                logging::register_secret(resolved.clone());
            }
            Ok(api_key)
        }
    }
}

/// Resolve one reference. Env references go through the lookup (chain
/// fall-through mirrors `$VAR`); store references honor an env-chain
/// override first, then the store. Every resolution registers with the
/// redactor before returning.
fn resolve_reference(
    reference: &SecretRef,
    key: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    store: &dyn SecretStore,
) -> Result<Option<String>, SmahError> {
    match reference {
        SecretRef::EnvVar(name) => {
            // One explicit variable; unset means unset (no chain probing
            // for the modern spelling — legacy `$VAR` keeps its
            // view-level chain fall-through).
            let resolved = lookup(name);
            if let Some(secret) = &resolved {
                logging::register_secret(secret.clone());
            }
            Ok(resolved)
        }
        SecretRef::Keyring { .. } | SecretRef::File { .. } => {
            // An exported provider env var wins over the referenced store.
            if let Some(override_key) = env_chain(key, lookup) {
                logging::register_secret(override_key.clone());
                return Ok(Some(override_key));
            }
            match store.get(reference) {
                Ok(Some(secret)) => {
                    logging::register_secret(secret.clone());
                    Ok(Some(secret))
                }
                Ok(None) => Ok(None),
                Err(detail) => Err(SmahError::ConfigUnreadable {
                    path: settingsless_path(key),
                    detail: one_line(&format!(
                        "the '{key}' key lives in a secret store that cannot be reached: {detail}"
                    )),
                }),
            }
        }
    }
}

/// The config directory (anchors `file:` references).
fn config_dir(settings: &Settings) -> PathBuf {
    settings
        .path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Placeholder path for store-unreachable errors (the real store location
/// lives in the detail line).
fn settingsless_path(key: &str) -> String {
    format!("secret-store:<{key}>")
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

#[cfg(test)]
mod tests {
    use super::*;
    use smah_core::secrets::MockStore;

    fn lookup_of(entries: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name: &str| {
            owned
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    fn settings_with(api_key: &str) -> Settings {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.yaml");
        let text = format!(
            "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    anthropic:\n      name: Anthropic\n      enabled: true\n      settings:\n        api_key: {api_key}\n      models: []\n"
        );
        std::fs::write(&path, text).expect("write");
        Settings::load_from(&path).expect("load")
    }

    fn no_block_settings() -> Settings {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "vsn: \"0.0.1\"\n").expect("write");
        Settings::load_from(&path).expect("load")
    }

    const KEY: &str = "test-key-0000";

    #[test]
    fn literal_key_still_resolves_and_registers() {
        let settings = settings_with(KEY);
        let resolved =
            resolve_api_key_with(&settings, "anthropic", None, &lookup_of(&[])).expect("resolve");
        assert_eq!(resolved.as_deref(), Some(KEY));
        assert!(
            logging::is_registered(KEY),
            "literal registered with the redactor"
        );
    }

    #[test]
    fn keychain_reference_resolves_through_the_store() {
        let settings = settings_with("keychain:smah/anthropic");
        let reference = SecretRef::Keyring {
            service: "smah".to_string(),
            account: "anthropic".to_string(),
        };
        let store = MockStore::new().with_entry(&reference, KEY);
        let resolved =
            resolve_api_key_with_store(&settings, "anthropic", None, &lookup_of(&[]), &store)
                .expect("resolve");
        assert_eq!(resolved.as_deref(), Some(KEY));
        assert!(logging::is_registered(KEY), "store key registered");
    }

    #[test]
    fn env_chain_overrides_the_referenced_store() {
        let settings = settings_with("keychain:smah/anthropic");
        let reference = SecretRef::Keyring {
            service: "smah".to_string(),
            account: "anthropic".to_string(),
        };
        let store = MockStore::new().with_entry(&reference, "stored-key");
        let lookup = lookup_of(&[("SMAH_ANTHROPIC_API_KEY", KEY)]);
        let resolved = resolve_api_key_with_store(&settings, "anthropic", None, &lookup, &store)
            .expect("resolve");
        assert_eq!(
            resolved.as_deref(),
            Some(KEY),
            "env var wins over the store"
        );
    }

    #[test]
    fn missing_store_entry_means_no_key() {
        let settings = settings_with("keychain:smah/anthropic");
        let store = MockStore::new();
        let resolved =
            resolve_api_key_with_store(&settings, "anthropic", None, &lookup_of(&[]), &store)
                .expect("resolve");
        assert_eq!(resolved, None, "absent entry is keyless, not an error");
    }

    #[test]
    fn store_error_is_a_config_class_error() {
        struct FailingStore;
        impl SecretStore for FailingStore {
            fn kind(&self) -> secrets::StoreKind {
                secrets::StoreKind::Runtime
            }
            fn describe(&self) -> String {
                "failing".to_string()
            }
            fn get(&self, _: &SecretRef) -> Result<Option<String>, String> {
                Err("bus unavailable".to_string())
            }
            fn set(&self, _: &SecretRef, _: &str) -> Result<(), String> {
                unreachable!()
            }
            fn delete(&self, _: &SecretRef) -> Result<(), String> {
                unreachable!()
            }
        }
        let settings = settings_with("keychain:smah/anthropic");
        let err = resolve_api_key_with_store(
            &settings,
            "anthropic",
            None,
            &lookup_of(&[]),
            &FailingStore,
        )
        .expect_err("store error propagates");
        assert!(matches!(err, SmahError::ConfigUnreadable { .. }), "{err}");
    }

    #[test]
    fn file_reference_resolves_against_the_config_dir() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.yaml");
        let secrets_path = dir.path().join("secrets.yaml");
        std::fs::write(
            &path,
            "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    anthropic:\n      name: Anthropic\n      enabled: true\n      settings:\n        api_key: file:secrets.yaml#anthropic\n      models: []\n",
        )
        .expect("write");
        std::fs::write(&secrets_path, "anthropic: test-key-0000\n").expect("secrets");
        let settings = Settings::load_from(&path).expect("load");
        let store = secrets::FileStore {
            path: secrets_path.clone(),
        };
        let resolved =
            resolve_api_key_with_store(&settings, "anthropic", None, &lookup_of(&[]), &store)
                .expect("resolve");
        assert_eq!(resolved.as_deref(), Some(KEY));
    }

    #[test]
    fn env_reference_resolves_and_registers() {
        let settings = settings_with("env:MY_KEY");
        let lookup = lookup_of(&[("MY_KEY", KEY)]);
        let resolved =
            resolve_api_key_with(&settings, "anthropic", None, &lookup).expect("resolve");
        assert_eq!(resolved.as_deref(), Some(KEY));
        assert!(logging::is_registered(KEY));
    }

    #[test]
    fn env_chain_registers_even_without_a_provider_block() {
        // F7: the no-block branch (reached by no-config `smah ask`) must
        // register the env key with the redactor too.
        let settings = no_block_settings();
        let lookup = lookup_of(&[("ANTHROPIC_API_KEY", KEY)]);
        let resolved =
            resolve_api_key_with(&settings, "anthropic", None, &lookup).expect("resolve");
        assert_eq!(resolved.as_deref(), Some(KEY));
        assert!(logging::is_registered(KEY), "F7: env-chain key registered");
    }

    #[test]
    fn openai_without_a_block_has_no_key() {
        let settings = no_block_settings();
        let lookup = lookup_of(&[("OPENAI_API_KEY", KEY)]);
        let resolved = resolve_api_key_with(&settings, "openai", None, &lookup).expect("resolve");
        assert_eq!(resolved, None, "openai stays keyless without a block");
    }
}
