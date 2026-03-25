pub mod engine;
pub mod handle;

pub use engine::*;
pub use handle::*;
use sqlx::PgPool;
use tokio::sync::mpsc;

// =========================================================

/// Nonce actor initiator
pub fn spawn_nonce_actor(db: PgPool, buffer_size: usize) -> NonceHandle {
    let (tx, rx) = mpsc::channel(buffer_size);

    let nonce_engine = NonceEngine::new(db, rx);
    tokio::spawn(async move {
        nonce_engine.run().await;
    });

    NonceHandle::new(tx)
}

// ============================================================
