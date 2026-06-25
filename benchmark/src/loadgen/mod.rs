mod keys;
mod trigger;

// ============================================================
// re-exports

pub use keys::{ApiStack, build_apistack, get_addresses, write_test_keys_json};
use std::time::Instant;
use tokio_util::sync::CancellationToken;
pub use trigger::{DynamicRateController, Payloads, RECIPIENT_ADDRESS, TxTrigger};

// ============================================================
// orchestrator

/// Orchestrates the concurrent dispatch of transactions across a calculated Tokio worker pool.
///
/// ## note:
/// Use Little's Law to determine the optimal concurrency level, preventing head-of-line
/// blocking while the shared `DynamicRateController` enforces strict aggregate throughput.
pub async fn run_load_generator(
    tx_trigger: TxTrigger,
    worker_threads: usize,
    cancellation_token: CancellationToken,
) {
    let mut handles = Vec::with_capacity(worker_threads);

    // spawn worker handles
    for worker_id in 0..worker_threads {
        let start = Instant::now();
        let duration = tx_trigger.duration();
        let trigger = tx_trigger.clone();
        let token = cancellation_token.clone();

        let handle = tokio::spawn(async move {
            let mut local_dispatches = 0u64;
            let mut local_failures = 0u64;
            loop {
                // check for gracefull shutdown
                if token.is_cancelled() || start.elapsed() >= duration {
                    break;
                }
                // send request to lobby
                match trigger.ramp_dispatch(start).await {
                    Ok(()) => local_dispatches += 1,
                    Err(e) => {
                        local_failures += 1;
                        tracing::warn!("Failed to ramp_dispatch: {}", e);
                    }
                }
            }
            tracing::info!(
                local_dispatches,
                local_failures,
                "worker{} shut down",
                worker_id
            );
        });

        handles.push(handle);
    }

    // wait for all worker to finish
    for handle in handles {
        let _ = handle.await;
    }
    tracing::info!("Load generation phase complete. All workers joined.");
}

// ============================================================
