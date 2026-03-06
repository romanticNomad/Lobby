use alloy::primitives::Address;
use async_trait::async_trait;
use kernel::{
    traits::Signer,
    types::{ChainId, Eip1559Transaction, ExecutionId, LocalError, SignedTransaction},
};
use tokio::sync::{mpsc, oneshot};

// =========================================================
// sign state type mapping for sqlx <-> Postgres

#[derive(Debug, Clone, Copy, PartialEq, sqlx::Type)]
#[sqlx(type_name = "sign.sign_requests", rename_all = "lowercase")]
pub enum SignState {
    Reserved,
    Signed,
    Failed,
}

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
    Revert {
        execution_id: ExecutionId,
        reply_tx: oneshot::Sender<Result<(), LocalError>>,
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
            .map_err(|_| LocalError::Internal("sign actor has shut down".to_owned()))?;

        reply_rx.await.map_err(|_| {
            LocalError::Internal("sign actor has dropped its reply channel".to_owned())
        })?
    }

    async fn revert(&self, execution_id: ExecutionId) -> Result<(), LocalError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = SignCommand::Revert {
            execution_id,
            reply_tx,
        };

        self.tx
            .send(cmd)
            .await
            .map_err(|_| LocalError::Internal("sign actor has shut down".to_owned()))?;

        reply_rx.await.map_err(|_| {
            LocalError::Internal("sign actor has dropped its reply channel".to_owned())
        })?
    }
}

// ============================================================
