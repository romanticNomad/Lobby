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
        Self {
            db,
            json_policy,
            rx,
        }
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
        let revision: i64;

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

                match state.as_str() {
                    "signed" => {
                        // changed: idempotent replay is rejected early
                        return Err(ExecutionError::Internal(
                            "transaction already signed".to_string(),
                        ));
                    }
                    "unsigned" | "failed" => {
                        // allowed states, continue
                    }
                    _ => {
                        return Err(ExecutionError::DatabaseError(
                            "invalid state".to_string(),
                        ));
                    }
                }
            }
        }

        // =========================================================
        // signing + guarded update

        match sign_eip1559_transaction(txn, pvt_key) {
            Ok(signed_tx) => {
                let result = sqlx::query!(
                    r#"
                    UPDATE sign.sign_requests
                    SET state = 'signed',
                        revision = revision + 1
                    WHERE execution_id = $1
                    AND revision = $2 
                    AND state IN ('unsigned', 'failed')
                    "#,
                    execution_id.0.as_bytes().as_slice(),
                    revision,
                )
                .execute(&self.db)
                .await
                .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;

                // changed: enforce exactly-once semantics
                if result.rows_affected() != 1 {
                    return Err(ExecutionError::Invariant(
                        "lost signing race".to_string(),
                    ));
                }

                Ok(signed_tx)
            }

            Err(e) => {
                let result = sqlx::query!(
                    r#"
                    UPDATE sign.sign_requests
                    SET state = 'failed'
                    WHERE execution_id = $1
                    AND revision = $2
                    AND state IN ('unsigned', 'failed')
                    "#,
                    execution_id.0.as_bytes().as_slice(),
                    revision,
                )
                .execute(&self.db)
                .await
                .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;

                // changed: failure must also win the race
                if result.rows_affected() != 1 {
                    return Err(ExecutionError::Invariant(
                        "lost failure race".to_string(),
                    ));
                }

                Err(e)
            }
        }
    }
}
