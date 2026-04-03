//! Healthy Path Integration Test for Lobby EVM Transaction Service.
//!
//! This test:
//! 1. Starts PostgreSQL, Redis, and 3 Anvil nodes (Ethereum, Polygon, Arbitrum)
//! 2. Funds 5 test accounts with 100 ETH each
//! 3. Sends 100 transactions within 1 second
//! 4. Polls for transaction status until all reach final state
//! 5. Reports success rate and timing statistics
//!
//! The test passes only if 100% of transactions reach ConfirmedOnChain status.

use crate::{
    containers::TestContainers,
    helpers::{
        TransactionSubmission, build_api_registry, build_transaction_params, is_success_status,
        load_test_account, poll_transaction_status, rpc_provider_stack_build,
        select_random_accounts, select_random_chain, send_transaction,
    },
};
use alloy::primitives::Address;
use axum::{
    Router, middleware,
    routing::{get, post},
};
use cortex::{artifacts::CortexConfig, spawn_cortex};
use lobby::{
    AppState,
    auth::auth_middleware,
    handler::{get_transaction_status, submit_transaction},
    scanner::spawn_scanner_bot,
    spawn_sweeper_bot,
};
use primitives::types::PipelineStatus;
use std::{
    fs::OpenOptions,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::Mutex, task::JoinSet};
use tracing::{Level, info};
use tracing_subscriber::{EnvFilter, Registry, fmt, layer::SubscriberExt, util::SubscriberInitExt};

mod containers;
mod helpers;

// ============================================================

const TRANSACTION_COUNT: usize = 1000;
// const SUBMISSION_DEADLINE_MS: u64 = 1000;
const POLL_TIMEOUT_SECS: u64 = 60;
const POLL_INTERVAL_MS: u64 = 50;

/// Test result statistics.
#[derive(Debug)]
struct TestResults {
    total_transactions: usize,
    successful: usize,
    failed: usize,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 16)]
async fn pipeline_test() -> Result<(), Box<dyn std::error::Error>> {
    // ============================================================
    // test state requirements

    // initialize tracing subscriber for structured logging

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("transactions.log")
        .expect("Failed to open log file");

    let file_layer = fmt::layer()
        .with_writer(file)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true);

    let stdout_layer = fmt::layer().with_ansi(true).with_target(true);

    Registry::default()
        .with(EnvFilter::from_default_env().add_directive(Level::INFO.into()))
        .with(file_layer)
        .with(stdout_layer)
        .init();
    info!("starting pipeline test");

    // container setup
    let containers = TestContainers::start().await?;
    info!("all containers started succesfully");

    // schema migrations
    let db_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(&containers.postgres_endpoint)
        .await?;

    sqlx::migrate!("../database/migrations")
        .run(&db_pool)
        .await?;

    // initiate test accounts
    let test_accounts = load_test_account()?;
    let test_addresses: Vec<Address> = test_accounts
        .iter()
        .map(|account| account.address)
        .collect();

    containers.fund_test_accounts(&test_addresses, 100).await?;
    info!("test accounts funded with 100 eth");

    // ============================================================
    // lobby server

    // rpc_registry
    let rpc_provider_stack = rpc_provider_stack_build(containers.anvil_endpoints.clone())?;

    // fetch api_keys and build api_registry
    let (api_registry, api_keys) = build_api_registry()?;

    // spawn cortex (requires unsafe env::set_var for status registry)

    // SAFETY: This is safe because:
    // 1. We are single-threaded at this point (no other tasks running)
    // 2. No other code is reading environment variables concurrently
    // 3. This happens before cortex spawns any internal threads
    unsafe {
        std::env::set_var("REDIS_URL", &containers.redis_endpoint);
    }

    let cortex_config = CortexConfig::from_env()?;
    let contex_handler =
        spawn_cortex(db_pool.clone(), rpc_provider_stack.clone(), cortex_config).await;
    let status_registry = contex_handler.status_registry();

    // Spawn background bots
    spawn_sweeper_bot(db_pool.clone());
    spawn_scanner_bot(
        db_pool.clone(),
        status_registry.clone(),
        rpc_provider_stack.validator_registry.clone(),
    );

    // Build app state
    let state = AppState::new(api_registry, contex_handler, status_registry);
    // Build router
    let app: Router = Router::new()
        .route("/v1/transactions", post(submit_transaction))
        .route("/status/{execution_id}", get(get_transaction_status))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));
    info!("lobby app-router installed");

    let listner = tokio::net::TcpListener::bind("0.0.0.0:3000".to_string()).await?;
    let base_url = format!("http://127.0.0.1:3000");

    let server_handle = tokio::spawn(async move {
        axum::serve(listner, app).await.unwrap();
    });

    // Give the server a moment to start
    tokio::time::sleep(Duration::from_millis(1000)).await;
    info!(lobby_server_url = %base_url, "Lobby server running");

    // ============================================================
    // build and submit transactions

    // submitting transactions
    let client = reqwest::Client::new();
    let submissions = Arc::new(Mutex::new(Vec::new()));
    let mut joinset = JoinSet::new();

    let submission_start = Instant::now();

    for i in 0..TRANSACTION_COUNT {
        let client = client.clone();
        let base_url = base_url.clone();
        let accounts = test_accounts.clone();
        let chain_ids = vec![1, 137, 42161];
        let submissions = submissions.clone();
        let api_keys = api_keys.clone();

        joinset.spawn(async move {
            // Select random from/to accounts and chain
            let (from_address, to_address) = select_random_accounts(&accounts);
            let chain_id = select_random_chain(&chain_ids);

            // Build transaction parameters
            let params =
                build_transaction_params(from_address.address, to_address.address, chain_id, 0.1);

            // submit transaction
            let api_key = api_keys.get(&from_address.address).unwrap().clone();
            match send_transaction(&client, &base_url, api_key, params).await {
                Ok(execution_id) => {
                    let submission = TransactionSubmission {
                        execution_id,
                        from_address: from_address.address,
                        chain_id,
                    };
                    submissions.lock().await.push(submission);
                }
                Err(e) => {
                    tracing::info!("transaction {} failed to submit: {}", i, e);
                }
            }
        });
    }

    // await all submissions
    while let Some(result) = joinset.join_next().await {
        if let Err(e) = result {
            tracing::info!("submission task panicked: {}", e);
        }
    }

    let submission_duration = submission_start.elapsed();
    let actual_submissions = submissions.lock().await.len();
    info!(
        "{}/{} transactions submitted in {:?}",
        actual_submissions, TRANSACTION_COUNT, submission_duration
    );

    assert_eq!(
        actual_submissions, TRANSACTION_COUNT,
        "Only {}/{} transactions were submitted successfully",
        actual_submissions, TRANSACTION_COUNT
    );

    // verify all tranasctions were submitted
    assert_eq!(actual_submissions, TRANSACTION_COUNT);

    // ============================================================
    // poll for transaction status

    let submissions_list = submissions.lock().await.clone();
    let results = Arc::new(Mutex::new(Vec::new()));
    let mut poll_joinset = JoinSet::new();

    for submission in submissions_list {
        let client = client.clone();
        let base_url = base_url.clone();
        let results = results.clone();
        let api_keys = api_keys.clone();

        poll_joinset.spawn(async move {
            let api_key = api_keys.get(&submission.from_address).unwrap().clone();
            match poll_transaction_status(
                &client,
                &base_url,
                submission.execution_id,
                api_key,
                Duration::from_secs(POLL_TIMEOUT_SECS),
                Duration::from_millis(POLL_INTERVAL_MS),
            )
            .await
            {
                Ok((status, _)) => {
                    let success = is_success_status(&status);
                    results.lock().await.push((submission, status, success));
                }
                Err(_) => {
                    tracing::info!(
                        "Polling timeout exceeded for id: {} on chain: {}",
                        submission.execution_id,
                        submission.chain_id
                    );
                    results.lock().await.push((
                        submission,
                        PipelineStatus::Failed {
                            stage: "polling".to_string(),
                            reason: "Polling timeout exceeded for id".to_string(),
                        },
                        false,
                    ));
                }
            }
        });
    }

    // Wait for all polling to complete
    while let Some(result) = poll_joinset.join_next().await {
        if let Err(e) = result {
            tracing::info!("Polling task panicked: {}", e);
        }
    }

    // ============================================================
    // calculating statistics

    let results_list = results.lock().await.clone();
    let test_results = calculate_statistics(&results_list);

    print_results(&test_results);

    let success_rate =
        (test_results.successful as f64 / test_results.total_transactions as f64) * 100.0;
    tracing::info!("\n SUCCESS RATE: {:.2}%", success_rate);
    assert_eq!(
        test_results.successful, TRANSACTION_COUNT,
        "Test failed: Only {}/{} transactions succeeded ({:.2}%)",
        test_results.successful, TRANSACTION_COUNT, success_rate
    );

    // Cleanup
    drop(server_handle);
    Ok(())
}

// ============================================================
// statistics processing helper

/// Calculate statistics from test results.
fn calculate_statistics(results: &[(TransactionSubmission, PipelineStatus, bool)]) -> TestResults {
    let total = results.len();
    let successful = results.iter().filter(|(_, _, s)| *s).count();
    let failed = total - successful;

    TestResults {
        total_transactions: total,
        successful,
        failed,
    }
}

// ============================================================
// preety formatting of results

/// Print test results in a formatted table.
fn print_results(results: &TestResults) {
    tracing::info!("╔══════════════════════════════════════════════════════════════╗");
    tracing::info!("║           HEALTHY PATH TEST - RESULTS REPORT                 ║");
    tracing::info!("╠══════════════════════════════════════════════════════════════╣");
    tracing::info!(
        "║ Total Transactions:  {:>39} ║",
        results.total_transactions
    );
    tracing::info!("║ Successful:          {:>39} ║", results.successful);
    tracing::info!("║ Failed:              {:>39} ║", results.failed);
    tracing::info!(
        "║ Success Rate:        {:>38.2}% ║",
        (results.successful as f64 / results.total_transactions as f64) * 100.0
    );
    tracing::info!("╚══════════════════════════════════════════════════════════════╝");
}

// ============================================================
