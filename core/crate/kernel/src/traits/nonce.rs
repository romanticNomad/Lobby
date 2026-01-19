use crate::types::{execution::ExecutionError, id::ChainId, tx_artifacts::TxNonce};
use alloy_primitives::Address;
use async_trait::async_trait;

#[async_trait]
pub trait NonceManager: Send + Sync {
    /// Reserve the next available nonce (provisional).
    async fn reserve(&self, chain_id: ChainId, from: Address) -> Result<TxNonce, ExecutionError>;

    /// Resolve a nonce after finality (success = confirmed, false = dropped).
    async fn resolve(
        &self,
        chain_id: ChainId,
        from: Address,
        nonce: TxNonce,
        success: bool,
    ) -> Result<(), ExecutionError>;

    /// Explicitly drop a nonce (broadcast failed or abandoned).
    async fn drop(
        &self,
        chain_id: ChainId,
        from: Address,
        nonce: TxNonce,
    ) -> Result<(), ExecutionError>;
}
