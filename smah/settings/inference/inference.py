import textwrap
from typing import Optional

import yaml

from .provider import Provider

class Inference:
    CONFIG_VSN: str = "0.0.1"

    @staticmethod
    def config_vsn() -> str:
        """
        Returns the version of the configuration.

        Returns:
            str: The version of the configuration.
        """
        return Inference.CONFIG_VSN

    @staticmethod
    def provider_factory(provider, config_data) -> Optional[Provider]:
        return Provider(provider, config_data)

    def __init__(self, config_data = None):
        config_data = config_data or {}
        self.vsn = config_data.get("vsn", self.config_vsn())
        self.instructions: Optional[str] = config_data.get("instructions")
        self.model_picker = config_data.get("model_picker") or {"default": ["anthropic.claude-haiku-5-5", "openai.gpt-4o-mini"]}
        self.providers: dict[str,Provider] = {}
        providers = config_data.get("providers", {})
        for k, v in providers.items():
             provider = self.provider_factory(k,v)
             if provider:
                self.providers[k] = provider

        self.models = {}
        for k, v in self.providers.items():
            if v.enabled and v.is_configured():
                for mv in v.models:
                    if mv.enabled and mv.is_configured():
                        self.models[f"{k}.{mv.name}"] = mv

    def is_configured(self):
        if not self.providers:
            return False
        for _,v in self.providers.items():
            if v.enabled and not(v.is_configured()):
                return False
        return True

    def to_yaml(self, options = None):
        options = options or {}
        if options.get('prompt'):
            return self.to_prompt_yaml(options=options)
        else:
            return self.to_standard_yaml(options=options)

    def to_prompt_yaml(self, options = None):
        options = options or {}
        models = []
        for _,v in self.models.items():
            models.append(v.to_yaml(options=options))
        o = {}
        if self.instructions:
            o['instructions'] = self.instructions
        o['models'] = models
        return o

    def to_standard_yaml(self, options = None):
        options = options or {}
        providers = {}
        for k,v in self.providers.items():
            if options.get('save') or options.get('disabled') or (v.enabled and v.is_configured()):
                providers[k] = v.to_yaml(options=options)

        o = {
            'vsn': self.config_vsn(),
            'instructions': self.instructions,
            'model_picker': self.model_picker,
            'providers': providers
        }

        if options.get('save'):
            pass
        else:
            o.pop('model_picker')
            o.pop('vsn')
        return o

    def show(self, options = None):
        return yaml.dump(self.to_yaml(options=options), sort_keys=False)
