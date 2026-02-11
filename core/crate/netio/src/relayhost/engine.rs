// use kernel::types::{ClientConfig, Eip1559Transaction, ExecutionId, RelayHostError};
// use sqlx::PgPool;
// use tokio::sync::mpsc;
// use tracing::info;
// use utils::eip1159_linter::transaction_lint;

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
//         // ============================================================
//         // idempotency check

//         let execution_id_exists = sqlx::query_scalar!(
//             r#"
//             SELECT EXISTS(
//                 SELECT 1 FROM relay_host.transaction_intents
//                 WHERE execution_id = $1
//             ) AS "exists!"
//             "#,
//             execution_id.0.as_bytes().as_slice()
//         )
//         .fetch_one(&self.db)
//         .await
//         .map_err(|e| RelayHostError::DatabaseError(e))?;
        
//         if execution_id_exists {
//             return Err(RelayHostError::DuplicateExecutionId(execution_id.0))
//         };

//         // ============================================================
//         // transaction business logic linting

//         transaction_lint(&txn)?;

//         // ============================================================
//     }
// }
