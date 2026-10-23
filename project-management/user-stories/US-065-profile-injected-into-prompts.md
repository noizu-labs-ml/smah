---
id: US-065
title: "Profile and system facts are injected into every prompt"
slug: profile-injected-into-prompts
personas: [P-001, P-003]
epic: "First Run and Onboarding"
priority: must-have
complexity: medium
tags: [personalization, prompts, os-awareness]
---

# US-065: Profile and System Facts Are Injected into Every Prompt

## User Story

**As a** user who told SMAH my role and OS once
**I want to** every query to carry my configured profile and detected system facts — unless the planner decides they do not belong
**So that** answers fit my macOS-to-Linux reality without re-explaining it, the actual differentiator versus browser chat.

## Acceptance Criteria

- **Given** a configured `user` and `system` block
  **When** any query or pipe prompt is built
  **Then** the personalization and system context are included (planner `include_settings` may omit them per request, with the reason recorded).
- **Given** Dana on macOS asking about a Linux deploy target
  **When** the answer is drafted
  **Then** the captured facts, not assumptions, anchor the OS-specific parts.
- **Given** a settings dump (`-vvv`)
  **When** inspected
  **Then** the injected profile matches the config — no silent divergence.

## Notes

- The persona/differentiator story: environment awareness without re-explaining (docs/ux P2 "the actual differentiator").
- Depends on system facts being truthful — the historical CPU-as-memory stat bug is the class of regression to guard.
