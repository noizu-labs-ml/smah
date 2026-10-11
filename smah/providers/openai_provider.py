"""
smah.providers.openai_provider

Wraps the OpenAI completion path exactly as the runner previously ran it
inline: same credential rules (a configured `openai` provider block is
required, then `--openai-api-key` / config `api_key` / env vars), same
request parameters (including the max_tokens vs max_completion_tokens
juggling), same payload logging.
"""
from __future__ import annotations

from typing import TYPE_CHECKING, NoReturn, Optional

import openai
from openai import NOT_GIVEN, OpenAIError

from smah.exit_codes import ExitCode, fatal
from smah.providers.base import UNSET, Unset, Completion, Provider, log_payload

if TYPE_CHECKING:  # pragma: no cover - typing only
    from smah.settings.inference.provider.model import Model
    from smah.settings.inference.provider.provider import Provider as ProviderSettings


class OpenAIProvider(Provider):
    key = "openai"

    def resolve_api_key(self) -> Optional[str]:
        # Preserves the historical rule: without an `openai` provider block in
        # the config there is no key, even if OPENAI_API_KEY is exported.
        if self.settings_provider is None:
            return None
        return super().resolve_api_key()

    def credential_help(self) -> str:
        if self.settings_provider is None:
            return (
                "No OpenAI provider is configured. Run `smah --configure` and set an API key, "
                "or add an `openai` provider block to your config file."
            )
        return (
            "No OpenAI API key found. Set the OPENAI_API_KEY (or SMAH_OPENAI_API_KEY) environment "
            "variable, pass --openai-api-key, or run `smah --configure`."
        )

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
        options = options or {}
        api_key = self.require_api_key()

        messages = list(thread)
        if system:
            messages = [{"role": "system", "content": system}] + messages

        log_payload(
            "OpenAI Completion Payload",
            {
                'model': model.to_yaml(),
                'response_format': response_format if response_format is not UNSET else False,
                'options': options or False,
                'thread': messages
            },
            show=show
        )

        client = openai.OpenAI(api_key=api_key)
        model_settings = model.settings or {}

        max_output_tokens = (model.context or {}).get("out", 4096)
        max_tokens = options.get("max_tokens", model_settings.get("max_tokens", max_output_tokens))
        max_completion_tokens = options.get(
            "max_completion_tokens", model_settings.get("max_completion_tokens", max_tokens)
        )

        if model_settings.get("max_completion_tokens"):
            max_tokens = NOT_GIVEN
        else:
            max_completion_tokens = NOT_GIVEN

        # UNSET (vendor-neutral) becomes the OpenAI SDK's own NOT_GIVEN here;
        # only this provider may touch the OpenAI sentinel.
        sdk_response_format = response_format if response_format is not UNSET else NOT_GIVEN
        sdk_tools = tools if tools is not UNSET else NOT_GIVEN

        try:
            response = client.chat.completions.create(
                model=model.model,
                messages=messages,
                max_completion_tokens=max_completion_tokens,
                max_tokens=max_tokens,
                response_format=sdk_response_format,
                tools=sdk_tools
            )
        except OpenAIError as exc:
            self.translate_error(exc)

        log_payload("OpenAI Completion Response", response, show=show)

        message = response.choices[0].message
        return Completion(text=message.content, role=message.role or "assistant", raw=response)

    def translate_error(self, exc: BaseException) -> NoReturn:
        # Most-specific-first chain; never string-match error messages.
        if isinstance(exc, (openai.AuthenticationError, openai.PermissionDeniedError)):
            fatal(
                f"OpenAI rejected the API key (HTTP {exc.status_code}). Update the key with "
                "`smah --configure`, or set OPENAI_API_KEY (or SMAH_OPENAI_API_KEY).",
                ExitCode.AUTH_OR_MODEL
            )
        elif isinstance(exc, openai.NotFoundError):
            fatal(
                f"OpenAI did not recognize the model or endpoint (HTTP {exc.status_code}): "
                f"{exc.message}. Check the model id via --model / --model-query / --model-pipe.",
                ExitCode.PROVIDER
            )
        elif isinstance(exc, openai.RateLimitError):
            fatal(f"OpenAI rate limited the request (HTTP {exc.status_code}). Try again shortly.", ExitCode.PROVIDER)
        elif isinstance(exc, openai.BadRequestError):
            fatal(f"OpenAI rejected the request (HTTP {exc.status_code}): {exc.message}.", ExitCode.PROVIDER)
        elif isinstance(exc, openai.APIStatusError):
            if exc.status_code is not None and 500 <= exc.status_code < 600:
                fatal(f"OpenAI is having trouble (HTTP {exc.status_code}). Try again shortly.", ExitCode.PROVIDER)
            fatal(f"OpenAI API error (HTTP {exc.status_code}): {exc.message}.", ExitCode.PROVIDER)
        elif isinstance(exc, openai.APITimeoutError):
            fatal("OpenAI timed out. Try again, or narrow the request.", ExitCode.PROVIDER)
        elif isinstance(exc, openai.APIConnectionError):
            fatal(f"Can't reach OpenAI ({exc}). Check the network, then retry.", ExitCode.PROVIDER)
        else:
            raise exc
