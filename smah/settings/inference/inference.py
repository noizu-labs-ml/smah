from .provider import Provider

class Inference:
    def __init__(self, config_data):
        config_data = config_data or {}
        self.providers = []
        providers = config_data.get("providers", {})
        for provider, config in providers.items():
            self.providers.append(Provider(provider, config))

    def is_configured(self):
        if not self.providers:
            return False
        return True

    def to_yaml(self, options = None):
        return {}