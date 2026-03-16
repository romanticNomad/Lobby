pub mod bots;
pub mod server;

use crate::{
    bots::{scanner::spawn_scanner_bot, sweeper::spawn_sweeper_bot},
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
use std::{env, fs::OpenOptions, net::SocketAddr};
use tracing_forest::ForestLayer;
use tracing_subscriber::{EnvFilter, Registry, fmt, layer::SubscriberExt, util::SubscriberInitExt};
use utils::{
    custody::export_custody_key_count,
    registry::{load_api_key_from_env, load_rpc_endpoints_from_env},
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
        .open("logs/transactions.log")
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

    tracing::info!("lobby bootup sequence active");

    // ============================================================
    // environment

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let server_addr: SocketAddr = env::var("SERVER_ADDR")
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

    // rpc-endpoint registry
    let rpc_registry = load_rpc_endpoints_from_env();
    let chains: Vec<_> = rpc_registry.iter().map(|entry| *entry.key()).collect();
    if rpc_registry.len() == 0 {
        tracing::warn!(
            "no RPC endpoints found in environment, \
            broadcast and validator will fail for all chains."
        );
    }
    tracing::info!("rpc_endpoints loaded: {:?}", chains);

    // custody keys
    let custody_keys_count = export_custody_key_count();
    tracing::info!("custody accounts loaded: {custody_keys_count}");

    //cortex handler
    let config = CortexConfig::from_env()?;
    let cortex_handler = spawn_cortex(db_pool.clone(), rpc_registry.clone(), config).await;

    // status registry -> fails if RockDB failed to boot up
    let status_registry = cortex_handler.status_registry();

    // ============================================================
    // bots

    spawn_sweeper_bot(db_pool.clone());
    spawn_scanner_bot(
        db_pool.clone(),
        status_registry.clone(),
        rpc_registry.clone(),
    );

    tracing::info!("bots spawned, monitoring status");

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

    tracing::info!(%server_addr, "lobby listening");
    let listner = tokio::net::TcpListener::bind(server_addr).await?;
    axum::serve(listner, app).await?;

    Ok(())
}

// ============================================================
