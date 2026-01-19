use crate::types::{
    broadcast::BroadcastOutcome, execution::ExecutionError, id::ChainId,
    tx_artifacts::SignedTransaction,
};
use async_trait::async_trait;

#[async_trait]
pub trait Broadcaster: Send + Sync {
    async fn broadcast(
        &self,
        chain_id: ChainId,
        tx: &SignedTransaction,
    ) -> Result<BroadcastOutcome, ExecutionError>;
}
