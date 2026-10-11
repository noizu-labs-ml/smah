import sys
from unittest import mock

import pytest

from smah.args import extract_args, merge_args
from smah.args.args import DEPRECATION_NOTICES, REMOVED_FLAGS, build_parser


def parse(argv):
    with mock.patch("smah.args.args.__get_pipe", return_value=None):
        args, pipe = extract_args(argv)
    assert pipe is None
    return args


def parse_tty(argv):
    """Parses with stdout pretending to be a TTY (rich defaults on)."""
    with mock.patch("smah.args.args.__get_pipe", return_value=None), \
            mock.patch.object(sys.stdout, "isatty", return_value=True):
        args, _ = extract_args(argv)
    return args


def parse_pipe(argv, pipe_text):
    with mock.patch("smah.args.args.__get_pipe", return_value=pipe_text):
        args, pipe = extract_args(argv)
    assert pipe == pipe_text
    return args


# --- global flags -----------------------------------------------------------

def test_supported_global_flags_still_parse():
    args = parse(
        [
            "-c", "/tmp/config.yaml",
            "--database", "/tmp/smah.db",
            "--model", "openai.gpt-4o-mini",
            "--model-query", "openai.gpt-4o-mini",
            "--model-pipe", "openai.gpt-4o",
            "--api-key", "sk-test",
            "-vv",
            "ask", "what is up",
        ]
    )
    assert args.command == "ask"
    assert args.ask_query == "what is up"
    assert args.config == "/tmp/config.yaml"
    assert args.database == "/tmp/smah.db"
    assert args.model == "openai.gpt-4o-mini"
    assert args.model_query == "openai.gpt-4o-mini"
    assert args.model_pipe == "openai.gpt-4o"
    assert args.api_key == "sk-test"
    assert args.verbose == 2
    # --api-key folds into the field the provider layer reads
    assert args.openai_api_key == "sk-test"
    # the ask positional must not be mistaken for the deprecated -q
    assert args.deprecated == []


def test_boolean_flag_defaults():
    args = parse([])
    assert args.command is None
    assert args.resume is False
    assert args.history is False
    assert args.configure is False
    assert args.interactive is False
    assert args.verbose == 0
    assert args.deprecated == []


def test_rich_follows_tty_detection():
    # under pytest stdout is not a TTY -> plain
    args = parse(["ask", "x"])
    assert args.rich is False
    # on a TTY -> rich
    assert parse_tty(["ask", "x"]).rich is True
    # explicit flags override the TTY either way
    assert parse(["--rich", "ask", "x"]).rich is True
    assert parse_tty(["--no-rich", "ask", "x"]).rich is False


def test_explicit_boolean_flags():
    args = parse(["--continue", "--history", "--interactive", "--configure"])
    assert args.resume is True
    assert args.history is True
    assert args.interactive is True
    assert args.configure is True


def test_resume_flag_maps_to_resume_dest():
    args = parse(["--continue"])
    assert args.resume is True


# --- subcommand surface -----------------------------------------------------

def test_ask_subcommand_parses_positional_and_instructions():
    args = parse(["ask", "what is up", "-i", "/tmp/instructions.md"])
    assert args.command == "ask"
    assert args.ask_query == "what is up"
    assert args.ask_instructions == "/tmp/instructions.md"
    assert args.deprecated == []


def test_chat_subcommand_parses():
    assert parse(["chat"]).command == "chat"


def test_history_subcommand_parses_with_limit():
    args = parse(["history"])
    assert args.command == "history"
    assert args.history_limit == 20
    args = parse(["history", "--limit", "50"])
    assert args.history_limit == 50


def test_resume_subcommand_forms():
    args = parse(["resume", "42"])
    assert args.command == "resume"
    assert args.resume_id == 42
    assert args.resume_last is False
    args = parse(["resume", "--last"])
    assert args.resume_id is None
    assert args.resume_last is True
    args = parse(["resume"])
    assert args.resume_id is None
    assert args.resume_last is False


def test_resume_rejects_id_and_last_together():
    with pytest.raises(SystemExit) as exc:
        parse(["resume", "--last", "7"])
    assert exc.value.code == 2


def test_config_subcommands_parse():
    assert parse(["config"]).config_command is None
    assert parse(["config", "show"]).config_command == "show"
    assert parse(["config", "path"]).config_command == "path"
    assert parse(["config", "edit"]).config_command == "edit"
    args = parse(["config", "set", "user.name", "Ines"])
    assert args.config_command == "set"
    assert args.config_set_key == "user.name"
    assert args.config_set_value == "Ines"


def test_models_subcommands_parse():
    assert parse(["models"]).command == "models"
    assert parse(["models"]).models_command is None
    assert parse(["models", "list"]).models_command == "list"


def test_doctor_subcommand_parses_flags():
    args = parse(["doctor"])
    assert args.command == "doctor"
    assert args.doctor_json is False
    assert args.doctor_offline is False
    args = parse(["doctor", "--json", "--offline"])
    assert args.doctor_json is True
    assert args.doctor_offline is True


def test_piped_stdin_is_captured_with_and_without_subcommand():
    args = parse_pipe(["-q", "summarize"], "piped body")
    assert args.query == "summarize"
    args = parse_pipe(["ask", "summarize"], "piped body")
    assert args.command == "ask"
    assert args.ask_query == "summarize"


# --- back-compat: legacy flags still parse and are recorded ------------------

def test_legacy_flags_still_parse():
    args = parse(
        [
            "-q", "what is up",
            "-i", "/tmp/instructions.md",
            "--openai-api-key", "sk-test",
        ]
    )
    assert args.query == "what is up"
    assert args.instructions == "/tmp/instructions.md"
    assert args.openai_api_key == "sk-test"


@pytest.mark.parametrize(
    "argv,flag",
    [
        (["-q", "x"], "-q"),
        (["--query", "x"], "-q"),
        (["-i", "/tmp/f.md"], "-i"),
        (["--instructions", "/tmp/f.md"], "-i"),
        (["--continue"], "--continue"),
        (["--session", "12"], "--session"),
        (["--history"], "--history"),
        (["--interactive"], "--interactive"),
        (["--configure"], "--configure"),
        (["--openai-api-key", "sk-1"], "--openai-api-key"),
    ],
)
def test_legacy_flags_are_recorded_as_deprecated(argv, flag):
    args = parse(argv)
    assert flag in args.deprecated


def test_deprecated_flags_combine_in_order():
    args = parse(["-q", "x", "--continue", "--history"])
    assert args.deprecated == ["-q", "--continue", "--history"]


def test_negative_boolean_forms_are_not_deprecated():
    args = parse(["--no-continue", "--no-history", "--no-interactive", "--no-configure"])
    assert args.deprecated == []
    assert args.resume is False
    assert args.history is False


def test_new_api_key_is_not_deprecated():
    args = parse(["--api-key", "sk-2"])
    assert args.deprecated == []
    assert args.openai_api_key == "sk-2"


def test_every_deprecation_has_spec_message():
    for flag, message in DEPRECATION_NOTICES.items():
        assert message.startswith("[deprecation] "), flag
        assert "0.4" in message or flag == "--interactive", flag


# --- removed flags: exact spec messages, exit 2 ------------------------------

@pytest.mark.parametrize("flag,message", REMOVED_FLAGS)
def test_removed_flags_rejected_with_spec_message(flag, message, capsys):
    with pytest.raises(SystemExit) as exc:
        parse([flag])
    assert exc.value.code == 2
    stderr = capsys.readouterr().err
    assert "smah: error: " + message in stderr


# --- help -------------------------------------------------------------------

def test_help_documents_subcommands_legacy_and_examples():
    help_text = build_parser().format_help()
    # subcommands are the primary surface
    for command in ("ask", "chat", "resume", "history", "config", "models", "doctor"):
        assert command in help_text, f"missing command {command}"
    # grouped flags
    for group in ("global flags:", "commands:", "legacy flags (deprecated"):
        assert group in help_text, f"missing group {group}"
    # worked examples, exit codes, and the print-only safety statement
    assert "examples:" in help_text
    assert "auth.log" in help_text
    assert "SCAN-SYSTEMCTL" in help_text
    assert "smah resume --last" in help_text
    assert "exit codes:" in help_text
    assert "SMAH does not run commands" in help_text
    # legacy flags still documented
    for legacy in ("--query", "--instructions", "--continue", "--session", "--history", "--interactive", "--configure", "--openai-api-key"):
        assert legacy in help_text, f"missing legacy flag {legacy}"
    # honesty markers
    assert "NOT IMPLEMENTED" in help_text
    # flags that never existed are not documented
    for dead in ("--setup", "--gui", "--model-picker", "--model-review", "--model-edit", "--openai-api-tier", "--openai-api-org"):
        assert dead not in help_text, f"{dead} should not be documented"


def test_subcommands_have_examples_in_help(capsys):
    for argv in (["ask", "--help"], ["resume", "--help"], ["history", "--help"], ["config", "--help"], ["models", "--help"], ["doctor", "--help"]):
        with pytest.raises(SystemExit) as exc:
            parse(argv)
        assert exc.value.code == 0  # --help exits 0
    # the ask help carries real worked examples
    capsys.readouterr()
    with pytest.raises(SystemExit):
        parse(["ask", "--help"])
    ask_help = capsys.readouterr().out
    assert "examples:" in ask_help
    assert "auth.log" in ask_help
    assert "--instructions" in ask_help


# --- merge_args -------------------------------------------------------------

def test_merge_args_fills_unset_and_skips_reserved_keys():
    args = parse(["-q", "cli-query"])
    args = merge_args(args, {"query": "stored-query", "model": "openai.gpt-4o", "deprecated": ["-q"], "command": "ask"})
    assert args.query == "cli-query"  # CLI wins
    assert args.model == "openai.gpt-4o"  # stored fills unset
    assert args.deprecated == ["-q"]  # reserved keys never overwritten
    assert args.command is None
