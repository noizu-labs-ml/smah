---
id: US-008
title: "Rich vs plain output follows TTY with explicit overrides"
slug: rich-vs-plain-tty-aware-output
personas: [P-004, P-006]
epic: "Command Generation and Output Panel"
priority: must-have
complexity: low
tags: [output, formatting, accessibility, flags]
---

# US-008: Rich vs Plain Output Follows TTY with Explicit Overrides

## User Story

**As a** pipeline user or screen-reader user
**I want to** get plain text whenever formatting is not wanted — no TTY, or `--no-rich` — and rich markdown only when I am reading interactively
**So that** downstream tools and assistive tech receive clean text.

## Acceptance Criteria

- **Given** stdout is not a TTY and no flag is passed
  **When** output renders
  **Then** formatting defaults to plain (TTY detection), with no ANSI escape sequences.
- **Given** `--no-rich` is passed explicitly
  **When** output renders
  **Then** plain text is produced even on a TTY.
- **Given** `--rich` is passed explicitly
  **When** output renders
  **Then** rich formatting is produced even when piped.
- **Given** plain mode
  **When** any content renders
  **Then** no information is conveyed by color or styling alone.

## Notes

- Legacy: docs/ux US-20 (supersedes — TTY detection is now the default; the flags force either way).
- Plain-mode plainness is the accessibility guarantee (see US-068).
