# Virtual tools — DB driver (`driver: db`)

The DB driver gives a virtual tool a **fabricated database**: the model
invents a schema and rows for the tool's declared interface, smah builds them
into a local SQLite store, and the driver serves SELECT-style queries over a
PostgreSQL wire-protocol subset. There is no real postgres, no FUSE, no kernel
mount, and no child process — the smah process is the server, the data is
fabricated, and the store is read-only.

    ~/.config/smah/virtual-tools/chinook.md
    psql -h 127.0.0.1 -p 55432 ...   # a simple-query client (subset, see §4)

This document is the user-facing contract for the DB driver (ticket #24).
Family overview: [virtual-tools.md](virtual-tools.md).

## 1. Declaring a DB tool

    ---
    name: chinook
    description: The Chinook music-store database, fabricated small.
    driver: db
    interface: |
      Tables: artists(id, name), albums(id, artist_id, title),
      tracks(id, album_id, name, composer, unit_price).
      A few dozen artists; realistic rock/jazz catalog.
    ---
    You curate a small music-store dataset. ...

- `driver: db` selects this lane.
- `interface` — the **contract artifact**: the schema the model should
  fabricate (tables, columns, rough row counts). Over 512 KiB it is ignored
  with a note and the persona drives fabrication alone.
- `constraints` are appended to the fabrication prompt as for every lane.

## 2. Flags

Everything after `<tool>` belongs to the driver; an unknown flag or positional
is refused (exit 3):

- `--stdio` — accepted no-op (the db lane has no stdio transport; it serves a
  socket or TCP).
- `--no-serve` — fabricate (or reuse) the store, print the ready notice, exit 0
  without serving.
- `--show-schema` — print the fabricated schema (tables + columns) and exit.
  Deterministic — read from the store, never model text.
- `--regen` — rebuild the store from a fresh fabrication.
- `--tcp-port <n>` — serve loopback TCP instead of the default unix socket;
  `0` = ephemeral port.

## 3. Fabrication — schema, not SQL

First run spends **one bounded model turn** producing a structured JSON schema
spec (never SQL — smah builds the database itself, so the model cannot hand
you a hostile `ATTACH` or pragma). The spec is validated against hard caps —
64 tables, 64 columns per table, 500 rows per table, 2000 rows total, spec
≤ 512 KiB — then materialized into:

    ~/.cache/smah/virtual-tools/chinook/db/
      store.sqlite       # the fabricated data (read-only when served)
      db.sha256          # digest of the tool file — cache key

An invalid reply gets one retry; two invalid specs abort with the turn-limit
error (exit 5). Editing the tool file re-keys the cache and re-fabricates on
the next run; `--regen` forces it.

## 4. Serving — a PG-wire subset, branded as fake

Default endpoint: a unix socket at
`~/.local/state/smah/vtool-db/<tool>.sock`. `--tcp-port <n>` serves loopback
TCP (`127.0.0.1` only) instead; either way the bind prints a loud disclosure:

    smah(db): 'chinook' — smah-virtual-postgres (FABRICATED DATA, read-only)
      serving on 127.0.0.1:55432 — loopback only (ctrl-c to stop; --no-serve skips serving)

Wire behavior (protocol v3, simple query):

- Startup negotiates with **trust auth** (local only) and brands the session:
  the server version string names `smah-virtual-postgres` and says the data is
  **fabricated — not a real database** (T-029). `psql` shows it in the banner;
  drivers see it in the startup ParameterStatus.
- `SELECT` and `WITH` queries run against the store and stream real
  RowDescription/DataRow/CommandComplete frames.
- Everything else is refused with a PG error: writes (`INSERT`/`UPDATE`/
  `DELETE`/DDL) answer `42501` read-only; multiple statements in one message
  and the extended protocol (Parse/Bind/Describe/Execute) answer `0A000`
  feature-not-supported; malformed or unknown messages answer `08P01`; an
  oversized message (> 256 KiB) answers `54000`.
- Statement text is capped at 64 KiB. One connection is served at a time.

The subset is deliberate: enough wire for read-only exploration clients, not
enough surface to matter. It is exercised by an in-tree wire test suite; do
not assume exotic client features (COPY, cursors, prepared statements) work.

## 5. Files smah keeps

| path | what |
|---|---|
| `~/.config/smah/virtual-tools/*.md` | your tool definitions (config; back up) |
| `~/.cache/smah/virtual-tools/<tool>/db/store.sqlite` | fabricated data — regenerable |
| `~/.local/state/smah/vtool-db/<tool>.sock` | serving socket (exists while serving) |
| `~/.local/state/smah/audit-virtual-tools.jsonl` | durable audit log (§6) |

## 6. Security notes

- **This is not your database** (T-029). The rows are invented by a model from
  the persona and the `interface:` text. The branding, the bind disclosure,
  `--show-schema`, and the audit log all exist so nobody mistakes a fabricated
  store for a real one — never point production tooling at a `smah(db)`
  endpoint, and never treat query results as ground truth.
- **Read-only by refusal** (T-030). The store is opened read-only and write
  statements are refused on the wire; the model never sees client queries, so
  no prompt-injection path leads from a query back to fabrication.
- **Local-only surface.** Unix socket under your state dir or loopback-only
  TCP, loudly disclosed, one connection at a time. No auth because there is no
  network surface.
- **Bounded spend.** Message (256 KiB) and statement (64 KiB) caps, table and
  row caps, and the two-strikes fabrication limit keep a hostile client or a
  hostile tool file from spinning the model or the store up without end.
- **Metadata-only audit.** Every served statement lands in the audit log as a
  kind (`select`/`with`/refused) and a decision — statement text and result
  rows are never logged.
