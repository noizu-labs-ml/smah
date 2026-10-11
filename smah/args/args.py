import argparse
import sys
import logging
import textwrap
from typing import Optional

# Flags whose old forms still work but print a one-line deprecation notice on
# stderr (never stdout). The message text is normative: docs/ux/interface-spec.md §2.
DEPRECATION_NOTICES: dict[str, str] = {
    "-q": "[deprecation] 'smah -q' works, but prefer 'smah ask'. Alias removes in 0.4.",
    "-i": "[deprecation] 'smah -i' works, but prefer 'smah ask --instructions'. Alias removes in 0.4.",
    "--interactive": "[deprecation] --interactive never worked in prior releases. Use 'smah chat'.",
    "--continue": "[deprecation] '--continue' works, but prefer 'smah resume'. Alias removes in 0.4.",
    "--session": "[deprecation] '--session N' works, but prefer 'smah resume N'. Alias removes in 0.4.",
    "--history": "[deprecation] '--history' works, but prefer 'smah history'. Alias removes in 0.4.",
    "--configure": "[deprecation] '--configure' works, but prefer 'smah config'. Alias removes in 0.4.",
    "--openai-api-key": "[deprecation] '--openai-api-key' works, but prefer '--api-key' (applies to the active provider). Removes in 0.4.",
}

# Flags that never worked in any release: parsing one is a usage error carrying
# the exact message from docs/ux/interface-spec.md §2 (argparse adds the
# "smah: error: " prefix; exit code 2).
REMOVED_FLAGS: list[tuple[str, str]] = [
    ("--setup", "--setup never existed in any release. Run 'smah config'."),
    ("--model-picker", "--model-picker was never implemented. Set the picker via 'smah models use <id> --for picker'."),
    ("--model-interactive", "--model-interactive was never implemented. Use 'smah chat --model'."),
    ("--model-review", "--model-review was never implemented; no review stage exists or is planned."),
    ("--model-edit", "--model-edit was never implemented; no edit stage exists or is planned."),
    ("--openai-api-tier", "--openai-api-tier was never implemented. (removed)"),
    ("--openai-api-org", "--openai-api-org was never implemented. (removed)"),
    ("--gui", "no GUI exists in SMAH and none is planned. (removed)"),
]

# Namespace attributes that stored-session args must never overwrite.
RESERVED_ARG_KEYS = {"command", "deprecated"}

CHAT_NOTICE = "chat is not available yet — use 'smah ask' for one-shot queries"


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
        if key in RESERVED_ARG_KEYS:
            continue
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
    __validate(a, parser)
    __normalize(a)
    return a, __get_pipe()

def build_parser() -> argparse.ArgumentParser:
    """
    Assembles the fully populated smah argument parser: global flags, the
    subcommand surface (ask/chat/history/resume/config/models/doctor), the
    deprecated legacy flags, and rejections for flags that never existed.

    Returns:
        ArgumentParser: The parser with all argument groups attached.
    """
    parser = __initialize_argument_parser()
    __add_global_arguments(parser)
    __add_subcommands(parser)
    __add_legacy_arguments(parser)
    __add_removed_flags(parser)
    return parser

def __initialize_argument_parser():
    """
    Initialize the argument parser with basic configurations.

    Returns:
        ArgumentParser: A configured argument parser.
    """
    return argparse.ArgumentParser(
        prog="smah",
        usage="smah [<global flags>] <command> [<args>]",
        description=textwrap.dedent(
            """\
            SMAH — the sysadmin helper. SMAH does not run commands: it drafts
            them, explains them, and hands them off for you to review and run.
            """
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent(
            """\
            exit codes:
              0 ok · 2 usage · 3 config · 4 auth or no models · 5 provider

            Data & config live under ~/.smah/ — see 'smah doctor' for exact paths.

            examples:
              # summarize the tail of an auth log (pipe processing)
              tail -n 500 /var/log/auth.log | smah ask "Analyze these access log entries and output a markdown summary with tables of suspicious ips and events."

              # process piped output using a saved instruction file
              systemctl status | smah ask --instructions ~/SCAN-SYSTEMCTL.md

              # ask a plain question
              smah ask "How do I setup a web proxy on this server that I can connect to via my remote desktop?"

              # resume your most recent conversation
              smah resume --last
            """
        )
    )

def __add_global_arguments(parser: argparse.ArgumentParser) -> None:
    """
    Add flags that apply to every command.

    Args:
        parser (ArgumentParser): The argument parser to which global arguments are added.
    """
    group = parser.add_argument_group("global flags")
    group.add_argument('-c', '--config', type=str, help='Config file (default: ~/.smah/config.yaml)')
    group.add_argument('--database', type=str, help='Session database (default: ~/.smah/smah.db)')
    group.add_argument('-v', '--verbose', action='count', default=0, help='Repeatable: -v plans, -vv payloads, -vvv settings dump (all output to stderr)')
    group.add_argument('--model', type=str, help='Override the model for this run (fully qualified, e.g. openai.gpt-4o-mini)')
    group.add_argument('--model-query', type=str, help='Model used for the query planning stage')
    group.add_argument('--model-pipe', type=str, help='Model used for the pipe planning stage')
    group.add_argument('--api-key', type=str, help='API key for the active provider (else env/config)')
    group.add_argument('--rich', action=argparse.BooleanOptionalAction, help='Force formatting on/off (default: TTY detection)', default=None)

def __add_subcommands(parser: argparse.ArgumentParser) -> None:
    """
    Attach the subcommand surface via argparse subparsers.

    Args:
        parser (ArgumentParser): The argument parser to which subcommands are added.
    """
    subparsers = parser.add_subparsers(
        dest="command",
        title="commands",
        description="Run 'smah <command> --help' for per-command options and examples.",
        metavar="<command>",
    )

    ask = subparsers.add_parser(
        "ask",
        help='Ask a one-shot question (argument or piped stdin)',
        description=textwrap.dedent(
            """\
            Ask a one-shot question. The query is a positional argument; when
            stdin is piped, stdin is the data and the query is the instruction
            (pipe mode). Suggested commands are printed with explanations —
            never executed.
            """
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent(
            """\
            examples:
              smah ask "rotate nginx logs and compress last week's"
              tail -n 500 /var/log/auth.log | smah ask "summarize suspicious ips as a markdown table"
              systemctl status | smah ask --instructions ~/SCAN-SYSTEMCTL.md
            """
        )
    )
    ask.add_argument('ask_query', metavar='query', nargs='?', help='The question or task to process')
    ask.add_argument('-i', '--instructions', dest='ask_instructions', metavar='FILE', help="File whose contents are used as the query")

    subparsers.add_parser(
        "chat",
        help='Start an interactive session (NOT IMPLEMENTED yet)',
        description="Interactive chat mode. Not implemented yet: prints a notice and exits 2.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )

    history = subparsers.add_parser(
        "history",
        help='List recent sessions',
        description="List recent sessions (newest first). Use 'smah resume' to reopen one.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent(
            """\
            examples:
              smah history
              smah history --limit 50
            """
        )
    )
    history.add_argument('--limit', dest='history_limit', type=int, default=20, help='Number of sessions to list (default: 20)')

    resume = subparsers.add_parser(
        "resume",
        help='Resume a saved session (default: most recent)',
        description="Resume a saved session by id, or the most recent one when no id is given.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent(
            """\
            examples:
              smah resume --last
              smah resume 142
            """
        )
    )
    resume.add_argument('resume_id', metavar='id', nargs='?', type=int, help='Session id to resume (omit for the most recent session)')
    resume.add_argument('--last', dest='resume_last', action='store_true', help='Resume the most recent session (same as omitting the id)')

    config = subparsers.add_parser(
        "config",
        help="Run setup, or show/edit the config",
        description=textwrap.dedent(
            """\
            Without a subcommand: run the interactive setup wizard.
            'show' prints the current config with secrets masked;
            'path' prints the config file location; 'edit' re-runs the wizard;
            'set' writes one dotted key (e.g. user.name).
            """
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent(
            """\
            examples:
              smah config
              smah config show
              smah config set user.name "Ines"
            """
        )
    )
    config_sub = config.add_subparsers(dest="config_command", metavar="<subcommand>")
    config_sub.add_parser("show", help="Print the current config with secrets masked")
    config_sub.add_parser("path", help="Print the config file path")
    config_sub.add_parser("edit", help="Re-run the interactive setup wizard")
    config_set = config_sub.add_parser("set", help="Set one config value")
    config_set.add_argument('config_set_key', metavar='key', help='Dotted config key, e.g. user.name')
    config_set.add_argument('config_set_value', metavar='value', help='Value to store (bool/int coerced from strings)')

    models = subparsers.add_parser(
        "models",
        help='List configured models',
        description="List configured/enabled models and which are the defaults.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent(
            """\
            examples:
              smah models
              smah models list
            """
        )
    )
    models_sub = models.add_subparsers(dest="models_command", metavar="<subcommand>")
    models_sub.add_parser("list", help="List models (default when no subcommand is given)")

    doctor = subparsers.add_parser(
        "doctor",
        help='Check the installation and report problems',
        description=textwrap.dedent(
            """\
            Diagnostics: config validity, database/migration state, key
            presence (never the value), enabled models and defaults, and the
            resolved planner model per stage. Reads config only — no network
            calls. Exit 0 when healthy, 3 config, 4 auth/models.
            """
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent(
            """\
            examples:
              smah doctor
              smah doctor --json
            """
        )
    )
    doctor.add_argument('--json', dest='doctor_json', action='store_true', help='Emit machine-readable results on stdout')
    doctor.add_argument('--offline', dest='doctor_offline', action='store_true', help='Skip provider reachability checks (none are performed anyway in this version)')

def __add_legacy_arguments(parser: argparse.ArgumentParser) -> None:
    """
    Add the deprecated legacy flags. Each still parses and dispatches to its
    new equivalent after a one-line deprecation notice on stderr.

    Args:
        parser (ArgumentParser): The argument parser to which legacy arguments are added.
    """
    group = parser.add_argument_group("legacy flags (deprecated — still accepted, notice on stderr)")
    group.add_argument('-q', '--query', type=str, help='DEPRECATED: prefer \'smah ask "<query>"\'')
    group.add_argument('-i', '--instructions', type=str, help="DEPRECATED: prefer 'smah ask --instructions'")
    group.add_argument('--continue', dest="resume", action=argparse.BooleanOptionalAction, help='DEPRECATED: prefer \'smah resume\' — resume the most recent conversation', default=None)
    group.add_argument('--session', type=int, help='DEPRECATED: prefer \'smah resume N\' — resume a specific conversation by id')
    group.add_argument('--history', action=argparse.BooleanOptionalAction, help='DEPRECATED: prefer \'smah history\' — pick one of the 10 most recent conversations to resume', default=None)
    group.add_argument('--interactive', action=argparse.BooleanOptionalAction, help='DEPRECATED (NOT IMPLEMENTED): use \'smah chat\'', default=None)
    group.add_argument('--configure', action=argparse.BooleanOptionalAction, help="DEPRECATED: prefer 'smah config' — run the interactive setup wizard", default=None)
    group.add_argument('--openai-api-key', type=str, help="DEPRECATED: prefer '--api-key' (applies to the active provider)")

def __add_removed_flags(parser: argparse.ArgumentParser) -> None:
    """
    Register flags that never worked in any release so they fail with the
    exact message from the interface spec instead of a generic argparse error.

    Args:
        parser (ArgumentParser): The argument parser to which the rejections are added.
    """
    class _Rejected(argparse.Action):
        def __init__(self, option_strings, dest, message=None, **kwargs):
            super().__init__(option_strings, dest, **kwargs)
            self.message = message

        def __call__(self, parser, namespace, values, option_string=None):
            parser.error(self.message)

    for flag, message in REMOVED_FLAGS:
        parser.add_argument(
            flag,
            action=_Rejected,
            message=message,
            nargs="?",
            help=argparse.SUPPRESS,
        )

def __validate(args: argparse.Namespace, parser: argparse.ArgumentParser) -> None:
    """
    Cross-flag validation that argparse alone cannot express.

    Args:
        args (Namespace): Parsed arguments (pre-normalization).
        parser (ArgumentParser): The parser, used for usage-error reporting (exit 2).
    """
    if args.command == "resume" and args.resume_last and args.resume_id is not None:
        parser.error("'resume' takes either a session id or --last, not both.")

def __normalize(args: argparse.Namespace) -> None:
    """
    Normalizes the parsed namespace:

    - records which deprecated legacy flags were used (for stderr notices),
    - resolves --rich default via TTY detection (stdout),
    - folds --api-key into the openai_api_key field the provider layer reads,
    - turns the sentinel legacy booleans into plain False.

    Args:
        args (Namespace): Parsed arguments, modified in place.
    """
    used: list[str] = []
    if args.query is not None:
        used.append("-q")
    if args.instructions is not None:
        used.append("-i")
    for dest, flag in (
        ("resume", "--continue"),
        ("history", "--history"),
        ("interactive", "--interactive"),
        ("configure", "--configure"),
    ):
        if getattr(args, dest) is True:
            used.append(flag)
    if args.session is not None:
        used.append("--session")

    if args.api_key is not None:
        args.openai_api_key = args.api_key
    elif args.openai_api_key is not None:
        used.append("--openai-api-key")

    args.deprecated = used

    for dest in ("resume", "history", "interactive", "configure"):
        setattr(args, dest, bool(getattr(args, dest)))

    if args.rich is None:
        args.rich = sys.stdout.isatty()

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
