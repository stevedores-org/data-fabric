use serde::{Deserialize, Serialize};

use super::EntityKind;

// ── Relationship types ──────────────────────────────────────────

/// What caused what — tracks causal chains across entities.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Causality {
    pub from_kind: EntityKind,
    pub from_id: String,
    pub to_kind: EntityKind,
    pub to_id: String,
    pub relation: String,
}

/// Task/plan dependency — must complete before another can start.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Dependency {
    pub source_kind: EntityKind,
    pub source_id: String,
    pub depends_on_kind: EntityKind,
    pub depends_on_id: String,
}

/// Who or what owns an entity.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Ownership {
    pub entity_kind: EntityKind,
    pub entity_id: String,
    pub owner: String,
}

/// Artifact/release lineage — what was derived from what.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Lineage {
    pub entity_kind: EntityKind,
    pub entity_id: String,
    pub parent_kind: EntityKind,
    pub parent_id: String,
}

/// A typed edge in the entity graph. Wraps all relationship variants.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Relationship {
    Causality(Causality),
    Dependency(Dependency),
    Ownership(Ownership),
    Lineage(Lineage),
}
