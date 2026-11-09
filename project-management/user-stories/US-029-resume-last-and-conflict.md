---
id: US-029
title: "resume --last and the id/--last conflict exit"
slug: resume-last-and-conflict
personas: [P-002, P-005]
epic: "Sessions and History"
priority: must-have
complexity: low
tags: [resume, exit-codes, flags]
---

# US-029: resume --last and the id/--last Conflict Exit

## User Story

**As a** solo sysadmin
**I want to** `smah resume --last` to pick up my most recent session — and `resume 4 --last` to be rejected as a usage error
**So that** "continue where I left off" is one flag, and ambiguous invocations fail loudly.

## Acceptance Criteria

- **Given** at least one session exists
  **When** `smah resume --last` runs
  **Then** the most recently modified session resumes.
- **Given** both a positional id and `--last`
  **When** `smah resume 4 --last` runs
  **Then** cross-flag validation rejects it with exit 2 and a usage message.
- **Given** zero sessions exist
  **When** `smah resume --last` runs
  **Then** an exit-2 error explains there is nothing to resume (see US-030's listing behavior).

## Notes

- Legacy: docs/ux US-23 (supersedes — `--continue` remains the deprecated alias for this behavior, US-044).
