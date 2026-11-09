---
id: US-001
title: "One-shot query via smah ask"
slug: one-shot-query-via-smah-ask
personas: [P-002, P-003]
epic: "Command Generation and Output Panel"
priority: must-have
complexity: low
tags: [ask, core, stdout]
---

# US-001: One-Shot Query via smah ask

## User Story

**As a** developer or sysadmin
**I want to** ask a one-line question and get a formatted answer
**So that** SMAH is as fast as a shell alias.

## Acceptance Criteria

- **Given** a configured install with a usable key
  **When** `smah ask "how do I shrink the systemd journal"` runs
  **Then** the answer prints to stdout with formatting appropriate to the output stream, and the process exits 0.
- **Given** a TTY on stdout
  **When** the answer renders
  **Then** rich markdown formatting is applied; chrome stays on stderr.

## Notes

- Legacy: docs/ux US-07 (supersedes — the `ask` subcommand is now the canonical path; `-q` remains a deprecated alias, see US-043).
- Query text plus configured user/system profile are sent to the model provider (see US-065).
