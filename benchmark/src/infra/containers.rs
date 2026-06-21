use crate::infra::PG_CMD;
use anyhow::{Context, Result};
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::path::PathBuf;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, core::WaitFor, runners::AsyncRunner};
use tracing::{info, warn};

// ============================================================
// data structures

/// Central infrastructure context for the benchmark harness.
///
/// Manages container lifecycles, health check probes, migrations, and dynamic port resolution.
pub struct InfraStack {
    pub pg_url: String,
    pub redis_url: String,
    pool: PgPool,
    pg_container: ContainerAsync<GenericImage>,
    redis_container: ContainerAsync<GenericImage>,
}

impl InfraStack {
    /// Initializes Postgres & Redis test-containers, waits for health checks, and applies migrations.
    pub async fn build() -> Result<Self> {
        info!("Booting ephemeral infrastructure for benchmark harness...");

        // 1. Concurrent Postgres & Redis startup
        // to minimize harness initialization latency.
        let pg_future = async {
            let pg_image = GenericImage::new("postgres", "18.3-alpine")
                .with_env_var("POSTGRES_USER", "lobby")
                .with_env_var("POSTGRES_PASSWORD", "lobby_dev_password")
                .with_env_var("POSTGRES_DB", "lobby-db")
                .with_cmd(PG_CMD)
                .with_ready_conditions(vec![WaitFor::message_on_stdout(
                    "database system is ready to accept connections",
                )]);
            pg_image
                .start()
                .await
                .context("Failed to start Postgres container")
        };

        let redis_future = async {
            let redis_image = GenericImage::new("redis", "8.6-alpine").with_wait_for(
                WaitFor::message_on_stdout("Ready to accept connections tcp"),
            );
            redis_image
                .start()
                .await
                .context("Failed to start Redis container")
        };

        let (pg_container, redis_container) = tokio::try_join!(pg_future, redis_future)?;
        info!("Postgres and Redis containers are running and healthy.");

        // 2. Resolve dynamic host ports
        let pg_port = pg_container.get_host_port_ipv4(5432).await?;
        let redis_port = redis_container.get_host_port_ipv4(6379).await?;

        let pg_url = format!(
            "postgresql://lobby:lobby_dev_password@127.0.0.1:{}/lobby-db",
            pg_port
        );
        let redis_url = format!("redis://127.0.0.1:{}", redis_port);

        info!(pg_port, redis_port, "Resolved dynamic host ports.");

        // 3. Runtime migration run with High-Throughput Pool Tuning
        let migration_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../database/migrations");

        let pool = PgPoolOptions::new()
            .max_connections(100)
            .min_connections(10)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(&pg_url)
            .await
            .context("Failed to connect to Postgres pool")?;

        Migrator::new(migration_path)
            .await
            .context("Failed to initialize SQLx migrator")?
            .run(&pool)
            .await
            .context("Failed to execute database migrations")?;

        info!("Database migrations applied successfully. Infrastructure ready.");

        Ok(Self {
            pg_url,
            redis_url,
            pool,
            pg_container,
            redis_container,
        })
    }

    /// Explicit async teardown. Preferred over relying solely on `Drop` for benchmarks
    ///
    /// ensures deterministic cleanup and avoid blocking the async runtime in Drop.
    pub async fn teardown(self) {
        info!("Initiating explicit teardown of infrastructure containers...");

        if let Err(e) = self.pg_container.stop().await {
            warn!(error = %e, "Failed to stop Postgres container");
        }
        if let Err(e) = self.pg_container.rm().await {
            warn!(error = %e, "Failed to remove Postgres container");
        }

        if let Err(e) = self.redis_container.stop().await {
            warn!(error = %e, "Failed to stop Redis container");
        }
        if let Err(e) = self.redis_container.rm().await {
            warn!(error = %e, "Failed to remove Redis container");
        }

        info!("Infrastructure teardown complete.");
    }

    /// Returns a reference to the internal `PgPool`.
    #[inline]
    pub fn get_pool(&self) -> &PgPool {
        &self.pool
    }
}

// ============================================================
