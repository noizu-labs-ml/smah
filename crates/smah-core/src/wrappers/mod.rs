//! Script wrappers: user-authored multi-step recipes the deterministic path
//! *prints* (smah stays print-only — owner decision 2026-11-09) and the
//! companion `smah-run` binary executes after explicit user approval.
//!
//! A wrapper is a YAML-frontmatter markdown file in
//! `~/.config/smah/wrappers/<name>.md` — a **separate namespace** from
//! virtual tools (owner decision Q2): virtual tools are single-purpose
//! personas; wrappers compose reusable steps into an ordered recipe. A name
//! collision between a wrapper and an installed virtual tool is a load
//! error (owner decision Q6 — silent shadowing is a footgun).
//!
//! Modules:
//! - [`loader`]: parse/validate/discover `~/.config/smah/wrappers/*.md`
//! - [`cast`]: asciinema v2 `.cast` parsing + ANSI stripping (pure Rust)
//! - [`builtin`]: the compiled-in tool registry (`cast-to-text`)
//! - [`resolve`]: the `{{params.x}}` / `{{steps.y.output}}` binding engine
//!   and the deterministic plan renderer
//! - [`prompt`] (SW-03): the `--prompt` inference composer — structured
//!   recipe diff, review flow, provenance stamp, (wrapper, prompt) cache
//!
//! Execution lives in the separate `smah-run` crate (the only workspace
//! member allowed to spawn processes); `smah` itself never invokes it — it
//! only prints the exact invocation (owner decision 2026-11-09).

pub mod builtin;
pub mod cast;
pub mod loader;
pub mod resolve;

/// Maximum number of steps one wrapper may declare (load-time cap; a
/// longer recipe is abuse of the format, not a pipeline).
pub const STEPS_MAX: usize = 64;

/// Maximum number of declared params.
pub const PARAMS_MAX: usize = 32;

/// A binding placeholder found inside a step template string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// `{{params.<name>}}` — a declared wrapper param.
    Param(String),
    /// `{{steps.<id>.output}}` — the bound stdout of an earlier step.
    StepOutput(String),
}

impl Binding {
    /// Render back to the placeholder form (error messages, plan display).
    pub fn to_placeholder(&self) -> String {
        match self {
            Binding::Param(name) => format!("{{{{params.{name}}}}}"),
            Binding::StepOutput(id) => format!("{{{{steps.{id}.output}}}}"),
        }
    }
}
