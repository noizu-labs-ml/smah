from types import SimpleNamespace
from unittest import mock

from openai import OpenAIError
from rich.console import Console

import smah.smah as smah_main
from smah.exit_codes import ExitCode


def base_args(**overrides):
    args = SimpleNamespace(
        resume=False,
        session=None,
        history=False,
        interactive=False,
        config=None,
        configure=False,
        verbose=0,
        rich=True,
        query=None,
        instructions=None,
    )
    for key, value in overrides.items():
        setattr(args, key, value)
    return args


def run_main(args, pipe=None, runner=None):
    """
    Runs smah.smah.main() with the heavy collaborators mocked out.

    Returns (exit_code_or_None, recorded_stderr_text).
    """
    console = Console(record=True, width=100)
    settings_instance = mock.MagicMock()
    settings_instance.is_configured.return_value = True
    runner = runner if runner is not None else mock.MagicMock()

    with mock.patch("smah.logs.configure"), \
            mock.patch("smah.args.extract_args", return_value=(args, pipe)), \
            mock.patch("smah.smah.init_database"), \
            mock.patch("smah.smah.Settings", return_value=settings_instance), \
            mock.patch("smah.smah.Runner", return_value=runner), \
            mock.patch("smah.smah.err_console", console):
        code = None
        try:
            smah_main.main()
        except SystemExit as e:
            code = e.code
    return code, console.export_text()


def test_query_dispatches_to_runner_and_does_not_exit():
    runner = mock.MagicMock()
    code, _ = run_main(base_args(query="hello"), runner=runner)
    assert code is None
    runner.query.assert_called_once_with(query="hello")


def test_pipe_dispatches_to_runner_pipe():
    runner = mock.MagicMock()
    code, _ = run_main(base_args(query="summarize"), pipe="piped body", runner=runner)
    assert code is None
    runner.pipe.assert_called_once_with(query="summarize", pipe="piped body")


def test_bare_smah_exits_usage_with_hint():
    code, text = run_main(base_args(), pipe=None)
    assert code == int(ExitCode.USAGE)
    assert "smah --help" in text
    assert "--continue" in text


def test_piped_input_without_query_exits_usage():
    code, text = run_main(base_args(), pipe="some piped content")
    assert code == int(ExitCode.USAGE)
    assert "no query was given" in text


def test_interactive_flag_is_explicitly_not_implemented():
    code, text = run_main(base_args(interactive=True))
    assert code == int(ExitCode.USAGE)
    assert "not implemented" in text


def test_unexpected_error_exits_one_and_names_failure():
    def boom(*a, **k):
        raise ValueError("kaboom")

    runner = mock.MagicMock()
    runner.query.side_effect = boom

    code, text = run_main(base_args(query="hello"), runner=runner)
    assert code == int(ExitCode.UNEXPECTED_ERROR)
    assert "kaboom" in text
    # full traceback only when verbosity is raised
    assert "Traceback" not in text

    code, text = run_main(base_args(query="hello", verbose=1), runner=runner)
    assert code == int(ExitCode.UNEXPECTED_ERROR)
    assert "Traceback" in text


def test_openai_error_exits_provider():
    def boom(*a, **k):
        raise OpenAIError("api exploded")

    runner = mock.MagicMock()
    runner.query.side_effect = boom

    code, text = run_main(base_args(query="hello"), runner=runner)
    assert code == int(ExitCode.PROVIDER)
    assert "api exploded" in text
