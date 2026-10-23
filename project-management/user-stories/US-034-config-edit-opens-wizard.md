---
id: US-034
title: "config edit and bare config open the wizard"
slug: config-edit-opens-wizard
personas: [P-002]
epic: "Config Management"
priority: should-have
complexity: medium
tags: [config, wizard, onboarding]
---

# US-034: config edit and Bare config Open the Wizard

## User Story

**As a** solo sysadmin rotating a key or switching providers
**I want to** re-run the setup interview any time via `smah config` (or `config edit`) without deleting my config
**So that** reconfiguration never costs me my sessions or my settings.

## Acceptance Criteria

- **Given** an existing config
  **When** `smah config` (bare) or `smah config edit` runs on a real TTY
  **Then** the interview wizard launches pre-filled from the existing `~/.smah/config.yaml`.
- **Given** the wizard runs
  **When** it completes or is interrupted (Ctrl-C)
  **Then** sessions in `~/.smah/smah.db` are untouched, and an interrupted run leaves the previous config intact.
- **Given** the wizard UI (rich panels, prompts)
  **When** it renders
  **Then** everything appears on stderr (see US-035).

## Notes

- The wizard is the only path that launches the interview; bare `smah` does not start it (see US-066 for the README gap).
- Legacy: docs/ux US-02 (supersedes — re-run non-destructively, now via the `config` subcommand).
