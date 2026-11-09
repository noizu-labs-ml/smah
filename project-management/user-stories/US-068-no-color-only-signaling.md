---
id: US-068
title: "No color-only or ANSI-only meaning in any output"
slug: no-color-only-signaling
personas: [P-006]
epic: "First Run and Onboarding"
priority: must-have
complexity: medium
tags: [accessibility, plain-text, output]
---

# US-068: No Color-Only or ANSI-Only Meaning in Any Output

## User Story

**As a** blind engineer using a screen reader
**I want to** every status, risk, and error to be conveyed in words — never by color, styling, or screen position alone — with clean plain text whenever formatting is off
**So that** my tools give me exactly the information sighted users get.

## Acceptance Criteria

- **Given** `--no-rich` or a non-TTY stdout
  **When** any output renders
  **Then** no ANSI escape sequences appear anywhere in stdout or stderr.
- **Given** a finding with severity (doctor), a risk hint (panel purpose text), or an error
  **When** rendered
  **Then** the meaning is present in the words themselves ("error:", "Risk: IRREVERSIBLE —" style labeling), independent of color.
- **Given** rich mode on a TTY
  **When** color or emphasis is applied
  **Then** it duplicates meaning already present in text — emphasis never carries information alone.

## Notes

- The suggestion panel and exit codes already comply by design; this story pins the rule so future features inherit it.
- Pairs with US-008 (plain mode) and US-053 (fixed wording).
