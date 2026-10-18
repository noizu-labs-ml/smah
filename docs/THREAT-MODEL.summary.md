# Threat Model — summary

Condensed reference extracted from [THREAT-MODEL.md](THREAT-MODEL.md); kept in sync.

**Posture**: local-only, print-only CLI — no daemon/server/inbound surface.
Crown jewels: provider API keys, session history + operator/system profile
(`~/.smah/`), the host (protected by the print-only invariant). Only external
boundary = egress to LLM APIs. Stale `docs/source/architecture.md` claims
RBAC/"SMAHRails" — **do not cite as mitigations; they don't exist.**

**Boundaries**: B1 local host ↔ process/`~/.smah/` · B2 ↔ provider APIs (egress)
· B3 model output ↔ parser/console/clipboard · B4 environment ↔ process ·
B5 CI/CD ↔ PyPI · B6 model output ↔ virtual-tool file-surface executor + stdout
gate (#17; print-only no longer fully separates model output from disk).

**Register (16)**: 5 mitigated · 5 partial · 3 open · 3 accepted.

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
| T-010 | High | Malicious/tampered tool definition (over-broad `allow_roots`, hostile persona) | Partial |
| T-011 | High | Prompt injection via stdin/read drives directives outside `allow_roots` | Mitigated |
| T-012 | Med | Symlink escape on the file surface + canonicalize→open TOCTOU | Partial |
| T-013 | Med | Directive-protocol abuse: sentinel leak to stdout / malformed directives | Mitigated |
| T-014 | Low | Shell injection via hook snippets / shim names | Mitigated |
| T-015 | Info | Audit-log tamper; metadata-only logging trade-offs | Partial |
| T-016 | Low | Unbounded stdin / directive loops / oversized writes (token+disk cost) | Mitigated |

**Key controls (verified)**: print-only test + CI grep gate (Rust gate
amended to execution primitives only — `Command::new`/`process::Command`/
`process::spawn`) · `yaml.safe_load` everywhere · no eval/exec/pickle ·
whitelisted-AST exec-if · keys never printed/never in DB · config cannot
redirect endpoints · PyPI OIDC trusted publishing · virtual-tool B6 controls:
directive scanner + stdout gate, canonicalized allowlist containment with
abort-on-violation, metadata-only audit JSONL, strict tool schema + reserved
names, driver-owned contract, `--no-fs` kill switch.

**Residual**: local attacker reads all plaintext stores (accepted — same-account
attacker is game over); user pastes a suggested command (accepted trade-off);
provider-side retention of sensitive pipes (user's choice); unsigned tool
files + TOCTOU window on the virtual-tool file surface (T-010/T-012 — read
tools before installing, try `--no-fs` first).

**Details**: [threats/attack-surface.md](threats/attack-surface.md) ·
[threats/data-stores.md](threats/data-stores.md) ·
[threats/supply-chain.md](threats/supply-chain.md)
