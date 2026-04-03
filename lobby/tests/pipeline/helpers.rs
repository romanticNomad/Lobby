//! Helper functions for healthy_path_test.
//! Handles transaction generation, submission, and status polling.

use alloy::{
    primitives::Address,
    providers::{Provider, ProviderBuilder},
};
use dashmap::DashMap;
use primitives::types::{
    ApiRegistry, ChainId, ClientConfig, Eip1193SendTransactionParams, ExecutionId, JsonRpcRequest,
    JsonRpcSuccessResponse, JsonStatusResponse, PipelineStatus,
};
use rand::{seq::SliceRandom, thread_rng};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    env,
    fs::File,
    io::BufReader,
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::time::sleep;
use utils::rpc::RpcProviderStack;
use uuid::Uuid;

// ============================================================
// helper structs

/// Test account configuration loaded from test_keys.json.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct TestAccountFormat {
    pvt_key: String,
    pub_key: String,
    address: String,
}
#[derive(Debug, Clone)]
pub struct TestAccount {
    pub address: Address,
}
impl TestAccount {
    fn new(address: Address) -> Self {
        Self { address }
    }
}

/// Result of a transaction submission.
#[derive(Debug, Clone)]
pub struct TransactionSubmission {
    pub execution_id: ExecutionId,
    pub from_address: Address,
    pub chain_id: u64,
}

/// migration code to gerenrate RpcProviderStack for this test suite.
pub fn rpc_provider_stack_build(
    anvil_endpoints: DashMap<u64, String>,
) -> Result<RpcProviderStack, Box<dyn std::error::Error>> {
    let broadcast_registry = Arc::new(DashMap::new());
    let validator_registry = Arc::new(DashMap::new());

    for (chain_id, rpc_url) in anvil_endpoints {
        // Each actor gets its own connection pool (default: 100 connections)
        let broadcast_provider = ProviderBuilder::new().connect_http(rpc_url.parse()?);
        let validation_provider = ProviderBuilder::new().connect_http(rpc_url.parse()?);
        let chain = ChainId::try_from(chain_id as i64)?;

        broadcast_registry.insert(
            chain,
            Arc::new(broadcast_provider) as Arc<dyn Provider + Send + Sync>,
        );
        validator_registry.insert(
            chain,
            Arc::new(validation_provider) as Arc<dyn Provider + Send + Sync>,
        );
    }

    Ok(RpcProviderStack {
        broadcast_registry,
        validator_registry,
    })
}

// ============================================================
// helper fucntions for transaction serializing

/// Load test accounts from the test_keys.json file.
/// Expected format:
/// ```json
/// {
///   "account1": {
///   "pvt_key": "0xfbebc0643be...",
///   "pub_key": "0x0453f74909a...",
///   "address": "0xfea6645d314..."
///   },
/// }
/// ```
pub fn load_test_account() -> Result<Vec<TestAccount>, Box<dyn std::error::Error>> {
    let test_keys_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test_keys.json");
    let file =
        File::open(test_keys_path.to_str().unwrap()).expect("test_keys.json file path invalid");
    let reader = BufReader::new(file);

    let raw_read: HashMap<String, TestAccountFormat> =
        serde_json::from_reader(reader).expect("test_keys.json file invalid");

    let accounts = raw_read
        .into_iter()
        .map(|(_, test_account)| {
            let address: Address = test_account.address.parse().unwrap();

            TestAccount::new(address)
        })
        .collect();

    Ok(accounts)
}

/// Randomly select two distinct accounts from the list.
/// Returns (from_account, to_account).
pub fn select_random_accounts(accounts: &[TestAccount]) -> (TestAccount, TestAccount) {
    let mut rng = thread_rng();
    let mut shuffled = accounts.to_vec();
    shuffled.shuffle(&mut rng);

    (shuffled[0].clone(), shuffled[1].clone())
}

/// Randomly select a chain ID from the available chains.
pub fn select_random_chain(chain_ids: &[u64]) -> u64 {
    let mut rng = thread_rng();
    *chain_ids.choose(&mut rng).unwrap()
}

/// Build EIP-1193 transaction parameters.
pub fn build_transaction_params(
    from: Address,
    to: Address,
    chain_id: u64,
    value_eth: f64,
) -> Eip1193SendTransactionParams {
    let value_wei = (value_eth * 1e18) as u128;
    let value_hex = format!("0x{:x}", value_wei);

    Eip1193SendTransactionParams {
        from,
        to: Some(to),
        gas: Some(format!("0x{:x}", 21000 as u64)),
        max_fee_per_gas: Some(format!("0x{:x}", 150_000_000_000 as u128)),
        max_priority_fee_per_gas: Some(format!("0x{:x}", 2_000_000_000 as u128)),
        value: Some(value_hex),
        data: None,
        chain_id: format!("0x{:x}", chain_id),
        access_list: None,
    }
}

/// Hashmap used for mapping from_addresses to corresponding API keys.
type ApiKeys = HashMap<Address, String>;

/// Build ApiRegistry for testing
pub fn build_api_registry() -> Result<(ApiRegistry, ApiKeys), Box<dyn std::error::Error>> {
    let api_registry: ApiRegistry = Arc::new(DashMap::new());
    let mut fetched_api_keys = HashMap::new();

    for (key, value) in env::vars() {
        if let Some(_n) = key.strip_prefix("LOBBY_API_KEY_") {
            let api_key = value.clone();
            let parts: Vec<&str> = value.split(":").collect();
            let api_token = parts[0].to_string();
            let client_config = ClientConfig {
                client_id: Uuid::from_str(parts[1]).unwrap(),
                from_address: Address::from_str(parts[2]).unwrap(),
            };
            api_registry.insert(api_token, client_config);
            fetched_api_keys.insert(Address::from_str(parts[2]).unwrap(), api_key);
        }
    }

    Ok((api_registry, fetched_api_keys))
}

// ============================================================
// helper functions for transaction handling

/// Submit a transaction to the lobby submit_transaction handler.
pub async fn send_transaction(
    client: &Client,
    base_url: &str,
    api_key: String,
    params: Eip1193SendTransactionParams,
) -> Result<ExecutionId, Box<dyn std::error::Error + Send + Sync>> {
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "eth_sendTransaction".to_string(),
        params: vec![params.clone()],
        id: json!(1),
    };

    let response = client
        .post(&format!("{}/v1/transactions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        let text = response.text().await?;
        return Err(format!("HTTP error: {}", text).into());
    }

    let success: JsonRpcSuccessResponse = response.json().await?;
    Ok(success.result.execution_id)
}

/// Poll for transaction status until it reaches a final state.
/// Returns the final `PipelineStatus` and the time it took to reach it.
pub async fn poll_transaction_status(
    client: &Client,
    base_url: &str,
    execution_id: ExecutionId,
    api_key: String,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<(PipelineStatus, Duration), Box<dyn std::error::Error + Send + Sync>> {
    let start = Instant::now();
    let execution_id_str = execution_id.to_string();

    loop {
        if start.elapsed() > timeout {
            return Err("Polling timeout exceeded".into());
        }

        let response = client
            .get(format!("{}/status/{}", base_url, execution_id_str))
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await?;

        if !response.status().is_success() {
            sleep(poll_interval).await;
            continue;
        }

        let status_response: JsonStatusResponse = response.json().await?;
        tracing::debug!("{:?}", status_response);

        // Check if we've reached a final state
        match &status_response.status {
            PipelineStatus::ConfirmedOnChain { .. } => {
                return Ok((status_response.status, start.elapsed()));
            }
            // PipelineStatus::Broadcasted { .. } => {
            //     return Ok((status_response.status, start.elapsed()));
            // }
            PipelineStatus::Failed { .. } => {
                return Ok((status_response.status, start.elapsed()));
            }
            _ => {
                // Still in progress, continue polling
                sleep(poll_interval).await;
            }
        }
    }
}

// ============================================================

/// Check if a transaction status represents success.
pub fn is_success_status(status: &PipelineStatus) -> bool {
    matches!(
        status,
        PipelineStatus::ConfirmedOnChain { .. } | PipelineStatus::Broadcasted { .. }
    )
}

// ============================================================
