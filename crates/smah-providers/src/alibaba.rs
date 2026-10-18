//! Alibaba Cloud DashScope provider — OpenAI-compatible client over the
//! shared engine in `crate::openai_compat` (virtual-tools e2e unblock).
//!
//! DashScope exposes an OpenAI-compatible chat-completions surface
//! (bearer-token auth, `chat.completion.chunk` SSE streaming, `[DONE]`
//! sentinel), so the wire behavior is exactly the OpenAI client's — only
//! the identity differs: config key `alibaba`, base-URL env
//! `ALIBABA_BASE_URL`, default base
//! `https://dashscope.aliyuncs.com/compatible-mode/v1`.
//!
//! Credentials resolve through the standard [credentials](crate::credentials)
//! chain: config `inference.providers.alibaba.settings.api_key` (with
//! `$ENV` indirection) → `SMAH_ALIBABA_API_KEY` → `ALIBABA_API_KEY`.

use smah_core::errors::SmahError;

use crate::openai_compat::{resolve_base_url, OpenAiCompat};
use crate::{Completion, CompletionRequest, Provider};

/// Config key and provider id used in every §4 message.
const KEY: &str = "alibaba";

/// Default API base: the DashScope public compatible-mode endpoint,
/// versioned path included (`ALIBABA_BASE_URL` overrides the whole base).
///
/// Env-only override, mirroring the `OPENAI_BASE_URL`/`ANTHROPIC_BASE_URL`
/// precedent; config-file base URLs are deliberately unsupported. Reviewer
/// note: at runtime a dedicated MAAS endpoint host may be supplied via
/// `ALIBABA_BASE_URL` (scheme + path included, e.g.
/// `https://<maas-host>/compatible-mode/v1`) — that is the intended use.
pub const ALIBABA_DEFAULT_BASE_URL: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";

/// Env var honored as the API base override.
pub const ALIBABA_BASE_URL_VAR: &str = "ALIBABA_BASE_URL";

/// Alibaba Cloud DashScope client (OpenAI-compatible). See the [Provider]
/// trait for the contract; this type is `pub` so tests and diagnostics can
/// pin the base URL.
pub struct AlibabaProvider {
    compat: OpenAiCompat,
}

impl AlibabaProvider {
    /// Construct against the default (or `ALIBABA_BASE_URL`) base.
    pub fn new(api_key: Option<String>) -> Result<Self, SmahError> {
        Ok(AlibabaProvider {
            compat: OpenAiCompat::new(KEY, api_key, base_url_from_env())?,
        })
    }

    /// Construct against an explicit base URL (test seam; overrides the env
    /// var). Trailing slashes are tolerated.
    pub fn with_base_url(api_key: Option<String>, base_url: String) -> Result<Self, SmahError> {
        Ok(AlibabaProvider {
            compat: OpenAiCompat::new(KEY, api_key, base_url)?,
        })
    }

    /// The base URL requests resolve against.
    pub fn base_url(&self) -> &str {
        self.compat.base_url()
    }
}

impl Provider for AlibabaProvider {
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

/// Resolve the base URL: `ALIBABA_BASE_URL` when set and non-empty, the
/// default otherwise.
pub fn base_url_from_env() -> String {
    resolve_base_url(ALIBABA_BASE_URL_VAR, ALIBABA_DEFAULT_BASE_URL)
}
