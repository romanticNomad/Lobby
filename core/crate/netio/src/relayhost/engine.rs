use kernel::types::{ClientConfig, Eip1559Transaction, ExecutionId, RelayHostError};
use sqlx::PgPool;
use tokio::sync::mpsc;
use tracing::info;
use utils::eip1159_linter::transaction_lint;

use crate::relayhost::handle::{RelayHostCommand, RelayHostHandle};

// ============================================================
// RelatHostEngine and spawn implimentation

pub struct RelayHostEngine {
    db: PgPool,
    rx: mpsc::Receiver<RelayHostCommand>,
}

impl RelayHostEngine {
    pub fn spawn(db: PgPool) -> RelayHostHandle {
        let (tx, rx) = mpsc::channel(1024);
        let relayhost_actor = RelayHostEngine { db, rx };

        tokio::spawn(async move {
            relayhost_actor.run().await;
        });

        RelayHostHandle::new(tx)
    }
}

// ============================================================
// RelayHostEngine functioning

impl RelayHostEngine {
    pub async fn run(mut self) {
        info!("RelayHostEngine started");

        while let Some(msg) = self.rx.recv().await {
            match msg {
                RelayHostCommand::SubmitTransaction {
                    execution_id,
                    txn,
                    client_config,
                    reply_tx,
                } => {
                    let result = self
                        .handle_submit_transaction(execution_id, txn, client_config)
                        .await;
                    let _ = reply_tx.send(result);
                }
            }
        }
    }

    async fn handle_submit_transaction(
        &self,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
        client_config: ClientConfig,
    ) -> Result<(), RelayHostError> {
        // ============================================================
        // idempotency check

        let execution_id_exists = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM relay_host.transaction_intents
                WHERE execution_id = $1
            ) AS "exists!"
            "#,
            execution_id.0.as_bytes().as_slice()
        )
        .fetch_one(&self.db)
        .await
        .map_err(|e| RelayHostError::DatabaseError(e))?;

        if execution_id_exists {
            return Err(RelayHostError::DuplicateExecutionId(execution_id.0));
        };

        // ============================================================
        // transaction business logic linting (from address is already checked in middleware)

        transaction_lint(&txn)?;

        // ============================================================
        // persist transaction intent to database and
        // necessary type conversion for Postgress type matching and precision

        let access_list_json =
            serde_json::to_value(&txn.access_list).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;

        let value_bd: sqlx::types::BigDecimal = txn
            .value
            .to_string()
            .parse()
            .map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
        let gas_limit_bd: sqlx::types::BigDecimal = txn
            .gas_limit
            .to_string()
            .parse()
            .map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
        let max_fee_bd: sqlx::types::BigDecimal = txn
            .max_fee_per_gas
            .to_string()
            .parse()
            .map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
        let max_priority_fee_bd: sqlx::types::BigDecimal = txn
            .max_priority_fee_per_gas
            .to_string()
            .parse()
            .map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
        let chain_id_i64: i64 =
            txn.chain_id.0.try_into().map_err(|_| {
                RelayHostError::ValidationFailed("invalid chain_id format".to_string())
            })?;

        sqlx::query!(
            r#"
            INSERT INTO relay_host.transaction_intents (
                execution_id,
                client_id,
                chain_id,
                from_address,
                to_address,
                value,
                data,
                gas_limit,
                max_fee_per_gas,
                max_priority_fee_per_gas,
                access_list
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            "#,
            execution_id.0.as_bytes().as_slice(),
            client_config.client_id.as_bytes().as_slice(),
            chain_id_i64,
            client_config.from_address.as_slice(),
            txn.to.as_ref().map(|addr| addr.as_slice()),
            value_bd,
            txn.data.as_ref(),
            gas_limit_bd,
            max_fee_bd,
            max_priority_fee_bd,
            access_list_json,
        )
        .execute(&self.db)
        .await
        .map_err(|e| RelayHostError::DatabaseError(e))?;

        Ok(())
    }
}

// ============================================================
