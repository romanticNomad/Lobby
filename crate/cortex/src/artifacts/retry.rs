use crate::artifacts::RetryConfig;
use rand::Rng;
use std::future::Future;
use std::time::Duration;
use tracing::Instrument;

// ============================================================
// retry decision

/// Controls whether an error should be retried or fail immediately.
///
/// This enum allows operations to signal whether a particular error is
/// transient (and should be retried with backoff) or deterministic/fatal
/// (and should abort the retry loop immediately).
///
/// ## Usage
/// Operations passed to `retry_with_backoff` should return
/// `Result<T, RetryDecision<E>>` and wrap their errors appropriately:
///
/// ```ignore
/// // Transient errors that may succeed on retry
/// Err(e) => Err(RetryDecision::Retry(e))
///
/// // Fatal errors that should not be retried
/// Err(e) => Err(RetryDecision::FailImmediately(e))
/// ```
///
/// ## Example
/// ```ignore
/// match broadcast_result {
///     Err(BroadcastError::NonceTooLow { .. }) => {
///         // Don't retry - needs special recovery logic
///         Err(RetryDecision::FailImmediately(err))
///     }
///     Err(BroadcastError::Unexpected { .. }) => {
///         // Transient - may succeed on retry
///         Err(RetryDecision::Retry(err))
///     }
///     Ok(result) => Ok(result)
/// }
/// ```
#[derive(Debug)]
pub enum RetryDecision<E> {
    /// The error is transient and should be retried with exponential backoff.
    Retry(E),
    /// The error is fatal/deterministic and should abort the retry loop immediately.
    FailImmediately(E),
}

impl<E: std::fmt::Display> std::fmt::Display for RetryDecision<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RetryDecision::Retry(e) => write!(f, "{}", e),
            RetryDecision::FailImmediately(e) => write!(f, "{}", e),
        }
    }
}

// ============================================================
// setting up retry facility

/// Execute `operation` up to `config.max_attempts + 1` times, sleeping a
/// full-jitter exponential back-off between failures.
///
/// ## Retry Decision
/// The operation must return `Result<T, RetryDecision<E>>`. Errors wrapped in:
/// - `RetryDecision::Retry(e)` — will be retried with backoff
/// - `RetryDecision::FailImmediately(e)` — abort retry loop, return immediately
///
/// ## Full-jitter formula
/// window = min(max_delay, base_delay * 2^attempt)
/// sleep  = rand(0, window)          // uniform over [0, window)
///
/// This is George Neville-Neil / AWS's recommended strategy for distributed
/// systems: it eliminates correlated retry spikes better than decorrelated
/// jitter while keeping average wait times low.
///
/// ## Tracing
/// Every retry emits a `WARN` event with structured fields:
/// - `stage`   — the label provided by the caller (e.g. `"nonce_reserve"`)
/// - `attempt` — 1-based attempt number
/// - `delay_ms`— the actual sleep chosen
/// - `error`   — the `Display` form of the failure
///
/// Final failure emits an `ERROR` event.
/// Immediate failures emit a `DEBUG` event indicating no retry was attempted.
pub async fn retry_with_backoff<F, Fut, T, E>(
    config: &RetryConfig,
    stage: &'static str,
    mut operation: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, RetryDecision<E>>>,
    E: std::fmt::Display,
{
    let max_total = config.max_attempts + 1;

    for attempt in 1..=max_total {
        let span = tracing::debug_span!("retry_attempt", stage, attempt);

        match operation().instrument(span).await {
            Ok(value) => {
                if attempt > 1 {
                    tracing::info!(stage, attempt, "stage succeded after retry");
                }
                return Ok(value);
            }

            Err(RetryDecision::FailImmediately(err)) => {
                tracing::debug!(
                    stage,
                    attempt,
                    %err,
                    "operation returned non-retryable error, failing immediately"
                );
                return Err(err);
            }

            Err(RetryDecision::Retry(err)) if attempt == max_total => {
                tracing::error!(
                    stage,
                    attempt,
                    %err,
                    "all attempts exhausted, shutting down"
                );
                return Err(err);
            }

            Err(RetryDecision::Retry(err)) => {
                let delay = jittered_delay(config.base_delay, config.max_delay, attempt);
                tracing::warn!(
                    stage,
                    attempt,
                    %err,
                    delay_ms = delay.as_millis(),
                    "attempt failed, retrying after backoff"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }

    unreachable!("retry loop exited without returning")
}

// ============================================================
// helper function for calculating jitter

/// Compute a full-jitter delay for the given attempt number (1-based).
///
/// `window = min(max_delay, base_delay * 2^(attempt-1))`
/// `delay  = rand(0, window)
fn jittered_delay(base: Duration, max: Duration, attempt: u32) -> Duration {
    // Saturating shift: cap the exponent to avoid u128 overflow on huge attempts.
    let shift = (attempt - 1).min(10) as u32;
    let window_ms = (base.as_millis() as u64)
        .saturating_mul(1u64 << shift)
        .min(max.as_millis() as u64);

    let jitter_ms = if window_ms > 0 {
        rand::thread_rng().gen_range(0..=window_ms)
    } else {
        0
    };

    Duration::from_millis(jitter_ms)
}

// ============================================================
// testing of the generic retry function:

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
    };

    #[derive(Debug)]
    pub enum TestError {
        Failuire,
        First,
    }

    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self)
        }
    }

    #[tokio::test]
    async fn retries_correct_number_of_times() {
        let cfg = RetryConfig {
            max_attempts: 2 as u32,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        };

        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = Arc::clone(&calls);

        // Always fail — should exhaust all retries.
        let result: Result<(), TestError> = retry_with_backoff(&cfg, "test_stage", || {
            let c = Arc::clone(&calls_clone);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(RetryDecision::Retry(TestError::Failuire))
            }
        })
        .await;

        assert!(result.is_err());
        // max_attempts = 2 → 3 total calls
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn succeeds_on_second_attempt() {
        let cfg = RetryConfig {
            max_attempts: 2,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        };

        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = Arc::clone(&calls);

        let result: Result<u32, TestError> = retry_with_backoff(&cfg, "test_stage", || {
            let c = Arc::clone(&calls_clone);
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err(RetryDecision::Retry(TestError::First))
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn fail_immediately() {
        let cfg = RetryConfig::default(); // default -> retries for 3 times.

        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = Arc::clone(&calls);

        let result: Result<(), TestError> = retry_with_backoff(&cfg, "test_stage", || {
            let c = Arc::clone(&calls_clone);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(RetryDecision::FailImmediately(TestError::Failuire))
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

// ============================================================
