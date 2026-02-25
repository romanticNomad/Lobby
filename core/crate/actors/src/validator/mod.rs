pub mod engine;
pub mod handle;

use crate::validator::{engine::ValidatorEngine, handle::ValidatorHandle};
use kernel::types::RpcProviderRegistry;
use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::mpsc;

// ============================================================

#[derive(Debug, Clone)]
/// Configuration for the transaction validation process.
pub struct ValidatorConfig {
    /// How often to poll the RPC node for a transaction receipt.
    pub poll_interval: Duration,

    /// Maximum time to wait for a transaction to be included before giving up.
    /// After this timeout, the transaction is considered NotIncluded.
    pub timeout: Duration,

    /// Number of block confirmations required before a transaction is
    /// considered definitively included (protection against shallow reorgs).
    pub required_confirmations: u64, // default value '3' for rerorg safety.
}

impl Default for ValidatorConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(3),
            timeout: Duration::from_secs(300),
            required_confirmations: 3,
        }
    }
}

// ============================================================

/// Spawn a new `ValidatorEngine` actor and return a handle to it.
///
/// The actor runs in a background Tokio task and processes validation requests
/// until the last handle is dropped (which closes the mpsc channel).
pub fn spawn_validator_actor(
    db: PgPool,
    rpc_registry: RpcProviderRegistry,
    config: ValidatorConfig,
    buffer: usize,
) -> ValidatorHandle {
    let (tx, rx) = mpsc::channel(buffer);
    let engine = ValidatorEngine::new(db, config, rpc_registry, rx);

    tokio::spawn(async move {
        engine.run().await;
    });

    ValidatorHandle::new(tx)
}

// ============================================================
