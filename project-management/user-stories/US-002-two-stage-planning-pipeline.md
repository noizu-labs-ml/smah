---
id: US-002
title: "Two-stage planning pipeline answers every query"
slug: two-stage-planning-pipeline
personas: [P-001, P-003]
epic: "Command Generation and Output Panel"
priority: must-have
complexity: medium
tags: [ask, pipeline, planner, providers]
---

# US-002: Two-Stage Planning Pipeline Answers Every Query

## User Story

**As a** platform engineer
**I want to** know that every query flows through a fast planner stage that classifies my request and picks the answering model
**So that** responses land on a model suited to the question, with my settings included only when relevant.

## Acceptance Criteria

- **Given** a query on a configured install
  **When** `smah ask` runs
  **Then** a fast planner model first returns a strict-JSON classification (title, answering model, extra instructions, include-settings flag, formatted-vs-raw flag) before the answering model responds.
- **Given** the planner decides system/user settings do not belong in the prompt
  **When** the answering model is called
  **Then** the personalization block is omitted for that request.
- **Given** the planner returns prose instead of valid JSON
  **When** the response is parsed
  **Then** SMAH exits 5 with a clean provider-class error (see US-024).

## Notes

- Planner model resolution per task (`query`/`pipe`): CLI override first, then candidate probing for constructibility and credentials, first usable wins.
- `-v` reveals the plan on stderr; `-vv` adds payloads.
