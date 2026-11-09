# Virtual tools — MCP driver (`driver: mcp`)

The MCP driver turns a virtual tool into a **Model Context Protocol server**:
the smah process IS the server. There are no child processes and no real MCP
runtime — tool discovery is fabricated by the model (one bounded turn), frozen
to disk, and then served over JSON-RPC 2.0 exactly as it was reviewed. SMAH
never executes anything the model writes; the catalog and every tool answer are
data, not code.

    ~/.config/smah/virtual-tools/crm.md
    echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | smah --emulate crm --stdio

This document is the user-facing contract for the MCP driver (ticket #23).
Family overview: [virtual-tools.md](virtual-tools.md); CLI baseline:
[virtual-tools-cli-driver.md](virtual-tools-cli-driver.md).

## 1. Declaring an MCP tool

    ---
    name: crm
    description: A tiny fabricated CRM the agent can search and update.
    driver: mcp
    interface: |
      Tools: search_contacts(q, limit) -> names; add_note(contact, text).
      Data lives in one address book of startup founders.
    ---
    You are the concierge of a small CRM. ...

- `driver: mcp` selects this lane.
- `interface` — optional. A free-form **contract artifact**: the model reads it
  when fabricating the tool catalog, so describe the tools the server should
  expose. Over 256 KiB it is ignored with a note (the persona alone drives
  fabrication).
- `constraints` still apply — they are appended to the fabrication prompt too.

## 2. Flags

Everything after `<tool>` belongs to the driver; an unknown flag or positional
is refused (exit 3):

- `--stdio` — accepted no-op; batch stdio is the default transport.
- `--no-serve` — fabricate (or reuse) the catalog, print the ready notice,
  exit 0 without serving. Use it to review before exposing anything.
- `--show-catalog` — print the frozen catalog and exit. No model turn.
- `--regen` — discard the frozen catalog and memo and re-fabricate.
- `--tcp-port <n>` — opt-in loopback TCP transport (§4). `0` = ephemeral.

## 3. Tool discovery — fabricated, then frozen

First run spends **one bounded model turn** producing a JSON array of tool
descriptions (`name`, `description`, `inputSchema`). The reply is parsed and
validated against hard caps — 64 tools, name ≤ 64 chars, description ≤ 1024
chars, schema ≤ 16 KiB, whole catalog ≤ 256 KiB — then **frozen**:

    ~/.cache/smah/virtual-tools/crm/mcp/
      mcp-tools.json     # the reviewed catalog (pretty-printed array)
      mcp.sha256         # digest of the tool file — cache key
      mcp-memo.jsonl     # memoized tools/call answers (keyed, see §5)

An invalid reply gets exactly one retry; two invalid catalogs abort with the
turn-limit error (exit 5). Editing the tool file changes the digest, so the
next run re-fabricates. `--regen` forces it; `--show-catalog` shows what a
client would see (do this before serving — see §7).

## 4. Transports

- **Batch stdio (default).** The prelude reads stdin to EOF and answers every
  request it finds, in order, on stdout — pipelined scripts need no listener
  and no long-running process:

      printf '%s\n%s\n' \
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
        '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
        | smah --emulate crm

- **Loopback TCP (opt-in).** `--tcp-port <n>` binds `127.0.0.1` only — never a
  routable interface — and prints a loud disclosure line first
  (`smah(mcp): 'crm' virtual MCP server on 127.0.0.1:4749 — loopback only …`).
  Newline-delimited JSON-RPC; one connection at a time (a second client waits);
  ctrl-c stops. A bind failure is an error, not a silent downgrade.
- **Standalone replay daemon.** `services/mcp-vtool` (Elixir, shipped beside
  the CLI) replays the same frozen artifacts on its own loopback port for
  tooling that wants a server without a model turn — it never fabricates.

All transports speak the same methods: `initialize` (negotiates
`2025-06-18` / `2024-11-05`), `ping`, `tools/list`, `tools/call`; notifications
are accepted silently.

## 5. `tools/call` — memoized fabrication

A call to a catalog tool is answered by **one fabrication turn**: the model
sees the persona, the constraints, and the call arguments, and writes the
result text. The answer is memoized under
`sha256(name + NUL + canonical-arguments-JSON)`, so the same call replays
byte-identically with **no model turn** — the second identical call, a rerun,
or the standalone daemon all hit the memo. Result text is capped at 64 KiB.

Turn accounting: a run's turn budget covers the discovery turn plus any
fabrication turns that call misses spent. A client that keeps asking new
questions keeps spending turns; the ordinary turn cap stops it (exit 5).

## 6. Files smah keeps

| path | what |
|---|---|
| `~/.config/smah/virtual-tools/*.md` | your tool definitions (config; back up) |
| `~/.cache/smah/virtual-tools/<tool>/mcp/mcp-tools.json` | frozen catalog — regenerable, reviewable |
| `~/.cache/smah/virtual-tools/<tool>/mcp/mcp-memo.jsonl` | memoized call answers — regenerable |
| `~/.local/state/smah/audit-virtual-tools.jsonl` | durable audit log (§7) |

All cache entries are safe to delete; the next run re-fabricates.

## 7. Security notes

- **The catalog is model output** (T-027). It cannot execute anything — it is
  a list of names, descriptions, and JSON-Schema — but a hostile tool file
  could fabricate a catalog that *lies* ("delete_all_users"). Read the tool
  file before running it, run `--show-catalog` (or `--no-serve`) once before
  exposing the server, and remember the caps bound how weird the catalog can
  get. The frozen file is content-addressed to the tool file (`mcp.sha256`);
  a changed tool re-fabricates instead of silently keeping a stale catalog.
- **Transports are local by construction** (T-028). stdio has no listener at
  all; TCP is loopback-only, loudly disclosed, and serves one connection at a
  time. There is no auth because there is no network surface — anything that
  can connect is already running as your user.
- **No child processes, ever.** "Serving" is this process answering requests
  inline; the repo-wide no-execution gate covers these modules.
- Every server operation lands in the audit log as metadata only — operation
  kind and decision, never statement text or result contents.
