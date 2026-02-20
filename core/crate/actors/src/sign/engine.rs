use crate::sign::SignCommand;
use alloy::primitives::Address;
use kernel::{
    traits::PolicyEngine,
    types::{ChainId, Eip1559Transaction, ExecutionId, LocalError, SignedTransaction},
};
use sqlx::PgPool;
use std::path::PathBuf;
use tokio::sync::mpsc;
use utils::{eip1559_signer::sign_eip1559_transaction, sign_policy::JsonPolicyEngine};

// ============================================================
// SignEngine struct declaration with policy details

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
// SignEngine functioning

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

    // =========================================================
    // state logic for concurrecny safe signing of raw transaction.

    async fn handle_sign(
        &self,
        from: Address,
        chain_id: ChainId,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
    ) -> Result<SignedTransaction, LocalError> {
        // =========================================================
        // loading pvt_key from key policy and setting types for db

        let pvt_key = self.json_policy.resolve_key(&from)?;
        let chain_id_i64: i64 = chain_id
            .0
            .try_into()
            .map_err(|_| LocalError::Invariant("chain_id does not fit in i64".to_string()))?;
        let from_bytes = &from.0.0;

        // =========================================================
        // idempotency safe atomic INSERT and lease locking

        let revision = sqlx::query_scalar!(
            r#"
            INSERT INTO sign.sign_requests
                (execution_id, revision, chain_id, from_address, state)
            SELECT
                $1,
                COALESCE(
                    (SELECT MAX(revision)
                    from sign.sign_requests
                    WHERE execution_id = $1),
                    0
                ) + 1,
                $2,
                $3,
                'reserved'
            WHERE NOT EXISTS (
                SELECT 1
                from sign.sign_requests
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
            from_bytes,
        )
        .fetch_optional(&self.db)
        .await
        .map_err(|e| LocalError::DatabaseError(e.to_string()))?;

        // =========================================================
        // pattern matching the state recieved (need to be done yet)

        let revision = revision
            .ok_or_else(|| LocalError::Invariant("sign database invariant faliure".to_string()))?;

        // =========================================================
        // signing logic and pattern matching

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
                .map_err(|e| LocalError::DatabaseError(e.to_string()))?;

                if result.rows_affected() != 1 {
                    return Err(LocalError::Invariant(
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
                .map_err(|err| LocalError::DatabaseError(err.to_string()))?;

                if result.rows_affected() != 1 {
                    return Err(LocalError::Invariant(
                        "invalid sign_requests state transition".to_string(),
                    ));
                }

                Err(e)
            }
        }
    }
}

// ============================================================
