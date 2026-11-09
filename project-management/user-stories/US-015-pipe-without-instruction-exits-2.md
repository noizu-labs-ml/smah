---
id: US-015
title: "Pipe without an instruction exits 2"
slug: pipe-without-instruction-exits-2
personas: [P-004, P-005]
epic: "Stdin Piping and Summaries"
priority: must-have
complexity: low
tags: [pipe, exit-codes, usage]
---

# US-015: Pipe without an Instruction Exits 2

## User Story

**As a** automation engineer
**I want to** piping data into SMAH without any instruction to fail fast with exit 2 and an error asking for one
**So that** a broken pipeline step stops immediately instead of guessing an instruction.

## Acceptance Criteria

- **Given** piped stdin present and no query or instructions file
  **When** `smah ask` runs
  **Then** an error asking for an instruction prints to stderr and the exit code is 2.
- **Given** piped stdin and an instruction
  **When** `smah ask` runs
  **Then** pipe mode proceeds normally (see US-011).

## Notes

- Mirrors US-006 (no query at all) — both no-input cases are usage errors, never silent fall-throughs into interactive behavior.
