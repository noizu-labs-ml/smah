---
id: US-022
title: "Provider and network failures map to exit 5"
slug: provider-network-failures-exit-5
personas: [P-005, P-007]
epic: "Providers and API Keys"
priority: must-have
complexity: low
tags: [providers, exit-codes, errors, network]
---

# US-022: Provider and Network Failures Map to Exit 5

## User Story

**As a** operator behind a flaky egress path
**I want to** rate limits, bad requests, 5xx, timeouts, and connection failures to all produce exit 5 with the provider named
**So that** my scripts retry or back off on their own terms and I can tell a dead network from a bad key (exit 4).

## Acceptance Criteria

- **Given** the provider returns 429, 5xx, or a malformed-request error
  **When** the error translates
  **Then** SMAH exits 5 with a message naming the provider and condition.
- **Given** the connection times out or resets
  **When** the error translates
  **Then** SMAH exits 5 promptly — no long internal retry storm against a proxy that is down.
- **Given** exit 5 occurs
  **When** the message renders
  **Then** it is distinguishable from exit 4 (auth) and exit 3 (config) in both text and code.

## Notes

- Legacy: docs/ux US-35 (partial — provider/network class of the error table).
- Retry/backoff policy is deliberately out of scope this cycle: a clear message is the fix (docs/ux E3 scope note).
