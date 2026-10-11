import textwrap
from unittest import mock

from rich.console import Console

from smah.exit_codes import ExitCode
from smah.runner.runner import Runner


def test_exit_code_values():
    assert int(ExitCode.OK) == 0
    assert int(ExitCode.UNEXPECTED_ERROR) == 1
    assert int(ExitCode.USAGE) == 2
    assert int(ExitCode.CONFIG) == 3
    assert int(ExitCode.AUTH_OR_MODEL) == 4
    assert int(ExitCode.PROVIDER) == 5


def test_show_suggested_commands_is_print_only():
    console = Console(record=True, width=100)
    commands = [
        {
            "title": "Echo Hello World",
            "purpose": "Show how to echo Hello World to the console",
            "command": "echo 'Hello World'",
            "shell": "zsh",
        }
    ]
    with mock.patch("smah.runner.runner.err_console", console):
        Runner.show_suggested_commands(commands)
    text = console.export_text()
    assert "echo 'Hello World'" in text
    assert "Echo Hello World" in text
    assert "Show how to echo Hello World" in text
    lowered = text.lower()
    assert "does not run commands" in lowered or "not executed" in lowered


def test_show_suggested_commands_handles_missing_fields():
    console = Console(record=True, width=100)
    commands = [{"title": None, "purpose": None, "command": "uptime", "shell": None}]
    with mock.patch("smah.runner.runner.err_console", console):
        Runner.show_suggested_commands(commands)
    text = console.export_text()
    assert "uptime" in text
    assert "does not run commands" in text.lower() or "not executed" in text.lower()


def test_clipboard_absent_degrades_silently():
    with mock.patch("smah.runner.runner.pyperclip", None):
        assert Runner.copy_to_clipboard("example") is False


def test_clipboard_failure_degrades_silently():
    with mock.patch("smah.runner.runner.pyperclip", mock.MagicMock()) as clip:
        clip.copy.side_effect = RuntimeError("no clipboard")
        assert Runner.copy_to_clipboard("example") is False


def _bare_runner() -> Runner:
    # skip Runner.__init__ (it opens the sqlite history database)
    return Runner.__new__(Runner)


def test_run_without_model_exits_auth_or_model():
    runner = _bare_runner()
    try:
        runner.run(None, [])
        assert False, "expected SystemExit"
    except SystemExit as e:
        assert e.code == int(ExitCode.AUTH_OR_MODEL)


def test_run_rejects_unknown_provider_key_as_config_error():
    from types import SimpleNamespace

    from smah.settings.inference.provider.model import Model

    runner = _bare_runner()
    runner.args = SimpleNamespace(openai_api_key=None)
    runner.settings = SimpleNamespace(inference=SimpleNamespace(providers={}))
    model = Model("mistral", {"name": "big-model", "model": "mistral-big", "enabled": True})
    try:
        runner.run(model, [])
        assert False, "expected SystemExit"
    except SystemExit as e:
        assert e.code == int(ExitCode.CONFIG)


def test_openai_provider_without_api_key_exits_auth_or_model_before_any_client(monkeypatch):
    from types import SimpleNamespace

    from smah.providers import get_provider
    from smah.settings.inference.provider.model import Model

    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    monkeypatch.delenv("SMAH_OPENAI_API_KEY", raising=False)
    provider_block = mock.MagicMock()
    provider_block.api_key.return_value = None
    provider = get_provider(
        "openai",
        args=SimpleNamespace(openai_api_key=None),
        settings_provider=provider_block,
    )
    model = Model("openai", {"name": "gpt-4o-mini", "model": "gpt-4o-mini", "enabled": True})
    with mock.patch("openai.OpenAI") as client_factory:
        try:
            provider.complete(model, [{"role": "user", "content": "hi"}])
            assert False, "expected SystemExit"
        except SystemExit as e:
            assert e.code == int(ExitCode.AUTH_OR_MODEL)
        client_factory.assert_not_called()


def test_openai_provider_without_openai_provider_block_exits_auth_or_model(monkeypatch):
    from types import SimpleNamespace

    from smah.providers import get_provider

    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    monkeypatch.delenv("SMAH_OPENAI_API_KEY", raising=False)
    provider = get_provider("openai", args=SimpleNamespace(openai_api_key=None), settings_provider=None)
    try:
        provider.require_api_key()
        assert False, "expected SystemExit"
    except SystemExit as e:
        assert e.code == int(ExitCode.AUTH_OR_MODEL)
