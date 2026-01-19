use crate::types::intent::SendTransactionIntent;
use crate::types::{
    canonicalize::RawTransaction, nonce::TxNonce, state::ChainId, validate::ExecutionError,
};
use async_trait::async_trait;

#[async_trait]
pub trait Canonicalizer: Send + Sync {
    async fn canonicalize(
        &self,
        intent: &SendTransactionIntent,
        chain_id: ChainId,
        nonce: TxNonce,
    ) -> Result<RawTransaction, ExecutionError>;
}
