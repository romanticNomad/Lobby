use alloy::providers::Provider;
use alloy::transports::TransportErrorKind;
use alloy::{providers::ProviderBuilder, rpc::types::TransactionReceipt};
use dashmap::DashMap;
use primitives::types::{ChainId, RpcProviderRegistry, TxHash, ValidatorError};
use std::sync::Arc;

// ============================================================
// rpc registry builder

/// Load RPC endpoints from environment variables and build a ready-to-use registry.
///
/// Expected format:
/// ```bash
/// RPC_ENDPOINT_1=https://eth-mainnet.g.alchemy.com/v2/KEY
/// RPC_ENDPOINT_137=https://polygon-mainnet.g.alchemy.com/v2/KEY
/// RPC_ENDPOINT_42161=https://arb-mainnet.g.alchemy.com/v2/KEY
/// ```
///
/// Chain IDs are parsed from the suffix after `RPC_ENDPOINT_`.
///
/// # Returns
/// An `Arc<DashMap<ChainId, Arc<dyn Provider>>>` ready to be cloned into actors.
///
/// # Panics
/// Panics if any RPC endpoint URL is invalid (cannot be parsed as a URL).
pub fn load_rpc_endpoints_from_env() -> RpcProviderRegistry {
    let registry = DashMap::new();

    for (key, value) in std::env::vars() {
        if let Some(chain_id_str) = key.strip_prefix("RPC_ENDPOINT_") {
            if let Ok(chain_id) = chain_id_str.parse::<ChainId>() {
                // build provider
                let provider =
                    ProviderBuilder::new().connect_http(value.parse().unwrap_or_else(|e| {
                        panic!("Invalid RPC URL for chain {}: {} ({})", chain_id, value, e)
                    }));

                // Wrap in Arc<dyn Provider> for type erasure and cheap cloning
                let provider_arc: Arc<dyn Provider + Send + Sync> = Arc::new(provider);

                registry.insert(chain_id, provider_arc);

                tracing::debug!(
                    ?chain_id,
                    url = %value,
                    "RPC provider registered"
                );
            } else {
                tracing::warn!(
                    key = %key,
                    "ignoring invalid chain ID in environment variable"
                );
            }
        }
    }

    Arc::new(registry)
}

// ============================================================
// helper functions

/// Fetch the transaction receipt for `tx_hash` on the given chain.
///
/// Returns `None` if the transaction is not yet mined (still pending).
pub async fn get_transaction_receipt(
    registry: &RpcProviderRegistry,
    chain_id: ChainId,
    tx_hash: TxHash,
) -> Result<Option<TransactionReceipt>, ValidatorError> {
    let provider = registry.get(&chain_id).ok_or_else(|| ValidatorError::Rpc {
        tx_hash,
        message: format!(
            "utils::rpc error: no rpc provider registered for chain_id: {}",
            chain_id
        ),
    })?;

    provider.get_transaction_receipt(tx_hash).await.map_err(
        |e: alloy::transports::RpcError<TransportErrorKind>| ValidatorError::Rpc {
            tx_hash,
            message: format!("utils::rpc error: {}", e.to_string()),
        },
    )
}

/// Fetch the current block number on the given chain.
pub async fn get_block_number(
    registry: &RpcProviderRegistry,
    chain_id: ChainId,
    tx_hash: TxHash,
) -> Result<u64, ValidatorError> {
    let provider = registry.get(&chain_id).ok_or_else(|| ValidatorError::Rpc {
        tx_hash,
        message: format!(
            "utils::rpc error: no rpc provider register for chain_id: {}",
            chain_id
        ),
    })?;

    provider.get_block_number().await.map_err(
        |e: alloy::transports::RpcError<TransportErrorKind>| ValidatorError::Rpc {
            tx_hash,
            message: format!("utils::rpc error: {}", e.to_string()),
        },
    )
}

// ============================================================
