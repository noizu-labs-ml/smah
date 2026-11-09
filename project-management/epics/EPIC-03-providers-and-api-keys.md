---
id: EPIC-03
title: "Providers and API Keys"
slug: providers-and-api-keys
personas: [P-001, P-003, P-005, P-007]
stories: [US-017, US-018, US-019, US-020, US-021, US-022, US-023, US-024, US-025]
legacy: "docs/ux E6"
---

# EPIC-03: Providers and API Keys

## Description

A provider abstraction with two backends: **anthropic** (Claude, the working default) and **openai** (retained alternate). The planner model is resolved per stage (`query`/`pipe`) by probing candidates for constructibility and credentials; the planner then picks the answering model per request via a strict JSON schema. Key resolution is layered and never printed. Every failure class maps to a distinct exit code.

## Goals

- A fresh install with only `ANTHROPIC_API_KEY` set works out of the box (Claude defaults).
- Keys resolve in documented order — CLI `--api-key`, config `settings.api_key` (with `$ENV_VAR` indirection), `SMAH_<KEY>_API_KEY`, `<KEY>_API_KEY` — and are never printed.
- Errors are typed: missing key exit 4, auth exit 4, provider/network exit 5, malformed provider config exit 3, planner prose exit 5.

## User Value

Users on either major provider get working answers with org-standard keys, and scripts can tell a bad key from a dead network by exit code alone.

## Personas Served

P-003 Dana (Anthropic by default), P-001 Priya (org key standards), P-007 Ren (key handling, restricted networks), P-005 Sam (failure-class branching).

## Stories

| ID | Title | Priority |
|----|-------|----------|
| US-017 | Claude works out of the box as default provider | must-have |
| US-018 | OpenAI retained as an alternate backend | must-have |
| US-019 | Layered API key resolution order | must-have |
| US-020 | Missing key preflight fails with exit 4 before any network call | must-have |
| US-021 | Provider auth failure maps to exit 4 and names the provider | must-have |
| US-022 | Provider and network failures map to exit 5 | must-have |
| US-023 | Unknown or malformed provider block exits 3 | should-have |
| US-024 | Planner answering in prose instead of JSON exits 5 | must-have |
| US-025 | Streaming responses | wont-have |

## MoSCoW Rollup

must-have: 7 · should-have: 1 · could-have: 0 · wont-have: 1

## Legacy

docs/ux E6 (Provider Abstraction). US-017/018 supersede docs/ux US-31; US-020 supersedes US-05; US-021/022 supersede US-35.
