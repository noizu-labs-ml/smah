//! The virtual-tools driver family (PR-0 seam, family plan §1).
//!
//! A tool file's `driver:` frontmatter names the presentation driver. The
//! canonical set is fixed and shared by every lane:
//!
//! ```text
//! cli | tui | web | api | soap | mcp | grpc | vfs | db
//! ```
//!
//! `"emulated"` is a permanent alias of `"cli"` (legacy tool files keep
//! loading; [`canonical`] maps it). Every driver module exposes the same
//! entry-point shape — a `run(...)` with the signature of
//! [`crate::virtual_tools::driver::run`] — so the `smah-cli` dispatch and
//! the lane-facing seams stay stable while lanes replace their stub.
//!
//! Status: `cli` is fully implemented ([`crate::virtual_tools::driver`]);
//! the other eight modules are stubs answering
//! [`SmahError::DriverUnavailable`] until their lane lands (#18–#25).
//! After PR-0 the lanes own their stub file, their driver module, and the
//! workspace crates they add — nothing else.

pub mod api;
pub mod db;
pub mod grpc;
pub mod mcp;
pub mod soap;
pub mod tui;
pub mod vfs;
pub mod web;

use crate::errors::SmahError;

/// The canonical driver names, in table order.
pub const DRIVER_NAMES: [&str; 9] = [
    "cli", "tui", "web", "api", "soap", "mcp", "grpc", "vfs", "db",
];

/// The one historical alias. `"emulated"` is a permanent alias of `"cli"`
/// (family plan D-taxonomy): existing tool files keep loading, [`canonical`]
/// maps it, and [`crate::virtual_tools::loader::ToolDef::driver`] stores the
/// canonical name.
const ALIASES: [(&str, &str); 1] = [("emulated", "cli")];

/// Map any accepted `driver:` spelling to its canonical name. `"emulated"`
/// → `"cli"`; every canonical name maps to itself; `None` = not a driver
/// this family knows (the loader rejects the file).
pub fn canonical(name: &str) -> Option<&'static str> {
    for (alias, target) in ALIASES {
        if alias == name {
            return Some(target);
        }
    }
    DRIVER_NAMES
        .iter()
        .copied()
        .find(|canonical| *canonical == name)
}

/// Is `name` an accepted `driver:` spelling (canonical or alias)?
pub fn known(name: &str) -> bool {
    canonical(name).is_some()
}

/// The V-series error every unavailable driver returns (`SmahError::
/// DriverUnavailable`, exit 3, Config class).
pub fn unavailable(driver: &str, hint: &str) -> SmahError {
    SmahError::DriverUnavailable {
        driver: driver.to_string(),
        hint: hint.to_string(),
    }
}

// Lane-facing seams, re-exported so new drivers depend on `drivers::` and
// never on the cli driver's module path. `RunOutcome`/`FsMode` ride along
// because a driver's `run` both takes and produces them.
pub use crate::virtual_tools::driver::{
    ChatBackend, ChatMessage, FsMode, Role, RunOutcome, Sink, StdSink,
};

#[cfg(test)]
pub(crate) mod stub_test_support {
    //! Shared fixtures for the eight per-stub self-tests. Lives beside the
    //! stubs so it dies with them: once every lane lands, the whole module
    //! (and its only callers) delete together.

    use std::path::PathBuf;

    use super::{ChatBackend, ChatMessage, FsMode, RunOutcome, Sink};
    use crate::errors::SmahError;
    use crate::exit_codes::ExitCode;
    use crate::virtual_tools::audit::Audit;
    use crate::virtual_tools::loader::ToolDef;

    /// A backend that must never be reached (stubs refuse before any model
    /// call).
    pub(crate) struct NullBackend;

    impl ChatBackend for NullBackend {
        fn stream_turn(
            &self,
            _model: &str,
            _system: &str,
            _messages: &[ChatMessage],
            _on_delta: &mut dyn FnMut(&str),
        ) -> Result<String, SmahError> {
            panic!("stub driver must refuse before any model call");
        }
    }

    /// A sink that swallows stderr notices (the driver prints its V-series
    /// notice before returning the error).
    pub(crate) struct NullSink;

    impl Sink for NullSink {
        fn content(&mut self, piece: &str) {
            panic!("stub driver must not stream content, got {piece:?}");
        }

        fn notice(&mut self, _line: &str) {}
    }

    /// A minimal valid tool claiming the given canonical driver.
    pub(crate) fn stub_tool(driver: &str) -> ToolDef {
        ToolDef {
            name: "stub".to_string(),
            description: "stub fixture".to_string(),
            model: None,
            driver: driver.to_string(),
            constraints: Vec::new(),
            allow_roots: Vec::new(),
            interface: None,
            body: "stub persona".to_string(),
            path: PathBuf::from("stub.md"),
        }
    }

    /// Assert a stub driver's `run` refuses with `DriverUnavailable`
    /// carrying the right driver name, a non-empty hint, and the Config
    /// exit class (3) — the contract every lane's replacement keeps.
    pub(crate) fn assert_refuses<F>(run: F, driver: &str)
    where
        F: FnOnce(
            &ToolDef,
            &dyn ChatBackend,
            &mut dyn Sink,
            &str,
            &[String],
            Option<&str>,
            bool,
            &FsMode,
            &Audit,
            &str,
        ) -> Result<RunOutcome, SmahError>,
    {
        // The Audit create_dir_all's its parent and chmods it 0700 — always
        // hand it a path under a fresh tempdir, never a shared one.
        let dir = tempfile::tempdir().expect("stub-test tempdir");
        let audit = Audit::open_at(&dir.path().join("audit.jsonl"), "run");
        let tool = stub_tool(driver);
        let mut sink = NullSink;
        let err = run(
            &tool,
            &NullBackend,
            &mut sink,
            "stub-model",
            &[],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "stub-run",
        )
        .expect_err("stub driver must refuse");
        let SmahError::DriverUnavailable { driver: got, hint } = &err else {
            panic!("expected DriverUnavailable, got {err}");
        };
        assert_eq!(got, driver, "error names the canonical driver");
        assert!(!hint.is_empty(), "hint is non-empty");
        assert_eq!(
            err.exit_code(),
            ExitCode::Config,
            "driver unavailability is Config class (exit 3)"
        );
        let rendered = err.to_string();
        assert!(
            rendered.contains(driver) && rendered.contains("driver"),
            "rendered message names the driver: {rendered}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical, known, DRIVER_NAMES};

    #[test]
    fn every_driver_name_is_canonical_identity() {
        for name in DRIVER_NAMES {
            assert_eq!(canonical(name), Some(name), "{name}");
            assert!(known(name), "{name}");
        }
    }

    #[test]
    fn emulated_is_a_permanent_alias_of_cli() {
        assert_eq!(canonical("emulated"), Some("cli"));
        assert!(known("emulated"));
        // The alias does not appear as a stored/canonical name.
        assert!(!DRIVER_NAMES.contains(&"emulated"));
    }

    #[test]
    fn unknown_drivers_are_rejected() {
        for name in ["", "fortran", "emulate", "CLI", "cli2", "tui "] {
            assert_eq!(canonical(name), None, "{name:?}");
            assert!(!known(name), "{name:?}");
        }
    }
}
