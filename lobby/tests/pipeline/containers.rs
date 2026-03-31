//! Container management module for healthy_path_test.
//! Handles local Anvil processes and testcontainers for PostgreSQL/Redis.

use alloy::{
    primitives::{Address, U256},
    providers::{ProviderBuilder, ext::AnvilApi},
};
use dashmap::DashMap;
use std::env;
use std::process::{Child, Command, Stdio};
use testcontainers_modules::{
    postgres::Postgres,
    redis::Redis,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};
use tokio::time::{Duration, sleep};

pub struct TestContainers {
    pub postgres: ContainerAsync<Postgres>,
    pub postgres_endpoint: String,
    pub redis: ContainerAsync<Redis>,
    pub redis_endpoint: String,
    pub anvil_processes: Vec<Child>,
    pub anvil_endpoints: DashMap<u64, String>,
}

impl TestContainers {
    pub async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        // 1. Setup Postgres
        let postgres = Postgres::default()
            .with_env_var("POSTGRES_USER", "lobby")
            .with_env_var("POSTGRES_PASSWORD", "lobby_test_pswd")
            .with_env_var("POSTGRES_DB", "lobby_test_db")
            .start()
            .await?;

        let postgres_port = postgres.get_host_port_ipv4(5432).await?;
        let postgres_endpoint = format!(
            "postgres://lobby:lobby_test_pswd@localhost:{}/lobby_test_db",
            postgres_port
        );

        // 2. Setup Redis
        let redis = Redis::default().start().await?;
        let redis_port = redis.get_host_port_ipv4(6379).await?;
        let redis_endpoint = format!("redis://localhost:{}", redis_port);

        // 3. Setup Anvil Forks
        let mut anvil_processes = Vec::new();
        let anvil_endpoints = DashMap::new();

        // Chain IDs: 1 (Mainnet), 137 (Polygon), 42161 (Arbitrum)
        for chain_id in [1, 137, 42161] {
            let rpc_url = get_rpc_url_for_chain(chain_id)?;
            let port = find_available_port().await?;

            tracing::info!(
                "Starting Anvil fork for chain {} on port {}...",
                chain_id,
                port
            );

            let child = start_anvil_process(chain_id, port, &rpc_url)?;

            let endpoint = format!("http://127.0.0.1:{}", port);
            verify_anvil_ready(&endpoint).await?;

            anvil_processes.push(child);
            anvil_endpoints.insert(chain_id, endpoint.clone());
        }

        Ok(Self {
            postgres,
            postgres_endpoint,
            redis,
            redis_endpoint,
            anvil_processes,
            anvil_endpoints,
        })
    }

    pub fn get_rpc_from_chain(&self, chain_id: u64) -> Option<String> {
        self.anvil_endpoints.get(&chain_id).as_deref().cloned()
    }

    pub async fn fund_test_accounts(
        &self,
        accounts: &[Address],
        amount_eth: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let amount_wei = U256::from(amount_eth) * U256::from(10).pow(U256::from(18));
        for entry in &self.anvil_endpoints {
            let provider = ProviderBuilder::new().connect_http(entry.value().parse()?);
            for account in accounts {
                provider.anvil_set_balance(*account, amount_wei).await?;
            }
        }
        Ok(())
    }
}

// ============================================================
// Helpers

fn get_rpc_url_for_chain(chain_id: u64) -> Result<String, Box<dyn std::error::Error>> {
    let var_name = match chain_id {
        1 => "ETH_RPC_URL",
        137 => "POLYGON_RPC_URL",
        42161 => "ARBITRUM_RPC_URL",
        _ => return Err(format!("No RPC configured for chain {}", chain_id).into()),
    };

    env::var(var_name).map_err(|_| format!("Missing environment variable: {}", var_name).into())
}

fn start_anvil_process(
    chain_id: u64,
    port: u16,
    rpc_url: &str,
) -> Result<Child, Box<dyn std::error::Error>> {
    let child = Command::new("anvil")
        .arg("--fork-url")
        .arg(rpc_url)
        .arg("--chain-id")
        .arg(chain_id.to_string())
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit()) // Show errors in terminal if fork fails
        .spawn()
        .map_err(|e| -> Box<dyn std::error::Error> {
            format!("Failed to spawn anvil: {}", e).into()
        })?;

    Ok(child)
}

async fn find_available_port() -> Result<u16, Box<dyn std::error::Error>> {
    for port in 8545..9000 {
        if tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Ok(port);
        }
    }
    Err("No available ports found".into())
}

async fn verify_anvil_ready(endpoint: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1});
    let timeout = Duration::from_secs(10);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        if let Ok(resp) = client.post(endpoint).json(&body).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err(format!("Anvil timeout at {}", endpoint).into())
}

impl Drop for TestContainers {
    fn drop(&mut self) {
        for process in &mut self.anvil_processes {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

// ============================================================
