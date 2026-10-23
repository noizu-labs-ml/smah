---
id: US-026
title: "history lists sessions with --limit"
slug: history-lists-sessions
personas: [P-002]
epic: "Sessions and History"
priority: must-have
complexity: low
tags: [history, sessions, stdout]
---

# US-026: history Lists Sessions with --limit

## User Story

**As a** solo sysadmin
**I want to** `smah history` to print a plain table of recent sessions (ID, MODIFIED, TITLE), newest first, with `--limit N` (default 20)
**So that** the thing I did last week is findable without a TTY.

## Acceptance Criteria

- **Given** sessions in the database
  **When** `smah history` runs
  **Then** a plain table with ID, MODIFIED, and TITLE columns prints to stdout, newest first.
- **Given** `--limit N`
  **When** the listing renders
  **Then** at most N rows print.
- **Given** stdout redirected to a file
  **When** the listing renders
  **Then** the table is plain text (grep-able), with no interactive picker attempted.

## Notes

- Legacy: docs/ux US-26 (supersedes — the search/`--drop`/`--show` scope of the old story shrank to the shipped surface: table + limit; the TTY picker lives on as the legacy `--history` alias, US-044).
