//! `data-fabric-repo` — shared D1/R2 repository abstraction for the
//! Cloudflare-native data fabric worker.
//!
//! Implements the WS1B deliverables from
//! [stevedores-org/data-fabric#53](https://github.com/stevedores-org/data-fabric/issues/53):
//!
//! 1. [`Repository`] trait — generic CRUD over D1 / R2 with consistent
//!    error handling.
//! 2. [`Error`] — typed taxonomy mapping cleanly to HTTP status codes
//!    400 / 401 / 404 / 409 / 500 / 503 (see [`Error::status_code`]).
//! 3. [`RepositoryConfig`] + per-adapter `health_check()` — initialization
//!    patterns for binding resolution and liveness probes.
//! 4. [`with_retry`] / [`RetryPolicy`] — exponential backoff with bounded
//!    jitter for the transient failure modes specific to Cloudflare D1 and
//!    R2 (busy, locked, timeout, internal_error).
//!
//! ## Scope (what this PR does NOT do)
//!
//! It does **not** refactor `src/db.rs` or `src/storage.rs` to *use* these
//! abstractions. Migration of the existing call sites is a follow-up
//! tracked separately. The trait was validated against 3 representative
//! patterns from `db.rs` and 2 from `storage.rs` — see the PR body for the
//! mapping table.
//!
//! ## Why the trait is `?Send`
//!
//! Workers futures capture `JsValue`s and are therefore `!Send`. Marking
//! the trait `?Send` lets implementations await `worker::*` futures
//! directly without `spawn_local`.

#![deny(clippy::unwrap_used, clippy::expect_used)]
#![warn(clippy::pedantic)]
#![allow(
    // We're explicit about which lints we silence and why. Each one below
    // is here because pedantic flags it on plain, correct code in the
    // wasm/worker idiom and not because we're hiding bugs.
    clippy::module_name_repetitions, // `D1Repository` in module `d1` is the point.
    clippy::missing_errors_doc,      // re-doc'd on the trait, not every impl.
    clippy::must_use_candidate,      // builder-style `with_logical_name`.
)]

pub mod d1;
pub mod error;
pub mod r2;
pub mod repository;
pub mod retry;

pub use d1::D1Repository;
pub use error::Error;
pub use r2::R2Repository;
pub use repository::{Repository, RepositoryConfig};
pub use retry::{with_retry, MonotonicClock, RetryPolicy, Sleeper, WorkerClock, WorkerSleeper};
