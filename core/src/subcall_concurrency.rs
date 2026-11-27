//! Subcall fanout concurrency gate (spec §10 Risk #2).
//!
//! The subcall primitive lets callers fan out multiple subcalls in
//! parallel.  Each
//! subcall runs a bounded ReAct loop that issues its own LLM completions
//! against OpenRouter.  `core/src/llm_client.rs` already configures
//! `async_openai::Client::with_backoff` (2s → 4s → 8s → 16s retry on 429/5xx,
//! 30s elapsed cap), **but it does not bound concurrency**.  Without an
//! explicit cap, a naive fanout of N subcalls would open N simultaneous
//! `chat().create()` streams against a single OpenRouter account — a good
//! way to trip per-account RPM ceilings and cascade into retry storms.
//!
//! This module exposes a process-global concurrency limiter sized to
//! [`crate::subcall::MAX_CONCURRENT_SUBCALLS`].  Every fanout site that
//! launches `run_subcall` in parallel MUST call [`acquire_subcall_permit`]
//! and hold the returned permit across the `run_subcall` await.  The
//! `OwnedSemaphorePermit` type is `'static`, so it can be moved into a
//! spawned task or returned from an async function without lifetime
//! plumbing.
//!
//! Non-subcall LLM callers (top-level `orchestrator::run_react_loop`,
//! `heartbeat`, etc.) are intentionally unaffected — they have their own
//! natural pacing (one user turn, one heartbeat tick) and bounding them
//! globally would throttle interactive flows.

use std::sync::{Arc, OnceLock};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::subcall::MAX_CONCURRENT_SUBCALLS;

/// Process-global singleton.  `std::sync::OnceLock` (synchronous) is a good
/// fit here because the initialiser (`Arc::new(Semaphore::new(N))`) never
/// blocks and does not need async context.  Matches the pattern already
/// used in `llm_client.rs` for `LLM_LOG_TX`.
static SUBCALL_SEMAPHORE: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn semaphore() -> &'static Arc<Semaphore> {
    SUBCALL_SEMAPHORE.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_SUBCALLS)))
}

/// Acquire a permit for launching one subcall.  Holds at most
/// [`MAX_CONCURRENT_SUBCALLS`] permits concurrently across the entire
/// process; callers that need more wait on the returned future.
///
/// The returned [`OwnedSemaphorePermit`] releases the slot on drop.
/// Callers should bind it to a local (not `_`) and hold it across the
/// `run_subcall` await:
///
/// ```ignore
/// let _permit = acquire_subcall_permit().await;
/// let result = run_subcall(spec, orchestrator, memory, ctx).await;
/// // permit released here
/// ```
///
/// Failure path: the underlying semaphore is never closed in this
/// codebase, so `.acquire_owned()` cannot error in practice.  We unwrap
/// with a clear panic rather than propagate an error the caller cannot
/// meaningfully handle.
pub async fn acquire_subcall_permit() -> OwnedSemaphorePermit {
    semaphore()
        .clone()
        .acquire_owned()
        .await
        .expect("subcall concurrency semaphore was closed — this is a bug")
}

/// Test-only accessor — returns the number of currently available permits.
/// Used by the concurrency-cap test to observe the gate without racing the
/// in-flight counter.
#[cfg(test)]
pub fn available_permits() -> usize {
    semaphore().available_permits()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Mutex as TokioMutex;

    /// Both tests below exercise the process-global semaphore
    /// [`SUBCALL_SEMAPHORE`], which is shared across the test binary.
    /// `cargo test` runs tests in parallel by default, so we gate them
    /// behind a test-only async mutex to prevent one test's outstanding
    /// permits from being observed by another.  `tokio::sync::Mutex` is
    /// the right choice here because the guard is held across `.await`
    /// points — a `std::sync::Mutex` would trip `clippy::await_holding_lock`.
    static TEST_LOCK: TokioMutex<()> = TokioMutex::const_new(());

    /// Spec §10 Risk #2 acceptance test: parallel fanout respects the
    /// global concurrency cap even when many more tasks than the cap try
    /// to acquire at the same moment.
    #[tokio::test]
    async fn subcall_fanout_respects_concurrency_cap() {
        let _guard = TEST_LOCK.lock().await;

        // Counts peak concurrent holders of a permit.  Each task bumps on
        // acquire, sleeps 50ms, decrements on drop — so if the cap is
        // honoured the peak can never exceed MAX_CONCURRENT_SUBCALLS.
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..10 {
            let in_flight = in_flight.clone();
            let peak = peak.clone();
            handles.push(tokio::spawn(async move {
                let _permit = acquire_subcall_permit().await;

                // Record the new in-flight count, then update peak with a
                // monotonic-max compare-exchange loop so no concurrent
                // bumper can clobber a higher observation.
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                let mut cur_peak = peak.load(Ordering::SeqCst);
                while now > cur_peak {
                    match peak.compare_exchange(cur_peak, now, Ordering::SeqCst, Ordering::SeqCst) {
                        Ok(_) => break,
                        Err(p) => cur_peak = p,
                    }
                }

                tokio::time::sleep(Duration::from_millis(50)).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                // `_permit` drops here, releasing the slot.
            }));
        }

        for h in handles {
            h.await.expect("task panicked");
        }

        let observed_peak = peak.load(Ordering::SeqCst);
        assert!(
            observed_peak <= MAX_CONCURRENT_SUBCALLS,
            "peak in-flight ({observed_peak}) exceeded cap ({MAX_CONCURRENT_SUBCALLS})"
        );

        // Sanity-check the fanout actually exercised the gate rather than
        // accidentally serialising (which would also satisfy the <= check).
        assert!(
            observed_peak >= 2,
            "peak in-flight ({observed_peak}) too low — test didn't exercise parallelism"
        );

        // After every task completes the gate must be fully released.
        assert_eq!(
            available_permits(),
            MAX_CONCURRENT_SUBCALLS,
            "semaphore leaked permits — some task forgot to drop its permit"
        );
    }

    /// Holding a permit across an await must actually serialise peer
    /// acquisitions — guards against regressions where someone swaps
    /// `acquire_owned()` for a non-blocking variant.
    #[tokio::test]
    async fn acquire_blocks_when_cap_reached() {
        let _guard = TEST_LOCK.lock().await;

        // Drain the semaphore.
        let mut held = Vec::new();
        for _ in 0..MAX_CONCURRENT_SUBCALLS {
            held.push(acquire_subcall_permit().await);
        }
        assert_eq!(available_permits(), 0);

        // Try to acquire one more with a short timeout — must time out.
        let timeout =
            tokio::time::timeout(Duration::from_millis(20), acquire_subcall_permit()).await;
        assert!(
            timeout.is_err(),
            "acquire_subcall_permit did not block when cap was reached"
        );

        // Release one, now a fresh acquire must succeed quickly.
        drop(held.pop());
        let timeout =
            tokio::time::timeout(Duration::from_millis(100), acquire_subcall_permit()).await;
        assert!(
            timeout.is_ok(),
            "acquire_subcall_permit should unblock once a permit is released"
        );
    }
}
