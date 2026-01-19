use crate::types::{
    broadcast::BroadcastOutcome, canonicalize::SignedTransaction, state::ChainId,
    validate::ExecutionError,
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
