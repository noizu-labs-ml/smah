//! MCP virtual-tool lane (ticket #23) — integration-test home.
//!
//! The driver itself lives in [`smah_core::virtual_tools::drivers::mcp`];
//! this crate exists so the lane's black-box tests run as their own cargo
//! target without widening `smah-core`'s public API. The tests drive the
//! exact `run(...)` seam `smah-cli` dispatches through: a scripted
//! [`ChatBackend`] stands in for the model, and the batch stdio transport
//! is fed newline-delimited JSON-RPC 2.0 in-process.
