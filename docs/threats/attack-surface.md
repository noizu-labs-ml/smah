# Attack Surface — Full Enumeration (STRIDE)

Details for [../THREAT-MODEL.md](../THREAT-MODEL.md). Severity/status per
vulnerability live in the main register; this file is the per-boundary
analysis.

## Inputs (ingress)

| Ingress | Trust | Notes |
|---------|-------|-------|
| CLI args | Trusted (local user) | argparse; no dynamic evaluation |
| Piped stdin | **Untrusted data** | Read only for `ask`/bare path; becomes prompt content (`--- INPUT ---`) |
| `--instructions FILE` | Untrusted data | File contents become the query |
| `~/.smah/config.yaml` | Trusted-ish (user-owned) | `yaml.safe_load` everywhere; `$ENV_VAR` indirection |
| Model responses | **Untrusted** | Parsed as XML, rendered, clipboard-copied |
| Env vars / `.envrc` | Trusted environment | direnv executes `.envrc.secret`/`.envrc.dev` shell — user-owned trust |

## B1 — Local host ↔ process/files

**Spoofing**: n/a (no auth surface). **Tampering**: config edits can change
models/providers but **cannot** redirect endpoints (clients built with
`api_key` only). **Repudiation**: n/a (single user). **Information
disclosure**: everything under `~/.smah/` is plaintext at default umask —
see data-stores. **DoS**: log rotation (1 MB × 3) caps disk growth.
**Elevation**: none — no setuid, no daemons; migration code loads only from
the shipped package dir (`migration.py:54`), never `~/.smah`.

## B2 — smah ↔ provider APIs (egress)

**Information disclosure (T-002, T-008)**: every ask ships the operator
profile, system stats, query, and full pipe to the provider, in plaintext
TLS but third-party-logged server-side. **Spoofing (T-005)**: TLS via
official SDKs; endpoint controlled by SDK env overrides (`OPENAI_BASE_URL`/
`ANTHROPIC_BASE_URL`) — not by smah config. **DoS/cost (T-009)**: full pipe
to the answer call is untruncated.

## B3 — Model output ↔ parser/console/clipboard

The model is treated as **untrusted content**, never instructions:
- `<exec>` tags become review panels labeled "SUGGESTED COMMAND — NOT
  EXECUTED BY SMAH" — there is no execution path to elevate (T-004,
  mitigated by the test+CI-enforced print-only invariant).
- **Tampering → Elevation via human (T-003)**: injected piped data can steer
  the suggested command; `pyperclip.copy` fires automatically per extracted
  command (`runner.py:146-152`, no prompt) — the payload lands one paste
  from the user's shell. Panels + copy notice are the only friction.
- **Tampering — parser (T-006)**: `etree.XMLPullParser(recover=True)` with
  library defaults (`no_network=True`, `load_dtd=False` implicit;
  `resolve_entities` allows internal entities). No external fetch, no
  explicit hardening args; recover=True tolerates malformed tags by design.
- **Repudiation**: none (no side effects beyond clipboard/logs).

## B4 — Environment ↔ process

`$ENV_VAR` indirection in config API keys; SDK base-URL env overrides
(T-005); direnv sources user-owned `.envrc.secret`/`.envrc.dev`. An
attacker controlling the environment controls endpoints and keys —
documented as the trust line, not mitigated.

## B5 — CI/CD ↔ PyPI

See [supply-chain.md](supply-chain.md). OIDC Trusted Publishing, no
long-lived tokens, tag↔version gate; moving-tag action pins (T-007).
