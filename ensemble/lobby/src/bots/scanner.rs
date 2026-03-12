use std::time::Duration;

use cortex::artifacts::StatusRegistry;
use kernel::types::RpcProviderRegistry;
use sqlx::PgPool;
use tokio::time::interval;

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

async fn scanner_handler(
    db: &PgPool,
    state: &StatusRegistry,
    rpc: &RpcProviderRegistry
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
