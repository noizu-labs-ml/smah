---
id: US-031
title: "smah-db migrate, status, rollback, and create"
slug: smah-db-migration-cli
personas: [P-001]
epic: "Sessions and History"
priority: should-have
complexity: medium
tags: [database, migrations, smah-db]
---

# US-031: smah-db migrate, status, rollback, and Create

## User Story

**As a** platform engineer managing state on managed hosts
**I want to** the `smah-db` entry point to expose `migrate` (with `--to N`, `--count N`, `--reset-checksums`), `rollback`, `status`, and `create NAME`, plus a global `--database` path
**So that** schema changes to my session store are explicit and reversible rather than magic.

## Acceptance Criteria

- **Given** pending migrations
  **When** `smah-db migrate` runs
  **Then** migrations apply in order and `status` reflects the new head.
- **Given** a migration that misbehaved
  **When** `smah-db rollback` runs
  **Then** the database steps back per the rollback contract.
- **Given** `smah-db create <name>`
  **When** it runs
  **Then** a new migration file is scaffolded with a correct help string.

## Notes

- Migrations also auto-apply at startup via `init_database` — `smah-db` is the explicit-control surface.
- KNOWN DEFECT (review §9.8): `create`'s help text currently says "Show the current migration status" (copy-paste from `status`). Fix is one line; folded into this story's AC.
- Legacy: docs/ux E2 scope note ("smah-db works, stays as-is").
