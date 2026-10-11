"""
smah.commands

Implementations behind the smah subcommand surface (history, config, models,
doctor). smah.smah owns dispatch; these modules hold the behavior. Output
discipline throughout: machine-consumable output on stdout, all notices,
hints, and human chrome on stderr.
"""

CHAT_NOTICE = "chat is not available yet — use 'smah ask' for one-shot queries"
