use rand::Rng;
use std::time::Duration;
use tracing::Instrument;
use crate::artifacts::RetryConfig;

// ============================================================
// setting up retry facility

/// Execute `operation` up to `config.max_attempts + 1` times, sleeping a
/// full-jitter exponential back-off between failures.
///
/// # Full-jitter formula
/// window = min(max_delay, base_delay * 2^attempt)
/// sleep  = rand(0, window)          // uniform over [0, window)
///
/// This is George Neville-Neil / AWS's recommended strategy for distributed
/// systems: it eliminates correlated retry spikes better than decorrelated
/// jitter while keeping average wait times low.
///
/// # Tracing
/// Every retry emits a `WARN` event with structured fields:
/// - `stage`   — the label provided by the caller (e.g. `"nonce_reserve"`)
/// - `attempt` — 1-based attempt number
/// - `delay_ms`— the actual sleep chosen
/// - `error`   — the `Display` form of the failure
///
/// Final failure emits an `ERROR` event.
pub async fn retry_with_backoff<F, Fut, T, E>(
    config: &RetryConfig,
    stage: &'static str,
    mut operation: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
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

            Err(err) if attempt == max_total => {
                tracing::error!(
                    stage,
                    attempt,
                    %err,
                    "all attempts exhausted, shutting down"
                );
                return Err(err);
            }

            Err(err) => {
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
