---
id: US-021
title: "Provider auth failure maps to exit 4 and names the provider"
slug: provider-auth-failure-exit-4
personas: [P-005]
epic: "Providers and API Keys"
priority: must-have
complexity: low
tags: [providers, exit-codes, errors]
---

# US-021: Provider Auth Failure Maps to Exit 4 and Names the Provider

## User Story

**As a** automation engineer
**I want to** a revoked or wrong key to produce exit 4 with a message naming the provider
**So that** my script can alert "credentials problem" distinctly from "network problem".

## Acceptance Criteria

- **Given** the provider returns 401/403 for a request
  **When** the error is translated
  **Then** SMAH exits 4 with a message naming the provider (e.g. anthropic/openai) and the remediation hint.
- **Given** a permission-scoped key without model access
  **When** the call fails with a permission error
  **Then** the same exit-4 auth class applies.
- **Given** the failure
  **When** it renders
  **Then** the key value never appears in the message.

## Notes

- Legacy: docs/ux US-35 (partial — auth class of the error table).
- Provider translation lives in `translate_error()`; SDK typed exceptions map deterministically.
