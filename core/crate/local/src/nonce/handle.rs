use alloy::primitives::Address;
use async_trait::async_trait;
use kernel::{
    traits::NonceManager,
    types::{ChainId, ExecutionId, LocalError, TxNonce},
};
use tokio::sync::{mpsc, oneshot};

// =========================================================
// sonce state type mapping for sqlx <-> Postgres

#[derive(Debug, Clone, Copy, PartialEq, sqlx::Type)]
#[sqlx(type_name = "nonce.nonce_state", rename_all = "lowercase")]
pub enum NonceState {
    Reserved,
    Finalized,
    Released,
}

// =========================================================
// commands to sent over the channel.

pub enum NonceCommand {
    Reserve {
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
        reply: oneshot::Sender<Result<TxNonce, LocalError>>,
    },
    Resolve {
        execution_id: ExecutionId,
        outcome: bool,
        reply: oneshot::Sender<Result<(), LocalError>>,
    },
}

// =========================================================
// entry point for commands into nonce engine.

#[derive(Clone)]
pub struct NonceHandle {
    tx: mpsc::Sender<NonceCommand>,
}

impl NonceHandle {
    pub fn new(tx: mpsc::Sender<NonceCommand>) -> Self {
        Self { tx }
    }
}

// =========================================================
// implimentation of NonceManager for NonceHandle.

#[async_trait]
impl NonceManager for NonceHandle {
    async fn reserve(
        &self,
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
    ) -> Result<TxNonce, LocalError> {
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
            .map_err(|_| LocalError::Internal("NonceActor not available".to_string()))?;

        reply_rx
            .await
            .map_err(|_| LocalError::Internal("NonceActor response corrupted".to_string()))?
    }

    async fn resolve(&self, execution_id: ExecutionId, outcome: bool) -> Result<(), LocalError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = NonceCommand::Resolve {
            execution_id,
            outcome,
            reply: reply_tx,
        };

        self.tx
            .send(cmd)
            .await
            .map_err(|_| LocalError::Internal("NonceActor unavailable".into()))?;

        reply_rx
            .await
            .map_err(|_| LocalError::Internal("NonceActor dropped response".into()))?
    }
}

// =========================================================
