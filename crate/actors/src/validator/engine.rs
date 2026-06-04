use crate::validator::{ValidatorConfig, handle::ValidatorCommand};
use primitives::types::{ChainId, ExecutionId, TxHash, ValidatorError, ValidatorOutcome};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::Instant};
use tracing::Instrument;
use utils::rpc::{LoadBalancingStrategy, RpcClient, get_block_number, get_transaction_receipt};

// ============================================================

/// Long-lived actor that validates transaction inclusion on-chain.
///
/// Receives `Validate` commands via an mpsc channel and polls the RPC node
/// for transaction receipts until the transaction is confirmed or times out.
pub struct ValidatorEngine {
    db: PgPool,
    provider_client: Arc<RpcClient>,
    validator_config: ValidatorConfig,
    rx: mpsc::Receiver<ValidatorCommand>,
}

impl ValidatorEngine {
    pub fn new(
        db: PgPool,
        provider_client: Arc<RpcClient>,
        validator_config: ValidatorConfig,
        rx: mpsc::Receiver<ValidatorCommand>,
    ) -> Self {
        Self {
            db,
            provider_client,
            validator_config,
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
                    sticky_index,
                    reply_tx,
                } => {
                    let span = tracing::debug_span!(
                        "validate",
                        %execution_id,
                        %chain_id,
                        %tx_hash,
                    );

                    let result = self
                        .handle_validation(chain_id, execution_id, tx_hash, sticky_index)
                        .instrument(span)
                        .await;

                    let _ = reply_tx.send(result);
                }
            }
        }

        tracing::debug!("validator engine stopped");
    }

    // ============================================================

    /// validator engine handler function
    async fn handle_validation(
        &self,
        chain_id: ChainId,
        execution_id: ExecutionId,
        tx_hash: TxHash,
        sticky_index: usize,
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

        // setting up parameters to fetch txn reciept.
        let client = Arc::clone(&self.provider_client);
        let strategy = LoadBalancingStrategy::StickySession { sticky_index };
        let timeout = Duration::from_secs(30);

        loop {
            //check timeout
            if start.elapsed() > self.validator_config.timeout {
                tracing::warn!(
                    elapsed_time = start.elapsed().as_secs(),
                    "validation timed out"
                );
                self.record_outcome(execution_id, ValidatorOutcome::Timeout)
                    .await?;
                return Ok(ValidatorOutcome::Timeout);
            }

            // fetch receipt using sticky session routing
            match get_transaction_receipt(&client, chain_id, strategy, tx_hash, timeout).await {
                Ok(Some(receipt)) => {
                    // transaction is mined -> check status
                    // status=0
                    if !receipt.status() {
                        tracing::warn!("transaction reverted on-chain status=0");
                        let outcome = ValidatorOutcome::NotIncluded;
                        self.record_outcome(execution_id, outcome.clone()).await?;
                        return Err(ValidatorError::Reverted { tx_hash });
                    }

                    // confirmation using sticky session routing
                    let current_block = get_block_number(&client, chain_id, strategy, timeout)
                        .await
                        .map_err(|e| ValidatorError::RpcError(format!("{:?}", e)))?;
                    let tx_block = receipt.block_number.unwrap_or(0);
                    let confirmations = current_block.saturating_sub(tx_block);

                    if confirmations > self.validator_config.required_confirmations {
                        tracing::debug!(
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
                            required = self.validator_config.required_confirmations,
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
                    tracing::warn!(%e, "rpc errored while fetching receipt, will retry");
                    self.record_outcome(execution_id, ValidatorOutcome::NotIncluded)
                        .await?;

                    return Err(ValidatorError::RpcError(format!("{:?}", e)));
                }
            }

            // sleep before next poll
            tokio::time::sleep(self.validator_config.poll_interval).await;
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
            SELECT state
            FROM validator.validation_requests
            WHERE execution_id = $1
            ORDER BY revision DESC
            LIMIT 1
            "#,
            execution_id.0.as_bytes().as_slice()
        )
        .fetch_optional(&self.db)
        .await?;

        Ok(row.and_then(|r| match r.state.as_str() {
            "included" => Some(ValidatorOutcome::Included {
                block_number: 0, // needs to be added to db
                confirmations: 0,
            }),

            "not_included" => Some(ValidatorOutcome::NotIncluded),
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
                        AND updated_at > now() - interval '2 minutes'
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

    /// Recording validation outcome
    async fn record_outcome(
        &self,
        execution_id: ExecutionId,
        outcome: ValidatorOutcome,
    ) -> Result<(), ValidatorError> {
        let outcome_str = match outcome {
            ValidatorOutcome::Included { .. } => "included",
            ValidatorOutcome::NotIncluded => "not_included",
            ValidatorOutcome::Timeout => "timed_out",
        };

        // Update the latest pending validation request to the final outcome
        let result = sqlx::query!(
            r#"
            UPDATE validator.validation_requests
            SET state = $2
            WHERE (execution_id, revision) = (
                SELECT execution_id, revision
                FROM validator.validation_requests
                WHERE execution_id = $1
                    AND state = 'pending'
                ORDER BY revision DESC
                LIMIT 1
                FOR UPDATE SKIP LOCKED
            )
            "#,
            execution_id.0.as_bytes().as_slice(),
            outcome_str,
        )
        .execute(&self.db)
        .await?;

        if result.rows_affected() > 0 {
            tracing::debug!(%outcome_str, "validation outcome recorded");
            Ok(())
        } else {
            // No rows updated - check why
            let existing = sqlx::query!(
                r#"
                SELECT state
                FROM validator.validation_requests
                WHERE execution_id = $1
                ORDER BY revision DESC
                LIMIT 1
                "#,
                execution_id.0.as_bytes().as_slice()
            )
            .fetch_optional(&self.db)
            .await?;

            match existing {
                Some(row) => {
                    if row.state == "included"
                        || row.state == "not_included"
                        || row.state == "timed_out"
                    {
                        Err(ValidatorError::Internal(format!(
                            "validation already completed with state '{}', cannot update to '{}'",
                            row.state, outcome_str
                        )))
                    } else {
                        Err(ValidatorError::Internal(
                            "validation should have been pending, update failed".to_string(),
                        ))
                    }
                }
                None => Err(ValidatorError::Internal(
                    "invalid execution_id, no validation request found".to_string(),
                )),
            }
        }
    }
}

// ============================================================
