mod keys;
mod trigger;

// ============================================================
// re-exports

pub use keys::{build_apistack, get_addresses, write_test_keys_json};
use std::time::Instant;
pub use trigger::{DynamicRateController, Payloads, RECIPIENT_ADDRESS, TxTrigger};

// ============================================================
// orchestrator

/// Orchestrates the concurrent dispatch of transactions across a dispatch workers
pub async fn run_load_generator(
    start_instant: Instant,
    tx_trigger: TxTrigger,
    worker_threads: usize,
) {
    let mut handles = Vec::with_capacity(worker_threads);

    // spawn worker handles
    for worker_id in 0..worker_threads {
        let start = start_instant.clone();
        let duration = tx_trigger.duration();
        let trigger = tx_trigger.clone();

        let handle = tokio::spawn(async move {
            let mut local_dispatches = 0u64;
            loop {
                // Check for benchmark timelimit
                if start.elapsed() >= duration {
                    break;
                }

                // Synchronously wait for the exact inter-arrival slot.
                trigger.wait_for_next_slot(start).await;

                // Spawn the I/O task.
                let trigger_clone = trigger.clone();
                tokio::spawn(async move {
                    if let Err(e) = trigger_clone.execute_dispatch().await {
                        tracing::warn!("Dispatch failed: {}", e);
                    }
                });

                local_dispatches += 1;
            }

            tracing::info!(local_dispatches, "worker{} shut down", worker_id);
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
