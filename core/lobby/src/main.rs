// use std::{env, net::SocketAddr, sync::Arc};

// use crate::server::{AppState, auth::auth_middleware, handler::submit_transaction};
// use axum::{
//     Router, middleware,
//     routing::{get, post},
// };
// use cortex::{config::CortexConfig, spawn_cortex, state::get_transaction_status};
// use sqlx::postgres::PgPoolOptions;
// use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
// use utils::directory::{load_api_key_from_env, load_rpc_endpoints_from_env};

// pub mod server;

// ============================================================
// lobby boot sequence

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ============================================================
    // logging

    // tracing_subscriber::registry()
    //     .with(
    //         tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
    //             "lobby=debug,orchestrator=debug,validator=debug,tower_http=debug".into()
    //         }),
    //     )
    //     .with(tracing_subscriber::fmt::layer())
    //     .init();

    // tracing::info!("lobby starting");

    // // ============================================================
    // // environment

    // let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    // let server_addr: SocketAddr = env::var("SERVER_ADDR")
    //     .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
    //     .parse()
    //     .expect("SERVER_ADDR is not a valid socket address");

    // // ============================================================
    // // database

    // let db_pool = PgPoolOptions::new()
    //     .max_connections(17)
    //     .connect(&database_url)
    //     .await?;

    // tracing::info!("database connection estabilished");

    // sqlx::migrate!("../database/migrations")
    //     .run(&db_pool)
    //     .await?;
    // tracing::info!("database migrations applied");

    // // ============================================================
    // // api keys

    // let api_registry = load_api_key_from_env()?;
    // tracing::info!(count = api_registry.len(), "api keys loaded");

    // // ============================================================
    // // rpc provider registry

    // let rpc_registry = load_rpc_endpoints_from_env();

    // let chain_count = rpc_registry.len();
    // if chain_count == 0 {
    //     tracing::warn!(
    //         "no RPC endpoints found in environment (expected RPC_ENDPOINT_1=https://...). \
    //          broadcast and validator will fail for all chains."
    //     );
    // } else {
    //     let chains: Vec<_> = rpc_registry.iter().map(|entry| *entry.key()).collect();
    //     tracing::info!(
    //         ?chains,
    //         count = chain_count,
    //         "RPC provider registry initialized"
    //     );
    // }

    // // ============================================================
    // // orchestrator inti

    // let config = CortexConfig::from_env()?;
    // let cortex_handler = spawn_cortex(db_pool.clone(), rpc_registry, config);

    // tracing::info!("cortex ready");

    // // ============================================================
    // // axum app

    // let status_registry = cortex_handler.status_registry();
    // let state = AppState::new(api_registry, cortex_handler);

    // let app = Router::new()
    //     // Transaction submission (fire-and-forget, returns immediately with execution_id)
    //     .route("/", post(submit_transaction))
    //     // Status polling — clients poll until `confirmed` or `failed`
    //     .route("/status/:execution_id", get(get_transaction_status))
    //     .with_state(status_registry)
    //     // Auth middleware applied to all routes
    //     .layer(middleware::from_fn_with_state(
    //         state.clone(),
    //         auth_middleware,
    //     ))
    //     .with_state(state);

    // tracing::info!(%server_addr, "lobby listening");
    // let listner = tokio::net::TcpListener::bind(server_addr).await?;
    // axum::serve(listner, app).await?;

    Ok(())
}

// ============================================================
