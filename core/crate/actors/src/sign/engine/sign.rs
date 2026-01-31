use crate::sign::{SignCommand, policy::JsonPolicyEngine};
use alloy_primitives::Address;
use kernel::{
    traits::PolicyEngine,
    types::{ChainId, Eip1559Transaction, ExecutionError, ExecutionId, SignedTransaction},
};
use props::{evm_signer::sign_eip1559_transaction, rlp::encode_eip1559_unsigned};
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
                    id,
                    txn,
                    reply_tx,
                } => {
                    let result = self.handle_sign(from, chain_id, id, txn).await;
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
        id: ExecutionId,
        txn: Eip1559Transaction,
    ) -> Result<SignedTransaction, ExecutionError> {
        // loading pvt_key from key policy
        let (key_id, pvt_key) = self.json_policy.resolve_key(&from)?;

        // idempotency check
        if let Some(_existing_raw_tx) = sqlx::query_scalar!(
            r#"
            SELECT raw_tx_hash
            FROM sign.sign_requests
            WHERE execution_id = $1
            "#,
            id.0.as_bytes().as_slice(),
        )
        .fetch_optional(&self.db)
        .await
        .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?
        {
            return Err(ExecutionError::Rejected("Signing already carried out once".to_string()))
        }

        let rlp_raw_tx = encode_eip1559_unsigned(&txn)?;
        let chain_id_i64: i64 = chain_id
            .0
            .try_into()
            .map_err(|_| ExecutionError::Invariant("chain_id does not fit in i64".to_string()))?;
        let from_address_bytes = &from.0.0;

        sqlx::query!(
            r#"
            INSERT INTO sign.sign_requests (
                execution_id,
                key_id,
                from_address,
                chain_id,
                raw_tx_hash,
                state
            )
            VALUES ($1, $2, $3, $4, $5, 'unsigned')
            "#,
            id.0.as_bytes().as_slice(),
            key_id,
            from_address_bytes,
            chain_id_i64,
            rlp_raw_tx.as_slice(),
        )
        .execute(&self.db)
        .await
        .map_err(|e| {
            // PRIMARY KEY violation = execution_id reused → idempotency breach
            ExecutionError::DatabaseError(e.to_string())
        })?;

        match sign_eip1559_transaction(txn, pvt_key) {
            Ok(signed_tx) => {
                sqlx::query!(
                    r#"
                    UPDATE sign.sign_requests
                    SET state = 'signed'
                    WHERE execution_id = $1 AND state = 'unsigned'
                    "#,
                    id.0.as_bytes().as_slice(),
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
                    "#,
                    id.0.as_bytes().as_slice(),
                )
                .execute(&self.db)
                .await
                .map_err(|e| ExecutionError::DatabaseError(e.to_string()))?;

                Err(e)
            }
        }
    }
}
