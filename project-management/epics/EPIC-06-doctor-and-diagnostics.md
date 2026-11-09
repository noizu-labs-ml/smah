---
id: EPIC-06
title: "Doctor and Diagnostics"
slug: doctor-and-diagnostics
personas: [P-001, P-003, P-005, P-006]
stories: [US-037, US-038, US-039, US-040, US-041]
legacy: "docs/ux E3 (doctor scope), US-34"
---

# EPIC-06: Doctor and Diagnostics

## Description

`doctor` answers "is it me or the tool" in one command: config validity, database and migration state, API key *presence* (never value), enabled models and resolved per-stage defaults. It performs no network calls. The human report goes to stderr; `--json` puts machine-readable findings on stdout. Exit code is the worst finding. `models list` completes the diagnostic picture with the catalog and defaults.

## Goals

- One command triages a broken install, with actionable text.
- `--json` output is stable enough for CI gating.
- `models list` shows provider, model, default-for, enabled, status — and zero models is a successful listing, not an error.

## User Value

Users and their pipelines distinguish a healthy install from specific breakage in seconds, without burning a provider call or leaking a key.

## Personas Served

P-003 Dana (triage), P-005 Sam (CI gating via --json), P-001 Priya (key-presence audit), P-006 Yuki (labeled plain-text findings).

## Stories

| ID | Title | Priority |
|----|-------|----------|
| US-037 | doctor human report with exit code = worst finding | must-have |
| US-038 | doctor --json prints findings to stdout | must-have |
| US-039 | doctor reports key presence, never values, with no network calls | must-have |
| US-040 | models list shows catalog and resolved stage defaults | must-have |
| US-041 | models list with zero enabled models is a successful listing | could-have |

## MoSCoW Rollup

must-have: 4 · should-have: 0 · could-have: 1 · wont-have: 0

## Legacy

docs/ux E3/US-34 (doctor, landed in the recovery wave) and US-29 (models list, landed). Supersedes both with the current surface: `--json`, `--offline` no-op flag, per-stage default resolution.
