---
id: P-006
name: "Yuki Tanaka"
slug: yuki-screen-reader-user
archetype: "Blind engineer using a screen reader"
segment: edge-case
tags: [accessibility, screen-reader, plain-text, no-color-only, verbosity]
---

# P-006: Yuki Tanaka

## Demographics

| Attribute | Value |
|-----------|-------|
| Age | 29 |
| Occupation | QA engineer and shell power user; blind, navigates entirely by screen reader and braille display |
| Location | Urban, fully remote team |
| Tech comfort | high |

## Bio

Yuki lives in the terminal by choice — it is the most accessible interface she has. Screen readers linearize text, so anything conveyed only by color, styling, or screen position is invisible to her. Rich ANSI formatting is not just noise: escape sequences can garble braille output or be read as garbage characters.

## Goals

- Every status, risk, and error conveyed in words, never color alone.
- Clean plain text on demand — no ANSI escapes when formatting is off.
- Output that reads sensibly linearly: headings, labeled fields, tables with header rows.

## Frustrations

- "The red line" — status that only exists as a color.
- Panels that only make sense visually (aligned columns with no headers, spinners).
- Formatted output forced on with no plain-text escape hatch.

## Behaviors

- Runs with `--no-rich` (or non-TTY defaults) as her normal mode.
- Routes stderr and stdout separately so the screen reader announces content and chrome in order.
- Prefers a verbosity ladder she can dial up when she needs the details.

## Job to Be Done

> "When a tool prints a result, I want every part of it expressed in labeled plain text with no color-only or position-only meaning, so my screen reader gives me the same information sighted users get."

## Relationship to Product

SMAH's existing discipline is nearly right for her: `--rich/--no-rich`, TTY-detection default, stderr for chrome, plain tables for `history` and `models list`. The product earns her trust by keeping all meaning in words — the "NOT EXECUTED BY SMAH" panel text, risk guidance as a labeled line — and never regressing to color-only signaling.

## Scenarios

- **Scenario 1:** Drafting a command — reads the suggestion panel linearly: title, purpose, the command itself, the copy hint; nothing depends on emphasis.
- **Scenario 2:** Diagnosing a failure — raises verbosity (`-vv`) and listens to the stderr ladder to hear the planner's decisions before the error.
- **Scenario 3:** Reviewing history — consumes the `history` table with a braille display; column headers make the rows self-describing.
