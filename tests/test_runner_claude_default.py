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
