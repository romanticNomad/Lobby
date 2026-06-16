pub mod bots;
pub mod server;

use crate::{
    bots::sweeper::spawn_sweeper_bot,
    server::{
        AppState,
        auth::auth_middleware,
        handler::{get_transaction_status, submit_transaction},
    },
};
use axum::{
    Router, middleware,
    routing::{get, post},
};
use cortex::{artifacts::config::CortexConfig, spawn_cortex};
use sqlx::postgres::PgPoolOptions;
use std::{env, fs::OpenOptions, net::SocketAddr, sync::Arc};
use tracing_forest::ForestLayer;
use tracing_subscriber::{EnvFilter, Registry, fmt, layer::SubscriberExt, util::SubscriberInitExt};
use utils::{
    api::load_api_key_from_env,
    custody::export_custody_key_count,
    rpc::{build_rpc_client, get_client_endpoint_hashmap},
};

// ============================================================

/// lobby boot sequence
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ============================================================
    // logging

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("transactions.log")
        .expect("Failed to open LOG file");

    Registry::default()
        .with(EnvFilter::from_default_env())
        .with(ForestLayer::default())
        .with(
            fmt::layer()
                .with_writer(file)
                .with_ansi(false)
                .with_target(true)
                .with_thread_ids(true),
        )
        .init();

    tracing::debug!("lobby bootup sequence active");

    // ============================================================
    // environment

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let address: SocketAddr = env::var("SERVER_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
        .parse()
        .expect("SERVER_ADDR is not a valid socket address");

    // ============================================================
    // database

    let db_pool = PgPoolOptions::new()
        .max_connections(17)
        .connect(&database_url)
        .await?;

    sqlx::migrate!("../database/migrations")
        .run(&db_pool)
        .await?;

    // ============================================================
    // lobby state artifacts

    // api_registry
    let api_registry = load_api_key_from_env()?;
    let count = api_registry.len();
    tracing::info!("api_keys loaded: {count}");

    // custody keys
    let custody_keys_count = export_custody_key_count();
    tracing::info!("custody accounts loaded: {custody_keys_count}");

    // rpc-endpoint registry
    let rcp_client = match build_rpc_client().await {
        Ok(client) => client,
        Err(e) => {
            tracing::error!("Failed to build RPC client: {}", e);
            return Err(e.into());
        }
    };
    let endpoint_hashmap = get_client_endpoint_hashmap(&rcp_client).await?;
    tracing::info!("rpc_endpoints loaded: {endpoint_hashmap:?}");

    // telemetry channel
    #[cfg(feature = "benchmark_telemetry")] {
        let (telemetry_tx, telemetry_rx) = tokio::sync::mpsc::unbounded_channel();
        let teletery_context = std::sync::Arc::new(cortex::artifacts::telemetry::TelemetryContext::new(telemetry_tx));
        let registry = teletery_context.get_registry();
        tokio::spawn(async move {
            run_telemetry_exporter(
                telemetry_rx,
                "/tmp/lobby_benchmark_telemetry.sock", // Standardized UDS path
                registry,
            ).await;
        });
    }

    // cortex handler
    let config = CortexConfig::from_env()?;
    let cortex_handler = spawn_cortex(db_pool.clone(), Arc::new(rcp_client), config).await;

    // status registry
    let status_registry = cortex_handler.status_registry();

    // ============================================================
    // sweeper bot -> checks 'reserved' transaction with expired 5 min lease and marks them 'released'.
    // scanner bot -> checks RPC for block inclusion status of 'timed_out' transactions.

    spawn_sweeper_bot(db_pool.clone());

    // spawn_scanner_bot(
    //     db_pool.clone(),
    //     status_registry.clone(),
    //     rpc_provider_stack.validator_registry.clone(),
    // ); -> Deprecated until further update

    tracing::info!("bots spawned: monitoring status");

    // ============================================================
    // axum app

    // AppState
    let state = AppState::new(api_registry, cortex_handler, status_registry);
    let app = Router::new()
        // Transaction submission (fire-and-forget, returns immediately with execution_id)
        .route("/v1/transactions", post(submit_transaction))
        // Status polling — clients poll until `confirmed` or `failed`
        .route("/status/{execution_id}", get(get_transaction_status))
        // Auth middleware applied to all routes
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    tracing::info!(%address, "lobby listening at:");
    let listner = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listner, app).await?;

    Ok(())
}

// ============================================================
