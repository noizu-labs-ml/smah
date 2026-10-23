---
id: EPIC-10
title: "First Run and Onboarding"
slug: first-run-and-onboarding
personas: [P-001, P-003, P-006, P-007]
stories: [US-063, US-064, US-065, US-066, US-067, US-068, US-069, US-070]
legacy: "docs/ux E4"
---

# EPIC-10: First Run and Onboarding

## Description

The first-run interview captures the user's background (name, experience, role, about) and auto-detected OS facts, injected into every prompt so answers fit the operator and their machine. Missing config on `ask`/`resume` fails cleanly with exit 3 and a pointer to `smah config` — never an EOF-crashing wizard. Accessibility (plain-text, no color-only meaning) and restricted-network failure clarity are onboarding-quality concerns, not afterthoughts. Known gap: the README still says bare `smah` starts setup while the code prints usage and exits 2.

## Goals

- A configured install produces answers that reflect the user's stated profile and detected system.
- First-run failure is loud, typed (exit 3), and names the fix.
- Output is consumable by screen readers; provider reachability failures are distinct and quiet.

## User Value

New users reach a working, personalized tool — and when they cannot, they know exactly why and what to run. Edge-case users (accessibility, restricted networks) are first-class.

## Personas Served

P-003 Dana (personalized learning), P-001 Priya (OS-tailored drafts), P-006 Yuki (accessible from first run), P-007 Ren (clean failures, key hygiene).

## Stories

| ID | Title | Priority |
|----|-------|----------|
| US-063 | Missing config exits 3 and points at smah config | must-have |
| US-064 | First-run wizard captures user profile and OS facts | must-have |
| US-065 | Profile and system facts are injected into every prompt | must-have |
| US-066 | First-run entry point matches the README (bare smah gap) | should-have |
| US-067 | Setup discloses data locations and egress | could-have |
| US-068 | No color-only or ANSI-only meaning in any output | must-have |
| US-069 | Screen-reader-friendly output structure and verbosity control | could-have |
| US-070 | Unreachable provider fails fast with exit 5 and a clear message | must-have |

## MoSCoW Rollup

must-have: 5 · should-have: 1 · could-have: 2 · wont-have: 0

## Legacy

docs/ux E4 (Onboarding & First Run — the exit-4 keyless path and catalog refresh landed; the wizard TTY gate and exit-3 missing-config behavior shipped in 0.2.1). US-063 partially supersedes US-01; US-067 supersedes US-06 (narrowed to what ships).
