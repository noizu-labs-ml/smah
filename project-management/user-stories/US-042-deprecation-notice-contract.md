---
id: US-042
title: "Deprecation notice contract: one line on stderr, equivalent runs"
slug: deprecation-notice-contract
personas: [P-002, P-005]
epic: "Legacy Flag Back-Compat"
priority: must-have
complexity: medium
tags: [legacy-flags, deprecation, stderr, back-compat]
---

# US-042: Deprecation Notice Contract: One Line on Stderr, Equivalent Runs

## User Story

**As a** user with old muscle memory or old scripts
**I want to** every deprecated legacy flag to still work — running its new equivalent — with a one-line `[deprecation] ...` notice on stderr stating the replacement and the removal version
**So that** nothing breaks today while migration stays visible and time-boxed.

## Acceptance Criteria

- **Given** any of the 8 deprecated legacy flags (`-q/--query`, top-level `-i/--instructions`, `--continue/--no-continue`, `--session N`, `--history/--no-history`, `--interactive/--no-interactive`, `--configure/--no-configure`, `--openai-api-key KEY`)
  **When** invoked
  **Then** the new-equivalent code path runs with behavior identical to the subcommand form.
- **Given** the invocation
  **When** it executes
  **Then** exactly one `[deprecation]` line prints on stderr naming the replacement; stdout stays clean.
- **Given** the notice text
  **When** rendered
  **Then** it matches the normative message table (interface-spec §2) and states the planned removal (0.4).
- **Given** combined deprecated flags in one invocation
  **When** it executes
  **Then** each deprecated flag produces its notice (or a single consolidated notice) — never silence.

## Notes

- Legacy: docs/ux E5 back-compat table and US-39 (supersedes with the as-shipped design).
- Removal slated for 0.4 per CHANGELOG; deprecation notices are the migration clock.
