// use crate::validator::{ValidatorConfig, handle::ValidatorCommand};
// use alloy::eips::eip2124::ValidationError;
// use kernel::types::{ChainId, ExecutionId, RpcProviderRegistry, TxHash, ValidatorOutcome};
// use sqlx::PgPool;
// use tokio::sync::mpsc;
// use tracing::Instrument;

// // ============================================================

// /// Long-lived actor that validates transaction inclusion on-chain.
// ///
// /// Receives `Validate` commands via an mpsc channel and polls the RPC node
// /// for transaction receipts until the transaction is confirmed or times out.
// pub struct ValidatorEngine {
//     db: PgPool,
//     config: ValidatorConfig,
//     rpc_registry: RpcProviderRegistry,
//     rx: mpsc::Receiver<ValidatorCommand>,
// }

// impl ValidatorEngine {
//     pub fn new(
//         db: PgPool,
//         config: ValidatorConfig,
//         rpc_registry: RpcProviderRegistry,
//         rx: mpsc::Receiver<ValidatorCommand>,
//     ) -> Self {
//         Self {
//             db,
//             config,
//             rpc_registry,
//             rx,
//         }
//     }
// }

// // ============================================================
// impl ValidatorEngine {
//     pub async fn run(mut self) {
//         tracing::info!("validator engine started");
//         while let Some(cmd) = self.rx.recv().await {
//             match cmd {
//                 ValidatorCommand::Validate {
//                     chain_id,
//                     execution_id,
//                     tx_hash,
//                     reply_tx,
//                 } => {
//                     let span = tracing::info_span!(
//                         "validate",
//                         %execution_id,
//                         %chain_id,
//                         %tx_hash,
//                     );

//                     let result = self
//                         .handle_validation(chain_id, execution_id, tx_hash)
//                         .instrument(span)
//                         .await;

//                     let _ = reply_tx.send(result);
//                 }
//             }
//         }

//         tracing::info!("validator engine stopped");
//     }

//     async fn handle_validation(
//         &self,
//         chain_id: ChainId,
//         execution_id: ExecutionId,
//         tx_hash: TxHash
//     ) -> Result<ValidatorOutcome, ValidationError> {

//     }
// }
