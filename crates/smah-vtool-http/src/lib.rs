//! SMAH virtual-tools HTTP plumbing: a loopback-only axum host for the
//! **api** driver (#20), OpenAPI contract validation + atomic cache writes,
//! and an interruptible sleep for the **web** driver's watch loop (#19).
//!
//! Leaf crate: plain strings + closures only, no smah-core types. All
//! listeners bind `127.0.0.1` exclusively (THREAT-MODEL T-019/T-020).

pub mod contract;
pub mod pins;
pub mod server;
pub mod sleeper;
