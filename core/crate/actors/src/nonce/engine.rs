use alloy::primitives::Address;
use kernel::types::{ChainId, ExecutionError, ExecutionId, TxNonce};
use sqlx::PgPool;
use tokio::sync::mpsc;

use crate::nonce::{NonceCommand, NonceState};

// =========================================================
// NonceEngine struct declaration

pub struct NonceEngine {
    db: PgPool,
    rx: mpsc::Receiver<NonceCommand>,
}

// =========================================================
// implimentations of NonceEngine

impl NonceEngine {
    // =========================================================
    // initiating the actor

    pub fn new(db: PgPool, rx: mpsc::Receiver<NonceCommand>) -> Self {
        Self { db, rx }
    }

    // =========================================================
    // running the NonceEngine

    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                NonceCommand::Reserve {
                    chain_id,
                    from,
                    execution_id,
                    reply,
                } => {
                    let result = self.handle_reserve(chain_id, from, execution_id).await;
                    let _ = reply.send(result);
                }
                NonceCommand::Resolve {
                    execution_id,
                    outcome,
                    reply,
                } => {
                    let result = self.handle_resolve(execution_id, outcome).await;
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
        // =========================================================
        // setting types for db

        let chain_id_i64: i64 = chain_id
            .0
            .try_into()
            .map_err(|_| ExecutionError::Invariant("chain_id does not fit in i64".to_string()))?;
        let from_address_bytes = &from.0.0;

        // =========================================================
        // atomic INSERT with concurrency-safe nonce selection and idempotency check

        let candidate = sqlx::query!(
            r#"
            INSERT INTO nonce.nonce_assignments
                (execution_id, revision, chain_id, from_address, nonce, state)
            SELECT
                $1,
                COALESCE(
                    (SELECT MAX(revision)
                    FROM nonce.nonce_assignments
                    WHERE execution_id = $1),
                    0
                ) + 1,
                $2,
                $3,
                COALESCE(
                    (SELECT MAX(nonce)
                    FROM nonce.nonce_assignments
                    WHERE chain_id = $2
                    AND from_address = $3
                    AND state IN ('reserved', 'released')),
                    -1
                ) + 1,
                'reserved'
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM nonce.nonce_assignments
                    WHERE execution_id = $1
                        AND (
                            state = 'finalized'
                            OR (
                                state = 'reserved'
                                AND updated_at > now() - interval '5 minutes'
                            )
                        )
                )
                RETURNING nonce, revision
            "#,
            execution_id.0.as_bytes().as_slice(),
            chain_id_i64,
            from_address_bytes,
        )
        .fetch_optional(&self.db)
        .await
        .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;

        // =========================================================
        // pattern matching the candidate (nonce, revision)

        match candidate {
            Some(row) => {
                Ok(TxNonce::try_from(row.nonce)?)
            }
            None => {
                let existing = sqlx::query!(
                    r#"
                    SELECT nonce, state as "state: NonceState"
                    FROM nonce.nonce_assignments
                    WHERE execution_id = $1
                        AND state IN ('reserved', 'finalized')
                    ORDER BY revision DESC
                    LIMIT 1
                    "#,
                    execution_id.0.as_bytes().as_slice()
                )
                .fetch_one(&self.db)
                .await
                .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;
                
                Ok(TxNonce::try_from(existing.nonce)?)
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
        // =========================================================
        // loading state update

        let new_state = if success {
            NonceState::Finalized
        } else {
            NonceState::Released
        };

        // =========================================================
        // Atomic state transition

        let updated = sqlx::query!(
            r#"
            INSERT INTO nonce.nonce_assignments
                (execution_id, revision, chain_id, from_address, nonce, state)
            SELECT
                $1,
                COALESCE(
                    (SELECT MAX(REVISION)
                        FROM nonce.nonce_assignments
                        WHERE execution_id = $1),
                        0
                ) + 1,
                chain_id,
                from_address,
                nonce,
                $2 as "new_state: NonceState"
            FROM nonce.nonce_assignments
            WHERE execution_id = $1
                AND state = 'reserved'
            ORDER BY revision DESC
            LIMIT 1
            ON CONFLICT (execution_id, revision) DO NOTHING 
            RETURNING revision
            "#,
            execution_id.0.as_bytes().as_slice(),
            new_state as NonceState,
        )
        .fetch_optional(&self.db)
        .await
        .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;

        // =========================================================
        // pattern matching the update result

        match updated {
            Some(_) => Ok(()),
            None => {
                // check for idempotency or error

                let latest = sqlx::query!(
                    r#"
                    SELECT state as "state: NonceState"
                    FROM nonce.nonce_assignments
                    WHERE execution_id = $1
                    ORDER BY revision DESC
                    LIMIT 1
                    "#,
                    execution_id.0.as_bytes().as_slice()
                )
                .fetch_optional(&self.db)
                .await
                .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;
                
                match latest {
                    Some(row) => {
                        if row.state == new_state {
                            Ok(())
                        } else if matches!(row.state, NonceState::Finalized | NonceState::Released) {
                            Err(ExecutionError::DatabaseError(format!("execution_id already resolved to {:?}, cannot transition to {:?}",
                        row.state, new_state)))
                        } else {
                            Err(ExecutionError::DatabaseError("expected reserved state but INSERT failed".to_string()))
                        }
                    }

                    None => {
                        Err(ExecutionError::Invariant("invalid execution_id".to_string()))
                    }
                }
            }
        }
    }
}

// =========================================================
