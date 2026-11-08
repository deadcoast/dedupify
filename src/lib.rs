//! `dedupe` — the Dedupelifier.
//!
//! Library surface so every stage is unit-testable and the binary stays a
//! thin dispatcher. Module order follows the data flow:
//!
//! cli → walk → exact → keeper → actions (journal) → render
//!
//! `doctor`, `help` and `menu` are the remaining entry points.

pub mod actions;
pub mod cli;
pub mod doctor;
pub mod exact;
pub mod hamming;
pub mod help;
pub mod journal;
pub mod keeper;
pub mod kind;
pub mod menu;
pub mod near;
pub mod pdq;
pub mod perceptual;
pub mod render;
pub mod theme;
pub mod util;
pub mod video;
pub mod walk;

/// Name of the directory `milk` moves duplicates into, created under the root.
pub const MILKED_DIR: &str = "_milked";
