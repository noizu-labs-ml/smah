# Virtual tools — VFS driver (`driver: vfs`)

The VFS driver gives a virtual tool a **fabricated file tree**: the model
invents a small directory tree for the tool's persona, smah writes it as real
files under the cache dir, and the tree is served through a code-owned set of
MCP-style filesystem tools. There is no FUSE and no kernel mount — the files
are ordinary files in a sandboxed directory, and the only ways in are the fs
tools.

    ~/.config/smah/virtual-tools/fieldnotes.md
    echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fs_list","arguments":{"path":""}}}' \
      | smah --emulate fieldnotes --stdio

This document is the user-facing contract for the VFS driver (ticket #25).
Family overview: [virtual-tools.md](virtual-tools.md); the MCP wire this lane
reuses: [virtual-tools-mcp-driver.md](virtual-tools-mcp-driver.md).

## 1. Declaring a VFS tool

    ---
    name: fieldnotes
    description: A naturalist's field notebook from a fabricated archipelago.
    driver: vfs
    interface: |
      Root: expedition notes from three islands; observations/*.md per island,
      species_index.md, README.md at the root.
    ---
    You are a meticulous field naturalist. ...

- `driver: vfs` selects this lane.
- `interface` — the **contract artifact**: the tree the model should fabricate
  (directories, files, what they contain).
- `constraints` are appended to the fabrication prompt as for every lane.

## 2. Flags

Everything after `<tool>` belongs to the driver; an unknown flag or positional
is refused (exit 3):

- `--stdio` — accepted no-op; batch stdio is the default transport.
- `--no-serve` — fabricate (or reuse) the tree, print the ready notice, exit 0
  without serving.
- `--show-tree` — print the fabricated tree and exit. No model turn.
- `--regen` — discard the tree and re-fabricate.
- `--tcp-port <n>` — opt-in loopback TCP transport; `0` = ephemeral port.

## 3. Fabrication — a real tree, sandboxed

First run spends **one bounded model turn** producing a JSON tree spec
(paths + text contents). The spec is validated and materialized under:

    ~/.cache/smah/virtual-tools/fieldnotes/vfs/
      root/              # the fabricated files themselves
      vfs.sha256         # digest of the tool file — cache key

Hard caps: **200 files, 64 KiB per file, 1 MiB total**. Paths in the spec may
not escape the tool root — a spec containing `..`, an absolute path, or a
symlink escape is a **containment violation: the run aborts with exit 3**
(it is never retried, because an escaping fabrication is hostile, not
confused). Editing the tool file re-keys the cache; `--regen` forces a
re-fabrication.

The files are ordinary files you can open in an editor — that is the point:
`--show-tree` for a quick look, `less $(...)/root/README.md` for the real one.

## 4. The fs tools (code-owned, never fabricated)

Serving is JSON-RPC 2.0 with a **fixed catalog** — the fs tool list is part of
smah, so containment policy cannot be talked around by editing the catalog:

| tool | does |
|---|---|
| `fs_list` | entries under a directory of the tree (`""` = root) |
| `fs_read` | read a text file (symlinks refused) |
| `fs_write` | write/overwrite a text file inside the tree, 64 KiB cap |
| `fs_stat` | kind (file/dir/missing) + byte size |
| `fs_generate` | ask the persona to fabricate more files into the tree (one bounded model turn) |

Transports mirror the MCP lane: **batch stdio by default** (stdin to EOF,
answers in order on stdout), or loopback TCP with `--tcp-port <n>` —
`127.0.0.1` only, loud disclosure, one connection at a time.

`fs_generate` spends fabrication turns from the same turn budget as every
lane; a client that keeps generating keeps spending, and the ordinary turn cap
stops it (exit 5).

## 5. Containment

Every path a client sends is resolved **inside the tree root** before
anything happens:

- `..` components, absolute paths, and paths that resolve outside the root are
  refused; canonicalized ancestor containment (the same allow_roots model as
  the CLI driver's file surface) is applied, not string prefixing.
- **Symlinks are refused** as a final component for both reads and writes —
  the tree contains no symlinks, and the tools will not create or follow one.
- A client escape attempt is refused **per request**: the answer is an error,
  the attempt is audited (`denied`), and the server keeps serving. Only a
  fabrication spec that tries to escape aborts the whole run (§3) — the
  difference between a confused client and a hostile tool.

## 6. Files smah keeps

| path | what |
|---|---|
| `~/.config/smah/virtual-tools/*.md` | your tool definitions (config; back up) |
| `~/.cache/smah/virtual-tools/<tool>/vfs/root/` | the fabricated tree — regenerable |
| `~/.local/state/smah/audit-virtual-tools.jsonl` | durable audit log (§7) |

## 7. Security notes

- **Containment is code, not cooperation** (T-026). The fs catalog is fixed in
  smah's source, every path is resolved and containment-checked in code, and
  escape attempts are refused and audited. The model never mediates access —
  it only fabricates contents within the spec the code validates.
- **The contents are fiction** (T-029's sibling). A fabricated `README.md`
  that claims to be your ssh config is still 64 KiB of fabricated text in
  `~/.cache`; the caps bound the tree, and the audit log records what was
  served and written (kinds and paths, never file contents).
- **Local-only surface.** stdio by default; TCP is loopback-only, loudly
  disclosed, one connection at a time. No auth because there is no network
  surface.
- **No child processes, ever** — serving is this process answering JSON-RPC
  inline; the repo-wide no-execution gate covers these modules.
