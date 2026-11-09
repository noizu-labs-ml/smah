//! DB virtual-tool lane (ticket #24) — integration-test home.
//!
//! The driver itself lives in [`smah_core::virtual_tools::drivers::db`]:
//! a virtual postgres serving the PG-wire-protocol subset over a unix
//! socket, backed by an embedded read-only SQLite store built from a
//! fabricated JSON spec. These tests speak raw wire bytes to the real
//! server via the same `run(...)` seam `smah-cli` dispatches.
