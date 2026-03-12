use std::time::Duration;

use alloy::primitives::TxHash;
use cortex::artifacts::StatusRegistry;
use dashmap::DashMap;
use kernel::types::{ChainId, ExecutionId, RpcProviderRegistry};
use sqlx::PgPool;
use tokio::time::interval;

/// Represents a timed-out transaction that needs to be rescanned
struct TimedOutTransaction {
    execution_id: ExecutionId,
    txn_hash: TxHash,
    chain_id: ChainId
}

/// `scanner_bot` actively keeps track of `state = 'timed_out'` transactions
/// and polls the RPC nodes to check if they eventually got included.
///
/// It runs continuously, scanning every 30 seconds for up to 100 timed-out
/// transactions, and updates both the database and StatusRegistry when a
/// transaction is found on-chain.
pub fn scanner_bot(db: PgPool, state: StatusRegistry, rpc: RpcProviderRegistry) {
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(30));

        loop{
            tick.tick().await;

            if let Err(e) = scanner_handler(&db, &state, &rpc).await {
                tracing::error!("scanner bot failed: {}", e);
            }
        }
    });
}

/// Main scanning logic: fetch 'timed-out' transactions, check RPC, update DB + registry
async fn scanner_handler(
    db: &PgPool,
    state: &StatusRegistry,
    rpc: &RpcProviderRegistry
) -> Result<(), Box<dyn std::error::Error>> {
    let timed_out_txn:Vec<TimedOutTransaction> = fetch_timed_out_txn(&db).await?;

    if timed_out_txn.is_empty() {
        tracing::debug!("no 'timed_out' transaction found");
        return Ok(());
    };

    tracing::info!("scanner_bot: found 'timed_out' transactions: {}", timed_out_txn.len());

    // Group transactions by chain_id for concurrent processing
    let by_chainid: DashMap<ChainId, Vec<TimedOutTransaction>> = DashMap::new();

    for tx in timed_out_txn {
        by_chainid.entry(tx.chain_id).or_insert_with(Vec::new()).push(tx);
    }

    // Process each chain concurrently
    let mut handles = Vec::new();
    for entry in by_chainid.into_iter() {
        let (chainid, txns) = entry;
        let db = db.clone();
        let state = state.clone();
        let rpc = rpc.clone()

        let handle = tokio::spawn(async move {
            process_chain(chainid,  txns, &db, &state, &rpc).await;
        });

        handles.push(handle);
    }

    // Wait for all chains to finish processing

    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}


