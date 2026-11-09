//! smah-ask :: SMAH question-answering engine — suggestion parsing, risk
//! classification, and rendering (print-only: proposed commands are
//! explained and handed off, never executed).
//!
//! Later tickets grow this crate: prompt assembly + the ask flow (RA-01),
//! sessions + the chat loop (RA-03), clipboard handoff (RA-05), config init
//! (RA-06), doctor (RA-07).

pub mod classify;
pub mod render;
pub mod suggestion;

pub use classify::{classify, RiskLevel};
pub use suggestion::{parse_response, Suggestion};
