import io
from types import SimpleNamespace
from unittest import mock

import pytest
import yaml
from openai import OpenAIError
from rich.console import Console

import smah.smah as smah_main
from smah.exit_codes import ExitCode


class FakeTTY(io.StringIO):
    """A stdin stand-in that claims to be a terminal."""

    def isatty(self):
        return True


def base_args(**overrides):
    args = SimpleNamespace(
        command=None,
        deprecated=[],
        # legacy surface
        resume=False,
        session=None,
        history=False,
        interactive=False,
        configure=False,
        config=None,
        database=None,
        verbose=0,
        rich=True,
        query=None,
        instructions=None,
        model=None,
        model_query=None,
        model_pipe=None,
        api_key=None,
        openai_api_key=None,
        # subcommand surface
        ask_query=None,
        ask_instructions=None,
        resume_id=None,
        resume_last=False,
        history_limit=20,
        config_command=None,
        config_set_key=None,
        config_set_value=None,
        models_command=None,
        doctor_json=False,
        doctor_offline=False,
    )
    for key, value in overrides.items():
        setattr(args, key, value)
    return args


def run(args, pipe=None, runner=None, database=None):
    """
    Runs smah.smah.main() with heavy collaborators mocked out.

    Returns (exit_code_or_None, recorded_stderr_text).
    """
    console = Console(record=True, width=250, stderr=True)
    settings_instance = mock.MagicMock()
    settings_instance.is_configured.return_value = True
    runner = runner if runner is not None else mock.MagicMock()

    with mock.patch("smah.logs.configure"), \
            mock.patch("smah.args.extract_args", return_value=(args, pipe)), \
            mock.patch("smah.smah.init_database"), \
            mock.patch("smah.smah.Settings", return_value=settings_instance), \
            mock.patch("smah.smah.Runner", return_value=runner), \
            mock.patch("smah.smah.configurator", return_value=settings_instance), \
            mock.patch("smah.smah.err_console", console), \
            (mock.patch("smah.smah.Database", return_value=database) if database is not None else mock.patch("smah.smah.Database", return_value=mock.MagicMock())):
        code = None
        try:
            smah_main.main()
        except SystemExit as e:
            code = e.code
    return code, console.export_text()


# --- subcommand dispatch ----------------------------------------------------

def test_ask_dispatches_to_runner_query():
    runner = mock.MagicMock()
    code, _ = run(base_args(command="ask", ask_query="hello"), runner=runner)
    assert code is None
    runner.query.assert_called_once_with(query="hello")


def test_ask_with_pipe_dispatches_to_runner_pipe():
    runner = mock.MagicMock()
    code, _ = run(base_args(command="ask", ask_query="summarize"), pipe="piped body", runner=runner)
    assert code is None
    runner.pipe.assert_called_once_with(query="summarize", pipe="piped body")


def test_ask_without_query_or_stdin_is_usage_error():
    code, text = run(base_args(command="ask"))
    assert code == int(ExitCode.USAGE)
    assert "'ask' needs a query argument or piped stdin" in text


def test_ask_with_piped_stdin_but_no_query_is_usage_error():
    code, text = run(base_args(command="ask"), pipe="piped content")
    assert code == int(ExitCode.USAGE)
    assert "no query was given" in text


def test_ask_instructions_file_becomes_query(tmp_path):
    instructions = tmp_path / "scan.md"
    instructions.write_text("process the output of systemctl")
    runner = mock.MagicMock()
    code, _ = run(base_args(command="ask", ask_instructions=str(instructions)), runner=runner)
    assert code is None
    runner.query.assert_called_once_with(query="process the output of systemctl")


def test_chat_is_honestly_not_implemented():
    code, text = run(base_args(command="chat"))
    assert code == int(ExitCode.USAGE)
    assert "chat is not available yet — use 'smah ask' for one-shot queries" in text


def test_history_dispatches_to_lister():
    with mock.patch("smah.commands.history_cmd.list_sessions") as list_sessions:
        code, _ = run(base_args(command="history", history_limit=35))
        assert code is None
        list_sessions.assert_called_once()


def test_resume_with_id_resumes_that_session():
    session = {"id": 9, "title": "t", "args": {}, "plan": {}, "pipe": None, "messages": [{"role": "user", "content": "hi"}]}
    db = mock.MagicMock()
    db.session.return_value = session
    runner = mock.MagicMock()
    code, _ = run(base_args(command="resume", resume_id=9), runner=runner, database=db)
    assert code is None
    db.session.assert_called_once_with(9)
    runner.resume.assert_called_once_with(id=9, title="t", plan={}, pipe=None, messages=session["messages"])


def test_resume_last_resumes_most_recent():
    session = {"id": 3, "title": "t", "args": {}, "plan": {}, "pipe": None, "messages": []}
    db = mock.MagicMock()
    db.last_session.return_value = session
    runner = mock.MagicMock()
    code, _ = run(base_args(command="resume", resume_last=True), runner=runner, database=db)
    assert code is None
    db.last_session.assert_called_once()
    runner.resume.assert_called_once()


def test_resume_unknown_id_is_usage_error_listing_recent():
    db = mock.MagicMock()
    db.session.return_value = None
    db.history.return_value = [{"id": 4}, {"id": 3}]
    code, text = run(base_args(command="resume", resume_id=99), database=db)
    assert code == int(ExitCode.USAGE)
    assert "no session found (id 99)" in text
    assert "4, 3" in text


def test_config_show_prints_masked_config_to_stdout(tmp_path, capsys):
    config_file = tmp_path / "config.yaml"
    config_file.write_text(yaml.safe_dump({
        "vsn": "0.0.1",
        "inference": {"providers": {"openai": {"settings": {"api_key": "sk-super-secret"}}}},
    }))
    code, _ = run(base_args(command="config", config_command="show", config=str(config_file)))
    assert code is None
    out = capsys.readouterr().out
    assert "sk-super-secret" not in out
    assert "••••••••" in out
    assert "api_key" in out  # the key name survives, the value does not


def test_config_path_prints_resolved_path(tmp_path, capsys):
    config_file = tmp_path / "config.yaml"
    code, _ = run(base_args(command="config", config_command="path", config=str(config_file)))
    assert code is None
    assert capsys.readouterr().out.strip() == str(config_file)


def test_config_set_writes_value_and_hides_secrets(tmp_path, capsys):
    config_file = tmp_path / "config.yaml"
    config_file.write_text(yaml.safe_dump({"vsn": "0.0.1", "user": {"name": "Old"}}))
    code, _ = run(base_args(
        command="config", config_command="set", config=str(config_file),
        config_set_key="inference.providers.openai.settings.api_key", config_set_value="sk-new-secret",
    ))
    assert code is None
    data = yaml.safe_load(config_file.read_text())
    assert data["inference"]["providers"]["openai"]["settings"]["api_key"] == "sk-new-secret"
    captured = capsys.readouterr()
    assert "sk-new-secret" not in captured.out + captured.err


def test_config_bare_runs_wizard():
    with mock.patch("sys.stdin", FakeTTY("")):
        code, _ = run(base_args(command="config"))
    assert code is None


def test_models_dispatches_to_lister():
    with mock.patch("smah.commands.models_cmd.list_models") as list_models:
        code, _ = run(base_args(command="models", models_command="list"))
        assert code is None
        list_models.assert_called_once()


def test_doctor_dispatches_and_propagates_exit_code():
    with mock.patch("smah.commands.doctor.run_doctor", return_value=int(ExitCode.AUTH_OR_MODEL)) as run_doctor:
        code, _ = run(base_args(command="doctor"))
        assert code == int(ExitCode.AUTH_OR_MODEL)
        run_doctor.assert_called_once()


# --- legacy flags: still work, deprecation on stderr ------------------------

def test_query_flag_dispatches_to_runner_and_does_not_exit():
    runner = mock.MagicMock()
    code, _ = run(base_args(query="hello", deprecated=["-q"]), runner=runner)
    assert code is None
    runner.query.assert_called_once_with(query="hello")


def test_instructions_flag_reads_file_and_dispatches(tmp_path):
    instructions = tmp_path / "scan.md"
    instructions.write_text("scan the system")
    runner = mock.MagicMock()
    code, _ = run(base_args(instructions=str(instructions), deprecated=["-i"]), runner=runner)
    assert code is None
    runner.query.assert_called_once_with(query="scan the system")


@pytest.mark.parametrize(
    "flag,notice",
    [
        ("-q", "[deprecation] 'smah -q' works, but prefer 'smah ask'. Alias removes in 0.4."),
        ("-i", "[deprecation] 'smah -i' works, but prefer 'smah ask --instructions'. Alias removes in 0.4."),
        ("--interactive", "[deprecation] --interactive never worked in prior releases. Use 'smah chat'."),
        ("--continue", "[deprecation] '--continue' works, but prefer 'smah resume'. Alias removes in 0.4."),
        ("--session", "[deprecation] '--session N' works, but prefer 'smah resume N'. Alias removes in 0.4."),
        ("--history", "[deprecation] '--history' works, but prefer 'smah history'. Alias removes in 0.4."),
        ("--configure", "[deprecation] '--configure' works, but prefer 'smah config'. Alias removes in 0.4."),
        ("--openai-api-key", "[deprecation] '--openai-api-key' works, but prefer '--api-key' (applies to the active provider). Removes in 0.4."),
    ],
)
def test_every_legacy_flag_prints_its_spec_deprecation_notice(flag, notice):
    overrides = {
        "-q": {"query": "x"},
        "-i": {"query": "x"},
        "--interactive": {"interactive": True},
        "--continue": {"resume": True},
        "--session": {"session": 5},
        "--history": {"history": True},
        "--configure": {"configure": True},
        "--openai-api-key": {"query": "x", "openai_api_key": "sk-1"},
    }[flag]
    database = None
    if flag in ("--continue", "--session", "--history"):
        session = {"id": 5, "title": "t", "args": {}, "plan": {}, "pipe": None, "messages": []}
        database = mock.MagicMock()
        database.last_session.return_value = session
        database.session.return_value = session
    if flag == "--history":
        with mock.patch("smah.smah.pick_session", return_value=5):
            code, text = run(base_args(deprecated=[flag], **overrides), database=database)
    else:
        code, text = run(base_args(deprecated=[flag], **overrides), database=database)
    assert notice in text
    if flag == "--interactive":
        assert code == int(ExitCode.USAGE)


def test_continue_flag_resumes_most_recent_session():
    session = {"id": 5, "title": "t", "args": {}, "plan": {}, "pipe": None, "messages": []}
    db = mock.MagicMock()
    db.last_session.return_value = session
    runner = mock.MagicMock()
    code, _ = run(base_args(resume=True, deprecated=["--continue"]), runner=runner, database=db)
    assert code is None
    db.last_session.assert_called_once()
    runner.resume.assert_called_once()


def test_session_flag_resumes_specific_session():
    session = {"id": 5, "title": "t", "args": {}, "plan": {}, "pipe": None, "messages": []}
    db = mock.MagicMock()
    db.session.return_value = session
    runner = mock.MagicMock()
    code, _ = run(base_args(session=5, deprecated=["--session"]), runner=runner, database=db)
    assert code is None
    db.session.assert_called_once_with(5)
    runner.resume.assert_called_once()


def test_history_flag_keeps_pick_and_resume_behavior():
    session = {"id": 7, "title": "t", "args": {}, "plan": {}, "pipe": None, "messages": []}
    db = mock.MagicMock()
    db.session.return_value = session
    runner = mock.MagicMock()
    with mock.patch("smah.smah.pick_session", return_value=7):
        code, _ = run(base_args(history=True, deprecated=["--history"]), runner=runner, database=db)
    assert code is None
    db.session.assert_called_once_with(7)
    runner.resume.assert_called_once()


def test_configure_flag_runs_wizard_and_exits_cleanly():
    with mock.patch("sys.stdin", FakeTTY("")):
        code, _ = run(base_args(configure=True, deprecated=["--configure"]))
    assert code is None


def test_configure_flag_without_a_terminal_exits_config():
    # a wizard nobody can answer must exit 3, never EOF-crash
    code, text = run(base_args(configure=True, deprecated=["--configure"]))
    assert code == int(ExitCode.CONFIG)
    assert "needs an interactive terminal" in text


def test_config_wizard_without_a_terminal_exits_config():
    code, text = run(base_args(command="config"))
    assert code == int(ExitCode.CONFIG)
    assert "needs an interactive terminal" in text


def test_interactive_flag_maps_to_chat_notice():
    code, text = run(base_args(interactive=True, deprecated=["--interactive"]))
    assert code == int(ExitCode.USAGE)
    assert "chat is not available yet" in text


# --- bare invocation and pipe safety ----------------------------------------

def test_bare_smah_exits_usage_pointing_at_subcommands():
    code, text = run(base_args(), pipe=None)
    assert code == int(ExitCode.USAGE)
    assert "smah --help" in text
    assert "smah ask" in text
    assert "chat is not available yet" in text


def test_piped_input_without_query_exits_usage():
    code, text = run(base_args(), pipe="some piped content")
    assert code == int(ExitCode.USAGE)
    assert "no query was given" in text


def test_deprecation_notices_never_land_on_stdout(capsys):
    capsys.readouterr()
    runner = mock.MagicMock()
    code, _ = run(base_args(query="hello", deprecated=["-q", "--continue"]), pipe="data", runner=runner, database=mock.MagicMock())
    assert code is None
    captured = capsys.readouterr()
    assert captured.out == ""
    runner.pipe.assert_called_once()


# --- fresh-install behavior: no config, non-interactive stdin ----------------

def run_real_settings(args, pipe=None, capsys_drain=False):
    """
    Runs main() with the runner/db/logs mocked but Settings REAL, so the
    config-file handling is exercised for real. Returns (exit_code, stderr_text).
    """
    console = Console(record=True, width=250, stderr=True)
    runner = mock.MagicMock()
    with mock.patch("smah.logs.configure"), \
            mock.patch("smah.args.extract_args", return_value=(args, pipe)), \
            mock.patch("smah.smah.init_database"), \
            mock.patch("smah.smah.Runner", return_value=runner), \
            mock.patch("smah.smah.err_console", console):
        code = None
        try:
            smah_main.main()
        except SystemExit as e:
            code = e.code
    return code, console.export_text()


def test_ask_with_missing_config_exits_three_not_internal_error(tmp_path, capsys):
    capsys.readouterr()
    missing = tmp_path / "config.yaml"
    code, text = run_real_settings(
        base_args(command="ask", ask_query="test", config=str(missing)), pipe="hi",
    )
    assert code == int(ExitCode.CONFIG)
    assert "no config found at" in text and str(missing) in text
    assert "Run 'smah config'" in text
    assert "unexpected error" not in text
    assert "EOF" not in text
    # stdout is byte-empty when a command fails before producing real output
    assert capsys.readouterr().out == ""


def test_ask_with_incomplete_config_non_interactive_exits_three(tmp_path, capsys):
    capsys.readouterr()
    config = tmp_path / "config.yaml"
    config.write_text(yaml.safe_dump({"vsn": "0.0.1"}))  # exists, but empty/incomplete
    code, text = run_real_settings(
        base_args(command="ask", ask_query="test", config=str(config)), pipe="hi",
    )
    assert code == int(ExitCode.CONFIG)
    assert "incomplete" in text
    assert "needs an interactive terminal" in text
    assert capsys.readouterr().out == ""


def test_wizard_ui_never_lands_on_stdout(tmp_path, capsys):
    capsys.readouterr()
    config = tmp_path / "config.yaml"
    config.write_text(yaml.safe_dump({"vsn": "0.0.1"}))

    from smah.console import std_console

    def fake_configurator(settings):
        print("WIZARD-PLAIN-PRINT")                      # plain stdout write
        std_console.print("WIZARD-RICH-PANEL")           # rich stdout console
        return settings

    with mock.patch("sys.stdin", FakeTTY("")), \
            mock.patch("smah.smah.configurator", side_effect=fake_configurator):
        code, _ = run_real_settings(base_args(command="config", config=str(config)))
    assert code is None
    captured = capsys.readouterr()
    assert captured.out == ""                              # stdout stays machine-clean
    assert "WIZARD-PLAIN-PRINT" in captured.err
    assert "WIZARD-RICH-PANEL" in captured.err


# --- exit-code mapping --------------------------------------------------------

def test_unexpected_error_exits_one_and_names_failure():
    def boom(*a, **k):
        raise ValueError("kaboom")

    runner = mock.MagicMock()
    runner.query.side_effect = boom

    code, text = run(base_args(query="hello"), runner=runner)
    assert code == int(ExitCode.UNEXPECTED_ERROR)
    assert "kaboom" in text
    # full traceback only when verbosity is raised
    assert "Traceback" not in text

    code, text = run(base_args(query="hello", verbose=1), runner=runner)
    assert code == int(ExitCode.UNEXPECTED_ERROR)
    assert "Traceback" in text


def test_openai_error_exits_provider():
    def boom(*a, **k):
        raise OpenAIError("api exploded")

    runner = mock.MagicMock()
    runner.query.side_effect = boom

    code, text = run(base_args(query="hello"), runner=runner)
    assert code == int(ExitCode.PROVIDER)
    assert "api exploded" in text


# --- defect regression: doctor must not block on an open stdin pipe -----------

def test_doctor_completes_with_stdin_held_open(tmp_path):
    """
    Reproduces the CI/daemon case: stdin is an open pipe nobody writes to.
    doctor must complete without reading it.
    """
    import os
    import subprocess
    import sys as _sys

    repo_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    env = dict(os.environ, PYTHONPATH=repo_root)
    proc = subprocess.Popen(
        [
            _sys.executable, "-m", "smah.smah",
            "-c", str(tmp_path / "missing-config.yaml"),
            "--database", str(tmp_path / "smah.db"),
            "doctor",
        ],
        stdin=subprocess.PIPE,  # held open for the child's lifetime
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=repo_root,
        env=env,
    )
    try:
        returncode = proc.wait(timeout=30)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
        pytest.fail("smah doctor hung on an open stdin pipe (regression of defect 3)")
    stdout, stderr = proc.communicate(timeout=10)
    assert returncode == int(ExitCode.CONFIG)  # missing config, not a hang
    assert stdout == b""                        # human report belongs to stderr
    assert b"FAIL" in stderr
