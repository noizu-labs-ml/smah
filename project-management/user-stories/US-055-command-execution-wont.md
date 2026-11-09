---
id: US-055
title: "Command execution or an execution toggle"
slug: command-execution-wont
personas: [P-001, P-002]
epic: "Safety and Print-Only Guarantees"
priority: wont-have
complexity: high
tags: [safety, print-only, wont, decision-record]
---

# US-055: Command Execution or an Execution Toggle

## User Story

**As a** product owner
**I want to** record that command execution — even opt-in, even sandboxed, even allowlisted — is permanently out of scope
**So that** no setting creeps in and no roadmap item hints at it.

## Acceptance Criteria

- **Given** any proposal to re-add execution behind a flag, confirm, or sandbox
  **When** triaged
  **Then** this story is the decision record: rejected. Reinstatement would require an explicit, explicit-owner decision — not a config default that drifted.
- **Given** Marcus's one-keystroke friction (P-002)
  **When** weighed
  **Then** the product answer is making the handoff cheaper (US-009), not adding execution — an execution toggle would delete Priya (P-001) to save Marcus one keystroke.

## Notes

- Legacy: docs/ux roadmap "Explicitly not doing" and the personas' print-only tension section (the one-sided cost analysis lives there).
- If Marcus-class abandonment ever shows up in data, revisiting is the owner's explicit call — tracked here, not in a flag.
