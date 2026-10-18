# Data Flow — SMAH Request Lifecycle

Stage-by-stage path of a request through SMAH. Overview: [../PROJ-ARCH.md](../PROJ-ARCH.md).

## 1. Startup & Logging

`smah.logs.configure()` first: rotating DEBUG file log under `~/.smah/logs/`
(1 MB × 3 backups) + WARN console stream to **stderr** (stdout stays
content-only). Then `args.extract_args()` parses, validates, normalizes, and
reads piped stdin — **only** when the command is `ask` or the bare legacy
path. `doctor`/`models`/`history`/`config` never read stdin, so an open
non-tty stdin in CI cannot hang them. Deprecation notices for legacy flags
print to stderr before dispatch.

## 2. Dispatch

Subcommands first: `chat` (honest stub — notice + exit 2) · `doctor` ·
`models` · `config` (show/path/set/wizard) · `history` · `resume` (init
database, then resume by id or `--last`) · `ask` (init database, then run).
Legacy flags second, each mapped to its new equivalent (`--continue` →
resume, `--session N` → resume by id, `--history` → interactive pick,
`--interactive` → chat notice, `--configure` → wizard). Bare path: legacy
`-q`/`--instructions` query or the usage screen (exit 2).

## 3. Settings

`Settings` loads one YAML file — default `~/.smah/config.yaml`, override
`--config` — with `User`, `System`, `Inference` sections (version-gated
`vsn <= "0.0.1"`). Missing/incomplete config ⇒ exit 3 with "run
'smah config'"; the wizard only launches on a real TTY with stdin unconsumed
(never EOF-crashes a pipe).

## 4. Plan (first LLM call)

`Runner.query_plan()` calls the task model for the task (`inference_model("query")`
etc.): walks the `model_picker` list for the task, falling through to the
`default` list, probing each candidate with `maybe_get_provider` +
`has_credentials()` (tolerant skip). The planner prompt embeds the model
catalog as YAML and returns a **strict JSON plan**: `{title, model, reason,
include_settings(+reason), format_output(+reason), instructions}`. Invalid
JSON ⇒ fatal exit 5. The planner is the router: its plan selects the
answering model, settings injection, and output formatting.

## 5. Answer (second LLM call)

The full thread goes to the planned model via `provider.complete()`: NPL
conventions primer, operator profile + (if planned) system settings as an
ack-required message, then the query — or the piped input inside a
`--- INPUT ---` block (planner saw only a head-1024/tail-2048 truncation;
the answer call gets the pipe **in full**). `pipe` requests demand raw,
fence-free, comment-free output.

## 6. Render

Answers print via rich panels (or plain). `ResponseParser.extract_commands()`
(lxml with custom element-class lookup) finds `<exec shell title purpose>`
tags; optional `exec-if` conditions evaluate through a whitelisted AST walker
(constants, and/or/not, comparisons, `in` — no `eval()`/`exec()`;
unevaluable ⇒ command skipped). Commands render as "SUGGESTED COMMAND — NOT
EXECUTED BY SMAH" panels with an explicit not-executed notice; clipboard
copy is best-effort via optional `pyperclip`. Pipe output goes raw to stdout
(Markdown only if the planner said `format_output` **and** `--rich`).

## 7. Persist

`db.save_chat()` writes transactionally: `chat_history` (id, title,
timestamps), `chat_history_details` (args JSON, plan JSON, pipe input),
`chat_history_message` (full message thread), and the `settings` last-session
pointer. `smah resume` merges stored session args back (RESERVED_ARG_KEYS
`command`/`deprecated` protected from overwrite), rebuilds the Runner, and
runs an interactive REPL until `exit` or empty line.

## 8. Error Funnel

`KeyboardInterrupt` → 1 · `yaml.YAMLError` → 3 · `OpenAIError` → 5 ·
anything else → 1. Tracebacks print only with `-v`. Providers translate
recognized vendor errors to the shared table: auth → 4, provider/API → 5.
Preflight `require_api_key()` runs before any network call: malformed config
block → 3, missing key → 4 with credential help.
