import argparse
import sys
import logging
import textwrap
from typing import Optional

def merge_args(args: argparse.Namespace, config: dict) -> argparse.Namespace:
    """
    Merges parsed command-line arguments with configuration settings.

    Args:
        args (Namespace): Parsed command-line arguments.
        config (dict): Configuration settings.

    Returns:
        Namespace: Merged arguments and settings.
    """
    for key, value in config.items():
        if key in args and getattr(args, key) is None:
            setattr(args, key, value)
    return args

def extract_args(argv: Optional[list[str]] = None) -> tuple[argparse.Namespace, str | None]:
    """
    Parses and extracts command line arguments for the SMAH CLI tool.

    Args:
        argv (Optional[list[str]]): Argument list to parse instead of sys.argv
            (used by tests; defaults to the process arguments).

    Returns:
        args (Namespace): Parsed arguments and options.
        pipe (str or None): Content read from standard input if available.
    """
    parser = build_parser()
    a = parser.parse_args(argv)
    return a, __get_pipe()

def build_parser() -> argparse.ArgumentParser:
    """
    Assembles the fully populated smah argument parser.

    Returns:
        ArgumentParser: The parser with all argument groups attached.
    """
    parser = __initialize_argument_parser()
    __add_input_arguments(parser)
    __add_session_arguments(parser)
    __add_model_arguments(parser)
    __add_output_arguments(parser)
    __add_configuration_arguments(parser)
    __add_diagnostic_arguments(parser)
    return parser

def __initialize_argument_parser():
    """
    Initialize the argument parser with basic configurations.

    Returns:
        ArgumentParser: A configured argument parser.
    """
    return argparse.ArgumentParser(
        prog="smah",
        description=(
            "SMAH: a system-aware command line assistant. Ask a question or pipe input "
            "through an LLM that knows your OS and preferences.\n"
            "SMAH prints suggested commands with explanations; it never executes them."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent(
            """\
            examples:
              # summarize the tail of an auth log (pipe processing)
              tail -n 500 /var/log/auth.log | smah -q "Analyze these access log entries and output a markdown summary with tables of suspicious ips and events."

              # process piped output using a saved instruction file
              systemctl status | smah -i ~/SCAN-SYSTEMCTL.md

              # ask a plain question
              smah -q "How do I setup a web proxy on this server that I can connect to via my remote desktop to access the web?"

              # resume your most recent conversation
              smah --continue

            input piping:
              When stdin is not a terminal its full contents are captured and sent along
              with your query (a query passed with -q or -i is still required).
            """
        )
    )

def __add_input_arguments(parser: argparse.ArgumentParser) -> None:
    """
    Add input arguments to the parser.

    Args:
        parser (ArgumentParser): The argument parser to which input arguments are added.
    """
    group = parser.add_argument_group("input")
    group.add_argument('-q', '--query', type=str, help='The question or task to process (required unless resuming a session)')
    group.add_argument('-i', '--instructions', type=str, help='Path to a file whose contents are used as the query')

def __add_session_arguments(parser: argparse.ArgumentParser) -> None:
    """
    Add conversation/session arguments to the parser.

    Args:
        parser (ArgumentParser): The argument parser to which session arguments are added.
    """
    group = parser.add_argument_group("sessions")
    group.add_argument('--continue', dest="resume", action=argparse.BooleanOptionalAction, help='Resume the most recent conversation', default=False)
    group.add_argument('--session', type=int, help='Resume a specific conversation by id')
    group.add_argument('--history', action=argparse.BooleanOptionalAction, help='Pick one of the 10 most recent conversations to resume', default=False)
    group.add_argument('--interactive', action=argparse.BooleanOptionalAction, help='Interactive chat mode (NOT IMPLEMENTED: exits with an error)', default=False)

def __add_model_arguments(parser: argparse.ArgumentParser) -> None:
    """
    Add model-selection arguments to the parser.

    Args:
        parser (ArgumentParser): The argument parser to which model arguments are added.
    """
    group = parser.add_argument_group("models")
    group.add_argument('--model', type=str, help='Override the model used to answer (e.g. openai.gpt-4o-mini)')
    group.add_argument('--model-query', type=str, help='Model used for the query planning stage')
    group.add_argument('--model-pipe', type=str, help='Model used for the pipe planning stage')
    group.add_argument('--openai-api-key', type=str, help='OpenAI API key (prefer the SMAH_OPENAI_API_KEY / OPENAI_API_KEY env vars or the config file)')

def __add_output_arguments(parser: argparse.ArgumentParser) -> None:
    """
    Add output-formatting arguments to the parser.

    Args:
        parser (ArgumentParser): The argument parser to which output arguments are added.
    """
    group = parser.add_argument_group("output")
    group.add_argument('--rich', action=argparse.BooleanOptionalAction, help='Render responses as rich Markdown instead of plain text', default=True)

def __add_configuration_arguments(parser: argparse.ArgumentParser) -> None:
    """
    Add configuration arguments to the parser.

    Args:
        parser (ArgumentParser): The argument parser to which configuration arguments are added.
    """
    group = parser.add_argument_group("configuration")
    group.add_argument('-c', '--config', type=str, help='Path to an alternative config file (default: ~/.smah/config.yaml)')
    group.add_argument('--database', type=str, help='Path to the sqlite history database (default: ~/.smah/smah.db)')
    group.add_argument('--configure', action=argparse.BooleanOptionalAction, help='Run the interactive setup wizard', default=False)

def __add_diagnostic_arguments(parser: argparse.ArgumentParser) -> None:
    """
    Add diagnostic arguments to the parser.

    Args:
        parser (ArgumentParser): The argument parser to which diagnostic arguments are added.
    """
    group = parser.add_argument_group("diagnostics")
    group.add_argument('-v', '--verbose', action='count', default=0, help='Increase verbosity (repeatable: -v, -vv, -vvv). -v shows plans and payloads, -vvv also dumps settings')

def __get_pipe():
    """
    Reads data from standard input if present and available.

    Returns:
        str or None: The content available from standard input; otherwise, None if input is not a TTY.
    """
    if sys.stdin.isatty():
        return None
    else:
        return sys.stdin.read()

if __name__ == "__main__":
    logging.basicConfig(level=logging.DEBUG)
    args, pipe = extract_args()
    logging.debug("Parsed arguments: %s", args)
    logging.debug("Pipe content: %s", pipe)
