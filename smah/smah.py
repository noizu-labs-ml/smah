"""
smah.py

This module serves as the entry point for the SMAH Command Line Tool. It is responsible for parsing
command line arguments, initializing settings, and coordinating execution based on user inputs.
The module utilizes the `argparse` library for command-line argument parsing and `rich` for enhanced
console outputs.

Dispatch order: subcommands first (ask/chat/history/resume/config/models/doctor), then the
deprecated legacy flags (each prints a one-line notice on stderr and runs its new equivalent),
then bare invocation (concise usage, exit 2). stdout carries content only; every notice, hint,
panel, and deprecation lands on stderr (see docs/ux/interface-spec.md §5).

Classes and Functions:
- pick_session: interactive session picker (legacy --history behavior).
- resume_session: restores and continues a saved session.
- init_database: applies pending database migrations.
- main: the primary entry point.

Dependencies:
- `os`, `argparse`, `textwrap`, and `yaml` for OS operations, argument parsing, text manipulation, and YAML operations.
- `rich` for enhanced console outputs, utilized throughout the module for logging and interactive feedback.
- `smah.settings.Settings` and `smah.runner.Runner`, which are the other core modules facilitating configuration handling and executing user queries.

Note:
Ensure the environment is set up with the necessary dependencies before executing this script.
"""

import logging
import sys
import textwrap
from types import SimpleNamespace
from typing import Optional

import yaml
from openai import OpenAIError
from rich.traceback import Traceback

import smah.args
import smah.logs
from smah.args.args import DEPRECATION_NOTICES
from smah.commands import CHAT_NOTICE, config_cmd, doctor as doctor_cmd, history_cmd, models_cmd
from smah.console import std_console, err_console
from smah.database import Database, Migration
from smah.exit_codes import ExitCode
from smah.runner import Runner
from smah.settings import Settings, configurator

pick_session = history_cmd.pick_session


def resume_session(args, session: Optional[int] = None):
    """
    Resumes a conversation from the database (the most recent one when no id
    is given).

    Args:
        args (argparse.Namespace): The parsed command-line arguments.
        session (Optional[int]): Session id to resume, or None for the most recent.
    """
    db = Database(args)
    if session:
        session = db.session(session)
    else:
        session = db.last_session()

    if session:
        args = smah.args.merge_args(args, session['args'])
        settings = __load_settings(args)

        # If settings are not configured, ask user to provide necessary information
        if not settings.is_configured() or args.configure:
            settings = configurator(settings)
            settings.log(print=True, format=True)
        else:
            settings.log(print=(args.verbose >= 3), format=True)
        runner = Runner(args, settings)
        runner.resume(id=session['id'], title=session['title'], plan=session['plan'], pipe=session['pipe'], messages=session['messages'])
    else:
        recent = __recent_session_ids(db)
        err_console.print(
            f"[bold red]smah:[/bold red] no session found ({__resume_target(args)}). "
            f"Recent sessions: {recent}"
        )
        sys.exit(ExitCode.USAGE)


def __resume_target(args) -> str:
    if getattr(args, "resume_id", None) is not None:
        return f"id {args.resume_id}"
    if getattr(args, "session", None) is not None:
        return f"id {args.session}"
    return "most recent"


def __recent_session_ids(db) -> str:
    try:
        ids = [str(s["id"]) for s in db.history(5)]
    except Exception:
        ids = []
    return ", ".join(ids) if ids else "none"


def init_database(args):
    """
    Initializes the database connection.

    Args:
        args (argparse.Namespace): The parsed command-line arguments.
    """
    try:
        db = Database(args)
        outcome, outcome_details = Migration.migrate(db, SimpleNamespace(silent=True, count=None, to=None), silent=True, exit_on_finish=False)

        if outcome == "success":
            logging.warning(f"[DB INIT (migrations applied)]: \"{outcome_details}\"")
        elif outcome == "error":
            error_type, error_details = outcome_details
            logging.error(f"[DB INIT ({error_type})]: \"{error_details}\"")
        else:
            # Ignore no migrations were pending.
            pass
    except Exception as e:
        err_console.print(f"[bold red]smah:[/bold red] failed to initialize the smah database: {e}")
        if getattr(args, "verbose", 0) >= 1:
            err_console.print(Traceback.from_exception(type(e), e, e.__traceback__))
        else:
            err_console.print("Run with -v for a full traceback.")
        sys.exit(ExitCode.UNEXPECTED_ERROR)


def main():
    """
    The primary function that sets up application configuration and executes user-specified queries.

    This function configures logging, parses command-line arguments, initializes settings, and
    coordinates the execution based on user inputs: subcommands first, then deprecated legacy
    flags (with a stderr notice), then bare invocation.

    Raises:
        SystemExit: With an ExitCode describing the failure for any error encountered during execution.
    """
    # Configure logging
    smah.logs.configure()

    args = None
    try:
        args, pipe = smah.args.extract_args()

        __print_deprecations(args)

        command = getattr(args, "command", None)

        if command == "chat":
            __chat_unavailable()
        elif command == "doctor":
            sys.exit(doctor_cmd.run_doctor(args))
        elif command == "models":
            models_cmd.list_models(args)
            return
        elif command == "config":
            __run_config(args)
            return
        elif command == "history":
            history_cmd.list_sessions(args, args.history_limit)
            return
        elif command == "resume":
            init_database(args)
            if args.resume_id is not None:
                resume_session(args, session=args.resume_id)
            else:
                resume_session(args)
            return
        elif command == "ask":
            init_database(args)
            __run_ask(args, pipe, query=args.ask_query, instructions=args.ask_instructions)
            return

        # Deprecated legacy forms: notice already printed; run the new equivalent.
        if args.resume:
            init_database(args)
            resume_session(args)
            return
        if args.session:
            init_database(args)
            resume_session(args, session=args.session)
            return
        if args.history:
            init_database(args)
            session = pick_session(args)
            resume_session(args, session=session)
            return
        if args.interactive:
            __chat_unavailable()

        if args.configure:
            __config_wizard(args)
            return

        query = __with_query(args)

        if not query:
            if pipe:
                err_console.print(
                    "[bold red]smah:[/bold red] input was piped in but no query was given; "
                    "add a query (e.g. smah ask \"summarize this\") to say what to do with it."
                )
            else:
                err_console.print(__usage(), markup=False, highlight=False)
                err_console.print(f"[bold red]smah:[/bold red] {CHAT_NOTICE}")
            sys.exit(ExitCode.USAGE)

        init_database(args)
        __run_query(args, pipe, query)
    except SystemExit:
        raise
    except KeyboardInterrupt:
        err_console.print("[bold red]smah:[/bold red] interrupted.")
        sys.exit(ExitCode.UNEXPECTED_ERROR)
    except yaml.YAMLError as e:
        __fatal(f"configuration file is corrupt: {e}", e, args, ExitCode.CONFIG)
    except OpenAIError as e:
        __fatal(f"OpenAI API error: {e}", e, args, ExitCode.PROVIDER)
    except Exception as e:
        __fatal(f"unexpected error: {e}", e, args, ExitCode.UNEXPECTED_ERROR)


def __print_deprecations(args) -> None:
    """
    Prints one deprecation notice per used legacy flag, to stderr only —
    piped stdout must stay clean. Exact message text: interface spec §2.
    """
    for flag in getattr(args, "deprecated", []) or []:
        notice = DEPRECATION_NOTICES.get(flag)
        if notice:
            err_console.print(notice, markup=False, highlight=False)


def __chat_unavailable() -> None:
    """
    Honest not-yet for chat (and, until chat ships, bare smah / --interactive).
    """
    err_console.print(f"[bold red]smah:[/bold red] {CHAT_NOTICE}")
    sys.exit(ExitCode.USAGE)


def __run_ask(args, pipe: Optional[str], query: Optional[str], instructions: Optional[str]) -> None:
    """
    The 'ask' subcommand: positional query, optional instruction file, piped
    stdin as data.
    """
    if not query and instructions:
        query = __read_instructions(instructions)

    if not query:
        if pipe:
            err_console.print(
                "[bold red]smah:[/bold red] input was piped in but no query was given; "
                "add a query (e.g. smah ask \"summarize this\") to say what to do with it."
            )
        else:
            err_console.print(
                "[bold red]smah:[/bold red] 'ask' needs a query argument or piped stdin. "
                "Try: smah ask \"...\" or: some-cmd | smah ask \"...\""
            )
        sys.exit(ExitCode.USAGE)

    __run_query(args, pipe, query)


def __run_query(args, pipe: Optional[str], query: str) -> None:
    """
    Shared one-shot path: settings (first-run wizard included), then the
    runner's public query/pipe methods.
    """
    settings = __load_settings(args)

    # If settings are not configured, ask user to provide necessary information
    if not settings.is_configured() or args.configure:
        settings = configurator(settings)
        settings.log(print=True, format=True)
    else:
        settings.log(print=(args.verbose >= 3), format=True)
    runner = Runner(args, settings)

    if pipe:
        runner.pipe(query=query, pipe=pipe)
    else:
        runner.query(query=query)


def __run_config(args) -> None:
    """
    The 'config' subcommand: show / path / set / edit / wizard.
    """
    sub = getattr(args, "config_command", None)
    if sub == "show":
        config_cmd.show(args)
    elif sub == "path":
        config_cmd.print_path(args)
    elif sub == "set":
        config_cmd.set_value(args, args.config_set_key, args.config_set_value)
    else:  # None (bare 'smah config') or 'edit'
        __config_wizard(args)


def __config_wizard(args) -> None:
    """
    Runs the interactive setup wizard (replaces --configure).
    """
    settings = __load_settings(args)
    settings = configurator(settings)
    settings.log(print=True, format=True)


def __load_settings(args) -> Settings:
    """
    Loads settings, mapping config-file failures onto the spec's exit-3
    messages (docs/ux/interface-spec.md §4).
    """
    path = args.config or Settings.default_config()
    try:
        return Settings(config=args.config)
    except yaml.YAMLError as e:
        __fatal(f"can't read config at {path}: {e}. Fix it, or move it aside and run 'smah config'.", e, args, ExitCode.CONFIG)
    except Exception as e:
        __fatal(f"config at {path} is invalid: {e} (expected vsn {Settings.config_vsn()}). Run 'smah config' to regenerate.", e, args, ExitCode.CONFIG)


def __fatal(message: str, exc: BaseException, args, code: ExitCode) -> None:
    """
    Prints a readable error message to stderr and exits with the given exit code.

    The full traceback is printed only when verbosity is raised (-v or higher);
    otherwise the user is pointed at -v and the log files.

    Args:
        message (str): Human readable description of what failed.
        exc (BaseException): The exception that caused the failure.
        args (Namespace): Parsed command-line arguments (may be None if argument parsing failed).
        code (ExitCode): Exit code to terminate with.
    """
    err_console.print(f"[bold red]smah:[/bold red] {message}")
    if getattr(args, "verbose", 0) >= 1:
        err_console.print(Traceback.from_exception(type(exc), exc, exc.__traceback__))
    else:
        err_console.print("Run with -v for a full traceback. Details are also logged under ~/.smah/logs/.")
    sys.exit(int(code))


def __usage() -> str:
    """
    Builds the concise usage message shown when smah is invoked with nothing to do.
    Points at the real subcommands.

    Returns:
        str: The usage message.
    """
    return textwrap.dedent(
        """\
        smah: nothing to do — no command given.

        Usage:
          smah ask "your question"               ask a one-shot question
          cat app.log | smah ask "summarize"     process piped input (add a query)
          smah resume --last                     resume your most recent conversation
          smah history                           list recent conversations
          smah models list                       see configured models and defaults
          smah doctor                            check the installation
          smah config                            run the setup wizard

        Run `smah --help` for the full command list.
        """
    )


def __read_instructions(path: str) -> str:
    """
    Reads an instruction file whose contents become the query.
    """
    try:
        with open(path, 'r') as file:
            return file.read()
    except FileNotFoundError as e:
        logging.error("Instruction file not found: %s", path)
        raise e
    except IOError as e:
        logging.error("IO error reading file '%s': %s", path, str(e))
        raise e


def __with_query(args) -> Optional[str]:
    """
    Extracts the query from the legacy command-line arguments.

    Args:
        args (argparse.Namespace): The parsed command-line arguments.

    Returns:
        Optional[str]: The query string if provided, None otherwise.
    """
    if args.query:
        return args.query

    if args.instructions:
        return __read_instructions(args.instructions)

    return None

if __name__ == "__main__":
    main()
