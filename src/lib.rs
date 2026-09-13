//! `skim` — syntax-aware source skimming and documentation-budget checks.
//!
//! Two families of work share one tree-sitter parse:
//!
//! * **render** — a source file's declaration skeleton (signatures with bodies
//!   elided), or a markdown file's heading/quote skeleton.
//! * **check** — the budget gate: a document's size must sit in a
//!   sqrt- or linear-proportion band against the source it tracks, and its
//!   comment share must stay under a ceiling.

pub mod check;
pub mod directive;
pub mod discover;
pub mod lang;
pub mod map;
pub mod metrics;
pub mod render;
pub mod search;
