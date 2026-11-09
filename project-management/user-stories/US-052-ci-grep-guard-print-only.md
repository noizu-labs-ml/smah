---
id: US-052
title: "CI grep guard enforces the print-only invariant"
slug: ci-grep-guard-print-only
personas: [P-001]
epic: "Safety and Print-Only Guarantees"
priority: must-have
complexity: low
tags: [safety, ci, regression-guard]
---

# US-052: CI Grep Guard Enforces the Print-Only Invariant

## User Story

**As a** maintainer
**I want to** CI to fail the build if any execution primitive (`subprocess`, `shell=True`, `os.system`, `eval(`, `exec(`) reappears under `smah/` — excluding the OS-probe readers under `operating_system/info/`
**So that** the print-only invariant is enforced by machinery, not memory.

## Acceptance Criteria

- **Given** a PR that reintroduces an execution primitive
  **When** CI runs
  **Then** the grep-guard job fails with the message "SMAH is print-only."
- **Given** the legitimate OS-probe readers (which read system info, not run commands)
  **When** CI scans
  **Then** they are excluded by the documented path exclusion, not by weakening the pattern.
- **Given** the test suite
  **When** `tests/test_no_shell_execution.py` runs
  **Then** the invariant is asserted independently of the grep (belt and suspenders).

## Notes

- 161 test functions across 10 files exist; the no-execution pair are the load-bearing ones.
- Legacy: docs/ux E1 acceptance criteria (the grep rule, landed in ci.yml).
