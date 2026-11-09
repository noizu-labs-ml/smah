---
id: US-056
title: "Exit-code contract 0-5 documented and enforced on every path"
slug: exit-code-contract
personas: [P-005]
epic: "Scripting and Exit-Code Contract"
priority: must-have
complexity: medium
tags: [exit-codes, scripting, contract]
---

# US-056: Exit-Code Contract 0-5 Documented and Enforced on Every Path

## User Story

**As a** automation engineer
**I want to** one exit-code taxonomy on every error path — 0 ok, 1 unexpected internal, 2 usage, 3 config, 4 auth/no usable model, 5 provider/API — documented in the parser epilog
**So that** my scripts branch correctly by code alone.

## Acceptance Criteria

- **Given** any failure mode
  **When** SMAH exits
  **Then** the code matches the taxonomy: usage/conflict errors 2, config errors 3, auth/no-key/no-usable-model 4, provider/API/network 5, unexpected internal 1, success 0.
- **Given** `smah --help`
  **When** displayed
  **Then** the epilog documents the six codes.
- **Given** any error path
  **When** it terminates the run
  **Then** it routes through the single `fatal()` source of truth — no ad-hoc `sys.exit` scatter, no silent exit-0 failures.

## Notes

- Legacy: docs/ux US-36 (supersedes — the 0.1.x always-exit-0 behavior is gone).
- CI asserts each code per failure class with fake provider responses.
