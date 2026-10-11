"""
config command: show / path / edit / set.

`show` must never print secret values: any key that looks like a credential
is masked. `set` writes one dotted key into the config YAML without echoing
secret values. `edit` re-runs the interactive wizard (in-process prompts —
SMAH never spawns editors or shells).
"""

import os
import sys
from typing import Optional

import yaml

from smah.console import err_console
from smah.exit_codes import ExitCode, fatal
from smah.settings import Settings

SECRET_KEYS = {"api_key", "apikey", "key", "token", "secret", "password"}


def config_path(args) -> str:
    """
    Resolves the effective config file path for this invocation.
    """
    return args.config or Settings.default_config()


def mask_secret(value: str) -> str:
    """
    Masks a secret value. `$ENV_VAR` indirection is revealed as the env var
    name (the name is not a secret; the resolved value never is).
    """
    value = str(value)
    if value.startswith("$"):
        return f"<env:{value.lstrip('$').lstrip('{').rstrip('}')}>"
    return "••••••••"


def redact(node):
    """
    Recursively masks values under secret-looking keys. Returns a new
    structure; the input is not modified.
    """
    if isinstance(node, dict):
        return {
            k: (mask_secret(v) if _is_secret_key(k) and isinstance(v, (str, int, float)) else redact(v))
            for k, v in node.items()
        }
    if isinstance(node, list):
        return [redact(item) for item in node]
    return node


def _is_secret_key(key: str) -> bool:
    return str(key).lower().replace("-", "_") in SECRET_KEYS


def show(args) -> None:
    """
    Prints the current config to stdout with secrets masked.
    Missing config is a usage error pointing at the wizard; a corrupt config
    is a config error (exit 3).
    """
    path = config_path(args)
    data = __load_yaml(path, missing_ok=False)
    sys.stdout.write(yaml.safe_dump(redact(data), sort_keys=False, default_flow_style=False, allow_unicode=True))


def print_path(args) -> None:
    """
    Prints the resolved config file path to stdout.
    """
    print(config_path(args))


def __load_yaml(path: str, missing_ok: bool) -> Optional[dict]:
    if not os.path.exists(path):
        if missing_ok:
            return None
        fatal(f"no config found at {path}. Run 'smah config' to create one.", ExitCode.USAGE)
    try:
        with open(path, "r") as file:
            return yaml.safe_load(file) or {}
    except yaml.YAMLError as e:
        fatal(f"can't read config at {path}: {e}. Fix it, or move it aside and run 'smah config'.", ExitCode.CONFIG)
    except OSError as e:
        fatal(f"can't read config at {path}: {e}. Fix it, or move it aside and run 'smah config'.", ExitCode.CONFIG)


def __coerce(value: str):
    """
    Coerces a CLI string into bool/int when it looks like one.
    """
    lowered = value.lower()
    if lowered in ("true", "yes", "on"):
        return True
    if lowered in ("false", "no", "off"):
        return False
    try:
        return int(value)
    except ValueError:
        return value


def set_value(args, key: str, value: str) -> None:
    """
    Writes one dotted key into the config file. Secret values are never
    echoed; the confirmation names only the key.
    """
    path = config_path(args)
    data = __load_yaml(path, missing_ok=False)
    if not isinstance(data, dict):
        fatal(f"config at {path} is invalid: expected a mapping at top level. Run 'smah config' to regenerate.", ExitCode.CONFIG)

    node = data
    parts = key.split(".")
    for part in parts[:-1]:
        child = node.get(part)
        if child is not None and not isinstance(child, dict):
            fatal(f"can't set '{key}': '{part}' is not a section in {path}.", ExitCode.CONFIG)
        node = node.setdefault(part, {})
    node[parts[-1]] = __coerce(value)

    try:
        os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
        with open(path, "w") as file:
            yaml.safe_dump(data, file, sort_keys=False, default_flow_style=False, allow_unicode=True)
    except OSError as e:
        fatal(f"failed to write config at {path}: {e}", ExitCode.CONFIG)

    shown = mask_secret(value) if _is_secret_key(parts[-1]) else value
    err_console.print(f"[bold green]smah:[/bold green] config updated: {key} = {shown}")
