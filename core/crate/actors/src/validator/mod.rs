pub mod engine;
pub mod handle;
// pub mod rpc;

use std::time::Duration;

use async_trait::async_trait;
use kernel::{
    traits::Validator,
    types::{ChainId, ExecutionId, TxHash, ValidatorError, ValidatorOutcome},
};

// ============================================================

#[derive(Debug, Clone)]
/// Configuration for the transaction validation process.
pub struct ValidationConfig {
    /// How often to poll the RPC node for a transaction receipt.
    pub poll_interval: Duration,

    /// Maximum time to wait for a transaction to be included before giving up.
    /// After this timeout, the transaction is considered NotIncluded.
    pub timeout: Duration,

    /// Number of block confirmations required before a transaction is
    /// considered definitively included (protection against shallow reorgs).
    pub required_confirmation: u64, // default value '3' for rerorg safety.
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(3),
            timeout: Duration::from_secs(300),
            required_confirmation: 3,
        }
    }
}

// ============================================================

pub struct ValidatorStub;

#[async_trait]
impl Validator for ValidatorStub {
    async fn validate(
        &self,
        _chain_id: ChainId,
        execution_id: ExecutionId,
        tx_hash: TxHash,
    ) -> Result<ValidatorOutcome, ValidatorError> {
        tracing::info!(
            "[STUB] temp implementation: execution_id = {:?}, tx_hash = {:?}",
            execution_id.0,
            tx_hash
        );
        Ok(ValidatorOutcome::Included { block_number: 10, confirmations: 3 })
    }
}

// ============================================================
