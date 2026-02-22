pub mod engine;
pub mod handle;
// pub mod rpc;

use async_trait::async_trait;
use kernel::{
    traits::Validator,
    types::{ChainId, ExecutionId, TxHash, ValidationOutcome, ValidatorError},
};

// ============================================================

pub struct ValidatorStub;

#[async_trait]
impl Validator for ValidatorStub {
    async fn validate(
        &self,
        _chain_id: ChainId,
        execution_id: ExecutionId,
        tx_hash: TxHash,
    ) -> Result<ValidationOutcome, ValidatorError> {
        tracing::info!(
            "[STUB] temp implementation: execution_id = {:?}, tx_hash = {:?}",
            execution_id.0,
            tx_hash
        );
        Ok(ValidationOutcome::Included)
    }
}

// ============================================================
