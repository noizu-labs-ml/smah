"""
Provider abstraction tests: registry resolution, Anthropic request shape,
content-block parsing, error-to-exit-code mapping, credential preflight, and
preservation of the OpenAI request parameters. The SDKs are mocked — no
network, no real API calls, no spending.
"""
from types import SimpleNamespace
from unittest import mock

import httpx
import pytest

import anthropic
import openai
from openai import NOT_GIVEN

from smah.exit_codes import ExitCode
from smah.providers import AnthropicProvider, OpenAIProvider, get_provider
from smah.settings.inference.provider.model import Model


def clean_env(monkeypatch):
    for var in (
        "ANTHROPIC_API_KEY", "SMAH_ANTHROPIC_API_KEY",
        "OPENAI_API_KEY", "SMAH_OPENAI_API_KEY",
    ):
        monkeypatch.delenv(var, raising=False)


def claude_model():
    return Model("anthropic", {
        "name": "claude-opus-5-5",
        "model": "claude-opus-5-5",
        "enabled": True,
        "context": {"window": 200000, "out": 32000},
    })


def text_block(text):
    return SimpleNamespace(type="text", text=text)


def fake_message(content):
    return SimpleNamespace(
        content=content,
        id="msg_test",
        model="claude-opus-5-5",
        stop_reason="end_turn",
    )


def anthropic_response(content):
    client = mock.MagicMock()
    client.messages.create.return_value = fake_message(content)
    return client


def anthropic_error_response(status):
    request = httpx.Request("POST", "https://api.anthropic.com/v1/messages")
    return httpx.Response(status, request=request)


# ----------------------------------------------------------------------
# Registry
# ----------------------------------------------------------------------
def test_registry_resolves_config_keys_to_implementations():
    openai_provider = get_provider("openai", args=None, settings_provider=None)
    anthropic_provider = get_provider("anthropic", args=None, settings_provider=None)
    assert isinstance(openai_provider, OpenAIProvider)
    assert isinstance(anthropic_provider, AnthropicProvider)


def test_unknown_provider_key_exits_config(monkeypatch):
    console = mock.MagicMock()
    with mock.patch("smah.exit_codes.err_console", console):
        with pytest.raises(SystemExit) as excinfo:
            get_provider("mistral", args=None, settings_provider=None)
    assert excinfo.value.code == int(ExitCode.CONFIG)
    text = "".join(str(call.args) for call in console.print.call_args_list)
    assert "mistral" in text
    assert "openai" in text and "anthropic" in text


# ----------------------------------------------------------------------
# Credential resolution (never printed)
# ----------------------------------------------------------------------
def test_anthropic_key_resolves_from_env_fallback(monkeypatch):
    clean_env(monkeypatch)
    provider = get_provider("anthropic", args=SimpleNamespace(), settings_provider=None)
    assert provider.has_credentials() is False

    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    assert provider.has_credentials() is True
    assert provider.resolve_api_key() == "sk-ant-test"

    monkeypatch.delenv("ANTHROPIC_API_KEY")
    monkeypatch.setenv("SMAH_ANTHROPIC_API_KEY", "sk-ant-prefixed")
    assert provider.resolve_api_key() == "sk-ant-prefixed"


def test_anthropic_preflight_exits_before_any_network_call(monkeypatch):
    clean_env(monkeypatch)
    provider = get_provider("anthropic", args=SimpleNamespace(), settings_provider=None)
    with mock.patch("anthropic.Anthropic") as client_factory:
        with pytest.raises(SystemExit) as excinfo:
            provider.complete(claude_model(), [{"role": "user", "content": "hi"}])
    assert excinfo.value.code == int(ExitCode.AUTH_OR_MODEL)
    client_factory.assert_not_called()


def test_anthropic_missing_key_message_names_the_right_variables(monkeypatch):
    clean_env(monkeypatch)
    provider = get_provider("anthropic", args=SimpleNamespace(), settings_provider=None)
    message = provider.credential_help()
    assert "ANTHROPIC_API_KEY" in message
    assert "SMAH_ANTHROPIC_API_KEY" in message
    assert "--configure" in message


def test_credentials_are_never_included_in_messages(monkeypatch):
    clean_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-secret-value")
    provider = get_provider("anthropic", args=SimpleNamespace(), settings_provider=None)
    assert "sk-ant-secret-value" not in provider.credential_help()


def test_settings_key_resolves_without_args(monkeypatch):
    # A provider constructed without a CLI namespace must still consult the
    # configured settings block — not silently fall through to env-only.
    from smah.settings.inference.provider.provider import Provider as ProviderSettings

    clean_env(monkeypatch)

    anthropic_block = ProviderSettings("anthropic", {
        "name": "Anthropic", "enabled": True, "settings": {"api_key": "cfg-ant-key"},
    })
    provider = get_provider("anthropic", args=None, settings_provider=anthropic_block)
    assert provider.has_credentials() is True
    assert provider.resolve_api_key() == "cfg-ant-key"

    openai_block = ProviderSettings("openai", {
        "name": "OpenAI", "enabled": True, "settings": {"api_key": "cfg-openai-key"},
    })
    provider = get_provider("openai", args=None, settings_provider=openai_block)
    assert provider.has_credentials() is True
    assert provider.resolve_api_key() == "cfg-openai-key"


def test_only_openai_provider_imports_the_openai_sdk():
    # The provider-neutral layer must not depend on any vendor SDK: only
    # openai_provider.py may import from openai.
    import pathlib

    import smah

    providers_dir = pathlib.Path(smah.__file__).parent / "providers"
    offenders = []
    for py in sorted(providers_dir.glob("*.py")):
        if py.name == "openai_provider.py":
            continue
        text = py.read_text(encoding="utf-8")
        for marker in ("from openai", "import openai"):
            if marker in text:
                offenders.append(f"{py.name}: {marker}")
    assert offenders == [], f"vendor SDK leaked into neutral layer: {offenders}"


# ----------------------------------------------------------------------
# Anthropic request shape
# ----------------------------------------------------------------------
def test_anthropic_request_shape_system_top_level_max_tokens_no_forbidden_params(monkeypatch):
    clean_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    provider = get_provider("anthropic", args=SimpleNamespace(), settings_provider=None)
    client = anthropic_response([text_block("answer")])

    with mock.patch("anthropic.Anthropic", return_value=client):
        completion = provider.complete(claude_model(), [{"role": "user", "content": "hi"}])

    kwargs = client.messages.create.call_args.kwargs
    assert kwargs["model"] == "claude-opus-5-5"
    assert kwargs["max_tokens"] == 32000  # honors the model's configured output budget
    # system is top-level only when provided; never a system-role message
    assert "system" not in kwargs
    assert all(m["role"] in ("user", "assistant") for m in kwargs["messages"])
    assert kwargs["messages"] == [{"role": "user", "content": "hi"}]
    for forbidden in ("temperature", "top_p", "top_k", "budget_tokens", "thinking"):
        assert forbidden not in kwargs, f"{forbidden} must not be sent"
    assert completion.text == "answer"
    assert completion.role == "assistant"


def test_anthropic_response_format_becomes_json_system_instruction_not_response_format(monkeypatch):
    clean_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    provider = get_provider("anthropic", args=SimpleNamespace(), settings_provider=None)
    client = anthropic_response([text_block('{"model": "anthropic.claude-opus-5-5"}')])

    with mock.patch("anthropic.Anthropic", return_value=client):
        provider.complete(
            claude_model(),
            [{"role": "user", "content": "pick a model"}],
            response_format={"type": "json_object"},
        )

    kwargs = client.messages.create.call_args.kwargs
    assert isinstance(kwargs["system"], str)
    assert "JSON" in kwargs["system"]
    assert "response_format" not in kwargs


def test_anthropic_max_tokens_falls_back_to_default_without_budget(monkeypatch):
    clean_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    provider = get_provider("anthropic", args=SimpleNamespace(), settings_provider=None)
    client = anthropic_response([text_block("ok")])
    model = Model("anthropic", {"name": "claude-opus-5-5", "model": "claude-opus-5-5", "enabled": True})

    with mock.patch("anthropic.Anthropic", return_value=client):
        provider.complete(model, [{"role": "user", "content": "hi"}])

    assert client.messages.create.call_args.kwargs["max_tokens"] == 16000


def test_anthropic_max_tokens_options_override_model_budget(monkeypatch):
    clean_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    provider = get_provider("anthropic", args=SimpleNamespace(), settings_provider=None)
    client = anthropic_response([text_block("ok")])

    with mock.patch("anthropic.Anthropic", return_value=client):
        provider.complete(claude_model(), [{"role": "user", "content": "hi"}], options={"max_tokens": 512})

    assert client.messages.create.call_args.kwargs["max_tokens"] == 512


def test_maybe_get_provider_returns_none_for_unknown_keys():
    from smah.providers import maybe_get_provider

    assert maybe_get_provider("mistral", args=None, settings_provider=None) is None
    resolved = maybe_get_provider("anthropic", args=SimpleNamespace(), settings_provider=None)
    assert isinstance(resolved, AnthropicProvider)


# ----------------------------------------------------------------------
# Content-block parsing
# ----------------------------------------------------------------------
def test_anthropic_multi_block_response_joins_text_blocks(monkeypatch):
    clean_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    provider = get_provider("anthropic", args=SimpleNamespace(), settings_provider=None)
    blocks = [
        text_block("first"),
        SimpleNamespace(type="thinking", thinking="internal"),  # non-text block
        text_block("second"),
    ]
    client = anthropic_response(blocks)

    with mock.patch("anthropic.Anthropic", return_value=client):
        completion = provider.complete(claude_model(), [{"role": "user", "content": "hi"}])
    assert completion.text == "first\nsecond"


def test_anthropic_non_text_first_block_is_not_assumed(monkeypatch):
    clean_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    provider = get_provider("anthropic", args=SimpleNamespace(), settings_provider=None)
    blocks = [
        SimpleNamespace(type="server_tool_use", input={}),
        text_block("the real answer"),
    ]
    client = anthropic_response(blocks)

    with mock.patch("anthropic.Anthropic", return_value=client):
        completion = provider.complete(claude_model(), [{"role": "user", "content": "hi"}])
    assert completion.text == "the real answer"


def test_anthropic_tool_and_set_condition_roles_are_dropped_from_messages(monkeypatch):
    clean_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    provider = get_provider("anthropic", args=SimpleNamespace(), settings_provider=None)
    client = anthropic_response([text_block("ok")])

    with mock.patch("anthropic.Anthropic", return_value=client):
        provider.complete(claude_model(), [
            {"role": "system", "content": "should be dropped"},
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "hello"},
        ])

    kwargs = client.messages.create.call_args.kwargs
    assert [m["role"] for m in kwargs["messages"]] == ["user", "assistant"]


# ----------------------------------------------------------------------
# Error translation: most-specific-first chain -> exit codes
# ----------------------------------------------------------------------
def _status_error(exc_cls, status, body=None):
    return exc_cls(
        "api exploded",
        response=anthropic_error_response(status),
        body=body or {"error": {"message": "api exploded"}},
    )


@pytest.mark.parametrize("exc_cls,status,expected", [
    (anthropic.AuthenticationError, 401, ExitCode.AUTH_OR_MODEL),
    (anthropic.NotFoundError, 404, ExitCode.PROVIDER),
    (anthropic.RateLimitError, 429, ExitCode.PROVIDER),
    (anthropic.BadRequestError, 400, ExitCode.PROVIDER),
    (anthropic.APIStatusError, 500, ExitCode.PROVIDER),
])
def test_anthropic_status_errors_map_to_exit_codes(exc_cls, status, expected):
    provider = AnthropicProvider(args=SimpleNamespace(), settings_provider=None)
    with pytest.raises(SystemExit) as excinfo:
        provider.translate_error(_status_error(exc_cls, status))
    assert excinfo.value.code == int(expected)


def test_anthropic_connection_error_maps_to_provider_exit():
    provider = AnthropicProvider(args=SimpleNamespace(), settings_provider=None)
    request = httpx.Request("POST", "https://api.anthropic.com/v1/messages")
    exc = anthropic.APIConnectionError(request=request, message="connection refused")
    with pytest.raises(SystemExit) as excinfo:
        provider.translate_error(exc)
    assert excinfo.value.code == int(ExitCode.PROVIDER)


def test_anthropic_timeout_error_maps_to_provider_exit():
    provider = AnthropicProvider(args=SimpleNamespace(), settings_provider=None)
    request = httpx.Request("POST", "https://api.anthropic.com/v1/messages")
    exc = anthropic.APITimeoutError(request=request)
    with pytest.raises(SystemExit) as excinfo:
        provider.translate_error(exc)
    assert excinfo.value.code == int(ExitCode.PROVIDER)


def test_anthropic_translate_error_reraises_foreign_exceptions():
    provider = AnthropicProvider(args=SimpleNamespace(), settings_provider=None)
    with pytest.raises(ValueError):
        provider.translate_error(ValueError("not mine"))


# ----------------------------------------------------------------------
# OpenAI behavior preservation
# ----------------------------------------------------------------------
def _openai_provider(monkeypatch, provider_block=None):
    clean_env(monkeypatch)
    if provider_block is None:
        provider_block = mock.MagicMock()
        provider_block.api_key.return_value = "sk-openai-test"
    return get_provider(
        "openai",
        args=SimpleNamespace(openai_api_key=None),
        settings_provider=provider_block,
    )


def _openai_model(context=None, settings=None):
    data = {"name": "gpt-4o-mini", "model": "gpt-4o-mini", "enabled": True}
    if context:
        data["context"] = context
    if settings:
        data["settings"] = settings
    return Model("openai", data)


def test_openai_request_parameters_preserved(monkeypatch):
    provider = _openai_provider(monkeypatch)
    client = mock.MagicMock()
    client.chat.completions.create.return_value = SimpleNamespace(
        choices=[SimpleNamespace(message=SimpleNamespace(role="assistant", content="answer"))]
    )

    with mock.patch("openai.OpenAI", return_value=client) as factory:
        completion = provider.complete(
            _openai_model(context={"window": 128000, "out": 16384}),
            [{"role": "user", "content": "hi"}],
        )

    factory.assert_called_once_with(api_key="sk-openai-test")
    kwargs = client.chat.completions.create.call_args.kwargs
    assert kwargs["model"] == "gpt-4o-mini"
    assert kwargs["messages"] == [{"role": "user", "content": "hi"}]
    assert kwargs["max_tokens"] == 16384
    assert kwargs["max_completion_tokens"] is NOT_GIVEN
    assert kwargs["response_format"] is NOT_GIVEN
    assert kwargs["tools"] is NOT_GIVEN
    assert completion.text == "answer"
    assert completion.role == "assistant"


def test_openai_model_settings_max_completion_tokens_still_wins(monkeypatch):
    provider = _openai_provider(monkeypatch)
    client = mock.MagicMock()
    client.chat.completions.create.return_value = SimpleNamespace(
        choices=[SimpleNamespace(message=SimpleNamespace(role="assistant", content="ok"))]
    )

    with mock.patch("openai.OpenAI", return_value=client):
        provider.complete(
            _openai_model(context={"out": 4096}, settings={"max_completion_tokens": 1024}),
            [{"role": "user", "content": "hi"}],
        )

    kwargs = client.chat.completions.create.call_args.kwargs
    assert kwargs["max_completion_tokens"] == 1024
    assert kwargs["max_tokens"] is NOT_GIVEN


def test_openai_auth_rejected_maps_to_auth_exit(monkeypatch):
    provider = _openai_provider(monkeypatch)
    client = mock.MagicMock()
    client.chat.completions.create.side_effect = _status_error(openai.AuthenticationError, 401)

    with mock.patch("openai.OpenAI", return_value=client):
        with pytest.raises(SystemExit) as excinfo:
            provider.complete(_openai_model(), [{"role": "user", "content": "hi"}])
    assert excinfo.value.code == int(ExitCode.AUTH_OR_MODEL)


def test_openai_rate_limit_maps_to_provider_exit(monkeypatch):
    provider = _openai_provider(monkeypatch)
    client = mock.MagicMock()
    client.chat.completions.create.side_effect = _status_error(openai.RateLimitError, 429)

    with mock.patch("openai.OpenAI", return_value=client):
        with pytest.raises(SystemExit) as excinfo:
            provider.complete(_openai_model(), [{"role": "user", "content": "hi"}])
    assert excinfo.value.code == int(ExitCode.PROVIDER)
