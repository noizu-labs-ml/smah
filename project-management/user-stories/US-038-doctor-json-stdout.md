---
id: US-038
title: "doctor --json prints findings to stdout"
slug: doctor-json-stdout
personas: [P-005]
epic: "Doctor and Diagnostics"
priority: must-have
complexity: medium
tags: [doctor, json, scripting, ci]
---

# US-038: doctor --json Prints Findings to Stdout

## User Story

**As a** automation engineer gating deploys on SMAH health
**I want to** `smah doctor --json` to emit machine-readable findings on stdout while the human report stays on stderr
**So that** a CI job can parse health without scraping prose.

## Acceptance Criteria

- **Given** `--json`
  **When** `smah doctor` runs
  **Then** the JSON findings document prints to stdout (human report suppressed or on stderr), valid JSON, exit code = worst finding.
- **Given** a CI step doing `smah doctor --json | jq ...`
  **When** stdout is captured
  **Then** it contains only the JSON — no banners or chrome.
- **Given** the same install state
  **When** `doctor --json` runs twice
  **Then** the structure (keys and their meaning) is stable across runs of the same version.

## Notes

- Stability contract: field *meaning* is stable per release; new fields may be added (additive).
- Key findings report presence booleans, never values (US-039 applies here too).
