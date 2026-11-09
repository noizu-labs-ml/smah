---
id: US-019
title: "Layered API key resolution order"
slug: layered-api-key-resolution
personas: [P-007]
epic: "Providers and API Keys"
priority: must-have
complexity: medium
tags: [providers, secrets, env, keys]
---

# US-019: Layered API Key Resolution Order

## User Story

**As a** operator in a managed environment
**I want to** keys resolved in one documented order — CLI `--api-key`, then config `settings.api_key` (honoring `$ENV_VAR` indirection), then `SMAH_<KEY>_API_KEY`, then `<KEY>_API_KEY`
**So that** I can control credential precedence explicitly and rotate keys without touching SMAH.

## Acceptance Criteria

- **Given** `--api-key` passed on the CLI
  **When** the provider resolves credentials
  **Then** the CLI key wins for the run.
- **Given** no CLI key but config `settings.api_key` containing `$MY_ROTATING_KEY`
  **When** credentials resolve
  **Then** the referenced environment variable is read at resolution time (indirection honored, literal `$...` string never sent).
- **Given** no CLI or config key
  **When** credentials resolve
  **Then** `SMAH_ANTHROPIC_API_KEY` / `SMAH_OPENAI_API_KEY` are tried, then `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`.
- **Given** any resolution path
  **When** keys are handled
  **Then** no key value is ever printed, logged, or included in error output.

## Notes

- Mechanically, `--api-key` folds into the `openai_api_key` settings field, which the generic settings reader consults for any provider block — behavior matches the "active provider" promise for both backends.
- Rotation story: swap the env var the config references; verify with `config show` (value stays masked, see US-032).
