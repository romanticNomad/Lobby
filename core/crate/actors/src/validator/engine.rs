use crate::validator::{ValidatorConfig, handle::ValidatorCommand};
use kernel::types::{
    ChainId, ExecutionId, RpcProviderRegistry, TxHash, ValidatorError, ValidatorOutcome,
};
use sqlx::PgPool;
use tokio::{sync::mpsc, time::Instant};
use tracing::Instrument;
use utils::directory::rpc;

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
        tracing::info!("validator engine started");
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
                return Err(ValidatorError::Timeout { chain_id, tx_hash, timeout_sec: start.elapsed().as_secs() });
            }

            // fetch receipt
            match rpc::get_transaction_receipt(&self.rpc_registry, chain_id, tx_hash).await {
                Ok(Some(receipt)) => {
                    // transaction is mined -> check status
                    // status=0
                    if !receipt.status() {
                        tracing::warn!("transaction reverted on-chain {status=0}");
                        let outcome = ValidatorOutcome::NotIncluded;
                        self.record_outcome(execution_id, outcome.clone()).await?;
                        return Err(ValidatorError::Reverted { tx_hash });
                    }

                    // confirmation
                    let current_block = rpc::get_block_number(&self.rpc_registry, chain_id, tx_hash).await?;
                    let tx_block = receipt.block_number.unwrap_or(0);
                    let confirmations = current_block.saturating_sub(tx_block);

                    if confirmations > self.config.required_confirmations {
                        tracing::info!(
                            block_number = tx_block,
                            confirmations,
                            "transaction confirmed"
                        );
                        let outcome = ValidatorOutcome::Included { block_number: tx_bloxk, confirmations };
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

                }
                Err(e) => {

                }
            }
        }
    }
}
