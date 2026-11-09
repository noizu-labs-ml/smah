---
id: P-001
name: "Priya Raman"
slug: priya-platform-engineer
archetype: "Platform engineer under strict change control"
segment: primary
tags: [change-control, linux-fleet, ticketing, audit-trail, print-only]
---

# P-001: Priya Raman

## Demographics

| Attribute | Value |
|-----------|-------|
| Age | 34 |
| Occupation | Platform engineer, payments-adjacent SaaS; owns a ~40-host Linux fleet behind SOC 2 change control |
| Location | Works from a macOS laptop through hardened jump hosts to Linux targets |
| Tech comfort | high |

## Bio

Priya keeps a regulated production fleet running. Every production change needs a reviewed ticket: the exact command, what it does, what it touches, rollback. She knows her field well but not every flag of every tool (`journalctl`, `openssl`, `iptables` vary across distros). Policy, not preference, forbids any tool from executing anything on production hosts without human review.

## Goals

- Draft flag-exact commands faster than searching man pages and internal wikis.
- Produce a reviewable artifact per suggestion — command, plain-language explanation, risk guidance — pasteable straight into a change ticket.
- Keep a trail of what she asked and what was suggested (sqlite sessions already provide this).

## Frustrations

- Hand-writing the "what this does / risk" section of every ticket.
- Browser AI chats that know nothing about her target OS or her role.
- Tools that blur the line between suggesting and doing — any execution is an audit finding.

## Behaviors

- Treats the model's output as draft text to review, never as a plan to auto-apply.
- Reads the explanation before the command; expects risk guidance labeled as guidance, not a guarantee.
- Resumes sessions to keep a thread of reasoning per incident.

## Job to Be Done

> "When I need a fleet-safe command for a change ticket, I want a drafted command with an explanation and risk guidance I can paste for review, so I can move fast without ever letting a tool touch production."

## Relationship to Product

SMAH's print-only design is a **requirement** for her, not a limitation: a tool that executes would be uninstalled the same day. The "SUGGESTED COMMAND — NOT EXECUTED BY SMAH" panel is exactly the artifact she wants; the two-stage pipeline and session history give her an auditable asking trail. Any execution toggle would delete this persona.

## Scenarios

- **Scenario 1:** Change ticket at speed — drafts a log-retention command for journalctl with an explanation she can paste into the review ticket, in under a minute.
- **Scenario 2:** Distro drift — asks for an openssl renewal procedure and gets one tailored to the OS facts captured in her config, not a generic answer.
- **Scenario 3:** Audit follow-up — resumes the session from last week to show exactly what was asked and what was suggested.
