---
id: US-044
title: "--continue, --session, and --history alias resume and history"
slug: continue-session-history-aliases
personas: [P-002]
epic: "Legacy Flag Back-Compat"
priority: must-have
complexity: medium
tags: [legacy-flags, resume, history, back-compat]
---

# US-044: --continue, --session, and --history Alias resume and history

## User Story

**As a** solo sysadmin with months of typed habits
**I want to** `--continue`, `--session N`, and `--history` to keep resuming and listing sessions as they always did
**So that** my evening firefighting rhythm does not change.

## Acceptance Criteria

- **Given** `smah --continue` (or `--no-continue` accepted for parse-compat)
  **When** it runs
  **Then** the most recent session resumes, identical to `smah resume --last`, plus the deprecation notice.
- **Given** `smah --session 4`
  **When** it runs
  **Then** session 4 resumes, identical to `smah resume 4`, plus the notice.
- **Given** `smah --history` on a TTY
  **When** it runs
  **Then** the interactive picker of the 10 most recent sessions appears and selection resumes — the one legacy surface that is richer than its subcommand equivalent.

## Notes

- `--history`'s picker predates the plain `history` table; it remains deprecated with no subcommand replacement planned for the picker itself.
- Legacy: docs/ux US-23/US-24/US-25 alias criteria (supersedes).
