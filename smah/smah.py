"""
smah.py

This module serves as the entry point for the SMAH Command Line Tool. It is responsible for parsing
command line arguments, initializing settings, and coordinating execution based on user inputs.
The module utilizes the `argparse` library for command-line argument parsing and `rich` for enhanced
console outputs.

Classes and Functions:
- extract_args: Parses and extracts command-line arguments.
- log_settings: Logs current application settings using `rich`.
- main: The primary function that sets up application configuration and executes user-specified queries.

Dependencies:
- `os`, `argparse`, `textwrap`, and `yaml` for OS operations, argument parsing, text manipulation, and YAML operations.
- `rich` for enhanced console outputs, utilized throughout the module for logging and interactive feedback.
- `smah.settings.Settings` and `smah.runner.Runner`, which are the other core modules facilitating configuration handling and executing user queries.

Use Cases:
- Can be run with flags to execute specific instructions or queries, configure settings, and manage output verbosity.

Note:
Ensure the environment is set up with the necessary dependencies before executing this script.
"""

import logging
import sys
import textwrap
from typing import Optional

import yaml
from openai import OpenAIError
from rich.traceback import Traceback
from rich.prompt import Prompt

import smah.console
from smah.database import Database, Migration
from smah.exit_codes import ExitCode
from smah.runner import Runner
from smah.settings import Settings, configurator
import smah.logs
import smah.args

from smah.console import std_console, err_console

from types import SimpleNamespace

from lxml import etree

def pick_session(args) -> int:
    """
    Picks a recent session from the database.
    """
    db = Database(args)
    sessions = db.history()
    choices = [""]
    choice_lookup = {}
    count = 0
    prompt = ""
    for session in sessions:
        count += 1
        choices.append(f"{count}")
        choice_lookup[str(count)] = session['id']
        prompt += f"[bold green]{count}[/bold green] - (#{session['id']}) {session['title']}\n"
    prompt += "[bold green]Select a session number to resume:[/bold green] (enter to cancel)"

    choice = Prompt.ask(prompt)
    if not choice:
        sys.exit(ExitCode.OK)
    while choice not in choices:
        choice = Prompt.ask("[bold red]Pick Valid Session[/bold red]")
        if not choice:
            sys.exit(ExitCode.OK)
    return choice_lookup[str(choice)]

def resume_session(args, session: Optional[int] = None):
    """
    Resumes the last conversation from the database.

    Args:
        args (argparse.Namespace): The parsed command-line arguments.
    """
    db = Database(args)
    if session:
        session = db.session(session)
    else:
        session = db.last_session()

    if session:
        args = smah.args.merge_args(args, session['args'])
        settings = Settings(config=args.config)

        # If settings are not configured, ask user to provide necessary information
        if not settings.is_configured() or args.configure:
            settings = configurator(settings)
            settings.log(print=True, format=True)
        else:
            settings.log(print=(args.verbose >= 3), format=True)
        runner = Runner(args, settings)
        runner.resume(id=session['id'], title=session['title'], plan=session['plan'], pipe=session['pipe'], messages=session['messages'])
    else:
        err_console.print("[bold red]smah:[/bold red] No previous session found.")
        sys.exit(ExitCode.UNEXPECTED_ERROR)

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
    coordinates the execution based on user inputs. It handles different modes of operation such as
    processing queries, processing instructions from a file, or resuming a prior session.

    Raises:
        SystemExit: With an ExitCode describing the failure for any error encountered during execution.
    """
    # Configure logging
    smah.logs.configure()

    args = None
    try:
        args, pipe = smah.args.extract_args()

        init_database(args)

        if args.resume:
            resume_session(args)
        elif args.session:
            resume_session(args, session=args.session)
        elif args.history:
            session = pick_session(args)
            resume_session(args, session=session)
        else:
            settings = Settings(config=args.config)

            # If settings are not configured, ask user to provide necessary information
            if not settings.is_configured() or args.configure:
                settings = configurator(settings)
                settings.log(print=True, format=True)
            else:
                settings.log(print=(args.verbose >= 3), format=True)
            runner = Runner(args, settings)

            if args.interactive:
                err_console.print(
                    "[bold red]smah:[/bold red] --interactive is not implemented yet.\n"
                    "Use smah -q \"...\" for one-shot queries, or smah --continue / --history to resume a session."
                )
                sys.exit(ExitCode.USAGE)

            query = __with_query(args)

            if not query:
                if pipe:
                    err_console.print(
                        "[bold red]smah:[/bold red] input was piped in but no query was given; "
                        "add -q \"...\" (or -i <file>) to say what to do with it."
                    )
                else:
                    err_console.print(__usage())
                sys.exit(ExitCode.USAGE)

            if pipe:
                runner.pipe(query=query, pipe=pipe)
            else:
                runner.query(query=query)
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

    Returns:
        str: The usage message.
    """
    return textwrap.dedent(
        """\
        smah: nothing to do — no query given.

        Usage:
          smah -q "your question"                ask a one-shot question
          cat app.log | smah -q "summarize"      process piped input (a query is still required)
          smah --continue                        resume your most recent conversation
          smah --history                         pick a recent conversation to resume
          smah --configure                       run the setup wizard

        Run `smah --help` for the full option list.
        """
    )


def __with_query(args) -> Optional[str]:
    """
    Extracts the query from the command-line arguments.

    Args:
        args (argparse.Namespace): The parsed command-line arguments.

    Returns:
        Optional[str]: The query string if provided, None otherwise.
    """
    if args.query:
        return args.query

    if args.instructions:
        try:
            with open(args.instructions, 'r') as file:
                return file.read()
        except FileNotFoundError as e:
            logging.error("Instruction file not found: %s", args.instructions)
            raise e
        except IOError as e:
            logging.error("IO error reading file '%s': %s", args.instructions, str(e))
            raise e

    return None

if __name__ == "__main__":
    main()
