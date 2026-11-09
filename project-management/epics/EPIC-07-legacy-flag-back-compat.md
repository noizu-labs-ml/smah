---
id: EPIC-07
title: "Legacy Flag Back-Compat"
slug: legacy-flag-back-compat
personas: [P-002, P-003, P-005, P-007]
stories: [US-042, US-043, US-044, US-045, US-046, US-047, US-048]
legacy: "docs/ux E2, E5"
---

# EPIC-07: Legacy Flag Back-Compat

## Description

The 0.2.x subcommand surface kept the old flat-flag invocations working: 8 legacy flags parse, run their new equivalent, and print a one-line `[deprecation] ...` notice on stderr (removal slated for 0.4; message text normative per interface-spec §2). 8 removed flags reject at the parser with exit 2 and exact replacement messages. The result is migration without breakage and no vaporware.

## Goals

- Every legacy invocation behaves identically to before, plus a visible deprecation notice.
- Removed flags fail honestly and name the replacement (`--setup` → "Run 'smah config'"; `--gui` → "no GUI exists in SMAH and none is planned").
- `--interactive` — which never worked in any release — points at `smah chat` and exits 2 rather than pretending.

## User Value

Marcus-style users and Sam-style scripts keep working through the interface migration, while every user is visibly steered to the new surface at a predictable pace.

## Personas Served

P-002 Marcus (muscle memory), P-003 Dana (docs-to-behavior trust), P-005 Sam (script stability), P-007 Ren (script review rigor).

## Stories

| ID | Title | Priority |
|----|-------|----------|
| US-042 | Deprecation notice contract: one line on stderr, equivalent runs | must-have |
| US-043 | -q/--query and top-level -i/--instructions alias ask | must-have |
| US-044 | --continue, --session, and --history alias resume and history | must-have |
| US-045 | --interactive points at smah chat and exits 2 | must-have |
| US-046 | --configure opens the setup wizard | should-have |
| US-047 | --openai-api-key folds into --api-key with its notice | should-have |
| US-048 | Removed flags reject with exact messages and exit 2 | must-have |

## MoSCoW Rollup

must-have: 5 · should-have: 2 · could-have: 0 · wont-have: 0

## Legacy

docs/ux E2 (Honesty Pass) and E5 (Command Surface back-compat table). Supersedes US-38/US-39 with the as-shipped design: rejected flags instead of silent removal, normative message text.
