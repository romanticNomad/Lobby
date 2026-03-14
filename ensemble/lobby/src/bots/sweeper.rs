use kernel::types::ExecutionId;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

/// sweeper bot actively looks for `reseved` nonce
/// state with expired lease.
///
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
            match expire_states(&db).await {
                Ok((count, txn_list)) => {
                    if count > 0 {
                        tracing::info!(
                            stale_states_released = count,
                            "Stale transactions: {:?}",
                            txn_list
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

// ============================================================
// helper function (handles db query)

async fn expire_states(db: &PgPool) -> Result<(usize, Vec<ExecutionId>), sqlx::Error> {
    let result = sqlx::query!(
        r#"
        UPDATE nonce.nonce_assignments
        SET state = 'released'
        WHERE (execution_id, revision) IN (
            SELECT DISTINCT ON (execution_id)
                execution_id,
                revision
            FROM nonce.nonce_assignments
            WHERE state = 'reserved'
                AND updated_at < now() - interval '5 minutes 5 seconds'
            ORDER BY execution_id, revision DESC
            LIMIT 100
        )
        RETURNING nonce, execution_id
        "#
    )
    .fetch_all(db)
    .await?;

    let count = result.len();
    let execution_id_list = result
        .into_iter()
        .map(|row| ExecutionId(Uuid::from_slice(&row.execution_id).unwrap()))
        .collect();

    Ok((count, execution_id_list))
}

// ============================================================
