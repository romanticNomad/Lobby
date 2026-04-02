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
    artifacts::config::CortexConfig,
    artifacts::error::CortexError,
    artifacts::pool::ShardPool,
    artifacts::state,
    pipeline::{PipelineContext, run_pipeline},
    state::StatusRegistry,
};
use actors::{broadcast, nonce, relayhost, sign, validator};
use primitives::{
    traits::{Broadcaster, IntentRelay, NonceManager, Signer, StateStore, Validator},
    types::{ClientConfig, Eip1559Transaction, ExecutionId, PipelineStatus, RpcProviderRegistry},
};
use sqlx::PgPool;
use std::{env, sync::Arc};
use tokio::sync::Semaphore;
use utils::rpc::load_rpc_endpoints_from_env;

// ============================================================
// Cortex (orchestrator) struct.

struct Cortex {
    // state handles
    cortex_config: CortexConfig,
    status_registry: StatusRegistry,

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
        .expect("pipeline: semaphore is closed");

        tracing::debug!(
            execution_id = ?execution_id,
            available_permits = orch.semaphore.available_permits(),
            "pipeline semaphore permit aquired"
        );

        orch.status_registry
            .set(execution_id, PipelineStatus::PermitAquired);

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
            status: orch.status_registry.clone(),
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
    pub fn status_registry(&self) -> StatusRegistry {
        self.inner.status_registry.clone()
    }
}

// ============================================================

/// RpcProviderStack builder for orchestration of `Validator`` and `Broadcaster``
///
/// Separate registries prevent connection pool exhaustion.
#[derive(Clone)]
pub struct RpcProviderStack {
    /// Used ONLY by broadcaster/signer (write operations)
    pub broadcast_registry: RpcProviderRegistry,
    /// Used ONLY by validator (read operations - polling receipts)
    pub validator_registry: RpcProviderRegistry,
}

impl RpcProviderStack {
    pub fn new() -> Self {
        // separate http clients for Broadcaster and Validator
        let broadcast_registry = load_rpc_endpoints_from_env();
        let validator_registry = load_rpc_endpoints_from_env();

        Self {
            broadcast_registry,
            validator_registry,
        }
    }
}

// ============================================================
// Cortex (orchestrator) boot function

/// Spawn all actor shards and assemble the `OrchestratorHandle`.
/// panics if number of shards in config = 0.
pub async fn spawn_cortex(
    db: PgPool,
    provider: RpcProviderStack,
    config: CortexConfig,
) -> CortextHandle {
    tracing::debug!(
        nonce_shards = config.nonce_shards,
        sign_shards = config.sign_shards,
        broadcast_shards = config.broadcast_shards,
        pipeline = config.pipeline_concurrency,
        "spawning cortex actor pools: "
    );

    // ============================================================
    // nonce pool - keyed by from_address.

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
                    Arc::clone(&provider.broadcast_registry),
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
            Arc::clone(&provider.validator_registry),
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
    // initialize StatusRegistry

    let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());
    let status_registry = StatusRegistry::new(&redis_url)
        .await
        .expect("StatusRegistry: failed to connect to Redis server");

    // ============================================================
    // returning final cortex handle.

    let inner = Arc::new(Cortex {
        cortex_config: config,
        status_registry,
        semaphore: pipeline_semaphore,
        relayhost: relayhost_handle,
        nonce: nonce_pool,
        sign: sign_pool,
        broadcast: broadcast_pool,
        validator: validator_handle,
    });

    tracing::info!("cortex online");
    CortextHandle { inner }
}

// ============================================================
