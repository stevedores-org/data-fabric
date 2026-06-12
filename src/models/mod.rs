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

pub use reasoning::*;
pub use telemetry::*;
pub use plays::*;
#[allow(dead_code)]
mod relationships;
mod requests;

// WS5 memory / retrieval.
mod memory;

// AIVCS — slice 1: native `change_set` projection (issue #148).
pub mod aivcs;

pub use entities::*;
pub use memory::*;
pub use orchestration::*;
#[allow(unused_imports)]
pub use relationships::*;
pub use requests::*;
pub use aivcs::{ChangeSet, ChangeSetStatus, CreateChangeSet};

#[cfg(test)]
mod tests;
