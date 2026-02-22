//! WS2: Domain Model and Ontology for autonomous agent-builder data.
//!
//! Canonical entities: Run, Task, Plan, ToolCall, Artifact, PolicyDecision, Release.
//! Relationships: Causality, Dependency, Ownership, Lineage.
//! Orchestration: AgentTask, Agent, Checkpoint, GraphEvent (M1-M3).

// Entities and relationships are the canonical schema — used in tests now,
// wired to D1 storage in M1.
#[allow(dead_code)]
mod entities;
pub mod orchestration;
#[allow(dead_code)]
mod relationships;
mod requests;

pub use entities::*;
pub use orchestration::*;
#[allow(unused_imports)]
pub use relationships::*;
pub use requests::*;

#[cfg(test)]
mod tests;
