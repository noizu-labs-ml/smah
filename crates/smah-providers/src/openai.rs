//! Hand-rolled OpenAI chat-completions client (Python
//! `smah/providers/openai_provider.py` parity, minus the SDK).
//!
//! The wire engine lives in the shared OpenAI-compatible core
//! (`crate::openai_compat`); this module pins the `openai` identity: config
//! key, base-URL env var, and the default API base.
//!
//! Credential note: construction does not consult the environment for the
//! key (that is [credentials](crate::credentials)); without an `openai`
//! config block there is no key at all.

use smah_core::errors::SmahError;

use crate::openai_compat::{resolve_base_url, OpenAiCompat};
use crate::{Completion, CompletionRequest, Provider};

/// Config key and provider id used in every §4 message.
const KEY: &str = "openai";

/// Default API base, versioned path included (`OPENAI_BASE_URL` overrides
/// the whole base; config-file base URLs are NOT supported — the env var is
/// the mock-server test seam, by deliberate plan decision).
pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Env var honored as the API base override.
pub const OPENAI_BASE_URL_VAR: &str = "OPENAI_BASE_URL";

/// Hand-rolled OpenAI client. See the [Provider] trait for the contract;
/// this type is `pub` so tests and diagnostics can pin the base URL.
pub struct OpenAiProvider {
    compat: OpenAiCompat,
}

impl OpenAiProvider {
    /// Construct against the default (or `OPENAI_BASE_URL`) base.
    pub fn new(api_key: Option<String>) -> Result<Self, SmahError> {
        Ok(OpenAiProvider {
            compat: OpenAiCompat::new(KEY, api_key, base_url_from_env())?,
        })
    }

    /// Construct against an explicit base URL (test seam; overrides the env
    /// var). Trailing slashes are tolerated.
    pub fn with_base_url(api_key: Option<String>, base_url: String) -> Result<Self, SmahError> {
        Ok(OpenAiProvider {
            compat: OpenAiCompat::new(KEY, api_key, base_url)?,
        })
    }

    /// The base URL requests resolve against.
    pub fn base_url(&self) -> &str {
        self.compat.base_url()
    }
}

impl Provider for OpenAiProvider {
    fn key(&self) -> &'static str {
        KEY
    }

    fn has_credentials(&self) -> bool {
        self.compat.has_credentials()
    }

    fn complete(&self, req: &CompletionRequest) -> Result<Completion, SmahError> {
        self.compat.complete(req)
    }

    fn complete_streaming(
        &self,
        req: &CompletionRequest,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<Completion, SmahError> {
        self.compat.complete_streaming(req, on_token)
    }
}

/// Resolve the base URL: `OPENAI_BASE_URL` when set and non-empty, the
/// default otherwise.
pub fn base_url_from_env() -> String {
    resolve_base_url(OPENAI_BASE_URL_VAR, OPENAI_DEFAULT_BASE_URL)
}
