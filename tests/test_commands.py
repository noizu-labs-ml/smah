"""
Tests for the smah.commands modules: config, models, history, doctor.

All interaction with external boundaries (files under tmp_path, sqlite,
environment) is local — no network, no provider clients.
"""

import json
import os
import sqlite3
from types import SimpleNamespace
from unittest import mock

import pytest
import yaml

from smah.commands import config_cmd, history_cmd, models_cmd
from smah.commands import doctor as doctor_cmd
from smah.database import Database, Migration
from smah.exit_codes import ExitCode
from smah.settings.inference.inference import Inference


def write_config(path, api_key="sk-test", enable_mini=True, picker=None):
    config = {
        "vsn": "0.0.1",
        "inference": {
            "vsn": "0.0.1",
            "instructions": "",
            "model_picker": picker if picker is not None else {"default": ["openai.gpt-4o-mini"]},
            "providers": {
                "openai": {
                    "vsn": "0.0.1",
                    "name": "OpenAI",
                    "description": "OpenAI provider",
                    "enabled": True,
                    "settings": {"api_key": api_key} if api_key is not None else {},
                    "models": [
                        {
                            "vsn": "0.0.1",
                            "name": "gpt-4o-mini",
                            "model": "gpt-4o-mini",
                            "description": "fast",
                            "enabled": enable_mini,
                        },
                        {
                            "vsn": "0.0.1",
                            "name": "gpt-4o",
                            "model": "gpt-4o",
                            "description": "big",
                            "enabled": False,
                        },
                    ],
                }
            },
        },
    }
    path.write_text(yaml.safe_dump(config))
    return path


def make_args(tmp_path, config=None, database=None, **extra):
    fields = dict(
        config=str(config) if config else None,
        database=str(database) if database else None,
        model=None,
        model_query=None,
        model_pipe=None,
        openai_api_key=None,
        doctor_json=False,
        doctor_offline=False,
        history_limit=20,
    )
    fields.update(extra)
    return SimpleNamespace(**fields)


def init_db(path):
    db = Database(SimpleNamespace(database=str(path)))
    Migration.migrate(db, SimpleNamespace(silent=True, count=None, to=None), silent=True, exit_on_finish=False)
    return db


# --- config_cmd --------------------------------------------------------------

def test_mask_secret_hides_values_but_names_env_indirection():
    assert config_cmd.mask_secret("sk-abc123") == "••••••••"
    assert config_cmd.mask_secret("$OPENAI_API_KEY") == "<env:OPENAI_API_KEY>"
    assert config_cmd.mask_secret("${MY_KEY}") == "<env:MY_KEY>"


def test_redact_masks_nested_secret_keys_only():
    data = {
        "user": {"name": "Ines", "password": "hunter2"},
        "inference": {"providers": {"openai": {"settings": {"api_key": "$OPENAI_API_KEY"}}}},
        "misc": [1, 2],
    }
    out = config_cmd.redact(data)
    assert out["user"]["name"] == "Ines"
    assert out["user"]["password"] == "••••••••"
    assert out["inference"]["providers"]["openai"]["settings"]["api_key"] == "<env:OPENAI_API_KEY>"
    assert out["misc"] == [1, 2]
    # input untouched
    assert data["user"]["password"] == "hunter2"


def test_show_masks_secrets_and_keeps_names(tmp_path, capsys):
    path = write_config(tmp_path / "config.yaml", api_key="sk-super-secret")
    config_cmd.show(make_args(tmp_path, config=path))
    out = capsys.readouterr().out
    assert "sk-super-secret" not in out
    assert "••••••••" in out
    assert "api_key" in out  # the key name survives, the value never prints


def test_show_missing_config_is_usage_error(tmp_path):
    with pytest.raises(SystemExit) as exc:
        config_cmd.show(make_args(tmp_path, config=tmp_path / "nope.yaml"))
    assert exc.value.code == int(ExitCode.USAGE)


def test_show_corrupt_config_is_config_error(tmp_path):
    path = tmp_path / "config.yaml"
    path.write_text("vsn: [unclosed")
    with pytest.raises(SystemExit) as exc:
        config_cmd.show(make_args(tmp_path, config=path))
    assert exc.value.code == int(ExitCode.CONFIG)


def test_set_coerces_bools_and_ints(tmp_path, capsys):
    path = write_config(tmp_path / "config.yaml")
    args = make_args(tmp_path, config=path, config_set_key=None, config_set_value=None)
    config_cmd.set_value(args, "inference.providers.openai.enabled", "false")
    config_cmd.set_value(args, "user.age", "42")
    config_cmd.set_value(args, "user.name", "Plain Text")
    data = yaml.safe_load(path.read_text())
    assert data["inference"]["providers"]["openai"]["enabled"] is False
    assert data["user"]["age"] == 42
    assert data["user"]["name"] == "Plain Text"


def test_set_missing_config_is_usage_error(tmp_path):
    with pytest.raises(SystemExit) as exc:
        config_cmd.set_value(
            make_args(tmp_path, config=tmp_path / "nope.yaml"),
            "user.name", "x",
        )
    assert exc.value.code == int(ExitCode.USAGE)


def test_set_secret_value_is_never_echoed(tmp_path, capsys):
    path = write_config(tmp_path / "config.yaml")
    config_cmd.set_value(
        make_args(tmp_path, config=path),
        "inference.providers.openai.settings.api_key", "sk-brand-new",
    )
    captured = capsys.readouterr()
    assert "sk-brand-new" not in captured.out + captured.err
    assert "api_key" in captured.err


def test_set_refuses_to_punch_through_a_scalar(tmp_path):
    path = write_config(tmp_path / "config.yaml")
    with pytest.raises(SystemExit) as exc:
        config_cmd.set_value(
            make_args(tmp_path, config=path),
            "vsn.something", "x",
        )
    assert exc.value.code == int(ExitCode.CONFIG)


# --- models_cmd --------------------------------------------------------------

def inference_from(inference_data):
    return Inference(inference_data)


def test_resolve_default_prefers_registered_override():
    inference = inference_from(
        {
            "model_picker": {"default": ["openai.gpt-4o-mini"], "pipe": ["openai.gpt-4o"]},
            "providers": {
                "openai": {
                    "name": "OpenAI",
                    "enabled": True, "settings": {"api_key": "sk"},
                    "models": [
                        {"name": "gpt-4o-mini", "model": "gpt-4o-mini", "description": "", "enabled": True},
                        {"name": "gpt-4o", "model": "gpt-4o", "description": "", "enabled": False},
                    ],
                }
            },
        }
    )
    # default list resolves through the enabled set
    assert models_cmd.resolve_default(inference, "query") == "openai.gpt-4o-mini"
    # a task list is used exclusively (mirrors Runner.inference_model):
    # disabled-only entries resolve to nothing, not to the default list
    assert models_cmd.resolve_default(inference, "pipe") is None
    # within a list, the first registered entry wins
    inference.model_picker["pipe"] = ["openai.gpt-4o", "openai.gpt-4o-mini"]
    assert models_cmd.resolve_default(inference, "pipe") == "openai.gpt-4o-mini"
    # an explicit override wins when registered, ignored when not
    assert models_cmd.resolve_default(inference, "query", override="openai.gpt-4o-mini") == "openai.gpt-4o-mini"
    assert models_cmd.resolve_default(inference, "query", override="no.such/model") == "openai.gpt-4o-mini"


def test_resolve_default_with_nothing_enabled():
    inference = inference_from({"model_picker": {"default": ["openai.gpt-4o-mini"]}, "providers": {}})
    assert models_cmd.resolve_default(inference, "query") is None


def test_list_models_prints_rows_and_defaults(tmp_path, capsys):
    path = write_config(tmp_path / "config.yaml")
    models_cmd.list_models(make_args(tmp_path, config=path))
    out = capsys.readouterr().out
    assert "PROVIDER" in out and "MODEL" in out and "DEFAULT FOR" in out
    assert "gpt-4o-mini" in out
    assert "gpt-4o" in out  # disabled models are listed too, marked
    assert "defaults:" in out
    assert "query = openai.gpt-4o-mini" in out


def test_list_models_marks_disabled_models_and_reports_unresolved_defaults(tmp_path, capsys):
    path = write_config(tmp_path / "config.yaml", enable_mini=False)
    code = models_cmd.list_models(make_args(tmp_path, config=path))
    assert code is None
    out = capsys.readouterr().out
    assert "gpt-4o-mini" in out
    assert "disabled" in out
    assert "defaults: query = none" in out


def test_list_models_with_empty_catalog_hints_on_stderr(tmp_path, capsys):
    path = tmp_path / "config.yaml"
    path.write_text(yaml.safe_dump({"vsn": "0.0.1", "inference": {"vsn": "0.0.1", "providers": {}}}))
    code = models_cmd.list_models(make_args(tmp_path, config=path))
    captured = capsys.readouterr()
    assert code is None
    assert "no models configured" in captured.err
    assert "defaults:" not in captured.out


# --- history_cmd -------------------------------------------------------------

def test_history_missing_database_is_a_friendly_note(tmp_path, capsys):
    history_cmd.list_sessions(make_args(tmp_path, database=tmp_path / "nope.db"))
    captured = capsys.readouterr()
    assert "no sessions yet" in captured.err
    assert captured.out == ""


def test_history_lists_sessions_newest_first(tmp_path, capsys):
    db_path = tmp_path / "smah.db"
    db = init_db(db_path)
    for n, title in ((1, "first chat"), (2, "second chat")):
        db.save_chat(
            title,
            SimpleNamespace(command=None, deprecated=[]),
            {"stages": {}},
            [{"role": "user", "content": f"hi {n}"}],
        )
    history_cmd.list_sessions(make_args(tmp_path, database=db_path))
    out = capsys.readouterr().out
    assert "first chat" in out
    assert "second chat" in out
    assert out.index("second chat") < out.index("first chat")
    assert "ID" in out and "TITLE" in out


def test_history_respects_limit(tmp_path, capsys):
    db_path = tmp_path / "smah.db"
    db = init_db(db_path)
    for n in range(5):
        db.save_chat(f"chat {n}", SimpleNamespace(), {}, [{"role": "user", "content": "x"}])
    history_cmd.list_sessions(make_args(tmp_path, database=db_path), limit=2)
    out = capsys.readouterr().out
    assert out.count("chat ") == 2


def test_history_empty_database_says_so(tmp_path, capsys):
    db_path = tmp_path / "smah.db"
    init_db(db_path)
    history_cmd.list_sessions(make_args(tmp_path, database=db_path))
    assert "no sessions" in capsys.readouterr().err


# --- doctor -------------------------------------------------------------------

def doctor_checks(tmp_path, config=None, database=None, offline=False):
    checks, worst = doctor_cmd.run_checks(make_args(tmp_path, config=config, database=database), offline=offline)
    by_name = {c["name"]: c for c in checks}
    return by_name, worst


def test_doctor_healthy_setup_passes(tmp_path):
    config = write_config(tmp_path / "config.yaml")
    db_path = tmp_path / "smah.db"
    init_db(db_path)
    db = Database(SimpleNamespace(database=str(db_path)))
    db.save_chat("t", SimpleNamespace(), {}, [{"role": "user", "content": "x"}])
    with mock.patch.dict(os.environ, {}, clear=False):
        os.environ.pop("OPENAI_API_KEY", None)
        os.environ.pop("SMAH_OPENAI_API_KEY", None)
        by_name, worst = doctor_checks(tmp_path, config=config, database=db_path)
    assert int(worst) == int(ExitCode.OK)
    assert by_name["config"]["status"] == "ok"
    assert by_name["database"]["status"] == "ok"
    assert by_name["key"]["status"] == "ok"
    assert by_name["models"]["status"] == "ok"
    assert by_name["catalog"]["status"] == "ok"
    assert by_name["provider"]["status"] == "ok"
    # presence only — the key itself must never appear
    assert "sk-test" not in json.dumps(by_name)


def test_doctor_missing_config_reports_root_cause_and_skips_downstream(tmp_path):
    by_name, worst = doctor_checks(tmp_path, config=tmp_path / "nope.yaml")
    assert int(worst) == int(ExitCode.CONFIG)
    assert by_name["config"]["status"] == "FAIL"
    for name in ("key", "models", "catalog", "provider"):
        assert by_name[name]["status"] == "skip", name
    assert by_name["database"]["status"] == "ok"  # missing db is not an error


def test_doctor_corrupt_config_fails_with_exit_three(tmp_path):
    path = tmp_path / "config.yaml"
    path.write_text("vsn: [unclosed")
    by_name, worst = doctor_checks(tmp_path, config=path)
    assert int(worst) == int(ExitCode.CONFIG)
    assert by_name["config"]["status"] == "FAIL"
    assert "invalid at" in by_name["config"]["detail"]


def test_doctor_missing_api_key_fails_auth(tmp_path):
    config = write_config(tmp_path / "config.yaml", api_key=None)
    env = {k: v for k, v in os.environ.items() if "_API_KEY" not in k}
    with mock.patch.dict(os.environ, env, clear=True):
        by_name, worst = doctor_checks(tmp_path, config=config)
    assert int(worst) == int(ExitCode.AUTH_OR_MODEL)
    assert by_name["key"]["status"] == "FAIL"
    assert "no api key found" in by_name["key"]["detail"]
    assert "sk" not in by_name["key"]["detail"]


def test_doctor_no_enabled_models_fails_auth_and_warns_catalog(tmp_path):
    config = write_config(tmp_path / "config.yaml", enable_mini=False)
    with mock.patch.dict(os.environ, {}, clear=False):
        by_name, worst = doctor_checks(tmp_path, config=config)
    assert int(worst) == int(ExitCode.AUTH_OR_MODEL)
    assert by_name["models"]["status"] == "FAIL"
    # the picker references the now-disabled model id
    assert by_name["catalog"]["status"] == "warn"
    assert "gpt-4o-mini" in by_name["catalog"]["detail"]


def test_doctor_env_var_key_counts_as_present(tmp_path):
    config = write_config(tmp_path / "config.yaml", api_key=None)
    env = {k: v for k, v in os.environ.items() if "_API_KEY" not in k}
    env["OPENAI_API_KEY"] = "sk-from-env"
    with mock.patch.dict(os.environ, env, clear=True):
        by_name, worst = doctor_checks(tmp_path, config=config)
    assert int(worst) == int(ExitCode.OK)
    assert by_name["key"]["status"] == "ok"


def test_doctor_non_openai_provider_is_not_wired(tmp_path):
    path = write_config(tmp_path / "config.yaml")
    data = yaml.safe_load(path.read_text())
    data["inference"]["providers"]["anthropic"] = {
        "vsn": "0.0.1",
        "name": "Anthropic",
        "description": "",
        "enabled": True,
        "settings": {"api_key": "sk-ant"},
        "models": [{"vsn": "0.0.1", "name": "sonnet", "model": "sonnet", "description": "", "enabled": True}],
    }
    data["inference"]["model_picker"] = {"default": ["openai.gpt-4o-mini"]}
    path.write_text(yaml.safe_dump(data))
    by_name, worst = doctor_checks(tmp_path, config=path)
    assert int(worst) == int(ExitCode.AUTH_OR_MODEL)
    assert by_name["provider"]["status"] == "FAIL"
    assert "anthropic" in by_name["provider"]["detail"]


def test_doctor_pending_migrations_are_noted(tmp_path):
    config = write_config(tmp_path / "config.yaml")
    db_path = tmp_path / "smah.db"
    init_db(db_path)
    # wipe the applied-migrations ledger: everything shows as pending
    with sqlite3.connect(db_path) as connection:
        connection.execute("DELETE FROM schema_migrations")
    by_name, worst = doctor_checks(tmp_path, config=config, database=db_path)
    assert int(worst) == int(ExitCode.OK)
    assert by_name["database"]["status"] == "ok"
    assert "pending" in by_name["database"]["detail"]


def test_doctor_uninitialized_db_file_is_ok(tmp_path):
    config = write_config(tmp_path / "config.yaml")
    db_path = tmp_path / "empty.db"
    db_path.write_bytes(b"")
    by_name, worst = doctor_checks(tmp_path, config=config, database=db_path)
    assert int(worst) == int(ExitCode.OK)
    assert "not initialized" in by_name["database"]["detail"]


def test_doctor_json_output_is_parseable_and_on_stdout(tmp_path, capsys):
    config = write_config(tmp_path / "config.yaml")
    args = make_args(tmp_path, config=config, doctor_json=True)
    code = doctor_cmd.run_doctor(args)
    payload = json.loads(capsys.readouterr().out)
    assert code == int(ExitCode.OK)
    assert [c["name"] for c in payload] == ["config", "database", "key", "models", "catalog", "provider"]
    assert all(set(c) == {"name", "status", "detail"} for c in payload)
    assert "sk-test" not in json.dumps(payload)


def test_doctor_human_report_goes_to_stderr(tmp_path, capsys):
    config = write_config(tmp_path / "config.yaml")
    code = doctor_cmd.run_doctor(make_args(tmp_path, config=config))
    captured = capsys.readouterr()
    assert code == int(ExitCode.OK)
    assert captured.out == ""
    assert "All checks passed" in captured.err
