# Virtual tools — CLI driver (`smah --emulate-cli`)

Virtual tools are small, user-authored command-line tools defined as a single
markdown file. The CLI driver turns `smah.<tool-name>` into a normal pipeline
stage: arguments and piped stdin go to the model, the model's answer streams to
stdout, and exit codes behave the way a shell expects. SMAH never executes
anything the model writes — the driver simulates the tool.

    ~/.config/smah/virtual-tools/log-analyzer.md
    cat my.log | smah.log-analyzer --info | bat

This document is the user-facing contract for the CLI driver (ticket #17).
Error text and exit codes follow interface-spec §4/§5; the virtual-tool family
PRD is tracked separately (#16).

## 1. Installing a tool

Drop a `.md` file into `~/.config/smah/virtual-tools/` (override for testing:
`SMAH_VIRTUAL_TOOLS_DIR`; `XDG_CONFIG_HOME` is honored). One file per tool;
the filename stem is the tool name.

    ~/.config/smah/virtual-tools/
      log-analyzer.md
      commit-msg.md

List what is installed:

    smah tools list [--json]

Invalid files are listed as `invalid` with the reason (also on stderr), and
are never run.

## 2. Tool file format (agent.md style)

    ---
    name: log-analyzer
    description: Summarizes and filters application logs from stdin.
    model: anthropic.claude-opus-5-5        # optional; default model otherwise
    driver: cli                            # optional; see the driver family below
    constraints:                            # optional hard rules for this tool
      - Never invent log lines that are not in the input.
    allow_roots:                            # optional; file-surface roots (see §5)
      - ~/notes
      - $PROJECT_LOGS
    ---
    You are a meticulous log analyst. ...

- `name` — required. Lowercase letters, digits, `-`, `_`; must match the
  filename stem; may not reuse a smah subcommand name (`ask`, `chat`,
  `history`, `resume`, `config`, `models`, `doctor`, `hooks`, `tools`,
  `emulate`, `help`, `version`).
- `description` — required, non-empty. Shown by `smah tools list`.
- `model` — optional. `<provider>.<model-id>` or a bare model id (default
  provider). A `--model` flag on the invocation wins over this; settings
  default otherwise.
  - **Runbook note (verified):** the model part accepts either a raw
    provider id (`alibaba.qwen3.8-flash`) or the catalog display name
    (`alibaba.Qwen 3.8 Flash`) — display names resolve case-insensitively
    through the embedded inference-defaults registry before the request is
    sent. A name the registry does not know is passed through unchanged, so
    the provider's own not-found error (exit 5) surfaces as before.
- `driver` — optional. One of the virtual-tools driver family: `cli`,
  `tui`, `web`, `api`, `soap`, `mcp`, `grpc`, `vfs`, `db` (default `cli`).
  `"emulated"` is a permanent alias of `cli`. Tools with a driver other
  than `cli` load fine but refuse to run until that driver ships (see
  [virtual-tools.md](virtual-tools.md) for the family and status).
- `interface` — optional string. Reserved for non-cli drivers (a pointer to
  the tool's interface description). Ignored by the `cli` driver; see
  [virtual-tools.md](virtual-tools.md).
- `constraints` — optional list of hard rules appended to the tool's prompt.
- `allow_roots` — optional list of directories the tool may read or write
  (see §5). `~/...` and `$ENVVAR` forms are expanded. No list = no file
  surface.
  - **Runbook note (verified):** every `allow_roots` entry must be an
    **existing directory at load time** — an entry pointing at a
    nonexistent path fails the invocation at load-for-invoke with exit 3
    (`... is not an existing directory`), not a lazy error at first use.

**Unknown keys make the file invalid** (typos must not silently change
behavior). The markdown body is the tool's persona and instructions — that is
your prompt to the model. SMAH appends its own operating rules (§5); you
cannot override those from the file.

## 3. Running a tool

    smah --emulate-cli <tool> [args...]          # direct form
    smah.log-analyzer --info                     # via a shell hook (§4)

- Everything after `<tool>` belongs to the tool, including things that look
  like flags: `smah --emulate-cli edit --version` passes `--version` through.
  Global smah flags (`-c`, `--model`, `-v`) go **before** `--emulate-cli`.
- **stdin**: piped input is read fully and given to the tool as its input
  data. At a terminal, stdin is not read. Inputs above 256 KiB are truncated
  — smah says so on stderr (`input truncated: sent ... of ... bytes
  (--max-input-chars to change)`) and the tool is told its input is partial.
- **stdout carries only the tool's answer** — safe to pipe into `bat`, `jq`,
  a file. Progress and notices go to stderr.
- Extra flags: `--no-fs` (disable the file surface for this run),
  `--max-input-chars N`.

## 4. Shell hooks

Register every installed tool as a shell function:

    eval "$(smah hooks zsh)"        # or: smah hooks bash

re-run after installing tools. For direnv (which cannot export functions),
use shim executables:

    smah hooks shims                # writes ~/.local/state/smah/bin/smah.<name>
    # .envrc:  PATH_add ~/.local/state/smah/bin

`smah hooks zsh|bash|envrc` print only the snippet to stdout; `smah hooks
shims` writes/refreshes/removes shims and reports on stderr.

## 5. File surface (read/write/append, allowlisted)

A tool with `allow_roots` may ask the driver to read, create, or append to
files **inside those roots only**. The tool cannot run code; it emits a
directive line, and the driver performs the operation with plain filesystem
calls:

    @@smah read {"path": "2026/october/incidents.md"}
    @@smah write {"path": "summary.md", "content": "# Summary\n\n..."}
    @@smah append {"path": "summary.md", "content": "more text\n"}

Semantics:

- Paths resolve inside the declared roots: relative paths land in the
  **first** declared root; absolute paths must be inside one of the roots.
  `..`, absolute paths outside the roots, and symlinks that escape a root
  are refused and the run aborts (exit 3) with the reason on stderr. Writes
  refuse a symlink as the final path component.
- Every attempt is recorded in `~/.local/state/smah/audit-virtual-tools.jsonl`
  (time, tool, operation, path, bytes, allowed/denied). File contents are
  never logged.
- Size limits: 128 KiB per write, 1 MiB per read, 1 MiB total written per
  run. Over-limit writes are refused back to the tool (it can retry smaller);
  the aggregate limit aborts the run.
- At most 8 model turns per run; a tool that keeps asking for file operations
  is stopped (exit 5).
- Each executed operation prints a notice on stderr. Every run of a
  file-enabled tool prints its roots:
  `file surface: /Users/you/notes (disable: --no-fs)`.

The `@@smah` prefix is reserved. A tool should never emit it in normal
output; if it does, the driver intercepts the line (it never reaches your
pipe) and treats it as an action attempt.

## 6. Exit codes

Same table as every smah command (interface-spec §4): `0` success · `2` no
such tool · `3` invalid tool file, or a file-surface violation aborted the
run · `4` auth (missing/rejected key, no enabled models) · `5` provider or
network failure, or the tool exceeded its turn limit. Exit codes survive
piping: a failed tool in a pipeline fails the pipeline stage.

## 7. Files smah keeps

| path | what |
|---|---|
| `~/.config/smah/virtual-tools/*.md` | your tool definitions (back these up; they are config) |
| `~/.cache/smah/virtual-tools/<tool>/` | last run's transcript — regenerable scratch, safe to delete |
| `~/.local/state/smah/audit-virtual-tools.jsonl` | durable audit log of every file operation |
| `~/.local/state/smah/bin/smah.<name>` | hook shims (direnv mode) |

## 8. Security notes

Tool files are like dotfiles you paste from the internet: **only install
tools you trust.** A tool's prompt can ask for anything and its `allow_roots`
decide how far file operations reach. Before running a new tool: read the
whole file, check `allow_roots` against what the tool claims to do, prefer
running it once with `--no-fs`, and watch the stderr notices and the audit
log. smah never executes commands; the worst case is confined to the
declared roots plus what you pipe in (which goes to the model provider).
