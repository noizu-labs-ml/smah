---
id: US-066
title: "First-run entry point matches the README (bare smah gap)"
slug: first-run-entry-point-honesty
personas: [P-003]
epic: "First Run and Onboarding"
priority: should-have
complexity: low
tags: [onboarding, docs-honesty, gap, readme]
---

# US-066: First-Run Entry Point Matches the README (bare smah Gap)

## User Story

**As a** new user following the README
**I want to** the documented first-run flow to match the code — the README says bare `smah` starts setup, the code prints usage and exits 2, and setup actually runs via `smah config` (or legacy `--configure`)
**So that** my first five minutes are not spent discovering the docs lie.

## Acceptance Criteria

- **Given** a new user reads the README "First Run" section and follows it
  **When** they type the documented command
  **Then** the observed behavior matches the documented behavior (either the doc is corrected to `smah config`, or the entry point changes — one of the two, deliberately).
- **Given** the resolution ships
  **When** released
  **Then** README, `--help`, and dispatch agree on how setup starts.

## Notes

- KNOWN GAP (review §9.3): the README setup flow predates the 0.2.1 fresh-install hardening that made bare `smah` a usage error. The code behavior is deliberate; the README is the stale side.
- The product's selling point is honesty about absence — an inaccurate first-run doc undercuts it more than most.
- Docs changes are out of scope for the artifact tree that houses this story; this records the product decision to close the gap.
