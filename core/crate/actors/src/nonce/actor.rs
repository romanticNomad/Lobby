// use alloy_primitives::Address;
// use kernel::types::{ChainId, TxNonce};
// use sqlx::PgPool;
// use std::collections::BTreeMap;
// use tokio::sync::mpsc;

// use crate::nonce::NonceCommand;

// pub struct NonceActor {
//     db: PgPool,
//     cursors: BTreeMap<(ChainId, Address), TxNonce>,
//     rx: mpsc::Receiver<NonceCommand>,
// }

// impl NonceActor {
//     pub async fn run(mut self) {
//         while let Some(cmd) = self.rx.recv().await {
//             match cmd {
//                 NonceCommand::Reserve { chain_id, from, id, reply } => {
//                     let result = self.handle_reserve(chain_id, from, id).await;
//                     let _ = reply.send(result);
//                 }
//                 NonceCommand::Resolve { chain_id, from, id, outcome, reply } => {
//                     let result = self.handle_resolve(chain_id, from, id, outcome).await;
//                     let _ = reply.send(result);
//                 }
//             }
//         }
//     }
// }
