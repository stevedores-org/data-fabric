//! [`R2Repository`] — [`Repository`] adapter over `worker::Bucket`.
//!
//! Wraps the three patterns we use in `src/storage.rs` today —
//! `put_blob`, `get_blob`, `delete_blob` — behind the same trait surface
//! D1 uses. This makes it possible to write retry-wrapped, error-classified
//! storage helpers that don't care whether the backend is D1 or R2.

use async_trait::async_trait;
use worker::Bucket;

use crate::error::Error;
use crate::repository::Repository;

/// R2-backed blob repository.
///
/// Same borrowed-binding pattern as [`crate::d1::D1Repository`]: the worker
/// hands the request handler a `Bucket` and the repo is a thin view over
/// that single handle.
pub struct R2Repository<'a> {
    bucket: &'a Bucket,
}

impl<'a> R2Repository<'a> {
    pub fn new(bucket: &'a Bucket) -> Self {
        Self { bucket }
    }

    /// Escape hatch for callers that need multipart uploads, listing, or
    /// `head()` metadata — i.e. anything outside the point-CRUD trait.
    pub fn bucket(&self) -> &Bucket {
        self.bucket
    }
}

#[async_trait(?Send)]
impl Repository for R2Repository<'_> {
    /// R2 keys are flat strings — no tenant dimension at the bucket level.
    /// Tenant scoping at the *application* level is implemented by prefixing
    /// keys with the tenant id, same as the existing `src/storage.rs`
    /// helpers do. We don't bake that into this adapter because some keys
    /// (e.g. shared schema artifacts) are deliberately tenantless.
    type Key = String;
    type Value = Vec<u8>;

    async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, Error> {
        let obj = self.bucket.get(key.clone()).execute().await.map_err(Error::from)?;
        match obj {
            Some(obj) => match obj.body() {
                Some(body) => {
                    let bytes = body.bytes().await.map_err(Error::from)?;
                    Ok(Some(bytes))
                }
                // Object exists but body is empty — matches existing
                // semantics in src/storage.rs::get_blob.
                None => Ok(Some(Vec::new())),
            },
            None => Ok(None),
        }
    }

    async fn put(&self, key: &Self::Key, value: Self::Value) -> Result<(), Error> {
        self.bucket
            .put(key.clone(), value)
            .execute()
            .await
            .map_err(Error::from)?;
        Ok(())
    }

    async fn delete(&self, key: &Self::Key) -> Result<(), Error> {
        // R2's delete is idempotent on the runtime side — missing key
        // returns Ok. Matches our trait contract.
        self.bucket.delete(key.clone()).await.map_err(Error::from)?;
        Ok(())
    }

    async fn health_check(&self) -> Result<(), Error> {
        // `head` of a sentinel key is the cheapest call that exercises
        // auth + network without writing anything. The key doesn't need
        // to exist — we accept `Ok(None)` as healthy.
        //
        // We intentionally use a fixed key under a reserved prefix so this
        // doesn't collide with real data even if `__healthcheck__` shows
        // up in user payloads.
        self.bucket
            .head("__data_fabric_repo_healthcheck__")
            .await
            .map_err(Error::from)?;
        Ok(())
    }
}
