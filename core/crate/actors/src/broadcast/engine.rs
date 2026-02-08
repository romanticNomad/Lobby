use crate::broadcast::relay::BroadcastCommand;
use alloy::{primitives::Address, providers::Provider};
use kernel::types::{
    BroadcastError, BroadcastOutcome, ChainId, ExecutionId, RpcProviderRegistry, SignedTransaction,
    TxHash,
};
use sqlx::PgPool;
use tokio::sync::mpsc;

// =========================================================
// BroadcastEngine struct declaration with provider details

pub struct BroadcastEngine {
    db: PgPool,
    provider: RpcProviderRegistry,
    rx: mpsc::Receiver<BroadcastCommand>,
}

impl BroadcastEngine {
    pub fn new(
        db: PgPool,
        provider: RpcProviderRegistry,
        rx: mpsc::Receiver<BroadcastCommand>,
    ) -> Self {
        Self { db, provider, rx }
    }
}

// =========================================================

impl BroadcastEngine {
    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                BroadcastCommand::Broadcast {
                    chain_id,
                    from_address,
                    execution_id,
                    txn,
                    reply_tx,
                } => {
                    let result: Result<BroadcastOutcome, BroadcastError> = self
                        .handle_broadcast(chain_id, from_address, execution_id, txn)
                        .await;
                    let _ = reply_tx.send(result);
                }
            }
        }
    }

    // =========================================================
    // state logic for concurrency safe broadcasting

    async fn handle_broadcast(
        &self,
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
        txn: SignedTransaction,
    ) -> Result<BroadcastOutcome, BroadcastError> {
        // =========================================================
        // setting types for db

        let chain_id_i64: i64 = chain_id
            .0
            .try_into()
            .map_err(|_| BroadcastError::Invariant("chain_id does not fit in i64".to_string()))?;
        let from_address_bytes = &from_address.0.0;

        // =========================================================
        // idempotency safe atomic INSERT and lease locking

        let revision = sqlx::query_scalar!(
            r#"
            INSERT INTO broadcast.broadcast_requests
                (execution_id, revision, chain_id, from_address, state)
            SELECT
                $1,
                COALESCE(
                    (SELECT MAX(revision)
                    FROM broadcast.broadcast_requests
                    WHERE execution_id = $1),
                    0
                ) + 1,
                $2,
                $3,
                'received'
            WHERE NOT EXISTS (
                SELECT 1
                FROM broadcast.broadcast_requests
                WHERE execution_id = $1
                AND (
                        state = 'submitted'
                    OR (
                            state = 'received'
                        AND updated_at > now() - interval '5 minutes'
                    )
                )
            )
            RETURNING revision
            "#,
            execution_id.0.as_bytes().as_slice(),
            chain_id_i64,
            from_address_bytes,
        )
        .fetch_optional(&self.db)
        .await
        .map_err(|e| BroadcastError::DatabaseError(e.to_string()))?;

        // =========================================================
        // pattern matching of revision outcome.

        let revision = match revision {
            Some(r) => r,
            None => {
                let row = sqlx::query!(
                    r#"
                    SELECT state::TEXT, tx_hash, rejection_reason
                    FROM broadcast.broadcast_requests
                    WHERE execution_id = $1
                    ORDER BY revision DESC
                    LIMIT 1
                    "#,
                    execution_id.0.as_bytes().as_slice()
                )
                .fetch_one(&self.db)
                .await
                .map_err(|e| BroadcastError::DatabaseError(e.to_string()))?;

                return Ok(match row.state.unwrap().as_str() {
                    "submitted" => BroadcastOutcome::Submitted {
                        txn_hash: TxHash::from_slice(row.tx_hash.as_ref().ok_or(
                            BroadcastError::Invariant("Submitted without TxHash".to_string()),
                        )?),
                    },

                    "rejected" => BroadcastOutcome::Rejected {
                        reason: row
                            .rejection_reason
                            .unwrap_or_else(|| "unknows rejection reason".to_string()),
                    },

                    _ => {
                        BroadcastOutcome::Unexpected("unknown database inclusion error".to_string())
                    }
                });
            }
        };

        // =========================================================
        // fetching provider and sending transaction

        let provider = self
            .provider
            .get(&chain_id)
            .map(|entry| entry.value().clone())
            .ok_or(BroadcastError::MissingProvider(chain_id))?;

        let send_txn = provider.send_raw_transaction(&txn.rlp).await;

        // =========================================================
        // pattern matching for the broadcasted transaction

        match send_txn {
            Ok(pending_tx) => {
                let tx_hash = pending_tx.tx_hash();
                sqlx::query!(
                    r#"
                    UPDATE broadcast.broadcast_requests
                    SET state = 'submitted',
                        tx_hash = $1
                    WHERE execution_id = $2
                    AND revision = $3
                    "#,
                    tx_hash.as_slice(),
                    execution_id.0.as_bytes().as_slice(),
                    revision,
                )
                .execute(&self.db)
                .await
                .map_err(|e| BroadcastError::DatabaseError(e.to_string()))?;

                Ok(BroadcastOutcome::Submitted { txn_hash: *tx_hash })
            }
            Err(err) => {
                let err_str = err.to_string();

                let deterministic_error = err_str.contains("nonce")
                    || err_str.contains("insufficient funds")
                    || err_str.contains("invalid sender")
                    || err_str.contains("replacement transaction underpriced");

                if deterministic_error {
                    sqlx::query!(
                        r#"
                        UPDATE broadcast.broadcast_requests
                        SET state = 'rejected',
                            rejection_reason = $1
                        WHERE execution_id = $2
                        AND revision = $3
                        "#,
                        err_str,
                        execution_id.0.as_bytes().as_slice(),
                        revision
                    )
                    .execute(&self.db)
                    .await
                    .map_err(|e| BroadcastError::DatabaseError(e.to_string()))?;

                    Ok(BroadcastOutcome::Rejected { reason: err_str })
                } else {
                    Ok(BroadcastOutcome::Unexpected(err_str))
                }
            }
        }
    }
}

// =========================================================
