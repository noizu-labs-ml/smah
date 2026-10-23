---
id: US-012
title: "Answer-only stdout, chrome on stderr"
slug: answer-only-stdout
personas: [P-004, P-005]
epic: "Stdin Piping and Summaries"
priority: must-have
complexity: medium
tags: [stdout, stderr, discipline, scripting]
---

# US-012: Answer-Only Stdout, Chrome on Stderr

## User Story

**As a** pipeline user
**I want to** nothing but the answer on stdout, with every banner, notice, panel, and diagnostic on stderr
**So that** `smah ask ... > out.md` is safe to redirect and my pipeline never ingests chrome.

## Acceptance Criteria

- **Given** any invocation
  **When** output is produced
  **Then** stdout carries content only — answers, and machine-readable output of `history`, `models list`, `doctor --json`, `config show`/`config path`.
- **Given** notices, deprecation warnings, the suggestion panel, wizard UI, and diagnostics
  **When** they render
  **Then** they go to stderr, never stdout.
- **Given** `smah ask "x" > out.md`
  **When** the run completes
  **Then** the file contains exactly the answer and nothing else.

## Notes

- Legacy: docs/ux US-19 (supersedes — the shipped console split; the old raw-query echo to stdout is gone).
- This discipline is a load-bearing safety property (review §4.6), not just ergonomics.
