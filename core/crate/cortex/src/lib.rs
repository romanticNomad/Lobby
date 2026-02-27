//! # Orchestrator
//!
//! Wires together all Lobby actors into a single pipeline:
//!
//! ```text
//! send_transaction
//!   └─ cortex_handler.submit(..)
//!        └─ spawn pipeline task (semaphore-gated)  [hard fail on backpressrue timeup or internal error]
//!              ├─ RelayHost.register_transaction() [retried, idempotent]
//!              ├─ Nonce.reserve()                  [retried]
//!              ├─ Sign.sign()                      [retried; releases nonce on hard-fail]
//!              ├─ Broadcast.broadcast()            [retried; releases nonce on hard-fail]
//!              └─ Validator.validate()             [retried; releases nonce on hard-fail]
//! ```
//!
//! The `OrchestratorHandle` is a cheap `Arc`-backed clone that can be placed
//! in Axum's `AppState`.

pub mod artifacts;
pub mod pipeline;

use crate::{
    artifacts::state, artifacts::config::CortexConfig, artifacts::error::CortexError, pipeline::{PipelineContext, run_pipeline}, artifacts::pool::ShardPool, state::StatusRegistry
};
use actors::{broadcast, nonce, relayhost, sign, validator};
use kernel::{
    traits::{Broadcaster, IntentRelay, NonceManager, Signer, Validator},
    types::{ClientConfig, Eip1559Transaction, ExecutionId, RpcProviderRegistry},
};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::Semaphore;

// ============================================================
// Cortex (orchestrator) struct.

struct Cortex {
    // state handles
    cortex_config: CortexConfig,
    status_registry: Arc<StatusRegistry>,

    // actor handles
    relayhost: Arc<dyn IntentRelay>,
    nonce: Arc<ShardPool<dyn NonceManager>>,
    sign: Arc<ShardPool<dyn Signer>>,
    broadcast: Arc<ShardPool<dyn Broadcaster>>,
    validator: Arc<dyn Validator>,
    semaphore: Arc<Semaphore>,
}

// ============================================================
// handle

/// cheap to clone handle to the orchestrator.
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
            validator_handle: Arc::clone(&orch.validator),
            nonce_pool: Arc::clone(&orch.nonce),
            sign_pool: Arc::clone(&orch.sign),
            broadcast_pool: Arc::clone(&orch.broadcast),
            retry_config: orch.cortex_config.retry.clone(),
            status: Arc::clone(&orch.status_registry),
        };

        // ===========================================================

        // Spawn the pipeline task
        // The semaphore permit is moved into the task and dropped when the
        // task completes, automatically freeing a slot.
        tokio::spawn(async move {
            let _permit = permit;
            run_pipeline(ctx).await;
        });

        Ok(())
    }

    // ===========================================================
    /// simple helper for obtaining StatusRegistry clone
    pub fn status_registry(&self) -> Arc<StatusRegistry> {
        Arc::clone(&self.inner.status_registry)
    }
}

// ============================================================

// Cortex (orchestrator) boot function

/// Spawn all actor shards and assemble the `OrchestratorHandle`.
/// panics if number of shards in config = 0.
pub fn spawn_cortex(
    db: PgPool,
    provider: RpcProviderRegistry,
    config: CortexConfig,
) -> CortextHandle {
    tracing::info!(
        nonce_shards = config.nonce_shards,
        sign_shards = config.sign_shards,
        broadcast_shards = config.broadcast_shards,
        pipeline = config.pipeline_concurrency,
        "spawning cortex actor pools"
    );

    // ============================================================
    // nonce pool - keyed by from address.

    let nonce_pool = {
        let shards: Vec<Arc<dyn NonceManager>> = (0..config.nonce_shards)
            .map(|i| {
                let handle = nonce::spawn_nonce_actor(db.clone(), config.actor_buffer);
                tracing::debug!(shard = i, "nonce actor spawned");
                Arc::new(handle) as Arc<dyn NonceManager>
            })
            .collect();
        Arc::new(ShardPool::new(shards))
    };

    // ============================================================
    // sign pool - keyed by execution_id.

    let sign_pool = {
        let shards: Vec<Arc<dyn Signer>> = (0..config.sign_shards)
            .map(|i| {
                let handle = sign::spawn_sign_actor(db.clone(), config.actor_buffer);
                tracing::debug!(shard = i, "sign actor spawned");
                Arc::new(handle) as Arc<dyn Signer>
            })
            .collect();
        Arc::new(ShardPool::new(shards))
    };

    // ============================================================
    // broadcast pool - keyed by chain_id.

    let broadcast_pool = {
        let shards: Vec<Arc<dyn Broadcaster>> = (0..config.broadcast_shards)
            .map(|i| {
                let handle = broadcast::spawn_broadcast_actor(
                    db.clone(),
                    Arc::clone(&provider),
                    config.actor_buffer,
                );
                tracing::debug!(shard = i, "broadcast actor spawned");
                Arc::new(handle) as Arc<dyn Broadcaster>
            })
            .collect();
        Arc::new(ShardPool::new(shards))
    };

    // ============================================================
    // relayhost handle

    let relayhost_handle = {
        let handle = relayhost::spawn_relayhost_actor(db.clone(), config.actor_buffer);
        tracing::debug!("relay_host actor spawned");
        Arc::new(handle) as Arc<dyn IntentRelay>
    };

    // ============================================================
    // validator handle

    let validator_handle = {
        let handle = validator::spawn_validator_actor(
            db.clone(),
            provider,
            validator::ValidatorConfig::default(),
            config.actor_buffer,
        );
        tracing::debug!("validate actor spawned");
        Arc::new(handle) as Arc<dyn Validator>
    };

    // ============================================================
    // pipeline semaphore

    let pipeline_semaphore = Arc::new(Semaphore::new(config.pipeline_concurrency));

    // ============================================================
    // returning final cortex handle.

    let inner = Arc::new(Cortex {
        cortex_config: config,
        status_registry: Arc::new(StatusRegistry::new()),
        semaphore: pipeline_semaphore,
        relayhost: relayhost_handle,
        nonce: nonce_pool,
        sign: sign_pool,
        broadcast: broadcast_pool,
        validator: validator_handle,
    });

    tracing::info!("orchestrator ready");
    CortextHandle { inner }
}

// ============================================================
