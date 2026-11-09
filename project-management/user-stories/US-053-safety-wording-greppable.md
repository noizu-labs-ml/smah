---
id: US-053
title: "Safety wording is fixed, greppable, and in --help"
slug: safety-wording-greppable
personas: [P-001, P-006]
epic: "Safety and Print-Only Guarantees"
priority: must-have
complexity: low
tags: [safety, wording, accessibility, help]
---

# US-053: Safety Wording Is Fixed, Greppable, and in --help

## User Story

**As a** user (or an auditor reading over a user's shoulder)
**I want to** the exact sentence "SMAH does not run commands." to appear in every suggestion panel and in the top-level `--help`
**So that** the policy is stated by the output itself, in plain words, without reading docs.

## Acceptance Criteria

- **Given** any invocation producing at least one suggestion
  **When** the panel renders
  **Then** the exact string "SMAH does not run commands." appears (panel header "SUGGESTED COMMAND — NOT EXECUTED BY SMAH" plus the bold statement line).
- **Given** the top-level `--help`
  **When** displayed
  **Then** the same safety sentence appears.
- **Given** an auditor or script
  **When** they grep output or help for the sentence
  **Then** it is found verbatim — the wording is fixed, not paraphrased per release.

## Notes

- Fixed wording is also the accessibility guarantee: the policy exists as text, not styling (US-068).
- Legacy: docs/ux US-14 (supersedes — landed with the 0.2.0 panel).
