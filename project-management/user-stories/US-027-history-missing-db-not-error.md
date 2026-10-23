---
id: US-027
title: "history with a missing database is not an error"
slug: history-missing-db-not-error
personas: [P-005]
epic: "Sessions and History"
priority: could-have
complexity: low
tags: [history, exit-codes, edge-case]
---

# US-027: history with a Missing Database Is Not an Error

## User Story

**As a** automation engineer bootstrapping a fresh machine
**I want to** `smah history` on a missing or uninitialized database to succeed as an empty listing
**So that** "no history yet" is not a failure state in my setup scripts.

## Acceptance Criteria

- **Given** no database file at the configured path
  **When** `smah history` runs
  **Then** an empty listing (or explanatory line) prints to stdout and the exit code is 0.
- **Given** a database that exists but has no sessions
  **When** `smah history` runs
  **Then** the same empty-listing behavior applies.

## Notes

- Distinguishing "empty" from "broken" is `doctor`'s job (US-037); history stays permissive.
