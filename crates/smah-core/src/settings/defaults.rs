//! Embedded inference defaults catalog (Python
//! `smah/settings/inference/inference_defaults.yaml`, embedded byte-exact).
//!
//! M1 parses the YAML into a [`serde_yaml_ng::Value`] only; catalog typing
//! (Model entries, picker tables) arrives with the models milestone. Parsing
//! is done once into a process-lifetime static.

use std::sync::OnceLock;

use serde_yaml_ng::Value;

/// The embedded defaults file, byte-exact from the Python tree.
pub const INFERENCE_DEFAULTS_YAML: &str =
    include_str!("../../../../smah/settings/inference/inference_defaults.yaml");

fn defaults_cell() -> &'static OnceLock<Value> {
    static DEFAULTS: OnceLock<Value> = OnceLock::new();
    &DEFAULTS
}

/// Parse (once) and return the embedded inference defaults. Panics only if
/// the embedded file stops being valid YAML — covered by a unit test.
pub fn inference_defaults() -> &'static Value {
    defaults_cell().get_or_init(|| {
        serde_yaml_ng::from_str(INFERENCE_DEFAULTS_YAML)
            .expect("embedded inference_defaults.yaml must parse")
    })
}

/// Resolve a provider catalog **display name** to its raw model id,
/// case-insensitively ("Qwen 3.8 Flash" → `qwen3.8-flash`). `None` when the
/// provider is not in the catalog or no entry's `name` matches — callers
/// pass the input through unchanged, so raw ids keep working and unknown
/// names surface as the provider's own not-found error.
pub fn resolve_display_model(provider: &str, model: &str) -> Option<String> {
    let models = inference_defaults()
        .get("providers")?
        .get(provider)?
        .get("models")?
        .as_sequence()?;
    models
        .iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?;
            let id = entry.get("model")?.as_str()?;
            name.eq_ignore_ascii_case(model).then(|| id.to_string())
        })
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_defaults_parse() {
        let defaults = inference_defaults();
        let providers = defaults
            .get(Value::String("providers".to_string()))
            .and_then(|p| p.as_mapping())
            .expect("defaults must expose a providers mapping");
        assert!(!providers.is_empty(), "defaults must list providers");
        assert!(
            providers.contains_key(Value::String("anthropic".to_string())),
            "anthropic must be in the defaults catalog"
        );
        assert!(
            providers.contains_key(Value::String("openai".to_string())),
            "openai must be in the defaults catalog"
        );
        assert!(
            providers.contains_key(Value::String("alibaba".to_string())),
            "alibaba must be in the defaults catalog"
        );
        // The alibaba entry carries the qwen 3.8 flash model (virtual-tools
        // e2e), enabled.
        let alibaba_models = providers
            .get(Value::String("alibaba".to_string()))
            .and_then(|p| p.get(Value::String("models".to_string())))
            .and_then(Value::as_sequence)
            .expect("alibaba models list");
        let qwen = alibaba_models
            .iter()
            .find(|m| {
                m.get(Value::String("model".to_string()))
                    .and_then(Value::as_str)
                    == Some("qwen3.8-flash")
            })
            .expect("qwen3.8-flash model entry");
        assert_eq!(
            qwen.get(Value::String("name".to_string()))
                .and_then(Value::as_str),
            Some("Qwen 3.8 Flash")
        );
        assert_eq!(
            qwen.get(Value::String("enabled".to_string()))
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn display_names_resolve_case_insensitively_unknown_stay_none() {
        assert_eq!(
            resolve_display_model("alibaba", "Qwen 3.8 Flash"),
            Some("qwen3.8-flash".to_string())
        );
        assert_eq!(
            resolve_display_model("alibaba", "qwen 3.8 FLASH"),
            Some("qwen3.8-flash".to_string())
        );
        // A raw id is not a display name: no match, caller passes through.
        assert_eq!(resolve_display_model("alibaba", "qwen3.8-flash"), None);
        assert_eq!(
            resolve_display_model("alibaba", "Definitely Not A Model"),
            None
        );
        assert_eq!(resolve_display_model("no-such-provider", "anything"), None);
    }
}
