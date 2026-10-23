---
id: US-070
title: "Unreachable provider fails fast with exit 5 and a clear message"
slug: unreachable-provider-exit-5
personas: [P-007]
epic: "First Run and Onboarding"
priority: must-have
complexity: low
tags: [providers, network, exit-codes, restricted-network]
---

# US-070: Unreachable Provider Fails Fast with Exit 5 and a Clear Message

## User Story

**As a** operator behind a down proxy or an air gap
**I want to** connection failures to end the run quickly with exit 5 naming the provider and condition
**So that** I know it is the network — not my key (exit 4), not my config (exit 3) — and nothing hangs or retries against a path that is down.

## Acceptance Criteria

- **Given** the provider endpoint unreachable (connection refused/reset, timeout)
  **When** the call fails
  **Then** SMAH exits 5 promptly with a message naming the provider and the network condition.
- **Given** the failure
  **When** it renders
  **Then** it is textually and numerically distinct from auth (4) and config (3) failures.
- **Given** no network at all
  **When** the user wants to verify the install anyway
  **Then** `smah doctor` completes fully offline (US-039).

## Notes

- No automatic retry storm this cycle (deliberate scope, see US-022) — a fast, clear failure is the contract.
- The scenario that makes SMAH viable in restricted environments: local diagnostics complete, egress failures are honest and typed.
