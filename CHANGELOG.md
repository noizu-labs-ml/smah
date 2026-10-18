CHANGE LOG
==============


## Changes

### Unreleased
October 18 2026

- Added `docs/PROJ-LAYOUT.md` (+ `docs/PROJ-LAYOUT.summary.md`): navigable
  project-tree map; marks generated (`_autosummary`), symlinked, and
  gitignored (`config/`, `.envrc.secret`/`.envrc.dev`) paths and the
  print-only safety invariant.

### 0.2.1
October 11 2026

Subcommand restructure per `docs/ux/interface-spec.md`:

- New command surface: `smah ask "<question>"` (one-shot; piped stdin is the
  data), `smah history`, `smah resume [ID|--last]`, `smah config
  [show|path|edit|set]`, `smah models list`, `smah doctor`, and `smah chat`
  (honestly not implemented: prints a notice and exits 2 — it is never faked).
- Every legacy flag (`-q/--query`, `-i/--instructions`, `--continue`,
  `--session`, `--history`, `--interactive`, `--configure`,
  `--openai-api-key`) still works and runs the new equivalent after a
  one-line `[deprecation]` notice on stderr. Legacy aliases are removed in
  0.4. `--interactive` never worked and now points at `smah chat`.
- `smah doctor` reports config found/valid, selected provider, API key
  presence (never the value), enabled model count and defaults, database/
  migration state, and per-stage resolved models. Read-only: no network, no
  provider clients, never creates files. Exit code is the worst finding
  (0 healthy, 3 config/database, 4 auth/models).
- `smah config show` prints the config with secret values masked (`$ENV`
  indirection shown as `<env:NAME>`); `smah config set key value` writes
  single dotted keys with bool/int coercion; `smah config edit` re-runs the
  wizard.
- `smah models list` shows enabled/disabled models, which is the default for
  query/pipe/picker, and notes the answering model is planner-chosen.
- Bare `smah` (no args, no piped stdin) prints a concise subcommand-pointing
  usage and exits 2. Piped stdin without a query asks for one explicitly.
- Pipe safety: stdout carries content only (answers and machine-readable
  output of `history`, `models list`, `doctor --json`, `config show`/`path`);
  all notices, deprecations, and diagnostics go to stderr. Exit codes survive
  piping. Rich formatting follows TTY detection; `--rich/--no-rich` override.
- Fresh-install hardening: `smah ask`/`resume` with no config file exit 3 with
  an actionable message ("Run 'smah config'") instead of launching a wizard
  that dies on EOF with an opaque internal error. The setup wizard only ever
  runs on a real terminal with stdin free; explicit `smah config`/`--configure`
  without a terminal exits 3 with "needs an interactive terminal".
- The setup wizard's entire UI (panels, prompts) now goes to stderr — stdout
  stays machine-consumable even mid-wizard.
- Only `ask` (and the legacy/bare piped path) consumes stdin. `doctor`,
  `models`, `history`, `config`, `resume`, and `chat` never read stdin, so an
  inherited open pipe (CI, daemons) can no longer hang them.
- Removed flags stay removed with honest messages (`--setup never existed`,
  etc.). New `--api-key` flag replaces `--openai-api-key` for the active
  provider.

### 0.2.0
October 11 2026

**BREAKING: SMAH no longer executes commands.** The shell-execution path
(`subprocess.run(..., shell=True)` behind a single y/n confirm) has been removed
from both the query and resume flows. Suggested commands are now printed in a
clearly delimited panel with the model's title/purpose explanation, an explicit
notice that SMAH does not run commands, and (when the optional `pyperclip`
package happens to be installed) a best-effort clipboard copy. Without
`pyperclip`, SMAH silently degrades to print-only; clipboard support is not a
dependency.

Safety and reliability wave:

- Failures are no longer swallowed. Unexpected errors now print a readable
  message to stderr (full traceback only with `-v`) and exit nonzero instead of
  logging silently and exiting 0.
- Single source of truth for exit codes (`smah/exit_codes.py`): 0 success,
  1 unexpected internal error, 2 usage error, 3 config error, 4 auth or
  no-usable-model (including provider not wired up yet), 5 provider/API error.
  All exit paths (smah, smah-db) route through it.
- Keyless installs fail fast with exit 4 and a message naming what to configure
  (env vars, `--openai-api-key`, or `smah --configure`) instead of silently
  doing nothing after the API call fails.
- Default model catalog is now Claude-first: Anthropic provider with
  `claude-opus-5-5` (query/pipe work) and `claude-haiku-5-5` (planner/picker
  stage); existing OpenAI entries are unchanged. Anthropic is fully wired up
  (see the provider abstraction entry below): a default install with only
  `ANTHROPIC_API_KEY` set gets a working Claude query out of the box.
- Planner robustness: a planner model that answers in prose instead of the
  required JSON (possible wherever JSON mode is an instruction rather than a
  native parameter) now fails cleanly with exit 5 and a message naming the
  planner, instead of a JSONDecodeError traceback.
- Model-picker candidate probing is tolerant: an unknown or unusable provider
  key listed among picker candidates is skipped and the next candidate is
  tried; the hard exit-3 for an unknown provider applies only to the provider
  actually chosen to run.
- Anthropic `max_tokens` honors the model's configured output budget
  (catalog `context.out`, or a model-settings override) and falls back to
  16000 only when the entry carries no budget.
- Credential resolution decoupled from the CLI namespace: a provider
  constructed without `args` still consults its configured settings block
  (a key in the config file with nothing in the environment no longer
  produces a spurious exit-4 depending on construction path).
- The provider abstraction no longer borrows OpenAI's `NOT_GIVEN`/`NotGiven`
  sentinel: a vendor-neutral `UNSET` sentinel in `smah/providers/base.py`
  expresses "parameter not set" across the contract and the Anthropic
  provider; only `openai_provider.py` touches the OpenAI SDK's sentinel,
  translating `UNSET` to `NOT_GIVEN` when it builds its request.
- The unset-sentinel boundary is tolerant: `is_unset()` in the neutral layer
  treats `UNSET`, `None`, and — defensively, duck-typed by class name so no
  vendor SDK is imported — another SDK's sentinel instance as "not set". A
  stray vendor sentinel can never be serialized into a request body.
- Candidate probing is resilient to malformed provider config, not just
  unknown keys: an exception while constructing a known-key provider, or a
  settings block whose `api_key` cannot be read (wrong shape/type), skips
  that candidate with a debug-level breadcrumb (visible with `-v`) and the
  next candidate is tried. The hard exit for the provider actually chosen to
  run is unchanged and now also covers a malformed block: clean exit 3 with
  a readable `malformed ... smah --configure` message, never a traceback
  and never a silent skip.
- Provider abstraction (`smah/providers/`): the runner now resolves providers
  by config key through a registry (`openai`, `anthropic`) behind a shared
  contract (credential resolution without ever printing keys, normalized
  `Completion` text, typed-exception-to-exit-code translation) instead of
  branching on `provider == "openai"`. The exit-4 "only the OpenAI provider
  is wired up" path is gone; unknown provider keys in config exit 3. The
  OpenAI request path is behavior-preserving (same credentials rules, request
  parameters, and payload logging). Anthropic calls go through the official
  `anthropic` SDK (new dependency): system prompt as the top-level `system`
  parameter, required `max_tokens` (16000), no `budget_tokens`/`temperature`/
  `top_p`, and content parsed block-by-block (`type == "text"` only) rather
  than assuming `content[0]`. Provider errors map to exit codes: missing key
  or rejected key → 4 (naming the env var for that provider, plus
  `smah --configure`), rate limit / API 4xx-5xx / network → 5, unknown
  provider key → 3. Credential preflight runs before any network call.
- Model picking is credential-aware: the picker prefers the first listed
  model whose provider has a resolvable key, falling through the task list to
  the default list — so an OpenAI-key-only install still gets a working
  model, and a Claude-first install with `ANTHROPIC_API_KEY` runs Claude.
- Security fix: `exec-if` conditions from model output were evaluated with
  Python `eval()` (arbitrary code execution from model-supplied strings; it
  also crashed with NameError whenever a condition referenced a variable and
  the conditions list was empty). Conditions are now parsed with `ast` and
  walked over a strict whitelist (constants, names, not/and/or,
  ==/!=/</<=/>/>=/in/not in; JSON-style `true`/`false`/`null` normalized).
  Anything else — calls, attribute access, subscripts, unknown names — skips
  the command with a notice on stderr instead of executing.
- Flag honesty pass: removed `--gui` (no GUI exists), `--model-picker`,
  `--model-interactive`, `--model-review`, `--model-edit`, `--openai-api-tier`
  and `--openai-api-org` (parsed but never used). `--interactive` now states it
  is not implemented and exits 2 instead of silently doing nothing; bare `smah`
  with no query and no stdin prints concise usage and exits 2.
- Removed the `smah-admin` console script and its scaffolding module (it only
  ever printed `None`).
- Restructured `--help` into argument groups (input, sessions, models, output,
  configuration, diagnostics) with an examples epilog; nothing undocumented.
- Fixed system stats reporting CPU readings as memory readings in prompts.
- Dropped four unused runtime dependencies: `aiohttp`, `requests`, `readchar`,
  `prompt-toolkit`.

### 0.1.13
December 25 2024

- Improved db setup flow removed requirement to invoke smah-db migrate when running smah after initial install/update.
- PyPI Config update (repos/description).

