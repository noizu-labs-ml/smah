---
id: US-040
title: "models list shows catalog and resolved stage defaults"
slug: models-list-catalog
personas: [P-001]
epic: "Doctor and Diagnostics"
priority: must-have
complexity: low
tags: [models, catalog, stdout]
---

# US-040: models list Shows Catalog and Resolved Stage Defaults

## User Story

**As a** platform engineer tracking what my questions cost and where they go
**I want to** `smah models list` to print the catalog (PROVIDER, MODEL, DEFAULT FOR, ENABLED, STATUS) plus the resolved per-stage defaults
**So that** I can see which model answers what, before asking.

## Acceptance Criteria

- **Given** the configured catalog
  **When** `smah models list` (or bare `smah models`) runs
  **Then** the table prints to stdout with provider, model id, default-for role, enabled state, and status.
- **Given** stage-specific resolution
  **When** the listing renders
  **Then** the resolved query-planner and pipe-planner defaults are shown alongside the catalog.
- **Given** the output
  **When** captured
  **Then** it is plain, stable text on stdout (exit 0).

## Notes

- Legacy: docs/ux US-29 (supersedes — shipped; note the old story's `models use/enable/disable/test` verbs were not built; default changes go through `config set`, US-033).
