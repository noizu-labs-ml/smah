---
id: US-032
title: "config show prints masked config to stdout"
slug: config-show-masked
personas: [P-001, P-006]
epic: "Config Management"
priority: must-have
complexity: low
tags: [config, secrets, stdout]
---

# US-032: config show Prints Masked Config to Stdout

## User Story

**As a** platform engineer auditing a shared machine
**I want to** `smah config show` to print the effective config as YAML with every secret-looking value masked
**So that** I can inspect configuration in tickets and screen-shares without exposing credentials.

## Acceptance Criteria

- **Given** a config containing values under keys like `api_key`, `apikey`, `key`, `token`, `secret`, `password` (case/underscore-insensitive)
  **When** `smah config show` runs
  **Then** those values render masked (not the real values) on stdout.
- **Given** a config value using `$ENV_VAR` indirection
  **When** `config show` renders it
  **Then** it shows as `<env:NAME>` — the indirection is visible, the resolved value is not.
- **Given** the output
  **When** it prints
  **Then** stdout carries only the config (exit 0), safe to redirect.

## Notes

- Legacy: docs/ux US-33 (partial — the redaction half of the landed doctor/config work).
