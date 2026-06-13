//! [`Repository`] trait + the small bag of context-bag types it needs.
//!
//! This is the *shape* the rest of the fabric will gradually migrate onto.
//! For this PR's scope we only define the trait and the two concrete
//! adapters ([`crate::d1::D1Repository`], [`crate::r2::R2Repository`]) —
//! call sites in `src/db.rs` / `src/storage.rs` are not migrated here.
//!
//! ## Why `?Send`
//!
//! All `worker-rs` futures are `!Send` because they capture `JsValue`s,
//! which can only be touched from the single Workers thread. Marking the
//! trait `?Send` lets us await `D1Database::prepare(...).first(...)` and
//! `Bucket::get(...).execute()` directly, with no `spawn_local` dance.
//! It's also forward-compatible with multi-threaded native test runners
//! since `?Send` *permits* but does not *require* non-Send futures.

use async_trait::async_trait;

use crate::error::Error;

/// Generic CRUD-flavored repository over a typed `Key`/`Value` pair.
///
/// Intentionally minimal — the trait isn't trying to be ORM-shaped. It
/// expresses the four shapes we use across the worker today:
///
/// 1. Point-read by key   → [`Repository::get`]
/// 2. Point-write by key  → [`Repository::put`]
/// 3. Point-delete by key → [`Repository::delete`]
/// 4. Liveness probe      → [`Repository::health_check`]
///
/// **Listing, pagination, and tenant scoping are deliberately not in this
/// trait.** They vary too much across our call sites (cursored vs. limit/
/// offset, tenant-prefixed vs. WHERE-claused) to capture without
/// over-fitting. Concrete adapter types add those as inherent methods —
/// see the validation section of PR #53 for which patterns we mapped to
/// this trait and which stay adapter-specific.
#[async_trait(?Send)]
pub trait Repository {
    /// Logical primary key. For D1 row repos this is a `(tenant_id, id)`
    /// pair; for R2 it's the object key as a `String`. Adapter-defined so
    /// the trait doesn't impose D1's two-column convention on R2 (or vice
    /// versa).
    type Key;
    /// Value type. For D1 row repos this is the model struct; for R2
    /// blobs it's `Vec<u8>`.
    type Value;

    /// Read one. `Ok(None)` means "definitely not there"; `Err(NotFound)`
    /// is reserved for the runtime explicitly reporting absence as an
    /// error (rare on D1, can happen on R2 metadata calls).
    async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, Error>;

    /// Write one. Insert-or-replace semantics by default; conflict-on-
    /// existing semantics should be expressed as an inherent method on the
    /// adapter or with a typed wrapper around `Value`.
    async fn put(&self, key: &Self::Key, value: Self::Value) -> Result<(), Error>;

    /// Delete one. Idempotent: deleting a missing key returns `Ok(())`.
    /// This matches both R2's `delete()` behavior and the way our D1 call
    /// sites already swallow zero-affected-rows responses.
    async fn delete(&self, key: &Self::Key) -> Result<(), Error>;

    /// Cheap end-to-end probe used by the worker's `/healthz` handler. Must
    /// touch the backend (not just the binding) so this catches credential
    /// rotation, network partitions, and bucket-deleted scenarios.
    async fn health_check(&self) -> Result<(), Error>;
}

/// Binding-resolution config.
///
/// We pass the *binding name* (e.g. `"DB"` for D1, `"BLOBS"` for R2) plus an
/// optional logical name for observability. Today both adapters are
/// constructed from already-resolved `worker::D1Database` / `worker::Bucket`
/// handles inside the request handler, so this struct mostly exists for
/// future use: a forthcoming PR can plumb it through an `init` constructor
/// once the worker request lifecycle is refactored to do binding lookup in
/// a single place.
#[derive(Debug, Clone)]
pub struct RepositoryConfig {
    /// Name of the binding as declared in `wrangler.toml`.
    pub binding: String,
    /// Optional human name for logs/metrics. Defaults to `binding` if unset.
    pub logical_name: Option<String>,
}

impl RepositoryConfig {
    pub fn new(binding: impl Into<String>) -> Self {
        Self {
            binding: binding.into(),
            logical_name: None,
        }
    }

    #[must_use]
    pub fn with_logical_name(mut self, name: impl Into<String>) -> Self {
        self.logical_name = Some(name.into());
        self
    }

    /// Name to use in logs. Falls back to `binding` so we always have
    /// *something* to grep on.
    pub fn name(&self) -> &str {
        self.logical_name.as_deref().unwrap_or(&self.binding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_name_falls_back_to_binding() {
        let c = RepositoryConfig::new("DB");
        assert_eq!(c.name(), "DB");
        let c = c.with_logical_name("runs_db");
        assert_eq!(c.name(), "runs_db");
    }
}
