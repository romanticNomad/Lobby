
//! Actor that validates whether broadcast transactions have been included on-chain.
//!
//! ## Responsibilities
//! - Poll RPC nodes for transaction receipts
//! - Track confirmation depth (protection against shallow reorgs)
//! - Timeout and return `NotIncluded` if the transaction is never mined
//! - Detect reverted transactions (status=0)
//!
//! ## State tracking
//! - PostgreSQL schema: `validator.validation_requests`
//! - Revision-based audit trail: `(execution_id, revision)` composite PK
//! - Lease-based idempotency: 5-minute window for duplicate requests
//!
//! ## Usage
//! ```rust
//! use validator::{spawn_validator_actor, ValidationConfig, RpcProviderRegistry};
//! use sqlx::PgPool;
//! use std::sync::Arc;
//! use dashmap::DashMap;
//!
//! let db_pool: PgPool = /* ... */;
//! let rpc_registry: RpcProviderRegistry = /* shared with broadcast actor */;
//! let config = ValidationConfig::default();
//!
//! let handle = spawn_validator_actor(
//!     db_pool,
//!     rpc_registry,
//!     config,
//!     64, // mpsc buffer size
//! );
//!
//! // Use the handle (it implements the Validator trait)
//! let outcome = handle.validate(chain_id, execution_id, tx_hash).await?;
//! ```

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
