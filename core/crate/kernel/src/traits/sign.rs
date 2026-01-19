use crate::types::{
    execution::ExecutionError,
    id::ChainId,
    tx_artifacts::{RawTransaction, SignedTransaction},
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
