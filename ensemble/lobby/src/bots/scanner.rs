use alloy::primitives::TxHash;
use cortex::artifacts::StatusRegistry;
use dashmap::DashMap;
use kernel::{
    traits::StateStore,
    types::{ChainId, ExecutionId, PipelineStatus, RpcProviderRegistry},
};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tokio::time::interval;
use uuid::Uuid;

// ============================================================

/// Represents a timed-out transaction that needs to be rescanned
#[derive(Debug, Clone, Copy)]
struct TimedOutTransaction {
    execution_id: ExecutionId,
    txn_hash: TxHash,
    chain_id: ChainId,
}

// ============================================================

/// `scanner_bot` actively keeps track of `state = 'timed_out'` transactions
/// and polls the RPC nodes to check if they eventually got included.
///
/// It runs continuously, scanning every 30 seconds for up to 100 timed-out
/// transactions, and updates both the database and StatusRegistry when a
/// transaction is found on-chain.
pub fn spawn_scanner_bot(db: PgPool, state: StatusRegistry, rpc: RpcProviderRegistry) {
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(30));

        loop {
            tick.tick().await;

            if let Err(e) = scanner_handler(&db, &state, &rpc).await {
                tracing::error!("scanner bot failed: {}", e);
            }
        }
    });
}

// ============================================================

/// Main scanning logic: fetch 'timed-out' transactions, check RPC, update DB + registry
async fn scanner_handler(
    db: &PgPool,
    state: &StatusRegistry,
    rpc: &RpcProviderRegistry,
) -> Result<(), Box<dyn std::error::Error>> {
    let timed_out_txn: Vec<TimedOutTransaction> = fetch_timed_out_txn(&db).await?;

    if timed_out_txn.is_empty() {
        tracing::debug!("no 'timed_out' transaction found");
        return Ok(());
    };

    tracing::info!(
        "scanner_bot: found 'timed_out' transactions: {}",
        timed_out_txn.len()
    );

    // Group transactions by chain_id for concurrent processing
    let by_chain_id: DashMap<ChainId, Vec<TimedOutTransaction>> = DashMap::new();

    for tx in timed_out_txn {
        by_chain_id
            .entry(tx.chain_id)
            .or_insert_with(|| Vec::new())
            .push(tx);
    }

    // Process each chain concurrently
    let mut handles = Vec::new();

    for entry in by_chain_id.into_iter() {
        let (chain_id, txns) = entry;
        let db = db.clone();
        let state = state.clone();
        let rpc = rpc.clone();

        let handle = tokio::spawn(async move {
            process_chain(chain_id, txns, &db, &state, &rpc).await;
        });

        handles.push(handle);
    }

    // Wait for all chains to finish processing

    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}

// ============================================================

async fn fetch_timed_out_txn(db: &PgPool) -> Result<Vec<TimedOutTransaction>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT DISTINCT ON(execution_id)
            execution_id,
            chain_id,
            tx_hash
        FROM validator.validation_requests
        WHERE state = 'timed_out'
        ORDER BY execution_id, revision DESC
        LIMIT 100
        "#
    )
    .fetch_optional(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| TimedOutTransaction {
            execution_id: ExecutionId(Uuid::from_slice(&row.execution_id).unwrap()),
            txn_hash: TxHash::from_slice(&row.tx_hash),
            chain_id: ChainId::try_from(row.chain_id).unwrap(),
        })
        .collect())
}

// ============================================================

/// Process all timed_out transactions for a specific chain
async fn process_chain(
    chain_id: ChainId,
    txns: Vec<TimedOutTransaction>,
    db: &PgPool,
    state: &StatusRegistry,
    rpc: &RpcProviderRegistry,
) {
    tracing::debug!(
        "tracing_bot: processing {} txn for chain_id {}",
        txns.len(),
        chain_id
    );

    let provider = match rpc.get(&chain_id) {
        Some(p) => p.clone(),
        None => {
            tracing::error!("no provider for chain_id: {}", chain_id);
            return;
        }
    };

    for txn in txns {
        if let Err(e) = process_txn(txn, &provider, db, state).await {
            tracing::error!(
                "tracing_bot: failed to process txn: {:?}, on chain: {}\n error: {}",
                txn.txn_hash,
                chain_id,
                e
            );
        }
    }
}

// ============================================================

/// Process a single transaction: fetch receipt, update DB, update registry
async fn process_txn(
    txn: TimedOutTransaction,
    provider: &Arc<dyn alloy::providers::Provider + Send + Sync>,
    db: &PgPool,
    state: &StatusRegistry,
) -> Result<(), Box<dyn std::error::Error>> {
    // query
    let receipt_option = match provider.get_transaction_receipt(txn.txn_hash).await {
        Ok(receipt) => receipt,
        Err(e) => {
            tracing::error!("tracing_bot: rpc error: {e}");
            // don't fail the whole branch, log error and continue
            return Ok(());
        }
    };

    // Determine new state based on receipt
    let new_state = match receipt_option {
        Some(receipt) => {
            if receipt.status() {
                tracing::info!(
                    "scanner_bot: txn = {}, INCLUDED on block = {}",
                    txn.txn_hash,
                    receipt.block_number.unwrap_or(0)
                );
                "included"
            } else {
                tracing::warn!("scanner_bot: txn = {}, REVERTED", txn.txn_hash);
                "not_included"
            }
        }

        None => {
            tracing::debug!("scanner_bot: txn = {}, NOT FOUND", txn.txn_hash);
            return Ok(());
        }
    };

    update_db(db, &txn, new_state).await?;
    update_state(state, &txn, new_state);

    Ok(())
}

// ============================================================
// state update helper functions

async fn update_db(
    db: &PgPool,
    txn: &TimedOutTransaction,
    new_state: &'static str,
) -> Result<(), sqlx::Error> {
    let execution_id_bytes = txn.execution_id.0.as_bytes().as_slice();

    sqlx::query!(
        r#"
        UPDATE validator.validation_requests
        SET state = $1
        WHERE execution_id = $2
        AND revision = (
            SELECT MAX(revision)
            FROM validator.validation_requests
            WHERE execution_id = $2
        )
        "#,
        new_state,
        execution_id_bytes
    )
    .execute(db)
    .await?;

    tracing::info!(
        "scanner_bot: updated execution_id: {}, to state: {}",
        txn.execution_id,
        new_state
    );

    Ok(())
}

fn update_state(state: &StatusRegistry, txn: &TimedOutTransaction, new_state: &'static str) {
    let new_state = match new_state {
        "included" => PipelineStatus::ConfirmedOnChain {
            tx_hash: format!("{:?}", txn.txn_hash),
        },
        "not_included" => PipelineStatus::Failed {
            stage: "validation".to_string(),
            reason: "txn reverted on chain".to_string(),
        },
        _ => return,
    };

    if state.status_book.contains_key(&txn.execution_id) {
        state.set(txn.execution_id, new_state);
    } else {
        tracing::warn!(
            "execution_id: {}, not found StatusRegistry",
            txn.execution_id
        );
    }
}

// ============================================================
