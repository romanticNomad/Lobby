mod client;
mod metrics;
mod pool;

use crate::rpc::{
    client::RpcClient,
    metrics::EndpointMetrics,
    pool::{EndpointPool, RpcProviderStack},
};
use primitives::types::ChainId;
use std::{sync::Arc, time::Duration};

// ============================================================================
// Error Types

/// Error types for RPC handling
#[derive(Debug, thiserror::Error, Clone)]
pub enum RpcError {
    #[error("Failed to acquire RPC permit within {timeout:?}")]
    PermitAcquisitionTimeout { timeout: Duration },

    #[error("Semaphore closed")]
    SemaphoreClosed,

    #[error("No RPC endpoints available for chain {chain_id}")]
    NoEndpointsAvailable { chain_id: ChainId },

    #[error("All RPC endpoints unhealthy for chain {chain_id}")]
    AllEndpointsUnhealthy { chain_id: ChainId },

    #[error("Transport error: {0}")]
    TransportError(String),

    #[error("Provider construction failed: {0}")]
    ProviderConstructionError(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
}

// ============================================================================
// Client Builder

/// Builds a complete `RpcClient` from environment variables
///
/// Scans environment for `RPC_ENDPOINT_*` variables,
/// validates URL schemes, and constructs the `RpcRegistryStack` which is the main component of `RpcClient`.
/// The `url`s are extracted from the `,` seperated format mentioned below.
///
/// ## Environment Format
/// ```bash
/// # Unary endpoints (HTTP/2, comma-separated)
/// RPC_ENDPOINT_1=<http(s) url1>,<http(s) url2>,<..>...
/// ```
///
/// ## Validation Rules
/// - Unary URLs must use `http://` or `https://` scheme
pub async fn build_rpc_client() -> Result<RpcClient, RpcError> {
    use std::env;

    let provider_stack = RpcProviderStack::new();
    let client = RpcClient::new(provider_stack);

    // Scan environment for RPC_ENDPOINT_* variables
    let mut found_endpoints = false;

    for (key, value) in env::vars() {
        if !key.starts_with("RPC_ENDPOINT_") {
            continue;
        }

        // Extract chain_id from the suffix (e.g., RPC_ENDPOINT_1 -> 1)
        let chain_id_str = key.strip_prefix("RPC_ENDPOINT_").unwrap_or("");
        let chain_id: ChainId = chain_id_str.parse().map_err(|_| {
            RpcError::ProviderConstructionError(format!(
                "Invalid chain ID in environment variable: {}",
                key
            ))
        })?;

        // Parse comma-separated URLs
        let urls: Vec<&str> = value
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        if urls.is_empty() {
            continue;
        }

        // Create separate endpoint pools for broadcast and validator actors
        let broadcast_pool = Arc::new(EndpointPool::new(chain_id));
        let validator_pool = Arc::new(EndpointPool::new(chain_id));

        // Add each endpoint to both pools (separate providers and metrics for true isolation)
        for (index, url) in urls.iter().enumerate() {
            // Create separate providers for each actor (true isolation)
            let broadcast_provider = RpcClient::create_unary_provider(url)?;
            let validator_provider = RpcClient::create_unary_provider(url)?;

            // Create separate metrics instances for each actor
            let broadcast_endpoint_id = format!("broadcast-{}-{}", chain_id, index);
            let validator_endpoint_id = format!("validator-{}-{}", chain_id, index);
            let broadcast_metrics = EndpointMetrics::new(broadcast_endpoint_id, url.to_string());
            let validator_metrics = EndpointMetrics::new(validator_endpoint_id, url.to_string());

            // Add endpoint to both pools with separate providers and metrics
            broadcast_pool
                .add_endpoint(broadcast_provider, broadcast_metrics)
                .await;
            validator_pool
                .add_endpoint(validator_provider, validator_metrics)
                .await;
        }

        // Register the chain with separate pools for each actor
        client.register_chain(chain_id, broadcast_pool, validator_pool);
        found_endpoints = true;
    }

    if !found_endpoints {
        return Err(RpcError::ProviderConstructionError(
            "No RPC_ENDPOINT_* environment variables found".to_string(),
        ));
    }

    Ok(client)
}

// ============================================================================
// API endpoints for making RPC calls.
