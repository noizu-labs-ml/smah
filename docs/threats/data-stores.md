# Data Stores — ~/.smah/ Inventory

Details for [../THREAT-MODEL.md](../THREAT-MODEL.md). All paths plaintext,
default umask; no chmod/umask/keyring/encryption code exists anywhere
(verified by grep, develop).

| Store | Contents | Sensitivity |
|-------|----------|-------------|
| `~/.smah/config.yaml` | `User`/`System`/`Inference` config; provider API keys (literal or `$ENV_VAR`) | Keys = spend authority |
| `~/.smah/smah.db` | SQLite: sessions only — queries, plans, full pipe input, message threads; **no secrets** | History/profile intel |
| `~/.smah/logs/smah.*.log` | Rotating DEBUG (1 MB × 3): full request/response payloads (query, pipe, operator+system profile) **and, via T-001, the API key** | Worst offender while T-001 is open |

## T-001 — Plaintext API key in logs (High, Open)

`Settings.log()` dumps with `{"stats": True, "save": True}`
(`smah/settings/settings.py:126-127`); save mode **retains** the provider
`settings` dict — including the resolved-or-literal `api_key` string —
which lands in the DEBUG rotating file log. Called on every ask path
(`smah/smah.py:327`). This contradicts the codebase's own discipline: keys
are otherwise never printed, `config show` redacts, provider payloads pop
`settings` outside save mode (`provider/provider.py:75,82-85`).

**Fix shape**: strip/redact `api_key` (and any secret-looking keys) in the
save-mode dump used by `log()`; consider `0600` on `~/.smah` files while in
there. Log-scrub for already-written files is out of scope (rotation
evicts within 3 MB).

## T-002 — Sensitive content at rest (Medium, Open)

Query text, full piped input, operator experience profile, and OS/CPU/disk/
memory stats are logged at INFO/DEBUG (`providers/base.py:94-109`,
`anthropic_provider.py:107,137`) and persisted in `smah.db` by design.
Accepted local posture per the residual-risk statement; the *log* copies
are the questionable part — they duplicate DB content into a diagnostics
path. Possible mitigation if desired: log prompt-thread digests/titles
instead of full bodies, or gate verbose payload logging behind a debug
flag.

## Non-stores (checked, clean)

- `smah-db` migrations execute only `up()/down()` from the **shipped**
  package directory — no plugin loading from `~/.smah`.
- Clipboard and stdout are transient (clipboard covered under T-003).
- No temp files beyond vendor SDK internals.
