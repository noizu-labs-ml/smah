---
id: US-067
title: "Setup discloses data locations and egress"
slug: setup-discloses-data-and-egress
personas: [P-001, P-007]
epic: "First Run and Onboarding"
priority: could-have
complexity: low
tags: [onboarding, transparency, egress, privacy]
---

# US-067: Setup Discloses Data Locations and Egress

## User Story

**As a** user with a data-egress review to pass
**I want to** the wizard (and help) to state where data lives — config at `~/.smah/config.yaml`, sessions at `~/.smah/smah.db`, logs under `~/.smah/logs/` — and that query text plus my configured profile are sent to the model provider
**So that** I can clear security review before my first real query.

## Acceptance Criteria

- **Given** the wizard completes
  **When** its footer renders
  **Then** the three local paths and the egress statement appear.
- **Given** `smah doctor`
  **When** it reports
  **Then** the same local-path inventory appears in the findings.
- **Given** the disclosure
  **When** audited against behavior
  **Then** it is exactly true — nothing else leaves the machine (no telemetry exists).

## Notes

- Legacy: docs/ux US-06 (supersedes, narrowed to what ships).
- Egress visibility is a stated abandonment trigger for P-001 and P-004.
