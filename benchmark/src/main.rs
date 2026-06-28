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
use tracing::{Level, error, info};

// ===========================================================
// constants

const RAMP_SECS: f64 = 5.0;
const STEADY_SEC: f64 = 50.0;
const TRIGGER_DURATION_SEC: Duration = Duration::from_secs(55);
const TARGET_TPS: f64 = 1_000.0;
const INITIAL_DELAY_US: f64 = 10_000.0;
const BENCH_DURATION: Duration = Duration::from_secs(60);
const BASE_URL: &str = "http://localhost:3000/v1/transactions";
const UDS_SOCKET: &str = "/tmp/lobby_benchmark_telemetry.sock";

/// `Little's Law`: L = k * λ * W, where
/// * `λ` = throughput (transaction per second) => eg: 1_000
/// * `W` = expected latency => eg: 1 ms
/// * `k` = correction term
const WORKER_THREADS: usize = 10;

// ===========================================================
// bench-harness boot sequence

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    // 0. Global variables and tracing config.
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    info!("Initializing Lobby Benchmark Harness");
    let total_start = Instant::now();
    let cancellation_token = CancellationToken::new();

    // 1. Set up Postgres and Redis
    info!("Phase 1: Bootstrapping infrastructure stack (PostgreSQL & Redis via Testcontainers)");
    let infra_stack = InfraStack::build().await?;
    let (pg_url, redis_url) = infra_stack.get_urls();
    info!("Infrastructure stack online.");

    // 2. Build test-keys and api-keys
    info!("Phase 2: Generating EVM test keys and API stack");
    write_test_keys_json(100)?;
    let path_test_keys = Path::new("benchmark/test_keys.json");
    let api_stack = build_apistack(path_test_keys)?;
    info!(
        account_count = api_stack.len(),
        "API stack built successfully."
    );

    // 3. Spawn mockrpc_servers
    let chain_ids = vec![1, 137, 560048];
    info!(
        chains = ?chain_ids,
        "Phase 3: Spawning MockRPC servers for target EVM chains"
    );
    let addresses = get_addresses(&api_stack)?;
    let app_state = RpcAppState::new(chain_ids.clone(), addresses);
    let port_map = app_state
        .spawn_mockrpc_servers(cancellation_token.clone())
        .await?;
    info!(
        ports = ?port_map.inner(),
        "MockRPC servers listening and ready."
    );

    // 4.1 Build environment variables
    info!("Phase 4: Assembling environment variables for Lobby process");
    let mut env_vars = parse_env_file("benchmark/bench.env");

    // Inject Postgres and Redis ports
    env_vars.insert("DATABASE_URL".to_string(), pg_url);
    env_vars.insert("REDIS_URL".to_string(), redis_url);

    // Inject dynamic RPC endpoints
    for (chain_id, port) in port_map.inner().iter() {
        env_vars.insert(
            format!("RPC_ENDPOINT_{}", chain_id),
            format!("http://localhost:{}", port),
        );
    }

    // Inject dynamic API keys
    for entry in api_stack.iter() {
        env_vars.insert(
            format!("LOBBY_API_KEY_{}", entry.key()),
            entry.value().clone(),
        );
    }
    info!("Environment variables assembled");

    // 4.2 Start Lobby process and check health
    info!("Spawning Lobby server process");
    let mut child = tokio::process::Command::new("cargo")
        .args([
            "run",
            "--release",
            "--bin",
            "lobby",
            "--features",
            "benchmark_telemetry",
        ])
        .envs(&env_vars)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn lobby");

    info!("Polling Lobby server health on port 3000 (timeout: 90s)");
    let is_healthy = poll_till_timeout(3000, Duration::from_secs(90)).await;
    if !is_healthy {
        error!("Lobby server failed to become healthy within timeout.");
        child.kill().await?;
        return Err(anyhow::anyhow!("Lobby health check failed"));
    }
    info!("Lobby server is healthy and bound to port 3000.");

    // 5. Initiate load generator and UDS stream reader.
    info!("Phase 5: Initiating load generator and UDS telemetry stream");
    let rate_controller = Arc::new(DynamicRateController::new(
        RAMP_SECS,
        TARGET_TPS,
        INITIAL_DELAY_US,
    ));
    let payloads = Payloads::build_payloads(api_stack.clone(), chain_ids);
    let client = Client::new();
    let base_url = String::from(BASE_URL);
    let tx_trigger = TxTrigger::new(
        TRIGGER_DURATION_SEC,
        payloads,
        client,
        base_url,
        rate_controller,
    );
    let warmup = Duration::from_secs_f64(RAMP_SECS);
    let steady_state = Duration::from_secs_f64(STEADY_SEC);
    let start_instant = Instant::now();

    info!(
        ramp_secs = RAMP_SECS,
        steady_secs = STEADY_SEC,
        target_tps = TARGET_TPS,
        worker_threads = WORKER_THREADS,
        "Executing benchmark load test"
    );

    let loadgen_handle = tokio::spawn(run_load_generator(
        start_instant.clone(),
        tx_trigger.clone(),
        WORKER_THREADS,
    ));

    let uds_stream_handle = tokio::spawn(telemetry_stream_reader(
        start_instant.clone(),
        UDS_SOCKET,
        warmup,
        steady_state,
        cancellation_token.clone(),
    ));

    // wait for loadgen task to complete
    let _ = loadgen_handle.await?;

    // 6. Graceful shutdown

    // wait fo buffer time to elapse
    tokio::time::sleep(BENCH_DURATION.saturating_sub(total_start.elapsed())).await;
    info!(
        elapsed = ?start_instant.elapsed(),
        "Phase 6: Benchmark phases complete. Initiating teardown"
    );

    info!("Terminating Lobby server process");
    if let Err(e) = child.kill().await {
        error!("Failed to gracefully terminate lobby process: {}", e);
    } else {
        info!("Lobby server process terminated");
    }

    info!("Tearing down infrastructure stack and cancelling tokens");
    cancellation_token.cancel();
    infra_stack.teardown().await;
    info!("Infrastructure stack torn down.");

    // 7. Report latencies
    info!("Phase 7: Generating latency and throughput report");
    let collected_metrics = uds_stream_handle.await??;
    collected_metrics.report();

    info!(
        total_harness_time = ?total_start.elapsed(),
        "Benchmark harness finished successfully."
    );

    Ok(())
}

// ===========================================================
// helper functions

/// Parses a `.env` file into a HashMap, handling `export` prefixes and quotes.
/// This allows us to merge static and dynamic variables in memory without disk I/O.
fn parse_env_file(path_str: &str) -> HashMap<String, String> {
    info!(path = path_str, "Parsing static environment file");
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
    } else {
        error!(path = path_str, "Failed to read env file");
    }

    env_variables
}

/// Polls a TCP port until it accepts connections or times out.
/// Ensures the Axum server is fully bound before the load generator fires requests.
async fn poll_till_timeout(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    let addr = format!("127.0.0.1:{}", port);
    while start.elapsed() < timeout {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    error!(
        port = port,
        timeout = ?timeout,
        "Failed to connect to lobby server within timeout"
    );

    false
}

// ===========================================================
