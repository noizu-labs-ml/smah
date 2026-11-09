---
id: US-064
title: "First-run wizard captures user profile and OS facts"
slug: wizard-captures-profile-and-os-facts
personas: [P-003]
epic: "First Run and Onboarding"
priority: must-have
complexity: medium
tags: [onboarding, wizard, personalization]
---

# US-064: First-Run Wizard Captures User Profile and OS Facts

## User Story

**As a** new user
**I want to** the `smah config` wizard to interview me once — name, admin experience, role, background — and to auto-detect my OS facts
**So that** every later answer is tailored to who I am and what machine I am on.

## Acceptance Criteria

- **Given** a fresh install on a real TTY
  **When** the wizard runs
  **Then** it captures the `user` block (name, system_admin_experience, role, about) into `~/.smah/config.yaml`.
- **Given** the machine's platform
  **When** the wizard reaches system detection
  **Then** OS facts (per-platform linux/darwin/bsd/windows readers, plus psutil cpu/memory/disk stats) are captured into the `system` block.
- **Given** the user prefers not to answer a question
  **When** they leave it empty
  **Then** the empty field degrades gracefully in prompts (no empty-string persona noise).

## Notes

- The wizard's y/n auto-detect question must accept explicit input (the historical "any input = yes" bug is the regression to watch).
- Legacy: docs/ux US-04 (partial — skippable optional questions shipped with the bounded interview).
