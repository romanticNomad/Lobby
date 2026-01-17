// use std::sync::Arc;

// use async_trait::async_trait;
// use tokio::select;

// use kernel::{
//     traits::{
//         Pipeline,
//         StateStore,
//         NonceManager,
//         Signer,
//         Encoder,
//         Broadcaster,
//         FinalityWatcher,
//     },
//     types::{
//         adapter::{Intent, IntentResult, IntentError},
//         execution::{
//             ExecutionId,
//             ExecutionState,
//             SignedTransaction,
//             RawTransaction,
//         },
//     },
// };

// pub struct SinkPipeline {
//     state: Arc<dyn StateStore>,
//     nonce: Arc<dyn NonceManager>,
//     signer: Arc<dyn Signer>,
//     encoder: Arc<dyn Encoder>,
//     broadcaster: Arc<dyn Broadcaster>,
//     finality: Arc<dyn FinalityWatcher>,
// }

// #[async_trait]
// impl Pipeline for SinkPipeline {
//     async fn submit(&self, intent: Intent) -> Result<IntentResult, IntentError> {
//         // ---------------------------------------------------------------------
//         // 1. Register or recover execution
//         // ---------------------------------------------------------------------

//         let execution = self
//             .state
//             .register_intent(intent.clone())
//             .await?;

//         let execution_id = execution.id();

//         // If this execution already completed (idempotency), return immediately
//         if let Some(result) = execution.final_result() {
//             return Ok(result);
//         }

//         // ---------------------------------------------------------------------
//         // 2. Enter deterministic execution scope
//         // ---------------------------------------------------------------------

//         // This scope defines *exactly once* semantics for nonce + signing
//         // Anything past this point must be recoverable after crash
//         let chain_id = intent.chain_id();
//         let from = intent.from();

//         // ---------------------------------------------------------------------
//         // 3. Acquire nonce (serialized per chain_id + from)
//         // ---------------------------------------------------------------------

//         let nonce = match execution.nonce() {
//             Some(nonce) => nonce,
//             None => {
//                 let next = self
//                     .nonce
//                     .acquire(chain_id, from)
//                     .await?;

//                 self.state
//                     .record_nonce(execution_id, next)
//                     .await?;

//                 next
//             }
//         };

//         // ---------------------------------------------------------------------
//         // 4. Encode transaction (pure, deterministic)
//         // ---------------------------------------------------------------------

//         let raw_tx: RawTransaction = match execution.raw_transaction() {
//             Some(tx) => tx,
//             None => {
//                 let tx = self
//                     .encoder
//                     .encode(&intent, nonce)
//                     .await?;

//                 self.state
//                     .record_raw_transaction(execution_id, &tx)
//                     .await?;

//                 tx
//             }
//         };

//         // ---------------------------------------------------------------------
//         // 5. Sign transaction (serialized per chain_id + from)
//         // ---------------------------------------------------------------------

//         let signed_tx: SignedTransaction = match execution.signed_transaction() {
//             Some(tx) => tx,
//             None => {
//                 let tx = self
//                     .signer
//                     .sign(chain_id, from, &raw_tx)
//                     .await?;

//                 self.state
//                     .record_signed_transaction(execution_id, &tx)
//                     .await?;

//                 tx
//             }
//         };

//         // ---------------------------------------------------------------------
//         // 6. Broadcast transaction
//         // ---------------------------------------------------------------------

//         let tx_hash = match execution.broadcast_hash() {
//             Some(hash) => hash,
//             None => {
//                 let hash = self
//                     .broadcaster
//                     .broadcast(chain_id, &signed_tx)
//                     .await?;

//                 self.state
//                     .mark_broadcasted(execution_id, hash)
//                     .await?;

//                 hash
//             }
//         };

//         // ---------------------------------------------------------------------
//         // 7. Transition to pending-finality
//         // ---------------------------------------------------------------------

//         self.state
//             .transition(
//                 execution_id,
//                 ExecutionState::PendingFinality { tx_hash },
//             )
//             .await?;

//         // ---------------------------------------------------------------------
//         // 8. Spawn finality tracking (non-blocking)
//         // ---------------------------------------------------------------------

//         let state = self.state.clone();
//         let finality = self.finality.clone();

//         tokio::spawn(async move {
//             // Finality watcher must be fully idempotent
//             // It may run multiple times or resume after crash
//             let outcome = finality
//                 .watch(chain_id, tx_hash)
//                 .await;

//             match outcome {
//                 Ok(finality) => {
//                     let _ = state
//                         .mark_finalized(execution_id, finality)
//                         .await;
//                 }
//                 Err(err) => {
//                     let _ = state
//                         .mark_failed(execution_id, err)
//                         .await;
//                 }
//             }
//         });

//         // ---------------------------------------------------------------------
//         // 9. Return immediate response
//         // ---------------------------------------------------------------------

//         Ok(IntentResult::Submitted {
//             execution_id,
//             tx_hash,
//         })
//     }
// }
