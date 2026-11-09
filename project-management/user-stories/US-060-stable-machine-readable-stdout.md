---
id: US-060
title: "Stable machine-readable stdout for list-style commands"
slug: stable-machine-readable-stdout
personas: [P-005]
epic: "Scripting and Exit-Code Contract"
priority: should-have
complexity: medium
tags: [scripting, stdout, stability]
---

# US-060: Stable Machine-Readable Stdout for List-Style Commands

## User Story

**As a** automation engineer
**I want to** `history`, `models list`, `doctor --json`, and `config show`/`path` to keep their stdout formats stable within a release — plain tables or JSON, no chrome
**So that** I can `grep`/`jq` their output without fragile parsing.

## Acceptance Criteria

- **Given** any list-style command
  **When** invoked twice on the same state within a release
  **Then** stdout differs only where the underlying data differs (no timestamps-of-invocation noise, no spinner fragments).
- **Given** stdout captured to a file
  **When** inspected
  **Then** it contains only the data — notices and warnings went to stderr.
- **Given** a format change becomes necessary
  **When** it lands
  **Then** it ships in a release note as a breaking-script change (format is contract).

## Notes

- Complements US-012 (the stdout/stderr split) and US-038 (doctor --json stability).
