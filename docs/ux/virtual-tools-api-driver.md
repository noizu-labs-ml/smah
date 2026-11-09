# Virtual tools — API driver (`smah --emulate`, `driver: api`)

The api driver turns a tool file into a **mock HTTP API**: the model first
writes an OpenAPI contract for the service, smah freezes it, and a
loopback-only server then answers every request by *emulating* the endpoint —
each call is a bounded model turn that must stay inside the frozen contract
and a rolling ledger of prior responses, so the mock behaves like one
consistent service instead of unrelated improvisations.

    smah --emulate widgets-api &            # driver: contract + serve
    curl -s http://127.0.0.1:<port>/openapi.json | jq .
    curl -s -X POST http://127.0.0.1:<port>/widgets -d '{"name":"gizmo"}'

This document is the user-facing contract for the api driver (ticket #20).
Exit codes and error text follow interface-spec §4/§5.

## 1. Installing a tool

Same as every driver: one `.md` file in `~/.config/smah/virtual-tools/`.

    ---
    name: widgets-api
    description: A mock REST API for a widget warehouse.
    driver: api
    interface: |
      A JSON REST API for a widget warehouse: list/create widgets,
      fetch one by id, adjust stock. Standard REST conventions.
    ---
    You emulate a pragmatic internal service. Invent consistent data. ...

**`interface` is required** for this driver — it is the service description
the contract is authored against. A file without it refuses to run
(`invalid tool file`, exit 3).

## 2. Running a tool

    smah --emulate widgets-api [contract request...] [--port N] [--no-serve]

- **Contract first.** The driver asks the model for an OpenAPI 3 spec of the
  service (up to 3 attempts), validates it (parseable YAML/JSON, `openapi`
  version present, at least one path), freezes it, and starts serving.
- **Serving.** The server binds `127.0.0.1` on an ephemeral port by default
  (`--port N` pins it) and discloses the URL on **stderr**:
  `serving http://127.0.0.1:<port> (ctrl-c to stop)`. stdout stays empty —
  safe to redirect anywhere.
- `GET /openapi.json` returns the frozen contract **byte-for-byte**.
- Every other request is one fabrication turn: the model sees the frozen
  contract summary, the method+path+query, the request body, and a compact
  ledger of prior responses, and must answer `{"status": <200-599>,
  "body": <any JSON>}`. Responses outside that shape or over 64 KiB are
  retried once, then refused (`500` with a fixed error body — a bad model
  turn never leaks into your client as garbage).
- `--no-serve` — generate and freeze the contract, print its path, exit
  (useful to inspect the contract before serving). A later run with an
  unchanged tool file reuses the cache with **zero model turns**.
- Ctrl-C stops the server; the run ends with exit 0.

## 3. Contract cache

| path | what |
|---|---|
| `~/.cache/smah/virtual-tools/<tool>/api/openapi.json` | the frozen contract — regenerable, safe to delete |
| `~/.cache/smah/virtual-tools/<tool>/api/openapi.json.sha256` | staleness marker pinning the cache to the tool file's sha |

Cache lifecycle: the cache is **validated on load** every run. An
unchanged tool file + valid contract = cache hit (no model turn). A cache
whose marker matches but whose body no longer validates is rejected loudly
(`contract cache rejected (...)`) and regenerated. A stale marker (tool file
edited) regenerates silently — the contract is derived state; regenerating
is the correct behavior, not an error worth a warning.

## 4. Caps

| cap | value |
|---|---|
| contract generation attempts | 3 (then exit 5) |
| fabrication attempts per request | 2 |
| fabricated response body | 64 KiB |
| request body accepted | 256 KiB (larger → `413`) |
| prior-state ledger entries | 8 (digests ≤ 512 B each) |
| model turns per run (all uses) | 8 |

## 5. Files smah keeps

| path | what |
|---|---|
| `~/.cache/smah/virtual-tools/<tool>/api/` | the frozen contract (above) — regenerable |
| `~/.local/state/smah/audit-virtual-tools.jsonl` | durable audit log (`contract.generate`, `api.serve`, `api.stop`, denials) |

## 6. Exit codes

Same table as every smah command (interface-spec §4): `0` success (including
a server stopped with Ctrl-C) · `2` no such tool · `3` invalid tool file, or
the contract failed validation after 3 attempts · `4` auth · `5`
provider/network failure or turn budget exhausted.

## 7. Security notes

The server is **loopback-only by construction** — every bind is
`127.0.0.1`, disclosed on stderr, and `--no-serve` exists for runs that
should not listen at all. The contract is frozen and served byte-for-byte;
the model can widen it only by regenerating it (a full driver run), never
mid-serving. Fabricated responses are schema-checked JSON with hard caps —
a runaway model turn cannot stream unbounded data to a client, and a
poisoned cache file is caught by validation on the next run. Emulated data
is invented; never point a client that makes real decisions at a mock. Only
install tools you trust; watch the audit log.
