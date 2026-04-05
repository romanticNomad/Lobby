mod engine;
mod handle;

use engine::*;
use handle::*;
use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::mpsc;
use utils::rpc::RpcEndpointRegistry;

// ============================================================

#[derive(Debug, Clone)]
///configuration for broadcast engine
pub struct BroadcastConfig {
    /// Number of allowed concurrent RPC calls
    rpc_concurrency: usize,
    /// Timeout for RPC calls
    rpc_timeout: Duration,
}

impl BroadcastConfig {
    pub fn new(rpc_concurrency: usize, rpc_timeout: Duration) -> Self {
        Self {
            rpc_concurrency,
            rpc_timeout,
        }
    }
}

impl Default for BroadcastConfig {
    fn default() -> Self {
        Self {
            rpc_concurrency: 50,
            rpc_timeout: Duration::from_secs(10),
        }
    }
}

// ============================================================

/// Broadcast actor initiator.
pub fn spawn_broadcast_actor(
    db: PgPool,
    provider: RpcEndpointRegistry,
    broadcast_config: BroadcastConfig,
    buffer_size: usize,
) -> BroadcastHandle {
    let (tx, rx) = mpsc::channel(buffer_size);

    let broadcast_engine = BroadcastEngine::new(db, provider, broadcast_config, rx);
    tokio::spawn(async move {
        broadcast_engine.run().await;
    });

    BroadcastHandle::new(tx)
}

// ============================================================
