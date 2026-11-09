---
id: P-003
name: "Dana Osei"
slug: dana-dev-into-ops
archetype: "Developer drifting into operations"
segment: primary
tags: [learning, per-flag-explanations, macos-to-linux, incident-support]
---

# P-003: Dana Osei

## Demographics

| Attribute | Value |
|-----------|-------|
| Age | 28 |
| Occupation | Backend developer who owns her service end-to-end, including the box it runs on; comfortable with git and docker, shaky on `ufw`, systemd units, log rotation, TLS renewal |
| Location | Develops on macOS, deploys to Linux |
| Tech comfort | high in her domain, medium in ops |

## Bio

Dana owns a service from commit to host. Commands that differ per OS are a recurring trap. She has pasted semi-blindly from browser AI chat before — once a hallucinated flag (it failed loudly, no harm) — and decided she wants to *learn* the flags, not just receive magic strings.

## Goals

- Intent in, correct command out, with education attached: what each flag does, what the risky parts are.
- Incident support: pipe logs at a problem and get a structured summary.
- Long "here is my situation" prompts from a file instead of retyping.

## Frustrations

- Answers calibrated for the wrong OS after she told the tool her setup.
- Magic strings with no explanation of what they do.
- Tools whose docs promise more than the binary delivers.

## Behaviors

- Reads explanations; wants the "why" per flag.
- Uses instruction files for repeated rituals.
- Pipes incident logs and iterates on the analysis.

## Job to Be Done

> "When a system-administration task is outside my comfort zone, I want the command plus a plain-language explanation of each part, so I solve the problem today and know how to do it myself next time."

## Relationship to Product

Print-only suits her learning goal — execution would teach her nothing. Explanation quality is the product for her. The first-run interview capturing her role and OS facts, and the honest placeholder behavior when something is not implemented (chat says so and exits), match the trust she needs to keep using it.

## Scenarios

- **Scenario 1:** First firewall rule — drafts a `ufw` command and reads the per-flag gloss before running it herself.
- **Scenario 2:** 2 a.m. incident — pipes the service logs in with "summarize errors by module" and acts on the summary.
- **Scenario 3:** Onboarding to TLS — asks for the certbot renewal flow as a learning exercise, instruction file saved for next quarter.
