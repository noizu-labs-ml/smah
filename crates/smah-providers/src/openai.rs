//! Hand-rolled OpenAI chat-completions client (Python
//! `smah/providers/openai_provider.py` parity, minus the SDK).
//!
//! The system prompt is a **leading `{"role": "system"}` message** (the
//! opposite of Anthropic's top-level `system` parameter). `max_tokens` is
//! only sent when the request computed a budget. Streaming consumes the SSE
//! body and surfaces `chat.completion.chunk` `delta.content` payloads
//! token-by-token, stopping at the `[DONE]` sentinel.
//!
//! Credential note: construction does not consult the environment for the
//! key (that is [credentials](crate::credentials)); without an `openai`
//! config block there is no key at all.

use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Map, Value};
use smah_core::errors::SmahError;

use crate::sse::{SseEvent, SseParser};
use crate::translate;
use crate::{Completion, CompletionRequest, Provider};

/// Config key and provider id used in every §4 message.
const KEY: &str = "openai";

/// Default API base, versioned path included (`OPENAI_BASE_URL` overrides
/// the whole base; config-file base URLs are NOT supported — the env var is
/// the mock-server test seam, by deliberate plan decision).
pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Env var honored as the API base override.
pub const OPENAI_BASE_URL_VAR: &str = "OPENAI_BASE_URL";

/// Endpoint path appended to the base URL.
const CHAT_COMPLETIONS_PATH: &str = "/chat/completions";

/// Total request timeout — Python SDK default parity (600s), streaming
/// included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// Hand-rolled OpenAI client. See the [Provider] trait for the contract;
/// this type is `pub` so tests and diagnostics can pin the base URL.
pub struct OpenAiProvider {
    api_key: Option<String>,
    base_url: String,
    http: reqwest::Client,
    runtime: tokio::runtime::Runtime,
}

impl OpenAiProvider {
    /// Construct against the default (or `OPENAI_BASE_URL`) base.
    pub fn new(api_key: Option<String>) -> Result<Self, SmahError> {
        Self::with_base_url(api_key, base_url_from_env())
    }

    /// Construct against an explicit base URL (test seam; overrides the env
    /// var). Trailing slashes are tolerated.
    pub fn with_base_url(api_key: Option<String>, base_url: String) -> Result<Self, SmahError> {
        Ok(OpenAiProvider {
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .map_err(|err| SmahError::Internal {
                    detail: format!("failed to build HTTP client: {err}"),
                })?,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| SmahError::Internal {
                    detail: format!("failed to build async runtime: {err}"),
                })?,
        })
    }

    /// The base URL requests resolve against.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Build the chat-completions request body.
    fn request_body(&self, req: &CompletionRequest, stream: bool) -> Value {
        // The system prompt is a leading system message, never omitted into
        // a null field.
        let mut messages: Vec<Value> = Vec::with_capacity(req.messages.len() + 1);
        if let Some(system) = &req.system {
            messages.push(json!({"role": "system", "content": system}));
        }
        messages.extend(
            req.messages
                .iter()
                .map(|message| json!({"role": message.role.as_str(), "content": &message.content})),
        );

        let mut body = Map::new();
        body.insert("model".to_string(), json!(req.model));
        body.insert("messages".to_string(), Value::Array(messages));
        if let Some(max_tokens) = req.max_tokens {
            body.insert("max_tokens".to_string(), json!(max_tokens));
        }
        if let Some(response_format) = &req.response_format {
            body.insert("response_format".to_string(), response_format.clone());
        }
        if stream {
            body.insert("stream".to_string(), json!(true));
        }
        Value::Object(body)
    }

    /// Send the request and return the response with a checked status.
    async fn send(&self, api_key: &str, body: &Value) -> Result<reqwest::Response, SmahError> {
        let url = format!("{}{CHAT_COMPLETIONS_PATH}", self.base_url);
        let response = self
            .http
            .post(&url)
            .bearer_auth(api_key)
            .json(body)
            .send()
            .await
            .map_err(|err| translate::transport_error(KEY, &err))?;
        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(translate::http_error(KEY, status, &body_text));
        }
        Ok(response)
    }

    /// Shared engine for both entry points.
    async fn run(
        &self,
        req: &CompletionRequest,
        stream: bool,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<Completion, SmahError> {
        let api_key = self
            .api_key
            .as_deref()
            .ok_or_else(|| SmahError::MissingApiKey {
                provider: KEY.to_string(),
            })?;
        let body = self.request_body(req, stream);
        tracing::debug!(
            model = %req.model,
            messages = body["messages"].as_array().map(Vec::len).unwrap_or(0),
            stream,
            "OpenAI completion request"
        );
        let response = self.send(api_key, &body).await?;

        if stream {
            let completion = self.consume_stream(response, on_token).await?;
            tracing::debug!(
                model = %req.model,
                chars = completion.text.len(),
                "OpenAI streaming completion finished"
            );
            Ok(completion)
        } else {
            let raw: Value = response
                .json()
                .await
                .map_err(|err| translate::transport_error(KEY, &err))?;
            let completion = parse_completion(&raw)?;
            tracing::debug!(
                model = %req.model,
                finish_reason = raw
                    .get("choices")
                    .and_then(|choices| choices.as_array())
                    .and_then(|choices| choices.first())
                    .and_then(|choice| choice.get("finish_reason"))
                    .and_then(|reason| reason.as_str())
                    .unwrap_or(""),
                chars = completion.text.len(),
                "OpenAI completion response"
            );
            Ok(completion)
        }
    }

    /// Read the SSE body, emitting `delta.content` tokens until `[DONE]`
    /// (or end of body, whichever comes first).
    async fn consume_stream(
        &self,
        response: reqwest::Response,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<Completion, SmahError> {
        let mut parser = SseParser::new();
        let mut text = String::new();
        let mut role = String::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| translate::transport_error(KEY, &err))?;
            for event in parser.feed(&chunk) {
                match chunk_signal(&event) {
                    Some(StreamSignal::Token(token)) => {
                        on_token(&token);
                        text.push_str(&token);
                    }
                    Some(StreamSignal::Done) => return finish_stream(text, role),
                    None => {
                        if role.is_empty() {
                            if let Some(first_role) = first_chunk_role(&event) {
                                role = first_role;
                            }
                        }
                    }
                }
            }
        }
        for event in parser.finish() {
            match chunk_signal(&event) {
                Some(StreamSignal::Token(token)) => {
                    on_token(&token);
                    text.push_str(&token);
                }
                Some(StreamSignal::Done) => break,
                None => {
                    if role.is_empty() {
                        if let Some(first_role) = first_chunk_role(&event) {
                            role = first_role;
                        }
                    }
                }
            }
        }
        finish_stream(text, role)
    }
}

impl Provider for OpenAiProvider {
    fn key(&self) -> &'static str {
        KEY
    }

    fn has_credentials(&self) -> bool {
        self.api_key.is_some()
    }

    fn complete(&self, req: &CompletionRequest) -> Result<Completion, SmahError> {
        let mut sink = |_: &str| {};
        self.runtime.block_on(self.run(req, false, &mut sink))
    }

    fn complete_streaming(
        &self,
        req: &CompletionRequest,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<Completion, SmahError> {
        self.runtime.block_on(self.run(req, true, on_token))
    }
}

/// Resolve the base URL: `OPENAI_BASE_URL` when set and non-empty, the
/// default otherwise.
pub fn base_url_from_env() -> String {
    match std::env::var(OPENAI_BASE_URL_VAR) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => OPENAI_DEFAULT_BASE_URL.to_string(),
    }
}

/// What one OpenAI SSE event means to the stream consumer.
enum StreamSignal {
    /// A `delta.content` text payload.
    Token(String),
    /// The `[DONE]` sentinel.
    Done,
}

/// Token payload of an OpenAI SSE event: `choices[0].delta.content` when
/// present. Role-only chunks, empty-choice usage chunks, and the `[DONE]`
/// sentinel map differently (see [chunk_signal] / [first_chunk_role]).
fn chunk_signal(event: &SseEvent) -> Option<StreamSignal> {
    if event.data.trim() == "[DONE]" {
        return Some(StreamSignal::Done);
    }
    let data: Value = serde_json::from_str(&event.data).ok()?;
    let content = data
        .get("choices")?
        .as_array()?
        .first()?
        .get("delta")?
        .get("content")?;
    content
        .as_str()
        .map(|token| StreamSignal::Token(token.to_string()))
}

/// Role carried by the first streaming chunk, when it has one and no
/// content yet.
fn first_chunk_role(event: &SseEvent) -> Option<String> {
    let data: Value = serde_json::from_str(&event.data).ok()?;
    let delta = data.get("choices")?.as_array()?.first()?.get("delta")?;
    if delta.get("content").is_some() {
        return None;
    }
    delta
        .get("role")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Assemble the streamed completion.
fn finish_stream(text: String, role: String) -> Result<Completion, SmahError> {
    Ok(Completion {
        text,
        role: if role.is_empty() {
            "assistant".to_string()
        } else {
            role
        },
        raw: None,
    })
}

/// Parse a blocking chat-completions response: `choices[0].message.content`
/// (null content — tool-call responses — reads as empty text).
fn parse_completion(raw: &Value) -> Result<Completion, SmahError> {
    let message = raw
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .ok_or_else(|| SmahError::Internal {
            detail: "unexpected OpenAI response shape: no choices[0].message".to_string(),
        })?;
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("assistant")
        .to_string();
    Ok(Completion {
        text,
        role,
        raw: Some(raw.clone()),
    })
}
