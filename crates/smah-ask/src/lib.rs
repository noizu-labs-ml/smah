//! smah-ask :: SMAH question-answering engine — prompt assembly, streaming
//! provider turns, suggestion parsing, risk classification, and rendering
//! (print-only: proposed commands are explained and handed off, never
//! executed).
//!
//! Later tickets grow this crate: sessions + the chat loop (RA-03),
//! clipboard handoff (RA-05), config init (RA-06), doctor (RA-07).

pub mod ask;
pub mod backend;
pub mod classify;
pub mod context;
pub mod prompt;
pub mod render;
pub mod stream;
pub mod suggestion;

pub use ask::{render_suggestion_blocks, turn_json, AskFlow, AskIo, TurnOutcome};
pub use backend::{AskBackend, ScriptedBackend, ThreadMessage, ThreadRole};
pub use classify::{classify, RiskLevel};
pub use context::{context_message, egress_disclosure, EgressToggles};
pub use prompt::{query_turn, refine_turn, system_prompt};
pub use stream::{StreamEvent, StreamFilter};
pub use suggestion::{parse_response, Suggestion};
