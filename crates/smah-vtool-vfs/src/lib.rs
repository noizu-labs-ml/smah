//! VFS virtual-tool lane (ticket #25) — integration-test home.
//!
//! The driver itself lives in [`smah_core::virtual_tools::drivers::vfs`];
//! this crate runs the lane's black-box tests as their own cargo target
//! without widening `smah-core`'s public API. Tests drive the exact
//! `run(...)` seam `smah-cli` dispatches through, over the batch stdio
//! JSON-RPC transport, against a tree fabricated by a scripted backend.
