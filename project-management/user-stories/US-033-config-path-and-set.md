---
id: US-033
title: "config path and config set with type coercion"
slug: config-path-and-set
personas: [P-002]
epic: "Config Management"
priority: should-have
complexity: low
tags: [config, scripting]
---

# US-033: config path and config set with Type Coercion

## User Story

**As a** solo sysadmin
**I want to** `smah config path` to tell me where config lives and `smah config set KEY VALUE` to change one dotted key with sensible bool/int coercion
**So that** I never hand-edit YAML for a one-flag change.

## Acceptance Criteria

- **Given** a standard install
  **When** `smah config path` runs
  **Then** the config file path prints to stdout, exit 0.
- **Given** `smah config set inference.model_picker.default anthropic.claude-opus-5-5`
  **When** it runs
  **Then** the dotted key is written and a confirmation appears (without echoing any secret value).
- **Given** `smah config set user.some_bool_flag true` or `some.count 3`
  **When** it runs
  **Then** the value is coerced to boolean/integer respectively rather than stored as a string.

## Notes

- One key per invocation by design; bulk edits go through `config edit` (US-034).
