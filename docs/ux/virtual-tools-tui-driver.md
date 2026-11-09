# Virtual tools — TUI driver (`smah --emulate`, `driver: tui`)

The tui driver is the cli driver's interactive twin: the same loop, the same
file surface, the same audit log — presented as a full-terminal UI instead of
a pipe stage. Where the cli driver answers once and streams to stdout, the
tui driver keeps a live session: the tool can refresh status lines, panes,
and menus while it works, and you keep typing to it.

This document is the user-facing contract for the tui driver (ticket #18).
Loop, exit codes, and result-line shapes follow the CLI driver contract
([virtual-tools-cli-driver.md](virtual-tools-cli-driver.md)) and
interface-spec §4/§5.

## 1. Installing a tool

Same as every driver: a single `.md` file in `~/.config/smah/virtual-tools/`
(`SMAH_VIRTUAL_TOOLS_DIR` / `XDG_CONFIG_HOME` honored), one tool per file.

    ---
    name: watch-party
    description: Live dashboard over a log directory.
    driver: tui
    interface: |
      A two-pane dashboard: a status line on top, a scrolling event list
      below. The tool refreshes the status line as it re-reads the logs.
    ---
    You are a log-watching assistant. ...

## 2. Tool file format

All cli-driver keys apply unchanged (`name`, `description`, `model`,
`constraints`, `allow_roots`). Two tui-specific notes:

- `driver: tui` selects this driver. Run it with `smah --emulate watch-party`
  — the dispatch follows the tool file's `driver:` field. (`--emulate-cli`
  forces the cli presentation and is not a tui entry point.)
- `interface` — optional here. It seeds the model's picture of the
  interactive surface (what panes exist, what they show). It is prompt text,
  not configuration, and is capped at 8 KiB; a larger value fails the tool
  file at load (`invalid tool file`, exit 3).

## 3. Running a tool

    smah --emulate watch-party [initial args...]

- **Full UI** — when both stdin and stdout are terminals, the run renders a
  ratatui session. Keys: type your message, **Enter** sends; **Up/Down**
  (or **j/k**) move menu selections; **PageUp/PageDown** scroll history;
  **Esc** steps back; **Ctrl-C** interrupts the current turn; **q** quits
  the session.
- **Line fallback** — when either side is not a terminal (pipes, CI, test
  harnesses), the run degrades to the cli driver's exact line presentation,
  byte-for-byte, plus one summary line per `ui` operation on stderr. Piping
  a tui tool is therefore safe and identical to piping a cli tool.
- Everything after `<tool>` belongs to the tool, including flag-looking
  tokens. Global smah flags go before `--emulate`.

## 4. The `@@smah ui` extension verb

On top of the cli verbs (`@@smah read/write/append`), a tui tool may emit
display directives:

    @@smah ui {"op": "status", "text": "scanning 1,204 lines"}
    @@smah ui {"op": "pane", "id": "events", "lines": ["...", "..."]}
    @@smah ui {"op": "menu", "title": "Pick a log", "items": ["a", "b"]}

`ui` payloads are **display data**: they are validated against per-op shapes
and rendered, never executed. An invalid payload becomes a soft
`[ERROR ui ...]` result line inside the session — the run continues, and the
model sees the error and can correct itself. The `@@smah` prefix remains
reserved and intercepted exactly as in the cli driver.

## 5. File surface, budget, audit

Identical to the cli driver: allowlisted `allow_roots` read/write/append,
path-escape refusal (exit 3), 128 KiB per write, 1 MiB per read, 1 MiB total
per run, 8 model turns per run, every attempt in
`~/.local/state/smah/audit-virtual-tools.jsonl` (metadata only), roots
disclosed on stderr at startup (`file surface: ... (disable: --no-fs)`).
Rendered content is written through a sanitizing sink — fabricated text can
never alter the UI chrome or emit terminal control sequences as UI (it is
shown as text; THREAT-MODEL T-017).

## 6. Exit codes

Same table as every smah command (interface-spec §4): `0` success · `2` no
such tool · `3` invalid tool file, or a file-surface violation aborted the
run · `4` auth · `5` provider/network failure or turn-limit overflow. A
non-TTY fallback run exits exactly like its cli equivalent.

## 7. Files smah keeps

| path | what |
|---|---|
| `~/.config/smah/virtual-tools/*.md` | your tool definitions (back these up) |
| `~/.cache/smah/virtual-tools/<tool>/` | last run's transcript — regenerable scratch, safe to delete |
| `~/.local/state/smah/audit-virtual-tools.jsonl` | durable audit log of every file operation |

## 8. Security notes

The tui driver adds one surface to the cli model: the `ui` verb. It is
bound-checked display data — an operator for panes and status lines, not an
escape hatch — and its rendering is sanitized, so a hostile tool file can
cosmetically confuse you (a pane that says `[OK]`) but cannot execute
anything, reach outside `allow_roots`, or alter the frame chrome. The usual
rules apply unchanged: only install tools you trust, read the file, check
`allow_roots`, prefer a first run with `--no-fs`, watch the audit log.
