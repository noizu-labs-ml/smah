---
id: US-050
title: "exec tags are parsed for display, never run"
slug: exec-tags-parsed-never-run
personas: [P-001, P-002]
epic: "Safety and Print-Only Guarantees"
priority: must-have
complexity: low
tags: [safety, response-parser, print-only]
---

# US-050: exec Tags Are Parsed for Display, Never Run

## User Story

**As a** sysadmin reading a suggested command
**I want to** the model's `<exec>` markup to be parsed into display data — nothing more
**So that** the richest part of the model output is still just text on my screen.

## Acceptance Criteria

- **Given** a model reply containing `<exec shell=...>` blocks with title, purpose, and command children
  **When** `ResponseParser.extract_commands()` runs
  **Then** it returns plain dicts for panel rendering — no shell invocation, no condition evaluation that can run anything.
- **Given** malformed or hostile markup in the model output
  **When** parsing runs
  **Then** the worst outcome is a rendering artifact or skipped block — never process creation.

## Notes

- The parser uses lxml/cssselect on the model's text; its output is data, and the only consumer is the panel renderer.
- Legacy: docs/ux US-09 (partial — extraction half).
