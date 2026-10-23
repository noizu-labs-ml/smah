---
id: US-030
title: "resume of an unknown id exits 2 listing recent sessions"
slug: resume-unknown-id-exits-2
personas: [P-002]
epic: "Sessions and History"
priority: should-have
complexity: low
tags: [resume, exit-codes, errors]
---

# US-030: Resume of an Unknown Id Exits 2 Listing Recent Sessions

## User Story

**As a** solo sysadmin who fat-fingered a session id
**I want to** the failure to list my recent session ids
**So that** I can pick the right one from the error message itself.

## Acceptance Criteria

- **Given** a session id with no matching row
  **When** `smah resume <id>` runs
  **Then** an exit-2 error prints listing recent session ids (the same data `history` shows).
- **Given** the error output
  **When** it renders
  **Then** it goes to stderr, keeping stdout clean for scripting.

## Notes

- The 0.1.x behavior for unknown ids was in the silent-failure class; exit 2 + a useful listing is the corrected contract (review §2 resume row).
