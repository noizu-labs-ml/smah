//! WEB virtual-tool driver — PR-0 stub (ticket #19).
//!
//! Its lane replaces this file wholesale; until then the driver refuses
//! every run with the family's V-series error. The `run` signature mirrors
//! [`crate::virtual_tools::driver::run`] so the `smah-cli` dispatch and the
//! lane-facing seams stay stable when the real implementation lands.

use crate::errors::SmahError;
use crate::virtual_tools::audit::Audit;
use crate::virtual_tools::drivers::{unavailable, ChatBackend, FsMode, RunOutcome, Sink};
use crate::virtual_tools::loader::ToolDef;

/// Run the web driver — unavailable until its lane lands.
#[allow(clippy::too_many_arguments)]
pub fn run(
    _tool: &ToolDef,
    _backend: &dyn ChatBackend,
    _sink: &mut dyn Sink,
    _model: &str,
    _args: &[String],
    _stdin: Option<&str>,
    _stdin_partial: bool,
    _fs_mode: &FsMode,
    _audit: &Audit,
    _run_id: &str,
) -> Result<RunOutcome, SmahError> {
    Err(unavailable(
        "web",
        "not implemented yet; the web virtual tool ships in a later release",
    ))
}

#[cfg(test)]
mod tests {
    /// Dies with the stub: the lane landing this driver deletes this file —
    /// and with it this test — wholesale.
    #[test]
    fn stub_refuses_driver_unavailable_exit_3() {
        crate::virtual_tools::drivers::stub_test_support::assert_refuses(super::run, "web");
    }
}
