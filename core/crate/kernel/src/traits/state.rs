use crate::types::{
    canonicalize::*,
    intent::Intent,
    nonce::TxNonce,
    state::{Execution, ExecutionId, ExecutionState, TxHash},
    validate::ExecutionError,
};
use async_trait::async_trait;

#[async_trait]
pub trait StateStore: Send + Sync {
    async fn register_intent(&self, intent: Intent) -> Result<Execution, ExecutionError>;

    async fn record_nonce(&self, id: ExecutionId, nonce: TxNonce) -> Result<(), ExecutionError>;

    async fn record_raw_tx(
        &self,
        id: ExecutionId,
        tx: &RawTransaction,
    ) -> Result<(), ExecutionError>;

    async fn record_signed_tx(
        &self,
        id: ExecutionId,
        tx: &SignedTransaction,
    ) -> Result<(), ExecutionError>;

    async fn mark_broadcasted(
        &self,
        id: ExecutionId,
        tx_hash: TxHash,
    ) -> Result<(), ExecutionError>;

    async fn transition(
        &self,
        id: ExecutionId,
        state: ExecutionState,
    ) -> Result<(), ExecutionError>;

    async fn mark_failed(
        &self,
        id: ExecutionId,
        error: ExecutionError,
    ) -> Result<(), ExecutionError>;
}
