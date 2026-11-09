---
id: US-059
title: "Bare smah prints usage and exits 2"
slug: bare-smah-usage-exit-2
personas: [P-005]
epic: "Scripting and Exit-Code Contract"
priority: must-have
complexity: low
tags: [usage, exit-codes, first-impression]
---

# US-059: Bare smah Prints Usage and Exits 2

## User Story

**As a** new user typing `smah` with no arguments
**I want to** a concise usage block and the chat notice on stderr with exit 2
**So that** my very first command teaches me the surface instead of doing nothing.

## Acceptance Criteria

- **Given** no command, no flags, and no piped stdin
  **When** `smah` runs
  **Then** a concise usage block plus the chat placeholder notice print to stderr and the exit code is 2.
- **Given** `smah` invoked with piped stdin and no query
  **When** it runs
  **Then** the error asks for an instruction (the legacy pipe path), exit 2 — never a hang.

## Notes

- KNOWN GAP adjacent: the README's "First Run" section still says bare `smah` starts setup — the code deliberately does not (fresh-install hardening, 0.2.1). See US-066.
- Legacy: docs/ux US-38 (supersedes — the pre-0.2 bare invocation fell into a silent interactive stub).
