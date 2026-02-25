//! WS2: Domain Model and Ontology for autonomous agent-builder data.
//!
//! Canonical entities: Run, Task, Plan, ToolCall, Artifact, PolicyDecision, Release.
//! Relationships: Causality, Dependency, Ownership, Lineage.
//! MCP infrastructure: AgentTask, Agent, Checkpoint, GraphEvent (M1-M3).

// WS2 domain ontology — canonical entities and relationships.
#[allow(dead_code)]
mod entities;
pub mod orchestration;
#[allow(dead_code)]
mod relationships;
mod requests;

// WS5 memory / retrieval.
mod memory;

pub use entities::*;
pub use memory::*;
pub use orchestration::*;
#[allow(unused_imports)]
pub use relationships::*;
pub use requests::*;

#[cfg(test)]
mod tests;
