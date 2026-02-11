// use kernel::types::{ClientConfig, Eip1559Transaction, ExecutionId, RelayHostError};
// use sqlx::PgPool;
// use tokio::sync::mpsc;
// use tracing::info;

// use crate::relayhost::handle::{RelayHostCommand, RelayHostHandle};

// // ============================================================
// // RelatHostEngine and spawn implimentation

// pub struct RelayHostEngine {
//     db: PgPool,
//     rx: mpsc::Receiver<RelayHostCommand>,
// }

// impl RelayHostEngine {
//     pub fn spawn(db: PgPool) -> RelayHostHandle {
//         let (tx, rx) = mpsc::channel(1024);
//         let relayhost_actor = RelayHostEngine { db, rx };

//         tokio::spawn(async move {
//             relayhost_actor.run().await;
//         });

//         RelayHostHandle::new(tx)
//     }
// }

// // ============================================================
// // RelayHostEngine functioning

// impl RelayHostEngine {
//     pub async fn run(mut self) {
//         info!("RelayHostEngine started");

//         while let Some(msg) = self.rx.recv().await {
//             match msg {
//                 RelayHostCommand::SubmitTransaction {
//                     execution_id,
//                     txn,
//                     client_config,
//                     reply_tx,
//                 } => {
//                     let result = self
//                         .handle_submit_transaction(execution_id, txn, client_config).await;
//                     let _ = reply_tx.send(result);
//                 }
//             }
//         }
//     }

//     async fn handle_submit_transaction(
//         &self,
//         execution_id: ExecutionId,
//         txn: Eip1559Transaction,
//         client_config: ClientConfig
//     ) -> Result<(), RelayHostError> {

//     }
// }
