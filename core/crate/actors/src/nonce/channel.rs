use alloy_primitives::Address;
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
    Inflight,
    Finalized,
    Released,
}

// =========================================================
// Commands to send over the channel.

pub enum NonceCommand {
    Reserve {
        chain_id: ChainId,
        from: Address,
        id: ExecutionId,
        reply: oneshot::Sender<Result<TxNonce, ExecutionError>>,
    },
    Resolve {
        id: ExecutionId,
        outcome: bool,
        reply: oneshot::Sender<Result<(), ExecutionError>>,
    },
}

// =========================================================
// Entry point for Lobby into nonce actor.

#[derive(Clone)]
pub struct NonceChannel {
    tx: mpsc::Sender<NonceCommand>,
}

impl NonceChannel {
    pub fn new(tx: mpsc::Sender<NonceCommand>) -> Self {
        Self { tx }
    }
}

// =========================================================
// implimentation of NonceManager for NonceChannel.

#[async_trait]
impl NonceManager for NonceChannel {
    async fn reserve(
        &self,
        chain_id: ChainId,
        from: Address,
        id: ExecutionId,
    ) -> Result<TxNonce, ExecutionError> {
        let (reply_tx, reply_rx) = oneshot::channel();

        let cmd = NonceCommand::Reserve {
            chain_id,
            from,
            id,
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

    async fn resolve(&self, id: ExecutionId, outcome: bool) -> Result<(), ExecutionError> {
        let (reply_tx, reply_rx) = oneshot::channel();

        let cmd = NonceCommand::Resolve {
            id,
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
