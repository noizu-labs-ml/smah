---
id: P-002
name: "Marcus Webb"
slug: marcus-solo-sysadmin
archetype: "Solo sysadmin, the whole IT department"
segment: primary
tags: [speed, mixed-os, firefighting, one-liners, print-only-cost]
---

# P-002: Marcus Webb

## Demographics

| Attribute | Value |
|-----------|-------|
| Age | 41 |
| Occupation | The only IT person at a 200-person logistics company — Ubuntu fleet, one Windows file server, a NAS; no change process, he is the process |
| Location | On-site office plus home firefighting in the evenings |
| Tech comfort | high (breadth over depth) |

## Bio

Marcus handles everything with keys for the company. He firefights in the evenings and speed matters more than ceremony. He would honestly prefer a tool that just runs the boring 90% of commands for him — but a near-miss with a wrong-path `rm -rf` years ago left him less cavalier than he sounds.

## Goals

- One-liners for rare tasks (LVM resize, `openssl` renewals, samba quirks) without re-googling.
- Get in, get the command, get out — minimum keystrokes.
- Environment-aware answers without re-explaining that his "linux box" is actually WSL2.

## Frustrations

- Print-only is a **cost** he pays grudgingly — select-with-mouse copy from a browser chat is already his habit, so SMAH must beat it, not match it.
- Silent no-ops; he assumes a hung tool and kills it.
- Setup flows that do not end with a working tool.

## Behaviors

- Lives in the shell; aliases everything he uses twice.
- Skims plain-language explanations instead of reading flags he does not know.
- Resumes sessions for multi-step tasks that span evenings.

## Job to Be Done

> "When a rare admin task lands on my desk, I want the exact command on my clipboard in one keystroke, so I can fix it and get back to my day."

## Relationship to Product

He uses SMAH as a fast command draftsman. The print-only policy is the tension of the whole product: Marcus pays for Priya's constraint. He stays only if the handoff — clipboard copy, paste-ready panel — is instant and reliable; anything slower and browser chat wins.

## Scenarios

- **Scenario 1:** Evening disk-full alert — asks for the LVM extend sequence, copies the command, runs it himself in his own shell.
- **Scenario 2:** Certificate sprint — drafts the openssl renewal for the Windows-adjacent NAS and skims the purpose text instead of re-learning the flags.
- **Scenario 3:** Multi-evening migration — resumes yesterday's session and keeps the context without re-explaining his setup.
