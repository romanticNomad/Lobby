use anyhow::{Context, Result};
use testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};
use testcontainers_modules::{postgres::Postgres, redis::Redis};

// ============================================================
// PostgreSQL Container

pub async fn spawn_postgres() -> Result<ContainerAsync<Postgres>> {
    let postgres = Postgres::default()
        .with_tag("18.3")
        .with_env_var("POSTGRES_PASSWORD", "lobby_test_pswd")
        .with_env_var("POSTGRES_DB", "lobby_test")
        .start()
        .await
        .context("containers: failed to start postgres")?;

    tracing::info!("containers: postgres statrted");
    Ok(postgres)
}

pub async fn contect_postgres(container: &ContainerAsync<Postgres>) -> Result<String> {
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(5432).await?;

    Ok(format!(
        "postgres://postgres:lobby_test_pswd@{}:{}/lobby_test",
        host, port
    ))
}

// ============================================================
// Redis Container

pub async fn spawn_redis() -> Result<ContainerAsync<Redis>> {
    let redis = Redis::default()
        .with_tag("8.6-alpine")
        .start()
        .await
        .context("containers: failed to start redis")?;

    tracing::info!("containers: redis started");
    Ok(redis)
}

pub async fn connect_redis(container: &ContainerAsync<Redis>) -> Result<String> {
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(6379).await?;

    Ok(format!(
        "redis://{}:{}", host, port
    ))
}

// ============================================================
// Anvil Containers


