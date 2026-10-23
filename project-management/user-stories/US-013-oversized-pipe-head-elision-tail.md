---
id: US-013
title: "Oversized piped input follows the head-elision-tail contract"
slug: oversized-pipe-head-elision-tail
personas: [P-004]
epic: "Stdin Piping and Summaries"
priority: should-have
complexity: low
tags: [pipe, truncation, contract]
---

# US-013: Oversized Piped Input Follows the Head-Elision-Tail Contract

## User Story

**As a** data engineer
**I want to** know exactly how SMAH handles piped input beyond 2048 characters — a 1024-char head, an elision marker, and a tail in the planning prompt
**So that** the behavior is a documented contract I can reason about, not a surprise.

## Acceptance Criteria

- **Given** piped input of exactly 2048 characters or fewer
  **When** the planner prompt is built
  **Then** the full input is included unmodified.
- **Given** piped input over 2048 characters
  **When** the planner prompt is built
  **Then** the input is represented as a 1024-character head, an elipsis elision marker, and the tail, per the `MAX_PIPE_LENGTH=2048` / `PIPE_HEAD_LENGTH=1024` constants.
- **Given** the constants change in a future release
  **When** they do
  **Then** the help text and docs state the new values (contract, not accident).

## Notes

- The elision is currently invisible to the user and the model; making it *visible* is US-014 (the must-have companion).
- Legacy: docs/ux US-21 (supersedes — the shipped cap is kept as a documented contract rather than removed or chunked this cycle).
