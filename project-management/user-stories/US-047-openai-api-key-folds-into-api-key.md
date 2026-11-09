---
id: US-047
title: "--openai-api-key folds into --api-key with its notice"
slug: openai-api-key-folds-into-api-key
personas: [P-007]
epic: "Legacy Flag Back-Compat"
priority: should-have
complexity: low
tags: [legacy-flags, keys, back-compat]
---

# US-047: --openai-api-key Folds into --api-key with Its Notice

## User Story

**As a** scripted-user maintainer
**I want to** the old `--openai-api-key KEY` flag to keep working — normalized into the same key channel as `--api-key` — with its deprecation notice
**So that** credential-injecting wrappers keep functioning while migrating.

## Acceptance Criteria

- **Given** `smah ask --openai-api-key KEY "..."` (key supplied via a wrapper, never typed)
  **When** it runs
  **Then** the key is normalized into the same settings field `--api-key` feeds and the request authenticates, plus the `[deprecation]` stderr notice.
- **Given** both `--api-key` and `--openai-api-key` in one invocation
  **When** resolution occurs
  **Then** the canonical `--api-key` takes precedence and both notices (if applicable) render.
- **Given** any path
  **When** keys are handled
  **Then** no key value is echoed in the notice or confirmation.

## Notes

- Mechanically the fold targets the `openai_api_key` field, which the generic settings reader consults for any provider block (see US-019) — so the old flag keeps working for both backends.
