use crate::types::intent::SendTransactionIntent;
use crate::types::{
    execution::ExecutionError,
    id::ChainId,
    tx_artifacts::{RawTransaction, TxNonce},
};
use async_trait::async_trait;

#[async_trait]
pub trait Cannonicalizer: Send + Sync {
    async fn canonicalize(
        &self,
        intent: &SendTransactionIntent,
        chain_id: ChainId,
        nonce: TxNonce,
    ) -> Result<RawTransaction, ExecutionError>;
}
