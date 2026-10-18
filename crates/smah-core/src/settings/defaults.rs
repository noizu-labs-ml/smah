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
    }
}
