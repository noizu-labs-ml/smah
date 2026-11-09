---
id: US-045
title: "--interactive points at smah chat and exits 2"
slug: interactive-flag-points-at-chat
personas: [P-005]
epic: "Legacy Flag Back-Compat"
priority: must-have
complexity: low
tags: [legacy-flags, chat, honesty]
---

# US-045: --interactive Points at smah chat and Exits 2

## User Story

**As a** automation engineer auditing the flag surface
**I want to** `--interactive` (which never worked in any release) to say so honestly — a notice pointing at `smah chat`, then exit 2
**So that** no invocation pretends to enter a mode that does not exist.

## Acceptance Criteria

- **Given** `smah --interactive` (or `--no-interactive`)
  **When** it runs
  **Then** its `[deprecation]` notice points at `smah chat`, the chat placeholder message prints, and the exit code is 2.
- **Given** the invocation
  **When** it completes
  **Then** no session is created and no provider call is made.

## Notes

- Legacy: docs/ux US-38 (partial) — the flag was a silent stub pre-0.2; honest refusal is the fix.
- Distinct from US-058 (bare `chat` subcommand placeholder) — this is the deprecated alias path into the same honest refusal.
