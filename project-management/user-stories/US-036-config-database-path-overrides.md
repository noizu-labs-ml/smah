---
id: US-036
title: "Per-run --config and --database path overrides"
slug: config-database-path-overrides
personas: [P-005]
epic: "Config Management"
priority: should-have
complexity: low
tags: [config, flags, paths]
---

# US-036: Per-Run --config and --database Path Overrides

## User Story

**As a** automation engineer testing against isolated state
**I want to** `-c/--config FILE` and `--database FILE` to override the default paths for a single run
**So that** I can run SMAH against a scratch config and a scratch session store without touching the user's real ones.

## Acceptance Criteria

- **Given** `-c /tmp/test-config.yaml`
  **When** any command runs
  **Then** the config is read from that path for the run.
- **Given** `--database /tmp/test.db`
  **When** history/resume/ask run
  **Then** session persistence uses that database file.
- **Given** an override path that does not exist (config)
  **When** a provider-backed command runs
  **Then** the missing-config behavior applies (exit 3, US-063) referencing the overridden path.

## Notes

- Defaults remain `~/.smah/config.yaml` and `~/.smah/smah.db`.
