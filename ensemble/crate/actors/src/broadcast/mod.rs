mod engine;
mod handle;

use engine::*;
use handle::*;
use kernel::types::RpcProviderRegistry;
use sqlx::PgPool;
use tokio::sync::mpsc;

// ============================================================

/// Broadcast actor initiator.
pub fn spawn_broadcast_actor(
    db: PgPool,
    provider: RpcProviderRegistry,
    buffer_size: usize,
) -> BroadcastHandle {
    let (tx, rx) = mpsc::channel(buffer_size);

    let broadcast_engine = BroadcastEngine::new(db, provider, rx);
    tokio::spawn(async move {
        broadcast_engine.run().await;
    });

    BroadcastHandle::new(tx)
}

// ============================================================
