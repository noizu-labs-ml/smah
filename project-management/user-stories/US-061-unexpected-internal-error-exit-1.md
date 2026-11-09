---
id: US-061
title: "Unexpected internal errors exit 1 with the log path"
slug: unexpected-internal-error-exit-1
personas: [P-005]
epic: "Scripting and Exit-Code Contract"
priority: must-have
complexity: low
tags: [exit-codes, errors, logging]
---

# US-061: Unexpected Internal Errors Exit 1 with the Log Path

## User Story

**As a** automation engineer
**I want to** a bug-triggered crash to exit 1 with a generic message pointing at the log file holding the full traceback
**So that** my pipeline treats it as "tool defect, file a bug" — distinct from every user-fixable class — without a traceback spewing into captured output.

## Acceptance Criteria

- **Given** an unhandled exception escaping all typed handling
  **When** the final catch fires
  **Then** a generic internal-error message prints to stderr with the log file path, and the exit code is 1.
- **Given** the log file
  **When** inspected
  **Then** it contains the full traceback for the bug report.
- **Given** the failure
  **When** compared with exit 5
  **Then** exit 1 is reserved for genuine defects — provider errors never masquerade as internal ones.

## Notes

- The 0.1.x blanket-except that *swallowed* errors into exit 0 is the anti-pattern this replaces.
- Legacy: docs/ux US-35/US-36 (partial — the "no silent no-op exits anywhere" guarantee).
