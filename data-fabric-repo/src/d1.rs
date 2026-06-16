//! [`D1Repository`] — generic [`Repository`] adapter over `worker::D1Database`.
//!
//! ## Scope of this adapter
//!
//! The existing `src/db.rs` runs ~30 hand-rolled queries. Migrating each
//! one is out of scope for this PR (issue #53 explicitly carves that out
//! as a follow-up). What this adapter establishes is the *shape* the
//! migration will land on:
//!
//! - construction takes a borrowed `&D1Database` so we don't double-own the
//!   binding the worker request handler already holds;
//! - the SQL strings (`select_sql`, `upsert_sql`, `delete_sql`) are passed
//!   in by the caller, so each domain repo (`RunsRepo`, `TasksRepo`, …) is a
//!   thin wrapper that supplies its own table-specific SQL + row decoder;
//! - errors round-trip through [`Error::from(worker::Error)`] so the retry
//!   helper can classify D1 busy / locked failures.
//!
//! We deliberately don't try to be sqlx — generating SQL is out of scope.
//! The point is to have one place that handles error mapping + health
//! probing + retry hooks.

use async_trait::async_trait;
use worker::wasm_bindgen::JsValue;
use worker::D1Database;

use crate::error::Error;
use crate::repository::Repository;

/// Decode a single D1 row into a domain `Value`. Each domain repo provides
/// its own decoder closure so we can stay generic without baking
/// `serde::Deserialize` into the trait surface.
pub type RowDecoder<V> = Box<dyn Fn(&worker::D1Result) -> Result<Option<V>, Error>>;

/// Generic D1 row repository.
///
/// Borrowed handle (`&'a D1Database`) — we **don't** clone the binding
/// because that would defeat the purpose of having a single resolved
/// binding per request. Lifetime `'a` ties the repo to the request scope.
pub struct D1Repository<'a, V> {
    db: &'a D1Database,
    select_sql: &'static str,
    upsert_sql: &'static str,
    delete_sql: &'static str,
    decode: RowDecoder<V>,
}

impl<'a, V> D1Repository<'a, V> {
    /// Build a D1-backed repo.
    ///
    /// SQL strings are `&'static str` because every domain repo statically
    /// owns its queries; this also matches D1's expectation that callers
    /// re-use prepared statements where possible.
    pub fn new(
        db: &'a D1Database,
        select_sql: &'static str,
        upsert_sql: &'static str,
        delete_sql: &'static str,
        decode: RowDecoder<V>,
    ) -> Self {
        Self {
            db,
            select_sql,
            upsert_sql,
            delete_sql,
            decode,
        }
    }

    /// Borrow the underlying D1 binding. Escape hatch for queries that
    /// don't fit the point-CRUD shape (lists, joins, batch writes). The
    /// existing `src/db.rs` will keep calling into this for the foreseeable
    /// future; this method makes the abstraction additive rather than
    /// exclusive.
    pub fn db(&self) -> &D1Database {
        self.db
    }
}

/// Implementation note: `put` here is *not* expected to be used with the
/// trait alone for most of our writes — the existing call sites bind 8-12
/// parameters per insert. The trait's `put(key, value) -> ()` shape is a
/// least-common-denominator that covers blob-shaped writes (e.g. a
/// serialized JSON snapshot keyed by id). For multi-column inserts the
/// inherent helpers + `db()` escape hatch are the right path.
///
/// The `Key` here is `(tenant_id, id)` matching how every table in
/// `src/db.rs` keys rows. We carry both as borrowed `&str` to avoid forcing
/// a `String` allocation per call.
#[async_trait(?Send)]
impl<V> Repository for D1Repository<'_, V>
where
    V: Into<String>,
{
    type Key = (String, String);
    type Value = V;

    async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, Error> {
        let (tenant_id, id) = key;
        let stmt = self
            .db
            .prepare(self.select_sql)
            .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])
            .map_err(Error::from)?;
        let res = stmt.all().await.map_err(Error::from)?;
        (self.decode)(&res)
    }

    async fn put(&self, key: &Self::Key, value: Self::Value) -> Result<(), Error> {
        let (tenant_id, id) = key;
        let payload: String = value.into();
        self.db
            .prepare(self.upsert_sql)
            .bind(&[
                JsValue::from_str(tenant_id),
                JsValue::from_str(id),
                JsValue::from_str(&payload),
            ])
            .map_err(Error::from)?
            .run()
            .await
            .map_err(Error::from)?;
        Ok(())
    }

    async fn delete(&self, key: &Self::Key) -> Result<(), Error> {
        let (tenant_id, id) = key;
        self.db
            .prepare(self.delete_sql)
            .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])
            .map_err(Error::from)?
            .run()
            .await
            .map_err(Error::from)?;
        Ok(())
    }

    async fn health_check(&self) -> Result<(), Error> {
        // `SELECT 1` round-trips the binding without depending on any
        // particular table existing. Matches the standard Workers D1
        // health-probe pattern.
        self.db
            .prepare("SELECT 1")
            .first::<i64>(None)
            .await
            .map_err(Error::from)?;
        Ok(())
    }
}
