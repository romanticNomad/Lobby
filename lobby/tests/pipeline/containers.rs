//! Container management module for healthy_path_test.
//! Handles local Anvil processes and testcontainers for PostgreSQL/Redis.

use alloy::{
    primitives::{Address, U256},
    providers::{ProviderBuilder, ext::AnvilApi},
};
use dashmap::DashMap;
use std::process::{Child, Command, Stdio};
use testcontainers_modules::{
    postgres::Postgres,
    redis::Redis,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};
use tokio::time::{sleep, Duration};

// ============================================================
// Container setup

/// Container manager that holds all test containers/processes and their connection info.
pub struct TestContainers {
    // Postgres connectors
    pub postgres: ContainerAsync<Postgres>,
    pub postgres_endpoint: String,

    // Redis connectors
    pub redis: ContainerAsync<Redis>,
    pub redis_endpoint: String,

    // Anvil child processes (NOT Docker containers)
    pub anvil_processes: Vec<Child>,
    pub anvil_endpoints: DashMap<u64, String>,
}

// ============================================================
// implimentaions for TestContainers

impl TestContainers {
    // ============================================================

    /// Start all required containers/processes for the test.
    /// Spawns PostgreSQL, Redis in Docker, and 3 local Anvil processes.
    pub async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        // Postgres container
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
        
        tracing::info!("PostgreSQL started on port {}", postgres_port);

        // Redis Container
        let redis = Redis::default().start().await?;
        let redis_port = redis.get_host_port_ipv4(6379).await?;
        let redis_endpoint = format!("redis://localhost:{}", redis_port);
        
        tracing::info!("Redis started on port {}", redis_port);

        // Start local Anvil processes instead of Docker containers
        let mut anvil_processes = Vec::new();
        let anvil_endpoints = DashMap::new();

        // Find available ports and start Anvil processes
        for chain_id in [1, 137, 42161] {
            let port = find_available_port().await?;
            tracing::info!("Starting local Anvil for chain {} on port {}...", chain_id, port);
            
            let child = start_anvil_process(chain_id, port)?;
            
            // Give Anvil a moment to start
            sleep(Duration::from_millis(300)).await;
            
            let endpoint = format!("http://127.0.0.1:{}", port);
            
            // Verify it's actually ready
            verify_anvil_ready(&endpoint).await?;
            
            anvil_processes.push(child);
            anvil_endpoints.insert(chain_id, endpoint.clone());
            
            tracing::info!("Anvil for chain {} ready at {}", chain_id, endpoint);
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

    // ============================================================

    /// Get the RPC endpoint for a specific chain ID.
    pub fn get_rpc_from_chain(&self, chain_id: u64) -> Option<String> {
        self.anvil_endpoints.get(&chain_id).as_deref().cloned()
    }

    // ============================================================

    /// Fund test accounts with ETH on all Anvil nodes.
    pub async fn fund_test_accounts(
        &self,
        accounts: &[Address],
        amount_eth: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let amount_wei = U256::from(amount_eth) * U256::from(10).pow(U256::from(18));

        for entry in &self.anvil_endpoints {
            let endpoint = entry.value();
            let provider = ProviderBuilder::new().connect_http(endpoint.parse()?);

            for account in accounts {
                provider.anvil_set_balance(*account, amount_wei).await?;
            }
        }

        Ok(())
    }
}

// ============================================================
// Helper functions for local Anvil management

/// Start a local Anvil process with the given chain ID and port.
fn start_anvil_process(chain_id: u64, port: u16) -> Result<Child, Box<dyn std::error::Error>> {
    let child = Command::new("anvil")
        .arg("--chain-id").arg(chain_id.to_string())
        .arg("--port").arg(port.to_string())
        .arg("--fork-block-number").arg("0") // Start from genesis, no forking
        .arg("--block-time").arg("0") // Instant mining
        .stdout(Stdio::null()) // Suppress output (change to Stdio::piped() for debugging)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| -> Box<dyn std::error::Error>{
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("'anvil' command not found. Is Foundry installed? Try: cargo install --git https://github.com/foundry-rs/foundry --locked forge cast anvil chisel").into()
            } else {
                format!("Failed to start anvil: {}", e).into()
            }
        })?;
    
    Ok(child)
}

/// Find an available TCP port.
async fn find_available_port() -> Result<u16, Box<dyn std::error::Error>> {
    // Try ports in the ephemeral range
    for port in 8545..9000 {
        if is_port_available(port).await {
            return Ok(port);
        }
    }
    Err("No available ports found".into())
}

/// Check if a port is available.
async fn is_port_available(port: u16) -> bool {
    tokio::net::TcpListener::bind(("127.0.0.1", port)).await.is_ok()
}

/// Verify that Anvil is actually ready to accept connections.
async fn verify_anvil_ready(endpoint: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_chainId",
        "params": [],
        "id": 1
    });
    
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(10);
    
    while start.elapsed() < timeout {
        match client.post(endpoint)
            .json(&body)
            .timeout(Duration::from_secs(1))
            .send()
            .await 
        {
            Ok(resp) if resp.status().is_success() => {
                return Ok(());
            }
            _ => {
                sleep(Duration::from_millis(100)).await;
            }
        }
    }
    
    Err(format!("Anvil at {} failed to become ready within {:?}", endpoint, timeout).into())
}

// ============================================================
// Cleanup implementation

impl Drop for TestContainers {
    fn drop(&mut self) {
        // Kill all Anvil processes on drop
        for process in &mut self.anvil_processes {
            let _ = process.kill();
            let _ = process.wait();
        }
        tracing::info!("All local Anvil processes terminated");
    }
}