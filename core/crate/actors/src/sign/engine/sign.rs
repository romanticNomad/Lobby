// use crate::sign::SignCommand;
// use alloy_primitives::Address;
// use kernel::types::{ChainId, Eip1559Transaction, ExecutionError, ExecutionId, SignedTransaction};
// use sqlx::PgPool;
// use tokio::sync::mpsc;

// // ============================================================

// pub struct SignEngine {
//     db: PgPool,
//     rx: mpsc::Receiver<SignCommand>,
// }

// impl SignEngine {
//     pub fn new(db: PgPool, rx: mpsc::Receiver<SignCommand>) -> Self {
//         Self { db, rx }
//     }
// }

// // ============================================================

// impl SignEngine {
//     // the type of 'rx' contains the information of cmd.

//     pub async fn run(mut self) {
//         while let Some(cmd) = self.rx.recv().await {
//             match cmd {
//                 SignCommand::Sign {
//                     from,
//                     chain_id,
//                     id,
//                     txn,
//                     reply_tx,
//                 } => {
//                     let result = self.handle_sign(from, chain_id, id, txn).await;
//                     let _ = reply_tx.send(result);
//                 }
//             }
//         }
//     }

//     // state management and signing raw transaction.

//     async fn handle_sign(
//         &self,
//         from: Address,
//         chain_id: ChainId,
//         id: ExecutionId,
//         txn: Eip1559Transaction,
//     ) -> Result<SignedTransaction, ExecutionError> {
//     }
// }
