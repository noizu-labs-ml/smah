# Virtual tools — gRPC driver (`smah --emulate`, `driver: grpc`)

The grpc driver turns a tool file into a **mock gRPC server**: you author the
`.proto` (inline in the tool file or as a path), smah compiles it at load
time (pure-Rust `protox` — no `protoc` binary), freezes it, and a
loopback-only HTTP/2 server then answers unary calls by *emulating* the
declared methods — each call is a bounded model turn whose protojson output
is parsed back through the typed descriptor before it reaches your client.

    smah --emulate widgets-grpc &                # driver: freeze + serve
    grpcurl -plaintext -d '{"name": "gizmo"}' \
      127.0.0.1:<port> widgets.v1.Widgets/GetWidget

This document is the user-facing contract for the grpc driver (ticket #21).
Exit codes and error text follow interface-spec §4/§5.

## 1. Installing a tool

Same as every driver: one `.md` file in `~/.config/smah/virtual-tools/`.

    ---
    name: widgets-grpc
    description: A mock gRPC service for a widget warehouse.
    driver: grpc
    interface: |
      syntax = "proto3";
      package widgets.v1;
      message GetWidgetRequest { string id = 1; }
      message Widget { string id = 1; string name = 2; }
      service Widgets {
        rpc GetWidget(GetWidgetRequest) returns (Widget);
      }
    ---
    You emulate a pragmatic internal service. Invent consistent data. ...

**`interface` is required** for this driver — it carries the `.proto`
inline, or `interface: $WIDGETS_PROTO` / a path to a `.proto` file. There is
no model-generated contract: a file without a usable proto refuses to run
(`invalid tool file`, exit 3). The proto is compiled and validated at load —
it must parse, declare **exactly one service**, at most 64 methods, and at
least one unary method — and only then frozen. **v1 is unary-only**:
streaming methods may be declared but every streaming call is refused at
call time with `UNIMPLEMENTED` (grpc-status 12).

## 2. Running a tool

    smah --emulate widgets-grpc [--port N] [--no-serve] [--max-serve-seconds N]

- **Freeze, then serve.** The proto is compiled and validated against the
  declared contract and frozen into the cache (§3); an unchanged tool file
  reuses the cache with **zero model turns**.
- **Serving.** The server binds `127.0.0.1` on an ephemeral port by default
  (`--port N` pins it) and discloses the address on **stderr**:
  `serving 127.0.0.1 (grpc, http/2 prior knowledge) on 127.0.0.1:<port>
  (ctrl-c to stop)`. stdout stays empty — safe to redirect anywhere. A
  non-loopback bind is refused, not corrected. The server speaks plain h2
  prior-knowledge gRPC — no TLS, no upgrade path, loopback only.
- Every unary call is one fabrication turn: the model sees the method's
  request message as **protojson**, a compact ledger of prior exchanges,
  and must answer with **only** the protojson response message the proto
  declares for that method. The reply is parsed back through the typed
  output descriptor — unknown fields and type mismatches are refused. A
  reply that keeps failing validation is retried once, then the run aborts
  with a bounded loop-overflow error (a bad model turn never reaches your
  client as untyped bytes).
- Streaming calls, unknown methods, and unknown services are refused with
  grpc-status `12` (`UNIMPLEMENTED`) **before any fabrication** — the
  refusal names the scope (unary-only v1) or the declared service.
- `--no-serve` — validate and freeze the proto, print its path, exit
  (useful to inspect the contract before serving).
- `--max-serve-seconds N` — stop the server after N seconds (for scripts;
  Ctrl-C also stops it, ending the run with exit 0).

## 3. Contract cache

| path | what |
|---|---|
| `~/.cache/smah/virtual-tools/<tool>/grpc/grpc.proto` | the frozen proto — regenerable, safe to delete |
| `~/.cache/smah/virtual-tools/<tool>/grpc/contract.sha256` | integrity marker over the frozen bytes |
| `~/.cache/smah/virtual-tools/<tool>/grpc/tool.sha256` | staleness marker pinning the cache to the tool file's sha |

Cache lifecycle: **validated on load** every run — the frozen proto is
re-compiled through protox on every start. An unchanged tool file +
marker-valid proto = cache hit (no model turn). A cache whose markers match
but whose body no longer validates is rejected loudly and re-frozen from the
tool file; a stale `tool.sha256` (tool edited) re-freezes silently — the
proto is derived state; regenerating is correct, not an error worth a
warning.

## 4. Caps

| cap | value |
|---|---|
| `interface:` proto (inline or file) | 512 KiB |
| methods per service | 64 |
| request message accepted | 1 MiB (larger → `RESOURCE_EXHAUSTED`-class refusal) |
| fabricated response message | 128 KiB (over → retry, then abort) |
| fabrication attempts per call | 2 |
| prior-exchange ledger entries | 6 pairs (snippets ≤ 2 KiB each) |
| request snippet shown to the model | 16 KiB |
| model turns per run (all uses) | 8 |
| serve deadline | `--max-serve-seconds N`, else until Ctrl-C |

## 5. Files smah keeps

| path | what |
|---|---|
| `~/.cache/smah/virtual-tools/<tool>/grpc/` | the frozen proto + markers (above) — regenerable |
| `~/.local/state/smah/audit-virtual-tools.jsonl` | durable audit log (`grpc.contract.load` cache hits and rejections, `grpc.contract.freeze`, `grpc.call` allowed/denied) |

Audit records are **metadata only**: method path, byte counts, decision,
reason — never request or response contents.

## 6. Exit codes

Same table as every smah command (interface-spec §4): `0` success (including
a server stopped with Ctrl-C) · `2` no such tool · `3` invalid tool file
(missing/unreadable `interface`, proto that fails compilation or
validation) · `4` auth · `5` provider/network failure, turn budget
exhausted, or the fabrication drift loop overflowing.

## 7. Security notes

The server is **loopback-only by construction** — every bind is `127.0.0.1`
(T-025), disclosed on stderr, and `--no-serve` exists for runs that should
not listen at all. The runtime proto parse is bounded (512 KiB contract cap,
64-method cap) so a hostile `interface:` cannot exhaust the load budget
(T-023). Fabricated responses are deserialized against the declared output
descriptor with hard size caps before they are encoded, so a runaway model
turn cannot stream unbounded or off-contract messages to your client
(T-024); a poisoned cache file is caught by re-compilation on the next load.
Dispatch is unary-only — streaming shapes are refused before fabrication.
The mock's data is invented; never point a client that makes real decisions
at it. Only install tools you trust; watch the audit log.
