use crate::types::{
    state::{ChainId, TxHash},
    validate::ExecutionError,
};
use async_trait::async_trait;

#[async_trait]
pub trait Validator: Send + Sync {
    async fn watch(&self, chain_id: ChainId, tx_hash: TxHash) -> Result<bool, ExecutionError>;
}
