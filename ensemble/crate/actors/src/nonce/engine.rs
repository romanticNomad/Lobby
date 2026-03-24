use crate::nonce::NonceCommand;
use alloy::primitives::Address;
use primitives::types::{ChainId, ExecutionId, LocalError, NonceState, TxNonce};
use sqlx::PgPool;
use tokio::sync::mpsc;

// =========================================================
// NonceEngine struct declaration

pub struct NonceEngine {
    db: PgPool,
    rx: mpsc::Receiver<NonceCommand>,
}

impl NonceEngine {
    pub fn new(db: PgPool, rx: mpsc::Receiver<NonceCommand>) -> Self {
        Self { db, rx }
    }

    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                NonceCommand::Reserve {
                    chain_id,
                    from_address,
                    execution_id,
                    reply,
                } => {
                    let result = self
                        .handle_reserve(chain_id, from_address, execution_id)
                        .await;
                    let _ = reply.send(result);
                }
                NonceCommand::Resolve {
                    execution_id,
                    state,
                    reply,
                } => {
                    let result = self.handle_resolve(execution_id, state).await;
                    let _ = reply.send(result);
                }
                NonceCommand::Sync {
                    chain_id,
                    from_address,
                    execution_id,
                    nonce_on_chain,
                    reply,
                } => {
                    let result = self
                        .handle_sync(chain_id, from_address, execution_id, nonce_on_chain)
                        .await;
                    let _ = reply.send(result);
                }
            }
        }
    }

    // =========================================================
    // nonce reservation handler

    async fn handle_reserve(
        &self,
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
    ) -> Result<TxNonce, LocalError> {
        // setting types for db
        let chain_id_i64: i64 = chain_id
            .0
            .try_into()
            .map_err(|_| LocalError::Invariant("chain_id does not fit in i64".to_string()))?;
        let from_address_bytes = &from.0.0;

        // atomic INSERT with concurrency-safe nonce selection and lease locking,
        // using CTE (Common Table Expression) to simultanously update released nonce.

        let candidate = sqlx::query!(
            r#"
            WITH consumed_nonce AS (
                UPDATE nonce.nonce_assignments
                SET state = 'consumed'
                WHERE (execution_id, revision) = (
                    SELECT execution_id, revision
                    FROM nonce.nonce_assignments
                    WHERE chain_id = $2
                    AND from_address = $3
                    AND state = 'released'
                    ORDER BY nonce, execution_id, revision DESC
                    LIMIT 1
                    FOR UPDATE SKIP LOCKED
                )
                RETURNING nonce
            ),
            max_nonce AS (
                SELECT COALESCE(MAX(nonce), -1) as nonce
                FROM nonce.nonce_assignments
                WHERE chain_id = $2
                AND from_address = $3
                AND state IN ('reserved', 'finalized')
            )
            INSERT INTO nonce.nonce_assignments
                (execution_id, revision, chain_id, from_address, nonce, state)
            SELECT
                $1,
                COALESCE(
                    (SELECT MAX(revision) FROM nonce.nonce_assignments WHERE execution_id = $1),
                    0
                ) + 1,
                $2,
                $3,
                COALESCE(
                -- Priority 1: 'released' nonce
                    (
                    SELECT nonce
                    FROM consumed_nonce
                    ),
                -- Priority 2: next sequential nonce
                    (
                    SELECT nonce
                    FROM max_nonce
                    ) + 1,
                -- Priority 3: first nonce assignment
                    0
                ),
                'reserved'
            WHERE NOT EXISTS (
                SELECT 1
                FROM nonce.nonce_assignments
                WHERE execution_id = $1
                AND (state = 'finalized' 
                    OR (state = 'reserved' AND updated_at > now() - interval '2 minutes')
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
        .map_err(|e| LocalError::DatabaseError(e.to_string()))?;

        // pattern matching the candidate (nonce, revision)

        match candidate {
            Some(row) => Ok(TxNonce::try_from(row.nonce)?),
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
                .map_err(|e| LocalError::DatabaseError(e.to_string()))?;

                Ok(TxNonce::try_from(existing.nonce)?)
            }
        }
    }

    // =========================================================
    // nonce sync operation handler

    async fn handle_sync(
        &self,
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
        nonce_on_chain: TxNonce,
    ) -> Result<TxNonce, LocalError> {
        // setting types for db

        let chain_id_i64: i64 = chain_id
            .0
            .try_into()
            .map_err(|_| LocalError::Invariant("chain_id does not fit in i64".to_string()))?;
        let from_address_bytes = &from.0.0;

        let nonce_on_chain_i64: i64 = nonce_on_chain
            .0
            .try_into()
            .map_err(|_| LocalError::Invariant("nonce_on_chain does not fit in i64".to_string()))?;

        // Step 1: Reserve the on-chain nonce for this execution

        let reserved = sqlx::query!(
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
                $4,
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
            -- Ensure this nonce isn't already reserved by another execution
            AND NOT EXISTS (
                SELECT 1
                FROM nonce.nonce_assignments
                WHERE chain_id = $2
                    AND from_address = $3
                    AND nonce = $4
                    AND state = 'reserved'
                    AND updated_at > now() - interval '5 minutes'
            )
            RETURNING nonce, revision
            "#,
            execution_id.0.as_bytes().as_slice(),
            chain_id_i64,
            from_address_bytes,
            nonce_on_chain_i64,
        )
        .fetch_optional(&self.db)
        .await
        .map_err(|e| LocalError::DatabaseError(e.to_string()))?;

        // Step 2: pattern matching the reservation result

        match reserved {
            Some(row) => {
                let reserved_nonce = TxNonce::try_from(row.nonce)?;
                tracing::debug!(
                    %execution_id,
                    %chain_id,
                    %from,
                    nonce = %reserved_nonce,
                    "nonce synced and reserved after on-chain mismatch"
                );
                Ok(reserved_nonce)
            }
            None => {
                // This could happen if:
                // 1. This execution_id already has a finalized/reserved nonce (idempotency)
                // 2. Another pipeline already reserved this nonce (race condition)

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
                .fetch_optional(&self.db)
                .await
                .map_err(|e| LocalError::DatabaseError(e.to_string()))?;

                match existing {
                    Some(row) => {
                        // Idempotent: we already have a nonce for this execution_id

                        Ok(TxNonce::try_from(row.nonce)?)
                    }
                    None => {
                        // Race condition: nonce_on_chain was taken by another pipeline
                        // This is rare but possible. Fail and let orchestrator retry.

                        Err(LocalError::Rejected(format!(
                            "nonce {} was already reserved by another execution during sync",
                            nonce_on_chain
                        )))
                    }
                }
            }
        }
    }

    // =========================================================
    // nonce state resolve handler

    async fn handle_resolve(
        &self,
        execution_id: ExecutionId,
        state: NonceState,
    ) -> Result<(), LocalError> {
        match state {
            NonceState::Finalized | NonceState::Released | NonceState::Consumed => {
                // Atomic update query

                let result = sqlx::query!(
                    r#"
                    UPDATE nonce.nonce_assignments
                    SET state = $2
                    WHERE (execution_id, revision) = (
                        SELECT execution_id, revision
                        FROM nonce.nonce_assignments
                        WHERE execution_id = $1
                            AND state = 'reserved'
                        ORDER BY revision DESC
                        LIMIT 1
                        FOR UPDATE SKIP LOCKED
                    )
                    "#,
                    execution_id.0.as_bytes().as_slice(),
                    state as NonceState
                )
                .execute(&self.db)
                .await
                .map_err(|e| LocalError::DatabaseError(e.to_string()))?;

                // Verify the operation succeeded

                if result.rows_affected() > 0 {
                    Ok(())
                } else {
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
                    .map_err(|e| LocalError::DatabaseError(e.to_string()))?;

                    match latest {
                        Some(row) => {
                            if row.state == state {
                                Ok(())
                            } else if matches!(
                                row.state,
                                NonceState::Finalized | NonceState::Released | NonceState::Consumed
                            ) {
                                Err(LocalError::Internal(format!(
                                    "execution_id already resolved to {:?}, cannot transition to {:?}",
                                    row.state, state
                                )))
                            } else {
                                Err(LocalError::Invariant(
                                    "expected reserved state but UPDATE failed".to_string(),
                                ))
                            }
                        }
                        None => Err(LocalError::Invariant("invalid execution_id".to_string())),
                    }
                }
            }

            NonceState::Reserved => Err(LocalError::Internal(
                "noce should have been 'reserved' by now".to_string(),
            )),
        }
    }
}

// =========================================================
