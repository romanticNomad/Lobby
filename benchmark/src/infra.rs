use anyhow::{Ok, Result};
use sqlx::{PgPool, migrate::Migrator};
use std::path::PathBuf;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, core::WaitFor, runners::AsyncRunner};
use tokio::process::Command;

// ============================================================
// Postgres with benchmark-tuned CLI flags (bypasses default healthcheck)

const PG_CMD: [&str; 13] = [
    "postgres",
    "-c",
    "shared_buffer=512MB",
    "-c",
    "max_connections=100",
    "-c",
    "wal_minimum",
    "-c",
    "fsync=off",
    "-c",
    "synchronous_commit=off",
    "-c",
    "checkpoint_timeout=300s",
];

// ============================================================

/// Lobby-Api keys stack, to be set up in the environment along with docker urls
///
/// API Key format:
/// 
/// `LOBBY_API_KEY_<N>=<api_token>:<client_id>:<from_address>`
pub struct ApiStack {
    key: String,
    api: String,
}

// ============================================================

/// Central infrastructure context for the benchmark harness.
/// Manages container lifecycles, health_check probes, migrations, and dynamic port resolution.
pub struct InfraStack {
    pub pg_url: String,
    pub redis_url: String,
    pool: PgPool,
    pg_container: ContainerAsync<GenericImage>,
    redis_container: ContainerAsync<GenericImage>,
}

impl InfraStack {
    // ============================================================

    /// Initializes Postgres & Redis, waits for health_checks, and applies migrations.
    pub async fn build() -> Result<Self> {
        // 1. Postgres startup
        let pg_image = GenericImage::new("postgres", "18.3-alpine")
            .with_env_var("POSTGRES_USER", "lobby")
            .with_env_var("POSTGRES_PASSWORD", "lobby_dev_password")
            .with_env_var("POSTGRES_DB", "lobby-db")
            .with_cmd(PG_CMD)
            .with_ready_conditions(vec![WaitFor::message_on_stdout(
                "database system is ready to accept connections",
            )]);
        let pg_container = pg_image.start().await?;

        // 2. Redis startup
        let redis_image = GenericImage::new("redis", "8.6-alpine").with_wait_for(
            WaitFor::message_on_stdout("Ready to accept connections tcp"),
        );
        let redis_container = redis_image.start().await?;

        // 3. Resolve dynamic host ports
        let pg_port = pg_container.get_host_port_ipv4(5432).await?;
        let redis_port = redis_container.get_host_port_ipv4(6379).await?;

        let pg_url = format!(
            "postgresql://lobby:lobby_dev_password@127.0.0.1:{}/lobby-db",
            pg_port
        );
        let redis_url = format!("redis://127.0.0.1:{}", redis_port);

        // 4. Runitime migration run
        let migration_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../database/migrations");
        let pool = PgPool::connect(&pg_url).await?;
        Migrator::new(migration_path).await?.run(&pool).await?;

        Ok(Self {
            pg_url,
            redis_url,
            pool,
            pg_container,
            redis_container,
        })
    }

    // ============================================================

    /// Returns a pre-configured `Command` builder for the `lobby` binary.
    /// Injects dynamic URLs, disables source `.env` requirement, and sets benchmark flags.
    pub fn lobby_command(&self, api_keys: Vec<ApiStack>) -> Command {
        let mut cmd = Command::new("cargo");
        cmd.args(["run", "--release", "--features", "bench", "--bin", "lobby"])
            .env("DATABASE_URL", &self.pg_url)
            .env("REDIS_URL", &self.redis_url)
            .env("SERVER_ADDR", "127.0.0.1:3000")
            .env("RUST_LOG", "WARN"); // Reduce overhead during bench

        for api_key in api_keys {
            cmd.env(api_key.key, api_key.api);
        }
        cmd
    }

    // ============================================================

    /// Explicit async teardown. Prefer over relying solely on `Drop` for benchmarks.
    pub async fn teardown(self) {
        let _ = self.pg_container.stop().await;
        let _ = self.pg_container.rm().await; // rm method does not accept a reference
        let _ = self.redis_container.stop().await;
        let _ = self.redis_container.rm().await;
    }

    // ============================================================
    // pool_accessor

    #[inline]
    pub fn get_pool(&self) -> &PgPool {
        &self.pool
    }
}

// ============================================================
