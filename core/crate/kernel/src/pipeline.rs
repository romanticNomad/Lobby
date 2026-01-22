use crate::{
    traits::*,
    types::{BroadcastOutcome, ExecutionError, ExecutionState, Intent, IntentResult},
};
use async_trait::async_trait;
use std::sync::Arc;

pub struct LobbyPipeline {
    state_mgr: Arc<dyn StateStore>,
    nonce_mgr: Arc<dyn NonceManager>,
    canonicalizer: Arc<dyn Canonicalizer>,
    signer: Arc<dyn Signer>,
    broadcaster: Arc<dyn Broadcaster>,
    validator: Arc<dyn Validator>,
}

#[async_trait]
impl Pipeline for LobbyPipeline {
    async fn submit(&self, intent: Intent) -> Result<IntentResult, ExecutionError> {
        // ---------------------------------------------------------------------
        // 1. Register or recover execution
        // ---------------------------------------------------------------------

        let execution = self.state_mgr.register_intent(intent).await?;
        let execution_id = execution.id;

        let existing_tx_hash = match &execution.state {
            ExecutionState::Broadcasted { tx_hash }
            | ExecutionState::PendingValidation {
                tx_hash: Some(tx_hash),
            }
            | ExecutionState::Validated { tx_hash, .. } => Some(*tx_hash),
            ExecutionState::PendingValidation { tx_hash: None } => {
                return Err(ExecutionError::Internal(
                    "execution in ambiguous broadcast state".to_string(),
                ));
            }
            ExecutionState::Failed { error } => {
                return Err(error.clone());
            }
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
                let reserved = self.nonce_mgr.reserve(chain_id, from).await?;
                self.state_mgr.record_nonce(execution_id, reserved).await?;
                reserved
            }
        };

        // ---------------------------------------------------------------------
        // 4. Encode transaction (pure, deterministic)
        // ---------------------------------------------------------------------

        let raw_tx = match execution.raw_tx {
            Some(tx) => tx,
            None => {
                let tx = self
                    .canonicalizer
                    .canonicalize(&tx_intent, chain_id, nonce)
                    .await?;

                self.state_mgr.record_raw_tx(execution_id, &tx).await?;
                tx
            }
        };

        // ---------------------------------------------------------------------
        // 5. Sign transaction (serialized per chain_id + from)
        // ---------------------------------------------------------------------

        let signed_tx = match execution.signed_tx {
            Some(tx) => tx,
            None => {
                let tx = self.signer.sign(chain_id, from, &raw_tx).await?;
                self.state_mgr.record_signed_tx(execution_id, &tx).await?;
                tx
            }
        };

        // ---------------------------------------------------------------------
        // 6. Broadcast transaction
        // ---------------------------------------------------------------------

        let tx_hash = match execution.tx_hash {
            Some(hash) => hash,
            None => match self.broadcaster.broadcast(chain_id, &signed_tx).await? {
                BroadcastOutcome::Submitted { tx_hash } => {
                    self.state_mgr
                        .mark_broadcasted(execution_id, tx_hash)
                        .await?;
                    tx_hash
                }
                BroadcastOutcome::Rejected { reason } => {
                    self.state_mgr
                        .mark_failed(execution_id, ExecutionError::BroadcastFailure)
                        .await?;
                    return Err(ExecutionError::Rejected(reason));
                }
                BroadcastOutcome::Unknown => {
                    self.state_mgr
                        .transition(
                            execution_id,
                            ExecutionState::PendingValidation { tx_hash: None },
                        )
                        .await?;
                    return Err(ExecutionError::Internal("broadcast outcome unknown".into()));
                }
            },
        };

        // ---------------------------------------------------------------------
        // 7. Transition to pending-validator
        // ---------------------------------------------------------------------

        self.state_mgr
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

        let v_state_mgr = self.state_mgr.clone();
        let validator = self.validator.clone();
        let v_nonce_mgr = self.nonce_mgr.clone();

        tokio::spawn(async move {
            let outcome = validator.watch(chain_id, tx_hash).await;
            match outcome {
                Ok(success) => {
                    let _ = v_state_mgr.mark_final(execution_id, success).await;
                    let _ = v_nonce_mgr.resolve(chain_id, from, nonce, success).await;
                }
                Err(err) => {
                    let _ = v_state_mgr.mark_failed(execution_id, err.clone()).await;
                    let _ = v_nonce_mgr.reject(chain_id, from, nonce).await;
                }
            }
        });

        // ---------------------------------------------------------------------
        // 9. Return immediate response (broadcast succeeded)
        // ---------------------------------------------------------------------

        Ok(IntentResult::TxHash(tx_hash.into()))
    }
}
