CHANGE LOG
==============


## Changes

### Unreleased
October 18 2026

- docs: virtual-tools family index refreshed — all nine drivers shipped, per-driver contract links; THREAT-MODEL.summary.md synced for T-026…T-030 (register now 30 items: 18 mitigated · 6 partial · 3 open · 3 accepted).
- Rust #23/#24/#25: virtual-tools wave 3 drivers — `mcp` (smah process IS the MCP server: one-turn fabricated catalog → frozen `mcp-tools.json` keyed by tool-file digest, memoized `tools/call` via `sha256(name+NUL+canonical-args)` replay, JSON-RPC 2.0 batch stdio + opt-in loopback-only `--tcp-port`, caps 64 tools/16 KiB schema/256 KiB catalog/64 KiB call text; standalone zero-dep Elixir replay service `services/mcp-vtool` with own SHA-pinned `ci-mcp-vtool.yml` — replays frozen artifacts only, never fabricates, no OS processes), `db` (PG-wire-protocol subset over unix socket / loopback TCP: model fabricates a JSON schema spec — never SQL — into an in-tree rusqlite store served read-only with branded `smah-virtual-postgres … FABRICATED DATA` ParameterStatus, writes refused 42501, extended protocol/multi-statement refused 0A000, 256 KiB message + 64 KiB statement caps, ≤64 tables/64 cols/500 rows/2000 rows total), `vfs` (fabricated tree as real files under `~/.cache/smah/virtual-tools/<tool>/vfs/root/`, caps 200 files/64 KiB/1 MiB, code-owned fs tools `fs_list/fs_read/fs_write/fs_stat/fs_generate`, canonicalized containment + symlink refusal — client escape refused per-request and audited, fabrication escape aborts exit 3); shared: `--no-serve`/`--regen` + per-driver `--show-*` flags, unknown-arg exit 3, fabrication turns folded into `RunOutcome::turns`, metadata-only server-op audit lines; `docs/ux/virtual-tools-{mcp,db,vfs}-driver.md` specs; THREAT-MODEL B7 boundary + T-026…T-030. Rust: 253 tests green, clippy -D warnings clean; Elixir service: 30 tests green, format-checked.
- Rust #22/#21: virtual-tools wave 2 — soap + grpc drivers. `drivers/soap.rs`: WSDL-first mock SOAP service — `interface:` carries inline WSDL or a path (validate-then-freeze, sha-pinned `virtual-tools/<tool>/soap/` cache, validate-on-load, tool edits re-freeze), loopback-only axum host serves the frozen WSDL byte-for-byte at `?wsdl` and answers posted envelopes by LLM fabrication, re-validated against the declared message parts (drift ⇒ bounded retry then `DirectiveLoopOverflow`); requests parsed by a DTD/entity-denying scanner (DOCTYPE ⇒ `soap:Fault`, no expansion, no external fetch — T-022), metadata-only audit (`soap.request`, contents never logged — T-015 pattern); `--port` (loopback-only, 0 = OS pick), `--no-serve`, `--max-serve-seconds`. `drivers/grpc.rs`: proto-first mock gRPC server — `interface:` carries inline `.proto` or a path, runtime protox parse into a `DescriptorPool` (bounded, size-capped — T-023), tonic `Codec` pair driven by `prost-reflect` dynamic messages (server-side encode=output/decode=input), UNARY-ONLY v1 (streaming ⇒ trailers-only grpc-status 12 before any fabrication), unknown method/service ⇒ 12, fabricated protojson strictly deserialized against the declared output descriptor (T-024), loopback-only h2 prior-knowledge listener with loud stderr disclosure (T-025). Both: no model-generated contracts (`interface:` required, missing ⇒ hard Config exit 3), fabrication serialized through a fabricator thread (`mpsc` + oneshot), 13 wire-level integration tests (raw-TCP HTTP + real tonic client) + env-gated `#[ignore]` real-qwen e2e (never in CI); `docs/ux/virtual-tools-{soap,grpc}-driver.md`; THREAT-MODEL B7 extension + T-022…T-025 (summary synced).
- Rust #18/#19/#20: virtual-tools wave 1 — tui + web + api drivers. `drivers/tui.rs`: ratatui session when stdin+stdout are TTYs, byte-parity cli line fallback otherwise, `@@smah ui` display verb (schema-validated, invalid ⇒ soft `[ERROR ui …]`, sanitizing sink — T-017), 8 KiB `interface` seed. `drivers/api.rs` + `crates/smah-vtool-http`: contract-first mock API — model authors+freezes an OpenAPI 3 contract (validate-then-freeze, sha-pinned cache, zero-turn re-runs), loopback-only axum server serves it byte-for-byte at `/openapi.json` and answers everything else with bounded fabrication turns (`{"status":200-599,"body":JSON}`, 1 retry then fixed 500, 64 KiB cap, 256 KiB requests → 413, 8-entry ledger — T-018/T-020); URL disclosed on stderr, stdout empty. `drivers/web.rs` + `services/web-vtool` (Go, stdlib+vendored htmx): driver fabricates strings-only `content.json` (32 keys/64 KiB/128 KiB caps, validate-on-load sha-pinned cache — T-021), watch loop regenerates on touched `regen.request` keeping last-good content; Go service renders it loopback-only (refuses non-loopback binds incl. bare `:port`), `html/template`+`textContent` under CSP, loopback+same-origin gated `/push` (SSE) and `/regen` (POST-only) — T-019; ci-web-vtool.yml gates build/vet/test + Go `os/exec` ban. `docs/ux/virtual-tools-{tui,web,api}-driver.md`; THREAT-MODEL B7 boundary + T-017…T-021 (summary synced); 39 driver tests green, Go build/vet/test green, clippy -D warnings clean.
- Rust #16 (PR-0): virtual-tools driver-taxonomy seam — `driver:` frontmatter accepts the 9-value family (`cli|tui|web|api|soap|mcp|grpc|vfs|db`; `"emulated"` kept as a permanent `cli` alias, loader stores canonical), `interface:` extension-point key, `drivers/` registry with 8 stub drivers (refuse with `DriverUnavailable`, exit 3) for tickets #18–#25, `Scanner::with_verbs` + raw `Directive::Extension` so driver lanes add verbs without loader edits, `smah --emulate` dispatching on the tool file's driver (`--emulate-cli` kept as forced-cli compat; hooks/shims emit `--emulate`), `tools list` DRIVER column (+json), pre-added driver-family deps (axum/tonic/prost/prost-reflect/protox, tokio net+io-util) so lanes never touch the workspace manifest; `docs/ux/virtual-tools.md` family index.
- Rust: virtual-tools e2e fixes (real-model finds) — directive scanner no longer aborts a streamed answer on blank lines (a blank line stayed "deciding" and hit the directive parser post-stream: exit-3 coin flip on multi-paragraph answers); paragraph breaks now stream byte-for-byte; `split_model_spec` resolves catalog display names (`alibaba.Qwen 3.8 Flash` → `qwen3.8-flash`, case-insensitive via the inference-defaults registry) while raw ids and unknown names pass through unchanged; runbook notes in `docs/ux/virtual-tools-cli-driver.md` (model display-name vs raw id; `allow_roots` must be existing directories at load). 14 new tests (scanner unit, driver streamed, wiremock resolution); full suite green; clippy -D warnings clean.
- Rust #17: cli-virtual-tool — `smah --emulate-cli <tool>` driver: emulated CLI (argv+stdin → provider → streamed stdout, print-only; zero process-execution primitives), restricted file surface (`@@smah` directive protocol, canonicalized allow_roots containment, abort-on-violation, metadata-only audit JSONL, `--no-fs` kill switch), `smah tools list`, `smah hooks zsh|bash|envrc|shims`, V-series error/exit rows (0/2/3/4/5), SMAH_VIRTUAL_TOOLS_DIR test seam; ci-rs.yml no-shell-gate narrowed to execution primitives (termination-only exit allowed at the binary edge) with a positive-control self-test; `docs/ux/virtual-tools-cli-driver.md` driver spec; THREAT-MODEL B6 boundary + T-010…T-016 (summary synced). 151 tests green; clippy -D warnings clean.
- Rust workspace scaffold (M0): 4-crate cargo workspace (core/providers/tui/cli), ci-rs.yml, toolchain pinning.
- Rust M2: smah-providers — hand-rolled Anthropic+OpenAI clients (blocking + SSE streaming), credential seams (CLI flag → settings `api_key`/`$ENV` → `SMAH_`/bare env chain), config-key registry, §4 error/exit translation table (#26).
- Rust M2: smah-providers — alibaba provider (DashScope compatible-mode, OpenAI-compatible) on a shared `openai_compat` core; `ALIBABA_BASE_URL` env seam; `SMAH_ALIBABA_API_KEY`/`ALIBABA_API_KEY` credential chain; qwen 3.8 flash catalog entry + env-gated real-endpoint smoke (virtual-tools e2e unblock).
- Rust M1: smah-core domain — exit codes, §4 error templates (insta-snapshotted), redaction-layer logging (T-001 class fixed), Value-centric settings loader with vsn gate + $ENV indirection.
- Added `docs/PROJ-LAYOUT.md` (+ `docs/PROJ-LAYOUT.summary.md`): navigable
  project-tree map; marks generated (`_autosummary`), symlinked, and
  gitignored (`config/`, `.envrc.secret`/`.envrc.dev`) paths and the
  print-only safety invariant.
- Added `docs/PROJ-ARCH.md` (+ `.summary.md`, `arch/data-flow.md`):
  architecture overview of the plan→answer pipeline, provider seam, safety
  model, stdout/stderr contract, and exit codes. Supersedes the stale
  aspirational `docs/source/architecture.md` draft (recorded as a known
  gap), and records the stale OpenAI-only `doctor.__check_provider` bug.
- Added `docs/THREAT-MODEL.md` (+ `.summary.md`, `threats/{attack-surface,
  data-stores,supply-chain}.md`): STRIDE register (9 items, stable T-IDs)
  over the print-only CLI's boundaries. Verified in code — highest finding:
  T-001 (High) provider API key written plaintext to the DEBUG log via
  `Settings.log()` save-mode dump on every ask; also clipboard auto-copy
  without prompt (T-003) and tag-pinned CI actions (T-007).

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

