"""
smah.providers.anthropic_provider

Runs completions against Anthropic's Messages API through the official
`anthropic` SDK (no raw HTTP, no OpenAI-compatible shim).

Request shape (Claude Opus 5.5 / Haiku 5.5):
* the system prompt is the top-level `system` parameter — never a
  {"role": "system"} message;
* `max_tokens` is required and sent on every call (16000 by default);
* `budget_tokens`, `temperature`, and `top_p` are NOT sent (rejected with a
  400 on these models);
* `response.content` is a list of typed blocks: only blocks with
  `type == "text"` contribute text, and content[0] is never assumed.

Error handling is the SDK's typed exception chain, most-specific-first — no
string matching, no single broad class.
"""
from __future__ import annotations

from typing import TYPE_CHECKING, NoReturn, Optional

import anthropic

from smah.exit_codes import ExitCode, fatal
from smah.providers.base import UNSET, Unset, Completion, Provider, log_payload

if TYPE_CHECKING:  # pragma: no cover - typing only
    from smah.settings.inference.provider.model import Model
    from smah.settings.inference.provider.provider import Provider as ProviderSettings

# max_tokens is required by the Messages API. 16000 is the fallback when a
# model entry carries no output budget; streaming is not wired up yet.
ANTHROPIC_DEFAULT_MAX_TOKENS = 16000

JSON_ONLY_INSTRUCTION = (
    "Respond with ONLY a single JSON object that matches the requested schema. "
    "No prose, no explanations, and no markdown code fences."
)


def _response_summary(response) -> dict:
    """Logging-safe summary of a Messages response (never includes credentials)."""
    content = []
    for block in getattr(response, "content", None) or []:
        content.append({
            "type": getattr(block, "type", None),
            "text": getattr(block, "text", None),
        })
    return {
        "id": getattr(response, "id", None),
        "model": getattr(response, "model", None),
        "stop_reason": getattr(response, "stop_reason", None),
        "content": content,
    }


class AnthropicProvider(Provider):
    key = "anthropic"

    def credential_help(self) -> str:
        return (
            "No Anthropic API key found. Set the ANTHROPIC_API_KEY (or SMAH_ANTHROPIC_API_KEY) "
            "environment variable, add an api_key to the anthropic provider block via `smah --configure`, "
            "or select an OpenAI model with --model / --model-query / --model-pipe (e.g. openai.gpt-4o-mini)."
        )

    def complete(
            self,
            model: "Model",
            thread: list,
            system: Optional[str] = None,
            response_format: dict | Unset = UNSET,
            tools: dict | Unset = UNSET,  # noqa: ARG002 - tool calling not wired up yet
            options: Optional[dict] = None,
            show: bool = False
    ) -> Completion:
        options = options or {}
        api_key = self.require_api_key()

        # Anthropic takes only user/assistant turns; the system prompt is the
        # top-level `system` parameter below.
        messages = [
            {"role": m["role"], "content": m["content"]}
            for m in thread
            if m.get("role") in ("user", "assistant")
        ]

        system_prompt = system
        if response_format is not UNSET and response_format is not None:
            # Anthropic has no response_format parameter; ask for JSON in the
            # system prompt and parse defensively on the way out.
            system_prompt = (
                f"{system_prompt}\n\n{JSON_ONLY_INSTRUCTION}" if system_prompt else JSON_ONLY_INSTRUCTION
            )

        # max_tokens is required by the Messages API. Honor the model's
        # configured output budget (catalog `context.out`, model settings
        # override) and fall back to the default only when absent.
        model_settings = model.settings or {}
        model_output_budget = (model.context or {}).get("out")
        max_tokens = options.get(
            "max_tokens",
            model_settings.get("max_tokens", model_output_budget or ANTHROPIC_DEFAULT_MAX_TOKENS)
        )

        log_payload(
            "Anthropic Completion Payload",
            {
                'model': model.to_yaml(),
                'system': system_prompt or False,
                'response_format': response_format if response_format is not UNSET else False,
                'options': options or False,
                'thread': messages
            },
            show=show
        )

        client = anthropic.Anthropic(api_key=api_key)

        # The system prompt is the top-level `system` parameter (never a
        # {"role": "system"} message); it is omitted entirely when absent so it
        # is not serialized as null.
        request_kwargs = {
            "model": model.model,
            "max_tokens": max_tokens,
            "messages": messages,
        }
        if system_prompt is not None:
            request_kwargs["system"] = system_prompt

        try:
            response = client.messages.create(**request_kwargs)
        except anthropic.APIError as exc:
            self.translate_error(exc)

        log_payload("Anthropic Completion Response", _response_summary(response), show=show)

        # content is a list of typed blocks; never assume content[0] is text.
        text = "\n".join(
            getattr(block, "text", "")
            for block in response.content
            if getattr(block, "type", None) == "text"
        )
        return Completion(text=text, role="assistant", raw=response)

    def translate_error(self, exc: BaseException) -> NoReturn:
        # Most-specific-first chain, exactly as the SDK defines them.
        if isinstance(exc, (anthropic.AuthenticationError, anthropic.PermissionDeniedError)):
            fatal(
                f"Anthropic rejected the API key (HTTP {exc.status_code}). Update it with "
                "`smah --configure`, or set ANTHROPIC_API_KEY (or SMAH_ANTHROPIC_API_KEY).",
                ExitCode.AUTH_OR_MODEL
            )
        elif isinstance(exc, anthropic.NotFoundError):
            fatal(
                f"Anthropic did not recognize the model or endpoint (HTTP {exc.status_code}): "
                f"{exc.message}. Check the model id via --model / --model-query / --model-pipe.",
                ExitCode.PROVIDER
            )
        elif isinstance(exc, anthropic.RateLimitError):
            fatal(f"Anthropic rate limited the request (HTTP {exc.status_code}). Try again shortly.", ExitCode.PROVIDER)
        elif isinstance(exc, anthropic.BadRequestError):
            fatal(f"Anthropic rejected the request (HTTP {exc.status_code}): {exc.message}.", ExitCode.PROVIDER)
        elif isinstance(exc, anthropic.APIStatusError):
            if exc.status_code is not None and 500 <= exc.status_code < 600:
                fatal(f"Anthropic is having trouble (HTTP {exc.status_code}). Try again shortly.", ExitCode.PROVIDER)
            fatal(f"Anthropic API error (HTTP {exc.status_code}): {exc.message}.", ExitCode.PROVIDER)
        elif isinstance(exc, anthropic.APITimeoutError):
            fatal("Anthropic timed out. Try again, or narrow the request.", ExitCode.PROVIDER)
        elif isinstance(exc, anthropic.APIConnectionError):
            fatal(f"Can't reach Anthropic ({exc}). Check the network, then retry.", ExitCode.PROVIDER)
        else:
            raise exc
