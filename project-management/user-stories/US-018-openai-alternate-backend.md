---
id: US-018
title: "OpenAI retained as an alternate backend"
slug: openai-alternate-backend
personas: [P-001]
epic: "Providers and API Keys"
priority: must-have
complexity: medium
tags: [providers, openai, choice]
---

# US-018: OpenAI Retained as an Alternate Backend

## User Story

**As a** platform engineer on an OpenAI-standardized org
**I want to** keep using OpenAI models — selectable per run or as configured defaults
**So that** the Claude-first default never locks me out of my provider.

## Acceptance Criteria

- **Given** an OpenAI key configured (config, `--api-key`, or `OPENAI_API_KEY` env)
  **When** a run selects an `openai.*` model (via `--model`, config defaults, or planner choice)
  **Then** the OpenAI backend completes the request with behavior preserved through the provider abstraction.
- **Given** both providers are credentialed
  **When** the planner picks an answering model
  **Then** it may choose from either provider's enabled catalog entries.
- **Given** the runner code
  **When** it issues requests
  **Then** it never branches on provider identity — everything flows through the `Provider` contract.

## Notes

- Legacy: docs/ux US-31 (partial — "OpenAI remains selectable").
- The vendor-neutral `UNSET` sentinel guards against vendor-specific values leaking into request bodies.
