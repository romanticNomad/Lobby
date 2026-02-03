use alloy::primitives::Address;
use async_trait::async_trait;
use kernel::{
    traits::Broadcaster,
    types::{BroadcastError, BroadcastOutcome, ChainId, ExecutionId, SignedTransaction},
};
use tokio::sync::{mpsc, oneshot};

// ============================================================
// commands sent over the mpsc channel

pub enum BroadcastCommand {
    Broadcast {
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
        txn: SignedTransaction,
        reply_tx: oneshot::Sender<Result<BroadcastOutcome, BroadcastError>>,
    },
}

// ============================================================
// entry point for commands into BroadcastEngine

pub struct BroadcastRelay {
    tx: mpsc::Sender<BroadcastCommand>,
}

impl BroadcastRelay {
    pub fn new(tx: mpsc::Sender<BroadcastCommand>) -> Self {
        Self { tx }
    }
}

// ============================================================
// implimentation of Broadcaster for BroadcastRelay

#[async_trait]
impl Broadcaster for BroadcastRelay {
    async fn broadcast(
        &self,
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
        txn: SignedTransaction,
    ) -> Result<BroadcastOutcome, BroadcastError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = BroadcastCommand::Broadcast {
            chain_id,
            from_address,
            execution_id,
            txn,
            reply_tx,
        };

        self.tx
            .send(cmd)
            .await
            .map_err(|_| BroadcastError::Internal("BroadcastEngine not available".to_string()))?;

        reply_rx.await.map_err(|_| {
            BroadcastError::Internal("BroadcastEngine response corrupted".to_string())
        })?
    }
}

// ============================================================
