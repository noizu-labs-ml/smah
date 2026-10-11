# SMAH Epics

Nine epics, ordered by dependency. E1 and E2 are independent of each other and come first (Phase 0). Stories are defined in `user-stories.md`; evidence citations refer to `capability-inventory.md` and the repo at HEAD `d803586`.

A note on regression guard: stories marked `[works today]` (US-07, US-08, US-18, US-20, US-23, US-24, US-25) are acceptance criteria inside whichever epic touches their code path — E1 must keep the query/pipe output contract while removing execution; E5 must keep behavior identical through the new subcommand surface and aliases.

---

## E1 — Safety & Trust: print-only handoff

**Goal:** Remove every code path that executes commands; replace the execute-confirm flow with a print-explain-handoff flow.

**Problem:** SMAH executes model-generated shell via `subprocess.run(cmd, shell=True)` after a single y/n confirm — query mode at `smah/runner/runner.py:394-429` (execution at `:429`) and resume mode at `runner.py:206-241` (execution at `:241`). The source comments it inline: `# This is dangerous`. There is no allowlist, timeout, output capture, or audit log; the `shell` attribute displayed in the panel is ignored; nothing about what ran is stored (inventory §8). The `exec-if` machinery compounds it with `eval()` on a model-controlled attribute (`smah/runner/response_parser.py:345`) — a latent code-execution vector even if it currently crashes rather than runs (empty conditions → `NameError`, itself swallowed into a silent truncation).

**Scope:**
- In: delete the executor and the execute-confirm prompts in query and resume modes; delete `exec-if`/`<set-condition>` dead machinery and its `eval()`; re-shape the suggestion block (title, command, what-it-does, per-flag explanation, risk class, blast radius, OS/shell mismatch warning); standing `SMAH does not run commands.` notice; clipboard handoff (`[c]opy` on TTY, stderr hint off-TTY); `--emit-cmd` raw-command output; prompt changes so the model explains rather than offers to act.
- Out: any execution toggle, sandbox, or allowlist (decision 1 is print-only, permanently); impact estimation as a separate engine (risk classification is prompt-driven explanation quality); changing pipe mode (it never executed).

**Stories:** US-09, US-10, US-11, US-12, US-13, US-14, US-15, US-16, US-17.

**Acceptance criteria:**
- [ ] `grep -rn "shell=True" smah/` returns nothing; `grep -rn "subprocess" smah/` returns nothing (or only non-command uses, ideally zero).
- [ ] `grep -rn "eval(" smah/` returns nothing.
- [ ] Every suggestion block renders risk class + blast radius + at least one handoff path.
- [ ] The notice string `SMAH does not run commands.` appears once per invocation with suggestions, and in `--help`.
- [ ] Query and resume modes behave as before except for the executor (answers, sessions, formatting contract preserved).
- [ ] Existing `tests/test_response_parser.py` still passes; new tests cover block rendering in plain and rich modes.

**Risks:** Prompt-only risk classification can be wrong or gamed by model output — mitigate by never presenting it as a guarantee (fixed wording "guidance, not a guarantee" in help) and by labeling destructive patterns client-side where cheap (`rm -rf`, `mkfs`, `dd`, `>` truncation, `--force`). Clipboard tooling varies by platform — degrade to the stderr hint, never fail. Removing `<set-condition>` machinery touches the response parser's test surface.

**Size:** M.

---

## E2 — Honesty Pass: delete advertised-but-absent surface

**Goal:** Make every flag, entrypoint, and documented command real, removed, or explicitly marked not-yet-available — no silent vaporware.

**Problem:** Seven visible flags parse to nothing: `--interactive` (stub; bare `smah` silently exits via `smah/smah.py:168-169` → `runner.py:512-513`), `--gui` (no GUI code exists; both configurator branches identical, `smah/settings/configurator.py:12-15`), `--model-picker`, `--model-review`, `--model-edit`, `--model-interactive`, `--openai-api-tier`, `--openai-api-org` (parsed, zero references — inventory §2). `smah-admin` prints `None` (`smah/smah_admin.py:245-248`; its `parse_arguments()` has no success-path return, `:217-238`). The README documents commands that never existed (`smah --setup`, README.md:43,56) and features with no code (interactive README.md:106-110, GUI README.md:164, monitoring README.md:25).

**Scope:**
- In: remove dead flags and `smah-admin` entrypoint; interim behavior for bare `smah` (stderr message + exit 2, honest "not yet" instead of silent exit); README rewritten to match reality (repo README — the brand site smah.pro is out of scope per decision 4); review/edit stage flags removed rather than implemented; TODO.md pruned or annotated to match decisions.
- Out: implementing any of the removed features; the smah.pro site rewrite; `smah-db` (works, stays as-is).

**Stories:** US-38 (interim half), US-39.

**Acceptance criteria:**
- [ ] Every flag in `--help` has an executing code path (grep rule: no parsed-but-unused args in `smah/args/args.py`).
- [ ] Removed flags fail with exit 2 and a message naming the replacement (back-compat table, interface-spec).
- [ ] `smah-admin` gone from `pyproject.toml` console scripts.
- [ ] Bare `smah` prints the not-yet message, exit 2 (until E9 makes it `chat`).
- [ ] README claims audited line-by-line against the inventory; zero known-false statements remain.

**Risks:** Removing flags breaks undocumented scripts that pass them — mitigated by keeping `--interactive` as a deprecated alias for the not-yet message rather than erroring on unknown flag at argparse level. README rewrite must not drift into marketing voice (owner decision 4 separates site copy).

**Size:** S.

---

## E3 — Reliability & Error Surfacing

**Goal:** Fail loudly and specifically: no swallowed exceptions, meaningful messages, the 0/2/3/4/5 exit-code contract, and `smah doctor`.

**Problem:** `main()` catches all exceptions and only logs them (`smah/smah.py:175-176`); the console logs at WARN while detail goes to rotating files (`smah/logs/logs.py:14`), so bad keys, dead models, network errors, and the `exec-if` `NameError` all manifest as silent no-op exits with exit code 0 (inventory §10). There is no health-check command, no error taxonomy, and scripts cannot detect failure. The system stats fed to prompts are wrong (memory readings report CPU data, `smah/settings/system/system.py:88`), so even "working" runs reason over false facts.

**Scope:**
- In: replace the broad except with typed error handling per the interface-spec error table; implement exit codes 0/2/3/4/5 end-to-end; `smah doctor` (config parse, db + migrations, key presence, enabled models, provider reachability, stale-id detection; `--json`); fix the CPU/memory swap at `system.py:88`; point error messages at the log file path.
- Out: retry/backoff policy for provider 5xx (a message is enough this cycle); telemetry of any kind; changing provider internals (E6).

**Stories:** US-05, US-33, US-34, US-35, US-36.

**Acceptance criteria:**
- [ ] No path exits 0 after a failure; CI asserts each exit code with fake provider responses.
- [ ] Every message in the error table renders verbatim (interface-spec).
- [ ] `smah doctor` healthy and broken outputs match the mockups; exit code is the worst finding.
- [ ] Memory stats in prompt YAML differ from CPU stats (regression test on `System.to_yaml`).
- [ ] A failure message always names the log file with the full detail.

**Risks:** Un-handled exception types surfacing as tracebacks — mitigate with a final catch that prints the generic message + log path + exit 5 and files an issue-worthy traceback in the log. Doctor's provider ping costs a token-tier call — make it the cheapest authenticated call available and skippable (`--offline`).

**Size:** M.

---

## E4 — Onboarding & First Run

**Goal:** A fresh install reaches a working first answer with no hand-edited files — or, when it cannot (no key), fails loudly with exit 4 — and setup states what it configures and where data goes.

**Problem:** The shipped catalog is frozen at late-2024 OpenAI ids (`gpt-4-turbo`, `gpt-4o`, `gpt-4o-mini`, `o1-preview`, `o1-mini` in `smah/settings/inference/inference_defaults.yaml`, with a hardcoded picker fallback `openai.gpt-4o-mini` at `smah/settings/inference/inference.py:29`) — `gpt-4o` and `gpt-4o-mini` ship enabled (`:74`, `:134`), the rest disabled, and `o1-preview` is decommissioned outright. The first-run defect is *not* the enabled flags: a keyless install still registers models, because registration gates on `enabled` + `is_configured()` and neither `Provider.is_configured()` (`provider.py:33-41`) nor `Model.is_configured()` (`model.py:76-83`) checks for an API key (`inference.py:37-42`). The provider call then fails at runtime and `main()`'s blanket except swallows it (`smah/smah.py:175-176`) — net effect: the new user's first commands appear to do nothing and exit 0. The fix is a preflight check, an exit-4 path naming what to configure, and error surfacing — not flipping enabled flags. The wizard also never asks which models to enable (`smah/settings/inference/configurator.py:22-32` asks only key + provider enable), the OS auto-detect prompt treats any input as yes (`smah/settings/system/operating_system/configurator.py:56`; manual path unreachable), and the wizard never states where data lives.

**Scope:**
- In: preflight key/config check before any provider call (exit 4 with the missing-key message; local check only — provider reachability stays in `doctor`); catalog staleness refresh — drop decommissioned ids, keep entries current (the defect is staleness, not the enabled flags; model ids in config per decision 2, interim OpenAI set until E6 lands Claude defaults); wizard gains a model-enable/default step; wizard states config/db/log paths and what is sent to the provider; fix the y/n auto-load bug; bounded wizard with skippable optional questions; setup ends by printing the first command to try.
- Out: flipping enabled flags as a "fix" (the shipped flags already enable `gpt-4o`/`gpt-4o-mini`); multi-provider selection UI (E6); key validation calls during the wizard (doctor's job); migrating existing configs beyond what already exists.

**Stories:** US-01, US-02, US-03, US-04, US-06.

**Acceptance criteria:**
- [ ] Fresh install → wizard → first `smah ask` succeeds, no file editing (verified in CI with a fake provider).
- [ ] Keyless install: `smah ask` exits 4 with the missing-key message before any provider call (CI: fake provider records zero calls).
- [ ] Wizard refuses to finish with zero enabled models.
- [ ] OS auto-detect question accepts explicit y/n; manual path reachable.
- [ ] Setup footer prints the three local paths + the egress statement.
- [ ] Catalog contains no ids the active provider no longer recognizes (staleness is the criterion, not flag states).
- [ ] Config schema accepts the refreshed catalog without version-gate rejection (`settings.py:27-39`).

**Risks:** Interim OpenAI catalog refresh becomes dead weight the moment E6 lands — keep it minimal (drop decommissioned ids) to avoid double migration. Preflight must stay local (key/config presence), or every ask pays an extra round-trip. Existing users' configs must not be silently rewritten; the wizard's confirm step covers it.

**Size:** M.

---

## E5 — Command Surface: subcommands + back-compat

**Goal:** Restructure the CLI into `ask` / `chat` / `history` / `resume` / `config` / `models` / `doctor` subcommands with back-compat flag aliases and deprecation messages (binding decision 3).

**Problem:** The current flat flag surface mixes working modes (`-q`, `-i`, `--continue`, `--session`, `--history`, `--configure`) with dead ones (E2) behind a single argparse blob (`smah/args/args.py`), giving no discoverable verb structure and no place to hang exit-code-aware behavior; mode dispatch is positional if/elif over flags (`smah/smah.py:147-174`), which is what let bare `smah` fall into the interactive stub.

**Scope:**
- In: subcommand tree per interface-spec (including the `models` verb: list/use/enable/disable/test); global flags (`--config`, `--database`, `-v`, `--model`, `--api-key`, `--rich/--no-rich`) shared; old flags routed as aliases with `[deprecation]` stderr notes; `smah ask` stdin handling spec'd (query+stdin = pipe mode; stdin-only = usage error; neither = usage error, replacing today's fall-through into the interactive stub).
- Out: `smah chat` implementation (E9 — E5 creates the verb with the honest not-yet message); multi-provider `models` content (E6); `--search` in history (E8).

**Stories:** US-29, US-30; keeps US-07/08/18/20/23/24/25 true through aliases.

**Acceptance criteria:**
- [ ] Every legacy invocation in the back-compat table behaves identically (golden-file tests for dispatch).
- [ ] Deprecation notes render once per invocation on stderr; removed flags exit 2 with replacement text from the table.
- [ ] `smah ask` with neither query nor stdin exits 2 with the usage message.
- [ ] Exit-code contract holds through the new surface.

**Risks:** Alias layer accreting into permanent dual syntax — set a removal version in each deprecation message (0.x major) and track it. Argparse nesting changes error text; keep usage errors in the established format.

**Size:** M.

---

## E6 — Provider Abstraction: Claude default

**Goal:** A Provider interface with Anthropic/Claude as default and OpenAI retained as one backend; model ids in config, enabled by default (binding decision 2).

**Problem:** Every layer is OpenAI-only: the sole client construction is `OpenAI(api_key=...)` (`smah/runner/runner.py:125-130`), `Runner.run()` returns `None` for any non-OpenAI model (`runner.py:260-291` — the caller then dies on `response.choices`, swallowed by main), and `Provider.api_key()` special-cases `identifier == "openai"` and returns `None` otherwise (`provider.py:43-54`). The config schema pretends to allow arbitrary providers (inventory §4) but nothing could ever authenticate or run. Current target ids per decision 2: `claude-opus-5-5` (query/pipe/chat default), `claude-haiku-5-5` (planner/picker).

**Scope:**
- In: Provider interface (auth, send, error mapping to the exit-code taxonomy); Anthropic backend (messages API, key via `ANTHROPIC_API_KEY`/config); OpenAI backend migrated onto the interface; defaults become `anthropic.claude-opus-5-5` + `anthropic.claude-haiku-5-5`; planner/picker structured-output prompt adapted per provider; `smah models` gains enable/disable across providers; catalog metadata (context, cost) refreshed for default entries.
- Out: streaming; tool/function calling; additional providers beyond the two (interface should make a third cheap, but none promised); the never-used `run_command_tool` prompt scaffolding (`prompts.py:16-44` — deleted, it was executor-adjacent).

**Stories:** US-31, US-32.

**Acceptance criteria:**
- [ ] `smah ask "..."` on a fresh configure-with-Anthropic works with zero OpenAI references in config.
- [ ] `smah models list` shows both providers; `smah models use openai.gpt-4o --for query` round-trips.
- [ ] Planner JSON-schema output parses on both backends (structured output or equivalent constrained decoding).
- [ ] Provider 401/403/5xx map to exits 4/4/5 with provider-named messages.
- [ ] Adding a stub third provider compiles against the interface without touching runner code (architecture test).

**Risks:** Two prompt/parsing dialects (planner JSON fidelity differs between providers) — mitigate with shared schema validation and per-provider fixtures. Default flip may strand early users who configured OpenAI keys only — doctor message covers it ("default provider changed; run smah config").

**Size:** L.

---

## E7 — Pipe & Output Discipline

**Goal:** Make `smah ask` a trustworthy pipeline citizen: answer-only stdout, TTY-aware formatting, honest handling of large input.

**Problem:** The stdout/stderr console split is right (`smah/console/console.py:15-18`) but undermined: query mode echoes the raw query to stdout unstyled (`print(query)`, `runner.py:375`); pipe input is silently truncated to 2048 chars (head 1024) before the planner (`runner.py:23-24`, `smah/runner/prompts.py:10-11`) with no notice to user or model; rich is unconditional rather than TTY-aware (`args.py:95` default True); prompts assume a TTY.

**Scope:**
- In: remove the query echo; TTY detection as the rich default (`--rich`/`--no-rich` force); no interactive prompts off-TTY (print alternatives instead); truncation notice (bytes seen vs supplied, stderr) while any cap survives; configurable `--max-input-chars`; chunk-and-merge strategy for oversized pipe input or an explicit documented refusal; prompt told when its input is partial.
- Out: changing what pipe mode does with the data (transformation semantics stay); new pipe features (formats, streaming).

**Stories:** US-19, US-21, US-22; keeps US-18/US-20 true.

**Acceptance criteria:**
- [ ] `smah ask "x" > out.md` yields exactly the answer (CI round-trip).
- [ ] Piping a >2 KB payload produces either a full-input result or a visible truncation warning; never silence.
- [ ] Off-TTY runs never block on prompts (CI with no pty).
- [ ] Rich defaults follow `-t 1` in both directions.

**Risks:** Chunk-and-merge multiplies provider calls and cost — start with a visible cap + notice, add chunking behind a flag only if demanded. TTY detection quirks under wrappers (expect, CI) — honor explicit flags above detection.

**Size:** M.

---

## E8 — Session & History Management

**Goal:** Make the sqlite history a first-class artifact: searchable, inspectable, deletable, and complete (suggestions included).

**Problem:** History access is a last-10 interactive picker only (`smah/smah.py:47-71`, `smah/database/database.py:89-112`) — no listing off-TTY, no search, no delete, no read-only view; and sessions store only response text, never the suggestion blocks (inventory §6), so the transcript — the artifact Priya pastes into tickets — is incomplete.

**Scope:**
- In: `smah history` non-TTY table + `--limit` (default 20) + `--search` over titles (local sqlite, no provider call); `--drop <id>` / `--drop --all`; `smah history --show <id>` read-only transcript; suggestion blocks persisted with messages and replayed on resume.
- Out: tagging/renaming sessions; export formats beyond plain transcript; cross-session search over message bodies (titles only this cycle).

**Stories:** US-26, US-27, US-28.

**Acceptance criteria:**
- [ ] Off-TTY `smah history | grep <term>` finds a March session.
- [ ] `--drop` removes all three tables' rows and reports counts.
- [ ] `--show` renders a session including suggestion blocks, no provider call, exit 0.
- [ ] Resume replays stored suggestions in the E1 block format.

**Risks:** Schema change for suggestion persistence — one additive migration via the existing `smah-db` runner (it works; inventory §1). Search over titles only must be stated in help to avoid the impression of full-text search.

**Size:** S.

---

## E9 — Interactive Mode: `smah chat`

**Goal:** Deliver the REPL that `--interactive` and bare `smah` always implied, on the chat default model.

**Problem:** `Runner.interactive()` is a two-line stub (`runner.py:512-513`); bare `smah` routes into it and silently exits (`smah/smah.py:168-169`) — the single worst UX bug in the prototype (inventory §10). Meanwhile a working multi-turn loop already exists inside resume mode (`Runner.resume`, `runner.py:165-249`): replays messages, prompts, appends each turn — the machinery to reuse.

**Scope:**
- In: `smah chat` REPL (default `claude-opus-5-5` per decision 2) reusing the resume loop's persistence; `/exit` handling and Ctrl-D; per-session `/model` switch; bare `smah` now enters chat (replacing E2's interim message); `--model-interactive` alias retired in favor of `smah chat --model`.
- Out: TUI chrome (full-screen, panels); slash-command language beyond `/model` and `/exit`; multiple concurrent sessions.

**Stories:** US-37, US-38 (final half).

**Acceptance criteria:**
- [ ] `smah chat` → multi-turn conversation, each turn persisted, resumable via `smah resume`.
- [ ] Bare `smah` enters chat (no args, configured machine).
- [ ] Ctrl-D or `exit` leaves cleanly with the session id printed.
- [ ] No execution surface anywhere in the loop (E1 contract holds; suggestions print with handoff).

**Risks:** Scope creep toward a TUI — the roadmap holds the line at REPL. Reusing resume's loop must not inherit its stored-args merge semantics for a fresh session (start clean).

**Size:** M.

---

## Dependency order

```
E1 Safety & Trust ─┐
                   ├─> E3 Reliability ─> E4 Onboarding ─> E5 Command Surface ─> E6 Providers ─> E7 Pipe/Output ─> E9 Chat
E2 Honesty Pass ───┘                                                                  └─> E8 Sessions (any time after E5)
```

E1 and E2 are parallel and first. E3 precedes E4 so onboarding failures surface properly. E5 follows E4 (defaults exist before the surface advertises them) and precedes E6 (the `models` verb exists before multi-provider content). E7 and E8 are independent of each other after E5; E9 is last.
