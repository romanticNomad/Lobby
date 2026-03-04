use crate::validator::{ValidatorConfig, handle::ValidatorCommand};
use kernel::types::{
    ChainId, ExecutionId, RpcProviderRegistry, TxHash, ValidatorError, ValidatorOutcome,
};
use sqlx::PgPool;
use tokio::{sync::mpsc, time::Instant};
use tracing::Instrument;
use utils::registry::rpc;

// ============================================================

/// Long-lived actor that validates transaction inclusion on-chain.
///
/// Receives `Validate` commands via an mpsc channel and polls the RPC node
/// for transaction receipts until the transaction is confirmed or times out.
pub struct ValidatorEngine {
    db: PgPool,
    config: ValidatorConfig,
    rpc_registry: RpcProviderRegistry,
    rx: mpsc::Receiver<ValidatorCommand>,
}

impl ValidatorEngine {
    pub fn new(
        db: PgPool,
        config: ValidatorConfig,
        rpc_registry: RpcProviderRegistry,
        rx: mpsc::Receiver<ValidatorCommand>,
    ) -> Self {
        Self {
            db,
            config,
            rpc_registry,
            rx,
        }
    }
}

// ============================================================

impl ValidatorEngine {
    /// Run the actor's event loop, processing commands until the channel closes.
    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                ValidatorCommand::Validate {
                    chain_id,
                    execution_id,
                    tx_hash,
                    reply_tx,
                } => {
                    let span = tracing::info_span!(
                        "validate",
                        %execution_id,
                        %chain_id,
                        %tx_hash,
                    );

                    let result = self
                        .handle_validation(chain_id, execution_id, tx_hash)
                        .instrument(span)
                        .await;

                    let _ = reply_tx.send(result);
                }
            }
        }

        tracing::info!("validator engine stopped");
    }

    // ============================================================

    /// validator engine handler function
    async fn handle_validation(
        &self,
        chain_id: ChainId,
        execution_id: ExecutionId,
        tx_hash: TxHash,
    ) -> Result<ValidatorOutcome, ValidatorError> {
        // idempotency check
        if let Some(cache) = self.check_cached_result(execution_id).await? {
            tracing::debug!(?cache, "cached validation result found");
            return Ok(cache);
        }

        // record validation request to db
        self.record_validation_request(chain_id, execution_id, tx_hash)
            .await?;

        // polling rpc node until timeout or confirmation
        let start = Instant::now();

        loop {
            //check timeout
            if start.elapsed() > self.config.timeout {
                tracing::warn!(
                    elapsed_time = start.elapsed().as_secs(),
                    "validation timed out"
                );
                self.record_outcome(execution_id, ValidatorOutcome::NotIncluded)
                    .await?;
                return Err(ValidatorError::Timeout {
                    chain_id,
                    tx_hash,
                    timeout_sec: start.elapsed().as_secs(),
                });
            }

            // fetch receipt
            match rpc::get_transaction_receipt(&self.rpc_registry, chain_id, tx_hash).await {
                Ok(Some(receipt)) => {
                    // transaction is mined -> check status
                    // status=0
                    if !receipt.status() {
                        tracing::warn!("transaction reverted on-chain status=0");
                        let outcome = ValidatorOutcome::NotIncluded;
                        self.record_outcome(execution_id, outcome.clone()).await?;
                        return Err(ValidatorError::Reverted { tx_hash });
                    }

                    // confirmation
                    let current_block =
                        rpc::get_block_number(&self.rpc_registry, chain_id, tx_hash).await?;
                    let tx_block = receipt.block_number.unwrap_or(0);
                    let confirmations = current_block.saturating_sub(tx_block);

                    if confirmations > self.config.required_confirmations {
                        tracing::info!(
                            block_number = tx_block,
                            confirmations,
                            "transaction confirmed"
                        );
                        let outcome = ValidatorOutcome::Included {
                            block_number: tx_block,
                            confirmations,
                        };
                        self.record_outcome(execution_id, outcome.clone()).await?;
                        return Ok(outcome);
                    } else {
                        tracing::debug!(
                            confirmations,
                            required = self.config.required_confirmations,
                            "waiting to more confirmations"
                        );
                    }
                }
                Ok(None) => {
                    // transaction not yet mined
                    tracing::debug!("receipt not found, polling rpc");
                }
                Err(e) => {
                    // RPC error — log and retry (caller will handle timeout)
                    tracing::warn!(%e, "rpc error while fetching receipt, will retry");
                }
            }

            // sleep before next poll
            tokio::time::sleep(self.config.poll_interval).await;
        }
    }

    // ============================================================
    // Db operations;

    /// Check if this execution_id has already been validated (idempotency).
    async fn check_cached_result(
        &self,
        execution_id: ExecutionId,
    ) -> Result<Option<ValidatorOutcome>, ValidatorError> {
        let row = sqlx::query!(
            r#"
            SELECT outcome
            FROM validator.validation_requests
            WHERE execution_id = $1
                AND outcome IS NOT NULL
                AND updated_at > now() - interval '5 minutes'
            ORDER BY revision DESC
            LIMIT 1
            "#,
            execution_id.0.as_bytes().as_slice()
        )
        .fetch_optional(&self.db)
        .await?;

        Ok(row.and_then(|r| match r.outcome.as_deref() {
            Some("included") => Some(ValidatorOutcome::Included {
                block_number: 0, // needs to be added to db
                confirmations: 0,
            }),
            Some("not_included") => Some(ValidatorOutcome::NotIncluded),
            _ => None,
        }))
    }

    /// Record a new validation request
    async fn record_validation_request(
        &self,
        chain_id: ChainId,
        execution_id: ExecutionId,
        tx_hash: TxHash,
    ) -> Result<(), ValidatorError> {
        let tx_hash_bytes = tx_hash.as_slice();
        let chain_id_i64: i64 = chain_id
            .0
            .try_into()
            .map_err(|_| ValidatorError::Internal("chain_id does not fit in i64".to_string()))?;

        sqlx::query!(
            r#"
            INSERT INTO validator.validation_requests
                (execution_id, revision, chain_id, tx_hash, state)
            SELECT
                $1,
                COALESCE(
                    (SELECT MAX(revision)
                     FROM validator.validation_requests
                     WHERE execution_id = $1),
                    0
                ) + 1,
                $2,
                $3,
                'pending'
            WHERE NOT EXISTS (
                SELECT 1
                FROM validator.validation_requests
                WHERE execution_id = $1
                  AND (
                      state IN ('included', 'not_included')
                      OR (
                          state = 'pending'
                          AND updated_at > now() - interval '5 minutes'
                      )
                  )
            )
            "#,
            execution_id.0.as_bytes().as_slice(),
            chain_id_i64,
            tx_hash_bytes,
        )
        .execute(&self.db)
        .await?;

        tracing::debug!("validation request recorded");
        Ok(())
    }

    /// recording validation outcome
    async fn record_outcome(
        &self,
        execution_id: ExecutionId,
        outcome: ValidatorOutcome,
    ) -> Result<(), ValidatorError> {
        let outcome_str = match outcome {
            ValidatorOutcome::Included { .. } => "included",
            ValidatorOutcome::NotIncluded => "not_included",
        };

        sqlx::query!(
            r#"
            INSERT INTO validator.validation_requests
                (execution_id, revision, chain_id, tx_hash, state, outcome)
            SELECT
                execution_id,
                MAX(revision) + 1,
                chain_id,
                tx_hash,
                $2,
                $2
            FROM validator.validation_requests
            WHERE execution_id = $1
            GROUP BY execution_id, chain_id, tx_hash
            "#,
            execution_id.0.as_bytes().as_slice(),
            outcome_str,
        )
        .execute(&self.db)
        .await?;

        tracing::debug!(%outcome_str, "validation outcome recorded");
        Ok(())
    }
}

// ============================================================
