# Threat Model — summary

Condensed reference extracted from [THREAT-MODEL.md](THREAT-MODEL.md); kept in sync.

**Posture**: local-only, print-only CLI — no daemon/server/inbound surface.
Crown jewels: provider API keys, session history + operator/system profile
(`~/.smah/`), the host (protected by the print-only invariant). Only external
boundary = egress to LLM APIs. Stale `docs/source/architecture.md` claims
RBAC/"SMAHRails" — **do not cite as mitigations; they don't exist.**

**Boundaries**: B1 local host ↔ process/`~/.smah/` · B2 ↔ provider APIs (egress)
· B3 model output ↔ parser/console/clipboard · B4 environment ↔ process ·
B5 CI/CD ↔ PyPI.

**Register (9)**: 1 mitigated · 2 partial · 3 open · 3 accepted.

| ID | Sev | Item | Status |
|----|-----|------|--------|
| T-001 | High | API key plaintext in DEBUG logs (`Settings.log()` save-mode) | Open |
| T-002 | Med | Query/pipe/profile plaintext in logs + smah.db | Open |
| T-003 | Med | Suggested command auto-copied to clipboard, no prompt | Partial |
| T-004 | Med | Prompt injection via pipe → suggested commands (execution test+CI contained) | Mitigated |
| T-005 | Low | SDK `*_BASE_URL` env override can redirect key+traffic | Accepted |
| T-006 | Low | lxml defaults relied on implicitly for untrusted XML | Partial |
| T-007 | Low | GH Actions tag-pinned, not SHA | Open |
| T-008 | Info | Provider-side logging of all sent data | Accepted |
| T-009 | Low | Full pipe untruncated to answer call (token cost) | Accepted |

**Key controls (verified)**: print-only test + CI grep gate (4 fixed-argv
OS-info exemptions) · `yaml.safe_load` everywhere · no eval/exec/pickle ·
whitelisted-AST exec-if · keys never printed/never in DB · config cannot
redirect endpoints · PyPI OIDC trusted publishing.

**Residual**: local attacker reads all plaintext stores (accepted — same-account
attacker is game over); user pastes a suggested command (accepted trade-off);
provider-side retention of sensitive pipes (user's choice).

**Details**: [threats/attack-surface.md](threats/attack-surface.md) ·
[threats/data-stores.md](threats/data-stores.md) ·
[threats/supply-chain.md](threats/supply-chain.md)
