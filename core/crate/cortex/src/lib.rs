//! # Orchestrator
//!
//! Wires together all Lobby actors into a single pipeline:
//!
//! ```text
//! submit()
//!   └─ RelayHost.send_transaction()   [retried, idempotent]
//!        └─ spawn pipeline task (semaphore-gated)
//!              ├─ Nonce.reserve()     [retried]
//!              ├─ Sign.sign()         [retried; releases nonce on hard-fail]
//!              ├─ Broadcast.broadcast()[retried; releases nonce on hard-fail]
//!              └─ Validator.validate()[retried; releases nonce on hard-fail]
//! ```
//!
//! The `OrchestratorHandle` is a cheap `Arc`-backed clone that can be placed
//! in Axum's `AppState`.

use std::sync::Arc;

use kernel::{traits::{Broadcaster, IntentRelay, NonceManager, Signer, Validator}, types::{ClientConfig, Eip1559Transaction, ExecutionId}};
use tokio::sync::Semaphore;

use crate::{config::CortexConfig, error::CortexError, pool::ShardPool, state::StatusRegistry};

pub mod config;
pub mod error;
pub mod pipeline;
pub mod pool;
pub mod retry;
pub mod state;

// ============================================================
// Cortex (orchestrator) struct.

struct Cortex {
    // state metadata
    cortex_congif: CortexConfig,
    status_registry: Arc<StatusRegistry>,
    semaphore: Arc<Semaphore>,

    // actor artifacts
    relayhost: Arc<dyn IntentRelay>,
    nonce: Arc<ShardPool<dyn NonceManager>>,
    sign: Arc<ShardPool<dyn Signer>>,
    broadcast: Arc<ShardPool<dyn Broadcaster>>,
    validate: Arc<dyn Validator>
}

// ============================================================
// handle

///cheap to clone handle to the orchestrator.
#[derive(Clone)]
pub struct CortextHandle {
    inner: Arc<Cortex>
}


impl CortextHandle {
    /// Accept a validated, normalized transaction and start the background
    /// pipeline.
    ///
    /// Returns immediately after:
    /// 1. Registering the execution as `Accepted` in the status registry.
    /// 2. Acquiring a pipeline semaphore permit (bounded concurrency).
    /// 3. Spawning the pipeline task.
    ///
    /// The caller should respond to the client with `execution_id` and
    /// `"accepted"` — the real outcome is available via the status endpoint.
    ///
    /// # Errors
    /// Returns `OrchestratorError::BackpressureTimeout` if the semaphore is
    /// exhausted and the timeout expires before a permit becomes available.
    pub async fn submit (
        &self,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
        client_config: ClientConfig,
    ) -> Result<(), CortexError> {
        Ok(())
    }
}