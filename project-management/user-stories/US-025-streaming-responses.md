---
id: US-025
title: "Streaming responses"
slug: streaming-responses
personas: [P-004]
epic: "Providers and API Keys"
priority: wont-have
complexity: high
tags: [providers, streaming, wont]
---

# US-025: Streaming Responses

## User Story

**As a** product owner
**I want to** record that token streaming is deliberately not offered this cycle
**So that** the two-stage pipeline's planner→answering handoff stays simple and the print-only output model stays coherent.

## Acceptance Criteria

- **Given** a user request for streaming output
  **When** triaged
  **Then** this story is the decision record: not planned for the current cycle.
- **Given** the current architecture
  **When** a response completes
  **Then** the full text is required before `<exec>` extraction and panel rendering, which streaming would complicate.

## Notes

- Legacy: docs/ux E6 scope note ("Out: streaming").
- Revisit only if interactive chat (US-062) ever ships — streaming is a chat-era feature, not a draft-and-pipe feature.
