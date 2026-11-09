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
#[cfg(test)]
pub(crate) mod stub_test_support {
    //! Fixtures shared by the real lanes' self-tests (mcp/db/vfs): a backend
    //! and a sink that must never be reached on the paths under test. The
    //! PR-0 stub fixtures (`stub_tool`, `assert_refuses`) died with the last
    //! stub driver once every lane landed.

    use super::{ChatBackend, ChatMessage, Sink};
    use crate::errors::SmahError;

    /// A backend that must never be reached (the code under test refuses
    /// before any model call).
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
