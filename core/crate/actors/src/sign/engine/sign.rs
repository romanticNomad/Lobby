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
    rx: mpsc::Receiver<SignCommand>,
}

impl SignEngine {
    pub fn new(db: PgPool, rx: mpsc::Receiver<SignCommand>) -> Self {
        Self { db, rx }
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
        let path = "../test_keys.json";
        let json_policy = JsonPolicyEngine::load_file(path);

        let (key_id, pvt_key) = json_policy.resolve_key(&from)?;

        // block to log the state row.

        match sign_eip1559_transaction(txn, pvt_key) {
            Ok(signed_tx) => {
                // update sign state to signed

                Ok(signed_tx)
            }

            Err(e) => {
                // update sign state to failed

                Err(e)
            }
        }
    }
}
