---
id: US-023
title: "Unknown or malformed provider block exits 3"
slug: malformed-provider-block-exit-3
personas: [P-005]
epic: "Providers and API Keys"
priority: should-have
complexity: low
tags: [providers, config, exit-codes]
---

# US-023: Unknown or Malformed Provider Block Exits 3

## User Story

**As a** automation engineer
**I want to** a hand-edited config with an unknown provider key or malformed block to fail with exit 3 and name the offending section
**So that** config errors are caught before any call and classified apart from auth and network failures.

## Acceptance Criteria

- **Given** a config `providers:` entry naming an unregistered provider
  **When** `get_provider()` resolves it
  **Then** SMAH exits 3 with a message naming the unknown key.
- **Given** a structurally malformed provider block
  **When** it loads
  **Then** SMAH exits 3 identifying the block.
- **Given** candidate probing during model resolution
  **When** one provider's block fails to construct
  **Then** probing skips that candidate with a debug breadcrumb rather than crashing the run.

## Notes

- Tolerant probing (`maybe_get_provider`) vs strict selection (`get_provider`): strict paths exit 3, probe paths skip quietly by design.
