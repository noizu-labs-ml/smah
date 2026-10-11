import pytest
from unittest import mock

from smah.args import extract_args
from smah.args.args import build_parser


def parse(argv):
    with mock.patch("smah.args.args.__get_pipe", return_value=None):
        args, pipe = extract_args(argv)
    assert pipe is None
    return args


def test_supported_flags_still_parse():
    args = parse(
        [
            "-q", "what is up",
            "-i", "/tmp/instructions.md",
            "-c", "/tmp/config.yaml",
            "--database", "/tmp/smah.db",
            "--model", "openai.gpt-4o-mini",
            "--model-query", "openai.gpt-4o-mini",
            "--model-pipe", "openai.gpt-4o",
            "--openai-api-key", "sk-test",
            "-vv",
        ]
    )
    assert args.query == "what is up"
    assert args.instructions == "/tmp/instructions.md"
    assert args.config == "/tmp/config.yaml"
    assert args.database == "/tmp/smah.db"
    assert args.model == "openai.gpt-4o-mini"
    assert args.model_query == "openai.gpt-4o-mini"
    assert args.model_pipe == "openai.gpt-4o"
    assert args.openai_api_key == "sk-test"
    assert args.verbose == 2


def test_boolean_flags_defaults():
    args = parse([])
    assert args.rich is True
    assert args.resume is False
    assert args.history is False
    assert args.configure is False
    assert args.interactive is False
    assert args.verbose == 0


def test_boolean_flags_explicit():
    args = parse(["--no-rich", "--continue", "--history", "--interactive"])
    assert args.rich is False
    assert args.resume is True
    assert args.history is True
    assert args.interactive is True


def test_resume_flag_maps_to_resume_dest():
    args = parse(["--continue"])
    assert args.resume is True


@pytest.mark.parametrize(
    "removed_flag",
    [
        "--gui",
        "--no-gui",
        "--model-picker",
        "--model-interactive",
        "--model-review",
        "--model-edit",
        "--openai-api-tier",
        "--openai-api-org",
    ],
)
def test_dead_flags_are_rejected(removed_flag):
    with pytest.raises(SystemExit) as exc:
        parse(["-q", "hello", removed_flag, "value"])
    assert exc.value.code == 2


def test_help_lists_groups_examples_and_no_dead_flags():
    help_text = build_parser().format_help()
    for group in ["input:", "sessions:", "models:", "output:", "configuration:", "diagnostics:"]:
        assert group in help_text, f"missing argument group {group}"
    assert "examples:" in help_text
    for dead in ["--gui", "--model-picker", "--model-review", "--model-edit", "--openai-api-tier", "--openai-api-org"]:
        assert dead not in help_text, f"{dead} should not be documented"
    # flags that do exist must still be documented
    for live in ["--query", "--instructions", "--continue", "--session", "--history", "--model", "--rich", "--verbose", "--config"]:
        assert live in help_text
    # interactive is kept but must be marked not implemented
    assert "NOT IMPLEMENTED" in help_text
