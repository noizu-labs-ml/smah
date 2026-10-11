"""
smah.providers

Provider abstraction for SMAH's inference backends. The runner resolves a
provider by config key through the registry and talks to it exclusively
through the `Provider` contract — it never branches on provider identity.
"""
from smah.providers.anthropic_provider import AnthropicProvider
from smah.providers.base import UNSET, Unset, Completion, Provider, ProviderSettingsError, is_unset
from smah.providers.openai_provider import OpenAIProvider
from smah.providers.registry import PROVIDER_CLASSES, get_provider, known_provider_keys, maybe_get_provider

__all__ = [
    "AnthropicProvider",
    "Completion",
    "OpenAIProvider",
    "PROVIDER_CLASSES",
    "Provider",
    "ProviderSettingsError",
    "UNSET",
    "Unset",
    "get_provider",
    "is_unset",
    "known_provider_keys",
    "maybe_get_provider",
]
