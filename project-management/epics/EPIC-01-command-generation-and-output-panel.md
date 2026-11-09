---
id: EPIC-01
title: "Command Generation and Output Panel"
slug: command-generation-and-output-panel
personas: [P-001, P-002, P-003, P-006]
stories: [US-001, US-002, US-003, US-004, US-005, US-006, US-007, US-008, US-009, US-010]
legacy: "docs/ux E1 (partial: handoff flow), E2 (--gui removal)"
---

# EPIC-01: Command Generation and Output Panel

## Description

The core product: `smah ask` turns a plain-language question into a two-stage LLM response — a fast planner model classifies the request in JSON, then the chosen answering model produces the answer. Any `<exec>` command blocks in the reply are extracted and rendered in the "SUGGESTED COMMAND — NOT EXECUTED BY SMAH" panel with the model's title and purpose text and a best-effort clipboard copy. Nothing is ever executed; the user runs the command themselves.

## Goals

- One-shot queries work reliably with formatted or plain output as the context demands.
- The suggestion panel is the complete handoff artifact: title, purpose ("per the model"), command, copy hint.
- Per-run model selection (`--model`, `--model-query`, `--model-pipe`) without editing config.
- No query is an honest usage error (exit 2), never a silent no-op.

## User Value

A user gets a drafted, explained, reviewable command in seconds — with certainty that SMAH cannot act on it. Speed of understanding, not speed of execution, is the value.

## Personas Served

P-001 Priya (reviewable artifact), P-002 Marcus (fast handoff), P-003 Dana (explanations), P-006 Yuki (plain-text panel).

## Stories

| ID | Title | Priority |
|----|-------|----------|
| US-001 | One-shot query via smah ask | must-have |
| US-002 | Two-stage planning pipeline answers every query | must-have |
| US-003 | Suggested-command panel with title, purpose, and no execution | must-have |
| US-004 | Per-run model overrides for query, pipe, and answering stages | should-have |
| US-005 | Instruction file becomes the query | must-have |
| US-006 | ask without a query exits 2 with guidance | must-have |
| US-007 | Verbose stderr ladder for plans and payloads | could-have |
| US-008 | Rich vs plain output follows TTY with explicit overrides | must-have |
| US-009 | Best-effort clipboard copy with silent degrade | should-have |
| US-010 | Graphical interface | wont-have |

## MoSCoW Rollup

must-have: 6 · should-have: 2 · could-have: 1 · wont-have: 1

## Legacy

docs/ux E1 (the print-explain-handoff flow, landed in 0.2.0) and E2 (`--gui` removed with the rejection message "no GUI exists in SMAH and none is planned").
