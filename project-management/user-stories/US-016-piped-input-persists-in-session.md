---
id: US-016
title: "Piped input persists with the session for re-query"
slug: piped-input-persists-in-session
personas: [P-004]
epic: "Stdin Piping and Summaries"
priority: could-have
complexity: low
tags: [pipe, sessions, resume]
---

# US-016: Piped Input Persists with the Session for Re-Query

## User Story

**As a** data engineer refining an analysis
**I want to** the piped input stored with its session so I can resume and narrow the question without re-piping
**So that** iteration on large or one-time inputs is cheap.

## Acceptance Criteria

- **Given** a pipe-mode run completed
  **When** the user runs `smah resume` on that session
  **Then** the stored piped input is available as context for follow-up questions.
- **Given** the user wants the piped data gone
  **When** they finish
  **Then** deleting the session removes the stored input (see US-026/US-031 for the listing and migration surface).

## Notes

- Sensitive piped data living in sqlite indefinitely is a known trade-off; users handling secrets should delete sessions (docs should say so).
- Legacy: docs/ux US-18 (partial — pipe input persistence is already shipped behavior).
