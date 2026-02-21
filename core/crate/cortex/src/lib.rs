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

use crate::{
    config::CortexConfig,
    error::CortexError,
    pipeline::{PipelineContext, run_pipeline},
    pool::ShardPool,
    state::StatusRegistry,
};
use actors::nonce;
use kernel::{
    traits::{Broadcaster, IntentRelay, NonceManager, Signer, Validator},
    types::{ClientConfig, Eip1559Transaction, ExecutionId},
};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::Semaphore;

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
    cortex_config: CortexConfig,
    status_registry: Arc<StatusRegistry>,
    semaphore: Arc<Semaphore>,

    // actor artifacts
    relayhost: Arc<dyn IntentRelay>,
    nonce: Arc<ShardPool<dyn NonceManager>>,
    sign: Arc<ShardPool<dyn Signer>>,
    broadcast: Arc<ShardPool<dyn Broadcaster>>,
    validate: Arc<dyn Validator>,
}

// ============================================================
// handle

///cheap to clone handle to the orchestrator.
#[derive(Clone)]
pub struct CortextHandle {
    inner: Arc<Cortex>,
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
    pub async fn submit(
        &self,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
        client_config: ClientConfig,
    ) -> Result<(), CortexError> {
        let orch = Arc::clone(&self.inner);

        // ============================================================
        // Semaphore: bound how many pipelines run concurrently

        let permit = tokio::time::timeout(
            orch.cortex_config.pipeline_semaphore_timeout,
            Arc::clone(&orch.semaphore).acquire_owned(),
        )
        .await
        .map_err(|_| CortexError::BackpressureTimeout {
            timeout_ms: orch.cortex_config.pipeline_semaphore_timeout.as_millis() as u64,
        })?
        .expect("semaphore should never be closed");

        tracing::debug!(
            execution_id = ?execution_id,
            available_permits = orch.semaphore.available_permits(),
            "pipeline semaphore permit aquired"
        );

        orch.status_registry
            .set(execution_id, state::PipelineStatus::PermitAquired);

        // ============================================================
        // building the pipeline context

        let ctx = PipelineContext {
            execution_id,
            client_config,
            txn,
            relayhost_handle: Arc::clone(&orch.relayhost),
            validator_handle: Arc::clone(&orch.validate),
            broadcast_pool: Arc::clone(&orch.broadcast),
            nonce_pool: Arc::clone(&orch.nonce),
            sign_pool: Arc::clone(&orch.sign),
            retry_config: orch.cortex_config.retry.clone(),
            status: Arc::clone(&orch.status_registry),
        };

        // ============================================================
        // Spawn the pipeline task
        // The semaphore permit is moved into the task and dropped when the
        // task completes, automatically freeing a slot.

        tokio::spawn(async move {
            let _permit = permit;
            run_pipeline(ctx).await;
        });

        Ok(())
    }
}

// ============================================================
// Cortex (orchestrator) boot function

/// Spawn all actor shards and assemble the `OrchestratorHandle`.
/// panics if number of shards in cofig = 0.

pub fn spawn_cortex(
    pg: PgPool,
    config: CortexConfig
) -> CortextHandle {
    tracing::info!(
        nonce_shards = config.nonce_shard,
        sign_shards = config.sign_shard,
        broadcast_shards = config.broadcast_shard,
        pipeline = config.pipeline_concurrency,
        "spawning cortex actor pools"
    );

    // ============================================================
    // nonce pool - keyed by from address.

    let nonce_pool = {
        let shards: Vec<Arc<dyn NonceManager>> = (0..config.nonce_shard)
            .map(|i| {
                let handle = nonce::spawn_nonce_actor(db, config.actor_buffer);
                tracing::debug!(shard = i, "nonce actor spawned");
                Arc::new(handle) as Arc<dyn NonceManager>
            })
            .collect();
        Arc::new(ShardPool::new(shards))
    };

    
}