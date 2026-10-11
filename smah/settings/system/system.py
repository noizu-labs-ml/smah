import os
import textwrap
from typing import Optional
from .stats import CpuStats, MemoryStats, DiskStats
from .operating_system import OperatingSystem

class System:
    """
    Represents the system configuration and status.

    Attributes:
        CONFIG_VSN (str): The version of the YAML format config section.
        disk (DiskStats): Disk statistics.
        cpu (CpuStats): CPU statistics.
        memory (MemoryStats): Memory statistics.
        operating_system (OperatingSystem): Operating System Details
        vsn (str): Version string.
    """
    CONFIG_VSN: str = "0.0.1"

    @staticmethod
    def config_vsn() -> str:
        """
        Returns the version of the configuration.

        Returns:
            str: The version of the configuration.
        """
        return System.CONFIG_VSN

    @staticmethod
    def version_supported(version) -> bool:
        """
        Checks if the given version is supported.

        Args:
            version (str): The version to check.

        Returns:
            bool: True if the version is supported, False otherwise.
        """
        if version is None:
            return False
        else:
            return version <= System.config_vsn()

    def __init__(self, config_data = None):
        """
        Initializes the System instance with the given configuration data.

        Args:
            config_data (dict): Configuration data for the system.
        """
        config_data = config_data or {}

        self.shell: str = os.getenv('SHELL', 'bash')
        self.disk: DiskStats = DiskStats()
        self.cpu: CpuStats = CpuStats()
        self.memory: MemoryStats = MemoryStats()
        self.operating_system: OperatingSystem = OperatingSystem(config_data.get("operating_system"))
        self.vsn: Optional[str] = config_data.get("vsn")

    def is_configured(self):
        """
        Checks if the system is configured.

        Returns:
            bool: True if the system is configured, False otherwise.
        """
        if not self.operating_system.is_configured():
            return False
        return True

    def to_yaml(self, options = None):
        """
        Converts the system configuration to a YAML-compatible dictionary.

        Returns:
            dict: The system configuration in YAML format.
        """
        options = options or {}
        if options.get("stats"):
            return {
                "vsn": self.config_vsn(),
                "shell": self.shell,
                "operating_system": self.operating_system.to_yaml(options=options) if self.operating_system else None,
                "cpu": self.cpu.readings(),
                "memory": self.memory.readings(),
                "disk": self.disk.readings()
            }
        else:
            return {
                "vsn": self.config_vsn(),
                "shell": self.shell,
                "operating_system": self.operating_system.to_yaml(options=options) if self.operating_system else None,
            }

    def show(self, options=None):
        options = options or {}
        o = self.operating_system.show(options=options) if self.operating_system else None

        if options.get("stats"):
            template = textwrap.dedent(
                """
                **Shell:**
                
                {shell}
                
                **Operating System:**
                
                {operating_system}
                
                **Cpu:**
                
                {cpu}
                
                **Memory:**
                
                {memory}
                
                **Disk:**
                
                {disk}
                """
            ).strip().format(
                shell=self.shell,
                operating_system=o,
                cpu=self.cpu.show(options=options) if self.cpu else None,
                memory=self.memory.show(options=options) if self.memory else None,
                disk=self.disk.show(options=options) if self.disk else None
            )
        else:
            template = textwrap.dedent(
                """
                **Shell:**
                
                {shell}
                
                **Operating System:**
                
                {operating_system}
                """
            ).strip().format(
                shell=self.shell,
                operating_system=o
            )
        return template

    # def terminal_configure(self, console):
    #     """
    #     Configures the system details interactively via the terminal.
    #
    #     Args:
    #         console (Console): The console instance for interactive prompts.
    #     """
    #
    #     self.errors = []
    #     auto = Confirm.ask("Would you like me to automatically setup your system details", default=True)
    #     os_details = self.os_details()
    #     if auto:
    #         self.type = os_details["type"] if "type" in os_details else None
    #         self.name = os_details["name"] if "name" in os_details else None
    #         self.version = os_details["version"] if "version" in os_details else None
    #         self.release = os_details["release"] if "release" in os_details else None
    #         self.info = self.prompt_info(self.type)
    #     else:
    #         self.type = self.prompt_type(console, os_details["type"] if "type" in os_details else None)
    #         self.name = self.prompt_name(console, os_details["name"] if "name" in os_details else None)
    #         self.version = self.prompt_version(console, os_details["version"] if "version" in os_details else None)
    #         self.release = self.prompt_release(console, os_details["release"] if "release" in os_details else None)
    #         auto = Confirm.ask("Would you like me to automatically load your OS specific details", default=True)
    #         if auto:
    #             self.info = self.prompt_info(self.type)
    #         else:
    #             self.info = None
    #
    #         console.print("...")
    #
    #     if not self.errors:
    #         self.errors = None
    #
    #     self.configured = None
    #     self.configured = self.is_configured()
    #
    # def to_yaml(self, options = {}):
    #     """
    #     Converts the system configuration to a YAML-compatible dictionary.
    #
    #     Args:
    #         options (dict): Options for YAML conversion.
    #
    #     Returns:
    #         dict: The system configuration in YAML format.
    #     """
    #
    #     system = {
    #         "vsn": self.vsn if self.vsn is not None else System.YAML_VERSION,
    #         "type": self.type,
    #         "name": self.name,
    #         "version": self.version,
    #         "release": self.release,
    #         "info": self.info.to_yaml(options = options) if self.info is not None else None,
    #     }
    #
    #
    #
    #
    #     if "stats" in options and options["stats"]:
    #         return {
    #             "vsn": System.YAML_VERSION,
    #             "os": system,
    #             "cpu": self.cpu.readings(),
    #             "memory": self.memory.readings(),
    #             "disk": self.disk.readings(),
    #         }
    #     else:
    #         return {
    #             "vsn": System.YAML_VERSION,
    #             "os": system,
    #         }
    #
    # def prompt_type(self, console, type):
    #     """
    #     Prompts the user to input the OS type.
    #
    #     Args:
    #         console (Console): The console instance for interactive prompts.
    #         type (str): The default OS type.
    #
    #     Returns:
    #         str: The selected OS type.
    #     """
    #
    #     default_type = None
    #     default_bsd_type = None
    #     if type is not None:
    #         if type == "Linux":
    #             default_type = "1"
    #         elif type == "Darwin":
    #             default_type = "2"
    #         elif type == "Windows":
    #             default_type = "3"
    #         elif type in ["FreeBSD", "OpenBSD", "NetBSD"]:
    #             default_type = "4"
    #             if type == "FreeBSD":
    #                 default_bsd_type = "1"
    #             elif type == "OpenBSD":
    #                 default_bsd_type = "2"
    #             elif type == "NetBSD":
    #                 default_bsd_type = "3"
    #
    #
    #     os_type_message = textwrap.dedent(
    #         """
    #         [bold green]What is your os type?[/bold green]
    #         1 - Linux
    #         2 - MacOS (Darwin)
    #         3 - Windows
    #         4 - BSD
    #         """
    #     )
    #     types = {
    #         "1": "Linux",
    #         "2": "Darwin",
    #         "3": "Windows",
    #         "4": "BSD",
    #     }
    #     bsd_type_message = textwrap.dedent(
    #         """
    #         [bold green]What is your BSD flavor?[/bold green]
    #         1 - FreeBSD
    #         2 - OpenBSD
    #         3 - NetBSD
    #         """
    #     )
    #     bsd_types = {
    #         "1": "FreeBSD",
    #         "2": "OpenBSD",
    #         "3": "NetBSD",
    #     }
    #
    #
    #     if self.type is None:
    #         type = Prompt.ask(os_type_message, choices=["1", "2", "3", "4"], default = default_type)
    #         v = types[type]
    #         if v == "BSD":
    #             bsd_type = Prompt.ask(bsd_type_message, choices=["1", "2", "3"], default=default_bsd_type)
    #             return bsd_types[bsd_type]
    #         else:
    #             return v
    #     else:
    #         type_name = "MacOs (Darwin)" if self.type == "Darwin" else self.type
    #
    #         console.print(f"OS Type: {type_name}")
    #         if Confirm.ask("correct?", default=False):
    #             return self.type
    #         else:
    #             type = Prompt.ask(os_type_message, choices=["1", "2", "3", "4"], default = default_type)
    #             v = types[type]
    #             if v == "BSD":
    #                 bsd_type = Prompt.ask(bsd_type_message, choices=["1", "2", "3"], default=default_bsd_type)
    #                 return bsd_types[bsd_type]
    #             else:
    #                 return v
    #
    # def prompt_release(self, console, release):
    #     """
    #     Prompts the user to input the OS release.
    #
    #     Args:
    #         console (Console): The console instance for interactive prompts.
    #         release (str): The default OS release.
    #
    #     Returns:
    #         str: The selected OS release.
    #     """
    #
    #     message = "[bold green]What is your Os Release?[/bold green]"
    #     if self.release is None:
    #         return Prompt.ask(message, default=release)
    #     else:
    #         console.print(f"Release: {self.release}")
    #         if Confirm.ask("correct?", default=False):
    #             return self.release
    #         else:
    #             return Prompt.ask(message, default=release)
    #
    # def prompt_version(self, console, version):
    #     """
    #     Prompts the user to input the OS version.
    #
    #     Args:
    #         console (Console): The console instance for interactive prompts.
    #         version (str): The default OS version.
    #
    #     Returns:
    #         str: The selected OS version.
    #     """
    #
    #     message = "[bold green]What is your OS Version[/bold green]"
    #     if self.version is None:
    #         return Prompt.ask(message, default=version)
    #     else:
    #         console.print(f"OS Version: {self.version}")
    #         if Confirm.ask("correct?", default=False):
    #             return self.version
    #         else:
    #             return Prompt.ask(message, default=version)
    #
    # def prompt_name(self, console, name):
    #     """
    #     Prompts the user to input the OS name.
    #
    #     Args:
    #         console (Console): The console instance for interactive prompts.
    #         name (str): The default OS name.
    #
    #     Returns:
    #         str: The selected OS name.
    #     """
    #
    #     message = "[bold green]What is your OS Name[/bold green]"
    #     if self.name is None:
    #         return Prompt.ask(message, default=name)
    #     else:
    #         console.print(f"OS Name: {self.name}")
    #         if Confirm.ask("correct?", default=False):
    #             return self.name
    #         else:
    #             return Prompt.ask(message, default=name)
    #
    # def prompt_info(self, type):
    #     """
    #     Prompts the user to input OS-specific details.
    #
    #     Args:
    #         type (str): The OS type.
    #
    #     Returns:
    #         object: The OS-specific details.
    #     """
    #
    #     if type == "Linux":
    #         return System.LinuxDetails(None, fetch=True)
    #     elif type == "Darwin":
    #         return System.DarwinDetails(None, fetch=True)
    #     elif type == "Windows":
    #         return System.WindowsDetails(None, fetch=True)
    #     elif type in ["FreeBSD", "OpenBSD", "NetBSD"]:
    #         return System.BSDDetails(None, fetch=True)
    #     else:
    #         return None
    #


