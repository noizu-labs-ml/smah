---
id: US-046
title: "--configure opens the setup wizard"
slug: configure-alias-wizard
personas: [P-002]
epic: "Legacy Flag Back-Compat"
priority: should-have
complexity: low
tags: [legacy-flags, config, wizard]
---

# US-046: --configure Opens the Setup Wizard

## User Story

**As a** long-time user
**I want to** `smah --configure` (and `--no-configure` for parse-compat) to launch the setup wizard exactly like `smah config`
**So that** my old reconfiguration reflex still works during the deprecation window.

## Acceptance Criteria

- **Given** a real TTY
  **When** `smah --configure` runs
  **Then** the wizard launches pre-filled, identical to `smah config`, plus the `[deprecation]` stderr notice.
- **Given** no real TTY
  **When** `smah --configure` runs
  **Then** the wizard's TTY gate applies (exit 3, US-035) — the alias adds only a notice, never a behavior change.

## Notes

- Legacy: docs/ux US-02 alias criteria (supersedes).
