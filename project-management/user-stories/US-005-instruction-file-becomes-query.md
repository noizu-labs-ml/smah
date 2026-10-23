---
id: US-005
title: "Instruction file becomes the query"
slug: instruction-file-becomes-query
personas: [P-003]
epic: "Command Generation and Output Panel"
priority: must-have
complexity: low
tags: [ask, flags, files]
---

# US-005: Instruction File Becomes the Query

## User Story

**As a** developer with a long "here is my situation" prompt
**I want to** point SMAH at an instruction file instead of typing it out
**So that** repeated rituals are one command.

## Acceptance Criteria

- **Given** an instructions file exists
  **When** `smah ask --instructions FILE` runs with no positional query
  **Then** the file contents become the query and the flow proceeds as a normal ask.
- **Given** both a positional query and `--instructions FILE`
  **When** `ask` runs
  **Then** both inputs are used (query as the question, file as the instruction context) without error.
- **Given** the instructions file does not exist
  **When** `ask` runs
  **Then** a config-class error (exit 3) names the missing path.

## Notes

- Legacy: docs/ux US-08 (supersedes — now `ask`'s own `-i/--instructions`; the top-level `-i` remains a deprecated alias, see US-043).
