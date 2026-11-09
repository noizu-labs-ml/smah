---
id: EPIC-09
title: "Scripting and Exit-Code Contract"
slug: scripting-and-exit-codes
personas: [P-003, P-005]
stories: [US-056, US-057, US-058, US-059, US-060, US-061, US-062]
legacy: "docs/ux E3 (exit codes), E5 (bare smah), E9 (chat placeholder)"
---

# EPIC-09: Scripting and Exit-Code Contract

## Description

Every error path routes through `fatal()` with one taxonomy: 0 ok, 1 unexpected internal, 2 usage, 3 config, 4 auth/no usable model, 5 provider/API — documented in the parser epilog. Stdin is consumed only by `ask` and the legacy/bare paths, so no other command can hang on an inherited pipe. `chat` is an honest placeholder (message + exit 2), bare `smah` prints usage and exits 2, and list-style commands keep machine-readable content on stdout.

## Goals

- Scripts branch on exit code alone and are never wrong.
- No non-interactive context ever blocks on a prompt or a hijacked stdin.
- Absent features say so honestly (chat) instead of faking or hanging.

## User Value

SMAH composes into cron, CI, and agent harnesses with the reliability contract of a Unix tool: streams are clean, exits are meaningful, and nothing hangs.

## Personas Served

P-005 Sam (primary), P-003 Dana (her scripts too).

## Stories

| ID | Title | Priority |
|----|-------|----------|
| US-056 | Exit-code contract 0-5 documented and enforced on every path | must-have |
| US-057 | Non-ask commands never consume stdin | must-have |
| US-058 | chat prints an honest not-available message and exits 2 | must-have |
| US-059 | Bare smah prints usage and exits 2 | must-have |
| US-060 | Stable machine-readable stdout for list-style commands | should-have |
| US-061 | Unexpected internal errors exit 1 with the log path | must-have |
| US-062 | Interactive chat REPL | wont-have |

## MoSCoW Rollup

must-have: 5 · should-have: 1 · could-have: 0 · wont-have: 1

## Legacy

docs/ux E3/US-36 (nonzero exits, landed), E5 (bare-smah interim), E9 (chat — deliberately deferred; the honest placeholder is the shipped behavior). US-058/059 supersede US-38.
