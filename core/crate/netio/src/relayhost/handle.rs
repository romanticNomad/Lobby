use kernel::types::{ClientConfig, Eip1559Transaction, RelayHostError};
use tokio::sync::{mpsc, oneshot};

// ============================================================
// payload passed to the relayhost engine

pub enum RelayHostCommand {
    SubmitTransaction{
        execution_id: uuid::Uuid,
        tx_payload: Eip1559Transaction,
        client_config: ClientConfig,
        reply_tx: oneshot::Sender<Result<(), RelayHostError>>
    }
}

// ============================================================

pub struct RelayHostHandle {
    tx: mpsc::Sender<RelayHostCommand>,
}

impl RelayHostHandle {
    pub fn new(tx: mpsc::Sender<RelayHostCommand>) -> Self {
        Self { tx }
    }
}

// ============================================================
