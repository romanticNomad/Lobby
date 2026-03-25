use alloy::primitives::U256;
use anyhow::{Context, Result};
use primitives::types::ChainId;
use testcontainers::{
    core::{ContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};
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

    Ok(format!("redis://{}:{}", host, port))
}

// ============================================================
// Anvil Containers

/// Spawn 3 Anvil instances for mainnet (1), polygon (137), and arbitrum (42161).
/// Returns: Vec<(ChainId, rpc_url, port, container)>
pub async fn spawn_anvil_instances() -> Result<Vec<(ChainId, String, ContainerAsync<GenericImage>)>>
{
    let chains = vec![
        (ChainId(U256::from(1)), "mainnet"),
        (ChainId(U256::from(137)), "polygon"),
        (ChainId(U256::from(42161)), "arbitrum"),
    ];

    let mut instances = Vec::new();

    for (chain_id, name) in chains {
        let container = spawn_anvil(chain_id)
            .await
            .context(format!("containers: failed to start anvil for: {}", name))?;

        let host = container.get_host().await?;
        let port = container.get_host_port_ipv4(8454).await?;
        let rpc_url = format!("http://{}:{}", host, port);

        tracing::info!(
            "containers: anvil started for {} (chain_id: {}, url: {}",
            name,
            chain_id,
            rpc_url
        );

        instances.push((chain_id, rpc_url, container));
    }

    Ok(instances)
}

async fn spawn_anvil(chain_id: ChainId) -> Result<ContainerAsync<GenericImage>> {
    // official Foundry image from GitHub Container Registry
    let port = ContainerPort::Tcp(8545);
    let anvil = GenericImage::new("ghcr.io/foundry-rs/foundry", "latest")
        .with_exposed_port(port)
        .with_wait_for(WaitFor::message_on_stdout("Listening on"))
        .with_cmd(vec![
            "anvil",
            "--host",
            "0.0.0.0",
            "--port",
            "8545",
            "--block-time",
            "1",
            "chain-id",
            &chain_id.0.to_string(),
            "--accounts",
            "0",
        ])
        .start()
        .await
        .context("containers: failed to start anvil")?;

    // Wait a bit for Anvil to fully initialize
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    Ok(anvil)
}

// ============================================================
