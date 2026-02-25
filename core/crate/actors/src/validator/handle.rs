use async_trait::async_trait;
use kernel::{
    traits::Validator,
    types::{ChainId, ExecutionId, TxHash, ValidatorError, ValidatorOutcome},
};
use tokio::sync::{mpsc, oneshot};

// ============================================================

pub enum ValidatorCommand {
    Validate {
        chain_id: ChainId,
        execution_id: ExecutionId,
        tx_hash: TxHash,
        reply_tx: oneshot::Sender<Result<ValidatorOutcome, ValidatorError>>,
    },
}

// ============================================================

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
        execution_id: ExecutionId,
        tx_hash: TxHash,
    ) -> Result<ValidatorOutcome, ValidatorError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = ValidatorCommand::Validate {
            chain_id,
            execution_id,
            tx_hash,
            reply_tx,
        };

        self.tx
            .send(cmd)
            .await
            .map_err(|_| ValidatorError::Internal("actor shut down".to_string()))?;

        reply_rx
            .await
            .map_err(|_| ValidatorError::Internal("actor shut down".to_string()))?
    }
}

// ============================================================
