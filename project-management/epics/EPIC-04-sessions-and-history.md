---
id: EPIC-04
title: "Sessions and History"
slug: sessions-and-history
personas: [P-001, P-002, P-005]
stories: [US-026, US-027, US-028, US-029, US-030, US-031]
legacy: "docs/ux E8"
---

# EPIC-04: Sessions and History

## Description

Sessions persist in sqlite at `~/.smah/smah.db` (migrations auto-applied at startup). `history` prints a plain table (ID, MODIFIED, TITLE) newest-first with `--limit` (default 20); `resume` continues by id or `--last`, merging stored args under `RESERVED_ARG_KEYS` protection. The `smah-db` entry point gives explicit control over migrations.

## Goals

- History is a plain, scriptable listing — missing or uninitialized db is not an error.
- Resume is precise: by id, by most-recent, with honest usage errors (id+`--last` conflict exit 2; unknown id exit 2 listing recent ids).
- Schema changes flow through `smah-db migrate` with status, rollback, and create.

## User Value

Multi-step work keeps context across days; the asking trail Priya needs for tickets is durable and inspectable; automation can list sessions without a TTY.

## Personas Served

P-002 Marcus (multi-evening threads), P-001 Priya (audit trail), P-005 Sam (non-TTY listing).

## Stories

| ID | Title | Priority |
|----|-------|----------|
| US-026 | history lists sessions with --limit | must-have |
| US-027 | history with a missing database is not an error | could-have |
| US-028 | resume by session id with stored-args merge | must-have |
| US-029 | resume --last and the id/--last conflict exit | must-have |
| US-030 | resume of an unknown id exits 2 listing recent sessions | should-have |
| US-031 | smah-db migrate, status, rollback, and create | should-have |

## MoSCoW Rollup

must-have: 3 · should-have: 2 · could-have: 1 · wont-have: 0

## Legacy

docs/ux E8 (Session & History). US-026 supersedes US-26 (the search/`--show`/`--drop` scope shrank to what shipped: table + limit); US-028/029 supersede US-24/US-23 (now `resume` subcommands, legacy flags aliased in EPIC-07).
