---
id: US-014
title: "Truncation is visible to the user and the model"
slug: truncation-visible
personas: [P-004]
epic: "Stdin Piping and Summaries"
priority: must-have
complexity: medium
tags: [pipe, truncation, honesty, gap]
---

# US-014: Truncation Is Visible to the User and the Model

## User Story

**As a** data engineer
**I want to** a loud notice when my piped input was cut, and the model to be told its input is partial
**So that** I never trust a summary of data SMAH did not read.

## Acceptance Criteria

- **Given** piped input exceeding the 2048-char cap
  **When** the run proceeds
  **Then** a stderr notice states that input was truncated, including bytes supplied versus bytes included (head+tail).
- **Given** the elided prompt is built
  **When** the model receives it
  **Then** the prompt itself states the input is partial, so summaries can say so.
- **Given** truncation occurred but the run succeeded
  **When** the command exits
  **Then** the exit code remains 0 — truncation is a warning, not an error.
- **Given** input within the cap
  **When** the run completes
  **Then** no truncation notice appears.

## Notes

- KNOWN GAP (review §9.9): today the elision is silent for both user and model — this story closes it. It is the honest-behavior gap Ines-class users will hit first.
- Legacy: docs/ux US-21/US-22 (supersedes both — narrowed from configurable caps/chunking to visible truncation of the shipped contract).
