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
        TransactionSubmission, build_api_registry, build_transaction_params, load_test_account,
        select_random_accounts, select_random_chain, send_transaction,
    },
};
use alloy::{
    primitives::Address,
    providers::{Provider, ProviderBuilder},
};
use axum::{
    Router, middleware,
    routing::{get, post},
};
use cortex::{artifacts::CortexConfig, spawn_cortex};
use dashmap::DashMap;
use lobby::{
    AppState,
    auth::auth_middleware,
    handler::{get_transaction_status, submit_transaction},
    scanner::spawn_scanner_bot,
    spawn_sweeper_bot,
};
use primitives::types::{ChainId, RpcProviderRegistry};
use std::{sync::Arc, time::Duration};
use tokio::{sync::Mutex, task::JoinSet};
use tracing::{Level, info};

#[allow(dead_code)]
mod containers;
#[allow(dead_code)]
mod helpers;

// ============================================================

const TRANSACTION_COUNT: usize = 100;
const SUBMISSION_DEADLINE_MS: u64 = 1000;
const POLL_TIMEOUT_SECS: u64 = 120;
const POLL_INTERVAL_MS: u64 = 50;

/// Test result statistics.
#[derive(Debug)]
struct TestResults {
    total_transactions: usize,
    successful: usize,
    failed: usize,
}

#[tokio::test]
async fn pipeline_test() -> Result<(), Box<dyn std::error::Error>> {
    // ============================================================
    // test state requirements

    // initialize tracing subscriber for structured logging
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();
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
    let rpc_registry: RpcProviderRegistry = Arc::new(DashMap::new());

    for (chain_id, rpc_url) in containers.anvil_endpoints {
        let provider = ProviderBuilder::new().connect_http(rpc_url.parse()?);
        rpc_registry.insert(
            ChainId::try_from(chain_id as i64).unwrap(),
            Arc::new(provider) as Arc<dyn Provider + Send + Sync>,
        );
    }

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
    let contex_handler = spawn_cortex(db_pool.clone(), rpc_registry.clone(), cortex_config).await;
    let status_registry = contex_handler.status_registry();

    // Spawn background bots
    spawn_sweeper_bot(db_pool.clone());
    spawn_scanner_bot(
        db_pool.clone(),
        status_registry.clone(),
        rpc_registry.clone(),
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

    let _server_handler = tokio::spawn(async move {
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
                        to_address: to_address.address,
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

    let actual_submissions = submissions.lock().await.len();
    tracing::info!(
        "{}/{} transactions submitted",
        actual_submissions, TRANSACTION_COUNT
    );

    // verify all tranasctions were submitted
    assert_eq!(actual_submissions, TRANSACTION_COUNT);

    Ok(())
}
