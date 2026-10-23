---
id: US-003
title: "Suggested-command panel with title, purpose, and no execution"
slug: suggested-command-panel
personas: [P-001, P-002]
epic: "Command Generation and Output Panel"
priority: must-have
complexity: medium
tags: [ask, safety, output-panel, handoff]
---

# US-003: Suggested-Command Panel with Title, Purpose, and No Execution

## User Story

**As a** sysadmin reviewing an AI-drafted command
**I want to** see each suggested command in a panel titled "SUGGESTED COMMAND — NOT EXECUTED BY SMAH" with the model's title and purpose text
**So that** I can evaluate and run the command myself, certain SMAH has not and cannot.

## Acceptance Criteria

- **Given** the answer contains one or more `<exec>` command blocks
  **When** the response renders
  **Then** each command appears in its own panel headed "SUGGESTED COMMAND — NOT EXECUTED BY SMAH" containing the bold line "SMAH does not run commands.", the model's title, its purpose text explicitly labeled "per the model", and the command in a shell-tagged code block.
- **Given** multiple commands in one answer
  **When** the panels render
  **Then** they appear in document order, one panel per command.
- **Given** any rendering mode (rich or plain, TTY or piped)
  **When** a suggestion is shown
  **Then** the not-executed statement is present in plain words.

## Notes

- Legacy: docs/ux US-09 and US-14 (supersedes both — the shipped panel is the post-E1 handoff flow).
- Risk classification is explanation quality inside the purpose text, presented as model guidance — never a client-side guarantee.
