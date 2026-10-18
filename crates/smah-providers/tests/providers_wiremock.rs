//! Wiremock integration suite for the hand-rolled provider clients (M2).
//!
//! Covers: blocking + streaming happy paths for the providers (Anthropic,
//! OpenAI, Alibaba DashScope), SSE fixture streams (Anthropic delta shapes,
//! OpenAI `[DONE]`), the error translation table (401 → exit 4; 429/500/
//! network → exit 5), the BASE_URL env seams, credential-resolution seams,
//! and the exit-3 config classes (unknown provider key, malformed settings
//! block).
//!
//! All keys are fakes (`test-key-0000`); nothing here touches a real API —
//! except the explicitly `#[ignore]`d real-endpoint smoke at the bottom,
//! which is env-gated and never runs in CI.

use std::net::TcpListener;
use std::sync::Mutex;

use serde_json::{json, Value};
use smah_core::errors::SmahError;
use smah_core::settings::Settings;
use smah_providers::alibaba::ALIBABA_DEFAULT_BASE_URL;
use smah_providers::credentials::resolve_api_key_with;
use smah_providers::registry::{get_provider, known_provider_keys};
use smah_providers::{
    AlibabaProvider, AnthropicProvider, ChatMessage, CompletionRequest, OpenAiProvider, Provider,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Fake key — never a real credential.
const FAKE_KEY: &str = "test-key-0000";

/// Serializes the env-mutating tests (the environment is process-global and
/// cargo test runs in parallel threads).
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Write `yaml` to a temp file and load it as Settings.
fn settings_from(yaml: &str) -> Settings {
    let file = tempfile::NamedTempFile::new().expect("temp file");
    std::fs::write(file.path(), yaml).expect("write fixture");
    Settings::load_from(file.path()).expect("settings load")
}

/// A minimal valid config whose `anthropic` block carries a literal key.
fn anthropic_settings() -> Settings {
    settings_from(
        "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    anthropic:\n      name: Anthropic\n      enabled: true\n      settings:\n        api_key: test-key-0000\n      models: []\n",
    )
}

/// A one-message request.
fn request(model: &str) -> CompletionRequest {
    let mut req = CompletionRequest::new(model, vec![ChatMessage::user("hello")]);
    req.system = Some("Be terse.".to_string());
    req
}

/// A unused local port (bind, note, drop — connections to it are refused).
fn dead_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    port
}

fn assert_exit(err: &SmahError, code: i32) {
    assert_eq!(
        err.exit_code().code(),
        code,
        "unexpected exit class for {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Anthropic — blocking
// ---------------------------------------------------------------------------

#[test]
fn anthropic_blocking_happy_path() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "msg_1",
                "model": "claude-haiku-5-5",
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "(inner monologue)"},
                    {"type": "text", "text": "Hello"},
                    {"type": "text", "text": "there"},
                    {"type": "tool_use", "id": "toolu_1", "name": "noop"}
                ],
                "stop_reason": "end_turn"
            })))
            .mount(&server),
    );

    let provider = AnthropicProvider::with_base_url(Some(FAKE_KEY.to_string()), server.uri())
        .expect("construct");
    let mut req = request("claude-haiku-5-5");
    req.max_tokens = Some(64);
    let completion = provider.complete(&req).expect("completion");

    // Only text blocks contribute, joined with \n; content[0] never assumed.
    assert_eq!(completion.text, "Hello\nthere");
    assert_eq!(completion.role, "assistant");

    let requests = rt
        .block_on(server.received_requests())
        .expect("requests recorded");
    assert_eq!(requests.len(), 1, "exactly one request expected");
    let sent = &requests[0];
    assert_eq!(sent.url.path(), "/v1/messages");
    assert_eq!(
        sent.headers.get("x-api-key").and_then(|v| v.to_str().ok()),
        Some(FAKE_KEY)
    );
    assert_eq!(
        sent.headers
            .get("anthropic-version")
            .and_then(|v| v.to_str().ok()),
        Some("2023-06-01")
    );
    let body: Value = serde_json::from_slice(&sent.body).expect("request body json");
    // System prompt is the top-level `system` parameter, never a message.
    assert_eq!(body["system"], json!("Be terse."));
    assert_eq!(
        body["messages"],
        json!([{"role": "user", "content": "hello"}])
    );
    assert_eq!(body["max_tokens"], json!(64));
    assert!(body.get("stream").is_none());
}

#[test]
fn anthropic_max_tokens_is_required_and_defaults() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "role": "assistant",
                "content": [{"type": "text", "text": "ok"}]
            })))
            .mount(&server),
    );

    let provider = AnthropicProvider::with_base_url(Some(FAKE_KEY.to_string()), server.uri())
        .expect("construct");
    let completion = provider
        .complete(&request("claude-haiku-5-5"))
        .expect("completion");
    assert_eq!(completion.text, "ok");

    let requests = rt
        .block_on(server.received_requests())
        .expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).expect("body json");
    // Absent budget ⇒ the Messages-API default is sent anyway (required).
    assert_eq!(body["max_tokens"], json!(16000));
}

// ---------------------------------------------------------------------------
// Anthropic — streaming
// ---------------------------------------------------------------------------

#[test]
fn anthropic_streaming_emits_text_deltas_only() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });
    let sse = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"role\":\"assistant\"}}\n",
        "\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"a\\\":1}\"}}\n",
        "\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n",
        "\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n",
        "\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n",
        "\n",
    );
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse)
                    .insert_header("content-type", "text/event-stream"),
            )
            .mount(&server),
    );

    let provider = AnthropicProvider::with_base_url(Some(FAKE_KEY.to_string()), server.uri())
        .expect("construct");

    let mut tokens: Vec<String> = Vec::new();
    let completion = provider
        .complete_streaming(&request("claude-haiku-5-5"), &mut |token: &str| {
            tokens.push(token.to_string());
        })
        .expect("streamed completion");

    assert_eq!(tokens, vec!["Hel".to_string(), "lo".to_string()]);
    assert_eq!(completion.text, "Hello");
    assert_eq!(completion.role, "assistant");

    let requests = rt
        .block_on(server.received_requests())
        .expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).expect("body json");
    assert_eq!(body["stream"], json!(true));
}

// ---------------------------------------------------------------------------
// OpenAI — blocking
// ---------------------------------------------------------------------------

#[test]
fn openai_blocking_happy_path() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-1",
                "choices": [{
                    "message": {"role": "assistant", "content": "hi there"},
                    "finish_reason": "stop"
                }]
            })))
            .mount(&server),
    );

    let provider =
        OpenAiProvider::with_base_url(Some(FAKE_KEY.to_string()), server.uri()).expect("construct");
    let completion = provider
        .complete(&request("gpt-4o-mini"))
        .expect("completion");

    assert_eq!(completion.text, "hi there");
    assert_eq!(completion.role, "assistant");

    let requests = rt
        .block_on(server.received_requests())
        .expect("requests recorded");
    assert_eq!(requests.len(), 1, "exactly one request expected");
    let sent = &requests[0];
    assert_eq!(sent.url.path(), "/chat/completions");
    assert_eq!(
        sent.headers
            .get("authorization")
            .and_then(|v| v.to_str().ok()),
        Some(format!("Bearer {FAKE_KEY}").as_str())
    );
    let body: Value = serde_json::from_slice(&sent.body).expect("request body json");
    // System prompt is a leading system message (opposite of Anthropic).
    assert_eq!(
        body["messages"],
        json!([
            {"role": "system", "content": "Be terse."},
            {"role": "user", "content": "hello"}
        ])
    );
    assert_eq!(body["model"], json!("gpt-4o-mini"));
    // max_tokens omitted when the request computed no budget.
    assert!(body.get("max_tokens").is_none());
}

// ---------------------------------------------------------------------------
// OpenAI — streaming
// ---------------------------------------------------------------------------

#[test]
fn openai_streaming_stops_at_done_sentinel() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"he\"}}]}\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{}}]}\n",
        "\n",
        "data: [DONE]\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"AFTER\"}}]}\n",
        "\n",
    );
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse)
                    .insert_header("content-type", "text/event-stream"),
            )
            .mount(&server),
    );

    let provider =
        OpenAiProvider::with_base_url(Some(FAKE_KEY.to_string()), server.uri()).expect("construct");

    let mut tokens: Vec<String> = Vec::new();
    let completion = provider
        .complete_streaming(&request("gpt-4o-mini"), &mut |token: &str| {
            tokens.push(token.to_string());
        })
        .expect("streamed completion");

    // Role-only and empty chunks emit nothing; [DONE] stops consumption, so
    // the trailing event never surfaces.
    assert_eq!(tokens, vec!["he".to_string()]);
    assert_eq!(completion.text, "he");
    assert_eq!(completion.role, "assistant");
}

// ---------------------------------------------------------------------------
// Error translation table
// ---------------------------------------------------------------------------

#[test]
fn rejected_key_maps_to_exit_4() {
    // Anthropic 401 with a vendor-shaped error body.
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "type": "error",
                "error": {"type": "authentication_error", "message": "invalid x-api-key"}
            })))
            .mount(&server),
    );
    let provider = AnthropicProvider::with_base_url(Some(FAKE_KEY.to_string()), server.uri())
        .expect("construct");
    let err = provider
        .complete(&request("claude-haiku-5-5"))
        .expect_err("401");
    assert!(matches!(err, SmahError::ProviderAuth { .. }));
    assert_exit(&err, 4);
}

#[test]
fn forbidden_key_maps_to_exit_4() {
    // OpenAI 403.
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "error": {"type": "invalid_request_error", "message": "no access"}
            })))
            .mount(&server),
    );
    let provider =
        OpenAiProvider::with_base_url(Some(FAKE_KEY.to_string()), server.uri()).expect("construct");
    let err = provider.complete(&request("gpt-4o-mini")).expect_err("403");
    assert!(matches!(err, SmahError::ProviderAuth { .. }));
    assert_exit(&err, 4);
}

#[test]
fn rate_limit_maps_to_exit_5() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(429).set_body_json(json!({
                "type": "error",
                "error": {"type": "rate_limit_error", "message": "slow down"}
            })))
            .mount(&server),
    );
    let provider = AnthropicProvider::with_base_url(Some(FAKE_KEY.to_string()), server.uri())
        .expect("construct");
    let err = provider
        .complete(&request("claude-haiku-5-5"))
        .expect_err("429");
    match &err {
        SmahError::ProviderRequest { code, detail, .. } => {
            assert_eq!(*code, 429);
            assert_eq!(detail, "slow down");
        }
        other => panic!("expected ProviderRequest, got {other:?}"),
    }
    assert_exit(&err, 5);
}

#[test]
fn server_error_maps_to_exit_5() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal"))
            .mount(&server),
    );
    let provider =
        OpenAiProvider::with_base_url(Some(FAKE_KEY.to_string()), server.uri()).expect("construct");
    let err = provider.complete(&request("gpt-4o-mini")).expect_err("500");
    assert!(matches!(err, SmahError::ProviderUnavailable { .. }));
    assert_exit(&err, 5);
}

#[test]
fn network_failure_maps_to_exit_5() {
    let provider = AnthropicProvider::with_base_url(
        Some(FAKE_KEY.to_string()),
        format!("http://127.0.0.1:{}", dead_port()),
    )
    .expect("construct");
    let err = provider
        .complete(&request("claude-haiku-5-5"))
        .expect_err("refused");
    assert!(matches!(err, SmahError::Network { .. }));
    assert_exit(&err, 5);
}

// ---------------------------------------------------------------------------
// BASE_URL seam
// ---------------------------------------------------------------------------

#[test]
fn base_url_env_seam_routes_requests() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let rt = tokio::runtime::Runtime::new().expect("test runtime");

    let anthropic = rt.block_on(async { MockServer::start().await });
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "role": "assistant",
                "content": [{"type": "text", "text": "via env"}]
            })))
            .mount(&anthropic),
    );

    let openai = rt.block_on(async { MockServer::start().await });
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "via env"}}]
            })))
            .mount(&openai),
    );

    std::env::set_var("ANTHROPIC_BASE_URL", anthropic.uri());
    std::env::set_var("OPENAI_BASE_URL", openai.uri());
    let (anthropic_provider, openai_provider) = (
        AnthropicProvider::new(Some(FAKE_KEY.to_string())).expect("construct"),
        OpenAiProvider::new(Some(FAKE_KEY.to_string())).expect("construct"),
    );
    std::env::remove_var("ANTHROPIC_BASE_URL");
    std::env::remove_var("OPENAI_BASE_URL");

    // The mocks answer only if the env vars actually routed the traffic.
    assert_eq!(
        anthropic_provider
            .base_url()
            .trim_end_matches('/')
            .to_string(),
        anthropic.uri().trim_end_matches('/').to_string()
    );
    assert_eq!(
        anthropic_provider
            .complete(&request("claude-haiku-5-5"))
            .expect("anthropic completion")
            .text,
        "via env"
    );
    assert_eq!(
        openai_provider.base_url().trim_end_matches('/').to_string(),
        openai.uri().trim_end_matches('/').to_string()
    );
    assert_eq!(
        openai_provider
            .complete(&request("gpt-4o-mini"))
            .expect("openai completion")
            .text,
        "via env"
    );
}

// ---------------------------------------------------------------------------
// Missing key → exit 4
// ---------------------------------------------------------------------------

#[test]
fn missing_key_maps_to_exit_4_naming_the_env_var() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });

    let anthropic = AnthropicProvider::with_base_url(None, server.uri()).expect("construct");
    assert!(!anthropic.has_credentials());
    let err = anthropic
        .complete(&request("claude-haiku-5-5"))
        .expect_err("no key");
    assert!(matches!(err, SmahError::MissingApiKey { .. }));
    assert_exit(&err, 4);
    assert!(
        err.render("/tmp/smah-test.log")
            .contains("ANTHROPIC_API_KEY"),
        "message must name the env var: {}",
        err.render("/tmp/smah-test.log")
    );

    let openai = OpenAiProvider::with_base_url(None, server.uri()).expect("construct");
    assert!(!openai.has_credentials());
    let err = openai
        .complete(&request("gpt-4o-mini"))
        .expect_err("no key");
    assert!(matches!(err, SmahError::MissingApiKey { .. }));
    assert_exit(&err, 4);
    assert!(
        err.render("/tmp/smah-test.log").contains("OPENAI_API_KEY"),
        "message must name the env var: {}",
        err.render("/tmp/smah-test.log")
    );
}

// ---------------------------------------------------------------------------
// Exit-3 config classes
// ---------------------------------------------------------------------------

#[test]
fn unknown_provider_key_maps_to_exit_3() {
    let settings = anthropic_settings();
    let err = get_provider("grok", &settings, None)
        .err()
        .expect("unknown key");
    match &err {
        SmahError::ConfigGate { detail, .. } => {
            assert!(detail.contains("grok"), "names the offender: {detail}");
            assert!(
                detail.contains("anthropic, openai"),
                "names the known providers: {detail}"
            );
        }
        other => panic!("expected ConfigGate, got {other:?}"),
    }
    assert_exit(&err, 3);
}

#[test]
fn malformed_settings_block_maps_to_exit_3() {
    // `settings` must be a mapping; a scalar block is malformed.
    let scalar = settings_from(
        "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    anthropic:\n      name: Anthropic\n      enabled: true\n      settings: 42\n      models: []\n",
    );
    let err = get_provider("anthropic", &scalar, None)
        .err()
        .expect("malformed block");
    assert!(matches!(err, SmahError::ConfigUnreadable { .. }));
    assert_exit(&err, 3);

    // A provider block that is not a mapping at all.
    let non_mapping = settings_from(
        "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    anthropic: nope\n",
    );
    let err = get_provider("anthropic", &non_mapping, None)
        .err()
        .expect("malformed block");
    assert!(matches!(err, SmahError::ConfigUnreadable { .. }));
    assert_exit(&err, 3);
}

// ---------------------------------------------------------------------------
// Credential resolution seams (injected env; no network)
// ---------------------------------------------------------------------------

fn lookup_from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name: &str| {
        pairs
            .iter()
            .find(|(var, _)| *var == name)
            .map(|(_, value)| value.to_string())
    }
}

#[test]
fn settings_literal_key_wins() {
    let settings = anthropic_settings();
    let resolved = resolve_api_key_with(&settings, "anthropic", None, &|_| None)
        .expect("resolve")
        .expect("key present");
    assert_eq!(resolved, FAKE_KEY);
}

#[test]
fn env_indirection_resolves_and_falls_through() {
    // `$MY_KEY` resolves through the environment.
    let settings = settings_from(
        "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    anthropic:\n      name: Anthropic\n      enabled: true\n      settings:\n        api_key: $MY_KEY\n      models: []\n",
    );
    let resolved = resolve_api_key_with(
        &settings,
        "anthropic",
        None,
        &lookup_from(&[("MY_KEY", "test-key-0000")]),
    )
    .expect("resolve")
    .expect("key present");
    assert_eq!(resolved, "test-key-0000");

    // `$MY_KEY` unset ⇒ the provider env chain applies.
    let resolved = resolve_api_key_with(
        &settings,
        "anthropic",
        None,
        &lookup_from(&[("SMAH_ANTHROPIC_API_KEY", "test-key-0000")]),
    )
    .expect("resolve")
    .expect("key present");
    assert_eq!(resolved, "test-key-0000");
}

#[test]
fn env_chain_prefers_the_smah_prefix() {
    // No anthropic block at all: env chain still resolves (Python base-class
    // parity — only OpenAI lacks this fallback).
    let settings = settings_from("vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers: {}\n");
    let resolved = resolve_api_key_with(
        &settings,
        "anthropic",
        None,
        &lookup_from(&[
            ("ANTHROPIC_API_KEY", "plain"),
            ("SMAH_ANTHROPIC_API_KEY", "smah"),
        ]),
    )
    .expect("resolve")
    .expect("key present");
    assert_eq!(resolved, "smah");

    // Plain fallback when the SMAH_ prefixed var is unset.
    let resolved = resolve_api_key_with(
        &settings,
        "anthropic",
        None,
        &lookup_from(&[("ANTHROPIC_API_KEY", "plain")]),
    )
    .expect("resolve")
    .expect("key present");
    assert_eq!(resolved, "plain");
}

#[test]
fn openai_without_config_block_has_no_key_at_all() {
    // The OpenAI special case: no `openai` block ⇒ no key, environment
    // notwithstanding.
    let settings = settings_from("vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers: {}\n");
    let resolved = resolve_api_key_with(
        &settings,
        "openai",
        None,
        &lookup_from(&[("OPENAI_API_KEY", "test-key-0000")]),
    )
    .expect("resolve");
    assert_eq!(resolved, None);
}

#[test]
fn cli_flag_folds_into_openai_only() {
    let both = "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    openai:\n      name: OpenAI\n      enabled: true\n      settings: {}\n      models: []\n    anthropic:\n      name: Anthropic\n      enabled: true\n      settings: {}\n      models: []\n";
    let settings = settings_from(both);

    // OpenAI: the flag becomes the key even with no configured api_key.
    let resolved = resolve_api_key_with(
        &settings,
        "openai",
        Some("flag-key-0000"),
        &lookup_from(&[("OPENAI_API_KEY", "test-key-0000")]),
    )
    .expect("resolve")
    .expect("key present");
    assert_eq!(resolved, "flag-key-0000");

    // Anthropic: the flag is ignored; the env chain answers.
    let resolved = resolve_api_key_with(
        &settings,
        "anthropic",
        Some("flag-key-0000"),
        &lookup_from(&[("ANTHROPIC_API_KEY", "test-key-0000")]),
    )
    .expect("resolve")
    .expect("key present");
    assert_eq!(resolved, "test-key-0000");
}

#[test]
fn registry_constructs_configured_providers() {
    let settings = anthropic_settings();
    let provider = get_provider("anthropic", &settings, None).expect("provider");
    assert_eq!(provider.key(), "anthropic");
    assert!(provider.has_credentials());

    let empty = settings_from("vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers: {}\n");
    let provider = get_provider("openai", &empty, None).expect("provider");
    assert_eq!(provider.key(), "openai");
    assert!(!provider.has_credentials());
}

// ---------------------------------------------------------------------------
// Alibaba (DashScope compatible-mode) — blocking / streaming
// ---------------------------------------------------------------------------

/// A minimal valid config whose `alibaba` block carries a literal key.
fn alibaba_settings() -> Settings {
    settings_from(
        "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    alibaba:\n      name: Alibaba\n      enabled: true\n      settings:\n        api_key: test-key-0000\n      models: []\n",
    )
}

#[test]
fn alibaba_blocking_happy_path() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-qwen-1",
                "choices": [{
                    "message": {"role": "assistant", "content": "ni hao there"},
                    "finish_reason": "stop"
                }]
            })))
            .mount(&server),
    );

    let provider = AlibabaProvider::with_base_url(Some(FAKE_KEY.to_string()), server.uri())
        .expect("construct");
    let mut req = request("qwen3.8-flash");
    req.max_tokens = Some(64);
    let completion = provider.complete(&req).expect("completion");

    assert_eq!(completion.text, "ni hao there");
    assert_eq!(completion.role, "assistant");

    let requests = rt
        .block_on(server.received_requests())
        .expect("requests recorded");
    assert_eq!(requests.len(), 1, "exactly one request expected");
    let sent = &requests[0];
    // OpenAI-compatible wire shape against the alibaba base: bearer auth,
    // /chat/completions path appended to the compatible-mode base.
    assert_eq!(sent.url.path(), "/chat/completions");
    assert_eq!(
        sent.headers
            .get("authorization")
            .and_then(|v| v.to_str().ok()),
        Some(format!("Bearer {FAKE_KEY}").as_str())
    );
    let body: Value = serde_json::from_slice(&sent.body).expect("request body json");
    // System prompt is a leading system message (OpenAI-compatible shape).
    assert_eq!(
        body["messages"],
        json!([
            {"role": "system", "content": "Be terse."},
            {"role": "user", "content": "hello"}
        ])
    );
    assert_eq!(body["model"], json!("qwen3.8-flash"));
    assert_eq!(body["max_tokens"], json!(64));
}

#[test]
fn alibaba_streaming_stops_at_done_sentinel() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"ni\"}}]}\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"hao\"}}]}\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{}}]}\n",
        "\n",
        "data: [DONE]\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"AFTER\"}}]}\n",
        "\n",
    );
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse)
                    .insert_header("content-type", "text/event-stream"),
            )
            .mount(&server),
    );

    let provider = AlibabaProvider::with_base_url(Some(FAKE_KEY.to_string()), server.uri())
        .expect("construct");

    let mut tokens: Vec<String> = Vec::new();
    let completion = provider
        .complete_streaming(&request("qwen3.8-flash"), &mut |token: &str| {
            tokens.push(token.to_string());
        })
        .expect("streamed completion");

    // Role-only and empty chunks emit nothing; [DONE] stops consumption, so
    // the trailing event never surfaces.
    assert_eq!(tokens, vec!["ni".to_string(), "hao".to_string()]);
    assert_eq!(completion.text, "nihao");
    assert_eq!(completion.role, "assistant");

    let requests = rt
        .block_on(server.received_requests())
        .expect("requests recorded");
    let body: Value = serde_json::from_slice(&requests[0].body).expect("body json");
    assert_eq!(body["stream"], json!(true));
}

// ---------------------------------------------------------------------------
// Alibaba — BASE_URL env seam
// ---------------------------------------------------------------------------

#[test]
fn alibaba_base_url_env_override_routes_requests() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let rt = tokio::runtime::Runtime::new().expect("test runtime");

    let server = rt.block_on(async { MockServer::start().await });
    rt.block_on(
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "via env"}}]
            })))
            .mount(&server),
    );

    std::env::remove_var("ALIBABA_BASE_URL");
    std::env::set_var("ALIBABA_BASE_URL", server.uri());
    let provider = AlibabaProvider::new(Some(FAKE_KEY.to_string())).expect("construct");
    // The mock answers only if the env var actually routed the traffic.
    assert_eq!(
        provider.base_url().trim_end_matches('/').to_string(),
        server.uri().trim_end_matches('/').to_string()
    );
    assert_eq!(
        provider
            .complete(&request("qwen3.8-flash"))
            .expect("alibaba completion")
            .text,
        "via env"
    );

    // Env unset ⇒ the DashScope public compatible-mode default applies.
    std::env::remove_var("ALIBABA_BASE_URL");
    let default_based = AlibabaProvider::new(Some(FAKE_KEY.to_string())).expect("construct");
    assert_eq!(default_based.base_url(), ALIBABA_DEFAULT_BASE_URL);
}

#[test]
fn openai_compat_extraction_leaves_openai_and_anthropic_identity_unchanged() {
    // Regression pin for the openai_compat extraction: the openai/anthropic
    // suites above run unchanged, and their identity constants (config key,
    // default bases) are byte-identical to pre-extraction.
    let _guard = ENV_LOCK.lock().expect("env lock");
    std::env::remove_var("OPENAI_BASE_URL");
    std::env::remove_var("ANTHROPIC_BASE_URL");

    let openai = OpenAiProvider::new(None).expect("construct");
    assert_eq!(openai.key(), "openai");
    assert_eq!(openai.base_url(), "https://api.openai.com/v1");

    let anthropic = AnthropicProvider::new(None).expect("construct");
    assert_eq!(anthropic.key(), "anthropic");
    assert_eq!(anthropic.base_url(), "https://api.anthropic.com");
}

// ---------------------------------------------------------------------------
// Alibaba — credentials + config classes
// ---------------------------------------------------------------------------

#[test]
fn alibaba_missing_key_maps_to_exit_4_naming_the_env_var() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = rt.block_on(async { MockServer::start().await });

    let alibaba = AlibabaProvider::with_base_url(None, server.uri()).expect("construct");
    assert!(!alibaba.has_credentials());
    let err = alibaba
        .complete(&request("qwen3.8-flash"))
        .expect_err("no key");
    assert!(matches!(err, SmahError::MissingApiKey { .. }));
    assert_exit(&err, 4);
    assert!(
        err.render("/tmp/smah-test.log").contains("ALIBABA_API_KEY"),
        "message must name the env var: {}",
        err.render("/tmp/smah-test.log")
    );
}

#[test]
fn alibaba_malformed_block_maps_to_exit_3() {
    // `settings` must be a mapping; a scalar block is malformed.
    let scalar = settings_from(
        "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    alibaba:\n      name: Alibaba\n      enabled: true\n      settings: 42\n      models: []\n",
    );
    let err = get_provider("alibaba", &scalar, None)
        .err()
        .expect("malformed block");
    assert!(matches!(err, SmahError::ConfigUnreadable { .. }));
    assert_exit(&err, 3);

    // A provider block that is not a mapping at all.
    let non_mapping = settings_from(
        "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    alibaba: nope\n",
    );
    let err = get_provider("alibaba", &non_mapping, None)
        .err()
        .expect("malformed block");
    assert!(matches!(err, SmahError::ConfigUnreadable { .. }));
    assert_exit(&err, 3);
}

#[test]
fn alibaba_env_chain_prefers_the_smah_prefix() {
    // No alibaba block at all: env chain still resolves (Python base-class
    // parity — only OpenAI lacks this fallback).
    let settings = settings_from("vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers: {}\n");
    let resolved = resolve_api_key_with(
        &settings,
        "alibaba",
        None,
        &lookup_from(&[
            ("ALIBABA_API_KEY", "plain"),
            ("SMAH_ALIBABA_API_KEY", "smah"),
        ]),
    )
    .expect("resolve")
    .expect("key present");
    assert_eq!(resolved, "smah");

    // Plain fallback when the SMAH_ prefixed var is unset.
    let resolved = resolve_api_key_with(
        &settings,
        "alibaba",
        None,
        &lookup_from(&[("ALIBABA_API_KEY", "plain")]),
    )
    .expect("resolve")
    .expect("key present");
    assert_eq!(resolved, "plain");
}

#[test]
fn alibaba_env_indirection_resolves_and_falls_through() {
    // `$MY_DASHSCOPE_KEY` resolves through the environment.
    let settings = settings_from(
        "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    alibaba:\n      name: Alibaba\n      enabled: true\n      settings:\n        api_key: $MY_DASHSCOPE_KEY\n      models: []\n",
    );
    let resolved = resolve_api_key_with(
        &settings,
        "alibaba",
        None,
        &lookup_from(&[("MY_DASHSCOPE_KEY", "test-key-0000")]),
    )
    .expect("resolve")
    .expect("key present");
    assert_eq!(resolved, "test-key-0000");

    // `$MY_DASHSCOPE_KEY` unset ⇒ the provider env chain applies.
    let resolved = resolve_api_key_with(
        &settings,
        "alibaba",
        None,
        &lookup_from(&[("SMAH_ALIBABA_API_KEY", "test-key-0000")]),
    )
    .expect("resolve")
    .expect("key present");
    assert_eq!(resolved, "test-key-0000");
}

#[test]
fn registry_constructs_alibaba_provider() {
    let settings = alibaba_settings();
    let provider = get_provider("alibaba", &settings, None).expect("provider");
    assert_eq!(provider.key(), "alibaba");
    assert!(provider.has_credentials());

    // The registry advertises alibaba among the known providers.
    let known = known_provider_keys();
    assert!(known.contains(&"alibaba"), "known keys: {known:?}");
    assert_eq!(known, {
        let mut sorted = known.clone();
        sorted.sort_unstable();
        sorted
    });
}

// ---------------------------------------------------------------------------
// Alibaba — real-endpoint smoke (env-gated, ignored)
// ---------------------------------------------------------------------------

/// Real DashScope compatible-mode smoke for the virtual-tools e2e program.
///
/// Runs ONLY when `SMAH_E2E_DASHSCOPE=1` AND `SMAH_ALIBABA_API_KEY` are set
/// (`ALIBABA_BASE_URL` optionally overrides the endpoint — e.g. a dedicated
/// MAAS host). Skips cleanly otherwise, which is what CI does: the test is
/// `#[ignore]`d and CI never sets the gate, so it never runs (and never
/// fails) there. Run locally with:
///
/// ```text
/// SMAH_E2E_DASHSCOPE=1 SMAH_ALIBABA_API_KEY=... cargo test \
///     --test providers_wiremock -- --ignored alibaba_real_endpoint_smoke
/// ```
///
/// The key is never printed, logged, or asserted on — only its presence
/// gates the run; any non-2xx HTTP status surfaces as the standard §4
/// provider error.
#[test]
#[ignore = "real-endpoint smoke: requires SMAH_E2E_DASHSCOPE=1 + SMAH_ALIBABA_API_KEY"]
fn alibaba_real_endpoint_smoke() {
    let gated = std::env::var("SMAH_E2E_DASHSCOPE")
        .map(|value| value.trim() == "1")
        .unwrap_or(false);
    let api_key = std::env::var("SMAH_ALIBABA_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty());
    if !gated || api_key.is_none() {
        eprintln!("skipping alibaba_real_endpoint_smoke: SMAH_E2E_DASHSCOPE=1 and SMAH_ALIBABA_API_KEY required");
        return;
    }

    let provider = AlibabaProvider::new(api_key).expect("construct");
    let mut req = CompletionRequest::new(
        "qwen3.8-flash",
        vec![ChatMessage::user("Reply with the single word: ok")],
    );
    req.max_tokens = Some(16);
    let completion = provider.complete(&req).expect("real completion");
    assert!(
        !completion.text.trim().is_empty(),
        "expected non-empty text from the real endpoint"
    );
}
