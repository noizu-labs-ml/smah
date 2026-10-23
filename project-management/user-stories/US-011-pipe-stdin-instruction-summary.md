---
id: US-011
title: "Pipe stdin with an instruction yields a summary on stdout"
slug: pipe-stdin-instruction-summary
personas: [P-004]
epic: "Stdin Piping and Summaries"
priority: must-have
complexity: medium
tags: [pipe, stdin, summaries]
---

# US-011: Pipe Stdin with an Instruction Yields a Summary on Stdout

## User Story

**As a** data engineer
**I want to** pipe logs into SMAH with an instruction and get the processed result on stdout
**So that** SMAH is a pipeline stage like jq.

## Acceptance Criteria

- **Given** data on stdin and a query on the command line
  **When** `kubectl logs svc/api | smah ask "summarize errors by service"` runs
  **Then** the piped content is treated as the data and the query as the instruction, and the summary prints to stdout.
- **Given** the pipe completes
  **When** the session persists
  **Then** the piped input is stored so the analysis can be re-queried via resume.

## Notes

- Legacy: docs/ux US-18 (supersedes — pipe mode is the shipped behavior this story pins down).
- Stdin is consumed only by `ask` and the legacy/bare paths (see US-057).
