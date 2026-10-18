//! SMAH terminal UI: ratatui app, widgets, event loop, and TTY-gated surfaces.
//!
//! The virtual-tools **tui driver** (#18) owns the driving loop; this crate
//! is its leaf presentation layer (plain-string API, no smah-core types —
//! see `Cargo.toml`). Entry point: [`vtool`].

pub mod vtool;
