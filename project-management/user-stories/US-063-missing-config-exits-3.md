---
id: US-063
title: "Missing config exits 3 and points at smah config"
slug: missing-config-exits-3
personas: [P-003]
epic: "First Run and Onboarding"
priority: must-have
complexity: low
tags: [onboarding, config, exit-codes]
---

# US-063: Missing Config Exits 3 and Points at smah config

## User Story

**As a** new user who installed but has not configured
**I want to** `smah ask` (or `resume`) without a config to fail with exit 3 and "no config found at <path>. Run 'smah config' to create one."
**So that** my first error is a one-step fix, not an EOF-crashing wizard or a silent failure.

## Acceptance Criteria

- **Given** no config file at the resolved path
  **When** `smah ask` runs
  **Then** exit 3 with the exact message naming the path and the `smah config` remedy.
- **Given** the same state on a non-TTY context
  **When** the command runs
  **Then** the behavior is identical — the wizard is never auto-launched, piped or not.
- **Given** `smah config` runs instead
  **When** a TTY is present
  **Then** the wizard creates the config (US-034).

## Notes

- Never an EOF-crashing wizard on a headless box — the TTY gate (US-035) plus this message are the pair.
- Legacy: docs/ux US-01 (partial — the "first command teaches you" principle, shipped as exit 3 rather than auto-setup).
