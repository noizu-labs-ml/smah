---
id: US-041
title: "models list with zero enabled models is a successful listing"
slug: models-list-zero-models
personas: [P-005]
epic: "Doctor and Diagnostics"
priority: could-have
complexity: low
tags: [models, exit-codes, edge-case]
---

# US-041: models list with Zero Enabled Models Is a Successful Listing

## User Story

**As a** automation engineer
**I want to** `smah models list` on a keyless or empty install to succeed with an (empty or all-disabled) listing
**So that** "nothing configured" is observable state, not a crash — `doctor` is the command that flags it as a problem.

## Acceptance Criteria

- **Given** zero enabled models in the catalog
  **When** `smah models list` runs
  **Then** the table prints (possibly empty or showing disabled rows) with exit 0.
- **Given** the same state
  **When** `smah doctor` runs instead
  **Then** doctor reports the finding and exits nonzero — separation between listing (neutral) and diagnostics (judgment).

## Notes

- Keeps `models list` safe in bootstrap scripts; the judgment call lives in doctor.
