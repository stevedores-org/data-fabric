//! WS2: Domain Model and Ontology for autonomous agent-builder data.
//!
//! Canonical entities: Run, Task, Plan, ToolCall, Artifact, PolicyDecision, Release.
//! Relationships: Causality, Dependency, Ownership, Lineage.
//! MCP infrastructure: McpTask, Agent, Checkpoint, GraphEvent (M1-M3).

// WS2 domain ontology — canonical entities and relationships.
#[allow(dead_code)]
mod entities;
#[allow(dead_code)]
mod relationships;
mod requests;

// M1-M3 agent infrastructure — MCP task queue, agents, checkpoints, events.
mod mcp;

pub use entities::*;
pub use mcp::*;
#[allow(unused_imports)]
pub use relationships::*;
pub use requests::*;

#[cfg(test)]
mod tests;
