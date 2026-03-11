use cortex::artifacts::StatusRegistry;
use kernel::types::RpcProviderRegistry;
use sqlx::PgPool;

/// `scanner_bot` actively keeps track of
/// ValidatorOutcome::TimeOut transaction's status on
/// the mempool.
///
/// well after the validator has timed out
/// and retruns any update on the transaction to the
/// `StatusRegistry`.
pub fn scanner_bot(db: PgPool, state: StatusRegistry, rpc: RpcProviderRegistry) {}
