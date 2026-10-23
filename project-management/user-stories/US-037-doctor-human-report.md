---
id: US-037
title: "doctor human report with exit code = worst finding"
slug: doctor-human-report
personas: [P-003]
epic: "Doctor and Diagnostics"
priority: must-have
complexity: medium
tags: [doctor, diagnostics, exit-codes]
---

# US-037: doctor Human Report with Exit Code = Worst Finding

## User Story

**As a** developer with a misbehaving install
**I want to** `smah doctor` to check config validity, database and migrations, key presence, and enabled models with per-stage defaults, and to exit with the worst finding
**So that** "is it me or the tool" is one command with a scriptable answer.

## Acceptance Criteria

- **Given** a healthy install
  **When** `smah doctor` runs
  **Then** the human-readable report prints to stderr and the exit code is 0.
- **Given** a broken config
  **When** `doctor` runs
  **Then** the report names the problem and the exit code is 3 (config class).
- **Given** a missing key or no usable model
  **When** `doctor` runs
  **Then** the exit code is 4 with the finding named.
- **Given** multiple findings of different classes
  **When** `doctor` completes
  **Then** the exit code equals the worst (highest-severity) finding.

## Notes

- Legacy: docs/ux US-34 (supersedes — the shipped doctor; provider *reachability* ping from the old spec was deliberately not built — doctor performs no network calls, US-039).
- `--offline` is accepted as a documented no-op for muscle-memory compatibility.
