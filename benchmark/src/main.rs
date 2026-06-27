mod infra;
mod loadgen;
mod metrics;
mod mockrpc;

use crate::{
    infra::InfraStack,
    loadgen::{
        DynamicRateController, Payloads, TxTrigger, build_apistack, get_addresses,
        run_load_generator, write_test_keys_json,
    },
    metrics::telemetry_stream_reader,
    mockrpc::RpcAppState,
};
use anyhow::Result;
use reqwest::Client;
use std::{
    collections::HashMap,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
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

    // 2. Build test-keys and api-keys

    write_test_keys_json(100)?;
    let path_test_keys = Path::new("test_keys.json");
    let api_stack = build_apistack(path_test_keys)?;

    // 3. Spawn mockrpc_servers

    let chain_ids = vec![1, 137, 560048];
    let addresses = get_addresses(&api_stack)?;
    let app_state = RpcAppState::new(chain_ids.clone(), addresses);
    let port_map = app_state
        .spawn_mockrpc_servers(cancelation_token.clone())
        .await?;

    // 4.1 Build environment variables

    let mut env_vars = parse_env_file("bench.env");
    for (chain_id, port) in port_map.inner().iter() {
        env_vars.insert(
            format!("RPC_ENDPOINT_{}", chain_id),
            format!("http://localhost:{}", port),
        );
    }
    for entry in api_stack.iter() {
        env_vars.insert(
            format!("LOBBY_API_KEY_{}", entry.key()),
            format!("{}", entry.value()),
        );
    }

    // 4.2 Start Lobby process and check health

    let mut child = tokio::process::Command::new("cargo")
        .args(["run", "--release", "--bin", "lobby"])
        .envs(env_vars)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn lobby");

    poll_till_timeout(3000, Duration::from_secs(30)).await;

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

    // 7. gracefull shutdown

    info!(
        "benchmark compelete in {:?}: initiating teardown",
        start_instant
    );
    if let Err(e) = child.kill().await {
        eprintln!("⚠️ Failed to gracefully terminate lobby process: {}", e);
    }
    cancelation_token.cancel();
    infra_stack.teardown().await;

    // 8. report latancies

    collected_metrics.report();

    Ok(())
}

// ===========================================================
// helper function

/// Parses a `.env` file into a HashMap, handling `export ` prefixes and quotes.
/// This allows us to merge static and dynamic variables in memory without disk I/O.
fn parse_env_file(path_str: &str) -> HashMap<String, String> {
    let mut env_variables = HashMap::new();
    if let Ok(env_file) = std::fs::read_to_string(&path_str) {
        for line in env_file.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim().to_string();
                let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
                env_variables.insert(k, v);
            }
        }
    }

    env_variables
}

/// Polls a TCP port until it accepts connections or times out.
/// Ensures the Axum server is fully bound before the load generator fires requests.
async fn poll_till_timeout(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    let url = format!("http://localhost:{}", port);
    while start.elapsed() < timeout {
        if tokio::net::TcpStream::connect(&url).await.is_ok() {
            info!("Connected to lobby server at port: {}", port);
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    error!(
        "faile to connect to lobby sever at port: {}, in {:?}",
        port, timeout
    );

    false
}

// ===========================================================
