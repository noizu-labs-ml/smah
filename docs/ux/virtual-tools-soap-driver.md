# Virtual tools — SOAP driver (`smah --emulate`, `driver: soap`)

The soap driver turns a tool file into a **mock SOAP service**: you author the
WSDL (inline in the tool file or as a path), smah validates and freezes it,
and a loopback-only server then answers posted SOAP envelopes by *emulating*
the declared operations — each call is a bounded model turn whose output is
re-validated against the frozen contract before it reaches your client.

    smah --emulate widgets-soap &              # driver: freeze + serve
    curl -s "http://127.0.0.1:<port>/?wsdl"    # the frozen WSDL, byte-for-byte
    curl -s -X POST http://127.0.0.1:<port>/ -H 'Content-Type: text/xml' \
      -H 'SOAPAction: "GetWidget"' --data-binary @envelope.xml

This document is the user-facing contract for the soap driver (ticket #22).
Exit codes and error text follow interface-spec §4/§5.

## 1. Installing a tool

Same as every driver: one `.md` file in `~/.config/smah/virtual-tools/`.

    ---
    name: widgets-soap
    description: A mock SOAP service for a widget warehouse.
    driver: soap
    interface: |
      <wsdl:definitions xmlns:wsdl="http://schemas.xmlsoap.org/wsdl/" ...>
        <!-- the WSDL you want served; message/portType/binding/service -->
      </wsdl:definitions>
    ---
    You emulate a pragmatic internal service. Invent consistent data. ...

**`interface` is required** for this driver — it carries the WSDL inline, or
`interface: $WIDGETS_WSDL` / a path to a `.wsdl`/`.xml` file. There is no
model-generated contract: a file without a usable WSDL refuses to run
(`invalid tool file`, exit 3). The WSDL is structurally validated at load —
namespaces, messages with element/literal parts, portType operations wired
to a binding and service — and only then frozen.

## 2. Running a tool

    smah --emulate widgets-soap [--port N] [--no-serve] [--max-serve-seconds N]

- **Freeze, then serve.** The WSDL is validated against the declared
  contract and frozen into the cache (§3); an unchanged tool file reuses the
  cache with **zero model turns**.
- **Serving.** The server binds `127.0.0.1` on an ephemeral port by default
  (`--port N` pins it) and discloses the address on **stderr**:
  `serving http://127.0.0.1:<port>/ (ctrl-c to stop) — VIRTUAL soap service: all ...`.
  stdout stays empty — safe to redirect anywhere. A non-loopback bind is
  refused, not corrected.
- `GET /?wsdl` returns the frozen WSDL **byte-for-byte**.
- Every posted envelope is one fabrication turn: the model sees the
  operation's declared parts, a compact ledger of prior exchanges, and a
  snippet of the request, and must answer with **only** the response
  element the WSDL declares for that operation. The reply is wrapped in a
  proper `soap:Envelope` (1.1 or 1.2 per the request's namespace), and
  checked against the frozen message parts — a reply naming elements the
  WSDL never declared is retried once, then the run aborts with a bounded
  loop-overflow error (a bad model turn never leaks into your client as
  unvalidated XML).
- Unparseable or oversized requests, unknown operations, and envelopes
  containing a `DOCTYPE` (DTD/XXE attempts) are answered with a
  `soap:Fault` — see §7.
- `--no-serve` — validate and freeze the WSDL, print its path, exit
  (useful to inspect the contract before serving).
- `--max-serve-seconds N` — stop the server after N seconds (for scripts;
  Ctrl-C also stops it, ending the run with exit 0).

## 3. Contract cache

| path | what |
|---|---|
| `~/.cache/smah/virtual-tools/<tool>/soap/wsdl.xml` | the frozen WSDL — regenerable, safe to delete |
| `~/.cache/smah/virtual-tools/<tool>/soap/contract.sha256` | integrity marker over the frozen bytes |
| `~/.cache/smah/virtual-tools/<tool>/soap/tool.sha256` | staleness marker pinning the cache to the tool file's sha |

Cache lifecycle: **validated on load** every run. An unchanged tool file +
marker-valid WSDL = cache hit (no model turn). A cache whose markers match
but whose body no longer validates is rejected loudly and re-frozen from the
tool file; a stale `tool.sha256` (tool edited) re-freezes silently — the
WSDL is derived state; regenerating is correct, not an error worth a
warning.

## 4. Caps

| cap | value |
|---|---|
| request envelope accepted | 1 MiB (larger → fault, audited) |
| fabricated response body | 128 KiB (over → retry, then abort) |
| fabrication attempts per request | 2 |
| prior-exchange ledger entries | 6 pairs (snippets ≤ 2 KiB each) |
| request snippet shown to the model | 16 KiB |
| model turns per run (all uses) | 8 |
| serve deadline | `--max-serve-seconds N`, else until Ctrl-C |

## 5. Files smah keeps

| path | what |
|---|---|
| `~/.cache/smah/virtual-tools/<tool>/soap/` | the frozen WSDL + markers (above) — regenerable |
| `~/.local/state/smah/audit-virtual-tools.jsonl` | durable audit log (`soap.contract.load` cache hits and rejections, `soap.contract.freeze`, `soap.request` allowed/denied) |

Audit records are **metadata only**: operation name, request byte count,
decision, reason — never envelope or response contents.

## 6. Exit codes

Same table as every smah command (interface-spec §4): `0` success (including
a server stopped with Ctrl-C) · `2` no such tool · `3` invalid tool file
(missing/unparseable `interface`, WSDL that fails validation) · `4` auth ·
`5` provider/network failure, turn budget exhausted, or the fabrication
drift loop overflowing.

## 7. Security notes

The server is **loopback-only by construction** — every bind is `127.0.0.1`
(T-025), disclosed on stderr, and `--no-serve` exists for runs that should
not listen at all. Requests are parsed by a DTD- and entity-denying scanner:
a `DOCTYPE` never gets past the front door, so billion-laughs expansion and
XXE file reads have nothing to bite (T-022) — they earn a `soap:Fault` and
an audit line. The frozen WSDL is served byte-for-byte; fabricated
responses are re-validated against it before wrapping, with hard size caps,
so a runaway model turn cannot stream unbounded or off-contract XML to your
client; a poisoned cache file is caught by validate-on-load (T-024). The
mock's data is invented; never point a client that makes real decisions at
it. Only install tools you trust; watch the audit log.
