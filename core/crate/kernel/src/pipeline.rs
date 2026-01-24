use crate::{
    traits::*,
    types::{ExecutionError, ExecutionState, Intent, IntentResult},
};
use async_trait::async_trait;
use std::sync::Arc;

pub struct LobbyPipeline {
    state_mgr: Arc<dyn StateStore>,
    nonce_mgr: Arc<dyn NonceManager>,
    canonicalizer: Arc<dyn Canonicalizer>,
    signer: Arc<dyn Signer>,
    broadcaster: Arc<dyn Broadcaster>,
}

#[async_trait]
impl Pipeline for LobbyPipeline {
    async fn submit(&self, intent: Intent) -> Result<IntentResult, ExecutionError> {
        // ---------------------------------------------------------------------
        // 1. Register or recover execution (authoritative)
        // ---------------------------------------------------------------------

        let execution = self.state_mgr.register_intent(intent).await?;
        let execution_id = execution.id;

        if let Some(tx_hash) = execution.tx_hash {
            return Ok(IntentResult::TxHash(tx_hash));
        }

        let tx_intent = match execution.payload {
            Intent::SendTransaction(tx) => tx,
        };
        let chain_id = tx_intent.chain_id;
        let from = tx_intent.from;

        // ---------------------------------------------------------------------
        // 2. Resolve or reserve nonce (actor-owned)
        // ---------------------------------------------------------------------

        let nonce = match execution.nonce {
            Some(nonce) => nonce,
            None => {
                let nonce = self.nonce_mgr.reserve(chain_id, from, execution_id).await?;
                self.state_mgr.record_nonce(execution_id, nonce).await?;

                nonce
            }
        };

        // ---------------------------------------------------------------------
        // 3. Canonicalize transaction (pure)
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
        // 4. Sign transaction (still inline, serialized by signer)
        // ---------------------------------------------------------------------

        let signed_tx = match execution.signed_tx {
            Some(tx) => tx,
            None => {
                let tx = self
                    .signer
                    .sign(chain_id, from, execution_id, &raw_tx)
                    .await?;
                self.state_mgr.record_signed_tx(execution_id, &tx).await?;

                tx
            }
        };

        // ---------------------------------------------------------------------
        // 5. Submit broadcast command (actor-owned)
        // ---------------------------------------------------------------------

        self.broadcaster
            .broadcast(chain_id, execution_id, signed_tx)
            .await?;

        // ---------------------------------------------------------------------
        // 6. Transition state and return immediately
        // ---------------------------------------------------------------------

        self.state_mgr
            .transition(execution_id, ExecutionState::BroadcasteInitiated)
            .await?;

        Ok(IntentResult::Submitted(execution_id))
    }
}
