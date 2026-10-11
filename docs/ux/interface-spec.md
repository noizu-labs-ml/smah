# SMAH Interface Spec

The concrete CLI design for the recovered SMAH. Implements binding decisions 1–3: print-only execution, Claude-default provider abstraction (`claude-opus-5-5` query/pipe/chat, `claude-haiku-5-5` picker), and the subcommand surface with exit codes `0 ok · 2 usage · 3 config · 4 auth-or-no-models · 5 provider`.

Everything here is forward-looking design except where marked **today**, in which case it cites current behavior from `capability-inventory.md` at HEAD `d803586`.

---

## 1. Command surface

```
smah [<global flags>] <command> [<args>]
```

Commands: `ask`, `chat`, `history`, `resume`, `config`, `models`, `doctor`. Global flags apply to every command. `smah` with no command enters `chat` once E9 ships; until then it prints the not-yet notice and exits 2 (today it routes into an empty stub and exits silently — `smah/smah.py:168-169`, `runner.py:512-513`).

### Global flags

| Flag | Meaning | Notes |
|---|---|---|
| `-c, --config PATH` | Config file | Default `~/.smah/config.yaml` (unchanged **today**, `args.py:60`) |
| `--database PATH` | Session db | Default `~/.smah/smah.db` (unchanged **today**) |
| `-v` (repeatable) | Verbosity | `-v` plans, `-vv` payloads, `-vvv` settings dump (matches **today**, `runner.py:318,334`); all verbose output to stderr |
| `--model ID` | Per-invocation model override | Fully qualified (`anthropic.claude-opus-5-5`); does not persist |
| `--api-key KEY` | Key for the active provider | Replaces `--openai-api-key` (**today**: `args.py:84`, consumed at `provider.py:43-53`) |
| `--rich` / `--no-rich` | Force formatting on/off | Default: TTY detection (see §5) |

---

### smah ask

**Synopsis:**
```
smah ask [<query>] [--instructions FILE] [--emit-cmd] [--max-input-chars N]
```
**Replaces:** `smah -q "<query>"` (query mode), `smah -i FILE` (`args.py:57-58`), and the piped-stdin forms `<cmd> | smah -q "..."`.

**Args/flags:**
- `<query>` — the question. Optional only if stdin is piped *and* an instruction is somehow given; see behavior matrix.
- `--instructions FILE` (alias `-i`) — file whose contents become the query (**today**: `smah.py:192-201`).
- `--emit-cmd` — print only the first suggested command, raw, to stdout (US-12).
- `--max-input-chars N` — pipe-input cap, default per config (see E7).

**Behavior:**

| stdin | query/`-i` | mode |
|---|---|---|
| no | yes | query mode: planner call picks model/format, then answer (matches **today** `runner.py:359-446`) |
| piped | yes | pipe mode: stdin is data, query is the instruction (matches **today** `args.py:97-107`) |
| piped | no | usage error, exit 2 (**today**: falls into the interactive stub and silently exits) |
| no | no | usage error, exit 2 (**today**: same silent-exit trap) |

Suggested commands are rendered as suggestion blocks (§3) and **never executed**. On a TTY, each block ends with a `[c]opy / [s]kip` prompt; off-TTY the copy hint prints to stderr. Sessions persist to sqlite exactly as **today** (inventory §6). The planner picker model defaults to `claude-haiku-5-5`; the answer model to `claude-opus-5-5` (decision 2).

**Exit codes:** `0` answer produced (warnings such as truncation do not change this) · `2` usage per matrix · `3` unreadable/invalid config · `4` missing/invalid key, zero enabled models · `5` provider 4xx/5xx, network failure.

---

### smah chat

**Synopsis:**
```
smah chat [--model ID]
```
**Replaces:** `smah --interactive` (dead stub **today**, `args.py:59`) and, after E9, bare `smah`.

**Behavior:** REPL on the chat-default model (`claude-opus-5-5`). Each turn is persisted to the session from the first message; `/model ID` switches mid-session; `exit` or Ctrl-D quits, printing the session id for later `smah resume`. Reuses the existing multi-turn loop from resume mode (**today**: `Runner.resume`, `runner.py:165-249`, is the only working REPL). Suggestion blocks inside chat follow the same print-only contract. Until E9 ships, the verb exists and prints: `chat is not available yet — use 'smah ask' for one-shot queries` (exit 2), so the surface is honest from Phase 2.

**Exit codes:** `0` clean exit · `2` usage / not-yet · `3/4/5` as per ask.

---

### smah history

**Synopsis:**
```
smah history [--limit N] [--search TEXT] [--show ID] [--drop ID | --all] [--yes]
```
**Replaces:** `smah --history` (**today**: interactive picker over the last 10 sessions, `smah.py:47-71`, `database.py:89-112`).

**Behavior:** TTY without other flags → the interactive picker (unchanged behavior, now over `--limit` default 20). Non-TTY → plain table to stdout: `ID  MODIFIED  TITLE`. `--search` matches titles (sqlite LIKE, local only). `--show ID` prints a read-only transcript including suggestion blocks (E8). `--drop` deletes a session (all three tables) and reports row counts; `--drop --all` requires `--yes` off-TTY or a TTY confirm. **Today** none of listing/search/delete exists — only the 10-item picker.

**Exit codes:** `0` listed/shown/dropped · `2` unknown ID, `--drop --all` without confirmation · `3` db unreadable/migrations stale.

---

### smah resume

**Synopsis:**
```
smah resume [<id>]
```
**Replaces:** `smah --continue` (`args.py:63`) and `smah --session N` (`args.py:64`).

**Behavior:** No `<id>` → resume the most recent session (the `settings.last_session` pointer, **today** `database.py:28-44`). With `<id>` → that session. Replays stored messages (suggestion blocks included after E8), then continues the REPL loop exactly as **today** (`runner.py:165-249`), appending each turn. Stored args still merge over current CLI args where the CLI left them unset (**today**: `smah.py:87`, `args.py:5-19`). No execute prompt anywhere in the loop (E1).

**Exit codes:** `0` session ended cleanly · `2` unknown id (message lists recent ids) · `3/4/5` as per ask.

---

### smah config

**Synopsis:**
```
smah config [show | path]
```
**Replaces:** `smah --configure` (`args.py:62`) and the README's never-existent `smah --setup` (README.md:43,56 — **today** argparse rejects it: "unrecognized arguments").

**Behavior:** No subcommand → the setup wizard (**today**: `--configure` → `settings/configurator.py:17-25`, sequential user/system/inference wizards). The wizard now includes a model-enable/default step (US-03), states local paths and data egress (US-06), and refuses to finish with zero enabled models. `show` prints the redacted current config (keys masked); `path` prints the config file path. Ctrl-C aborts without touching the existing file (US-02).

**Exit codes:** `0` configured/shown · `2` `show` of a nonexistent config with no offer to run the wizard (message points at `smah config`) · `3` config exists but fails validation.

---

### smah models

**Synopsis:**
```
smah models list
smah models use <id> [--for query|pipe|chat|picker]
smah models enable <id> | disable <id>
smah models test [<id>]
```
**Replaces:** nothing — new. (The dead `--model-picker`/`--model-review`/`--model-edit` flags it superficially resembles never did anything, `args.py:76,80-81`.)

**Behavior:** `list` renders the table in §3. `use` persists a task default (default task: `query`); validates the id exists in the catalog at set time (US-30). `enable`/`disable` flip catalog entries and write config. `test` performs the cheapest authenticated call against the provider for the given (or each enabled) model and reports ok/failure with the provider's message — the same check `doctor` uses.

**Exit codes:** `0` success · `2` unknown subcommand/unknown id · `3` config write failure · `4` `test` auth failure · `5` `test` provider failure.

---

### smah doctor

**Synopsis:**
```
smah doctor [--json] [--offline]
```
**Replaces:** nothing — new (US-34).

**Behavior:** Runs, in order: config parse (path, schema, version gate), database (present, migrations current), key presence for enabled providers, enabled-model count, defaults set, provider reachability (skipped with `--offline`), stale-id detection (ids the provider no longer recognizes — catches the frozen-catalog class of failure, inventory §4). Output per §3. Exit code is the worst finding: `0` all pass · `3` config · `4` key/models · `5` provider unreachable. `--json` emits machine-readable results (one object per check: name, status, detail).

---

## 2. Back-compat table

Old form → new form → exact deprecation message (stderr, once per invocation). **kept** = old form still works through 0.x majors; **removed** = old form exits 2 with the message.

| Old | New | Status | Deprecation / error message |
|---|---|---|---|
| `smah -q "<query>"` | `smah ask "<query>"` | kept | `[deprecation] 'smah -q' works, but prefer 'smah ask'. Alias removes in 0.4.` |
| `smah -i FILE` | `smah ask --instructions FILE` | kept | `[deprecation] 'smah -i' works, but prefer 'smah ask --instructions'. Alias removes in 0.4.` |
| `<cmd> \| smah -q "..."` | `<cmd> \| smah ask "..."` | kept | (same as `-q` row) |
| `smah --interactive` | `smah chat` | kept* | `[deprecation] --interactive never worked in prior releases. Use 'smah chat'.` (*prints the chat not-yet message until E9) |
| bare `smah` | `smah chat` | n/a | Until E9: `chat is not available yet — use 'smah ask' for one-shot queries` (exit 2; replaces today's silent exit) |
| `smah --continue` | `smah resume` | kept | `[deprecation] '--continue' works, but prefer 'smah resume'. Alias removes in 0.4.` |
| `smah --session N` | `smah resume N` | kept | `[deprecation] '--session N' works, but prefer 'smah resume N'. Alias removes in 0.4.` |
| `smah --history` | `smah history` | kept | `[deprecation] '--history' works, but prefer 'smah history'. Alias removes in 0.4.` |
| `smah --configure` | `smah config` | kept | `[deprecation] '--configure' works, but prefer 'smah config'. Alias removes in 0.4.` |
| `smah --setup` (README-documented, never existed) | `smah config` | removed | `error: --setup never existed in any release. Run 'smah config'.` (exit 2) |
| `smah --model ID` | unchanged (global/per-command flag) | n/a | — |
| `smah --model-query ID` / `--model-pipe ID` | unchanged, valid on `ask` | n/a | — |
| `smah --model-picker ID` | (config: picker default) | removed | `error: --model-picker was never implemented. Set the picker via 'smah models use <id> --for picker'.` (exit 2) |
| `smah --model-interactive ID` | `smah chat --model ID` | removed | `error: --model-interactive was never implemented. Use 'smah chat --model'.` (exit 2) |
| `smah --model-review ID` | — | removed | `error: --model-review was never implemented; no review stage exists or is planned.` (exit 2) |
| `smah --model-edit ID` | — | removed | `error: --model-edit was never implemented; no edit stage exists or is planned.` (exit 2) |
| `smah --openai-api-key KEY` | `smah --api-key KEY` | kept | `[deprecation] '--openai-api-key' works, but prefer '--api-key' (applies to the active provider). Removes in 0.4.` |
| `smah --openai-api-tier N` | — | removed | `error: --openai-api-tier was never implemented. (removed)` (exit 2) |
| `smah --openai-api-org ORG` | — | removed | `error: --openai-api-org was never implemented. (removed)` (exit 2) |
| `smah --gui` | — | removed | `error: no GUI exists in SMAH and none is planned. (removed)` (exit 2) |
| `smah-admin ...` | — | removed (entrypoint) | `error: smah-admin was scaffolding that printed 'None' and has been removed.` (exit 2) |
| `smah-db ...` | unchanged | n/a | Works today (`smah_migrate.py:42-53`); stays as-is this cycle |

---

## 3. Mockups

### 3.1 Top-level `--help`

```
$ smah --help
usage: smah [<global flags>] <command> [<args>]

SMAH — the sysadmin helper. SMAH does not run commands: it drafts
them, explains them, and hands them off for you to review and run.

Commands:
  ask       Ask a one-shot question (argument or piped stdin)
  chat      Start an interactive session
  resume    Resume a saved session (default: most recent)
  history   List, search, show, or drop saved sessions
  config    Run setup, or show the config
  models    List, enable, and select models
  doctor    Check the installation and report problems

Global flags:
  -c, --config PATH     Config file (default: ~/.smah/config.yaml)
      --database PATH   Session database (default: ~/.smah/smah.db)
  -v, --verbose         Repeatable: -v plans, -vv payloads, -vvv settings
      --model ID        Override the model for this run
      --api-key KEY     API key for the active provider (else env/config)
      --rich/--no-rich  Force formatting on/off (default: TTY detection)

Exit codes: 0 ok · 2 usage · 3 config · 4 auth or no models · 5 provider

Data & config live under ~/.smah/ — see 'smah doctor' for exact paths.
```

### 3.2 `smah ask` output (TTY shown; stream discipline in §5)

```
$ smah ask "rotate nginx logs and compress last week's"

  smah · anthropic.claude-opus-5-5 · session 142                      [stderr]

  ## Rotating and compressing nginx logs                                        [stdout]

  nginx rotates logs on its own schedule via logrotate on this host;
  for an immediate rotation of the current files:

  ┌─ Suggested command ───────────────────────────────────────────────┐
  │                                                                   │
  │    logrotate -f /etc/logrotate.d/nginx                            │
  │                                                                   │
  │  What it does: forces the nginx logrotate policy to run now,     │
  │  renaming access.log/error.log and signalling nginx to reopen.   │
  │  --vacuum-style compression is not part of this policy.          │
  │                                                                   │
  │  Flags:                                                           │
  │    -f              force rotation now (not just when due)        │
  │                                                                   │
  │  Risk: REVERSIBLE — moves/renames log files; no data deleted.    │
  │  Touches: /var/log/nginx/*, nginx master process (reopen signal) │
  │                                                                   │
  │  SMAH does not run commands. Review, then run it yourself.       │
  └───────────────────────────────────────────────────────────────────┘

  [c]opy command  [s]kip                                               [stderr, TTY]

  Copied to clipboard.                                                 [stderr]
```

The same block off-TTY prints the fenced command, explanation, risk, and touch lines (plain text, stdout) with no prompts; the standing notice moves entirely to stderr. The risk line is model-generated guidance, labeled as such in `--help` — never a guarantee.

### 3.3 First run

```
$ smah ask "check why cron didn't fire"

  No config found at /Users/ina/.smah/config.yaml — first-run setup (~1 min).

  [1/4] your background   [2/4] system   [3/4] provider & models   [4/4] verify

  ? Name (any value, used to personalize prompts): Ines
  ? System admin experience: Intermediate
  ? Role: Administrator
  ? About you (optional, blank to skip):
  ? Detect OS details automatically? [y/n] (y)
      detected: darwin 24.6.0 · zsh
  ? Provider: anthropic (recommended) / openai
  ? API key (blank to use $ANTHROPIC_API_KEY): ••••
  ? Default model for queries/pipes/chat:
      claude-opus-5-5  (recommended)
      claude-haiku-5-5
  ? Planner/picker model: claude-haiku-5-5  (recommended)

  Stored:
    config    ~/.smah/config.yaml
    sessions  ~/.smah/smah.db
    logs      ~/.smah/logs/
  Sent to the provider with every query: your question, piped input,
  and your background/system profile above.

  Ready. Try: smah ask "check why cron didn't fire"
```

Notes: the wizard asks y/n with explicit choices (fixes the bug where any input triggered auto-detect — `operating_system/configurator.py:56`); optional fields are skippable (US-04); the model step exists (US-03); the disclosure block is US-06.

### 3.4 `smah doctor` — healthy

```
$ smah doctor

  smah 0.2.0 · python 3.12.4 · darwin 24.6.0

  config      ok      /Users/ina/.smah/config.yaml (vsn 0.0.1)
  database    ok      /Users/ina/.smah/smah.db · 27 sessions · schema current
  key         ok      anthropic: present (not printed)
  models      ok      2 enabled · defaults set
  catalog     ok      no stale model ids
  provider    ok      anthropic reachable (claude-haiku-5-5 ping, 214 ms)

  All checks passed.
  $ echo $?
  0
```

### 3.5 `smah doctor` — broken

```
$ smah doctor

  smah 0.2.0 · python 3.12.4 · darwin 24.6.0

  config      ok      /Users/ina/.smah/config.yaml (vsn 0.0.1)
  database    ok      /Users/ina/.smah/smah.db · 27 sessions · schema current
  key         FAIL    anthropic: no api key found
                      fix: 'smah config' or export ANTHROPIC_API_KEY
  models      ok      2 enabled · defaults set
  catalog     ok      no stale model ids
  provider    FAIL    anthropic: can't authenticate — no api key

  2 checks failed.
  $ echo $?
  4
```

### 3.6 `smah models list`

```
$ smah models list

  PROVIDER    MODEL                DEFAULT FOR      ENABLED   STATUS
  anthropic   claude-opus-5-5      query pipe chat  yes       ok
  anthropic   claude-haiku-5-5     picker           yes       ok
  openai      gpt-4o               —                no        disabled
  openai      gpt-4o-mini          —                no        disabled

  defaults: query/pipe/chat = anthropic.claude-opus-5-5 · picker = anthropic.claude-haiku-5-5
  change with: smah models use <model> [--for query|pipe|chat|picker]
```

Zero enabled models is a successful listing (exit 0) — the failure class belongs to `ask`/`doctor`, not to `list`.

---

## 4. Error messages

Exact user-facing text (single line each, stderr). `<provider>` is the active provider id; `<path>` the resolved config path; `<detail>` the provider/library message, trimmed to one line. Every error ends by naming the log file with full detail.

| Condition | Message | Exit |
|---|---|---|
| No query and no piped stdin | `smah: 'ask' needs a query argument or piped stdin. Try: smah ask "..." or: some-cmd \| smah ask "..."` | 2 |
| Unknown flag / removed flag | per back-compat table §2 | 2 |
| Config file unreadable/invalid | `smah: can't read config at <path>: <detail>. Fix it, or move it aside and run 'smah config'.` | 3 |
| Config fails version/schema gate | `smah: config at <path> is invalid: <detail> (expected vsn 0.0.1). Run 'smah config' to regenerate.` | 3 |
| Session db unreadable / migrations stale | `smah: session database problem at <path>: <detail>. Try 'smah-db status'.` | 3 |
| Missing API key | `smah: no API key for provider "<provider>". Run 'smah config' to add one, or export <PROVIDER>_API_KEY.` | 4 |
| Zero models enabled | `smah: no models enabled. Run 'smah models enable <suggested-id>' or 'smah config'.` | 4 |
| Provider auth rejected (401/403) | `smah: provider "<provider>" rejected the API key (HTTP <code>). Run 'smah config' to update it.` | 4 |
| Provider 4xx (non-auth) | `smah: provider "<provider>" rejected the request (HTTP <code>): <detail>. Check model ids with 'smah models list'.` | 5 |
| Provider 5xx | `smah: provider "<provider>" is failing (HTTP <code>). Try again shortly, or 'smah models use <alt>' to switch.` | 5 |
| Network failure / timeout | `smah: can't reach provider "<provider>" (<detail>). Check network/DNS, then retry.` | 5 |
| Unexpected internal error | `smah: internal error: <detail>. Full traceback in <logfile>. Please file an issue.` | 5 |

This replaces today's behavior where every failure class above is swallowed by the broad except at `smah/smah.py:175-176` and the process exits 0 silently.

---

## 5. Pipe-safety contract

Rules that keep `smah` usable as a pipeline stage (protecting Ines, P4):

1. **stdout carries content only:** the model's answer (markdown or plain per formatting mode), the suggestion block's content (command fence, explanation, risk, touch lines), and machine output of `history`/`models list`/`doctor --json`. Nothing else ever writes to stdout.
2. **stderr carries chrome:** mode banner and session id, the standing `SMAH does not run commands.` notice, deprecation notes, pickers and handoff prompts, warnings (truncation), all error messages. The existing console split is the foundation (`smah/console/console.py:15-18`) and is preserved.
3. **The query echo dies.** Today `Runner.query` prints the raw query to stdout unstyled (`smah/runner/runner.py:375`) — removed; query-mode output becomes as redirect-clean as pipe mode.
4. **Rich follows the TTY.** Default: rich when stdout is a TTY, plain otherwise. `--rich` forces on (human piping into `less`), `--no-rich` forces off (machine consumption). No interactive prompt may appear when stdout is not a TTY; off-TTY runs print their alternative (the copy hint, the numbered list instead of a picker) instead of blocking.
5. **Exit codes survive piping.** Failure exits are nonzero regardless of redirection; a consuming script never mistakes an empty stdout for success. Truncation warnings are stderr and do not alter the exit code.
6. **First byte discipline.** When piped, the first stdout byte is the answer's first character — no leading blank lines or banners (CI round-trip: `smah ask "x" | wc -c` equals the answer's byte length).
7. **Input truncation is loud.** While any `--max-input-chars` cap survives, exceeding it prints `input truncated: sent 2048 of 98304 bytes (--max-input-chars to change)` to stderr, and the model is told its input is partial (today the model sees a 1024-char head with no such hint — `runner.py:23-24`, `prompts.py:10-11`).
