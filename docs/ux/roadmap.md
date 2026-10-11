# SMAH Roadmap

Phased, dependency-ordered, no calendar dates. Phases ship in order; epics and story IDs refer to `epics.md` and `user-stories.md`. The premise of the whole plan: SMAH is a dormant early prototype (last commit 2025-01-20, v0.1.13) whose working core (query, pipe, sessions) is worth keeping, whose executor must go, and whose honest gaps must close before any new capability is advertised.

---

## Phase 0 — Trust & Safety Recovery

**Goal:** Make SMAH safe to have on a machine and honest about what it is. Nothing new is added; two things are removed.

**Epics:** E1 (Safety & Trust — executor removal, print-only handoff), E2 (Honesty Pass — dead flags, `smah-admin`, README truth).

**Exit criteria:**
- No code path executes model-generated commands (`shell=True`/`subprocess`/`eval` greps clean).
- Every suggestion prints with explanation, risk class, and a handoff path; the standing notice renders.
- No advertised flag, entrypoint, or README command is vaporware; bare `smah` says "not yet" instead of exiting silently.

**A user can newly:** run `smah -q "..."` on a production-adjacent box and get commands that are explained, risk-labeled, and impossible to execute from inside SMAH — with one-keystroke copy into their own shell. Trust the README.

---

## Phase 1 — Usable Out of the Box

**Goal:** A fresh install works end-to-end: setup ends with a working model and key, keyless installs fail loudly (exit 4) instead of silently, and every failure is visible and exit-coded.

**Epics:** E3 (Reliability & Error Surfacing — typed errors, exit codes 0/2/3/4/5, `smah doctor`, CPU/memory stats fix), E4 (Onboarding & First Run — preflight key check + exit-4 path, catalog staleness refresh, model-choice step in the wizard, data-location disclosure, y/n bug fix).

**Exit criteria:**
- Fresh install → wizard → first query answers, no hand-edited files (CI-verified with a fake provider).
- Keyless installs exit 4 with the missing-key message before any provider call (today they register models anyway, fail at the call, and get swallowed into a silent exit 0).
- The error-message table renders verbatim for every failure class; scripts branch on exit codes.
- `smah doctor` distinguishes healthy from broken installs with actionable output.

**A user can newly:** install, configure once, and get an answer — and when something breaks (key, config, provider), see exactly what and fix it in one step.

---

## Phase 2 — Command Surface

**Goal:** The verb structure the product always needed: `ask` / `chat` / `history` / `resume` / `config` / `models` / `doctor`, with every legacy flag aliased and deprecation-noted.

**Epics:** E5 (Command Surface — subcommands, back-compat table, `models` verb skeleton, honest `chat` placeholder).

**Exit criteria:**
- Golden-file dispatch tests: every legacy invocation from the back-compat table behaves identically.
- All seven subcommands exist; removed flags exit 2 with replacement text.
- Exit-code contract holds through the new surface.

**A user can newly:** discover SMAH from `--help` alone (verb-first), manage defaults with `smah models use`, and keep old scripts working unchanged.

---

## Phase 3 — Provider Abstraction

**Goal:** Provider-neutral SMAH with Claude as the default and OpenAI retained as a backend; model ids in config, enabled by default.

**Epics:** E6 (Provider Abstraction — interface, Anthropic backend, `claude-opus-5-5` / `claude-haiku-5-5` defaults, cross-provider error mapping).

**Exit criteria:**
- Fresh configure-with-Anthropic runs with zero OpenAI references in config.
- `smah models` lists/uses/enables across both providers; provider errors map to the exit-code taxonomy.
- Architecture test: a stub third provider compiles against the interface without touching the runner.

**A user can newly:** use SMAH with their org's Anthropic key as the default path, or pin tasks to OpenAI models per task type — and switch without editing YAML.

---

## Phase 4 — Output & History

**Goal:** Trustworthy pipeline behavior and a history worth keeping.

**Epics:** E7 (Pipe & Output Discipline — answer-only stdout, TTY-aware rich, honest truncation), E8 (Session & History — search, list, delete, transcripts with suggestions).

**Exit criteria:**
- `smah ask ... > out.md` contains exactly the answer; no silent truncation of piped input; off-TTY runs never block on prompts.
- History is searchable and deletable; transcripts include suggestion blocks.

**A user can newly:** embed SMAH in pipelines reliably, find a session from months back, delete sensitive ones, and paste a resumed transcript (with suggestions) straight into a change ticket.

---

## Phase 5 — Interactive Chat

**Goal:** The REPL the `--interactive` flag always implied.

**Epics:** E9 (Interactive Mode — `smah chat` on the resume-loop machinery, bare `smah` enters chat).

**Exit criteria:**
- Multi-turn chat persisted per session and resumable; bare `smah` opens chat; E1's no-execution contract holds inside the loop.

**A user can newly:** explore a problem conversationally from a cold start — the current bare-`smah` silent exit becomes the product's front door.

---

## Explicitly not doing

- **Command execution — permanently.** The executor is removed, not disabled (decision 1). No config toggle, no `--yes-just-run` flag, no sandboxed "safe list". If this ever changes it is an explicit owner decision, not a setting that crept in.
- **The GUI.** `--gui` advertised a thing that never existed (no GUI code in the repo). Removed; none is planned.
- **`smah-admin` prompt/script library.** The entrypoint printed `None`; it is deleted. Instruction files (`-i`) cover the need; prompt management can be revisited only after interactive mode proves out.
- **AI-assisted monitoring** (README.md:25 claim) — dropped.
- **The README's 1-year roadmap items** (multi-user, secrets management, data visualization, automated reporting — README.md:358-393) — out of scope for this cycle; none has code.
- **Review/edit model stages.** No code exists behind `--model-review`/`--model-edit`; they are deleted rather than built. Output quality is an explanation/prompt concern (E1), not a second-pass model.
- **`exec-if` conditional execution and its `eval()` path** — deleted with the executor; a latent code-execution vector is not mothballed.
- **`--openai-api-tier` / `--openai-api-org`** — dropped outright (never referenced by any code).
- **TODO.md ideas** (saved commands, codebase scanning, `smah.my-command` executables) — parked; note `smah -c <name>` collides with the existing `-c/--config` flag if ever revived.
- **Brand site copy** — smah.pro is a separate workstream (decision 4); nothing in this doc set is site content.
