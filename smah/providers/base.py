"""
smah.providers.base

The provider contract SMAH's runner talks to, plus the normalized completion
type every provider returns.

The contract is deliberately shaped by what the runner actually needs:

* resolve an API key (CLI flags / config `settings.api_key` / environment,
  with `$ENV_VAR` indirection) and report whether one is present — without
  ever printing the key or any part of one;
* `complete()` — run a chat completion for a model and return normalized
  text (`Completion`);
* `translate_error()` — map the provider SDK's typed exceptions onto SMAH's
  exit codes (see `smah.exit_codes`). Translation always terminates the
  process through `fatal`, so callers never see these errors return.
"""
from __future__ import annotations

import logging
import os
from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, ClassVar, NoReturn, Optional

import rich.box
import yaml
from rich.markdown import Markdown
from rich.panel import Panel

from smah.console import err_console
from smah.exit_codes import ExitCode, fatal

if TYPE_CHECKING:  # pragma: no cover - typing only, avoids a runtime cycle
    from smah.settings.inference.provider.model import Model
    from smah.settings.inference.provider.provider import Provider as ProviderSettings


class _Unset:
    """
    Sentinel for "parameter not set" — vendor-neutral on purpose. The provider
    abstraction must not borrow one vendor SDK's sentinel (the runner still
    accepts openai's NOT_GIVEN for signature compatibility, but it is
    translated to UNSET before reaching providers; each vendor provider
    translates UNSET to its own SDK's sentinel internally).
    """
    __slots__ = ()

    def __repr__(self) -> str:
        return "UNSET"


Unset = _Unset
UNSET: Unset = _Unset()


@dataclass
class Completion:
    """
    Provider-neutral completion result.

    Attributes:
        text: The assistant's textual answer (all text blocks joined).
        role: The message role that produced it (always "assistant" today).
        raw: The provider SDK's native response object, kept for logging and
            diagnostics only. Callers must not depend on its shape.
    """
    text: str
    role: str = "assistant"
    raw: Any = None


def log_payload(title: str, payload: Any, show: bool = False, level: int = logging.INFO) -> None:
    """
    Log (and optionally panel-print) a YAML payload exactly the way the runner
    historically logged OpenAI request/response payloads.
    """
    body = yaml.dump(payload, sort_keys=False)
    logging.log(level, f"{title}:\n{body}")
    if show:
        err_console.print(
            Panel(
                Markdown("```yaml\n" + body + "\n```\n"),
                title=title,
                style="bold white",
                box=rich.box.ROUNDED
            )
        )


class Provider(ABC):
    """
    Abstract inference provider.

    Concrete providers (OpenAI, Anthropic, ...) implement credential
    resolution, the completion call, and error translation behind one seam so
    the runner never branches on provider identity.
    """
    key: ClassVar[str]

    def __init__(self, args=None, settings_provider: Optional["ProviderSettings"] = None):
        self.args = args
        self.settings_provider = settings_provider

    # ------------------------------------------------------------------
    # Credentials
    # ------------------------------------------------------------------
    def resolve_api_key(self) -> Optional[str]:
        """
        Resolve this provider's API key without ever printing it.

        Order: the config provider block (which honors CLI flags for OpenAI,
        `settings.api_key` including `$ENV_VAR` indirection) then the
        SMAH_<KEY>_API_KEY / <KEY>_API_KEY environment variables.

        Returns:
            The resolved key, or None when no credential is available.
        """
        if self.settings_provider is not None:
            key = self.settings_provider.api_key(self.args)
            if key:
                return key
        prefix = self.key.upper()
        return os.environ.get(f"SMAH_{prefix}_API_KEY") or os.environ.get(f"{prefix}_API_KEY")

    def has_credentials(self) -> bool:
        """Whether a credential is resolvable for this provider (never prints it)."""
        return bool(self.resolve_api_key())

    @abstractmethod
    def credential_help(self) -> str:
        """Message naming exactly which env var / flag / config field supplies the key."""

    def require_api_key(self) -> str:
        """
        Preflight: resolve the key or exit 4 (auth) with provider-specific
        guidance. Must run before any network activity.
        """
        key = self.resolve_api_key()
        if not key:
            fatal(self.credential_help(), ExitCode.AUTH_OR_MODEL)
        return key

    # ------------------------------------------------------------------
    # Completion
    # ------------------------------------------------------------------
    @abstractmethod
    def complete(
            self,
            model: "Model",
            thread: list,
            system: Optional[str] = None,
            response_format: dict | Unset = UNSET,
            tools: dict | Unset = UNSET,
            options: Optional[dict] = None,
            show: bool = False
    ) -> Completion:
        """Run a completion and return the normalized text."""

    # ------------------------------------------------------------------
    # Error translation
    # ------------------------------------------------------------------
    @abstractmethod
    def translate_error(self, exc: BaseException) -> NoReturn:
        """
        Map a provider SDK exception onto SMAH's exit codes. Terminates the
        process for recognized errors; re-raises anything it does not own.
        """
