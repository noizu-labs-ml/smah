---
id: US-009
title: "Best-effort clipboard copy with silent degrade"
slug: best-effort-clipboard-copy
personas: [P-002]
epic: "Command Generation and Output Panel"
priority: should-have
complexity: low
tags: [handoff, clipboard, print-only-cost]
---

# US-009: Best-Effort Clipboard Copy with Silent Degrade

## User Story

**As a** solo sysadmin
**I want to** each suggested command copied to my clipboard as part of showing the panel
**So that** print-only costs me nothing versus my select-with-mouse browser habit.

## Acceptance Criteria

- **Given** `pyperclip` (or a platform clipboard backend) is available
  **When** a suggestion panel renders
  **Then** a best-effort copy of the raw command (no markdown) is attempted and a copy hint is shown.
- **Given** `pyperclip` is not installed
  **When** a suggestion panel renders
  **Then** SMAH silently degrades to print-only — no error, no crash, no missing dependency failure.
- **Given** multiple commands in one answer
  **When** the panels render
  **Then** the copy attempt covers the suggested command(s) shown, and the panel text tells the user to review before running.

## Notes

- `pyperclip` is deliberately an optional, undeclared dependency: absent it, SMAH still works. Making it a formal extra is a candidate packaging story (could-have follow-up).
- The copy is best-effort by design — clipboard tooling varies by platform and must never fail the command.
- Legacy: docs/ux US-11 (supersedes — the shipped design replaces the interactive `[c]opy` prompt with automatic best-effort copy).
