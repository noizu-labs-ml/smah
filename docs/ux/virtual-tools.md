# Virtual tools — driver family

A virtual tool is a markdown file (`name`, frontmatter, persona body) that
behaves like a subcommand: `smah --emulate <tool> [args...]`. The **cli
driver** (the only one implemented today) runs the tool by prompting the
model with a file surface and directive protocol — see
[virtual-tools-cli-driver.md](virtual-tools-cli-driver.md) for the full
cli-driver contract.

This page is the family index: the driver taxonomy, how dispatch works,
and where each driver's status lives.

## Driver taxonomy

A tool file's `driver:` frontmatter key (optional, default `cli`) names the
driver that executes it. The full set:

| value | executes a tool as | status |
|---|---|---|
| `cli` | a prompt-driven CLI tool over the file surface + directive protocol | **shipped** |
| `tui` | an interactive terminal UI | not yet (refuses, exit 3) |
| `web` | a local web app / HTTP endpoint | not yet (refuses, exit 3) |
| `api` | a client for an external REST API | not yet (refuses, exit 3) |
| `soap` | a SOAP/WSDL client | not yet (refuses, exit 3) |
| `mcp` | a Model Context Protocol client | not yet (refuses, exit 3) |
| `grpc` | a gRPC client | not yet (refuses, exit 3) |
| `vfs` | a virtual filesystem mounted for the model | not yet (refuses, exit 3) |
| `db` | a database query surface | not yet (refuses, exit 3) |

`"emulated"` — the original key value — is a **permanent alias of `cli`**;
existing tool files keep working and the loader stores `cli`.

An unknown driver makes the tool file invalid (typos must not silently
change behavior). A known, not-yet-shipped driver **loads fine** — it
appears in `smah tools list` with its driver column — and refuses to run:

    smah: virtual-tool driver 'tui' is not available in this build (not
    implemented yet; the tui virtual tool ships in a later release). See
    docs/ux/virtual-tools.md for the driver family.

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

1. Replace the stub `crates/smah-core/src/virtual_tools/drivers/<name>.rs`
   (`run` refuses with `DriverUnavailable` today; keep the signature).
2. Extend the directive verb set with `Scanner::with_verbs(&[...])` —
   verbs outside the base set arrive as raw
   `Directive::Extension { verb, payload }` for the driver to interpret.
3. Parse whatever `interface:` means for your driver in your own module.

Driver-family crates (axum, tonic, prost, prost-reflect, protox) are
already in the workspace dependency table — lanes must not edit the
workspace `Cargo.toml`.
