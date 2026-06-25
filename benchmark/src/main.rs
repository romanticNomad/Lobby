#[allow(dead_code, unused_imports)]
mod infra;
#[allow(dead_code, unused_imports)]
mod loadgen;
#[allow(dead_code)]
mod metrics;
#[allow(dead_code)]
mod mockrpc;

use crate::infra::InfraStack;
use crate::loadgen::{DynamicRateController, Payloads, TxTrigger, build_apistack, get_addresses, write_test_keys_json, run_load_generator};
use crate::mockrpc::RpcAppState;
use anyhow::{Ok, Result};
use reqwest::Client;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

// =============================================================================
// constants
const RAMP_SECS: f64 = 5.0;
const TARGET_TPS: f64 = 1_000.0;
const INITIAL_DELAY_US: f64 = 10_000.0;
const BASE_URL: &str = "http://localhost:3000/v1/transactions";
const BENCH_DURATION: Duration = Duration::from_secs(60);

/// `Little's Law`: L = k * λ * W, where
/// * `λ` = throughput (transaction per second) => eg: 1_000
/// * `W` = expected latency => eg: 1 ms
/// * `k` = correction term
const WORKER_THREADS: usize = 3;

// =============================================================================
// bench boot sequence

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Set up Postgres and Redis

    let infra_stack = InfraStack::build().await?;
    let _pg_pool = infra_stack.get_pool();

    // 2. Build test-keys and api-keys

    write_test_keys_json(100)?;
    let path_test_keys = Path::new("test_keys.json");
    let api_stack = build_apistack(path_test_keys)?;

    // 3. Spawn mockrpc_servers

    let cancelation_token = CancellationToken::new();
    let chain_ids = vec![1, 137, 560048];
    let addresses = get_addresses(&api_stack)?;
    let app_state = RpcAppState::new(chain_ids.clone(), addresses);
    app_state
        .spawn_mockrpc_servers(cancelation_token.clone())
        .await?;

    // 4. Inititate tokio::process for lobby
    // 4.1 Build env variables
    // 4.2 Launch lobby binary with "benchmark-telemetry" feature flag

    // 5. Start benchark with loadgen::TxTrigger
    let rate_controller = Arc::new(DynamicRateController::new(
        RAMP_SECS,
        TARGET_TPS,
        INITIAL_DELAY_US,
    ));
    let payloads = Payloads::build_payloads(&api_stack, &chain_ids);
    let client = Client::new();
    let base_url = String::from(BASE_URL);
    let tx_trigger = TxTrigger::new(BENCH_DURATION, payloads, client, base_url, rate_controller);

    run_load_generator(tx_trigger, WORKER_THREADS, cancelation_token.clone()).await;

    // 6. Initiate telemetry_stream_reader task

    // 7. Shutdown after a minute
    // 7.1 tokio::process -> for Lobby
    // 7.2 CancelationToken -> for {mock_rpc, telemetry_stream_reader}
    // 7.3 teardown.await -> test-containers.

    // 8. report() recorded latencies.

    Ok(())
}
