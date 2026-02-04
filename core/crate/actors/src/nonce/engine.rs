// use alloy::primitives::Address;
// use kernel::types::{ChainId, ExecutionError, ExecutionId, TxNonce};
// use sqlx::{PgPool, postgres::PgDatabaseError};
// use tokio::sync::mpsc;

// use crate::nonce::{NonceCommand, NonceState};

// // =========================================================
// // NonceEngine struct declaration

// pub struct NonceEngine {
//     db: PgPool,
//     rx: mpsc::Receiver<NonceCommand>,
// }

// // =========================================================
// // implimentations of NonceEngine

// impl NonceEngine {
//     // =========================================================
//     // initiating the actor

//     pub fn new(db: PgPool, rx: mpsc::Receiver<NonceCommand>) -> Self {
//         Self { db, rx }
//     }

//     // =========================================================
//     // running the NonceEngine

//     pub async fn run(mut self) {
//         while let Some(cmd) = self.rx.recv().await {
//             match cmd {
//                 NonceCommand::Reserve {
//                     chain_id,
//                     from,
//                     execution_id,
//                     reply,
//                 } => {
//                     let result = self.handle_reserve(chain_id, from, execution_id).await;
//                     let _ = reply.send(result);
//                 }
//                 NonceCommand::Resolve {
//                     execution_id,
//                     outcome,
//                     reply,
//                 } => {
//                     let result = self.handle_resolve(execution_id, outcome).await;
//                     let _ = reply.send(result);
//                 }
//             }
//         }
//     }

//     // =========================================================
//     // nonce reservation management

//     async fn handle_reserve(
//         &self,
//         chain_id: ChainId,
//         from: Address,
//         execution_id: ExecutionId,
//     ) -> Result<TxNonce, ExecutionError> {
//         // =========================================================
//         // setting types for db

//         let chain_id_i64: i64 = chain_id
//             .0
//             .try_into()
//             .map_err(|_| ExecutionError::Invariant("chain_id does not fit in i64".to_string()))?;
//         let from_address_bytes = &from.0.0;

//         // =========================================================
//         // atomic INSERT with concurrency-safe nonce selection and idempotency check

//         let candidate = sqlx::query!(
//             r#"
//             INSERT INTO nonce.nonce_assignments
//                 (execution_id, revision, chain_id, from_address, nonce, state)
//             SELECT
//                 $1,
//                 COALASCE(
//                     (SELECT MAX(revision)
//                     FROM)
//                 )
//             "#,
//         )
//     }

//     // =========================================================
//     // resolving nonce state

//     async fn handle_resolve(
//         &self,
//         execution_id: ExecutionId,
//         success: bool,
//     ) -> Result<(), ExecutionError> {
//         let new_state = if success {
//             NonceState::Finalized
//         } else {
//             NonceState::Released
//         };

//         let updated_state = sqlx::query_scalar::<_, NonceState>(
//             r#"
//             UPDATE nonce.nonce_assignments
//             SET state = $2
//             WHERE execution_id = $1
//             AND state = 'reserved'
//             RETURNING state
//             "#,
//         )
//         .bind(execution_id.0.as_bytes().as_slice())
//         .bind(new_state)
//         .fetch_optional(&self.db)
//         .await
//         .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;

//         match updated_state {
//             Some(_) => Ok(()), // transition succeeded
//             None => {
//                 // Either already terminal OR nonexistent
//                 let exists = sqlx::query_scalar!(
//                     r#"
//                     SELECT 1
//                     FROM nonce.nonce_assignments
//                     WHERE execution_id = $1
//                     "#,
//                     execution_id.0.as_bytes().as_slice(),
//                 )
//                 .fetch_optional(&self.db)
//                 .await
//                 .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;

//                 if exists.is_none() {
//                     Err(ExecutionError::DatabaseError(
//                         "unknown execution_id".to_string(),
//                     ))
//                 } else {
//                     Ok(()) // idempotent: already resolved
//                 }
//             }
//         }
//     }
// }

// // =========================================================
// // race conditoin checker.

// fn is_unique_violation_on(err: &sqlx::Error, constraint: &str) -> bool {
//     let sqlx::Error::Database(db_err) = err else {
//         return false;
//     };
//     let pg_err = db_err.downcast_ref::<PgDatabaseError>();

//     // 23505 = unique_violation
//     if pg_err.code() != "23505" {
//         return false;
//     }

//     // Ensure it's *your* index, not some other uniqueness rule
//     match pg_err.constraint() {
//         Some(name) => name == constraint,
//         None => false,
//     }
// }

// // =========================================================
