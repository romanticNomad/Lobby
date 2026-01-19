use crate::types::{execution::ExecutionError, id::ChainId, tx_artifacts::TxHash};
use async_trait::async_trait;

#[async_trait]
pub trait Validator: Send + Sync {
    async fn watch(&self, chain_id: ChainId, tx_hash: TxHash) -> Result<bool, ExecutionError>;
}
