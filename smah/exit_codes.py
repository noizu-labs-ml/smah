"""
smah.exit_codes

Single source of truth for the exit codes the SMAH command line tools use.

Code  Meaning
----  -------------------------------------------------------------
0     success
1     unexpected internal error
2     usage error (bad arguments, no query and no stdin)
3     configuration error (missing or corrupt config file)
4     auth or no usable model (no API key, zero enabled models, provider not wired up)
5     provider or API error (4xx/5xx responses, network failures)
"""
import sys
from enum import IntEnum

from smah.console import err_console


class ExitCode(IntEnum):
    OK = 0
    UNEXPECTED_ERROR = 1
    USAGE = 2
    CONFIG = 3
    AUTH_OR_MODEL = 4
    PROVIDER = 5


def fatal(message: str, code: ExitCode = ExitCode.UNEXPECTED_ERROR) -> None:
    """
    Print a readable error message to stderr and exit with the given exit code.

    Args:
        message (str): Human readable description of what went wrong.
        code (ExitCode): Exit code to terminate with.
    """
    err_console.print(f"[bold red]smah:[/bold red] {message}")
    sys.exit(int(code))
