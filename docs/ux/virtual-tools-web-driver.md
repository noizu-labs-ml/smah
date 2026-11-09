# Virtual tools — web driver (`smah --emulate`, `driver: web`)

The web driver turns a tool file into a small fabricated web page: the model
writes the page **content** (a strings-only `content.json`), smah freezes it
in a cache, and a tiny loopback-only companion service renders it in a
browser. Refreshing the page can ask the driver to regenerate the content —
the model stays in the loop across regenerations.

    smah --emulate inventory-board &        # driver: fabricates + watches
    go run ./services/web-vtool --content ~/.cache/smah/virtual-tools/\
inventory-board/web/content.json --tool inventory-board
    open http://127.0.0.1:8090

This document is the user-facing contract for the web driver (ticket #19).
Exit codes and error text follow interface-spec §4/§5.

## 1. Installing a tool

Same as every driver: one `.md` file in `~/.config/smah/virtual-tools/`.

    ---
    name: inventory-board
    description: A mock warehouse dashboard for the demo.
    driver: web
    interface: |
      A dashboard for a widget warehouse: a title, a status headline, and
      three sections — stock levels, incoming shipments, alerts.
    ---
    You maintain a plausible fake warehouse. Invent consistent data. ...

**`interface` is required** for this driver: it is the page description the
model authors against. A file without it (or with an empty one) loads but
refuses to run (`invalid tool file`, exit 3).

## 2. Running a tool

    smah --emulate inventory-board [page request...] [--no-watch]
         [--watch-interval SECS]

- The driver fabricates the page content (up to 2 attempts), writes it to
  the content cache, and prints the serve instructions on **stderr**.
  stdout stays empty.
- **Watch mode (default)** — the driver then watches for regeneration
  requests (below), each costing one model turn, until the 8-turn budget or
  Ctrl-C ends the run. A regen that fails validation keeps the previous
  content on disk; the page never goes blank on a bad model turn.
- `--no-watch` — fabricate, print the instructions, exit. Re-runs with an
  unchanged tool file are cache hits (no model turn).
- `--watch-interval N` — poll interval in seconds for regeneration
  requests (default 2).
- **The driver binds nothing.** It writes files and prints; the browser
  service is a separate process you start.

## 3. The companion service

`services/web-vtool/` (Go, standard library + vendored htmx) renders the
page:

    go run ./services/web-vtool --content <content.json path> \
        --tool inventory-board [--addr 127.0.0.1:8090]

- Serves the page at `http://127.0.0.1:8090` (loopback only — **it refuses
  to start on any non-loopback address**, including a bare `:port`, which
  would bind every interface).
- `Ask the driver to regenerate` (the page button) POSTs `/regen`, which
  touches `regen.request` in the content cache; the driver's watch loop
  picks it up and fabricates a new page. When new content lands, the page
  updates itself (Server-Sent Events push).
- The page renders fabricated strings as escaped text — model output can
  never inject markup or script (server-side `html/template` auto-escaping,
  client-side `textContent`, restrictive CSP).

## 4. Content format and caps

`content.json` is a JSON object whose values are all strings — keys are page
sections (`title` is the page heading), rendered as plain text. Caps: 32
keys, 64 KiB per value, 128 KiB total. Anything else fails validation and is
sent back to the model for correction (initial generation: 2 attempts, then
exit 5).

## 5. Files smah keeps

| path | what |
|---|---|
| `~/.cache/smah/virtual-tools/<tool>/web/content.json` | the frozen page content — regenerable, safe to delete |
| `~/.cache/smah/virtual-tools/<tool>/web/content.json.sha256` | staleness marker pinning the cache to the tool file's sha — a changed tool file invalidates the cache |
| `~/.cache/smah/virtual-tools/<tool>/web/regen.request` | touched by the page's refresh button; mtime is the signal |
| `~/.local/state/smah/audit-virtual-tools.jsonl` | durable audit log (`web.generate`, `web.serve`, `web.push`) |

## 6. Exit codes

Same table as every smah command (interface-spec §4): `0` success (including
watch mode ended by Ctrl-C or budget) · `2` no such tool · `3` invalid tool
file (missing `interface`) · `4` auth · `5` provider/network failure or the
8-turn budget exhausted.

## 7. Security notes

Three surfaces, all loopback-confined: the **content** is model-written but
rendered as escaped text only (a hostile page can say scary things; it
cannot run script in your browser); the **service** binds loopback only,
refuses cross-origin regen/push requests, and makes no network egress; the
**cache** is validate-on-load — a poisoned `content.json` is rejected and
regenerated, and the sha marker pins it to the tool file so editing the tool
never serves stale content. Only install tools you trust; watch the audit
log.
