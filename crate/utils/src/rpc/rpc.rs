use alloy::primitives::Address;
use alloy::providers::Provider;
use alloy::{providers::ProviderBuilder, rpc::types::TransactionReceipt};
use dashmap::DashMap;
use primitives::types::{ChainId, TxHash, ValidatorError};
use std::sync::Arc;
use std::time::Duration;

use crate::rpc::{ManagedRpcProviderRegistry, RpcEndpointRegistry};

// ============================================================
// rpc registry builder with endpoint pool support

/// Load RPC endpoints from environment variables and build endpoint pools.
///
/// Supports two formats:
/// 1. Single endpoint per chain (legacy): RPC_ENDPOINT_1=https://...
/// 2. Multiple endpoints per chain (new): RPC_ENDPOINT_1_A=https://..., RPC_ENDPOINT_1_B=https://...
///
/// Expected format:
/// ```bash
/// # Legacy single endpoint
/// RPC_ENDPOINT_1=https://eth-mainnet.g.alchemy.com/v2/KEY
///
/// # New pool format (multiple endpoints)
/// RPC_ENDPOINT_1_A=https://eth-mainnet-1.g.alchemy.com/v2/KEY
/// RPC_ENDPOINT_1_B=https://eth-mainnet-2.g.alchemy.com/v2/KEY
/// RPC_ENDPOINT_137_A=https://polygon-1.g.alchemy.com/v2/KEY
/// RPC_ENDPOINT_137_B=https://polygon-2.g.alchemy.com/v2/KEY
/// ```
///
/// Chain IDs are parsed from the suffix after `RPC_ENDPOINT_`.
///
/// # Returns
/// An `RpcEndpointRegistry` with pools containing all configured endpoints per chain.
///
/// # Panics
/// Panics if any RPC endpoint URL is invalid (cannot be parsed as a URL).
pub fn load_rpc_endpoints_from_env() -> RpcEndpointRegistry {
    use crate::rpc::{EndpointMetadata, RpcEndpointPool};
    use std::collections::HashMap;

    let registry = DashMap::new();

    // Collect endpoints grouped by chain_id -> suffix -> url
    let mut endpoint_groups: HashMap<ChainId, Vec<(String, String)>> = HashMap::new();

    for (key, value) in std::env::vars() {
        // Check for pool format: RPC_ENDPOINT_{CHAIN}_{SUFFIX}
        if let Some(rest) = key.strip_prefix("RPC_ENDPOINT_") {
            // Try to parse as pool format first: RPC_ENDPOINT_1_A
            let parts: Vec<&str> = rest.split('_').collect();

            if parts.len() == 2 {
                // Pool format: RPC_ENDPOINT_1_A
                if let Ok(chain_id) = parts[0].parse::<i64>() {
                    let chain = ChainId::try_from(chain_id).expect("Invalid chain ID");
                    let suffix = parts[1].to_string();

                    endpoint_groups
                        .entry(chain)
                        .or_default()
                        .push((suffix, value));
                }
            } else if parts.len() == 1 {
                // Legacy format: RPC_ENDPOINT_1
                if let Ok(chain_id) = parts[0].parse::<i64>() {
                    let chain = ChainId::try_from(chain_id).expect("Invalid chain ID");
                    // Use "default" as suffix for legacy format
                    endpoint_groups
                        .entry(chain)
                        .or_default()
                        .push(("default".to_string(), value));
                }
            }
        }
    }

    // Build endpoint pools from groups
    for (chain_id, endpoints) in endpoint_groups {
        let mut pool_endpoints = Vec::new();

        for (suffix, url) in endpoints {
            // Build provider
            let provider = ProviderBuilder::new().connect_http(url.parse().unwrap_or_else(|e| {
                panic!(
                    "Invalid RPC URL for chain {} endpoint {}: {} ({})",
                    chain_id, suffix, url, e
                )
            }));

            let endpoint_id = format!("{}_{}", chain_id, suffix);
            let metadata = EndpointMetadata::new(endpoint_id.clone(), url.clone());

            pool_endpoints.push((
                Arc::new(provider) as Arc<dyn Provider + Send + Sync>,
                metadata,
            ));

            tracing::debug!(
                chain_id = %chain_id,
                endpoint_id = %endpoint_id,
                url = %url,
                "RPC endpoint registered"
            );
        }

        let pool = RpcEndpointPool {
            chain_id,
            endpoints: pool_endpoints,
        };

        registry.insert(chain_id, pool);

        let endpoint_count = registry
            .get(&chain_id)
            .map(|p| p.endpoints.len())
            .unwrap_or(0);
        tracing::info!(
            chain_id = %chain_id,
            endpoint_count = endpoint_count,
            "RPC endpoint pool created"
        );
    }

    Arc::new(registry)
}

// ============================================================
// helper functions

/// Fetch the transaction receipt for `tx_hash` on the given chain.
///
/// Returns `None` if the transaction is not yet mined (still pending).
/// If `from_address` is provided, uses sticky session routing to poll the same
/// endpoint that was used for broadcasting (critical for multi-instance setups).
pub async fn get_transaction_receipt(
    registry: &ManagedRpcProviderRegistry,
    chain_id: ChainId,
    tx_hash: TxHash,
    from_address: Option<Address>,
) -> Result<Option<TransactionReceipt>, ValidatorError> {
    let provider = if let Some(addr) = from_address {
        registry
            .acquire_permit_and_select(&chain_id, Some(addr), Duration::from_secs(10))
            .await
            .map(|(_, ctx)| ctx.provider)
            .map_err(|e| ValidatorError::Rpc {
                tx_hash,
                message: format!("utils::rpc error: {:?}", e),
            })?
    } else {
        registry
            .provider(&chain_id)
            .map_err(|e| ValidatorError::Rpc {
                tx_hash,
                message: format!("utils::rpc error: {:?}", e),
            })?
    };

    let result = provider
        .get_transaction_receipt(tx_hash)
        .await
        .map_err(|e| {
            registry.record_failure("get_transaction_reciept");
            ValidatorError::Rpc {
                tx_hash,
                message: format!("utils::rpc error: {:?}", e),
            }
        })?;

    Ok(result)
}

/// Fetch the current block number on the given chain.
/// If `from_address` is provided, uses sticky session routing.
pub async fn get_block_number(
    registry: &ManagedRpcProviderRegistry,
    chain_id: ChainId,
    tx_hash: TxHash,
    from_address: Option<Address>,
) -> Result<u64, ValidatorError> {
    let provider = if let Some(addr) = from_address {
        registry
            .acquire_permit_and_select(&chain_id, Some(addr), Duration::from_secs(10))
            .await
            .map(|(_, ctx)| ctx.provider)
            .map_err(|e| ValidatorError::Rpc {
                tx_hash,
                message: format!("utils::rpc error: {:?}", e),
            })?
    } else {
        registry
            .provider(&chain_id)
            .map_err(|e| ValidatorError::Rpc {
                tx_hash,
                message: format!("utils::rpc error: {:?}", e),
            })?
    };

    let result = provider.get_block_number().await.map_err(|e| {
        registry.record_failure("get_block_number");
        ValidatorError::Rpc {
            tx_hash,
            message: format!("utils::rpc error: {:?}", e),
        }
    })?;

    Ok(result)
}

// ============================================================
