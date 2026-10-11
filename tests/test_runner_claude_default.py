"""
End-to-end verification that the Claude-first default configuration runs a
query and a pipe without any real API call: the Anthropic SDK is replaced
with an in-memory fake, and a real Runner (real Settings/Inference from the
defaults yaml, real temp sqlite database) drives the whole query/pipe flow —
planner on the haiku model, answer on the opus model, normalized
Completion parsing, command extraction, and history persistence.
"""
import json
import os
from types import SimpleNamespace
from unittest import mock

import yaml

from smah.runner.runner import Runner
from smah.settings.inference.inference import Inference

DEFAULTS_YAML = os.path.join(
    os.path.dirname(__file__), "..", "smah", "settings", "inference", "inference_defaults.yaml"
)

PLANNER_PLAN = {
    "title": "test session",
    "model": "anthropic.claude-opus-5-5",
    "reason": "simple question",
    "include_settings": False,
    "include_settings_reason": "not needed",
    "format_output": False,
    "format_output_reason": "plain answer",
    "instructions": "Answer directly.",
}


def clean_provider_env(monkeypatch):
    for var in ("ANTHROPIC_API_KEY", "SMAH_ANTHROPIC_API_KEY", "OPENAI_API_KEY", "SMAH_OPENAI_API_KEY"):
        monkeypatch.delenv(var, raising=False)


def real_inference():
    with open(DEFAULTS_YAML, "r") as file:
        return Inference(yaml.safe_load(file))


def fake_anthropic_sdk(planner_reply: str, answer_reply: str):
    """Returns (client_mock, models_used) with sequenced plan/answer responses."""
    client = mock.MagicMock()
    models_used = []

    def fake_create(**kwargs):
        models_used.append(kwargs["model"])
        reply = planner_reply if len(models_used) == 1 else answer_reply
        return SimpleNamespace(
            content=[SimpleNamespace(type="text", text=reply)],
            id="msg_test",
            model=kwargs["model"],
            stop_reason="end_turn",
        )

    client.messages.create.side_effect = fake_create
    return client, models_used


def make_runner(tmp_path, inference, rich=False):
    settings = mock.MagicMock()
    settings.inference = inference
    settings.user.to_yaml.return_value = {"name": "Test User"}
    settings.system.to_yaml.return_value = {"os": "test"}
    args = SimpleNamespace(
        model=None, model_query=None, model_pipe=None,
        rich=rich, verbose=0, openai_api_key=None,
        database=str(tmp_path / "smah-test.db"),
    )
    runner = Runner(args, settings)
    # smah.main() migrates the database before use; do the same for the temp db.
    from smah.database import Migration
    outcome, _ = Migration.migrate(
        runner.db, SimpleNamespace(silent=True, count=None, to=None), silent=True, exit_on_finish=False
    )
    assert outcome == "success"
    return runner


def test_claude_default_query_end_to_end_with_only_anthropic_key(tmp_path, monkeypatch, capsys):
    clean_provider_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test-only")

    runner = make_runner(tmp_path, real_inference())
    client, models_used = fake_anthropic_sdk(json.dumps(PLANNER_PLAN), "The answer is 4.")

    with mock.patch("anthropic.Anthropic", return_value=client) as sdk:
        result = runner.query("what is 2+2")

    assert result == "The answer is 4."
    # planner ran on haiku, answer on opus — with only ANTHROPIC_API_KEY set
    assert models_used == ["claude-haiku-5-5", "claude-opus-5-5"]
    sdk.assert_called()
    out = capsys.readouterr().out
    assert "The answer is 4." in out
    assert "sk-ant-test-only" not in out + capsys.readouterr().err

    # history persisted through the real temp database
    assert len(runner.db.history()) == 1


def test_claude_default_pipe_end_to_end_with_only_anthropic_key(tmp_path, monkeypatch, capsys):
    clean_provider_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test-only")

    runner = make_runner(tmp_path, real_inference())
    client, models_used = fake_anthropic_sdk(json.dumps(PLANNER_PLAN), "processed output")

    with mock.patch("anthropic.Anthropic", return_value=client):
        result = runner.pipe("summarize", "piped body content")

    assert result == "processed output"
    assert models_used == ["claude-haiku-5-5", "claude-opus-5-5"]
    assert "processed output" in capsys.readouterr().out


def test_openai_only_environment_selects_openai_models(tmp_path, monkeypatch):
    # Only OPENAI_API_KEY set: the credential-aware picker falls back from the
    # Claude-first default to an OpenAI model instead of exiting.
    clean_provider_env(monkeypatch)
    monkeypatch.setenv("OPENAI_API_KEY", "sk-openai-test")

    runner = make_runner(tmp_path, real_inference())
    planner = runner.inference_model("query")
    assert planner is not None
    assert planner.provider == "openai"
    assert planner.model == "gpt-4o-mini"


def test_anthropic_first_when_both_keys_available(tmp_path, monkeypatch):
    clean_provider_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")
    monkeypatch.setenv("OPENAI_API_KEY", "sk-openai-test")

    runner = make_runner(tmp_path, real_inference())
    planner = runner.inference_model("query")
    assert planner.provider == "anthropic"
    assert planner.model == "claude-haiku-5-5"


def test_no_keys_anywhere_still_fails_with_named_env_var(tmp_path, monkeypatch):
    from smah.exit_codes import ExitCode

    clean_provider_env(monkeypatch)
    runner = make_runner(tmp_path, real_inference())
    client, _ = fake_anthropic_sdk("x", "y")

    with mock.patch("anthropic.Anthropic", return_value=client) as sdk:
        try:
            runner.query("what is 2+2")
            raised = None
        except SystemExit as e:
            raised = e
    assert raised is not None
    assert raised.code == int(ExitCode.AUTH_OR_MODEL)
    sdk.assert_not_called()  # preflight fired before any SDK construction
    client.messages.create.assert_not_called()


def test_planner_prose_output_exits_five_with_readable_message(tmp_path, monkeypatch, capsys):
    # The Anthropic JSON-mode shim is an instruction, so the planner can answer
    # in prose; that must produce a clean exit-5 message, never a traceback.
    from smah.exit_codes import ExitCode

    clean_provider_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test-only")

    runner = make_runner(tmp_path, real_inference())
    client, models_used = fake_anthropic_sdk(
        "Sorry, I cannot answer that in JSON format.", "never reached"
    )

    with mock.patch("anthropic.Anthropic", return_value=client):
        try:
            runner.query("what is 2+2")
            raised = None
        except SystemExit as e:
            raised = e

    assert raised is not None
    assert raised.code == int(ExitCode.PROVIDER)
    assert len(models_used) == 1  # planner call only; no answer call happened
    err = capsys.readouterr().err
    assert "planner" in err
    assert "JSON" in err
    assert "Traceback" not in err


UNKNOWN_PROVIDER_CONFIG = {
    "model_picker": {
        "query": ["mistral.big", "anthropic.claude-haiku-5-5"],
        "default": ["anthropic.claude-haiku-5-5"],
    },
    "providers": {
        "mistral": {
            "name": "Mistral",
            "enabled": True,
            "models": [{"name": "big", "model": "mistral-large", "enabled": True}],
        },
        "anthropic": {
            "name": "Anthropic",
            "enabled": True,
            "models": [{"name": "claude-haiku-5-5", "model": "claude-haiku-5-5", "enabled": True}],
        },
    },
}


def test_unknown_provider_among_picker_candidates_is_skipped(tmp_path, monkeypatch):
    # An unknown provider key listed ahead of good candidates must not kill
    # the run while probing; the working candidate is still selected.
    clean_provider_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")

    runner = make_runner(tmp_path, Inference(UNKNOWN_PROVIDER_CONFIG))
    planner = runner.inference_model("query")
    assert planner is not None
    assert planner.provider == "anthropic"
    assert planner.model == "claude-haiku-5-5"


def test_unknown_provider_as_selected_model_still_exits_config(tmp_path, monkeypatch):
    # Hard exit-3 stays on the provider actually chosen to run.
    from smah.exit_codes import ExitCode

    clean_provider_env(monkeypatch)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "sk-ant-test")

    runner = make_runner(tmp_path, Inference(UNKNOWN_PROVIDER_CONFIG))
    mistral_model = runner.settings.inference.models["mistral.big"]
    try:
        runner.run(mistral_model, [{"role": "user", "content": "hi"}])
        raised = None
    except SystemExit as e:
        raised = e
    assert raised is not None
    assert raised.code == int(ExitCode.CONFIG)


MALFORMED_PROVIDER_CONFIG = {
    "model_picker": {
        "query": ["anthropic.claude-haiku-5-5", "openai.gpt-4o-mini"],
        "default": ["anthropic.claude-haiku-5-5", "openai.gpt-4o-mini"],
    },
    "providers": {
        "anthropic": {
            "name": "Anthropic",
            "enabled": True,
            "settings": {"api_key": {"bad": "shape"}},
            "models": [{"name": "claude-haiku-5-5", "model": "claude-haiku-5-5", "enabled": True}],
        },
        "openai": {
            "name": "OpenAI",
            "enabled": True,
            "models": [{"name": "gpt-4o-mini", "model": "gpt-4o-mini", "enabled": True}],
        },
    },
}


def test_malformed_provider_block_among_candidates_is_skipped(tmp_path, monkeypatch, caplog):
    # Finding 3221acbf: a known-key provider whose settings block is
    # malformed must be skipped while probing — with a debug breadcrumb, not
    # a silent swallow — so a later healthy candidate still wins.
    import logging

    clean_provider_env(monkeypatch)
    monkeypatch.setenv("OPENAI_API_KEY", "sk-openai-test")

    runner = make_runner(tmp_path, Inference(MALFORMED_PROVIDER_CONFIG))
    with caplog.at_level(logging.DEBUG):
        planner = runner.inference_model("query")
    assert planner is not None
    assert planner.provider == "openai"
    assert planner.model == "gpt-4o-mini"
    assert "anthropic" in caplog.text


def test_malformed_selected_provider_is_clean_config_exit(tmp_path, monkeypatch, capsys):
    # The chosen provider with the same malformed block: clean exit 3, a
    # readable message, no traceback.
    from smah.exit_codes import ExitCode

    clean_provider_env(monkeypatch)
    runner = make_runner(tmp_path, Inference(MALFORMED_PROVIDER_CONFIG))
    model = runner.settings.inference.models["anthropic.claude-haiku-5-5"]
    try:
        runner.run(model, [{"role": "user", "content": "hi"}])
        raised = None
    except SystemExit as e:
        raised = e
    assert raised is not None
    assert raised.code == int(ExitCode.CONFIG)
    err = capsys.readouterr().err
    flat = " ".join(err.split())  # err_console wraps lines mid-string
    assert "malformed" in flat
    assert "smah --configure" in flat
    assert "Traceback" not in err
