use crate::nonce::{NonceCommand, NonceState};
use alloy::primitives::Address;
use kernel::types::{ChainId, ExecutionId, LocalError, SYNC_MARKER_EXECUTION_ID, TxNonce};
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
                    outcome,
                    reply,
                } => {
                    let result = self.handle_resolve(execution_id, outcome).await;
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

        // atomic INSERT with concurrency-safe nonce selection and lease locking

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
                    -- Priority 1: Released nonces
                    (SELECT MIN(nonce)
                    FROM (
                        SELECT DISTINCT ON (execution_id) 
                            nonce,
                            state
                        FROM nonce.nonce_assignments
                        WHERE chain_id = $2
                        AND from_address = $3
                        ORDER BY execution_id, revision DESC
                    ) latest_revisions
                    WHERE state = 'released'),

                    -- Priority 2: Next sequential nonce
                    (SELECT MAX(nonce)
                    FROM (
                        SELECT DISTINCT ON (execution_id)
                            nonce,
                            state
                        FROM nonce.nonce_assignments
                        WHERE chain_id = $2
                        AND from_address = $3
                        ORDER BY execution_id, revision DESC
                    ) latest_revisions
                    WHERE state IN ('reserved', 'finalized')) + 1,

                    -- Priority 3: First nonce
                    0
                ),
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

    /// ### sync with on-chain state and reserve nonce
    ///
    /// This is called when a broadcast fails with "nonce too low", indicating
    /// our database state is behind the on-chain state. We create a "sync marker"
    /// record with the on-chain nonce as 'finalized', then reserve the next nonce.
    ///
    /// This ensures race-safety: even if multiple pipelines detect nonce mismatch
    /// simultaneously, only one will successfully create the sync marker.
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

        let marker_execution_id = SYNC_MARKER_EXECUTION_ID;

        // Step 1: Create a sync marker (phantom finalized record) if we're behind
        //
        // We insert a 'finalized' record with nonce = (nonce_on_chain - 1).
        // This tells our system: "nonce N-1 has been consumed on-chain".
        //
        // The WHERE NOT EXISTS ensures we only create this marker if we don't
        // already have a finalized/reserved record >= nonce_on_chain - 1.

        let sync_marker_inserted = sqlx::query_scalar!(
            r#"
            INSERT INTO nonce.nonce_assignments
                (execution_id, revision, chain_id, from_address, nonce, state)
            SELECT
                $4,
                COALESCE(
                    (SELECT MAX(revision)
                    FROM nonce.nonce_assignments
                    WHERE execution_id = $4),
                    0
                ) + 1,         
                $1,
                $2,
                ($3::BIGINT - 1),
                'finalized'
            WHERE NOT EXISTS (
                SELECT 1
                FROM nonce.nonce_assignments
                WHERE chain_id = $1
                    AND from_address = $2
                    AND nonce >= $3 - 1
                    AND state IN ('reserved', 'finalized')
            )
            AND $3 > 0  -- Safety: don't try to finalize nonce -1
            RETURNING nonce
            "#,
            chain_id_i64,
            from_address_bytes,
            nonce_on_chain_i64,
            marker_execution_id.0.as_bytes().as_slice(),
        )
        .fetch_optional(&self.db)
        .await
        .map_err(|e| LocalError::DatabaseError(e.to_string()))?;

        if let Some(synced_nonce) = sync_marker_inserted {
            tracing::debug!(
                %chain_id,
                %from,
                synced_nonce,
                "nonce sync marker created: marked nonce {} as finalized",
                synced_nonce
            );
        } else {
            tracing::debug!(
                %chain_id,
                %from,
                "nonce sync marker not needed (state already up-to-date)"
            );
        }

        // Step 2: Reserve the on-chain nonce for this execution
        //
        // Now that we've synced our state, reserve nonce_on_chain.
        // we explicitly reserve the nonce we know is available on-chain.

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

        // pattern matching the reservation result

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
    // resolving nonce state

    async fn handle_resolve(
        &self,
        execution_id: ExecutionId,
        success: bool,
    ) -> Result<(), LocalError> {
        // loading state update

        let new_state = if success {
            NonceState::Finalized
        } else {
            NonceState::Released
        };

        // Atomic state transition

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
            new_state as NonceState,
        )
        .execute(&self.db)
        .await
        .map_err(|e| LocalError::DatabaseError(e.to_string()))?;

        // pattern matching the update result

        if result.rows_affected() > 0 {
            Ok(())
        } else {
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
            .map_err(|e| LocalError::DatabaseError(e.to_string()))?;

            match latest {
                Some(row) => {
                    if row.state == new_state {
                        Ok(())
                    } else if matches!(row.state, NonceState::Finalized | NonceState::Released) {
                        Err(LocalError::Internal(format!(
                            "execution_id already resolved to {:?}, cannot transition to {:?}",
                            row.state, new_state
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
}

// =========================================================
