# Threat Model — summary

Condensed reference extracted from [THREAT-MODEL.md](THREAT-MODEL.md); kept in sync.

**Posture**: local-only, print-only CLI — no daemon/server/inbound surface
beyond explicitly user-started loopback-only mock servers (B7).
Crown jewels: provider API keys, session history + operator/system profile
(`~/.smah/`), the host (protected by the print-only invariant). Only external
boundary = egress to LLM APIs. Stale `docs/source/architecture.md` claims
RBAC/"SMAHRails" — **do not cite as mitigations; they don't exist.**

**Boundaries**: B1 local host ↔ process/`~/.smah/` · B2 ↔ provider APIs (egress)
· B3 model output ↔ parser/console/clipboard · B4 environment ↔ process ·
B5 CI/CD ↔ PyPI · B6 model output ↔ virtual-tool file-surface executor + stdout
gate (#17; print-only no longer fully separates model output from disk) ·
B7 model-fabricated content ↔ loopback mock servers + browser/clients (#20/#19/#22/#21/#23/#24/#25:
loopback-only (or unix-socket) binds, escaped rendering, origin-checked regen/push,
DTD-denying XML + descriptor-validated gRPC dispatch, stdio-default MCP transport,
code-owned VFS containment; Rust
drivers stay print-only — they write files, never execute).

**Register (30)**: 18 mitigated · 6 partial · 3 open · 3 accepted.

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
| T-017 | Med | TUI chrome injection via fabricated content / `@@smah ui` payloads (#18) | Mitigated |
| T-018 | Med | API-driver fabricated-response abuse: unbounded/garbage responses (#20) | Mitigated |
| T-019 | High | Web-driver XSS / cross-origin `/regen`+`/push` abuse (#19) | Mitigated |
| T-020 | Med | Loopback mock server accidentally exposed beyond the host (#20/#19) | Mitigated |
| T-021 | Med | Driver cache poisoning (`openapi.json`/`content.json` served unvalidated) | Mitigated |
| T-022 | Med | SOAP XML abuse: DTD expansion / XXE / oversized envelopes (#22) | Mitigated |
| T-023 | Med | grpc proto parse DoS + streaming shapes outside unary-only dispatch (#21) | Mitigated |
| T-024 | Med | soap/grpc contract poisoning (cached WSDL/proto served unvalidated) | Mitigated |
| T-025 | Med | soap/grpc loopback emulator mistaken for a real service / exposed beyond host | Mitigated |
| T-026 | Med | VFS escape: `..`/absolute/symlink smuggled out of the fabricated tree (#25) | Mitigated |
| T-027 | Med | Fabricated MCP tool-catalog injection to downstream clients (#23) | Mitigated |
| T-028 | Low | MCP transport exposure: listener reachable when believed private (#23) | Mitigated |
| T-029 | Med | Real-vs-virtual confusion: fabricated DB mistaken for a real one (#24) | Partial |
| T-030 | Low | Wire-level abuse of the db/vfs/mcp servers (oversize, statement bombs, write attempts) | Mitigated |

**Key controls (verified)**: print-only test + CI grep gate (Rust gate
amended to execution primitives only — `Command::new`/`process::Command`/
`process::spawn`) · `yaml.safe_load` everywhere · no eval/exec/pickle ·
whitelisted-AST exec-if · keys never printed/never in DB · config cannot
redirect endpoints · PyPI OIDC trusted publishing · virtual-tool B6 controls:
directive scanner + stdout gate, canonicalized allowlist containment with
abort-on-violation, metadata-only audit JSONL, strict tool schema + reserved
names, driver-owned contract, `--no-fs` kill switch · virtual-tool B7
controls: bind-address validation (loopback or refuse to start) with loud
stderr disclosure, `html/template` + `textContent` rendering under CSP,
loopback + same-origin gates on `/push` and `/regen`, sha-pinned
validate-on-load caches, Go-service CI `os/exec` ban with positive-control
self-test, soap DTD/entity-denying XML parsing with request caps, grpc
bounded protox parse + unary-only descriptor-validated dispatch, mcp
two-strikes bounded catalog fabrication + digest-keyed frozen
`mcp-tools.json` + `--show-catalog`, vfs canonicalized containment
(`..`/absolute/symlink refusal read+write) with code-owned fs tools,
db PG-wire subset (writes `42501`, extended/multi-statement `0A000`,
size caps `54000`, read-only store, one sequential connection).

**Residual**: local attacker reads all plaintext stores (accepted — same-account
attacker is game over); user pastes a suggested command (accepted trade-off);
provider-side retention of sensitive pipes (user's choice); unsigned tool
files + TOCTOU window on the virtual-tool file surface (T-010/T-012 — read
tools before installing, try `--no-fs` first); loopback mocks are deliberately
convincing fabricated data (T-017..T-030 — never point a decision-making
client at one).

**Details**: [threats/attack-surface.md](threats/attack-surface.md) ·
[threats/data-stores.md](threats/data-stores.md) ·
[threats/supply-chain.md](threats/supply-chain.md)
