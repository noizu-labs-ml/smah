//! The provider seam for the ask engine: everything upstream (prompt
//! assembly, the loop, the renderers, the tests) speaks [`AskBackend`], so
//! provider drift stays a one-file fix at the CLI edge (same pattern as the
//! virtual-tools `ChatBackend` seam).
//!
//! Tests inject scripted backends — no real API calls anywhere.

use smah_core::errors::SmahError;

/// A turn in the conversation thread (user/assistant only; system context
/// rides the [`AskBackend::stream`] `system` argument).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMessage {
    /// Who produced the content.
    pub role: ThreadRole,
    /// The message text.
    pub content: String,
}

/// Thread roles (the system prompt is separate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadRole {
    /// User turn.
    User,
    /// Assistant turn.
    Assistant,
}

impl ThreadMessage {
    /// A user turn.
    pub fn user(content: impl Into<String>) -> Self {
        ThreadMessage {
            role: ThreadRole::User,
            content: content.into(),
        }
    }

    /// An assistant turn.
    pub fn assistant(content: impl Into<String>) -> Self {
        ThreadMessage {
            role: ThreadRole::Assistant,
            content: content.into(),
        }
    }
}

/// A streaming chat backend (one provider + one model).
pub trait AskBackend: Send + Sync {
    /// The provider-side model id requests are addressed to.
    fn model(&self) -> &str;

    /// Provider key (diagnostics/banner only).
    fn provider_key(&self) -> &str;

    /// Whether a credential is resolved (preflight key check — a keyless
    /// backend fails fast with exit 4 instead of a wasted HTTP call).
    fn has_credentials(&self) -> bool;

    /// Run one streaming turn: `on_delta` receives raw text deltas off the
    /// wire; the return value is the full joined text.
    fn stream(
        &self,
        system: &str,
        messages: &[ThreadMessage],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, SmahError>;
}

/// A scripted backend for tests: replays a fixed answer, optionally in
/// deltas; records the system prompt and thread it was handed.
pub struct ScriptedBackend {
    /// Provider-side model id reported by [`AskBackend::model`].
    pub model_id: String,
    /// Answer text replayed by [`AskBackend::stream`].
    pub answer: String,
    /// Delta chunk size (1 = byte-by-byte streaming; the answer splits on
    /// char boundaries).
    pub chunk_chars: usize,
    /// Recorded system prompts (one per stream call).
    pub seen_system: std::sync::Mutex<Vec<String>>,
    /// Recorded threads (one per stream call).
    pub seen_threads: std::sync::Mutex<Vec<Vec<ThreadMessage>>>,
}

impl ScriptedBackend {
    /// A backend answering `answer` in one delta.
    pub fn once(answer: impl Into<String>) -> Self {
        ScriptedBackend {
            model_id: "scripted-model".to_string(),
            answer: answer.into(),
            chunk_chars: usize::MAX,
            seen_system: std::sync::Mutex::new(Vec::new()),
            seen_threads: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// A backend answering `answer` char-by-char (exercises the stream
    /// filter's split-tag handling).
    pub fn char_by_char(answer: impl Into<String>) -> Self {
        ScriptedBackend {
            chunk_chars: 1,
            ..Self::once(answer)
        }
    }
}

impl AskBackend for ScriptedBackend {
    fn model(&self) -> &str {
        &self.model_id
    }

    fn provider_key(&self) -> &str {
        "scripted"
    }

    fn has_credentials(&self) -> bool {
        true
    }

    fn stream(
        &self,
        system: &str,
        messages: &[ThreadMessage],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, SmahError> {
        self.seen_system
            .lock()
            .expect("system log")
            .push(system.to_string());
        self.seen_threads
            .lock()
            .expect("thread log")
            .push(messages.to_vec());
        if self.chunk_chars == usize::MAX {
            on_delta(&self.answer);
        } else {
            for ch in self.answer.chars() {
                on_delta(ch.to_string().as_str());
            }
        }
        Ok(self.answer.clone())
    }
}
