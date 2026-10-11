"""
models command: list.

Renders the configured model catalog (enabled and disabled) with which model
is the default for each stage. Reads the settings layer only — no provider
modules, no network. Resolution mirrors the Runner's config fallback
(CLI override → model_picker[task] → model_picker.default) without
constructing a Runner.
"""

import sys
from typing import Optional

import yaml

from smah.console import err_console
from smah.exit_codes import ExitCode, fatal
from smah.settings import Settings
from smah.settings.inference.inference import Inference


def load_settings(args) -> Settings:
    """
    Loads settings for read-only inspection. A missing config is not an error
    here — the empty catalog is the finding. A corrupt config is a config
    error (exit 3).
    """
    path = args.config or Settings.default_config()
    try:
        return Settings(config=args.config)
    except yaml.YAMLError as e:
        fatal(f"can't read config at {path}: {e}. Fix it, or move it aside and run 'smah config'.", ExitCode.CONFIG)
    except Exception as e:
        fatal(f"config at {path} is invalid: {e}. Run 'smah config' to regenerate.", ExitCode.CONFIG)


def resolve_default(inference: Inference, task: str, override: Optional[str] = None) -> Optional[str]:
    """
    Resolves the default model id for a stage ('query', 'pipe', 'picker')
    from the settings layer, mirroring Runner.inference_model's fallback:
    an explicit override that is registered wins, else the first
    model_picker entry (task list, then default list) that is registered.
    """
    if override and override in inference.models:
        return override
    picker = inference.model_picker or {}
    for key in (picker.get(task) or picker.get("default") or []):
        if key in inference.models:
            return key
    return None


def stage_defaults(args, inference: Inference) -> dict[str, Optional[str]]:
    """
    The resolved default model id per stage, honoring CLI overrides.
    """
    model = getattr(args, "model", None)
    return {
        "query": resolve_default(inference, "query", getattr(args, "model_query", None) or model),
        "pipe": resolve_default(inference, "pipe", getattr(args, "model_pipe", None) or model),
        "picker": resolve_default(inference, "picker"),
    }


def list_models(args) -> None:
    """
    Prints the catalog table to stdout (machine-consumable). Zero enabled
    models is a successful listing (exit 0) — the failure class belongs to
    ask/doctor, not to list.
    """
    settings = load_settings(args)
    inference = settings.inference or Inference(None)
    defaults = stage_defaults(args, inference)

    rows = []
    for pid, provider in inference.providers.items():
        for model in provider.models:
            mid = f"{pid}.{model.name}"
            if mid in inference.models:
                enabled, status = "yes", "ok"
            elif not model.enabled:
                enabled, status = "no", "disabled"
            else:
                enabled, status = "no", "incomplete"
            default_for = " ".join(t for t in ("query", "pipe", "picker") if defaults[t] == mid) or "—"
            rows.append((pid, model.name or "?", default_for, enabled, status))

    header = f"{'PROVIDER':<12}{'MODEL':<22}{'DEFAULT FOR':<14}{'ENABLED':<9}STATUS"
    print(header)
    for pid, name, default_for, enabled, status in rows:
        print(f"{pid:<12}{name:<22}{default_for:<14}{enabled:<9}{status}")

    if not rows:
        err_console.print(
            "[bold red]smah:[/bold red] no models configured. Run 'smah config' to set up a provider and enable a model."
        )
        return

    resolved = " · ".join(f"{t} = {defaults[t] or 'none'}" for t in ("query", "pipe", "picker"))
    print(f"\ndefaults: {resolved}")
    print("the answering model is chosen per request by the planner")
