---
id: US-069
title: "Screen-reader-friendly output structure and verbosity control"
slug: screen-reader-output-structure
personas: [P-006]
epic: "First Run and Onboarding"
priority: could-have
complexity: medium
tags: [accessibility, output, structure]
---

# US-069: Screen-Reader-Friendly Output Structure and Verbosity Control

## User Story

**As a** screen-reader user
**I want to** output that reads sensibly in a linear pass — labeled fields, tables with header rows, panels that announce their title before their content — and a verbosity ladder I can dial up
**So that** I can parse results by ear or braille without visual layout.

## Acceptance Criteria

- **Given** the suggestion panel, history table, or models table in plain mode
  **When** read linearly top to bottom
  **Then** each element is self-describing (column headers present; panel title precedes panel body; labels precede values).
- **Given** content and chrome separated across streams
  **When** the user routes stdout and stderr separately
  **Then** each stream reads coherently on its own.
- **Given** `-v`/`-vv`/`-vvv`
  **When** the user needs more detail
  **Then** progressive detail is available on stderr without affecting stdout content.

## Notes

- Plain ASCII tables with header rows (as `history`/`models list` render) are the pattern to preserve; full-screen redraws or spinners would regress this.
- Extends US-068 (no color-only meaning) from "no lost meaning" to "readable structure".
