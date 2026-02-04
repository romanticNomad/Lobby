use crate::sign::SignCommand;
use alloy::primitives::Address;
use kernel::{
    traits::PolicyEngine,
    types::{ChainId, Eip1559Transaction, ExecutionError, ExecutionId, SignedTransaction},
};
use props::{eip_1559_signer::sign_eip1559_transaction, policy::JsonPolicyEngine};
use sqlx::PgPool;
use std::path::PathBuf;
use tokio::sync::mpsc;

// ============================================================

pub struct SignEngine {
    db: PgPool,
    json_policy: JsonPolicyEngine,
    rx: mpsc::Receiver<SignCommand>,
}

impl SignEngine {
    pub fn new(db: PgPool, rx: mpsc::Receiver<SignCommand>) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test_keys.json");
        let json_policy = JsonPolicyEngine::load_file(path.to_str().unwrap());
        Self {
            db,
            json_policy,
            rx,
        }
    }
}

// ============================================================

impl SignEngine {
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

        let pvt_key = self.json_policy.resolve_key(&from)?;
        let chain_id_i64: i64 = chain_id
            .0
            .try_into()
            .map_err(|_| ExecutionError::Invariant("chain_id does not fit in i64".to_string()))?;
        let from_address_bytes = &from.0.0;

        // =========================================================
        // atomic insertion and lookup to avoid race condition (TOCTOU) and idempotency

        let revision = sqlx::query_scalar!(
            r#"
            INSERT INTO sign.sign_requests
                (execution_id, revision, chain_id, from_address, state)
            SELECT
                $1,
                COALESCE(
                    (SELECT MAX(revision)
                    FROM sign.sign_requests
                    WHERE execution_id = $1),
                    0
                ) + 1,
                $2,
                $3,
                'reserved'
            WHERE NOT EXISTS (
                SELECT 1
                FROM sign.sign_requests
                WHERE execution_id = $1
                    AND (
                        state = 'signed'
                        OR (
                            state = 'reserved'
                            AND updated_at > now() - interval '5 minutes'    
                        )
                    )
            )
            RETURNING revision
            "#,
            execution_id.0.as_bytes().as_slice(),
            chain_id_i64,
            from_address_bytes,
        )
        .fetch_optional(&self.db)
        .await
        .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;

        let revision = revision.ok_or_else(|| {
            ExecutionError::Invariant("sign database invariant faliure".to_string())
        })?;

        // =========================================================
        // pattern matching the signing result

        match sign_eip1559_transaction(txn, pvt_key) {
            Ok(signed_tx) => {
                let result = sqlx::query!(
                    r#"
                    UPDATE sign.sign_requests
                    SET state = 'signed'
                    WHERE execution_id = $1
                    AND revision = $2
                    AND state = 'reserved'
                    AND updated_at > now() - interval '5 minutes'
                    "#,
                    execution_id.0.as_bytes().as_slice(),
                    revision
                )
                .execute(&self.db)
                .await
                .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;

                if result.rows_affected() != 1 {
                    return Err(ExecutionError::Invariant(
                        "invalid sign_requests state transition".to_string(),
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
                    AND state = 'reserved'
                    AND updated_at > now() - interval '5 minutes'
                    "#,
                    execution_id.0.as_bytes().as_slice(),
                    revision
                )
                .execute(&self.db)
                .await
                .map_err(|err| ExecutionError::DatabaseError(err.to_string()))?;

                if result.rows_affected() != 1 {
                    return Err(ExecutionError::Invariant(
                        "invalid sign_requests state transition".to_string(),
                    ));
                }

                Err(e)
            }
        }
    }
}
