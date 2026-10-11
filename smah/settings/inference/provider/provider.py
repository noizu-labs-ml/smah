import os

import yaml

from .model import Model

class Provider:
    CONFIG_VSN: str = "0.0.1"

    @staticmethod
    def config_vsn() -> str:
        """
        Returns the version of the configuration.

        Returns:
            str: The version of the configuration.
        """
        return Provider.CONFIG_VSN


    def __init__(self, identifier, config_data = None):
        config_data = config_data or {}
        self.identifier = identifier
        self.vsn = config_data.get("vsn")
        self.name = config_data.get("name")
        self.description = config_data.get("description")
        self.enabled = config_data.get("enabled")
        self.settings = config_data.get("settings") or {}
        self.models: list[Model] = []
        for model in config_data.get("models", []):
            self.models.append(Model(self.identifier, model))

    def is_configured(self):
        if not self.name:
            return False
        if self.enabled is None:
            return False
        for model in self.models:
            if model.enabled and not(model.is_configured()):
                return False
        return True

    def api_key(self, args):
        if self.identifier == "openai":
            k = args.openai_api_key or self.settings.get("api_key")
        else:
            k = self.settings.get("api_key")
        if k:
            if k.startswith("$"):
                k = k.lstrip("$").lstrip("{").rstrip("}")
                return os.environ.get(k)
            else:
                return k
        env_prefix = self.identifier.upper()
        return os.environ.get(f"SMAH_{env_prefix}_API_KEY") or os.environ.get(f"{env_prefix}_API_KEY")

    def to_yaml(self, options = None):
        options = options or {}
        models: list[dict] = []
        for model in self.models:
            if options.get("disabled") or options.get("save") or (model.enabled and model.is_configured()):
                models.append(model.to_yaml(options=options))

        o = {
            'name': self.name,
            'description': self.description,
            'enabled': self.enabled,
            'vsn': self.config_vsn(),
            'settings': self.settings,
            'models': models,
        }

        if options.get("save"):
            pass
        elif options.get("disabled"):
            o.pop('vsn')
            o.pop('settings')
        else:
            o.pop('vsn')
            o.pop('settings')
            o.pop('enabled')
        return o

    def show(self, options = None):
        y = self.to_yaml(options=options)
        return yaml.dump(y, sort_keys=False)

