---
id: US-010
title: "Graphical interface"
slug: graphical-interface
personas: [P-002]
epic: "Command Generation and Output Panel"
priority: wont-have
complexity: high
tags: [gui, wont]
---

# US-010: Graphical Interface

## User Story

**As a** product owner
**I want to** record that no GUI exists in SMAH and none is planned
**So that** nobody waits for or builds toward a front end that is out of scope.

## Acceptance Criteria

- **Given** a user passes the removed `--gui` flag
  **When** the parser rejects it
  **Then** the exact message "no GUI exists in SMAH and none is planned" prints with exit 2 (see US-048).
- **Given** any roadmap discussion
  **When** GUI work is proposed
  **Then** this story is the cited decision record.

## Notes

- Legacy: docs/ux E2 (Honesty Pass) — `--gui` advertised a thing that never existed; it was removed, not deferred.
- The CLI is the product; interface investment belongs in output quality and scripting ergonomics.
