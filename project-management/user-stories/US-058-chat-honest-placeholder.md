---
id: US-058
title: "chat prints an honest not-available message and exits 2"
slug: chat-honest-placeholder
personas: [P-005]
epic: "Scripting and Exit-Code Contract"
priority: must-have
complexity: low
tags: [chat, honesty, exit-codes, placeholder]
---

# US-058: chat Prints an Honest Not-Available Message and Exits 2

## User Story

**As a** automation engineer probing the surface
**I want to** `smah chat` to print "chat is not available yet — use 'smah ask'" and exit 2
**So that** an unbuilt feature is honestly absent — never faked, never hung, never a silent no-op.

## Acceptance Criteria

- **Given** any context (TTY or not)
  **When** `smah chat` runs
  **Then** the not-available message prints to stderr naming `smah ask` as the working alternative.
- **Given** the invocation
  **When** it completes
  **Then** the exit code is 2 (usage class — the command exists, the feature does not yet) and no session or provider call occurs.

## Notes

- The verb exists so scripts and users discover it now; the honest refusal is the product's "honesty about absence" principle in miniature.
- Legacy: docs/ux E9/US-38 (supersedes — the placeholder *is* the interim behavior; the full REPL is US-062, wont-have this cycle).
