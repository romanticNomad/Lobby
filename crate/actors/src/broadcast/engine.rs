use crate::broadcast::{BroadcastCommand, BroadcastConfig};
use alloy::primitives::Address;
use primitives::types::{
    BroadcastError, BroadcastOutcome, ChainId, ExecutionId, SignedTransaction, TxHash, TxNonce,
};
use sqlx::PgPool;
use tokio::sync::mpsc;
use utils::rpc::{ManagedRpcProviderRegistry, RpcCallContext, RpcEndpointRegistry};

// =========================================================
// BroadcastEngine struct declaration with provider details

pub struct BroadcastEngine {
    db: PgPool,
    config: BroadcastConfig,
    managed_provider: ManagedRpcProviderRegistry,
    rx: mpsc::Receiver<BroadcastCommand>,
}

impl BroadcastEngine {
    pub fn new(
        db: PgPool,
        provider: RpcEndpointRegistry,
        broadcast_config: BroadcastConfig,
        rx: mpsc::Receiver<BroadcastCommand>,
    ) -> Self {
        let managed_provider =
            ManagedRpcProviderRegistry::new(provider, broadcast_config.rpc_concurrency).unwrap();
        Self {
            db,
            config: broadcast_config,
            managed_provider,
            rx,
        }
    }

    /// Fetch the current nonce for an address from the RPC provider.
    /// Uses sticky session routing to maintain nonce consistency.
    async fn fetch_current_nonce(
        &self,
        chain_id: ChainId,
        from_address: Address,
    ) -> Result<TxNonce, BroadcastError> {
        // Use sticky session routing for nonce management
        let (permit, ctx) = self
            .managed_provider
            .acquire_permit_and_select(&chain_id, Some(from_address), self.config.rpc_timeout)
            .await
            .map_err(|_| BroadcastError::MissingProvider { chain_id })?;

        let start = std::time::Instant::now();
        let nonce_u64 = ctx
            .provider
            .get_transaction_count(from_address)
            .pending() // Use pending to get the most up-to-date nonce
            .await
            .map_err(|e| {
                self.managed_provider
                    .record_endpoint_failure(&chain_id, &ctx.endpoint_id);
                BroadcastError::Unexpected {
                    message: format!("failed to fetch nonce from RPC: {e}"),
                }
            })?;

        // Record success for metrics
        self.managed_provider
            .record_success(&chain_id, &ctx.endpoint_id, start.elapsed());

        // Permit is dropped here automatically
        drop(permit);

        Ok(TxNonce(alloy::primitives::U256::from(nonce_u64)))
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
                    reply_tx,
                } => {
                    // Acquire permit and select endpoint with sticky session
                    let result = match self
                        .managed_provider
                        .acquire_permit_and_select(
                            &chain_id,
                            Some(from_address),
                            self.config.rpc_timeout,
                        )
                        .await
                    {
                        Ok((permit, ctx)) => {
                            let result: Result<BroadcastOutcome, BroadcastError> = self
                                .handle_broadcast(chain_id, from_address, execution_id, txn, ctx)
                                .await;
                            drop(permit);
                            result
                        }
                        Err(e) => {
                            tracing::error!("Failed to acquire permit: {:?}", e);
                            Err(BroadcastError::Unexpected {
                                message: format!("semaphore error: {:?}", e),
                            })
                        }
                    };

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
        ctx: RpcCallContext,
    ) -> Result<BroadcastOutcome, BroadcastError> {
        // Track endpoint for metrics
        let endpoint_id = ctx.endpoint_id.clone();
        let provider = ctx.provider;
        let start = std::time::Instant::now();
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
        // fetching provider and sending transaction

        let send_txn = provider.send_raw_transaction(&txn.rlp).await;

        // =========================================================
        // pattern matching for the broadcasted transaction tracing

        match send_txn {
            Ok(pending_tx) => {
                let tx_hash = pending_tx.tx_hash();

                // Record success for metrics
                self.managed_provider
                    .record_success(&chain_id, &endpoint_id, start.elapsed());

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

                Ok(BroadcastOutcome { txn_hash: *tx_hash })
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

                    let nonce_on_chain = self.fetch_current_nonce(chain_id, from_address).await?;

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
                self.managed_provider
                    .record_endpoint_failure(&chain_id, &endpoint_id);

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
