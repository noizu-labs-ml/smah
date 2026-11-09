# Virtual tools — driver family

A virtual tool is a markdown file (`name`, frontmatter, persona body) that
behaves like a subcommand: `smah --emulate <tool> [args...]`. Each tool
file's `driver:` key picks one of the nine drivers below; all nine ship in
this build. Wherever a driver listens on the network it binds loopback only
and says so loudly on stderr (THREAT-MODEL boundary B7).

This page is the family index: the driver taxonomy, how dispatch works,
and where each driver's contract lives (one `virtual-tools-<driver>-driver`
doc per driver).

## Driver taxonomy

A tool file's `driver:` frontmatter key (optional, default `cli`) names the
driver that executes it. The full set — all **shipped**:

| value | executes a tool as | contract |
|---|---|---|
| `cli` | a prompt-driven CLI tool over the file surface + directive protocol | [cli](virtual-tools-cli-driver.md) |
| `tui` | an interactive terminal UI (ratatui session on TTYs; byte-parity cli line fallback; `@@smah ui` display verb) | [tui](virtual-tools-tui-driver.md) |
| `web` | a local web page (model writes strings-only `content.json`; loopback-only Go companion renders it; refresh re-engages the model) | [web](virtual-tools-web-driver.md) |
| `api` | a mock REST API (frozen OpenAPI 3 contract served byte-for-byte at `/openapi.json`; other requests answered by bounded fabrication) | [api](virtual-tools-api-driver.md) |
| `soap` | a mock SOAP service (frozen WSDL served at `?wsdl`; DTD/entity-denying envelope handling) | [soap](virtual-tools-soap-driver.md) |
| `mcp` | an MCP server (the smah process itself: frozen fabricated tool catalog over JSON-RPC 2.0 stdio, opt-in loopback TCP; standalone Elixir replay service in `services/mcp-vtool`) | [mcp](virtual-tools-mcp-driver.md) |
| `grpc` | a mock gRPC server (proto-first: frozen `.proto` parsed into a descriptor pool, unary-only tonic dispatch) | [grpc](virtual-tools-grpc-driver.md) |
| `vfs` | a virtual filesystem (fabricated tree materialized under the tool's cache dir; code-owned fs tools; canonicalized containment) | [vfs](virtual-tools-vfs-driver.md) |
| `db` | a PostgreSQL-compatible query surface (`smah-virtual-postgres`: PG-wire subset over unix socket / loopback TCP; read-only fabricated store) | [db](virtual-tools-db-driver.md) |

`"emulated"` — the original key value — is a **permanent alias of `cli`**;
existing tool files keep working and the loader stores `cli`.

An unknown driver makes the tool file invalid (typos must not silently
change behavior). A driver absent from a build (e.g. a feature-gated
build) **loads fine** — it appears in `smah tools list` with its driver
column — and refuses to run:

    smah: virtual-tool driver '<driver>' is not available in this build.
    See docs/ux/virtual-tools.md for the driver family.

Exit code 3 (`Config` class).

## Dispatch

- `smah --emulate <tool>` — dispatch follows the tool file's `driver:`.
- `smah --emulate-cli <tool>` — the forced-cli compat form: always runs the
  cli driver regardless of the file's `driver:`. Kept forever.

Shell hooks and shims emit the `--emulate` form, so a tool's driver is
decided by its file, not by how it was invoked.

## The `interface:` key

Non-cli drivers need more than a persona body (an OpenAPI doc, a proto
file, a WSDL, a UI layout). The optional `interface:` frontmatter key
(string) is the extension point: each driver lane defines what it points
at and how it is resolved. The cli driver ignores it. Putting it on a cli
tool is allowed but inert.

## Adding a driver (for implementers)

The seam is designed so a driver lane needs **no loader or dispatch
edits**:

1. Implement `crates/smah-core/src/virtual_tools/drivers/<name>.rs`
   (the stub it replaces refused with `DriverUnavailable`; keep the
   `run` signature).
2. Extend the directive verb set with `Scanner::with_verbs(&[...])` —
   verbs outside the base set arrive as raw
   `Directive::Extension { verb, payload }` for the driver to interpret.
3. Parse whatever `interface:` means for your driver in your own module.

Driver-family crates (axum, tonic, prost, prost-reflect, protox) are
already in the workspace dependency table — lanes must not edit the
workspace `Cargo.toml`.
