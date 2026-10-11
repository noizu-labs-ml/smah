CHANGE LOG
==============


## Changes

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
  stage); existing OpenAI entries are unchanged. Note: only the OpenAI provider
  is wired up in this version — selecting a Claude model exits 4 with guidance
  until provider support lands.
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

