//! Hand-rolled Anthropic Messages API client (Python
//! `smah/providers/anthropic_provider.py` parity, minus the SDK).
//!
//! Request shape (Claude Opus 5.5 / Haiku 5.5):
//! * the system prompt is the **top-level `system` parameter** — never a
//!   `{"role": "system"}` message — and is omitted entirely when absent;
//! * `max_tokens` is **required** and sent on every call (16000 default);
//! * `response.content` is a list of typed blocks: only blocks with
//!   `type == "text"` contribute text, and `content[0]` is never assumed.
//!
//! Streaming consumes the SSE body and surfaces `content_block_delta` /
//! `text_delta` payloads token-by-token.

use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Map, Value};
use smah_core::errors::SmahError;

use crate::sse::{SseEvent, SseParser};
use crate::translate;
use crate::{Completion, CompletionRequest, MessageRole, Provider};

/// Config key and provider id used in every §4 message.
const KEY: &str = "anthropic";

/// Default API base (`ANTHROPIC_BASE_URL` overrides; config-file base URLs
/// are NOT supported — the env var is the T-005 parity seam and the
/// mock-server test seam, by deliberate plan decision).
pub const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Env var honored as the API base override.
pub const ANTHROPIC_BASE_URL_VAR: &str = "ANTHROPIC_BASE_URL";

/// Messages API path appended to the base URL (SDK parity).
const MESSAGES_PATH: &str = "/v1/messages";

/// API version header the Messages API requires.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// max_tokens is required by the Messages API; 16000 is the fallback when a
/// request carries no output budget.
pub const ANTHROPIC_DEFAULT_MAX_TOKENS: u32 = 16000;

/// Appended to the system prompt in place of a `response_format` parameter
/// (the Messages API has none).
const JSON_ONLY_INSTRUCTION: &str = "Respond with ONLY a single JSON object that matches the requested schema. No prose, no explanations, and no markdown code fences.";

/// Total request timeout — Python SDK default parity (600s), streaming
/// included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// Hand-rolled Anthropic client. See the [Provider] trait for the contract;
/// this type is `pub` so tests and diagnostics can pin the base URL.
pub struct AnthropicProvider {
    api_key: Option<String>,
    base_url: String,
    http: reqwest::Client,
    runtime: tokio::runtime::Runtime,
}

impl AnthropicProvider {
    /// Construct against the default (or `ANTHROPIC_BASE_URL`) base.
    pub fn new(api_key: Option<String>) -> Result<Self, SmahError> {
        Self::with_base_url(api_key, base_url_from_env())
    }

    /// Construct against an explicit base URL (test seam; overrides the env
    /// var). Trailing slashes are tolerated.
    pub fn with_base_url(api_key: Option<String>, base_url: String) -> Result<Self, SmahError> {
        Ok(AnthropicProvider {
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

    /// Build the Messages API request body.
    fn request_body(&self, req: &CompletionRequest, stream: bool) -> Value {
        // Anthropic takes only user/assistant turns; the system prompt is
        // the top-level `system` parameter below.
        let messages: Vec<Value> = req
            .messages
            .iter()
            .filter(|message| message.role != MessageRole::System)
            .map(|message| json!({"role": message.role.as_str(), "content": &message.content}))
            .collect();

        // No response_format parameter exists: ask for JSON in the system
        // prompt instead.
        let system = match (&req.system, &req.response_format) {
            (Some(system), Some(_)) => Some(format!("{system}\n\n{JSON_ONLY_INSTRUCTION}")),
            (Some(system), None) => Some(system.clone()),
            (None, Some(_)) => Some(JSON_ONLY_INSTRUCTION.to_string()),
            (None, None) => None,
        };

        let mut body = Map::new();
        body.insert("model".to_string(), json!(req.model));
        // Required by the Messages API on every call.
        body.insert(
            "max_tokens".to_string(),
            json!(req.max_tokens.unwrap_or(ANTHROPIC_DEFAULT_MAX_TOKENS)),
        );
        body.insert("messages".to_string(), Value::Array(messages));
        // Omitted entirely when absent so it is not serialized as null.
        if let Some(system) = system {
            body.insert("system".to_string(), json!(system));
        }
        if stream {
            body.insert("stream".to_string(), json!(true));
        }
        Value::Object(body)
    }

    /// Send the request and return the response with a checked status.
    async fn send(&self, api_key: &str, body: &Value) -> Result<reqwest::Response, SmahError> {
        let url = format!("{}{MESSAGES_PATH}", self.base_url);
        let response = self
            .http
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
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
            "Anthropic completion request"
        );
        let response = self.send(api_key, &body).await?;

        if stream {
            let completion = self.consume_stream(response, on_token).await?;
            tracing::debug!(
                model = %req.model,
                chars = completion.text.len(),
                "Anthropic streaming completion finished"
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
                stop_reason = raw.get("stop_reason").and_then(|stop| stop.as_str()).unwrap_or(""),
                chars = completion.text.len(),
                "Anthropic completion response"
            );
            Ok(completion)
        }
    }

    /// Read the SSE body, emitting `text_delta` tokens as they arrive.
    async fn consume_stream(
        &self,
        response: reqwest::Response,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<Completion, SmahError> {
        let mut parser = SseParser::new();
        let mut text = String::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| translate::transport_error(KEY, &err))?;
            for event in parser.feed(&chunk) {
                if let Some(token) = text_delta(&event) {
                    on_token(&token);
                    text.push_str(&token);
                }
            }
        }
        for event in parser.finish() {
            if let Some(token) = text_delta(&event) {
                on_token(&token);
                text.push_str(&token);
            }
        }
        Ok(Completion {
            text,
            role: "assistant".to_string(),
            raw: None,
        })
    }
}

impl Provider for AnthropicProvider {
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

/// Resolve the base URL: `ANTHROPIC_BASE_URL` when set and non-empty,
/// the default otherwise.
pub fn base_url_from_env() -> String {
    match std::env::var(ANTHROPIC_BASE_URL_VAR) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => ANTHROPIC_DEFAULT_BASE_URL.to_string(),
    }
}

/// Text payload of an Anthropic SSE event, when it is a `content_block_delta`
/// carrying a `text_delta`. Every other event shape (message_start, ping,
/// tool-use `input_json_delta`, stops) yields `None`.
fn text_delta(event: &SseEvent) -> Option<String> {
    let data: Value = serde_json::from_str(&event.data).ok()?;
    if data.get("type").and_then(Value::as_str) != Some("content_block_delta") {
        return None;
    }
    let delta = data.get("delta")?;
    if delta.get("type").and_then(Value::as_str) != Some("text_delta") {
        return None;
    }
    delta
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Parse a blocking Messages API response: content blocks with
/// `type == "text"` contribute text (joined with `\n`), never assuming
/// `content[0]`.
fn parse_completion(raw: &Value) -> Result<Completion, SmahError> {
    let content = raw
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| SmahError::Internal {
            detail: "unexpected Anthropic response shape: no content array".to_string(),
        })?;
    let text = content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .map(|block| block.get("text").and_then(Value::as_str).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let role = raw
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
