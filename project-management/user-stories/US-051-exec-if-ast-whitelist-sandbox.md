---
id: US-051
title: "exec-if conditions are sandboxed by an ast whitelist"
slug: exec-if-ast-whitelist-sandbox
personas: [P-001]
epic: "Safety and Print-Only Guarantees"
priority: must-have
complexity: medium
tags: [safety, security, ast, exec-if]
---

# US-051: exec-if Conditions Are Sandboxed by an ast Whitelist

## User Story

**As a** platform engineer
**I want to** model-supplied `exec-if` display conditions parsed with `ast` and walked over a strict whitelist — constants, names, not/and/or, comparisons, in/not in, JSON-style true/false/null — instead of `eval()`
**So that** a malicious or confused model string can never execute arbitrary code on my machine.

## Acceptance Criteria

- **Given** an `exec-if` condition composed only of whitelisted constructs
  **When** the condition is evaluated
  **Then** the result determines whether the command's panel shows, as intended.
- **Given** a condition containing calls, attribute access, subscripts, or unknown names
  **When** the walker encounters it
  **Then** the command is *skipped* with a notice on stderr — never evaluated, never run.
- **Given** the pre-0.2.0 `eval()` implementation
  **When** compared with the current one
  **Then** no code-execution vector remains for model-controlled strings.

## Notes

- This was the 0.2.0 security fix (eval → ast whitelist walker); the skip-with-notice behavior is deliberate fail-closed design.
- Legacy: docs/ux E1 (the exec-if deletion evolved into sandboxing rather than full removal — the conditions now only gate display).
