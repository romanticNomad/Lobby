use kernel::types::{ChainId, RpcProviderRegistry, TxHash, ValidatorError};
use alloy::rpc::types::TransactionReceipt;
use alloy::transports::TransportErrorKind;

// ============================================================

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
        message: format!("no rpc provider registered for chain_id: {}", chain_id)
    })?;

    provider
        .get_transaction_receipt(tx_hash)
        .await
        .map_err(|e: alloy::transports::RpcError<TransportErrorKind>| ValidatorError::Rpc {
            tx_hash,
            message: e.to_string()
        })
}

// ============================================================

/// Fetch the current block number on the given chain.
pub async fn get_block_number(
    registry: &RpcProviderRegistry,
    chain_id: ChainId,
    tx_hash: TxHash
) -> Result<u64, ValidatorError> {
    let provider = registry.get(&chain_id).ok_or_else(|| ValidatorError::Rpc {
        tx_hash,
        message: format!("no rpc provider register for chain_id: {}", chain_id)
    })?;

    provider
        .get_block_number()
        .await
        .map_err(|e: alloy::transports::RpcError<TransportErrorKind>| ValidatorError::Rpc {
            tx_hash,
            message: e.to_string()
        })
}

// ============================================================
