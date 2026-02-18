use async_trait::async_trait;
use kernel::{
    traits::IntentRelay,
    types::{ClientConfig, Eip1559Transaction, ExecutionId, RelayHostError},
};
use tokio::sync::{mpsc, oneshot};

// ============================================================
// payload passed to the relayhost engine

pub enum RelayHostCommand {
    SubmitTransaction {
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
        client_config: ClientConfig,
        reply_tx: oneshot::Sender<Result<(), RelayHostError>>,
    },
}

// ============================================================
// entry point for messeges sent to relayhost engine and for app state in middleware

#[derive(Clone)]
pub struct RelayHostHandle {
    tx: mpsc::Sender<RelayHostCommand>,
}

impl RelayHostHandle {
    pub fn new(tx: mpsc::Sender<RelayHostCommand>) -> Self {
        Self { tx }
    }
}

// ============================================================
// implimenting IntentRelay for RelatHostHandler

#[async_trait]
impl IntentRelay for RelayHostHandle {
    async fn send_transaction(
        &self,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
        client_config: ClientConfig,
    ) -> Result<(), RelayHostError> {
        let (reply_tx, reply_rx) = oneshot::channel();

        let cmd = RelayHostCommand::SubmitTransaction {
            execution_id,
            txn,
            client_config,
            reply_tx,
        };

        self.tx
            .send(cmd)
            .await
            .map_err(|_| RelayHostError::ActorShutdown)?;

        reply_rx.await.map_err(|_| RelayHostError::ActorShutdown)?
    }
}

// ============================================================
