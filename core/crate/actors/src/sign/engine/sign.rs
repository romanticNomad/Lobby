use crate::sign::{SignCommand, policy::JsonPolicyEngine};
use alloy_primitives::Address;
use kernel::{
    traits::PolicyEngine,
    types::{ChainId, Eip1559Transaction, ExecutionError, ExecutionId, SignedTransaction},
};
use props::evm_signer::sign_eip1559_transaction;
use sqlx::PgPool;
use tokio::sync::mpsc;

// ============================================================

pub struct SignEngine {
    db: PgPool,
    json_policy: JsonPolicyEngine,
    rx: mpsc::Receiver<SignCommand>,
}

impl SignEngine {
    pub fn new(db: PgPool, rx: mpsc::Receiver<SignCommand>) -> Self {
        let path = "../test_keys.json";
        let json_policy = JsonPolicyEngine::load_file(path);
        Self { db, json_policy, rx }
    }
}

// ============================================================

impl SignEngine {
    // the type of 'rx' contains the information of cmd.

    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                SignCommand::Sign {
                    from,
                    chain_id,
                    execution_id,
                    txn,
                    reply_tx,
                } => {
                    let result = self.handle_sign(from, chain_id, execution_id, txn).await;
                    let _ = reply_tx.send(result);
                }
            }
        }
    }

    // state management and signing raw transaction.

    async fn handle_sign(
        &self,
        from: Address,
        chain_id: ChainId,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
    ) -> Result<SignedTransaction, ExecutionError> {
        // =========================================================
        // loading pvt_key from key policy and setting types for db

        let (key_id, pvt_key) = self.json_policy.resolve_key(&from)?;
        let chain_id_i64: i64 = chain_id
            .0
            .try_into()
            .map_err(|_| ExecutionError::Invariant("chain_id does not fit in i64".to_string()))?;
        let from_address_bytes = &from.0.0;
        let mut revision: i64 = 0;

        // =========================================================
        // row check: latest version only

        let row = sqlx::query!(
            r#"
            SELECT 
                revision,
                state::TEXT AS "state!"
            FROM sign.sign_requests
            WHERE execution_id = $1
            ORDER BY revision DESC
            LIMIT 1
            "#,
            execution_id.0.as_bytes().as_slice()
        )
        .fetch_optional(&self.db)
        .await
        .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;

        // =========================================================
        // Idempotency and control flow

        match row {
            None => {
                revision = 1;

                sqlx::query!(
                    r#"
                    INSERT INTO sign.sign_requests
                        (execution_id, revision, key_id, chain_id, from_address, state)
                    VALUES ($1, $2, $3, $4, $5, 'unsigned')
                    "#,
                    execution_id.0.as_bytes().as_slice(),
                    revision,
                    key_id,
                    chain_id_i64,
                    from_address_bytes,
                )
                .execute(&self.db)
                .await
                .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;
                
            }

            Some(row) => {
                revision = row.revision;
                let state = row.state;

                match state {
                    s if s == String::from("unsigned") => {
                        Err(ExecutionError::Invariant("corrupted transaction request".to_string()))?
                    }
                    s if s == String::from("signed") => {
                        Err(ExecutionError::Internal("transaction already signed".to_string()))?
                    }
                    s if s == String::from("failed") => {
                        
                    }
                    _ => {
                        Err(ExecutionError::DatabaseError("invalid State".to_string()))?
                    }
                }
            }
        }

        match sign_eip1559_transaction(txn, pvt_key) {
            Ok(signed_tx) => {
                sqlx::query!(
                    r#"
                    UPDATE sign.sign_requests
                    SET state = 'signed',
                        revision = revision + 1
                    WHERE execution_id = $1
                    AND revision = $2 
                    AND state = 'unsigned'
                    "#,
                    execution_id.0.as_bytes().as_slice(),
                    revision,
                )
                .execute(&self.db)
                .await
                .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;

                Ok(signed_tx)
            }

            Err(e) => {
                sqlx::query!(
                    r#"
                    UPDATE sign.sign_requests
                    SET state = 'failed'
                    WHERE execution_id = $1
                    AND revision = $2
                    AND state = 'unsigned'
                    "#,
                    execution_id.0.as_bytes().as_slice(),
                    revision,
                )
                .execute(&self.db)
                .await
                .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;

                Err(e)
            }
        }
    }
}
