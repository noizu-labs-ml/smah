---
id: P-007
name: "Ren Volkov"
slug: ren-restricted-network-operator
archetype: "Operator behind restricted and unreliable networks"
segment: edge-case
tags: [air-gapped, restricted-network, key-handling, provider-outage, offline-diagnostics]
---

# P-007: Ren Volkov

## Demographics

| Attribute | Value |
|-----------|-------|
| Age | 45 |
| Occupation | Senior sysadmin at a defense contractor; some hosts are fully air-gapped, the rest sit behind egress proxies with allowlists that change weekly |
| Location | On-site secure facility; field sites with poor connectivity |
| Tech comfort | high |

## Bio

Ren's machines are not allowed to talk to much of anything, and when they are, the path out is a proxy that may be down. API keys live in managed environment files, never in shell history. A tool that dials out without saying so — or that cannot tell him cleanly whether its provider was reachable — does not survive his review.

## Goals

- Know precisely which keys SMAH resolved and where each came from, without any key value ever being printed.
- Distinct, quiet failures when a provider is unreachable — no retry storms against a proxy, no hangs.
- Diagnostics that work with the network absent, so he can prove an install's local state is sound before requesting egress.

## Frustrations

- Tools that conflate "bad key" with "network down" — the remediations are completely different.
- Diagnostics that require the very network that is missing.
- Secrets leaking into logs, output, or confirmation text.

## Behaviors

- Sets keys via environment variables injected from a managed store; expects `$ENV_VAR` indirection in config files to be honored.
- Runs offline checks first, network checks only with explicit approval.
- Reads every security-relevant tool's output once, carefully, before trusting it in scripts.

## Job to Be Done

> "When my egress path is down or restricted, I want SMAH to fail fast with the right failure class and to let me verify everything local without any network, so I know exactly what to ask the proxy team for."

## Relationship to Product

SMAH fits Ren's constraints better than most AI tools because the surface is honest about the network: `doctor` performs no network calls, connection failures map to exit 5 distinct from auth's exit 4, key resolution has a documented order (CLI flag, then config with `$ENV` indirection, then `SMAH_<KEY>_API_KEY`, then `<KEY>_API_KEY`), and no code path prints a key.

## Scenarios

- **Scenario 1:** Proxy outage — `smah ask` fails with a provider/network error (exit 5) in seconds, not a minute of retries; the message names the provider.
- **Scenario 2:** Pre-egress review — runs `smah doctor` with no network to confirm config, database, and key *presence* are all sound before filing the allowlist request.
- **Scenario 3:** Key rotation — swaps the environment variable reference in config (`$SMCI_ROTATING_KEY`) and verifies with `config show` that the value stays masked.
