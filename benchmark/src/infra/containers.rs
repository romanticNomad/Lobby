use anyhow::{Context, Result};
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::path::PathBuf;
use std::time::Duration;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, core::WaitFor, runners::AsyncRunner};
use tracing::{info, warn};

// ============================================================
// data structures

/// Central infrastructure context for the benchmark harness.
///
/// Manages container lifecycles, health check probes, migrations, and dynamic port resolution.
#[allow(dead_code)]
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
        // minimizes harness initialization latency.
        let pg_future = async {
            let pg_image = GenericImage::new("postgres", "18.3-alpine")
                .with_env_var("POSTGRES_USER", "lobby")
                .with_env_var("POSTGRES_PASSWORD", "lobby_dev_password")
                .with_env_var("POSTGRES_DB", "lobby-db")
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

        // Give Postgres a moment to fully initialize TCP listener
        tokio::time::sleep(Duration::from_millis(1000)).await;

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
            .acquire_timeout(Duration::from_secs(5))
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
    #[allow(dead_code)]
    pub fn get_pool(&self) -> PgPool {
        self.pool.clone() // cheap Arc<> clone
    }
}

// ============================================================
// unit tests

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;

    #[tokio::test]
    async fn test_infra_stack_build_and_teardown() -> Result<()> {
        // 1. Build the infrastructure stack
        let infra = InfraStack::build().await?;

        // 2. Verify URLs are properly formatted
        assert!(
            infra
                .pg_url
                .starts_with("postgresql://lobby:lobby_dev_password@127.0.0.1:")
        );
        assert!(infra.redis_url.starts_with("redis://127.0.0.1:"));

        // 3. Verify database pool is functional
        let pool = infra.get_pool();

        // Test basic connectivity
        let result = sqlx::query(r#"SELECT 1 as num"#).fetch_one(&pool).await?;
        let num: i32 = result.get("num");
        assert_eq!(num, 1);

        // 4. Verify migrations were applied
        let table_check = sqlx::query(
            r#"SELECT EXISTS (
                SELECT FROM information_schema.tables
                WHERE table_schema = 'nonce'
                AND table_name = 'nonce_assignments'
            ) as exists"#,
        )
        .fetch_one(&pool)
        .await?;

        let table_exists: bool = table_check.get("exists");
        assert!(
            table_exists,
            "Migration should have created transaction_intents table"
        );

        // 5. Verify containers are running by checking they respond to commands
        // (testcontainers handles this internally, but we verify the stack is usable)
        assert!(!infra.pg_url.is_empty());
        assert!(!infra.redis_url.is_empty());

        // 6. Teardown
        infra.teardown().await;

        // 7. After teardown, attempting to use the pool should fail
        // (containers are stopped, so connection should fail)
        let query_result = sqlx::query(r#"SELECT 1"#).fetch_one(&pool).await;

        assert!(
            query_result.is_err(),
            "Pool should be unusable after teardown"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_infra_stack_multiple_builds() -> Result<()> {
        // Verify that multiple stacks can coexist (different ports)
        let infra1 = InfraStack::build().await?;
        let infra2 = InfraStack::build().await?;

        // Ports should be different
        assert_ne!(infra1.pg_url, infra2.pg_url);
        assert_ne!(infra1.redis_url, infra2.redis_url);

        // Both should be functional
        let pool1 = infra1.get_pool();
        let pool2 = infra2.get_pool();

        sqlx::query(r#"SELECT 1"#).fetch_one(&pool1).await?;
        sqlx::query(r#"SELECT 1"#).fetch_one(&pool2).await?;

        // Cleanup
        infra1.teardown().await;
        infra2.teardown().await;
        Ok(())
    }

    #[tokio::test]
    async fn test_infra_stack_pool_configuration() -> Result<()> {
        let infra = InfraStack::build().await?;
        let pool = infra.get_pool();

        // Verify pool options were applied (max_connections = 100)
        let pool_options = pool.options();
        assert_eq!(pool_options.get_max_connections(), 100);
        assert_eq!(pool_options.get_min_connections(), 10);

        infra.teardown().await;
        Ok(())
    }
}

// ============================================================
