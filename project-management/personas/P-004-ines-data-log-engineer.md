---
id: P-004
name: "Ines Duarte"
slug: ines-data-log-engineer
archetype: "Data engineer and log wrangler"
segment: secondary
tags: [pipelines, stdout-discipline, egress, truncation-honesty, machine-readable]
---

# P-004: Ines Duarte

## Demographics

| Attribute | Value |
|-----------|-------|
| Age | 31 |
| Occupation | Data engineer; pipelines, k8s logs, access logs. Lives in the shell and treats every tool as a pipeline stage |
| Location | Remote; clusters and notebooks on tap |
| Tech comfort | high |

## Bio

Ines's primary interaction with any tool is `kubectl logs ... | <tool> | <next tool>`. Output discipline is not a nicety, it is the contract. Logs she pipes can contain tokens and user data, so she thinks about egress constantly. She rarely wants a command *run* — she wants text *transformed*.

## Goals

- `kubectl logs svc/api | smah ask "summarize errors by service"` producing clean output on stdout, nothing else.
- Iterate on an analysis by resuming the session.
- Know exactly when her input was truncated and what was sent to the provider.

## Frustrations

- Anything on stdout that is not the answer — that class of defect is disqualifying.
- Silent truncation: a confidently wrong summary of data the tool never read.
- No visibility into what data leaves the machine.

## Behaviors

- Chains tools; expects exit codes and stream discipline to survive piping.
- Forces `--no-rich` for downstream tools, rich only for her own eyeballs.
- Redirects answers into files and diffs them.

## Job to Be Done

> "When I pipe a log into a tool, I want only the answer on stdout and honest notice if the input was cut, so the result composes in my pipeline and I never trust a summary of data that was not read."

## Relationship to Product

SMAH's pipe path is her whole product: stdin as data, query as instruction. The stdout/stderr split, TTY-aware formatting default, and session persistence are her gating requirements. Command execution is irrelevant to her risk axis — data egress and silent truncation are what she watches.

## Scenarios

- **Scenario 1:** Incident triage — pipes 6,000 lines of pod logs in and asks for a markdown summary; a truncation notice tells her how much was actually read.
- **Scenario 2:** Pipeline stage — embeds SMAH between `jq` and her dashboard feed with `--no-rich`, machine-readable output only.
- **Scenario 3:** Refinement loop — resumes the session to narrow "summarize errors" to "only 5xx after 14:00".
