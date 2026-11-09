---
id: US-049
title: "No execution path exists; print-only has no toggle"
slug: no-execution-path-print-only
personas: [P-001]
epic: "Safety and Print-Only Guarantees"
priority: must-have
complexity: medium
tags: [safety, print-only, invariant]
---

# US-049: No Execution Path Exists; Print-Only Has No Toggle

## User Story

**As a** platform engineer under change control
**I want to** certainty that no code path in SMAH can execute a model-generated command — the executor deleted, not gated, with no opt-in toggle anywhere
**So that** SMAH is permissible on production-adjacent machines at all.

## Acceptance Criteria

- **Given** the entire `smah/` package
  **When** searched for `subprocess`, `shell=True`, `os.system`, `eval(`, or `exec(` used on model output
  **Then** nothing is found (the CI grep guard, US-052, enforces this mechanically).
- **Given** any config file, env var, or flag combination
  **When** a user tries to enable execution
  **Then** no such option exists — there is no y/n confirm, no sandbox, no allowlist, no escape hatch.
- **Given** a suggested command
  **When** the user wants to run it
  **Then** the only path is the user's own shell (copy/clipboard handoff, US-009).

## Notes

- Legacy: docs/ux US-10 (supersedes — v0.2.0 removed the executor as a breaking change; `Runner.replace_exec_tags()`, the old execution hook, is now a `pass` stub).
- This is the product's defining constraint: Priya is uninstallable the day it breaks.
