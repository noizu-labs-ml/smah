# Threat Model — smah

## Overview

SMAH is a **local-only, print-only CLI**: no daemon, no server, no inbound
network surface. Its crown jewels are (1) provider **API keys** (spend
authority on Anthropic/OpenAI), (2) **session history + operator/system
profile** in `~/.smah/smah.db`, and (3) the **host itself** — which the
print-only invariant protects from model output. The only external boundary
is egress to LLM provider APIs; the only untrusted *data* inputs are piped
stdin and model responses.

⚠ `docs/source/architecture.md` (stale draft) claims RBAC and "SMAHRails"
guardrails — **neither exists**. Never cite them as mitigations; this file
is the authoritative control inventory.

**Trust boundaries:**
- **B1** host user / local attacker ↔ smah process and `~/.smah/` files
- **B2** smah ↔ LLM provider APIs (TLS via vendor SDKs; data egress)
- **B3** model output (untrusted) ↔ response parser, console, clipboard
- **B4** environment ↔ process (env vars incl. SDK base-URL overrides, direnv)
- **B5** CI/CD ↔ PyPI (release path)
- **B6** model output (untrusted) ↔ virtual-tool driver file-surface executor + stdout gate (CLI driver only; the print-only invariant no longer fully separates model output from disk)

## Attack Surface

```mermaid
graph LR
    U["User terminal (trusted)"] --> P["smah process"]
    IN["Piped stdin (untrusted data)"] --> P
    ENV["Env vars / .envrc (B4)"] -.-> P
    P -- "B2 TLS: query + profile + pipe" --> LLM["Anthropic / OpenAI APIs"]
    LLM -- "B3: model output (untrusted)" --> P
    P --> OUT["stdout panels + clipboard"]
    P --> DISK[("~/.smah/: config.yaml · smah.db · logs/ — plaintext")]
```

No deployed perimeter exists (no Dockerfile/helm/ingress); distribution is
the PyPI package plus GitHub Actions release via Trusted Publishing.

→ *See [threats/attack-surface.md](threats/attack-surface.md) for the full STRIDE enumeration*

## Vulnerability Register

| ID | Severity | STRIDE | Component | Status |
|----|----------|--------|-----------|--------|
| T-001 | **High** | Info disclosure | Plaintext API key written to DEBUG log (`Settings.log()` save-mode dump, `settings.py:126`, every ask) | **Open** |
| T-002 | Medium | Info disclosure | Query, full pipe input, operator/system profile logged to `~/.smah/logs/` and stored plaintext in `smah.db`; no encryption or permission hardening anywhere | Open |
| T-003 | Medium | Elevation (social) | Suggested command auto-copied to clipboard, no prompt (`runner.py:146-152`) — injected command sits one paste from execution | Partial |
| T-004 | Medium | Tampering / Elevation | Prompt injection via piped input → model-suggested commands. Execution contained by print-only invariant (test + CI gate); residual = user pastes | Mitigated (execution) |
| T-005 | Low | Spoofing / Tampering | No `base_url` config in code (good) — but SDK-native `OPENAI_BASE_URL`/`ANTHROPIC_BASE_URL` env overrides redirect key + traffic if the environment is attacker-controlled | Accepted (env = B4 trust) |
| T-006 | Low | Tampering | lxml parsing of model output relies on library defaults (no explicit `resolve_entities=False`, `no_network`, `huge_tree` args); behavior is version-dependent | Partial |
| T-007 | Low | Tampering (supply chain) | GitHub Actions pinned by moving tags (`checkout@v4`, `setup-python@v5`, `pypi-publish@release/v1`), not commit SHA | Open |
| T-008 | Info | Repudiation | All queries/pipes/system profile sent to third-party providers that log server-side — inherent to the product | Accepted |
| T-009 | Low | Denial of service / cost | Full pipe is sent to the answer call untruncated (planner truncates; answer does not) — large pipe = large token spend | Accepted |
| T-010 | **High** | Elevation / Tampering | Malicious or tampered tool definition (`~/.config/smah/virtual-tools/*.md`, often copy-pasted from the internet) declares over-broad `allow_roots` or a hostile persona — the tool file is the new trust anchor | Partial (strict schema, root disclosure on stderr, `--no-fs`, driver-owned contract, metadata audit; no signing in v1 — docs: install only tools you trust) |
| T-011 | **High** | Elevation | Prompt injection via piped stdin or `read`-returned content drives directive emission targeting paths outside `allow_roots` (e.g. `~/.ssh/authorized_keys`) | Mitigated (lexical `..` rejection, canonicalize + component-prefix containment, abort-on-violation exit 3, negative-test battery) |
| T-012 | Medium | Elevation | Symlink escape: in-root symlink → outside target; final-component symlink on write; TOCTOU window between canonicalize and open | Partial (realpath containment; final-component symlink rejected on write/append; TOCTOU accepted+documented; `openat2 RESOLVE_BENEATH` noted as Linux-only future hardening) |
| T-013 | Medium | Tampering | Directive-protocol abuse: sentinel lines leak to stdout and inject into downstream pipe consumers, or malformed directives confuse the driver | Mitigated (scanner intercepts all sentinel lines, malformed ⇒ abort; reserved-sentinel contract; scanner tests) |
| T-014 | Low | Spoofing / Tampering | Hook snippet injection: crafted tool/file names injecting shell into `smah hooks zsh` output or shim files | Mitigated (name regex + reserved set; unquoted-but-validated emission; golden snapshots) |
| T-015 | Info | Repudiation / Info disclosure | Audit log tamper; secrets leaking via logged paths/args; args+stdin egress to provider (inherent, cf. T-008) | Partial (metadata-only JSONL, contents never logged, 0700/0600, user-owned dir; no tamper-proofing v1) |
| T-016 | Low | DoS / cost | Unbounded stdin, endless directive loops, oversized writes → token/disk spend (T-009's analog for virtual tools) | Mitigated (loud truncation cap, `TURNS_MAX=8`, per-directive + aggregate write caps) |

**Coverage:** 5 mitigated · 5 partial · 3 open · 3 accepted (16 items —
T-001…T-016). Open items have no tracker tickets yet — file issues before
fixing (T-001 first).

→ *Control details: [threats/data-stores.md](threats/data-stores.md) · [threats/supply-chain.md](threats/supply-chain.md)*

## Key Controls (verified in code)

- **Print-only invariant**: `tests/test_no_shell_execution.py` scans every
  `smah/**/*.py` for `subprocess.run`/`shell=True`/`os.system`/`Popen`
  (allowlist: 4 OS-info readers, fixed-argv `check_output`, never
  model-touched); CI re-enforces the same grep gate. No `eval`/`exec`/
  `pickle`/`os.exec*` anywhere; `exec-if` conditions use a whitelisted AST
  walker.
- **YAML**: `yaml.safe_load` at every load site; no unsafe `yaml.load`.
- **Secrets handling**: keys never printed, never stored in the DB; `config
  show` redacts; `$ENV_VAR` indirection masks as `<env:NAME>`; provider
  settings dumps pop `settings` outside save mode.
- **Endpoint pinning**: clients constructed with `api_key` only — config
  tampering cannot redirect traffic.
- **PyPI release**: Trusted Publishing (OIDC), no long-lived tokens;
  tag↔version match gate.
- **Virtual-tool file surface (B6, #17)**: directive scanner gates stdout
  (all sentinel lines intercepted, malformed ⇒ abort exit 3); canonicalized
  allowlist containment with component-prefix matching and abort-on-
  violation; metadata-only audit JSONL (0700/0600, contents never logged);
  strict tool-file schema (unknown-key hard fail) + reserved command-name
  set; driver-owned operating contract (not author-editable); `--no-fs` kill
  switch; amended Rust `no-shell-gate` still bans every execution primitive
  (`Command::new` / `process::Command` / `process::spawn` — termination-only
  `exit`/`ExitCode` allowed at the binary edge).

## Residual Risk

- **Local attacker / compromised host**: plaintext `config.yaml`, `smah.db`,
  and logs mean full key + history compromise. Accepted: a local attacker
  with the user's account can already read process memory; hardening (keyring,
  0600 perms, T-001 fix) raises cost but not the boundary.
- **User pastes a suggested command**: print-only cannot protect against the
  human in the loop (T-003/T-004 residual). Panels label commands
  "NOT EXECUTED BY SMAH"; clipboard auto-copy lowers user friction — the
  accepted trade-off for speed.
- **Provider-side data**: everything sent to Anthropic/OpenAI is outside
  SMAH's control (T-008); sensitive pipes should not be piped into smah
  until provider data-retention choices are explicit.
- **Virtual tools (B6, T-010..T-016)**: tool files are unsigned — a hostile
  `allow_roots` is as broad as the user installed it; the canonicalize→open
  TOCTOU window is accepted for v1 (future: Linux `openat2 RESOLVE_BENEATH`).
  Install only tools you have read; run new tools once with `--no-fs` and
  watch the audit log.

## Grounding

Components and data flow: [PROJ-ARCH.md](PROJ-ARCH.md) · directories:
[PROJ-LAYOUT.md](PROJ-LAYOUT.md) · normative UX contract:
[ux/interface-spec.md](ux/interface-spec.md).
