mod engine;
mod handle;

use engine::*;
use handle::*;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::mpsc;
use utils::rpc::RpcClient;

// ============================================================

/// Broadcast actor initiator.
pub fn spawn_broadcast_actor(
    db: PgPool,
    provider_client: Arc<RpcClient>,
    buffer_size: usize,
) -> BroadcastHandle {
    let (tx, rx) = mpsc::channel(buffer_size);

    let broadcast_engine = BroadcastEngine::new(db, provider_client, rx);
    tokio::spawn(async move {
        broadcast_engine.run().await;
    });

    BroadcastHandle::new(tx)
}

// ============================================================
