//! Config-key → provider-implementation registry (Python
//! `smah/providers/registry.py` parity, minus the OS-level termination: the
//! Rust layer returns [`SmahError`] and the CLI owns exiting).
//!
//! Unknown keys and malformed blocks are configuration problems — both map
//! to the §4 config class (exit 3).

use smah_core::errors::SmahError;
use smah_core::settings::Settings;

use crate::anthropic::AnthropicProvider;
use crate::credentials;
use crate::openai::OpenAiProvider;
use crate::Provider;

/// The provider keys SMAH knows how to run, sorted for stable messages.
pub fn known_provider_keys() -> Vec<&'static str> {
    vec!["anthropic", "openai"]
}

/// Resolve a provider implementation by config key.
///
/// Credentials are resolved at construction (see [credentials]); a provider
/// with no resolvable key still constructs — its completion methods return
/// [`SmahError::MissingApiKey`] (exit 4), so probes can distinguish
/// "constructed, keyless" from "not a provider SMAH knows".
///
/// Errors: unknown key → [`SmahError::ConfigGate`] (exit 3, naming the known
/// providers); malformed block → [`SmahError::ConfigUnreadable`] (exit 3).
pub fn get_provider(
    key: &str,
    settings: &Settings,
    cli_key: Option<&str>,
) -> Result<Box<dyn Provider>, SmahError> {
    match key {
        "anthropic" => {
            let api_key = credentials::resolve_api_key(settings, key, cli_key)?;
            Ok(Box::new(AnthropicProvider::new(api_key)?))
        }
        "openai" => {
            let api_key = credentials::resolve_api_key(settings, key, cli_key)?;
            Ok(Box::new(OpenAiProvider::new(api_key)?))
        }
        other => Err(SmahError::ConfigGate {
            path: settings.path().display().to_string(),
            detail: format!(
                "config references the provider '{other}', which SMAH does not know how to run. \
                 Known providers: {}. Fix the provider blocks in the SMAH config",
                known_provider_keys().join(", ")
            ),
        }),
    }
}
