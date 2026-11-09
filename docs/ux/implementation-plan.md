# SMAH Implementation Plan — `epic.rust-tui` delivery

Status basis: `develop` @ `b6eced4` + epic M0 (`67525c3`), verified 2026-10-18.
`epics.md` / `user-stories.md` remain the **requirement contracts** (story ids and
acceptance criteria used verbatim below); `roadmap.md`'s phase sequencing is
**superseded** by the approved Rust rewrite plan
(`~/.claude/plans/fuzzy-tickling-kurzweil.md`) — same scope, new vehicle: one
epic branch, milestone PRs M0–M9, full-parity cutover to 0.3.0.

## Superseding decisions (2026-10-18, user-approved rewrite)

| Roadmap said | Now |
|---|---|
| E9 "holds the line at REPL — no TUI chrome" | `smah chat` ships as a ratatui TUI (hybrid surface); line-REPL remains the non-TTY fallback |
| E6 "out: streaming" | TUI chat/resume stream tokens; `ask`/pipe stay non-streaming (§5.6 first-byte discipline) |
| E8 "suggestion blocks persisted" (additive migration) | **No schema change**: stored message text is re-parsed deterministically on resume/`--show` (meets US-28's intent; parser is a pure function of stored content) |
| E7 "chunk-and-merge … only if demanded" | Parked, as the roadmap itself suggested: cap + loud notice only (`--max-input-chars`, §5.7) |

`exec-if` note: roadmap "not doing" killed conditional **execution**. 0.2.x
already reduced exec-if to display-only condition evaluation (whitelisted AST,
nothing executes). The Rust port keeps exactly that semantics; the execution
linkage stays dead permanently.

## Python-side status vs the epics (verified at b6eced4)

0.2.0/0.2.1 already landed most of E1–E6 in Python. Fixed since the epics'
v0.1.13 assessment: memory/CPU stats cross-wiring (system.py), README `--setup`
and GUI vaporware. **Still broken in Python** (all fixed in the Rust port, no
backports — Python retires at cutover):

1. Wizard OS auto-detect treats any non-empty input as yes (`operating_system/configurator.py:55-56`).
2. Decommissioned ids shipped in `inference_defaults.yaml` (`gpt-4-turbo`, `o1-preview`, `o1-mini`, disabled) + picker drift: yaml header vs code fallback at `inference.py:29` disagree.
3. Wizard has no model-enable/default step (`inference/configurator.py:28-38`) — US-03/US-04 unmet.
4. Query echo on stdout in query mode (`runner.py:416 print(query)`) — violates §5 rule 3 / US-19.
5. Silent pipe truncation head-1024/tail-2048 with no notice, model unaware (`runner.py:30-31`, `prompts.py:277-300`) — US-21/US-22.
6. `smah models use/enable/disable/test` **do not exist** (models_cmd.py is list-only) — yet `args.py:25`'s removed-flag error tells users to run `smah models use <id> --for picker`. The Rust port makes the referenced command real (US-29/US-30).
7. README residuals: "(pending)" claims at README.md:25 (AI-assisted monitoring), :43, :107 (history search) — E2 truth pass at cutover.

## Epic → milestone map

| Epic | Python today | Lands in |
|---|---|---|
| E1 Safety & Trust | core landed 0.2.0 (print-only, test+CI enforced) | M3 (block fields US-13/15/16/17, notice US-14), M7 (TUI block US-09/11) |
| E2 Honesty Pass | mostly landed (dead flags → exit-2 table, smah-admin gone, --setup/GUI gone) | M5 (alias/removed-flag table), M9 (README "(pending)" residuals, TODO.md prune, spec errata) |
| E3 Reliability | landed 0.2.1 (exit codes, doctor, error table) minus known doctor bug | M1 (§4 templates + snapshots), M5 (doctor w/ fixed provider check, --json/--offline, stale-ids) |
| E4 Onboarding | partial (preflight exit-4 landed; wizard gaps remain) | M8 (4-step wizard: model step US-03, skippable US-04, y/n fix, disclosure US-06, zero-enabled refusal), M2 (catalog refresh) |
| E5 Command Surface | landed 0.2.1 (subcommands + aliases) | M5 (clap tree, back-compat table tests, US-29/30 models verb) |
| E6 Providers | landed 0.2.0 (anthropic+openai, Claude default) | M2 (hand-rolled clients, error mapping, catalog single-source) |
| E7 Pipe discipline | NOT landed (echo + silent truncation live) | M3 (§5.7, TTY-rich, off-TTY non-blocking) |
| E8 Sessions | partial (picker only; search/show/drop missing) | M4 (US-26/27/28, smah-db parity), M7/M8 (TUI picker US-25) |
| E9 Chat | stub (honest exit-2) | M6 (headless REPL + resume merge), M7 (TUI) |

## Story → milestone register (all 39)

| Stories | Milestone | Notes |
|---|---|---|
| US-01, 02, 03, 04, 06 | M8 (+M2 catalog) | wizard: prefill re-run (02), model step (03), skippable ≤8 prompts (04), paths+egress disclosure (06); first-answer-after-setup CI-verified w/ fake provider (01) |
| US-05 | M2/M5 | exit-4 preflight + exact §4 strings; 401/403 distinct |
| US-07, 08 | M5 | regression guards through subcommands; `-i` missing file → exit 3 |
| US-09, 13, 14, 15, 16, 17 | M3/M7 | block: title/command/what-it-does/per-flag gloss/risk class (4 classes, visually distinct)/blast radius/OS-shell mismatch advisory; notice once per invocation + in `--help`; prompts request the fields (model-generated, "guidance not a guarantee") |
| US-10 | M0–M9 (structural) | zero `process::Command` in tree, empty-allowlist test + CI gate |
| US-11 | M7 (+M3 off-TTY hint) | `[c]opy/[s]kip`; multi-block `1..n` focus select; arboard degrades to stderr hint |
| US-12 | M5 | `--emit-cmd` raw stdout + review-then-run idiom in help |
| US-18, 19, 20, 21, 22 | M3 | echo dies; `--rich/--no-rich` force, TTY default; §5.7 `--max-input-chars` + loud stderr bytes-seen/supplied + model told partial; exit stays 0 |
| US-23, 24, 25 | M4/M6 (+M7 picker) | resume last/by-id; unknown id → exit 2 naming recent ids; stored-args merge (RESERVED keys protected) |
| US-26, 27, 28 | M4 | `--limit` (20) / `--search` titles LIKE / `--drop [--all] --yes` w/ row counts + db path / `--show` read-only replay incl. re-parsed suggestions |
| US-29, 30 | M5 | `models list` (exit 0 at zero-enabled), `use [--for query\|pipe\|chat\|picker]` validated at set time — makes `args.py:25`'s referenced command real |
| US-31, 32 | M2 | anthropic default-first picker; catalog: drop decommissioned ids, single source (no yaml-vs-code drift); doctor flags stale ids |
| US-33, 34, 35, 36 | M1/M5 | §4 table verbatim (internal=1 erratum), doctor config/db/keys/models/catalog/provider, worst-finding exit, `--json`, `--offline` |
| US-37, 38 | M6 | chat REPL persisted-from-first-turn; bare `smah` → chat (no interim not-yet needed — chat ships in this epic) |
| US-39 | M5/M9 | every §2 row tested; removed flags exact exit-2 text; README truth at cutover |

## Milestone acceptance gates

Each milestone PR additionally asserts its stories' acceptance criteria from
`user-stories.md` (referenced, not duplicated). Cross-cutting gates every PR:
`cargo fmt/clippy -D warnings/test` green · no-shell-exec scan · parity tests
unbroken · CHANGELOG line. E1/E3 acceptance greps from `epics.md` are
re-expressed in Rust as: source scan (no `Command`/`spawn`/`exec`), §4
snapshot suite, doctor mockup-shape tests.

## Cutover (M9) — adds to the approved plan's checklist

- README truth pass: remove "(pending)" claims (README.md:25, :43, :107); document the TUI; install via release binaries.
- TODO.md prune/annotate against the "explicitly not doing" list (roadmap.md — still binding: no execution toggle, no GUI, no smah-admin, no review/edit stages, no monitoring, no 1-year-roadmap items).
- interface-spec errata: §4 internal-error row → exit 1; §5.7 "today" note; §3.4 runtime header; §1 synopsis gains `resume --last`.
- Docs: PROJ-ARCH/PROJ-LAYOUT/THREAT-MODEL updates as per approved plan (T-001/T-003 → mitigated; B5 → GitHub Releases; T-007 fixed via SHA-pinned actions).
- Version **0.3.0**; epic → develop regular merge; monorepo gitlink bump after release reaches main.

## Future (tracked, not this epic)

**Virtual-tool family** — tickets #16–#25 (umbrella: #16 virtual-tool-prd; drivers: #17 cli,
#18 tui, #19 web; interface-first emulators: #20 api, #21 grpc, #22 soap, #23 mcp; stateful:
#24 db, #25 vfs). Gated on M3/M6/M7 landing; PRD must carry the threat-model section before
any family implementation PR; five planning agents spawn at PRD sign-off. Cache/state split:
`~/.cache/smah/virtual-tools/<tool>/` (regenerable artifacts) · `~/.local/state/smah/`
(durable state).
