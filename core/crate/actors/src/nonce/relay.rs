use alloy::primitives::Address;
use async_trait::async_trait;
use kernel::{
    traits::NonceManager,
    types::{ChainId, ExecutionError, ExecutionId, TxNonce},
};
use tokio::sync::{mpsc, oneshot};

// =========================================================
// Nonce state type mapping for sqlx <-> Postgres

#[derive(Debug, Clone, Copy, sqlx::Type)]
#[sqlx(type_name = "nonce.nonce_state", rename_all = "lowercase")]
pub enum NonceState {
    Reserved,
    Finalized,
    Released,
}

// =========================================================
// Commands to send over the channel.

pub enum NonceCommand {
    Reserve {
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
        reply: oneshot::Sender<Result<TxNonce, ExecutionError>>,
    },
    Resolve {
        execution_id: ExecutionId,
        outcome: bool,
        reply: oneshot::Sender<Result<(), ExecutionError>>,
    },
}

// =========================================================
// Entry point for commands into nonce engine.

#[derive(Clone)]
pub struct NonceRelay {
    tx: mpsc::Sender<NonceCommand>,
}

impl NonceRelay {
    pub fn new(tx: mpsc::Sender<NonceCommand>) -> Self {
        Self { tx }
    }
}

// =========================================================
// implimentation of NonceManager for NonceRelay.

#[async_trait]
impl NonceManager for NonceRelay {
    async fn reserve(
        &self,
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
    ) -> Result<TxNonce, ExecutionError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = NonceCommand::Reserve {
            chain_id,
            from,
            execution_id,
            reply: reply_tx,
        };

        self.tx
            .send(cmd)
            .await
            .map_err(|_| ExecutionError::Internal("NonceActor not available".to_string()))?;

        reply_rx
            .await
            .map_err(|_| ExecutionError::Internal("NonceActor response corrupted".to_string()))?
    }

    async fn resolve(
        &self,
        execution_id: ExecutionId,
        outcome: bool,
    ) -> Result<(), ExecutionError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = NonceCommand::Resolve {
            execution_id,
            outcome,
            reply: reply_tx,
        };

        self.tx
            .send(cmd)
            .await
            .map_err(|_| ExecutionError::Internal("NonceActor unavailable".into()))?;

        reply_rx
            .await
            .map_err(|_| ExecutionError::Internal("NonceActor dropped response".into()))?
    }
}

// =========================================================
