---
id: US-054
title: "Delete dead executor-era code"
slug: delete-dead-executor-era-code
personas: [P-001]
epic: "Safety and Print-Only Guarantees"
priority: could-have
complexity: low
tags: [safety, housekeeping, honesty]
---

# US-054: Delete Dead Executor-Era Code

## User Story

**As a** maintainer running the project's own "honesty pass"
**I want to** the dead executor vestiges removed — `Prompts.run_command_tool()` (an unwired `run_command` tool schema reading "Runs a specified command...") and the `Runner.replace_exec_tags()` pass-stub
**So that** the codebase contains no residue suggesting SMAH can act.

## Acceptance Criteria

- **Given** `smah/runner/prompts.py`
  **When** searched for a `run_command` tool schema
  **Then** it is gone (the `tools=` parameter stays UNSET everywhere today — the schema is pure dead code).
- **Given** `smah/runner/runner.py`
  **When** searched for `replace_exec_tags`
  **Then** either the stub is gone or it is renamed to reflect its display-only purpose with no execution-era name.

## Notes

- KNOWN GAP (review §9.7): both vestiges exist today. No behavior change — this is code hygiene that removes confusion for auditors and contributors.
- Legacy: docs/ux E6 scope note ("the never-used run_command_tool prompt scaffolding — deleted").
