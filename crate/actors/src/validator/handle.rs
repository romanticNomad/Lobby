use alloy::primitives::Address;
use async_trait::async_trait;
use primitives::{
    traits::Validator,
    types::{ChainId, ExecutionId, TxHash, ValidatorError, ValidatorOutcome},
};
use tokio::sync::{mpsc, oneshot};

// ============================================================

pub enum ValidatorCommand {
    Validate {
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
        tx_hash: TxHash,
        reply_tx: oneshot::Sender<Result<ValidatorOutcome, ValidatorError>>,
    },
}

// ============================================================

#[derive(Clone)]
pub struct ValidatorHandle {
    tx: mpsc::Sender<ValidatorCommand>,
}

impl ValidatorHandle {
    pub fn new(tx: mpsc::Sender<ValidatorCommand>) -> Self {
        Self { tx }
    }
}

// ============================================================

#[async_trait]
impl Validator for ValidatorHandle {
    async fn validate(
        &self,
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
        tx_hash: TxHash,
    ) -> Result<ValidatorOutcome, ValidatorError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = ValidatorCommand::Validate {
            chain_id,
            from_address,
            execution_id,
            tx_hash,
            reply_tx,
        };

        self.tx
            .send(cmd)
            .await
            .map_err(|_| ValidatorError::Internal("validator actor shutdown".to_owned()))?;

        reply_rx.await.map_err(|_| {
            ValidatorError::Internal("validator actor has droped the reply channel".to_owned())
        })?
    }
}

// ============================================================
