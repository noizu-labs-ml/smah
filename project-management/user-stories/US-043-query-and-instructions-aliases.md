---
id: US-043
title: "-q/--query and top-level -i/--instructions alias ask"
slug: query-and-instructions-aliases
personas: [P-002, P-003]
epic: "Legacy Flag Back-Compat"
priority: must-have
complexity: medium
tags: [legacy-flags, ask, back-compat]
---

# US-043: -q/--query and Top-Level -i/--instructions Alias ask

## User Story

**As a** user with 0.1.x-era aliases and scripts
**I want to** `smah -q "..."` and top-level `smah -i FILE` to keep working as `smah ask "..."` and `smah ask --instructions FILE`
**So that** my existing workflows survive the subcommand migration unchanged.

## Acceptance Criteria

- **Given** `smah -q "how do I ..."` (or `--query`)
  **When** it runs
  **Then** it behaves identically to `smah ask "how do I ..."` plus its `[deprecation]` stderr notice.
- **Given** top-level `smah -i FILE` with no query
  **When** it runs
  **Then** the file becomes the query exactly as `smah ask --instructions FILE` would.
- **Given** both the top-level `-i` and `ask`'s own `-i` could apply
  **When** parsing resolves
  **Then** the dispatch is unambiguous (top-level `-i` is the deprecated alias; `ask -i` is canonical).

## Notes

- Golden-rule: alias behavior ≡ subcommand behavior in every observable respect except the notice.
- Legacy: docs/ux US-07/US-08 alias criteria (supersedes).
