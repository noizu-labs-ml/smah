//! SMAH provider layer: the [`Provider`] trait, the config-key
//! [registry](registry), hand-rolled Anthropic/OpenAI HTTP clients (Alibaba
//! DashScope rides the shared OpenAI-compatible core), and a
//! hand-rolled [SSE parser](sse) for token streaming (rewrite plan §B/§G M2).
//!
//! No provider SDK crates: the clients are raw `reqwest` over rustls so the
//! request/response shapes and the error semantics are exactly SMAH's (plan
//! §B — "no acceptable Anthropic crate exists; async-openai rejected for
//! symmetry + exact error semantics").
//!
//! Every failure surfaces as [`smah_core::SmahError`], which carries the §4
//! exit-code class: missing/rejected key → 4, rate limit / API 4xx-5xx /
//! network → 5, unknown provider key / malformed settings block → 3. The
//! full table lives in [translate].
//!
//! Print-only invariant: this crate is pure HTTP + std — it never touches
//! any OS interface beyond sockets (CI gate `no-shell-gate`).

pub mod alibaba;
pub mod anthropic;
pub mod credentials;
pub mod openai;
pub mod openai_compat;
pub mod registry;
pub mod sse;
pub mod translate;

use smah_core::SmahError;

pub use alibaba::AlibabaProvider;
pub use anthropic::AnthropicProvider;
pub use openai::OpenAiProvider;
pub use registry::{get_provider, known_provider_keys};

/// The role of a [`ChatMessage`] in a conversation thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageRole {
    /// System instruction (Anthropic hoists these out as the top-level
    /// `system` parameter; OpenAI keeps them as a leading message).
    System,
    /// User turn.
    User,
    /// Assistant turn.
    Assistant,
}

impl MessageRole {
    /// Wire name of the role.
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        }
    }
}

/// One message in a conversation thread (Python thread-dict parity:
/// `{"role": ..., "content": ...}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// Who produced the content.
    pub role: MessageRole,
    /// The message text.
    pub content: String,
}

impl ChatMessage {
    /// A message with an explicit role.
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        ChatMessage {
            role,
            content: content.into(),
        }
    }

    /// A user turn.
    pub fn user(content: impl Into<String>) -> Self {
        ChatMessage::new(MessageRole::User, content)
    }

    /// An assistant turn.
    pub fn assistant(content: impl Into<String>) -> Self {
        ChatMessage::new(MessageRole::Assistant, content)
    }

    /// A system instruction.
    pub fn system(content: impl Into<String>) -> Self {
        ChatMessage::new(MessageRole::System, content)
    }
}

/// A provider-neutral completion request.
///
/// `model` is the provider-side model id (the catalog maps `anthropic.x` →
/// `x`; the catalog itself is a later milestone). `max_tokens` is required
/// by the Anthropic Messages API (which applies its own default when `None`)
/// and optional for OpenAI (omitted from the body when `None`).
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    /// Provider-side model id.
    pub model: String,
    /// The conversation thread, in order.
    pub messages: Vec<ChatMessage>,
    /// System prompt. Anthropic sends it as the top-level `system`
    /// parameter; OpenAI prepends it as a `{"role": "system"}` message.
    pub system: Option<String>,
    /// Output-token budget, when the caller computed one.
    pub max_tokens: Option<u32>,
    /// Structured-output request (`response_format`); Anthropic has no such
    /// parameter and instead appends a JSON-only instruction to the system
    /// prompt (Python client parity).
    pub response_format: Option<serde_json::Value>,
}

impl CompletionRequest {
    /// A request for `model` over `messages` with everything else unset.
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        CompletionRequest {
            model: model.into(),
            messages,
            system: None,
            max_tokens: None,
            response_format: None,
        }
    }
}

/// A provider-neutral completion result (Python `Completion` parity minus the
/// SDK object: `raw` keeps the parsed JSON body for diagnostics).
#[derive(Debug, Clone)]
pub struct Completion {
    /// The assistant's textual answer (all text content joined).
    pub text: String,
    /// The role that produced the answer (`"assistant"` today).
    pub role: String,
    /// The provider's parsed JSON response body, for logging/diagnostics
    /// only. Callers must not depend on its shape.
    pub raw: Option<serde_json::Value>,
}

/// A chat-completion inference backend.
///
/// Both entry points are **blocking** on the outside; each client owns a
/// single-threaded tokio runtime that drives its reqwest work. Tokens in
/// [`Provider::complete_streaming`] are delivered to the callback as they
/// arrive on the wire, while the call itself blocks until the response ends.
///
/// Credentials are resolved at construction (see [credentials]) — the seam
/// mirrors the Python rules: CLI flag → config `settings.api_key` (with
/// `$ENV` indirection) → `SMAH_<KEY>_API_KEY` → `<KEY>_API_KEY`, with the
/// OpenAI special case that no `openai` config block means no key at all.
/// Calling either method on a keyless provider returns
/// [`SmahError::MissingApiKey`] (exit 4).
pub trait Provider: Send + Sync {
    /// The provider config key (`"anthropic"`, `"openai"`).
    fn key(&self) -> &'static str;

    /// Whether a credential was resolved for this provider (never prints it).
    fn has_credentials(&self) -> bool;

    /// Run a completion and return the normalized text.
    fn complete(&self, req: &CompletionRequest) -> Result<Completion, SmahError>;

    /// Run a streaming completion: `on_token` is invoked once per text delta
    /// as it arrives, and the returned [`Completion`] carries the joined text.
    fn complete_streaming(
        &self,
        req: &CompletionRequest,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<Completion, SmahError>;
}
