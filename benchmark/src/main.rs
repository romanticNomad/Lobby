mod infra;
mod loadgen;
mod metrics;
mod mockrpc;
#[cfg(test)]
mod testbench;

use crate::infra::InfraStack;
use crate::loadgen::{
    DynamicRateController, Payloads, TxTrigger, build_apistack, get_addresses, run_load_generator,
    write_test_keys_json,
};
use crate::metrics::telemetry_stream_reader;
use crate::mockrpc::RpcAppState;
use anyhow::{Ok, Result};
use reqwest::Client;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

// ===========================================================
// constants

const RAMP_SECS: f64 = 5.0;
const TARGET_TPS: f64 = 1_000.0;
const INITIAL_DELAY_US: f64 = 10_000.0;
const STEADY_SEC: f64 = 55.0;
const BENCH_DURATION: Duration = Duration::from_secs(60);
const BASE_URL: &str = "http://localhost:3000/v1/transactions";
const UDS_SOCKET: &str = "/tmp/lobby_benchmark_telemetry.sock";

/// `Little's Law`: L = k * λ * W, where
/// * `λ` = throughput (transaction per second) => eg: 1_000
/// * `W` = expected latency => eg: 1 ms
/// * `k` = correction term
const WORKER_THREADS: usize = 3;

// ===========================================================
// bench-harness boot sequence

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    // 0. Cancellation Token

    let cancelation_token = CancellationToken::new();

    // 1. Set up Postgres and Redis

    let infra_stack = InfraStack::build().await?;
    let _pg_pool = infra_stack.get_pool();

    // 2. Build test-keys and api-keys

    write_test_keys_json(100)?;
    let path_test_keys = Path::new("test_keys.json");
    let api_stack = build_apistack(path_test_keys)?;

    // 3. Spawn mockrpc_servers

    let chain_ids = vec![1, 137, 560048];
    let addresses = get_addresses(&api_stack)?;
    let app_state = RpcAppState::new(chain_ids.clone(), addresses);
    app_state
        .spawn_mockrpc_servers(cancelation_token.clone())
        .await?;

    // 4. Inititate tokio::process for lobby

    // 5. Initiate load generator and UDS stream reader.
    let rate_controller = Arc::new(DynamicRateController::new(
        RAMP_SECS,
        TARGET_TPS,
        INITIAL_DELAY_US,
    ));
    let payloads = Payloads::build_payloads(api_stack.clone(), chain_ids);
    let client = Client::new();
    let base_url = String::from(BASE_URL);
    let tx_trigger = TxTrigger::new(BENCH_DURATION, payloads, client, base_url, rate_controller);
    let warmup = Duration::from_secs_f64(RAMP_SECS);
    let steady_state = Duration::from_secs_f64(STEADY_SEC);
    let start_instant = Instant::now();

    let (_, metrics_collector_result) = tokio::join!(
        run_load_generator(
            start_instant.clone(),
            tx_trigger.clone(),
            WORKER_THREADS,
            cancelation_token.clone()
        ),
        telemetry_stream_reader(
            start_instant.clone(),
            UDS_SOCKET,
            warmup,
            steady_state,
            cancelation_token.clone()
        )
    );

    let collected_metrics = metrics_collector_result?;

    // 7. gracefull shutdowm

    loop {
        let now = Instant::now();
        if now.elapsed() > BENCH_DURATION {
            cancelation_token.cancel();
            break;
        }
    }

    // 8. report latancies

    collected_metrics.report();

    Ok(())
}

// ===========================================================
