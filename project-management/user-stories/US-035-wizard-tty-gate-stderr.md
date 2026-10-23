---
id: US-035
title: "Wizard requires a real TTY, else exit 3; UI on stderr"
slug: wizard-tty-gate-stderr
personas: [P-005, P-006]
epic: "Config Management"
priority: must-have
complexity: low
tags: [config, wizard, non-interactive, stderr]
---

# US-035: Wizard Requires a Real TTY, Else Exit 3; UI on Stderr

## User Story

**As a** automation engineer
**I want to** the setup wizard to refuse to launch without a real terminal and render all of its UI on stderr
**So that** piped or scheduled invocations never hang on prompts and interactive setup never pollutes captured stdout.

## Acceptance Criteria

- **Given** no real TTY on stdin (piped, or under a job runner)
  **When** `smah config` (wizard path) runs
  **Then** it exits 3 with a config-class message instead of blocking on prompts.
- **Given** a real TTY
  **When** the wizard runs
  **Then** every panel, prompt, and progress line renders on stderr; stdout stays clean.
- **Given** piped stdin while a config command would launch the wizard
  **When** it runs
  **Then** the TTY gate blocks the launch — an inherited open pipe can never deadlock the wizard.

## Notes

- The TTY gate is a safety property (review §5), not a nicety: it is what makes the wizard safe to have on PATH in scripts.
- Related scripting guarantee: US-057 (stdin discipline).
