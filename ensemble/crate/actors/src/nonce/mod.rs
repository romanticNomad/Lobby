pub mod engine;
pub mod handle;

pub use engine::*;
pub use handle::*;
use sqlx::PgPool;
use std::time::Duration;
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

/// sweeper bot actively looks for `reseved` nonce
/// state with expired lease.
/// such states may accumulate due to:
/// * mid transaction, system failiure
/// * database malfunctions
///
/// primary function of the sweeper bot is to
/// update such curropted states to `released`, hence filling
/// the nonce gap created by them.
pub fn spawn_sweeper_bot(db: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));

        loop {
            interval.tick().await;
            match expire_state_lease(&db).await {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!(
                            expired_lease = count,
                            "Background lease expiration completed"
                        )
                    }
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        "Background lease expiration failed"
                    )
                }
            }
        }
    });
}

async fn expire_state_lease(db: &PgPool) -> Result<usize, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        INSERT INTO nonce.nonce_assignments
            (execution_id, revision, chain_id, from_address, nonce, state, created_at, updated_at)
        SELECT
            execution_id,
            MAX(revision) + 1,
            chain_id,
            from_address,
            nonce,
            'released',
            now(),
            now()
        FROM nonce.nonce_assignments
        WHERE state = 'reserved'
            AND updated_at <= now() - interval '5 minutes 5 seconds'
        GROUP BY execution_id, chain_id, from_address, nonce
        LIMIT 100
        RETURNING nonce
        "#
    )
    .fetch_all(db)
    .await?;

    Ok(result.len())
}

// ============================================================
