"""
doctor command: installation diagnostics.

Checks, in order: config (found + valid + version gate), database (present,
readable, migrations current), API key presence for enabled providers
(presence only — never the value), enabled-model count and defaults, stale
ids in the model_picker lists, and provider wiring. Reads config and the
settings layer only: no network calls, no provider clients.

Output discipline: the human report goes to stderr; --json goes to stdout.
Exit code is the worst finding: 0 all pass · 3 config/database · 4 key/models.
"""

import json
import os
import platform
import sqlite3
from importlib import metadata

import yaml

from smah.console import err_console
from smah.database import Database, Migration
from smah.exit_codes import ExitCode
from smah.settings import Settings
from smah.settings.inference.inference import Inference

from smah.commands import models_cmd


def __smah_version() -> str:
    try:
        return metadata.version("smah")
    except Exception:
        return "unknown"


def __check_config(args) -> tuple[dict, object]:
    """
    Returns (check, settings). settings is None when the config is missing or
    invalid — downstream checks are skipped in that case, so exit 3 (config)
    dominates instead of cascading into auth/model failures.
    """
    path = args.config or Settings.default_config()
    if not os.path.exists(path):
        return {
            "name": "config",
            "status": "FAIL",
            "detail": f"not found at {path}",
            "hint": "run 'smah config' to create one",
            "exit": ExitCode.CONFIG,
        }, None
    try:
        settings = Settings(config=args.config)
    except Exception as e:
        return {
            "name": "config",
            "status": "FAIL",
            "detail": f"invalid at {path}: {e}",
            "hint": "run 'smah config' to regenerate",
            "exit": ExitCode.CONFIG,
        }, None
    incomplete = "" if settings.is_configured() else " · incomplete (wizard runs on next ask)"
    return {
        "name": "config",
        "status": "ok",
        "detail": f"{path} (vsn {settings.vsn}){incomplete}",
        "hint": None,
        "exit": None,
    }, settings


def __check_database(args) -> dict:
    path = args.database or Database.default_database()
    if not os.path.exists(path):
        return {
            "name": "database",
            "status": "ok",
            "detail": f"{path} · not created yet (created on first run)",
            "hint": None,
            "exit": None,
        }
    try:
        connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
        try:
            try:
                count = connection.execute("SELECT COUNT(*) FROM chat_history").fetchone()[0]
            except sqlite3.OperationalError as e:
                if "no such table" in str(e):
                    return {
                        "name": "database",
                        "status": "ok",
                        "detail": f"{path} · not initialized yet (schema created on first run)",
                        "hint": None,
                        "exit": None,
                    }
                raise
            try:
                applied = {
                    row[0] for row in connection.execute(
                        "SELECT migration FROM schema_migrations WHERE applied"
                    ).fetchall()
                }
            except sqlite3.OperationalError:
                applied = set()
            shipped = {m["file"] for m in Migration.get_migrations()}
            pending = sorted(shipped - applied)
            state = "schema current" if not pending else f"{len(pending)} migration(s) pending (auto-applied on next run)"
            return {
                "name": "database",
                "status": "ok",
                "detail": f"{path} · {count} session{'s' if count != 1 else ''} · {state}",
                "hint": None,
                "exit": None,
            }
        finally:
            connection.close()
    except sqlite3.DatabaseError as e:
        return {
            "name": "database",
            "status": "FAIL",
            "detail": f"problem at {path}: {e}",
            "hint": "try 'smah-db status'",
            "exit": ExitCode.CONFIG,
        }


def __enabled_providers(settings) -> list:
    inference = settings.inference or Inference(None)
    return [(pid, p) for pid, p in inference.providers.items() if p.enabled]


def __check_key(args, settings) -> dict:
    providers = __enabled_providers(settings)
    if not providers:
        return {
            "name": "key",
            "status": "FAIL",
            "detail": "no enabled providers in config",
            "hint": "run 'smah config'",
            "exit": ExitCode.AUTH_OR_MODEL,
        }
    for pid, provider in providers:
        key = provider.api_key(args)
        if not key:
            env = f"{pid.upper()}_API_KEY"
            return {
                "name": "key",
                "status": "FAIL",
                "detail": f"{pid}: no api key found",
                "hint": f"fix: 'smah config' or export {env}",
                "exit": ExitCode.AUTH_OR_MODEL,
            }
    names = ", ".join(f"{pid}: present (not printed)" for pid, _ in providers)
    return {"name": "key", "status": "ok", "detail": names, "hint": None, "exit": None}


def __check_models(args, settings) -> dict:
    inference = settings.inference or Inference(None)
    enabled = inference.models
    if not enabled:
        return {
            "name": "models",
            "status": "FAIL",
            "detail": "no models enabled",
            "hint": "run 'smah config' and enable a model, or check 'smah models list'",
            "exit": ExitCode.AUTH_OR_MODEL,
        }
    defaults = models_cmd.stage_defaults(args, inference)
    missing = [t for t in ("query", "pipe", "picker") if not defaults[t]]
    if missing:
        return {
            "name": "models",
            "status": "FAIL",
            "detail": f"{len(enabled)} enabled · defaults unresolved for: {', '.join(missing)}",
            "hint": "check 'smah models list' and the model_picker lists in config",
            "exit": ExitCode.AUTH_OR_MODEL,
        }
    return {
        "name": "models",
        "status": "ok",
        "detail": f"{len(enabled)} enabled · defaults set",
        "hint": None,
        "exit": None,
    }


def __check_catalog(args, settings) -> dict:
    inference = settings.inference or Inference(None)
    referenced: list[str] = []
    for keys in (inference.model_picker or {}).values():
        if isinstance(keys, list):
            referenced.extend(k for k in keys if isinstance(k, str))
    stale = sorted({k for k in referenced if k not in inference.models})
    if stale:
        return {
            "name": "catalog",
            "status": "warn",
            "detail": f"stale/disabled model ids in model_picker: {', '.join(stale)}",
            "hint": "enable them or update model_picker in 'smah config'",
            "exit": None,
        }
    return {"name": "catalog", "status": "ok", "detail": "no stale model ids", "hint": None, "exit": None}


def __check_provider(args, settings, offline: bool) -> dict:
    providers = __enabled_providers(settings)
    if not providers:
        return {
            "name": "provider",
            "status": "FAIL",
            "detail": "no enabled providers in config",
            "hint": "run 'smah config'",
            "exit": ExitCode.AUTH_OR_MODEL,
        }
    for pid, _ in providers:
        if pid != "openai":
            return {
                "name": "provider",
                "status": "FAIL",
                "detail": f"{pid}: not wired up yet (only the OpenAI provider is runnable)",
                "hint": "disable it in 'smah config' or wait for provider support",
                "exit": ExitCode.AUTH_OR_MODEL,
            }
    suffix = " · offline" if offline else ""
    names = ", ".join(pid for pid, _ in providers)
    return {
        "name": "provider",
        "status": "ok",
        "detail": f"{names} wired{suffix} (reachability not probed in this version; no network calls)",
        "hint": None,
        "exit": None,
    }


def run_checks(args, offline: bool = False) -> tuple[list[dict], ExitCode]:
    """
    Runs every check and returns (checks, worst_exit_code). When the config is
    missing/invalid, the settings-dependent checks report 'skip' so the exit
    code reflects the root cause (3, config) rather than its consequences.
    """
    config, settings = __check_config(args)

    checks = [config, __check_database(args)]
    if settings is None:
        for name in ("key", "models", "catalog", "provider"):
            checks.append({"name": name, "status": "skip", "detail": "skipped — fix config first", "hint": None, "exit": None})
    else:
        checks += [
            __check_key(args, settings),
            __check_models(args, settings),
            __check_catalog(args, settings),
            __check_provider(args, settings, offline),
        ]
    failures = [c["exit"] for c in checks if c["status"] == "FAIL" and c["exit"] is not None]
    worst = max(failures) if failures else ExitCode.OK
    return checks, worst


def run_doctor(args) -> int:
    """
    Executes the doctor command. Returns the process exit code.
    """
    checks, worst = run_checks(args, offline=args.doctor_offline)

    if args.doctor_json:
        payload = [{k: c[k] for k in ("name", "status", "detail")} for c in checks]
        print(json.dumps(payload, indent=2))
        return int(worst)

    err_console.print(f"\nsmah {__smah_version()} · python {platform.python_version()} · "
                      f"{platform.system().lower()} {platform.release()}\n")
    for check in checks:
        err_console.print(f"  {check['name']:<10}{check['status']:<6}{check['detail']}", markup=False, highlight=False)
        if check["hint"]:
            err_console.print(f"  {'':<10}fix: {check['hint']}", markup=False, highlight=False)
    failed = sum(1 for c in checks if c["status"] == "FAIL")
    if failed:
        err_console.print(f"\n  {failed} check{'s' if failed != 1 else ''} failed.")
    else:
        err_console.print("\n  All checks passed.")
    return int(worst)
