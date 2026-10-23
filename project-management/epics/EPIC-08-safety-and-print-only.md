---
id: EPIC-08
title: "Safety and Print-Only Guarantees"
slug: safety-and-print-only
personas: [P-001, P-002, P-006]
stories: [US-049, US-050, US-051, US-052, US-053, US-054, US-055]
legacy: "docs/ux E1"
---

# EPIC-08: Safety and Print-Only Guarantees

## Description

The executor is gone, not gated: 0.2.0 removed `subprocess.run(cmd, shell=True)` behind its y/n confirm from query and resume flows with no opt-in toggle. `<exec>` tags are parsed, never run. `exec-if` conditions — formerly `eval()`-ed model strings — are now parsed with `ast` and walked over a strict whitelist; anything unsafe skips the command with a stderr notice. CI enforces the invariant with a grep guard, and the panel wording is fixed and greppable.

## Goals

- Zero code paths execute model-generated shell; no toggle exists or may creep in.
- Model-supplied condition logic is sandboxed to a whitelist AST walker.
- The safety invariant is mechanically enforced (CI grep + tests) and user-visibly stated.

## User Value

Priya-class users can put SMAH on production-adjacent machines at all; everyone else gets a tool whose most dangerous failure mode is a bad suggestion on a screen, not a command in a shell.

## Personas Served

P-001 Priya (the requirement), P-002 Marcus (trust despite wanting execution), P-006 Yuki (safety stated in words, not styling).

## Stories

| ID | Title | Priority |
|----|-------|----------|
| US-049 | No execution path exists; print-only has no toggle | must-have |
| US-050 | exec tags are parsed for display, never run | must-have |
| US-051 | exec-if conditions are sandboxed by an ast whitelist | must-have |
| US-052 | CI grep guard enforces the print-only invariant | must-have |
| US-053 | Safety wording is fixed, greppable, and in --help | must-have |
| US-054 | Delete dead executor-era code | could-have |
| US-055 | Command execution or an execution toggle | wont-have |

## MoSCoW Rollup

must-have: 5 · should-have: 0 · could-have: 1 · wont-have: 1

## Legacy

docs/ux E1 (Safety & Trust, landed in 0.2.0 including the eval→ast security fix) and the roadmap's "explicitly not doing" list. US-049 supersedes docs/ux US-10; US-053 supersedes US-14.
