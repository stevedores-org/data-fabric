//! WS2: Domain Model and Ontology for autonomous agent-builder data.
//!
//! Canonical entities: Run, Task, Plan, ToolCall, Artifact, PolicyDecision, Release.
//! Relationships: Causality, Dependency, Ownership, Lineage.
//! MCP infrastructure: AgentTask, Agent, Checkpoint, GraphEvent (M1-M3).

// WS2 domain ontology — canonical entities and relationships.
#[allow(dead_code)]
mod entities;
pub mod orchestration;
mod reasoning;
mod telemetry;
mod plays;

// Issue #148 / AIVCS slice 3 — human decision projection.
mod aivcs_human_decision;

pub use reasoning::*;
pub use telemetry::*;
pub use plays::*;
pub use aivcs_human_decision::*;
#[allow(dead_code)]
mod relationships;
mod requests;

// WS5 memory / retrieval.
mod memory;

// AIVCS — slice 1: native `change_set` projection (issue #148).
pub mod aivcs;

// AIVCS review projections (issue #148 slice 2).
// Read by the slice-3+ HTTP routes (not yet landed) and by the
// follower process that projects events onto these tables, so
// the public API is intentionally broader than what's referenced
// from lib.rs in this slice.
#[allow(dead_code)]
pub mod aivcs_review;

pub use entities::*;
pub use memory::*;
pub use orchestration::*;
#[allow(unused_imports)]
pub use relationships::*;
pub use requests::*;
pub use aivcs::{ChangeSet, ChangeSetStatus, CreateChangeSet};
#[allow(unused_imports)]
pub use aivcs_review::*;

#[cfg(test)]
mod tests;
