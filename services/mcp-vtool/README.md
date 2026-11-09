# mcp-vtool — frozen-artifact replay service for smah virtual MCP tools

Standalone Elixir service (ticket #23) that replays the MCP artifacts the
Rust lane fabricates and freezes under the smah cache dir. It serves the
same JSON-RPC 2.0 surface (`initialize`, `ping`, `tools/list`,
`tools/call`) as the Rust lane's `mcp` driver, so an MCP client can talk
to a virtual tool without keeping the `smah` process alive.

**It never fabricates.** Discovery reads only what a `smah` run froze:

```
<cache>/smah/virtual-tools/<tool>/mcp/mcp-tools.json   tool catalog
<cache>/smah/virtual-tools/<tool>/mcp/mcp-memo.jsonl   call memo
```

- `tools/list` replays the frozen catalog.
- `tools/call` replays the memo entry for
  `sha256(tool name <> 0x00 <> canonical arguments JSON)` — byte-identical
  to the Rust lane's key (object key order preserved by the in-tree JSON
  codec).
- A call with no memo entry answers `isError: true` with
  *"not fabricated yet — run `smah --emulate <tool> --no-serve`"*.

Zero hex dependencies; no OS processes are ever spawned (CI greps
`System.cmd`/`Port.open` out of `lib/`).

## Run

```sh
cd services/mcp-vtool
mix run --no-halt                      # loopback TCP on MCPVTOOL_TCP_PORT (4749)
MCPVTOOL_STDIO=1 mix run --no-halt     # newline-delimited JSON-RPC on stdio
```

Environment:

| var                 | meaning                                             | default                          |
|---------------------|-----------------------------------------------------|----------------------------------|
| `MCPVTOOL_CACHE_DIR`| virtual-tools cache root                            | `$XDG_CACHE_HOME/smah/virtual-tools` |
| `MCPVTOOL_TOOL`     | which discovered tool to serve                      | first sorted tool                |
| `MCPVTOOL_TCP_PORT` | loopback listen port (binds 127.0.0.1 only)         | `4749`                           |
| `MCPVTOOL_STDIO`    | `1` adds the stdio transport to the supervision tree | off                              |

TCP is loopback-only and serves connections **sequentially** — one
connection at a time, like the Rust lane's `--tcp-port` mode. If the
default port is busy the listener falls back to an ephemeral port and
logs loudly; `McpVtool.Server.Tcp.bound_port/0` reports what bound.

One tool per instance: run several instances on different ports to serve
multiple virtual tools.

## Test

```sh
mix compile --warnings-as-errors && mix test
```
