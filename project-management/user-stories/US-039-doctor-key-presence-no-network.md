---
id: US-039
title: "doctor reports key presence, never values, with no network calls"
slug: doctor-key-presence-no-network
personas: [P-001, P-006]
epic: "Doctor and Diagnostics"
priority: must-have
complexity: low
tags: [doctor, secrets, offline, safety]
---

# US-039: doctor Reports Key Presence, Never Values, with No Network Calls

## User Story

**As a** operator who cannot risk credential exposure
**I want to** `smah doctor` to report whether each provider's key is *present* (and which resolution source) without ever printing a value, and to perform no network calls
**So that** I can run it anywhere — shared sessions, logged terminals, air-gapped hosts — safely.

## Acceptance Criteria

- **Given** a configured key from any resolution source (CLI flag, config, env)
  **When** `doctor` runs
  **Then** the report says the key is present and from where; the value never appears in any output.
- **Given** no network at all (air-gapped, offline)
  **When** `doctor` runs
  **Then** all checks complete locally — no provider calls are attempted.
- **Given** the report
  **When** rendered
  **Then** findings are labeled in words (not color-only), so screen readers get identical information.

## Notes

- Legacy: docs/ux US-06 (partial — the "know what goes where" disclosure) and US-34 (offline diagnostics).
- This is the story that makes P-007's pre-egress review scenario possible.
