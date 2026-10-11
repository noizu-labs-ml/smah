# SMAH User Stories

39 stories grouped by theme, each mapped to a persona (P1 Priya / P2 Marcus / P3 Dana / P4 Ines — see `personas.md`) and to an epic (E1–E9, see `epics.md`).

State markers (assessed at HEAD `d803586`, per `capability-inventory.md`):

- `[works today]` — verified working end-to-end in the current code. Some will change behavior under Phase 0 (executor removal); noted inline.
- `[broken]` — a code path exists but fails or actively harms the experience.
- `[missing]` — no code exists.

Acceptance criteria are the forward contract: for `[works today]` stories they define what must remain true; for the others they define done.

---

## First run & onboarding

### US-01 — First query works straight after setup `[broken]`
As **Dana (P3)**, I want to complete first-run setup and immediately get an answer to my first question, so that my introduction to SMAH is a success rather than a silent no-op.
Epic: E4.
- [ ] Fresh install → run `smah ask "..."` → completes setup → gets an answer, no hand-edited files.
- [ ] A keyless install fails loudly *before* any provider call: exit 4 with the missing-key message. Today the opposite happens — models register anyway because registration gates on `enabled` + `is_configured()` and neither check looks for an API key (`smah/settings/inference/inference.py:37-42`, `provider.py:33-41`, `model.py:76-83`), the provider call then fails at runtime, and `smah/smah.py:175-176` swallows it into a silent exit 0.
- [ ] Setup ends by printing the exact next command to try.

### US-02 — Re-run setup non-destructively `[works today]`
As **Marcus (P2)**, I want to re-run the setup wizard any time to change my key or models, so that rotating credentials or switching providers does not mean deleting my config.
Epic: E4.
- [ ] `smah config` re-runs the wizard pre-filled from the existing `~/.smah/config.yaml` (today: `--configure` does this, inventory §3).
- [ ] Sessions in `~/.smah/smah.db` survive reconfiguration.
- [ ] Ctrl-C during the wizard leaves the previous config intact.

### US-03 — Choose models during setup `[missing]`
As **Priya (P1)**, I want setup to show which models it will enable and let me change that, so that I know what my questions are costing and where they are going.
Epic: E4.
- [ ] The inference step of the wizard lists models with provider, id, enabled state (today the wizard never asks; `smah/settings/inference/configurator.py:22-32` only asks for key + provider enable).
- [ ] Selecting a default model persists it as the task default.
- [ ] At least one enabled model is enforced before the wizard exits.

### US-04 — Skip optional interview questions `[missing]`
As **Dana (P3)**, I want to skip optional personalization questions with sensible defaults, so that setup takes one minute when I am in a hurry.
Epic: E4.
- [ ] Free-text "about" and instructions fields accept empty with a default.
- [ ] Skipped fields degrade gracefully in prompts (no empty-string persona block).
- [ ] Total wizard length is bounded (≤ 8 prompts on the fast path).

### US-05 — Be told my key is missing or invalid `[broken]`
As **Dana (P3)**, I want a clear message when my API key is absent or rejected, so that I fix the right thing instead of assuming SMAH is broken.
Epic: E3.
- [ ] Missing key → exact message per error table (interface-spec), exit 4 (today: swallowed by the broad except at `smah/smah.py:175-176`; console stays silent because log level is WARN).
- [ ] Invalid key (provider 401/403) → distinct message naming the provider, exit 4.
- [ ] Message names both remediation paths: `smah config` and the provider env var.

### US-06 — Know what data goes where `[missing]`
As **Priya (P1)**, I want setup and help to state what is stored where and what is sent to the provider, so that I can clear data-egress review before my first real query.
Epic: E4.
- [ ] Setup footer states: config at `~/.smah/config.yaml`, sessions at `~/.smah/smah.db`, logs under `~/.smah/logs/`, and that query text + configured system/user profile are sent to the model provider.
- [ ] `smah doctor` repeats the same inventory of local paths.
- [ ] Docs page matches the statement (no marketing claims).

---

## One-shot questions & command suggestions

### US-07 — One-line question, formatted answer `[works today]`
As **Dana (P3)**, I want to ask a question in a single line and get a formatted answer, so that SMAH is as fast as a shell alias.
Epic: regression guard (E5 keeps it true under subcommands).
- [ ] `smah ask "how do I ..."` prints a rich-markdown answer (today: `-q` path, `smah/runner/runner.py:359-446`).
- [ ] Works identically through the `ask` subcommand and the `-q` alias.
- [ ] Answer lands on stdout, chrome on stderr (see US-19).

### US-08 — Long prompts from a file `[works today]`
As **Dana (P3)**, I want to point at an instruction file instead of typing a long prompt, so that repeated rituals are one command.
Epic: regression guard (E5).
- [ ] `smah ask --instructions FILE` uses the file contents as the query (today: `-i`, `smah/smah.py:192-201`).
- [ ] Missing file → config-class error message, exit 3.

### US-09 — Structured command suggestion block `[works today, changes under Phase 0]`
As **Marcus (P2)**, I want each suggested command delivered as a block with title, purpose, and the command itself, so that I can evaluate it before touching my keyboard.
Epic: E1 (re-shapes the block).
- [ ] `<exec>` extraction produces one block per command with title/purpose/command (today: `ResponseParser.extract_commands`, inventory §7/§8).
- [ ] After E1: block additionally carries what-it-does, risk classification, and handoff — and no execute prompt anywhere.
- [ ] Multiple commands render as multiple blocks, in document order.

### US-10 — Never offered command execution `[broken]`
As **Priya (P1)**, I want SMAH to never offer to execute a command, so that using it on production-adjacent work cannot violate my change-control policy even by accident.
Epic: E1.
- [ ] No code path invokes `subprocess` on model output (today: `subprocess.run(cmd, shell=True)` after one y/n at `smah/runner/runner.py:241` and `:429`, commented `# This is dangerous` in source).
- [ ] No y/n "execute?" prompt exists in any mode (query, resume, future chat).
- [ ] Grep for `shell=True` over the package returns nothing.
- [ ] Release notes state the removal plainly.

### US-11 — One-keystroke clipboard copy `[missing]`
As **Marcus (P2)**, I want the suggested command on my clipboard with one keystroke, so that print-only costs me nothing versus my browser habit.
Epic: E1.
- [ ] TTY: suggestion block ends with a `[c]opy / [s]kip` prompt; `c` puts the raw command (no markdown) on the clipboard (pbcopy/xclip/wl-copy by platform).
- [ ] Non-TTY: no prompt, no hang; instead a trailing stderr hint (`# smah: pipe me through pbcopy`).
- [ ] Copy works when multiple blocks are shown (copies the focused one; `1..n` selects).

### US-12 — Paste-ready shell handoff `[missing]`
As **Marcus (P2)**, I want an alternative handoff that emits the command where my shell can use it, so that clipboard tools I do not have are not a blocker.
Epic: E1.
- [ ] `smah ask --emit-cmd` prints only the raw command to stdout (composable: `$(smah ask --emit-cmd "..." )` reviewed via `echo` first).
- [ ] Help shows the review-then-run idiom explicitly, with a warning to read before running.
- [ ] No wrapper that auto-executes the emitted command ships in SMAH itself.

### US-13 — Per-flag explanation `[missing]`
As **Dana (P3)**, I want each flag of a suggested command explained in one line, so that I learn the tool instead of pasting magic.
Epic: E1.
- [ ] Suggestion block lists non-obvious flags with a short gloss (`--vacuum-size=200M` → "shrink journal to 200MB total").
- [ ] Obvious flags (`-l`, common ones) may be grouped, not omitted silently — a "common flags" line is acceptable.
- [ ] Explanation is prompt-driven (model output), not a static flag database.

### US-14 — Standing "SMAH does not run commands" notice `[missing]`
As **Priya (P1)**, I want the output itself to state that SMAH does not run commands, so that a colleague watching my screen (or an auditor reading my transcript) sees the policy without reading docs.
Epic: E1.
- [ ] Every invocation with at least one suggestion prints the notice once (stderr; also in the block footer on TTY).
- [ ] Wording is fixed and greppable: `SMAH does not run commands.`
- [ ] The top-level `--help` carries the same sentence.

### US-15 — Risk classification per suggestion `[missing]`
As **Priya (P1)**, I want each suggested command labeled by risk class (read-only / reversible / destructive / irreversible), so that I can triage review depth in seconds.
Epic: E1 (this replaces the old "impact estimator" concept as an explanation-quality concern).
- [ ] Block shows a risk line with one of the four classes and a one-phrase justification.
- [ ] Destructive/irreversible classes are visually distinct (color/style) on TTY and plain-text marked (`Risk: IRREVERSIBLE —`) when piped.
- [ ] Classification is model-generated guidance, presented as such — never as a safety guarantee (wording in help).

### US-16 — Blast-radius note `[missing]`
As **Priya (P1)**, I want to know what a command touches before running it, so that my change ticket's impact section writes itself.
Epic: E1.
- [ ] Block lists affected paths/services/hosts where determinable (`/var/log/journal/*`, `service nginx`).
- [ ] Commands with remote effect (ssh, network tools) say so explicitly.
- [ ] Absence of a note reads "scope: this host" — never silence.

### US-17 — Mismatch warning against my OS/shell `[missing]`
As **Dana (P3)**, I want a warning when a suggestion assumes an OS or shell different from my configured one, so that macOS-vs-Linux differences stop biting me.
Epic: E1.
- [ ] Configured OS/shell (from settings) compared against the suggestion's evident target; mismatch → warning line in the block.
- [ ] Warning is advisory text, not a block on output.
- [ ] Depends on system facts being correct (see E4: `smah/settings/system/system.py:88` reports CPU readings as memory).

---

## Pipe processing

### US-18 — Pipe data in, get analysis out `[works today]`
As **Ines (P4)**, I want to pipe data into SMAH with an instruction and get the processed result on stdout, so that SMAH is a pipeline stage like jq.
Epic: regression guard (E7 protects it).
- [ ] `cat log | smah ask "summarize errors"` returns the analysis (today: pipe path, `smah/runner/runner.py:449-509`).
- [ ] Session saved so the same input can be re-queried (today: pipe input is persisted, inventory §6).

### US-19 — Answer-only stdout `[broken]`
As **Ines (P4)**, I want nothing but the answer on stdout, so that `smah ask ... > out.md` is safe to redirect.
Epic: E7.
- [ ] All chrome (banners, notices, plans) on stderr (the console split exists, `smah/console/console.py:15-18`).
- [ ] The raw-query echo at `smah/runner/runner.py:375` (`print(query)`) is removed from stdout.
- [ ] Round-trip check in CI: `smah ask "x" | wc -l` equals the answer's line count alone.

### US-20 — Choose rich vs raw output `[works today]`
As **Ines (P4)**, I want explicit control over formatting, so that downstream tools get plain text while I get readable markdown.
Epic: regression guard (E7 formalizes default).
- [ ] `--rich` / `--no-rich` still force formatting on/off (today: `smah/args/args.py:95`).
- [ ] Default becomes TTY-detection (not unconditional rich) per interface-spec.
- [ ] `--no-rich` implies no interactive prompts.

### US-21 — Handle inputs larger than 2 KB `[broken]`
As **Ines (P4)**, I want large piped inputs handled honestly — processed or explicitly chunked — so that summaries reflect all my data, not the first kilobyte.
Epic: E7.
- [ ] Today's silent truncation (MAX 2048 chars / head 1024, `smah/runner/runner.py:23-24`, `smah/runner/prompts.py:10-11`) is removed or made chunked-with-merge.
- [ ] If a cap remains for cost reasons, it is configurable (`--max-input-chars`) and never silent.
- [ ] Large-input strategy documented (chunk summaries merged by a second call).

### US-22 — Truncation is visible `[missing]`
As **Ines (P4)**, I want a loud notice when input was cut, so that I never trust a summary of data SMAH did not read.
Epic: E7.
- [ ] Truncation (any surviving cap) prints bytes-seen vs bytes-supplied to stderr.
- [ ] The model prompt also receives the fact of truncation (it currently sees a preview without knowing it is partial).
- [ ] Exit code remains 0 (this is a warning, not an error).

---

## Session resume

### US-23 — Continue the last conversation `[works today]`
As **Marcus (P2)**, I want to pick up my last session where I left it, so that multi-evening tasks keep context.
Epic: regression guard (E5 aliases it).
- [ ] `smah resume` (today `--continue`) restores and continues the latest session (inventory §3/§6).
- [ ] Stored args merge with current CLI args (today: `smah/smah.py:87`, `smah/args/args.py:5-19`).

### US-24 — Resume by id `[works today]`
As **Marcus (P2)**, I want to resume a specific session by id, so that parallel threads of work stay separate.
Epic: regression guard (E5 aliases it).
- [ ] `smah resume <id>` (today `--session N`) restores that session (inventory §6).
- [ ] Unknown id → usage-class error naming valid ids (today: silent failure class).

### US-25 — Pick from recent sessions `[works today]`
As **Marcus (P2)**, I want a picker over recent sessions, so that I do not have to remember ids.
Epic: E8 extends; E5 aliases.
- [ ] TTY `smah history` shows the interactive picker (today `--history`: last 10, `smah/smah.py:47-71`, `smah/database/database.py:89-112`).
- [ ] Selection resumes the session.

### US-26 — List and search beyond the last 10 `[missing]`
As **Marcus (P2)**, I want to list and search all sessions, so that the thing I did in March is findable.
Epic: E8.
- [ ] `smah history --limit N` (default 20) and `--search TEXT` over titles.
- [ ] Non-TTY `smah history` prints a plain table (id, date, title) to stdout.
- [ ] Search is local (sqlite LIKE over titles; no provider call).

### US-27 — Delete old sessions `[missing]`
As **Ines (P4)**, I want to delete sessions, so that sensitive piped data does not live in sqlite forever.
Epic: E8.
- [ ] `smah history --drop <id>` deletes session + details + messages.
- [ ] `smah history --drop --all` with a confirm (TTY) / `--yes` (non-TTY).
- [ ] Deletion reports rows removed and the db path.

### US-28 — Suggestions visible in session history `[missing]`
As **Dana (P3)**, I want past command suggestions preserved and reviewable in a resumed session, so that the transcript is the artifact I paste into tickets.
Epic: E8.
- [ ] Suggestion blocks stored with the message (today: only response text is stored; executed commands were never stored — inventory §6 — and suggestions fare the same).
- [ ] `smah resume` replays them as blocks.
- [ ] `smah history --show <id>` prints a session read-only including suggestions.

---

## Config & model management

### US-29 — List models `[missing]`
As **Priya (P1)**, I want to list models with provider and defaults, so that I can see what my questions cost and where they go.
Epic: E5 (subcommand), E6 (multi-provider content).
- [ ] `smah models list` prints the table per interface-spec mockup.
- [ ] Shows enabled state and default-for (query/pipe/chat/picker).
- [ ] Works when zero models are enabled — that is a listing, not an error (exit 0).

### US-30 — Set defaults without editing YAML `[missing]`
As **Priya (P1)**, I want to change the default model for a task with one command, so that tuning cost/quality does not mean hand-editing config.
Epic: E5.
- [ ] `smah models use <id> [--for query|pipe|chat|picker]` persists the default.
- [ ] Validation rejects unknown model ids at set time, not first use.
- [ ] Today's `--model` remains a per-invocation override that does not persist.

### US-31 — Anthropic out of the box `[missing]`
As **Dana (P3)**, I want to use Anthropic models without touching OpenAI, so that I can use the provider my org standardized on.
Epic: E6.
- [ ] Provider interface with Anthropic backend; `claude-opus-5-5` default for query/pipe/chat, `claude-haiku-5-5` default picker (binding decision 2).
- [ ] OpenAI remains selectable (today: OpenAI-only at every layer, `smah/runner/runner.py:260-291`, `smah/settings/inference/provider/provider.py:43-54`).
- [ ] Key resolution per provider env var (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY`) plus config.

### US-32 — Current model ids without hand-editing `[broken]`
As **Marcus (P2)**, I want the shipped catalog to name models that exist and work, so that I never debug a decommissioned id.
Epic: E6 (interim relief in E4).
- [ ] No hardcoded late-2024 ids in code paths (today: `gpt-4-turbo`/`gpt-4o`/`gpt-4o-mini`/`o1-preview`/`o1-mini` frozen in `smah/settings/inference/inference_defaults.yaml`, and a picker fallback at `smah/settings/inference/inference.py:29`).
- [ ] Model ids live in config with sane enabled-by-default values (binding decision 2).
- [ ] `smah doctor` flags ids the provider no longer recognizes.

### US-33 — Validate config in one command `[missing]`
As **Priya (P1)**, I want a config check that tells me what is wrong and where, so that I am never left guessing which field broke.
Epic: E3.
- [ ] `smah doctor` parses config and reports file path, schema errors with the offending key, exit 3 on bad config.
- [ ] `smah config show` renders redacted current values for eyeballing.

---

## Diagnosing a broken install

### US-34 — One-command health check `[missing]`
As **Dana (P3)**, I want `smah doctor` to check key, config, db, and provider reachability, so that "is it me or the tool" is one command.
Epic: E3.
- [ ] Checks: config parse, db present + migrations current, key present, ≥1 enabled model, provider reachable (cheap authenticated call).
- [ ] Output per interface-spec mockups (healthy and broken).
- [ ] Exit code = worst finding (0/3/4/5); `--json` for scripting.

### US-35 — Distinguish failure classes `[missing]`
As **Dana (P3)**, I want auth, network, and config failures distinguishable by message and exit code, so that my script and I both know what to fix.
Epic: E3.
- [ ] Error table in interface-spec implemented verbatim.
- [ ] Exit codes: 3 config, 4 auth/no-models, 5 provider/network.
- [ ] Today's behavior — all exceptions swallowed at `smah/smah.py:175-176`, invisible at WARN console level — is gone; no silent no-op exits anywhere.

### US-36 — Nonzero exit on failure `[broken]`
As **Marcus (P2)**, I want every failure to exit nonzero, so that my scripts stop on SMAH errors instead of consuming empty output.
Epic: E3.
- [ ] All five exit codes wired per interface-spec (today: effectively always 0 — errors are logged and swallowed).
- [ ] CI asserts exit codes for each failure class (fake provider responses).

### US-37 — Interactive chat from scratch `[broken]`
As **Dana (P3)**, I want a real interactive session without first creating one via a query, so that exploring a problem does not require a fake first question.
Epic: E9.
- [ ] `smah chat` opens a REPL on the default chat model; exit via `exit`/Ctrl-D.
- [ ] Session persisted from the first turn, resumable via `smah resume`.
- [ ] Today: `--interactive` is an empty stub (`smah/runner/runner.py:512-513`); bare `smah` routes into it and silently exits.

### US-38 — Bare `smah` does something sensible `[broken]`
As **Marcus (P2)**, I want `smah` with no arguments to help me, so that the first command I ever type is not a silent no-op.
Epic: E2 (interim), E9 (final).
- [ ] Phase 0: bare `smah` prints "interactive mode is not available yet — see `smah ask`" to stderr, exit 2 (today: silent exit via the stub, `smah/smah.py:168-169`).
- [ ] Phase 5: bare `smah` enters `smah chat`.

### US-39 — Every documented flag works `[broken]`
As **Dana (P3)**, I want every flag and entrypoint that appears in `--help` or the README to actually work, so that I stop discovering vaporware by trying it.
Epic: E2 (removal), E5 (surface).
- [ ] The seven dead flags are removed or implemented: `--interactive`, `--gui`, `--model-picker`, `--model-review`, `--model-edit`, `--model-interactive`, `--openai-api-tier`, `--openai-api-org` (inventory §2; `smah/args/args.py:59,76,79-81,83,85,94`).
- [ ] `smah-admin` entrypoint removed (prints `None` today, `smah/smah_admin.py:245-248`) or implemented — decision: removed.
- [ ] README documents only real behavior (`smah --setup` at README.md:43,56 never existed; `--gui` at README.md:164 has no code; interactive at README.md:106-110 is a stub).
- [ ] Grep-able rule: no parsed-but-unused arguments in `smah/args/args.py`.
