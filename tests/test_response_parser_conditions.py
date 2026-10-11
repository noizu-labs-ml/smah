"""
exec-if condition safety tests: extract_commands must never execute
model-supplied code, must not crash on empty/unknown conditions, and must
skip commands whose condition is unevaluable.
"""
import pytest
from rich.console import Console

from smah.runner.response_parser import ConditionError, ResponseParser


def exec_command(response: str, options=None):
    # Skip notices go to the error console (stderr); pytest captures them.
    return ResponseParser.extract_commands(response, options or {})


COMMAND = """
<exec shell="zsh">
    <title>Run Hello</title>
    <purpose>Show hello</purpose>
    <command>echo 'Hello World'</command>
</exec>
"""


def tagged_command(condition: str) -> str:
    return COMMAND.replace("<exec shell=\"zsh\">", f"<exec shell=\"zsh\" exec-if=\"{condition}\">")


def test_command_without_exec_if_is_included():
    commands = exec_command(COMMAND)
    assert commands is not None and len(commands) == 1
    assert commands[0]["command"] == "echo 'Hello World'"


def test_condition_with_empty_conditions_no_longer_crashes():
    # Regression: the old eval(c, {}) path raised NameError here.
    commands = exec_command(tagged_command("verbose == true"), {"conditions": {}})
    assert commands == []


def test_condition_true_includes_command():
    commands = exec_command(
        tagged_command("verbose == true"),
        {"conditions": {"verbose": True}},
    )
    assert len(commands) == 1


def test_condition_false_skips_command():
    commands = exec_command(
        tagged_command("verbose == true"),
        {"conditions": {"verbose": False}},
    )
    assert commands == []


def test_condition_and_or_not_logic():
    response = tagged_command("verbose and not quiet")
    assert len(exec_command(response, {"conditions": {"verbose": True, "quiet": False}})) == 1
    assert exec_command(response, {"conditions": {"verbose": True, "quiet": True}}) == []


def test_condition_comparison_operators():
    response = tagged_command("count >= 3 and name == 'smah'")
    conditions = {"conditions": {"count": 5, "name": "smah"}}
    assert len(exec_command(response, conditions)) == 1
    conditions = {"conditions": {"count": 2, "name": "smah"}}
    assert exec_command(response, conditions) == []


def test_model_supplied_code_is_never_executed():
    # A model that tries to smuggle code into exec-if gets a skip, not execution.
    from unittest import mock

    console = Console(record=True, width=200)
    with mock.patch("smah.runner.response_parser.err_console", console):
        commands = ResponseParser.extract_commands(
            tagged_command("__import__('os').system('echo pwned')"), {"conditions": {}}
        )
    assert commands == []
    assert "unevaluable" in console.export_text()


def test_attribute_access_and_calls_are_unevaluable():
    for condition in [
        "os.environ",
        "open('/etc/passwd').read()",
        "[x for x in range(10)]",
        "f'{1+1}'",
    ]:
        with pytest.raises(ConditionError):
            ResponseParser.eval_condition(condition, {})


def test_eval_condition_type_mismatch_is_unevaluable():
    with pytest.raises(ConditionError):
        ResponseParser.eval_condition("count < 'many'", {"count": 3})


def test_source_contains_no_eval_or_exec_calls():
    # Parse the module and assert no call to the builtins eval/exec exists
    # (docstring prose may mention them by name; only calls are dangerous).
    import ast
    import pathlib

    import smah
    source = pathlib.Path(smah.__file__).parent / "runner" / "response_parser.py"
    tree = ast.parse(source.read_text(encoding="utf-8"))
    calls = [
        node.func.id
        for node in ast.walk(tree)
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id in ("eval", "exec")
    ]
    assert calls == []


def test_extract_json_passthrough_fenced_and_prose():
    assert ResponseParser.extract_json('{"a": 1}') == '{"a": 1}'
    assert ResponseParser.extract_json('```json\n{"a": 1}\n```') == '{"a": 1}'
    assert ResponseParser.extract_json('Sure! {"a": 1, "b": [2]} thanks') == '{"a": 1, "b": [2]}'
    assert ResponseParser.extract_json("no json here") == "no json here"
