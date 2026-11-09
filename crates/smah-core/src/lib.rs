//! 〚📁:453837ac-f3d1-5247-95ec-e5c954bd2238〛 smah-core :: UI-agnostic domain core — settings, runner, parser, db, console.
//! SMAH domain core: UI-agnostic settings, runner, parser, db, console, and
//! checks. Milestone M1 lands the domain floor: exit codes, the §4 error
//! contract, the console split, redacted logging, and the settings loader.

pub mod console;
pub mod errors;
pub mod exit_codes;
pub mod logging;
pub mod settings;
pub mod virtual_tools;
pub mod wrappers;

pub use errors::SmahError;
pub use exit_codes::ExitCode;
