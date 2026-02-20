mod server;

use actors::relayhost::spawn_relayhost_actor;
use server::*;
use sqlx::postgres::PgPoolOptions;
use std::{env, net::SocketAddr, sync::Arc};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use utils::api_keys_load::load_api_key_from_env;

// ============================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ============================================================
    // lobby booting sequemce

    //logging pipeline
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lobby=debug, tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // loading environment variables
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let server_addr: SocketAddr = env::var("SERVER_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
        .parse()
        .expect("Invalid server address");

    // initialize database connection pool and sqlx migration
    let db_pool = PgPoolOptions::new()
        .max_connections(17)
        .connect(&database_url)
        .await?;

    tracing::info!("database connection estabilished");

    sqlx::migrate!("../database/migrations")
        .run(&db_pool)
        .await?;

    tracing::info!("database migrations applied");

    // load api keys from the environment
    let api_keys = load_api_key_from_env()?;

    // spawn RelayHost actor
    let relayhost_handle = spawn_relayhost_actor(db_pool.clone(), 17);

    // start axum app
    let state = AppState::new(Arc::new(api_keys), relayhost_handle);
    let app = build_app(state);
    serve(app, server_addr).await?;

    Ok(())
}
