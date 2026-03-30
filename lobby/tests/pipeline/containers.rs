//! Container management module for healthy_path_test.
//! Handles Docker container lifecycle for PostgreSQL, Redis, and Anvil nodes.

use alloy::{
    primitives::{Address, U256},
    providers::{ProviderBuilder, ext::AnvilApi},
};
use dashmap::DashMap;
use testcontainers_modules::{
    anvil::{ANVIL_PORT, AnvilNode},
    postgres::Postgres,
    redis::Redis,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

// ============================================================
// Container setup

/// Container manager that holds all test containers and their connection info.
pub struct TestContainers {
    // Postgres connectors
    pub postgres: ContainerAsync<Postgres>,
    pub postgres_endpoint: String,

    // Redis connectors
    pub redis: ContainerAsync<Redis>,
    pub redis_endpoint: String,

    // Anvil connectors
    pub anvil_containers: Vec<ContainerAsync<AnvilNode>>,
    pub anvil_endpoints: DashMap<u64, String>,
}

// ============================================================
// implimentaions for TestContainers

impl TestContainers {
    // ============================================================

    /// Start all required containers for the test.
    /// Spawns PostgreSQL, Redis, and 3 Anvil nodes for Ethereum, Polygon, and Arbitrum.
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

        // Redis Container
        let redis = Redis::default().start().await?;
        let redis_port = redis.get_host_port_ipv4(6379).await?;
        let redis_endpoint = format!("redis://localhost:{}", redis_port);

        // Anvil containers connectors for each chain

        let mut anvil_containers = Vec::new();
        let anvil_endpoints = DashMap::new();

        for chain_id in [1, 137, 42161] {
            let anvil = AnvilNode::default().with_chain_id(chain_id).start().await?;

            let host_port = anvil.get_host_port_ipv4(ANVIL_PORT).await?;
            let endpoint = format!("http://localhost:{}", host_port);

            anvil_containers.push(anvil);
            anvil_endpoints.insert(chain_id, endpoint);
        }

        Ok(Self {
            postgres,
            postgres_endpoint,
            redis,
            redis_endpoint,
            anvil_containers,
            anvil_endpoints,
        })
    }

    // ============================================================

    /// Get the RPC endpoint for a specific chain ID.
    pub fn get_rpc_from_chain(&self, chain_id: u64) -> Option<String> {
        let rpc_url = self.anvil_endpoints.get(&chain_id).as_deref().cloned();

        rpc_url
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
                // Use anvil_setBalance to fund the account
                provider.anvil_set_balance(*account, amount_wei).await?;
            }
        }

        Ok(())
    }
}

// ============================================================
