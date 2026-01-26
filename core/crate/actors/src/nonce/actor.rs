// use alloy_primitives::Address;
// use kernel::types::{ChainId, ExecutionError, ExecutionId, TxNonce};
// use sqlx::PgPool;
// use tokio::sync::mpsc;

// use crate::nonce::NonceCommand;

// // =========================================================
// // NonceActor struct declaration

// pub struct NonceActor {
//     db: PgPool,
//     rx: mpsc::Receiver<NonceCommand>,
// }

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
//     async fn handle_reserve(
//         &self,
//         chain_id: ChainId,
//         from: Address,
//         execution_id: ExecutionId,
//     ) -> Result<TxNonce, ExecutionError> {

//         // 1️⃣ Idempotency check: ExecutionId already reserved?
//         if let Some(existing) = sqlx::query_scalar!(
//             r#"
//             SELECT nonce
//             FROM nonce.nonce_assignments
//             WHERE execution_id = $1
//             "#,
//             execution_id.as_bytes(),
//         )
//         .fetch_optional(&self.db)
//         .await?
//         {
//             return Ok(existing as TxNonce);
//         }

//         // 2️⃣ Derive initial nonce candidate from DB (authoritative)
//         let mut candidate = {
//             let max = sqlx::query_scalar!(
//                 r#"
//                 SELECT COALESCE(MAX(nonce), -1)
//                 FROM nonce.nonce_assignments
//                 WHERE chain_id = $1 AND from_address = $2
//                 "#,
//                 chain_id as i64,
//                 from.as_bytes(),
//             )
//             .fetch_one(&self.db)
//             .await?;

//             (max + 1) as TxNonce
//         };

//         // 3️⃣ Retry loop guarded by DB constraints
//         loop {
//             let res = sqlx::query!(
//                 r#"
//                 INSERT INTO nonce.nonce_assignments
//                     (execution_id, chain_id, from_address, nonce, state)
//                 VALUES ($1, $2, $3, $4, 'reserved')
//                 "#,
//                 execution_id.as_bytes(),
//                 chain_id as i64,
//                 from.as_bytes(),
//                 candidate as i64,
//             )
//             .execute(&self.db)
//             .await;

//             match res {
//                 // ✅ Successful reservation
//                 Ok(_) => {
//                     return Ok(candidate);
//                 }

//                 // 🔁 Expected contention: retry or idempotent return
//                 Err(e) if is_unique_violation(&e) => {
//                     // Was this execution already inserted concurrently?
//                     if let Some(existing) = sqlx::query_scalar!(
//                         r#"
//                         SELECT nonce
//                         FROM nonce.nonce_assignments
//                         WHERE execution_id = $1
//                         "#,
//                         execution_id.as_bytes(),
//                     )
//                     .fetch_optional(&self.db)
//                     .await?
//                     {
//                         return Ok(existing as TxNonce);
//                     }

//                     // Otherwise, active nonce collision → try next nonce
//                     candidate += 1;
//                     continue;
//                 }

//                 // ❌ Real DB error
//                 Err(e) => {
//                     return Err(ExecutionError::Db(e));
//                 }
//             }
//         }
//     }
// }
