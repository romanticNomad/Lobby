use alloy::primitives::Address;
use async_trait::async_trait;
use kernel::{
    traits::Signer,
    types::{ChainId, Eip1559Transaction, ExecutionId, LocalError, SignedTransaction},
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
        reply_tx: oneshot::Sender<Result<SignedTransaction, LocalError>>,
    },
}

// ============================================================
// entry point for commands into SignEngine

pub struct SignHandle {
    tx: mpsc::Sender<SignCommand>,
}

impl SignHandle {
    pub fn new(tx: mpsc::Sender<SignCommand>) -> Self {
        Self { tx }
    }
}

// ============================================================
// implimentation of Signer trait for SignHandle

#[async_trait]
impl Signer for SignHandle {
    async fn sign(
        &self,
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
    ) -> Result<SignedTransaction, LocalError> {
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
            .map_err(|_| LocalError::Internal("SignEngine not available".to_string()))?;

        reply_rx
            .await
            .map_err(|_| LocalError::Internal("SignEngine response corrupted".to_string()))?
    }
}

// ============================================================
