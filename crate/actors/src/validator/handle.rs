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
        execution_id: ExecutionId,
        tx_hash: TxHash,
        sticky_index: usize,
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
// Implementation of Validator trait for ValidatorHandle

#[async_trait]
impl Validator for ValidatorHandle {
    async fn validate(
        &self,
        chain_id: ChainId,
        execution_id: ExecutionId,
        tx_hash: TxHash,
        sticky_index: usize,
    ) -> Result<ValidatorOutcome, ValidatorError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = ValidatorCommand::Validate {
            chain_id,
            execution_id,
            tx_hash,
            sticky_index,
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
