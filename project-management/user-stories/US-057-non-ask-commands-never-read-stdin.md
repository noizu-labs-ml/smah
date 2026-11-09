---
id: US-057
title: "Non-ask commands never consume stdin"
slug: non-ask-commands-never-read-stdin
personas: [P-005]
epic: "Scripting and Exit-Code Contract"
priority: must-have
complexity: low
tags: [scripting, stdin, hang-guard]
---

# US-057: Non-Ask Commands Never Consume Stdin

## User Story

**As a** automation engineer
**I want to** only `ask` (and the legacy/bare paths) ever to read stdin — `history`, `resume`, `config`, `models`, and `doctor` never do
**So that** an inherited open pipe in a scheduled job can never hang a non-interactive command.

## Acceptance Criteria

- **Given** `smah history` (or `config`, `models`, `doctor`) invoked with an open, never-closing stdin
  **When** the command runs
  **Then** it completes and exits normally without waiting on stdin.
- **Given** `smah ask` with piped data
  **When** it runs
  **Then** stdin is consumed as the data (pipe mode) — the one intended reader.
- **Given** the wizard path
  **When** stdin is not a real TTY
  **Then** the TTY gate blocks the launch (US-035) rather than reading the foreign pipe.

## Notes

- This is the deliberate CI-hang guard (review §2): stdin discipline is what makes every subcommand safe inside job runners.
