use alloy_primitives::Address;
use async_trait::async_trait;
use kernel::{
    traits::Signer,
    types::{ChainId, Eip1559Transaction, ExecutionError, ExecutionId, SignedTransaction},
};
use tokio::sync::{mpsc, oneshot};

// ============================================================
// command sent over the mpsc channel

pub enum SignCommand {
    Sign {
        from: Address,
        chain_id: ChainId,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
        reply_tx: oneshot::Sender<Result<SignedTransaction, ExecutionError>>,
    },
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
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
    ) -> Result<SignedTransaction, ExecutionError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = SignCommand::Sign {
            from,
            chain_id,
            execution_id,
            txn,
            reply_tx,
        };

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
