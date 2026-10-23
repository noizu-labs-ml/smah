---
id: US-062
title: "Interactive chat REPL"
slug: interactive-chat-repl
personas: [P-003]
epic: "Scripting and Exit-Code Contract"
priority: wont-have
complexity: high
tags: [chat, repl, wont, deferred]
---

# US-062: Interactive Chat REPL

## User Story

**As a** developer exploring a problem conversationally
**I want to** a real multi-turn `smah chat` REPL — persisted from the first turn, resumable, on the default chat model
**So that** cold-start exploration does not require a fake first question.

## Acceptance Criteria

- **Given** the current cycle
  **When** triaged
  **Then** this story is the decision record: deferred. `smah chat` remains the honest exit-2 placeholder (US-058).
- **Given** a future cycle that builds it
  **When** it ships
  **Then** each turn persists and resumes via `smah resume`, and US-049's no-execution contract holds inside the loop.

## Notes

- Legacy: docs/ux E9/US-37 (supersedes — deliberately deferred; the resume loop machinery is the designated implementation path when the owner greenlights it).
- Bare `smah` entering chat is part of the deferred scope (the shipped behavior is US-059's usage + exit 2).
