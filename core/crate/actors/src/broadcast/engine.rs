use crate::broadcast::relay::BroadcastCommand;
use alloy::primitives::Address;
use kernel::types::{BroadcastError, BroadcastOutcome, ChainId, ExecutionId, SignedTransaction, RpcProviderRegistry};
use sqlx::PgPool;
use tokio::sync::mpsc;

pub struct BroadcastEngine {
    db: PgPool,
    provider: RpcProviderRegistry,
    rx: mpsc::Receiver<BroadcastCommand>,
}

impl BroadcastEngine {
    pub fn new(db: PgPool, provider: RpcProviderRegistry, rx: mpsc::Receiver<BroadcastCommand>) -> Self {
        Self { db, provider, rx }
    }
}

impl BroadcastEngine {
    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                BroadcastCommand::Broadcast {
                    chain_id,
                    from_address,
                    execution_id,
                    txn,
                    reply_tx,
                } => {
                    let result: Result<BroadcastOutcome, BroadcastError> = self
                        .handle_broadcast(chain_id, from_address, execution_id, txn)
                        .await;
                    let _ = reply_tx.send(result);
                }
            }
        }
    }

    async fn handle_broadcast(
        &self,
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
        txn: SignedTransaction,
    ) -> Result<BroadcastOutcome, BroadcastError> {
    }
}
