"""
smah.providers.registry

Maps a provider key from config (`openai`, `anthropic`, ...) to its
implementation. Unknown keys are a configuration problem: SMAH exits 3 with
a message naming the known providers.
"""
from __future__ import annotations

from typing import Type

from smah.exit_codes import ExitCode, fatal
from smah.providers.anthropic_provider import AnthropicProvider
from smah.providers.base import Provider
from smah.providers.openai_provider import OpenAIProvider

PROVIDER_CLASSES: dict[str, Type[Provider]] = {
    OpenAIProvider.key: OpenAIProvider,
    AnthropicProvider.key: AnthropicProvider,
}


def known_provider_keys() -> list[str]:
    """The provider keys SMAH knows how to run, sorted for stable messages."""
    return sorted(PROVIDER_CLASSES)


def get_provider(
        key: str,
        args=None,
        settings_provider=None
) -> Provider:
    """
    Resolve a provider implementation by config key.

    Raises:
        SystemExit (3): the key names no known provider — a config problem.
    """
    cls = PROVIDER_CLASSES.get(key)
    if cls is None:
        known = ", ".join(known_provider_keys())
        fatal(
            f"The config references the provider '{key}', which SMAH does not know how to run. "
            f"Known providers: {known}. Fix the provider blocks in the SMAH config or run "
            f"`smah --configure`.",
            ExitCode.CONFIG
        )
    return cls(args=args, settings_provider=settings_provider)
