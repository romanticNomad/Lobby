use alloy_primitives::Address;
use kernel::types::{ChainId, ExecutionError, ExecutionId, TxNonce};
use sqlx::PgPool;
use tokio::sync::mpsc;

use crate::nonce::NonceCommand;

// =========================================================
// NonceActor struct declaration

pub struct NonceActor {
    db: PgPool,
    rx: mpsc::Receiver<NonceCommand>,
}

// =========================================================
// implimentations of NonceActor

impl NonceActor {

    pub fn new(db: PgPool, rx: mpsc::Receiver<NonceCommand>) -> Self {
        Self { db, rx }
    }

    // =========================================================
    // running the NonceChannel

    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                NonceCommand::Reserve {
                    chain_id,
                    from,
                    id,
                    reply,
                } => {
                    let result = self.handle_reserve(chain_id, from, id).await;
                    let _ = reply.send(result);
                }
                NonceCommand::Resolve {
                    chain_id,
                    from,
                    id,
                    outcome,
                    reply,
                } => {
                    let result = self.handle_resolve(id, outcome).await;
                    let _ = reply.send(result);
                }
            }
        }
    }

    // =========================================================
    // nonce reservation management

    async fn handle_reserve(
        &self,
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
    ) -> Result<TxNonce, ExecutionError> {
        if let Some(existing_nonce) = sqlx::query_scalar!(
            r#"
            SELECT nonce
            FROM nonce.nonce_assignments
            WHERE execution_id = $1
            "#,
            execution_id.0.as_bytes().as_slice(),
        )
        .fetch_optional(&self.db)
        .await
        .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?
        {
            return Ok(TxNonce::try_from(existing_nonce)?);
        }

        let mut candidate = {
            let max = sqlx::query_scalar!(
                r#"
                SELECT COALESCE(MAX(nonce), -1)
                FROM nonce.nonce_assignments
                WHERE chain_id = $1 AND from_address = $2
                "#,
                chain_id.0 as i64,
                from.as_fixed_bytes() as &[u8],
            )
            .fetch_one(&self.db)
            .await?;

            (max + 1) as TxNonce
        };

        loop {
            let res = sqlx::query!(
                r#"
                INSERT INTO nonce.nonce_assignments
                    (execution_id, chain_id, from_address, nonce, state)
                VALUES ($1, $2, $3, $4, 'reserved')
                "#,
                execution_id.as_bytes(),
                chain_id as i64,
                from.as_bytes(),
                candidate as i64,
            )
            .execute(&self.db)
            .await;

            match res {
                Ok(_) => {
                    return Ok(candidate);
                }

                Err(e) if is_unique_violation(&e) => {
                    // Was this execution already inserted concurrently?
                    if let Some(existing) = sqlx::query_scalar!(
                        r#"
                        SELECT nonce
                        FROM nonce.nonce_assignments
                        WHERE execution_id = $1
                        "#,
                        execution_id.as_bytes(),
                    )
                    .fetch_optional(&self.db)
                    .await?
                    {
                        return Ok(existing as TxNonce);
                    }

                    // Otherwise, active nonce collision → try next nonce
                    candidate += 1;
                    continue;
                }

                Err(e) => {
                    return Err(ExecutionError::Db(e));
                }
            }
        }
    }

    // =========================================================
    // resolving nonce state

    async fn handle_resolve(
        &self,
        execution_id: ExecutionId,
        success: bool,
    ) -> Result<(), ExecutionError> {

        let new_state = if success {
            "finalized"
        } else {
            "released"
        };

        let res = sqlx::query!(
            r#"
            UPDATE nonce.nonce_assignments
            SET state = $2
            WHERE execution_id = $1
              AND state IN ('reserved', 'inflight')
            "#,
            execution_id.as_bytes(),
            new_state,
        )
        .execute(&self.db)
        .await?;

        if res.rows_affected() == 0 {
            // Either:
            // - already terminal (OK, idempotent)
            // - or execution_id does not exist (invariant violation)

            let exists = sqlx::query_scalar!(
                r#"
                SELECT 1
                FROM nonce.nonce_assignments
                WHERE execution_id = $1
                "#,
                execution_id.0.as_bytes(),
            )
            .fetch_optional(&self.db)
            .await?;

            if exists.is_none() {
                return Err(ExecutionError::Invariant("unknown execution_id"));
            }
        }

        Ok(())
    }
}
