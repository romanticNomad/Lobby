use crate::broadcast::BroadcastCommand;
use alloy::primitives::Address;
use primitives::types::{
    BroadcastError, BroadcastOutcome, ChainId, ExecutionId, SignedTransaction, TxHash, TxNonce,
};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use utils::rpc::{LoadBalancingStrategy, RpcClient, get_transaction_count, send_raw_transaction};

// =========================================================
// BroadcastEngine struct declaration with provider details

pub struct BroadcastEngine {
    db: PgPool,
    provider_client: Arc<RpcClient>,
    rx: mpsc::Receiver<BroadcastCommand>,
}

impl BroadcastEngine {
    pub fn new(
        db: PgPool,
        provider_client: Arc<RpcClient>,
        rx: mpsc::Receiver<BroadcastCommand>,
    ) -> Self {
        Self {
            db,
            provider_client,
            rx,
        }
    }

    /// Fetch the current nonce for an address from the RPC provider.
    /// Uses sticky session routing to maintain nonce consistency.
    async fn fetch_current_nonce(
        &self,
        chain_id: ChainId,
        from_address: Address,
        sticky_index: usize,
    ) -> Result<TxNonce, BroadcastError> {
        // configure RPC parameters
        let client = &self.provider_client;
        let strategy = LoadBalancingStrategy::StickySession { sticky_index };
        let timeout = Duration::from_secs(30); // hardcoding semaphore wait limit to 30 sec.

        let result = get_transaction_count(client, chain_id, strategy, from_address, timeout)
            .await
            .map_err(|e| BroadcastError::RpcError(format!("{:?}", e)))?;

        Ok(result)
    }
}

// =========================================================
// BroadcastEngine functioning

impl BroadcastEngine {
    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                BroadcastCommand::Broadcast {
                    chain_id,
                    from_address,
                    execution_id,
                    txn,
                    sticky_index,
                    reply_tx,
                } => {
                    let result = self
                        .handle_broadcast(chain_id, from_address, execution_id, txn, sticky_index)
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
        sticky_index: usize,
    ) -> Result<BroadcastOutcome, BroadcastError> {
        // =========================================================
        // setting types for db

        let chain_id_i64: i64 = chain_id
            .0
            .try_into()
            .map_err(|_| BroadcastError::Invariant("chain_id does not fit in i64".to_string()))?;
        let from_address_bytes = &from_address.0.0;

        // =========================================================
        // conccurency safe atomic INSERT and lease locking

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
                        AND updated_at > now() - interval '2 minutes'
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

                match row.state.unwrap().as_str() {
                    "submitted" => {
                        let txn_hash = TxHash::from_slice(row.tx_hash.as_ref().ok_or(
                            BroadcastError::Invariant(
                                "Tx hash submitted with invalid format".to_string(),
                            ),
                        )?);
                        return Ok(BroadcastOutcome { txn_hash });
                    }
                    "rejected" => {
                        return Err(BroadcastError::Rejected {
                            reason: row.rejection_reason.unwrap_or_else(|| {
                                "rejection reason unknown to database".to_string()
                            }),
                        });
                    }
                    _ => {
                        return Err(BroadcastError::DatabaseError(
                            "unknown database inclusion error".to_string(),
                        ));
                    }
                }
            }
        };

        // =========================================================
        // setting up parameters for broadcasting txn ( using eth_sendRawTransaction )

        let client = Arc::clone(&self.provider_client);
        let strategy = LoadBalancingStrategy::StickySession { sticky_index };
        let timeout = Duration::from_secs(30);
        let signed_txn = txn.rlp;

        let send_txn_result =
            send_raw_transaction(&client, chain_id, strategy, &signed_txn, timeout)
                .await
                .map_err(|e| BroadcastError::RpcError(format!("{:?}", e)));

        // =========================================================
        // pattern matching for the broadcasted transaction tracing

        match send_txn_result {
            Ok(txn_hash) => {
                sqlx::query!(
                    r#"
                    UPDATE broadcast.broadcast_requests
                    SET state = 'submitted',
                        tx_hash = $1
                    WHERE execution_id = $2
                    AND revision = $3
                    "#,
                    txn_hash.as_slice(),
                    execution_id.0.as_bytes().as_slice(),
                    revision,
                )
                .execute(&self.db)
                .await
                .map_err(|e| BroadcastError::DatabaseError(e.to_string()))?;

                Ok(BroadcastOutcome { txn_hash })
            }

            Err(err) => {
                let err_str = err.to_string();

                // None mismatch detected - Query RPC for correct nonce
                if err_str.contains("nonce") || err_str.contains("known transaction") {
                    tracing::debug!(
                        %execution_id,
                        %chain_id,
                        %from_address,
                        error = %err_str,
                        "nonce mismatch detected, querying RPC for on-chain nonce"
                    );

                    // Fetch authoritative nonce from RPC and update rejection in Db
                    let nonce_on_chain = self
                        .fetch_current_nonce(chain_id, from_address, sticky_index)
                        .await?;

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

                    return Err(BroadcastError::NonceTooLow {
                        nonce_on_chain,
                        attempted_nonce: txn.with_nonce,
                    });
                }

                // Other deterministic errors
                
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

                Err(BroadcastError::Rejected { reason: err_str })
            }
        }
    }
}

// =========================================================
