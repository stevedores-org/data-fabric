//! WS2: Domain Model and Ontology for autonomous agent-builder data.
//!
//! Canonical entities: Run, Task, Plan, ToolCall, Artifact, PolicyDecision, Release.
//! Relationships: Causality, Dependency, Ownership, Lineage.
//!
//! Entity and relationship types are defined here for schema validation and
//! serialization tests. They will be persisted to D1 in M1 (core fabric IO).

// Entities and relationships are the canonical schema — used in tests now,
// wired to D1 storage in M1.
#[allow(dead_code)]
mod entities;
#[allow(dead_code)]
mod relationships;
mod requests;

pub use entities::*;
#[allow(unused_imports)]
pub use relationships::*;
pub use requests::*;

#[cfg(test)]
mod tests;
