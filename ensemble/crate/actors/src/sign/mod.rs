pub mod engine;
pub mod handle;
// pub mod test;

pub use engine::*;
pub use handle::*;
use sqlx::PgPool;
use tokio::sync::mpsc;

// ============================================================

/// Sign actor initiator
pub fn spawn_sign_actor(db: PgPool, buffer_size: usize) -> SignHandle {
    let (tx, rx) = mpsc::channel(buffer_size);

    let sign_engine = SignEngine::new(db, rx);
    tokio::spawn(async move {
        sign_engine.run().await;
    });

    SignHandle::new(tx)
}

// ============================================================
