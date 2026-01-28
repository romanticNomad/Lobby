use async_trait::async_trait;
use kernel::{
    traits::Signer,
    types::{ExecutionError, ExecutionId, RawTransaction, SignedTransaction},
};
use tokio::sync::{mpsc, oneshot};

// ============================================================
// command sent over the mpsc channel

pub struct SignCommand {
    id: ExecutionId,
    txn: RawTransaction,
    reply_tx: oneshot::Sender<Result<SignedTransaction, ExecutionError>>,
}

// ============================================================
// Entry point for commands into SignEngine

pub struct SignRelay {
    tx: mpsc::Sender<SignCommand>,
}

impl SignRelay {
    fn new(tx: mpsc::Sender<SignCommand>) -> Self {
        Self { tx }
    }
}

// ============================================================
// implimentation of Signer trait for SignRelay

#[async_trait]
impl Signer for SignRelay {
    async fn sign(
        &self,
        id: ExecutionId,
        txn: RawTransaction,
    ) -> Result<SignedTransaction, ExecutionError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = SignCommand { id, txn, reply_tx };

        self.tx
            .send(cmd)
            .await
            .map_err(|_| ExecutionError::Internal("SignEngine not available".to_string()))?;

        reply_rx
            .await
            .map_err(|_| ExecutionError::Internal("SignEngine response corrupted".to_string()))?
    }
}

// ============================================================
