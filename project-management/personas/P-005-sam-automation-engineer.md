---
id: P-005
name: "Sam Okafor"
slug: sam-automation-engineer
archetype: "CI and scripting automation user"
segment: secondary
tags: [ci, non-interactive, exit-codes, machine-readable, stdin-discipline]
---

# P-005: Sam Okafor

## Demographics

| Attribute | Value |
|-----------|-------|
| Age | 36 |
| Occupation | Automation engineer; maintains cron jobs, CI pipelines, and internal bots that wrap CLI tools |
| Location | Behind a laptop full of terminals; rarely at one for long |
| Tech comfort | high |

## Bio

Sam never types SMAH queries by hand. SMAH runs inside scheduled jobs, pipeline steps, and agent harnesses — non-TTY contexts where a tool that blocks on a prompt or prints banners into captured stdout breaks the build. For Sam the CLI *is* the API: exit codes, stream discipline, and stable output formats are the contract.

## Goals

- Branch on exit codes (0 ok, 2 usage, 3 config, 4 auth, 5 provider) instead of parsing prose.
- Machine-readable output where it exists: `doctor --json` on stdout, plain tables for `history` and `models list`.
- Guarantees that non-`ask` commands never consume stdin, so an inherited open pipe cannot hang a job.
- Documentation and help text that match behavior, so scripts survive upgrades.

## Frustrations

- Tools that hang forever waiting on a TTY that will never come.
- Output formats that change between minor versions without notice.
- Error exits disguised as success (exit 0 after a failure).

## Behaviors

- Wraps everything in `set -euo pipefail`; a wrong exit code stops the pipeline.
- Checks `--help` and `doctor --json` before and after upgrades.
- Prefers explicit flags (`--no-rich`, `--api-key` via env) over interactive anything.

## Job to Be Done

> "When my pipeline calls SMAH unattended, I want deterministic exit codes and clean stdout with stdin never hijacked, so the job either produces the artifact or fails loudly and early."

## Relationship to Product

Sam is the persona the deliberate design decisions serve: stdin consumed only by `ask` (and the legacy/bare paths), notices and panels on stderr, chat honestly refusing with exit 2 rather than faking it, `doctor --json` for scripting. He does not care about rich formatting; he cares that it is off by default in his context.

## Scenarios

- **Scenario 1:** Nightly log digest — a cron job pipes rotated logs through `smah ask "summarize anomalies"` and mails the stdout; a nonzero exit skips the mail.
- **Scenario 2:** Post-upgrade smoke test — CI runs `smah doctor --json` and fails the job if the worst finding exceeds a threshold.
- **Scenario 3:** Runbook automation — a script feeds a file via `--instructions` and greps the answer; usage errors (exit 2) are caught and logged distinctly from provider outages (exit 5).
