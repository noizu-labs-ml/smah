---
id: EPIC-02
title: "Stdin Piping and Summaries"
slug: stdin-piping-and-summaries
personas: [P-004, P-005]
stories: [US-011, US-012, US-013, US-014, US-015, US-016]
legacy: "docs/ux E7"
---

# EPIC-02: Stdin Piping and Summaries

## Description

Piped stdin is the data, the query is the instruction. `smah ask` consumes stdin only in pipe mode; the answer lands on stdout and everything else on stderr. Input larger than 2048 characters is elided in the planning prompt to a 1024-char head, an elision marker, and a tail — currently with no notice to user or model, which this epic makes visible.

## Goals

- `smah ask` is a trustworthy pipeline citizen: answer-only stdout, machine-friendly defaults.
- The truncation contract (MAX_PIPE_LENGTH=2048, PIPE_HEAD_LENGTH=1024) is documented and *visible*.
- Pipe mode without an instruction is a usage error, not a guess.

## User Value

Ines-style users compose SMAH into pipelines knowing exactly what was read, what was cut, and what came back — and can redirect stdout safely.

## Personas Served

P-004 Ines (primary), P-005 Sam (pipeline embedding).

## Stories

| ID | Title | Priority |
|----|-------|----------|
| US-011 | Pipe stdin with an instruction yields a summary on stdout | must-have |
| US-012 | Answer-only stdout, chrome on stderr | must-have |
| US-013 | Oversized piped input follows the head-elision-tail contract | should-have |
| US-014 | Truncation is visible to the user and the model | must-have |
| US-015 | Pipe without an instruction exits 2 | must-have |
| US-016 | Piped input persists with the session for re-query | could-have |

## MoSCoW Rollup

must-have: 4 · should-have: 1 · could-have: 1 · wont-have: 0

## Legacy

docs/ux E7 (Pipe & Output Discipline). US-011/012 supersede docs/ux US-18/US-19 (both now implemented); US-014 closes the US-21/US-22 truncation gap the review confirms still exists.
