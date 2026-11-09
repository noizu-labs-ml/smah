---
id: US-007
title: "Verbose stderr ladder for plans and payloads"
slug: verbose-stderr-ladder
personas: [P-004]
epic: "Command Generation and Output Panel"
priority: could-have
complexity: low
tags: [verbosity, stderr, diagnostics]
---

# US-007: Verbose Stderr Ladder for Plans and Payloads

## User Story

**As a** data engineer debugging a surprising answer
**I want to** dial verbosity with repeatable `-v` flags — plans at `-v`, payloads at `-vv`, a settings dump at `-vvv`
**So that** I can see exactly what was asked and sent, without ever polluting stdout.

## Acceptance Criteria

- **Given** `-v` is passed
  **When** a query or pipe runs
  **Then** the planner's classification appears on stderr.
- **Given** `-vv` is passed
  **When** a query or pipe runs
  **Then** full request/response payloads appear on stderr in addition to the plan.
- **Given** `-vvv` is passed
  **When** the run starts
  **Then** the resolved settings dump appears on stderr.
- **Given** any verbosity level with stdout redirected to a file
  **When** the run completes
  **Then** the file contains the answer only — verbose output never touches stdout.

## Notes

- Verbose payload logging carries prompts/responses, never credentials.
