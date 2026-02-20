pub mod engine;
pub mod handle;

pub use engine::*;
pub use handle::*;
use sqlx::PgPool;
use tokio::sync::mpsc;

// =========================================================

/// RelayHost actor initiator
pub fn spawn_relayhost_actor(db: PgPool, buffer_size: usize) -> RelayHostHandle {
    let (tx, rx) = mpsc::channel(buffer_size);

    let relayhost_engine = RelayHostEngine::new(db, rx);
    tokio::spawn(async move {
        relayhost_engine.run().await;
    });

    RelayHostHandle::new(tx)
}

// =========================================================
