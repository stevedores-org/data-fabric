//! Retry policy with exponential backoff and bounded jitter.
//!
//! ## Why a custom helper instead of `backoff` / `tokio-retry`
//!
//! The Workers runtime is single-threaded and `worker::Delay` is the only
//! sanctioned sleep primitive on wasm32 — `tokio::time::sleep` doesn't work
//! there, and `std::thread::sleep` blocks the only event loop. Existing
//! crates pull `rand` (for jitter) and either `tokio` or `std::time::Instant`
//! (for clocks), neither of which we want in the wasm binary.
//!
//! ## Why no `rand` dependency
//!
//! Real PRNG-based jitter would require pulling `rand` + `getrandom` with
//! the `js` feature. We already pay for `getrandom` at the worker layer,
//! but `rand` brings ~100KB of code paths we don't need. Instead we use
//! `worker::Date::now().as_millis()` as a chaotic-enough source for jitter:
//! across concurrent invocations the millisecond timestamps differ, which
//! is the only property we need to prevent thundering-herd retry storms.
//! For unit tests we expose [`with_retry_clock`] which takes any
//! [`MonotonicClock`], so the tests don't depend on `Date::now`.

use crate::error::Error;
use std::future::Future;
use std::time::Duration;

/// Retry policy.
///
/// `max_attempts` is the total number of *attempts* (not retries) — a policy
/// with `max_attempts = 1` will never retry. `initial_backoff_ms` is the
/// delay before the first retry; subsequent retries double up to
/// `max_backoff_ms`, then jitter is layered on top.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total number of attempts including the first. Must be >= 1.
    pub max_attempts: u32,
    /// Delay before the 2nd attempt, in milliseconds.
    pub initial_backoff_ms: u64,
    /// Cap on the deterministic part of the backoff, in milliseconds.
    pub max_backoff_ms: u64,
}

impl RetryPolicy {
    /// Sensible defaults for D1/R2: 4 attempts, 50ms → 800ms (50, 100, 200, 400 then cap).
    /// Total worst-case wait ≈ 750ms before reporting failure, which fits
    /// comfortably under typical Workers CPU-time budgets.
    pub fn default_for_storage() -> Self {
        Self {
            max_attempts: 4,
            initial_backoff_ms: 50,
            max_backoff_ms: 800,
        }
    }

    /// A no-retry policy. Useful for read paths where the caller would rather
    /// surface failure fast than wait.
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
        }
    }

    /// Compute the deterministic backoff for `attempt_index` (0-based —
    /// `attempt_index == 0` is the delay *before* the 2nd attempt).
    ///
    /// Doubling proceeds in `u128` to avoid overflow on policies with large
    /// `initial_backoff_ms`, then clamps back into `u64`.
    pub fn backoff_for(&self, attempt_index: u32) -> Duration {
        let base = self.initial_backoff_ms as u128;
        let shifted = base
            .checked_shl(attempt_index)
            .unwrap_or(self.max_backoff_ms as u128);
        let capped = shifted.min(self.max_backoff_ms as u128) as u64;
        Duration::from_millis(capped)
    }
}

/// Source of monotonic-ish "now" used to derive jitter and to allow tests to
/// inject a fake clock. In production it's wired to `worker::Date::now`.
pub trait MonotonicClock {
    fn now_ms(&self) -> u64;
}

/// Production clock: `worker::Date::now()`.
///
/// `Date::now()` returns wall-clock milliseconds. It can move backwards
/// across NTP adjustments — fine for jitter, **don't** use this for timing
/// SLAs.
pub struct WorkerClock;

impl MonotonicClock for WorkerClock {
    fn now_ms(&self) -> u64 {
        worker::Date::now().as_millis()
    }
}

/// Sleep primitive. Abstracted for the same reason as the clock: unit tests
/// shouldn't actually sleep, and `worker::Delay` doesn't work on the host
/// target anyway.
#[async_trait::async_trait(?Send)]
pub trait Sleeper {
    async fn sleep(&self, dur: Duration);
}

/// Production sleeper: `worker::Delay`.
pub struct WorkerSleeper;

#[async_trait::async_trait(?Send)]
impl Sleeper for WorkerSleeper {
    async fn sleep(&self, dur: Duration) {
        worker::Delay::from(dur).await;
    }
}

/// Bounded jitter: returns a value in `[0, base_ms * JITTER_FRACTION_NUM / JITTER_FRACTION_DEN]`.
///
/// We use ±25% of the base backoff, derived from `clock_ms % (base/4 + 1)`.
/// The `+ 1` avoids a divide-by-zero when `base_ms == 0`. The modulus is
/// not cryptographically random; that's deliberate — see module docs.
pub(crate) fn jitter_ms(clock_ms: u64, base_ms: u64) -> u64 {
    let span = base_ms / 4 + 1;
    clock_ms % span
}

/// Retry `op` according to `policy`, sleeping with `worker::Delay` between
/// attempts. Only [`Error::Transient`] is retried; everything else is
/// returned immediately.
///
/// This is the production entry point — wired to `WorkerClock` and
/// `WorkerSleeper`. For unit tests use [`with_retry_clock`].
pub async fn with_retry<F, Fut, T>(policy: &RetryPolicy, op: F) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Error>>,
{
    with_retry_clock(policy, &WorkerClock, &WorkerSleeper, op).await
}

/// Variant of [`with_retry`] parameterized over the clock and sleeper.
/// Production code should call [`with_retry`]; this exists so unit tests can
/// count attempts and verify backoff without depending on `worker::*`.
pub async fn with_retry_clock<C, S, F, Fut, T>(
    policy: &RetryPolicy,
    clock: &C,
    sleeper: &S,
    mut op: F,
) -> Result<T, Error>
where
    C: MonotonicClock,
    S: Sleeper,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Error>>,
{
    // Guard against `max_attempts = 0` configs slipping in — treat as 1.
    let max = policy.max_attempts.max(1);
    let mut last_err: Option<Error> = None;

    for attempt in 0..max {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) if !e.is_transient() => return Err(e),
            Err(e) => {
                // Stash the transient error so we can return it if we
                // exhaust the budget; we'll overwrite on each retry.
                last_err = Some(e);

                // Don't sleep after the *last* attempt — there's no next
                // try to wait for.
                if attempt + 1 == max {
                    break;
                }

                let base = policy.backoff_for(attempt);
                let base_ms = base.as_millis() as u64;
                let total = base_ms + jitter_ms(clock.now_ms(), base_ms);
                sleeper.sleep(Duration::from_millis(total)).await;
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        // Unreachable: the loop always assigns last_err on the Err arm.
        // Keep an explicit fallback so the type checker doesn't need
        // `unreachable!()` panics in a wasm-targeted crate.
        Error::Internal("with_retry: no attempts executed".into())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    /// Fake clock with a controllable counter.
    struct FakeClock(Cell<u64>);
    impl MonotonicClock for FakeClock {
        fn now_ms(&self) -> u64 {
            let n = self.0.get();
            self.0.set(n + 1);
            n
        }
    }

    /// Records every sleep duration, doesn't actually wait.
    struct RecordingSleeper(Rc<Cell<Vec<u64>>>);
    #[async_trait::async_trait(?Send)]
    impl Sleeper for RecordingSleeper {
        async fn sleep(&self, dur: Duration) {
            let mut v = self.0.take();
            v.push(dur.as_millis() as u64);
            self.0.set(v);
        }
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        // Tiny single-threaded runner — sufficient because none of the
        // futures in this crate's tests touch a reactor. We can't pull
        // `futures-executor` because the workspace toolchain forbids
        // adding deps; this hand-rolled one is ~10 lines.
        use std::pin::Pin;
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};
        struct Noop;
        impl Wake for Noop {
            fn wake(self: Arc<Self>) {}
        }
        let waker = Waker::from(Arc::new(Noop));
        let mut cx = Context::from_waker(&waker);
        let mut f = Box::pin(f);
        loop {
            match Pin::new(&mut f).as_mut().poll(&mut cx) {
                Poll::Ready(v) => return v,
                Poll::Pending => {} // our futures don't actually yield
            }
        }
    }

    #[test]
    fn backoff_doubles_then_caps() {
        let p = RetryPolicy {
            max_attempts: 6,
            initial_backoff_ms: 10,
            max_backoff_ms: 100,
        };
        assert_eq!(p.backoff_for(0), Duration::from_millis(10));
        assert_eq!(p.backoff_for(1), Duration::from_millis(20));
        assert_eq!(p.backoff_for(2), Duration::from_millis(40));
        assert_eq!(p.backoff_for(3), Duration::from_millis(80));
        // Next doubling would be 160 — cap at 100.
        assert_eq!(p.backoff_for(4), Duration::from_millis(100));
        assert_eq!(p.backoff_for(5), Duration::from_millis(100));
    }

    #[test]
    fn backoff_does_not_overflow_at_extreme_attempt_counts() {
        // 1ms << 64 would overflow u64. We use checked_shl into u128 then
        // clamp; expected behavior is to land at the cap, not panic.
        let p = RetryPolicy {
            max_attempts: 100,
            initial_backoff_ms: 1,
            max_backoff_ms: 500,
        };
        assert_eq!(p.backoff_for(64), Duration::from_millis(500));
        assert_eq!(p.backoff_for(99), Duration::from_millis(500));
    }

    #[test]
    fn jitter_is_bounded_within_25_percent() {
        // jitter_ms must never exceed base_ms / 4. Sweep a wide clock
        // range to be sure (this is exactly the property that keeps the
        // worst-case total wait predictable).
        for base in [10u64, 100, 1000, 10_000] {
            for clock in [0u64, 1, 7, 13, 999, u64::MAX] {
                let j = jitter_ms(clock, base);
                assert!(
                    j <= base / 4,
                    "jitter {j} exceeded cap {} for base {base} clock {clock}",
                    base / 4
                );
            }
        }
    }

    #[test]
    fn jitter_handles_zero_base() {
        // base_ms == 0 -> span is 1 -> jitter is always 0. Important
        // because RetryPolicy::none() has initial_backoff_ms = 0.
        assert_eq!(jitter_ms(123, 0), 0);
        assert_eq!(jitter_ms(0, 0), 0);
    }

    #[test]
    fn retry_stops_after_max_attempts_on_transient() {
        let calls = Rc::new(Cell::new(0u32));
        let sleeps = Rc::new(Cell::new(Vec::<u64>::new()));
        let clock = FakeClock(Cell::new(0));
        let sleeper = RecordingSleeper(sleeps.clone());
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 10,
            max_backoff_ms: 80,
        };

        let calls_inner = calls.clone();
        let res: Result<(), Error> = block_on(with_retry_clock(
            &policy,
            &clock,
            &sleeper,
            move || {
                let c = calls_inner.clone();
                async move {
                    c.set(c.get() + 1);
                    Err::<(), _>(Error::Transient("D1 busy".into()))
                }
            },
        ));

        assert!(matches!(res, Err(Error::Transient(_))));
        assert_eq!(calls.get(), 3, "expected exactly max_attempts calls");
        // 2 sleeps between 3 attempts.
        assert_eq!(sleeps.take().len(), 2);
    }

    #[test]
    fn retry_does_not_retry_permanent() {
        let calls = Rc::new(Cell::new(0u32));
        let sleeps = Rc::new(Cell::new(Vec::<u64>::new()));
        let clock = FakeClock(Cell::new(0));
        let sleeper = RecordingSleeper(sleeps.clone());
        let policy = RetryPolicy::default_for_storage();

        let calls_inner = calls.clone();
        let res: Result<(), Error> = block_on(with_retry_clock(
            &policy,
            &clock,
            &sleeper,
            move || {
                let c = calls_inner.clone();
                async move {
                    c.set(c.get() + 1);
                    Err::<(), _>(Error::Permanent("nope".into()))
                }
            },
        ));

        assert!(matches!(res, Err(Error::Permanent(_))));
        assert_eq!(calls.get(), 1, "permanent error must not retry");
        assert!(sleeps.take().is_empty(), "no sleeps for permanent");
    }

    #[test]
    fn retry_returns_ok_on_second_attempt() {
        // Common case: first call is transient, second succeeds. Verify
        // we return the success and don't continue to attempt 3.
        let calls = Rc::new(Cell::new(0u32));
        let sleeps = Rc::new(Cell::new(Vec::<u64>::new()));
        let clock = FakeClock(Cell::new(0));
        let sleeper = RecordingSleeper(sleeps.clone());
        let policy = RetryPolicy {
            max_attempts: 5,
            initial_backoff_ms: 1,
            max_backoff_ms: 10,
        };

        let calls_inner = calls.clone();
        let res: Result<&'static str, Error> = block_on(with_retry_clock(
            &policy,
            &clock,
            &sleeper,
            move || {
                let c = calls_inner.clone();
                async move {
                    let n = c.get() + 1;
                    c.set(n);
                    if n == 1 {
                        Err(Error::Transient("blip".into()))
                    } else {
                        Ok("ok")
                    }
                }
            },
        ));

        assert_eq!(res.unwrap(), "ok");
        assert_eq!(calls.get(), 2);
        assert_eq!(sleeps.take().len(), 1);
    }

    #[test]
    fn retry_with_max_attempts_one_never_sleeps() {
        let calls = Rc::new(Cell::new(0u32));
        let sleeps = Rc::new(Cell::new(Vec::<u64>::new()));
        let clock = FakeClock(Cell::new(0));
        let sleeper = RecordingSleeper(sleeps.clone());
        let policy = RetryPolicy::none();

        let calls_inner = calls.clone();
        let _: Result<(), Error> = block_on(with_retry_clock(
            &policy,
            &clock,
            &sleeper,
            move || {
                let c = calls_inner.clone();
                async move {
                    c.set(c.get() + 1);
                    Err::<(), _>(Error::Transient("once".into()))
                }
            },
        ));

        assert_eq!(calls.get(), 1);
        assert!(sleeps.take().is_empty());
    }

    #[test]
    fn retry_total_wait_is_bounded_by_policy() {
        // Worst-case: every attempt fails transiently. Sum of recorded
        // sleeps must be <= sum of (capped backoff + max jitter) for
        // (max_attempts - 1) gaps. Locks down the bound the docstring
        // promises so a future "improvement" doesn't blow up our CPU
        // budget.
        let sleeps = Rc::new(Cell::new(Vec::<u64>::new()));
        let clock = FakeClock(Cell::new(0));
        let sleeper = RecordingSleeper(sleeps.clone());
        let policy = RetryPolicy {
            max_attempts: 4,
            initial_backoff_ms: 50,
            max_backoff_ms: 200,
        };

        let _: Result<(), Error> = block_on(with_retry_clock(
            &policy,
            &clock,
            &sleeper,
            || async { Err::<(), _>(Error::Transient("x".into())) },
        ));

        // backoff_for(0..3): 50, 100, 200 (200 doubled would be 200 capped).
        // jitter per step <= base/4.
        let recorded = sleeps.take();
        assert_eq!(recorded.len(), 3);
        let max_each = [50 + 50 / 4, 100 + 100 / 4, 200 + 200 / 4];
        for (i, ms) in recorded.iter().enumerate() {
            assert!(
                *ms <= max_each[i],
                "step {i}: slept {ms}ms, expected <= {}ms",
                max_each[i]
            );
        }
    }
}
