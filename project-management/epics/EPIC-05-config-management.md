---
id: EPIC-05
title: "Config Management"
slug: config-management
personas: [P-001, P-002, P-005, P-006]
stories: [US-032, US-033, US-034, US-035, US-036]
legacy: "docs/ux E4 (partial: wizard), US-33 (config show)"
---

# EPIC-05: Config Management

## Description

Everything lives under `~/.smah/` — `config.yaml` (version-gated), `smah.db`, `logs/`. The `config` subcommand offers `show` (masked YAML to stdout), `path`, `edit`, `set KEY VALUE` (one dotted key, bool/int coercion), with bare `config` or `edit` launching the interview wizard. The wizard requires a real TTY (else exit 3) and renders entirely on stderr. `-c/--config` and `--database` override paths per run.

## Goals

- Inspect config without exposing secrets (`show` masks values under secret-looking keys; `$ENV` indirection shows as `<env:NAME>`).
- Change one setting without hand-editing YAML (`set`).
- The wizard is a TTY-only, stderr-only experience that never blocks piped workflows.

## User Value

Users can audit, tweak, and relocate their configuration safely — including in scripts (`show`, `path`) — and onboarding never hijacks a non-interactive session.

## Personas Served

P-001 Priya (secret-safe audit), P-002 Marcus (wizard re-runs), P-005 Sam (scriptable show/path), P-006 Yuki (plain-text stdout output).

## Stories

| ID | Title | Priority |
|----|-------|----------|
| US-032 | config show prints masked config to stdout | must-have |
| US-033 | config path and config set with type coercion | should-have |
| US-034 | config edit and bare config open the wizard | should-have |
| US-035 | Wizard requires a real TTY, else exit 3; UI on stderr | must-have |
| US-036 | Per-run --config and --database path overrides | should-have |

## MoSCoW Rollup

must-have: 2 · should-have: 3 · could-have: 0 · wont-have: 0

## Legacy

docs/ux E4 (onboarding wizard scope, narrowed to what shipped) and US-33 (config show redaction, landed). The wizard's stderr discipline and TTY gate come from the interface-spec §3 mockups.
