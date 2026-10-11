"""
history command: list recent sessions.

Non-blocking by design: prints a plain table (ID, MODIFIED, TITLE, newest
first) to stdout. The interactive pick-then-resume picker stays available
through the deprecated --history flag so existing behavior is unchanged.
"""

import os
import sqlite3
import sys

from rich.prompt import Prompt

from smah.console import err_console
from smah.database import Database
from smah.exit_codes import ExitCode, fatal


def list_sessions(args, limit: int = 20) -> None:
    """
    Prints recent sessions to stdout. A database that does not exist yet is
    not an error (no sessions yet); an unreadable one is a config error.
    """
    path = args.database or Database.default_database()
    if not os.path.exists(path):
        err_console.print("[bold red]smah:[/bold red] no sessions yet — the database is created on first use.")
        return

    try:
        db = Database(args)
        sessions = db.history(limit)
    except sqlite3.OperationalError as e:
        if "no such table" in str(e):
            err_console.print("[bold red]smah:[/bold red] no sessions yet — the database is initialized on first use.")
            return
        fatal(f"session database problem at {path}: {e}. Try 'smah-db status'.", ExitCode.CONFIG)
    except sqlite3.DatabaseError as e:
        fatal(f"session database problem at {path}: {e}. Try 'smah-db status'.", ExitCode.CONFIG)

    if not sessions:
        err_console.print("[bold red]smah:[/bold red] no sessions yet.")
        return

    print(f"{'ID':<8}{'MODIFIED':<22}TITLE")
    for session in sessions:  # db.history() is newest-first
        modified = str(session["modified_on"] or "")[:19]
        print(f"{session['id']:<8}{modified:<22}{session['title'] or ''}")


def pick_session(args) -> int:
    """
    Picks a recent session from the database (interactive picker; used by the
    deprecated --history flag so its behavior is unchanged).
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
