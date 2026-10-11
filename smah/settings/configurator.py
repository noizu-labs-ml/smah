from .inference import inference_terminal_configurator
from .settings import Settings
from .user import User, user_terminal_configurator
from .system import System, system_terminal_configurator
from smah.console import std_console, err_console
from rich.markdown import Markdown

def configurator(settings: Settings) -> Settings:
    """
    Interactive Setup
    """
    return terminal_configurator(settings)

def terminal_configurator(settings: Settings) -> Settings:
    std_console.print(Markdown("# Configure"))

    settings.user = user_terminal_configurator(settings.user)
    settings.system = system_terminal_configurator(settings.system)
    settings.inference = inference_terminal_configurator(settings.inference)

    settings.save()
    return settings