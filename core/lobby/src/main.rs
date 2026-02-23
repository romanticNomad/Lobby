use std::{env, net::SocketAddr};

use cortex::config::CortexConfig;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use utils::directory::load_api_key_from_env;

#[allow(dead_code)]
mod server;

// ============================================================
// lobby boot sequence
#[allow(dead_code)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ============================================================
    // logging

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lobby=debug,cortex=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("lobby boot sequence started");

    // ============================================================
    // load environment

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let _server_addr: SocketAddr = env::var("SERVER_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
        .parse()
        .expect("SERVER_ADDR is not a valid socket address");

    // ============================================================
    // load database

    let db_pool = PgPoolOptions::new()
        .max_connections(17)
        .connect(&database_url)
        .await?;

    tracing::info!("database connection estabilished");

    sqlx::migrate!("../database/migrations")
        .run(&db_pool)
        .await?;
    tracing::info!("database migrations applied");

    // ============================================================
    // API keys

    let api_keys = load_api_key_from_env()?;
    tracing::info!(count = api_keys.len(), "api_keys loaded");

    // ============================================================
    // Cortex -> orchestrator
    // Reads shard counts, concurrency limits, and retry policy from env.
    // Falls back to prototype defaults (17 shards, 17 concurrent pipelines,
    // 2 retries with full-jitter exponential backoff).

    let _config = CortexConfig::from_env()?;
    // let orchestrator = spawn_cortex(db_pool.clone(), provider, config);

    Ok(())
}
