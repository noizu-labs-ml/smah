---
id: US-017
title: "Claude works out of the box as default provider"
slug: claude-default-out-of-the-box
personas: [P-003]
epic: "Providers and API Keys"
priority: must-have
complexity: medium
tags: [providers, anthropic, defaults]
---

# US-017: Claude Works Out of the Box as Default Provider

## User Story

**As a** developer whose org standardized on Anthropic
**I want to** set only `ANTHROPIC_API_KEY` and have SMAH work with Claude models immediately
**So that** I never touch OpenAI configuration to get a first answer.

## Acceptance Criteria

- **Given** a fresh install with only `ANTHROPIC_API_KEY` set
  **When** `smah ask "..."` runs
  **Then** a working Claude query completes out of the box (claude-haiku-5-5 as planner/picker default, claude-opus-5-5 enabled in the catalog).
- **Given** zero OpenAI references in config
  **When** any query runs
  **Then** no OpenAI client is constructed and no OpenAI key is consulted.
- **Given** the Anthropic backend
  **When** a request is built
  **Then** the system prompt is sent as the top-level `system` parameter (never a system-role message) with a per-call output budget.

## Notes

- Legacy: docs/ux US-31 (supersedes — the provider abstraction and Claude defaults landed in the 0.2.x recovery).
- Anthropic responses are parsed block-by-block (`type == "text"` only); `budget_tokens`/`temperature`/`top_p` are deliberately not sent (400s on current models).
