import os

import yaml

from smah.settings.inference.inference import Inference
from smah.settings.system.system import System

DEFAULTS_YAML = os.path.join(
    os.path.dirname(__file__), "..", "smah", "settings", "inference", "inference_defaults.yaml"
)


def test_defaults_register_claude_and_openai_models():
    with open(DEFAULTS_YAML, "r") as file:
        config_data = yaml.safe_load(file)
    inference = Inference(config_data)

    for expected in [
        "anthropic.claude-opus-5-5",
        "anthropic.claude-haiku-5-5",
        "openai.gpt-4o",
        "openai.gpt-4o-mini",
    ]:
        assert expected in inference.models, f"{expected} should be registered from defaults"

    assert inference.models["anthropic.claude-opus-5-5"].model == "claude-opus-5-5"
    assert inference.models["anthropic.claude-haiku-5-5"].model == "claude-haiku-5-5"


def test_defaults_are_claude_first():
    with open(DEFAULTS_YAML, "r") as file:
        config_data = yaml.safe_load(file)
    inference = Inference(config_data)
    assert inference.model_picker["default"][0] == "anthropic.claude-haiku-5-5"
    assert inference.model_picker["query"] == ["anthropic.claude-haiku-5-5"]
    assert inference.model_picker["pipe"] == ["anthropic.claude-haiku-5-5"]


def test_existing_openai_entries_keep_their_enabled_flags():
    with open(DEFAULTS_YAML, "r") as file:
        config_data = yaml.safe_load(file)
    openai_models = {m["name"]: m for m in config_data["providers"]["openai"]["models"]}
    assert openai_models["gpt-4o"]["enabled"] is True
    assert openai_models["gpt-4o-mini"]["enabled"] is True
    assert openai_models["gpt-4-turbo"]["enabled"] is False
    assert openai_models["o1"]["enabled"] is False
    assert openai_models["o1-mini"]["enabled"] is False


def test_system_stats_reports_memory_not_cpu_readings():
    system = System()
    stats = system.to_yaml({"stats": True})
    assert "cpu_count" in stats["cpu"]
    assert "total" in stats["memory"]
    assert "cpu_count" not in stats["memory"]
