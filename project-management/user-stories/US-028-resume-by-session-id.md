---
id: US-028
title: "resume by session id with stored-args merge"
slug: resume-by-session-id
personas: [P-002]
epic: "Sessions and History"
priority: must-have
complexity: medium
tags: [resume, sessions, flags]
---

# US-028: Resume by Session Id with Stored-Args Merge

## User Story

**As a** solo sysadmin running parallel threads of work
**I want to** `smah resume <id>` to restore a specific session and continue it with context
**So that** multi-step tasks stay separate and resumable.

## Acceptance Criteria

- **Given** a known session id
  **When** `smah resume <id>` runs
  **Then** the session's messages and stored arguments are restored and the interactive continue-flow proceeds on stderr.
- **Given** a resumed session
  **When** stored args merge with current CLI args
  **Then** the `RESERVED_ARG_KEYS` (command, deprecated) are never overwritten by stored values.
- **Given** piped stdin during a resume
  **When** the session continues
  **Then** stdin is consumed only by the ask/legacy flow rules — resume never blocks on a foreign pipe.

## Notes

- Legacy: docs/ux US-24 (supersedes; the stored-args `merge_args()` behavior moved here from US-23's acceptance list).
