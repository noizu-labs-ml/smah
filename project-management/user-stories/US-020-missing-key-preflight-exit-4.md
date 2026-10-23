---
id: US-020
title: "Missing key preflight fails with exit 4 before any network call"
slug: missing-key-preflight-exit-4
personas: [P-003, P-005]
epic: "Providers and API Keys"
priority: must-have
complexity: low
tags: [providers, keys, exit-codes, preflight]
---

# US-020: Missing Key Preflight Fails with Exit 4 Before Any Network Call

## User Story

**As a** new user who skipped key setup
**I want to** `smah ask` to preflight the key and fail with exit 4 and provider-specific guidance before any network call
**So that** I fix the right thing immediately instead of watching a confusing provider error.

## Acceptance Criteria

- **Given** no key resolvable for the active provider
  **When** any provider-backed command starts
  **Then** the run fails with exit 4, a message naming the provider, and both remediation paths (`smah config`, the provider env var).
- **Given** the preflight failure
  **When** it fires
  **Then** zero network requests have been made (verifiable with a fake provider recording no calls).
- **Given** a malformed provider config block
  **When** the preflight runs
  **Then** a config-class error (exit 3) results instead (see US-023).

## Notes

- Legacy: docs/ux US-05 (supersedes — the 0.1.x silent-swallow behavior is gone; `require_api_key()` preflights before every network call).
