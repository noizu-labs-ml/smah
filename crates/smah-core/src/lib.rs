//! SMAH domain core: UI-agnostic settings, runner, parser, db, console, and
//! checks. Milestone M1 lands the domain floor: exit codes, the §4 error
//! contract, the console split, redacted logging, and the settings loader.

pub mod console;
pub mod errors;
pub mod exit_codes;
pub mod logging;
pub mod settings;
pub mod virtual_tools;

pub use errors::SmahError;
pub use exit_codes::ExitCode;
