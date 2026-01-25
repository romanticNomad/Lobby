use alloy_primitives::Address;
use kernel::types::{ChainId, ExecutionError, ExecutionId, TxNonce};
use sqlx::PgPool;
use std::collections::BTreeMap;
use tokio::sync::mpsc;

use crate::nonce::NonceCommand;

// =========================================================
// NonceActor struct declaration

pub struct NonceActor {
    db: PgPool,
    cursors: BTreeMap<(ChainId, Address), TxNonce>,
    rx: mpsc::Receiver<NonceCommand>,
}

// // =========================================================
// // implimentations of NonceActor

// impl NonceActor {
//     // =========================================================
//     // running the NonceChannel

//     pub async fn run(mut self) {
//         while let Some(cmd) = self.rx.recv().await {
//             match cmd {
//                 NonceCommand::Reserve {
//                     chain_id,
//                     from,
//                     id,
//                     reply,
//                 } => {
//                     let result = self.handle_reserve(chain_id, from, id).await;
//                     let _ = reply.send(result);
//                 }
//                 NonceCommand::Resolve {
//                     chain_id,
//                     from,
//                     id,
//                     outcome,
//                     reply,
//                 } => {
//                     let result = self.handle_resolve(chain_id, from, id, outcome).await;
//                     let _ = reply.send(result);
//                 }
//             }
//         }
//     }

//     // =========================================================
//     // nonce reservation management

//     pub async fn handle_reserve(
//         &mut self,
//         chain_id: ChainId,
//         from: Address,
//         id: ExecutionId,
//     ) -> Result<TxNonce, ExecutionError> {
//     }
// }
