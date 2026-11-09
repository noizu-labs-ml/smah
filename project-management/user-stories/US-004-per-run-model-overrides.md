---
id: US-004
title: "Per-run model overrides for query, pipe, and answering stages"
slug: per-run-model-overrides
personas: [P-001]
epic: "Command Generation and Output Panel"
priority: should-have
complexity: low
tags: [ask, models, flags]
---

# US-004: Per-Run Model Overrides for Query, Pipe, and Answering Stages

## User Story

**As a** platform engineer tuning cost and quality
**I want to** override the model for a single run — overall (`--model`), query planner (`--model-query`), or pipe planner (`--model-pipe`) — without editing config
**So that** I can experiment or pin a stage for one invocation and leave my saved defaults untouched.

## Acceptance Criteria

- **Given** a configured install
  **When** `smah ask --model openai.gpt-4o-mini "..."` runs
  **Then** the run uses the named fully-qualified model and exits 0 on success.
- **Given** `--model-query` or `--model-pipe` is passed
  **When** the corresponding planning stage resolves
  **Then** the override takes precedence over the probe/default candidate list for that stage only.
- **Given** an override naming an unknown or uncredentialed model
  **When** the stage resolves
  **Then** the run fails with the appropriate typed exit (4 no usable model / 3 unknown provider) rather than silently falling back.

## Notes

- Overrides are per-invocation only; they never persist (persistent changes are a config concern).
- Fully-qualified form is `provider.model`, e.g. `anthropic.claude-haiku-5-5`.
