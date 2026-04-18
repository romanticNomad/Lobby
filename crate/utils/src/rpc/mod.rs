mod client;
mod metrics;
mod pool;

use crate::rpc::{
    metrics::EndpointMetrics,
    pool::{EndpointPool, RpcProviderStack},
};
use alloy::{
    primitives::{Address, TxHash, U256, bytes::Bytes},
    rpc::types::TransactionReceipt,
};
use primitives::types::{ChainId, TxNonce};
use std::{sync::Arc, time::Duration};
use tokio::sync::OwnedSemaphorePermit;

/// Re-export APIs Lobby crates
pub use client::{RpcClient, RpcExecutionResult, UnaryContext};
pub use pool::{LoadBalancingStrategy, SelectActor};

// ============================================================================
// Error Types

/// Error types for RPC handling
#[derive(Debug, thiserror::Error, Clone)]
pub enum LobbyRpcError {
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

    #[error("Rpc broadcast failed error: {0}")]
    RpcBroadcastError(String),

    #[error("Nonce fetch failed : {0}")]
    NonceFetchError(String),

    #[error("Transaction receipt fetching failed for tx_hash: {0}")]
    ReceiptFetchFailed(String),

    #[error("Block not found for transaction: {0}")]
    BlockNotFound(String),
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
pub async fn build_rpc_client() -> Result<RpcClient, LobbyRpcError> {
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
            LobbyRpcError::ProviderConstructionError(format!(
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

            // Create separate metrics instances for each actor, `id = <actor>-<chain)id>-<index>`
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
        return Err(LobbyRpcError::ProviderConstructionError(
            "No RPC_ENDPOINT_* environment variables found".to_string(),
        ));
    }

    Ok(client)
}

// ============================================================================
// Transaction Broadcasting API

/// Broadcasts a raw signed transaction to the network.
///
/// This function is designed for the **broadcast actor** and uses the broadcast
/// endpoint pool with configurable load balancing strategy.
///
/// # Arguments
/// * `client` - The RPC client instance
/// * `chain_id` - Target chain ID
/// * `strategy` - Load balancing strategy (weighted or sticky session)
/// * `raw_tx` - The signed transaction bytes (RLP encoded)
/// * `timeout` - Maximum timeout to wait for the operation
///
/// # Returns
/// The transaction hash returned by the RPC node
///
/// # Load Balancing
/// - `WeightedLeastResponseTime`: Selects endpoint based on response time and health.
/// - `StickySession`: Uses a specific `EndpointEntry` in the `EndpointPool` fetched by the `sticky_index`.
///
/// # Example
/// ```ignore
/// let send_txn_result = send_raw_transaction(
///     &client,
///     ChainId::from(1),
///     LoadBalancingStrategy::weighted(),
///     signed_tx_bytes,
///     Duration::from_secs(10),
/// ).await?;
/// ```
pub async fn send_raw_transaction(
    client: &RpcClient,
    chain_id: ChainId,
    strategy: LoadBalancingStrategy,
    signed_txn: &Bytes,
    timeout: Duration,
) -> Result<RpcExecutionResult<TxHash>, LobbyRpcError> {
    let sticky_index = match strategy {
        LoadBalancingStrategy::StickySession { sticky_index } => Some(sticky_index),
        LoadBalancingStrategy::WeightedLeastResponseTime => None,
    };

    let rpc_execution_result = client
        .execute_unary(
            SelectActor::Broadcast,
            chain_id,
            sticky_index,
            timeout,
            |provider| async move { provider.send_raw_transaction(&signed_txn).await },
        )
        .await
        .map_err(|e| LobbyRpcError::RpcBroadcastError(e.to_string()))?;

    let tx_hash = rpc_execution_result.result().tx_hash();
    let index = rpc_execution_result.index();
    Ok(RpcExecutionResult::new(*tx_hash, index))
}

// ============================================================================
// Account State API

/// Retrieves the pending transaction count (nonce) for an address.
///
/// This function is designed for the **broadcast actor** to determine the next
/// nonce for transaction signing and solve nonce_mismatch situations. Uses the `pending` block tag
/// for mempool-aware nonce calculation.
///
/// ## Arguments
/// * `client` - The RPC client instance
/// * `chain_id` - Target chain ID
/// * `strategy` - Load balancing strategy (sticky session recommended for nonce consistency)
/// * `address` - The address to query
/// * `timeout` - Maximum timeout to wait for the operation
///
/// ## Returns
/// The pending transaction count as U256
///
/// ## Sticky Session Recommendation
/// Use `LoadBalancingStrategy::sticky(index)` to ensure consistent nonce reads
/// from the same endpoint, preventing nonce collisions due to replication lag.
///
/// ## Example
/// ```ignore
/// let nonce_result = get_transaction_count(
///     &client,
///     ChainId::from(1),
///     LoadBalancingStrategy::sticky(index), // index of endpoint used for broadcasting.
///     signer_address,
///     Duration::from_secs(5),
/// ).await?;
/// ```
pub async fn get_transaction_count(
    client: &RpcClient,
    chain_id: ChainId,
    strategy: LoadBalancingStrategy,
    from_address: Address,
    timeout: Duration,
) -> Result<RpcExecutionResult<TxNonce>, LobbyRpcError> {
    let sticky_index = match strategy {
        LoadBalancingStrategy::StickySession { sticky_index } => Some(sticky_index),
        LoadBalancingStrategy::WeightedLeastResponseTime => None,
    };

    let rpc_execution_result = client
        .execute_unary(
            SelectActor::Broadcast,
            chain_id,
            sticky_index,
            timeout,
            |provider| async move { provider.get_transaction_count(from_address).pending().await },
        )
        .await
        .map_err(|e| LobbyRpcError::NonceFetchError(e.to_string()))?;

    let nonce_u64 = rpc_execution_result.result();
    let tx_nonce = TxNonce(U256::from(*nonce_u64));
    let index = rpc_execution_result.index();

    Ok(RpcExecutionResult::new(tx_nonce, index))
}

// ============================================================================
// Receipt Query API

/// Retrieves the transaction receipt for a given transaction hash.
///
/// This function is designed for the **validator actor** to confirm transaction
/// inclusion. Uses the validator endpoint pool for isolation from broadcast traffic.
///
/// # Arguments
/// * `client` - The RPC client instance
/// * `chain_id` - Target chain ID
/// * `strategy` - Load balancing strategy (sticky session recommended for nonce consistency)
/// * `tx_hash` - The transaction hash to query
/// * `timeout` - Maximum duration to wait for the operation
///
/// # Returns
/// The transaction receipt if found, None if pending or not found
///
/// # Errors
/// Returns `RpcError::ReceiptFetchFailed` if RPC returns an error.
///
/// # Note
/// Recommended to use `sticky_index` in order to avoid nonce inconsistency accross
/// different RPC nodes, which may be caused due to sync lags.
///
/// # Example
/// ```ignore
/// let receipt_result = get_transaction_receipt(
///     &client,
///     ChainId::from(1),
///     LoadBal
///     tx_hash,
///     Duration::from_secs(5),
/// ).await?;
/// ```
pub async fn get_transaction_reciept(
    client: &RpcClient,
    chain_id: ChainId,
    strategy: LoadBalancingStrategy,
    tx_hash: TxHash,
    timeout: Duration,
) -> Result<RpcExecutionResult<Option<TransactionReceipt>>, LobbyRpcError> {
    let sticky_index = match strategy {
        LoadBalancingStrategy::StickySession { sticky_index } => Some(sticky_index),
        LoadBalancingStrategy::WeightedLeastResponseTime => None,
    };

    let rpc_execution_result = client
        .execute_unary(
            SelectActor::Validator,
            chain_id,
            sticky_index,
            timeout,
            |provider| async move { provider.get_transaction_receipt(tx_hash).await },
        )
        .await
        .map_err(|e| LobbyRpcError::ReceiptFetchFailed(e.to_string()))?;

    Ok(rpc_execution_result)
}

// ============================================================================
// Block Tracking API

/// Retrieves the block number.
///
/// Designed for the **validator actor** to
/// track inclusion blocks for broadcasted transactions.
///
/// # Arguments
/// * `client` - The RPC client instance
/// * `chain_id` - Target chain ID
/// * `strategy` - Load balancing strategy (sticky session recommended for nonce consistency)
/// * `timeout` - Maximum duration to wait for the operation
///
/// # Returns
/// The block number (U256)
///
/// # Errors
/// Returns `RpcError::BlockNotFound` if receipt exists but block number is None
///
/// # Example
/// ```ignore
/// let block_num_result = get_block_number(
///     &client,
///     ChainId::from(1),
///     LoadBalancingStrategy::sticky(index),
///     Duration::from_secs(5),
/// ).await?;
/// ```
pub async fn get_block_number(
    client: &RpcClient,
    chain_id: ChainId,
    strategy: LoadBalancingStrategy,
    timeout: Duration,
) -> Result<RpcExecutionResult<U256>, LobbyRpcError> {
    let sticky_index = match strategy {
        LoadBalancingStrategy::StickySession { sticky_index } => Some(sticky_index),
        LoadBalancingStrategy::WeightedLeastResponseTime => None,
    };

    let rpc_execution_result = client
        .execute_unary(
            SelectActor::Validator,
            chain_id,
            sticky_index,
            timeout,
            |provider| async move { provider.get_block_number().await },
        )
        .await
        .map_err(|e| LobbyRpcError::BlockNotFound(e.to_string()))?;

    let block_num = rpc_execution_result.result();
    let index = rpc_execution_result.index();

    Ok(RpcExecutionResult::new(U256::from(*block_num), index))
}

// ============================================================================
// Advanced Context API (for sticky session management)

/// Acquires a unary context for advanced use cases requiring manual metric recording
/// or sticky session index management.
///
/// This is an escape hatch for actors that need:
/// - Manual control over metric recording timing
/// - Dynamic sticky index selection based on previous responses
/// - Batch operations with consistent endpoint affinity
///
/// ## Arguments
/// * `client` - The RPC client instance
/// * `actor` - SelectActor::Broadcast or SelectActor::Validator
/// * `chain_id` - Target chain ID
/// * `sticky_index` - Optional sticky session index
/// * `timeout` - Maximum duration to wait for permit acquisition
///
/// ## Returns
/// A tuple of (UnaryContext, OwnedSemaphorePermit) for executing RPC calls
///
/// ## Example
/// ```ignore
/// let (ctx, permit) = acquire_unary_context(
///     &client,
///     SelectActor::Broadcast,
///     &ChainId::from(1),
///     Some(0),
///     Duration::from_secs(5),
/// ).await?;
///
/// let result = ctx.provider().get_block_number().await;
/// ctx.record_success(start.elapsed());
/// drop(permit);
/// ```
pub async fn acquire_unary_context(
    client: &RpcClient,
    actor: SelectActor,
    chain_id: &ChainId,
    sticky_index: Option<usize>,
    timeout: Duration,
) -> Result<(UnaryContext, OwnedSemaphorePermit), LobbyRpcError> {
    client
        .acquire_unary_context(actor, chain_id, sticky_index, timeout)
        .await
}

// ============================================================================
// Endpoint Index API

/// Fetches the best endpoint index for sticky session initialization.
///
/// Designed for the `cortex` (orchestrator) to obtain a stick_index for the
/// running pipeline.
/// - uses the `EndpointPool` of `validator` since validator recieves the most traffic.
/// - obtained sticky_index is consistent over the both actors since they share identicel `EndpointRegistry`s.
///
/// ## Arguments
/// * `client` - RPC client instance
/// * `chain_id` - Target chain ID
/// * `timeout` - Maximum duration to wait for permit acquisition
///
/// ## Returns
/// Optional endpoint index if available, None if no healthy endpoints found.
///
/// ## Example
/// ```ignore
/// let index = acquire_healthy_endpoint(&client, &ChainId::from(1), Duration::from_secs(5)).await?;
/// ```
pub async fn acquire_healthy_endpoint(
    client: &RpcClient,
    chain_id: ChainId,
    timeout: Duration,
) -> Result<Option<usize>, LobbyRpcError> {
    let actor = SelectActor::Validator;
    let permit = client
        .get_provider_stack()
        .get_semaphore(timeout, &actor)
        .await?;

    // Get unary pool (lock-free DashMap read)
    let pool = client
        .get_provider_stack()
        .get_pool(&actor, chain_id)
        .ok_or_else(|| LobbyRpcError::NoEndpointsAvailable { chain_id })?;

    let index = pool.best_endpoint_index().await;
    drop(permit);

    Ok(index)
}

// ============================================================================
