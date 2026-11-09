# mallm → SMAH Fold-In Plan (Phased Rust Port)

Session: d4114180-f0b7-4eee-84db-d92cb1f23def · Owner decisions: 2026-11-09 (§8 Decisions D1–D4) + 2026-11-10 (D5–D7, promoted from the Q5–Q7 assumptions) · Status: DECIDED (P0 owner-gates resolved)

Smah citations refer to the `epic.rust-tui` tree (`git show epic.rust-tui:<path>`); mallm citations to
`/Users/keithbrings/Work/Space/Noizu/Projects/mallm` (read-only, current checkout).

## 1. Summary, goals, non-goals

Fold mallm ("man pages for LLM agents", `Projects/mallm/README.md`) into SMAH as a new `smah-docs`
crate plus a `smah docs <tool>` subcommand, adopting `Projects/mallm/schemas/mallm.schema.json` as the
spec. Inference reuses `crates/smah-providers`; sessions/history reuse SMAH's SQLite store
(`~/.smah/smah.db`, `Projects/smah/smah/database/database.py:9`). The mallm.yaml corpus feeds the
virtual_tools registry (`crates/smah-core/src/virtual_tools/loader.rs`). Config paths converge on
`~/.smah` with a one-way migration from mallm's XDG dirs. The Node mallm stays alive until feature
parity; retirement is a separate owner call.

**Goals**

- G1: Rust parity for mallm's file-based surface: resolve tiers 1–2, validate, format MD/JSON, list,
      init, search, corpus (http/dir), sources, doctor checks.
- G2: One config home (`~/.smah`), one history store (SQLite), zero duplicated state after migration.
- G3: Corpus → virtual-tools export so documented tools become emulatable vtools.
- G4: Absolute preservation of SMAH's print-only invariant unless the owner explicitly rules otherwise (§4).

**Non-goals**

- N1: Executing generated shell — forbidden (`docs/ux/README.md` binding decision 1; CI `no-shell-gate`,
      `.github/workflows/ci-rs.yml`).
- N2: Porting mallm's how-to pipeline (`Projects/mallm/src/howto/machine.ts`, 724 lines) — it installs
      and runs scripts via `execFile` (`src/howto/cli.ts:47,78,247`) and pushes branches
      (`src/howto/writeback.ts:40`); irreconcilable with print-only. Stays Node-only until retirement,
      then re-homes as a SEPARATE non-SMAH product — not dropped (§8 Decisions D3).
- N3: Retiring the Node mallm (separate owner call, P6).
- N4 *(revoked 2026-11-09, §8 Decisions D4)*: ~~Jev quiz/learn (`src/jev/`) and memory tiers
      (`src/memory/`) — drop/defer~~ → **port in P2**, provider-backed via smah-providers or the
      hosted Jev endpoint, zero exec.

## 2. Concept map — mallm → smah

| mallm module | smah target | Verdict | Rationale |
|---|---|---|---|
| `schemas/mallm.schema.json` | vendored copy in smah repo (P0) | **port** | Canonical spec (`mallm/AGENT-README.md` gotchas); freeze + checksum to stop drift while Node lives |
| `src/schema.ts` (TS types) | `smah-docs` serde types | **port** | Direct struct mapping; serde_yaml_ng already a workspace dep (`Cargo.toml`) |
| `src/resolver.ts` (4-tier chain) | `smah-docs::resolve` | **port (tiers 1–2)**; tiers 3–4 gated | Tier walk is read-only fs (git-root walk = ancestor dir scan, no git exec); tiers 3–4 use `execSync` (`resolver.ts:58,73`) → §4 |
| `src/help-parser.ts` | `smah-docs::help_parse` | **port** | Pure text→struct (101 lines), no exec; input arrives piped/from file under §4-A |
| `src/formatter.ts` | `smah-docs::format` | **port** | MD default, JSON + `_meta` provenance (`mallm/README.md`); golden byte-parity tests vs Node `--no-color` |
| `src/index.ts` (commander, 892 lines) | `crates/smah-cli/src/args.rs` `Docs` subcommand | **port (subset)** | clap 4 tree already hosts `Tools`/`Hooks` (`args.rs:106-136`); N2/N4 verbs excluded |
| `src/init.ts` | `smah docs init` | **port (file/stdin seed)** | Stub creation is a file write; `--help` seeding via `execSync` (`init.ts:17`) replaced by piped input (§4-A) |
| `src/config/loader.ts` + `types.ts` | `~/.smah/config.yaml` `docs:` block | **port + converge** | smah-core settings are Value-centric (`crates/smah-core/src/settings/mod.rs:1`); chain `.mallm/config.yaml` → user → defaults preserved (§5) |
| `src/corpus/adapters.ts` | `smah-docs::corpus` | **port http + dir; drop git (v1)** | http via reqwest/rustls (workspace dep); git adapter shells out via `execFile` (`adapters.ts:44`) → N1 (§8 R7) |
| `src/corpus/cache.ts`, `merge.ts` | `smah-docs::corpus::cache` | **port** | Follows existing validate-then-freeze + sha256-marker convention (T-021, `docs/THREAT-MODEL.md:69`) |
| `src/search/bm25.ts` + `corpus.ts` | `smah-docs::search` | **port** | Hand-rolled BM25 (111 lines), pure |
| `src/session/{kv,token,index}.ts` | SMAH SQLite store | **reuse store / drop glyph tokens** | Glyph tokens + approvals exist to gate how-to execution (`mallm/docs/PROJ-SCHEMA.summary.md`); with N2 they have no consumer |
| `src/changelog/history.ts` + `redactor.ts` | SQLite history + ported redactor | **port** | JSONL → SQLite rows (§5); regex-parity risk R5 |
| `src/env/probe.ts` | — | **defer** | `--version` probes are `execFile` (`probe.ts:53`); SMAH uses sysinfo "without subprocess" (`Cargo.toml` comment). PATH presence scan (dir reads, no exec) is the portable subset |
| `src/llm/client.ts` | `crates/smah-providers` | **reuse, drop mallm client** | Provider trait + registry + hand-rolled clients already exist (`crates/smah-providers/src/lib.rs`); mallm's OpenAI-compat client is redundant |
| `src/jev/*` | `smah-docs::jev` + provider wiring | **port (P2)** | D4 (§8 Decisions): quiz/learn ported provider-backed — inference via `smah-providers` or the hosted Jev endpoint, zero exec primitives |
| `src/memory/{knowledge,tiers}.ts` | `smah-docs::memory` | **port (P2)** | D4 (§8 Decisions): knowledge.yaml + memory tiers ride along with the jev port, provider-backed, no exec |
| `src/howto/*` | — | **spin out (separate product)** | N2/D3 — executes and pushes; stays Node-only until retirement, then re-homes as a SEPARATE non-SMAH product (not dropped) |
| corpus (merged docs) | `~/.config/smah/virtual-tools/*.md` | **port (export)** | New `smah docs export-vtool` renders mallm.yaml → vtool frontmatter valid per `loader.rs` rules (name==stem, known driver, no reserved names) |

## 3. `smah-docs` crate design

Workspace member `crates/smah-docs` (workspace globs `crates/*`, root `Cargo.toml`). Deps: serde,
serde_yaml_ng, serde_json (all workspace), smah-core (errors/exit codes), smah-providers (P2 jev port
per §8 D4; P5 inference niceties otherwise),
reqwest+rustls (corpus http), rusqlite only via a store handle passed in from smah-cli (crate itself
stays storage-agnostic).

**Public API sketch (types/prose only — no code lands with this plan):**

- `DocSpec` — serde mirror of `mallm.schema.json`: `mallm` version enum `"1.0"|"1.1"`, required
  `name`/`summary`, plus `description`, `usage.examples`, typed `arguments[]` (flag/option/positional/
  variadic per `mallm/docs/PROJ-SCHEMA.summary.md`), `subcommands[]`, `environment`, `files`,
  `context { when_to_use, when_not_to_use, common_patterns[], gotchas[] }`, `skills`, `related`,
  `output`, and v1.1 platform keys `sources[]`, `inference`, `redaction.patterns[]`, `sessionTtl`.
- `ResolvedDoc { spec: DocSpec, source: Source, path: Option<PathBuf> }` with
  `Source = ProjectLocal | UserConfig | Native | HelpGenerated` — parity with `ResolvedMallm`
  (`mallm/src/resolver.ts`).
- `resolve(app, opts{ version, cwd }) -> Result<ResolvedDoc, SmahError>` — tier chain
  ProjectLocal(`.mallm/<app>.yaml` at nearest ancestor containing `.mallm/` or `.git/`, walk per
  `resolver.ts:11-20`) → UserConfig(`~/.smah/docs/<app>/{<ver>,latest}/mallm.yaml`, migrated from
  `~/.config/mallm/<app>/…`, `resolver.ts:40-55`) → Native → HelpGenerated. Tiers 3–4 dispatch through
  a `HelpCapture` trait (§4); the shipped default `NoExecCapture` returns `None`, so the chain
  degrades to a clean exit-2 "not found" row instead of executing anything.
- `validate(spec) -> Result<(), Vec<Violation>>` — structural validation against the vendored schema
  (embedded via `include_str!`). Recommendation: pull the maintained `jsonschema` crate as a dev+prod
  dep rather than hand-rolling draft 2020-12 semantics; keep the JSON file as the single source of
  truth (§8 D6).
- `format_md(doc) -> String` / `format_json(doc, meta) -> String` — `_meta` carries source + timestamp
  like `mallm/src/formatter.ts`; color is a smah-cli concern (pulldown-cmark/rich styling lives there),
  goldens captured `--no-color`.
- `help_parse(app, text) -> DocSpec` — port of `help-parser.ts` (summary from first non-usage lines,
  options/subcommands section scraping, auto-generated gotcha disclaimers).
- `corpus::{SourceCfg, Cache, fetch, merge}` — http (reqwest) + dir adapters, priority merge
  (lower wins, `mallm.schema.json` `sources[].priority`), cache under `~/.cache/smah/docs-corpus/<key>/`
  with `meta.json` (etag/sha256/fetchedAt parity with `mallm/src/corpus/cache.ts`) and size caps.
- `search::{index, query}` — BM25 port (`mallm/src/search/bm25.ts`).
- `export_vtool(doc) -> String` — renders a virtual-tools markdown file: frontmatter
  `name` (== stem, `is_valid_name`, not reserved), `description` (from `summary`), optional `model`,
  `driver: cli`; body = persona assembled from `context`/`usage` sections, satisfying every loader
  rejection rule in `crates/smah-core/src/virtual_tools/loader.rs:9-26`.
- Errors: reuse `smah_core::SmahError` + the six-class exit table (`crates/smah-core/src/exit_codes.rs`);
  new rows only via the existing `errors.rs` spec + snapshot pattern (`crates/smah-core/tests/errors_spec.rs`).

**CLI wiring:** `Command::Docs { cmd: DocsCommand }` in `crates/smah-cli/src/args.rs` with verbs
`show` (default), `list`, `validate`, `schema`, `init`, `search`, `sources`, `export-vtool`,
`migrate` (§5). History rides SMAH's existing history surface once M4 lands
(`docs/ux/implementation-plan.md:66`, US-26/27/28).

## 4. The help-capture / exec question — **OWNER DECISION REQUIRED**

**The conflict.** mallm's discovery tiers execute the target tool: `resolver.ts:58`
(`execSync(\`${app} --mallm\`)` — template string, flagged as residual risk in
`mallm/docs/PROJ-ARCH.summary.md` "Security posture"), `resolver.ts:73` (`--help` via
`shell: "/bin/sh"`), `init.ts:17` (same shape), `env/probe.ts:53` (`execFile … --version`). SMAH's
print-only invariant is absolute today: CI `no-shell-gate` greps `Command::new|process::Command|
process::spawn` over all of `crates/*/src` with a positive-control self-test and **zero exceptions**
(`.github/workflows/ci-rs.yml`); US-10 requires "zero `process::Command` in tree, empty-allowlist
test" (`docs/ux/implementation-plan.md:61`); the workspace comment on sysinfo reads "System info
without subprocess (print-only invariant: zero process::Command)" (root `Cargo.toml`).

**Option A — corpus-only + user-piped help (no exec, ever).** Tiers 3–4 get their text from the user:
`smah docs init <tool>` reads help text from stdin (`<tool> --help | smah docs init <tool>`) or
`--help-file <path>`; native `--mallm` likewise via pipe. This mirrors SMAH's existing data model
("piped stdin is the data", `AGENT-README.md` dev-mode) and mallm's own `--json`/stdin discipline.
Gate untouched. Cost: no silent auto-discovery — a bare `smah docs <unknown-tool>` errors with a
row telling the user the exact pipe to run, instead of mallm's automatic fallback.

**Option B — allowlisted fixed-argv reader with a CI-gate exception.** One audited module (e.g.
`crates/smah-docs/src/capture.rs`) may spawn exactly `<tool> --mallm` / `<tool> --help` with fixed
argv, no shell, tool name validated `^[A-Za-z0-9._-]+$`, PATH-resolved, hard timeout + output cap,
every capture written to an audit JSONL (pattern of `crates/smah-core/src/virtual_tools/audit.rs`).
The gate grows a path-scoped allowlist and keeps its positive control everywhere else; new
threat-model rows required (binary-name-controlled execution = user-supplied code runs; `--help` is
not universally side-effect-free). Cost: the headline invariant ("never executes", `AGENT-README.md`
Purpose, `docs/ux/README.md` decision 1) becomes conditional — a safety and messaging regression
that outlives this feature.

**Option C — A now, B later behind the seam.** Ship A in P1; the `HelpCapture` trait makes B a
single-module, single-gate-edit addition later if the owner ever wants auto-discovery.

**DECIDED 2026-11-09 (§8 D1): Option C.** Ship A (corpus-only + user-piped help) in P1; the
`HelpCapture` seam stays so B remains a single-module, single-gate-edit addition — but implementing
B requires a further explicit owner decision. Do not implement B without it.

## 5. Data / config migration

Legacy layout (`mallm/AGENT-README.md`, `mallm/docs/PROJ-ARCH.summary.md` "Local state"; all
XDG-overridable): docs `~/.config/mallm/<app>/[<ver>|latest/]mallm.yaml`; platform config
`.mallm/config.yaml` → `~/.config/mallm/config.yaml`; sessions
`~/.local/share/mallm/sessions/<token>.json`; history `~/.local/share/mallm/history.jsonl`
(pre-redacted, append-only); corpus cache `~/.cache/mallm/corpus/<key>/`; knowledge
`~/.config/mallm/knowledge.yaml`.

New canonical layout, converged on `~/.smah` (matching `smah/settings/settings.py:16` and
`smah/database/database.py:9`):

| Data | New home | Mechanism |
|---|---|---|
| Per-tool docs | `~/.smah/docs/<app>/[<ver>\|latest/]mallm.yaml` | one-way copy |
| Platform config | `docs:` block in `~/.smah/config.yaml` (sources, inference→providers mapping, redaction patterns, sessionTtl) | merge on migrate; project tier `.mallm/config.yaml` still read in place (kept — §8 Decisions D2) |
| History | SQLite `~/.smah/smah.db`, new tables via the checksummed migration runner (byte-exact MD5 scheme, `AGENT-README.md` gotcha; rusqlite bundled, root `Cargo.toml`) | JSONL import, redactor re-applied at import |
| Sessions | dropped (glyph-token approvals serve N2 only); session-scoped history rows survive via import | — |
| Corpus cache | `~/.cache/smah/docs-corpus/<key>/` | rebuild-on-demand, not imported (cache is derivable; `meta.json` format ported) |
| Knowledge | migrated in P2 with the jev/memory port (§8 Decisions D4) | one-way copy to the smah docs tier; Node mallm keeps reading its XDG original through the parity window |

`smah docs migrate`: idempotent, copy-never-move (Node mallm keeps working through the parity
window), prints per-category counts + skipped/unreadable rows, re-applies redaction patterns on
history import, exits 0 with a "nothing to migrate" row when legacy dirs are absent. `smah doctor`
gains docs checks (config block valid, db migrated, corpus sources reachable — parity with
`mallm doctor`, `mallm/README.md`). Coexistence drift (R6): docs are write-rarely; re-run `migrate`
before retirement to pick up anything the Node CLI wrote after the first pass.

## 6. Phases + exit criteria (sequenced against `epic.rust-tui`)

Sequencing facts (updated 2026-11-09): the Rust epic landed on `develop` via the regular-merge PR #36
(squash for feature legs is unaffected); fold-in work happens on `feature/mallm-*` branches → **squash
PRs into the epic branch** until it is fully absorbed, else straight to `develop` (repo merge matrix).
P1 may start immediately; P3 hard-depends on epic milestone M4 (SQLite/smah-db parity,
`docs/ux/implementation-plan.md:50,66`).

| Phase | Scope | Exit criteria |
|---|---|---|
| **P0 — spec freeze** | Vendor `mallm.schema.json`; delta record (`howtos[]` not in schema — excluded with N2); owner answers recorded | Schema committed + checksum pinned; D1–D4 decided 2026-11-09 (§8), D5–D7 decided 2026-11-10 (§8); feature-parity matrix drafted |
| **P1 — crate core** | `smah-docs` scaffold, `DocSpec`, resolver tiers 1–2, validator, MD/JSON formatter, `smah docs show/list/validate/schema` | `cargo test` + `no-shell-gate` green; formatter goldens byte-parity with Node `--no-color` on `mallm/examples/*.mallm.yaml`; exit-code rows snapshot-tested |
| **P2 — parity core** | help-parser port, `init` from stdin/file, BM25 search, corpus http+dir + cache, `sources` verbs, **jev quiz/learn + memory/knowledge.yaml port (D4 — provider-backed, no exec)** | Every file-based mallm verb reproducible via `smah docs`; corpus cache markers/caps match T-021 pattern; piped-init flow documented; jev features run through smah-providers/hosted Jev with no-shell-gate green |
| **P3 — state convergence** | config `docs:` block, SQLite history + JSONL import + redactor, `smah docs migrate`, doctor checks | Migrate imports a fixture XDG tree idempotently; history queryable through smah's history surface; doctor rows green; M4 landed first |
| **P4 — vtool feed** | `export-vtool` renderer + invalid-file reporting | Exported files load through `loader.rs` with zero invalid rows and emulate via the cli driver; round-trip test corpus→vtool→`smah tools list` |
| **P5 — inference parity** | Remaining llm-backed niceties via `smah-providers` (jev moved to P2 per D4; P5 keeps only other owner-kept niceties) | Features run through the Provider trait; failures map to exit 4/5 per `crates/smah-providers/src/translate.rs` |
| **P6 — retirement (separate owner call)** | Parity matrix sign-off, README/doc hand-off, how-to pipeline **spin-out hand-off to a separate non-SMAH product** (D3), Node mallm archived | Matrix all-green or explicitly waived **by the owner**; howto re-home lands in its own product repo, not SMAH; mallm repo untouched until then |

## 7. Ticket breakdown

Features enter `epic.rust-tui` via squash PRs (one ticket ≈ one `feature/mallm-<x>` PR unless noted).

| ID | Title | Phase | Size | Depends on | Acceptance criteria |
|---|---|---|---|---|---|
| SM-01 | Vendor + freeze mallm.schema.json | P0 | S | — | Schema file committed with source-repo commit SHA recorded; checksum pinned in doc |
| SM-02 | HelpCapture seam + decision record | P0/P1 | S | D1 | Trait + `NoExecCapture` default; §4 answer appended to this plan; no exec primitives anywhere |
| SM-03 | smah-docs scaffold + DocSpec serde types | P1 | M | SM-01 | Crate in workspace; round-trip parse/serialize of both `mallm/examples/*.yaml` losslessly (key order preserved) |
| SM-04 | Resolver tiers 1–2 + git-root walk | P1 | M | SM-03 | Ancestor walk finds `.mallm/`/`.git` without exec; version/latest fallback order matches `resolver.ts:40-55`; not-found → exit-2 row |
| SM-05 | Schema validator | P1 | M | SM-01, SM-03 | `validate` rejects each schema-violation class with a located reason; passes both examples |
| SM-06 | Formatter MD/JSON + `_meta` | P1 | M | SM-03 | Golden tests byte-identical to Node `--no-color` output for examples; JSON carries `_meta{source,ts}` |
| SM-07 | smah-cli `docs` subcommand wiring | P1 | M | SM-04..06 | `smah docs show/list/validate/schema` live in clap tree; back-compat table tests per M5 convention |
| SM-08 | help-parser port | P2 | M | SM-03 | Same summary/args/subcommand extraction on a fixture corpus of real `--help` texts captured from Node mallm |
| SM-09 | `docs init` from stdin/file | P2 | S | SM-08 | `<tool> --help \| smah docs init <tool>` writes a valid stub to project or `--global` tier; refuses to overwrite |
| SM-10 | BM25 search port | P2 | M | SM-11 | Rank parity with `mallm/src/search/bm25.ts` on a shared fixture corpus; `--json` shape documented |
| SM-11 | Corpus http+dir adapters + cache + merge | P2 | L | SM-03 | Priority merge matches `merge.ts`; cache `meta.json` fields parity; size caps + atomic writes; **no git adapter** |
| SM-12 | `sources list/add/update` | P2 | S | SM-11 | Config writes land in `docs:` block; creds stay name-only (`credsRef`/`tokenEnv`, never values) |
| SM-13 | Config merge into `~/.smah/config.yaml` | P3 | M | SM-07 | `docs:` block loads through smah-core settings; malformed block → exit 3 row |
| SM-14 | SQLite history + JSONL import + redactor | P3 | L | epic M4, SM-13 | Migration-runner checksums byte-exact; import idempotent; redaction patterns re-applied; regex-parity audit per R5 |
| SM-15 | `smah docs migrate` + doctor checks | P3 | M | SM-13, SM-14 | Fixture XDG tree imports with counts; copy-never-move asserted; doctor rows for config/db/sources |
| SM-16 | `export-vtool` renderer | P4 | M | SM-11 | Exports load via `loader.rs` with zero invalid rows; reserved-name/stem rules enforced pre-write |
| SM-17 | Provider wiring for kept inference features | P5 | M | SM-07, Q4 | Uses `smah-providers` registry only; exit 4/5 mapping tested with wiremock per `providers_wiremock.rs` pattern |
| SM-18 | Parity matrix + retirement runbook | P6 | S | all | Matrix covers every `mallm` verb (`mallm/docs/PROJ-SCHEMA.summary.md` CLI table) with keep/waive/drop + owner initials |
| SM-19 | jev quiz/learn port (provider-backed, no exec) | P2 | M | SM-07, D4 | Quiz/learn flows reproducible against `src/jev/` parity fixtures; all inference via `smah-providers` or the hosted Jev endpoint; `no-shell-gate` green (zero exec primitives); provider failures map to exit 4/5 |
| SM-20 | memory tiers + knowledge.yaml port | P2 | M | SM-19 | `knowledge.yaml` parses/validates against the mallm knowledge shape; tier logic ported pure (fs reads only); serves SM-19 features; legacy XDG knowledge migrated one-way per §5; no exec primitives |

## 8. Risks + open questions

**Risks**

- R1 Exec question unresolved blocks tiers 3–4 discovery parity — mitigated by SM-02 seam + Option A default.
- R2 Epic churn: P3 waits on M4 (sessions/SQLite not yet in the Rust tree — `smah_db.rs` is an M0
      scaffold stub); if M4 slips, SM-13/15 (config-only) can still land.
- R3 Formatter parity: chalk/rich styling differ; goldens only for `--no-color`; styled output is
      explicitly smah-native, not parity-bound.
- R4 Schema drift while Node mallm lives: vendored copy + pinned checksum; any mallm schema change
      during the window is an owner-visible re-freeze.
- R5 Redactor regex parity: TS vs Rust `regex` semantics differ (no lookaround in Rust); audit
      `src/changelog/redactor.ts` patterns at SM-14 and keep unportable patterns Node-side until retirement.
- R6 Dual-state divergence during coexistence (mallm writes XDG, smah writes `~/.smah`): one-way
      migrate + re-run before P6; docs are write-rare.
- R7 Corpus git adapter dropped in v1: git-sourced corpora stay Node-mallm-only until an exec-free
      fetch (e.g. dumb-HTTP) is justified — tracked as a parity-matrix waiver, not a silent gap.

**Decisions (owner — D1–D4 2026-11-09 · D5–D7 2026-11-10)**

1. **D1 — exec decision (was Q1, §4):** Option C — corpus-only + user-piped help now; `HelpCapture`
   seam kept; any exec reader (Option B) requires a further explicit owner decision.
2. **D2 — project-local tier directory (was Q2):** keep `.mallm/` (shared with Node mallm users
   during coexistence; no rename, no dual-read).
3. **D3 — how-to pipeline (was Q3):** re-home later as a SEPARATE non-SMAH product — not dropped;
   stays Node-only until the P6 spin-out hand-off.
4. **D4 — jev quiz/learn + memory/knowledge.yaml (was Q4):** port in P2; jev inference goes through
   `smah-providers` or the hosted Jev endpoint, with no exec (tickets SM-19/SM-20).
5. **D5 — history target (was Q5, assumed 2026-11-09):** SQLite `smah.db` as the single history
   store — import `history.jsonl` (§5 table); owner confirmed 2026-11-10.
6. **D6 — validation engine (was Q6, assumed 2026-11-09):** the maintained `jsonschema` crate
   against the vendored `mallm.schema.json`; owner confirmed 2026-11-10.
7. **D7 — retirement trigger (was Q7, assumed 2026-11-09):** parity matrix all green + an explicit
   owner call (no usage-signal gate); owner confirmed 2026-11-10; gates P6.
