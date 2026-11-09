---
id: US-024
title: "Planner answering in prose instead of JSON exits 5"
slug: planner-prose-exits-5
personas: [P-005]
epic: "Providers and API Keys"
priority: must-have
complexity: low
tags: [providers, planner, exit-codes]
---

# US-024: Planner Answering in Prose Instead of JSON Exits 5

## User Story

**As a** automation engineer
**I want to** a planner model that ignores the strict JSON schema to produce a clean exit-5 provider-class error
**So that** a misbehaving model is a visible failure my pipeline can catch, not a mysterious crash or a silently degraded answer.

## Acceptance Criteria

- **Given** the planner model returns prose that fails the strict JSON schema
  **When** the response parses
  **Then** SMAH exits 5 with a message indicating the planning stage failed, not a traceback.
- **Given** the failure
  **When** it renders
  **Then** the answering stage is never invoked with garbage parameters.

## Notes

- The planner schema is strict (title, model, reason, include_settings, format_output, instructions) — a lenient fallback would silently route requests to wrong models, so failure is the honest outcome.
