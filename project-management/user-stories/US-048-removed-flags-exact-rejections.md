---
id: US-048
title: "Removed flags reject with exact messages and exit 2"
slug: removed-flags-exact-rejections
personas: [P-005]
epic: "Legacy Flag Back-Compat"
priority: must-have
complexity: medium
tags: [legacy-flags, honesty, exit-codes]
---

# US-048: Removed Flags Reject with Exact Messages and Exit 2

## User Story

**As a** automation engineer with an old script
**I want to** the 8 removed flags to fail at the parser with exit 2 and their exact replacement messages — never a silent unknown-arg error, never a fake success
**So that** breakage is immediate, explained, and greppable.

## Acceptance Criteria

- **Given** `--setup`
  **When** parsed
  **Then** exit 2 with the message stating it "never existed in any release" and pointing at `smah config`.
- **Given** `--gui`
  **When** parsed
  **Then** exit 2 with "no GUI exists in SMAH and none is planned".
- **Given** `--model-picker`, `--model-interactive`, `--model-review`, `--model-edit`, `--openai-api-tier`, `--openai-api-org`
  **When** parsed
  **Then** each rejects with exit 2 and its normative message from the back-compat table (interface-spec §2).
- **Given** any removed flag
  **When** it rejects
  **Then** the message names the replacement or states plainly that the feature never existed — no traceback, no exit 0.

## Notes

- Implemented via a `_Rejected` argparse action so the refusal is part of the parser, not downstream code.
- Legacy: docs/ux US-39 (supersedes — flags are *rejected with messages* rather than silently deleted).
