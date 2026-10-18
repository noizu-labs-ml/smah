//! M2 provider adapter (plan §0): the **sole file allowed to know the
//! provider layer's real types**. Everything upstream of this — the driver
//! loop, the tools, the tests — speaks [`ChatBackend`], so provider drift is
//! a one-file fix.
//!
//! Model-spec shape (interface-spec §1): `"<provider>.<model-id>"` or a bare
//! model id under the default provider. The split is by known provider key,
//! so adding a provider (e.g. the planned `alibaba` OpenAI-compatible key)
//! is registry/config work, not a code change here.

use smah_core::errors::SmahError;
use smah_core::virtual_tools::driver::{ChatBackend, ChatMessage, Role};
use smah_providers::{
    registry::known_provider_keys, ChatMessage as WireMessage, CompletionRequest, MessageRole,
    Provider,
};

/// Default provider for bare model ids (spec decision 2's answer model).
pub const DEFAULT_PROVIDER: &str = "anthropic";

/// Default model id when neither the invocation nor the frontmatter names
/// one (interface-spec decision 2: the answer model).
pub const DEFAULT_MODEL_ID: &str = "claude-opus-5-5";

/// A [`ChatBackend`] over an M2 [`Provider`].
pub struct ProviderBackend {
    provider: Box<dyn Provider>,
}

impl ProviderBackend {
    /// Wrap a resolved provider.
    pub fn new(provider: Box<dyn Provider>) -> Self {
        ProviderBackend { provider }
    }
}

impl ChatBackend for ProviderBackend {
    fn stream_turn(
        &self,
        model: &str,
        system: &str,
        messages: &[ChatMessage],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, SmahError> {
        let wire_messages = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::User => MessageRole::User,
                    Role::Assistant => MessageRole::Assistant,
                };
                WireMessage::new(role, m.content.clone())
            })
            .collect();
        let mut request = CompletionRequest::new(model, wire_messages);
        request.system = Some(system.to_string());
        let completion = self.provider.complete_streaming(&request, on_delta)?;
        Ok(completion.text)
    }
}

/// Split a model spec into `(provider key, provider-side model id)`.
///
/// The first dot separates the provider **only when the prefix is a known
/// provider key**; a bare id containing dots (e.g. `gpt-4.1-mini`) stays a
/// bare id under the default provider. Unknown provider prefixes are a
/// settings problem (`smah-providers` maps them to exit 3), not a parse one.
///
/// The provider-side model part is resolved through the embedded defaults
/// catalog when it names a display name ("alibaba.Qwen 3.8 Flash" →
/// `qwen3.8-flash`); anything else — a raw id, or a name the catalog does
/// not know — passes through unchanged so the provider's own not-found
/// error (exit 5) is unchanged.
pub fn split_model_spec(spec: &str, default_provider: &str) -> (String, String) {
    if let Some((prefix, rest)) = spec.split_once('.') {
        let known = known_provider_keys();
        if !rest.is_empty() && known.contains(&prefix) {
            return (prefix.to_string(), resolve_model_id(prefix, rest));
        }
    }
    (default_provider.to_string(), spec.to_string())
}

/// Catalog resolution for the provider-side model part: a display name maps
/// to its raw id; everything else is passed through as-is.
fn resolve_model_id(provider_key: &str, model: &str) -> String {
    smah_core::settings::resolve_display_model(provider_key, model)
        .unwrap_or_else(|| model.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_resolves_through_the_registry() {
        assert_eq!(
            split_model_spec("alibaba.Qwen 3.8 Flash", "anthropic"),
            ("alibaba".to_string(), "qwen3.8-flash".to_string())
        );
        assert_eq!(
            split_model_spec("alibaba.qwen 3.8 FLASH", "anthropic"),
            ("alibaba".to_string(), "qwen3.8-flash".to_string())
        );
    }

    #[test]
    fn raw_ids_pass_through_unchanged() {
        assert_eq!(
            split_model_spec("alibaba.qwen3.8-flash", "anthropic"),
            ("alibaba".to_string(), "qwen3.8-flash".to_string())
        );
        assert_eq!(
            split_model_spec("anthropic.claude-opus-5-5", "anthropic"),
            ("anthropic".to_string(), "claude-opus-5-5".to_string())
        );
    }

    #[test]
    fn unknown_display_name_stays_raw() {
        // The provider 404s on it (exit 5) — same as before, no new class.
        assert_eq!(
            split_model_spec("alibaba.Definitely Not A Model", "anthropic"),
            ("alibaba".to_string(), "Definitely Not A Model".to_string())
        );
    }

    #[test]
    fn bare_ids_keep_the_default_provider_path() {
        assert_eq!(
            split_model_spec("gpt-4.1-mini", "anthropic"),
            ("anthropic".to_string(), "gpt-4.1-mini".to_string())
        );
        assert_eq!(
            split_model_spec("claude-opus-5-5", "anthropic"),
            ("anthropic".to_string(), "claude-opus-5-5".to_string())
        );
    }
}
