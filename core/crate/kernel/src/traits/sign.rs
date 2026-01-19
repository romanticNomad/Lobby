use crate::types::{
    canonicalize::{RawTransaction, SignedTransaction},
    state::ChainId,
    validate::ExecutionError,
};
use alloy_primitives::Address;
use async_trait::async_trait;

#[async_trait]
pub trait Signer: Send + Sync {
    async fn sign(
        &self,
        chain_id: ChainId,
        from: Address,
        tx: &RawTransaction,
    ) -> Result<SignedTransaction, ExecutionError>;
}
