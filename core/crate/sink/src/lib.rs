use async_trait::async_trait;
use kernel::{
    traits::*,
    types::{
        BroadcastOutcome, ExecutionError, ExecutionState, Intent, IntentResult, RawTransaction,
        SignedTransaction, TxHash,
    },
};
use std::sync::Arc;

pub struct SinkPipeline {
    state: Arc<dyn StateStore>,
    nonce: Arc<dyn NonceManager>,
    canonicalize: Arc<dyn Canonicalizer>,
    signe: Arc<dyn Signer>,
    broadcaste: Arc<dyn Broadcaster>,
    validate: Arc<dyn Validator>,
}

#[async_trait]
impl Pipeline for SinkPipeline {
    async fn submit(&self, intent: Intent) -> Result<IntentResult, ExecutionError> {

        // ---------------------------------------------------------------------
        // 1. Register or recover execution
        // ---------------------------------------------------------------------

        let execution = self.state.register_intent(intent).await?;
        let execution_id = execution.id;

        // If this execution already completed (idempotency), return immediately
        let existing_tx_hash: Option<TxHash> = match &execution.state {

            // Already broadcast (or beyond) — return immediately
            ExecutionState::Broadcasted { tx_hash }
            | ExecutionState::PendingValidation { tx_hash: Some(tx_hash) }
            | ExecutionState::Validated { tx_hash, .. } => {
                Some(*tx_hash)
            }

            // Ambiguous broadcast: cannot safely return success
            ExecutionState::PendingValidation { tx_hash: None } => {
                return Err(ExecutionError::Internal(
                    "execution in ambiguous broadcast state".to_string(),
                ));
            }

            // Terminal failure before broadcast — propagate error
            ExecutionState::Failed { error } => {
                return Err(error.clone());
            }

            // Otherwise: execution is incomplete, continue pipeline
            _ => None,
        };

        if let Some(tx_hash) = existing_tx_hash {
            return Ok(IntentResult::TxHash(tx_hash));
        };

        // ---------------------------------------------------------------------
        // 2. Enter deterministic execution scope
        // ---------------------------------------------------------------------

        let tx_intent = match &execution.payload {
            Intent::SendTransaction(tx) => tx,
        };
        
        let chain_id = tx_intent.chain_id;
        let from = tx_intent.from;

        // ---------------------------------------------------------------------
        // 3. Reserve nonce (provisional, not Validated)
        // ---------------------------------------------------------------------

        let nonce = match execution.nonce {
            Some(nonce) => nonce,
            None => {
                let reserved = self.nonce.reserve(chain_id, from).await?; 

                self.state.record_nonce(execution_id, reserved).await?;

                reserved
            }
        };

        // ---------------------------------------------------------------------
        // 4. Encode transaction (pure, deterministic)
        // ---------------------------------------------------------------------

        let raw_tx: RawTransaction = match execution.raw_tx {
            Some(tx) => tx,
            None => {
                let tx = self.canonicalize.canonicalize(&tx_intent, chain_id, nonce).await?;

                self.state.record_raw_tx(execution_id, &tx).await?;

                tx
            }
        };

        // ---------------------------------------------------------------------
        // 5. Sign transaction (serialized per chain_id + from)
        // ---------------------------------------------------------------------

        let signed_tx: SignedTransaction = match execution.signed_tx {
            Some(tx) => tx,
            None => {
                let tx = self.signe.sign(chain_id, from, &raw_tx).await?;

                self.state
                    .record_signed_tx(execution_id, &tx)
                    .await?;

                tx
            }
        };

        // ---------------------------------------------------------------------
        // 6. Broadcast transaction
        // ---------------------------------------------------------------------

        let tx_hash = match execution.tx_hash {
            Some(hash) => hash,
            None => {
                match self.broadcaste.broadcast(chain_id, &signed_tx).await? {
                    BroadcastOutcome::Submitted { tx_hash } => {
                        self.state.mark_broadcasted(execution_id, tx_hash).await?;
                        tx_hash
                    }

                    BroadcastOutcome::Rejected { reason } => {
                        self.state
                            .mark_failed(execution_id, ExecutionError::BroadcastFailure)
                            .await?;

                        return Err(ExecutionError::Rejected(reason));
                    }

                    BroadcastOutcome::Unknown => {
                        // Assume broadcast happened; we must not double-send
                        self.state
                            .transition(
                                execution_id,
                                ExecutionState::PendingValidation { tx_hash: None },
                            )
                            .await?;

                        // We do NOT have a tx hash, so submit cannot succeed
                        return Err(ExecutionError::Internal("broadcast outcome unknown".into()));
                    }
                }
            }
        };
        // ---------------------------------------------------------------------
        // 7. Transition to pending-validator
        // ---------------------------------------------------------------------

        self.state
            .transition(
                execution_id,
                ExecutionState::PendingValidation {
                    tx_hash: Some(tx_hash),
                },
            )
            .await?;

        // ---------------------------------------------------------------------
        // 8. Spawn validator tracking (non-blocking, resolves nonce)
        // ---------------------------------------------------------------------

        let state = self.state.clone();
        let validator = self.validate.clone();
        let nonce_mgr = self.nonce.clone();

        tokio::spawn(async move {
            let outcome = validator.watch(chain_id, tx_hash).await;

            match outcome {
                Ok(success) => {
                    // Update execution state
                    let _ = state.mark_final(execution_id, success).await;

                    // Resolve nonce based on outcome
                    let _ = nonce_mgr.resolve(chain_id, from, nonce, success).await;
                }

                Err(err) => {
                    // Mark execution failed
                    let _ = state.mark_failed(execution_id, err.clone()).await;

                    // Mark nonce as dropped / replaceable
                    let _ = nonce_mgr.reject(chain_id, from, nonce).await;
                }
            }
        });

        // ---------------------------------------------------------------------
        // 9. Return immediate response (broadcast succeeded)
        // ---------------------------------------------------------------------

        Ok(IntentResult::TxHash(tx_hash.into()))
    }
}
