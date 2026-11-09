---
id: US-006
title: "ask without a query exits 2 with guidance"
slug: ask-without-query-exits-2
personas: [P-005]
epic: "Command Generation and Output Panel"
priority: must-have
complexity: low
tags: [ask, exit-codes, usage]
---

# US-006: ask without a Query Exits 2 with Guidance

## User Story

**As a** automation engineer
**I want to** `smah ask` with neither a query nor an instructions file to fail immediately with usage guidance and exit 2
**So that** my script's misconfiguration is caught at the first step, not after a provider call.

## Acceptance Criteria

- **Given** `smah ask` invoked with no positional query, no `--instructions` file, and no piped stdin
  **When** the command runs
  **Then** guidance text prints to stderr and the exit code is 2.
- **Given** any of the three inputs is present
  **When** `ask` runs
  **Then** the command proceeds normally.

## Notes

- The guidance should name the accepted inputs (positional query, `--instructions` file, piped stdin).
- Legacy: docs/ux US-38 (partial — the "no silent no-op" principle applied to the `ask` verb).
