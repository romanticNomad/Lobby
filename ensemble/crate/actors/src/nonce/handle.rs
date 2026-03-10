use alloy::primitives::Address;
use async_trait::async_trait;
use kernel::{
    traits::NonceManager,
    types::{ChainId, ExecutionId, LocalError, TxNonce},
};
use tokio::sync::{mpsc, oneshot};

// =========================================================
// nonce state type mapping for sqlx <-> Postgres

#[derive(Debug, Clone, Copy, PartialEq, sqlx::Type)]
#[sqlx(type_name = "nonce.nonce_state", rename_all = "lowercase")]
pub enum NonceState {
    Reserved,
    Finalized,
    Released,
    Consumed,
}

// =========================================================
// commands to sent over the channel.

pub enum NonceCommand {
    Reserve {
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
        reply: oneshot::Sender<Result<TxNonce, LocalError>>,
    },
    Resolve {
        execution_id: ExecutionId,
        outcome: bool,
        reply: oneshot::Sender<Result<(), LocalError>>,
    },
    Sync {
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
        nonce_on_chain: TxNonce,
        reply: oneshot::Sender<Result<TxNonce, LocalError>>,
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
        from_address: Address,
        execution_id: ExecutionId,
    ) -> Result<TxNonce, LocalError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = NonceCommand::Reserve {
            chain_id,
            from_address,
            execution_id,
            reply: reply_tx,
        };

        self.tx
            .send(cmd)
            .await
            .map_err(|_| LocalError::Internal("nonce actor has shut down".to_owned()))?;

        reply_rx.await.map_err(|_| {
            LocalError::Internal("nonce actor has dropped the reply channel".to_owned())
        })?
    }

    async fn resolve(
        &self,
        execution_id: ExecutionId,
        outcome: bool,
    ) -> Result<(), LocalError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = NonceCommand::Resolve {
            execution_id,
            outcome,
            reply: reply_tx,
        };

        self.tx
            .send(cmd)
            .await
            .map_err(|_| LocalError::Internal("nonce actor has shut down".to_owned()))?;

        reply_rx.await.map_err(|_| {
            LocalError::Internal("nonce actor has dropped the reply channel".to_owned())
        })?
    }

    async fn sync(
        &self,
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
        nonce_on_chain: TxNonce,
    ) -> Result<TxNonce, LocalError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = NonceCommand::Sync {
            chain_id,
            from_address,
            execution_id,
            nonce_on_chain,
            reply: reply_tx,
        };

        self.tx
            .send(cmd)
            .await
            .map_err(|_| LocalError::Internal("nonce actor has shut down".to_owned()))?;

        reply_rx.await.map_err(|_| {
            LocalError::Internal("nonce actor has dropped the reply channel".to_owned())
        })?
    }
}

// =========================================================
