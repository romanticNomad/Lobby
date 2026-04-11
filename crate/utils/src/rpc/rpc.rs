//! High-throughput RPC client with async-optimized request orchestration

use crate::rpc::{
    metadata::{EndpointHealth, EndpointMetrics},
    pool::{EndpointRegistry, LoadBalancingStrategy},
};
use alloy::{providers::Provider, transports::TransportError};
use dashmap::DashMap;
use primitives::types::ChainId;
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinSet,
    time::Instant,
};

// ============================================================================
// Constants

/// Default RPC timeout
const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// Failure window for circuit breaker (seconds)
const FAILURE_WINDOW_SECS: u64 = 60;

/// Warning threshold for failures per window
const FAILURE_WARNING_THRESHOLD: usize = 10;

/// Cache TTL for endpoint stats (milliseconds)
const STATS_CACHE_TTL_MS: u64 = 1000;

// ============================================================================
// Important structs.

/// High-performance RPC client for 1000+ TPS
///
/// Architecture:
/// - Semaphore for global rate limiting
/// - DashMap for lock-free registry access
/// - Lock-free metric recording via Arc<EndpointMetrics>
pub struct RpcClient {
    /// EndpointRegistry: lockfree read by using dashmap
    endpoint_registry: EndpointRegistry,

    /// Global concurrency limiter
    semaphore: Arc<Semaphore>,

    /// failiure tracker for every endpoint
    faliure_tacker: Arc<DashMap<String, FailureWindow>>,

    /// chached states for monitoring (reduce lock contention)
    state_cache: Arc<DashMap<ChainId, (Vec<EndpointStats>, Instant)>>,
}

/// Context for RPC execution (zero-copy design)
pub struct RpcContext {
    /// The RPC provider
    pub provider: Arc<dyn Provider + Send + Sync>,

    /// Shared metrics reference (for lock-free recording)
    pub metrics: Arc<EndpointMetrics>,

    /// Chain ID
    pub chain_id: ChainId,

    /// Permit for concurrency control (auto-released on drop)
    _permit: OwnedSemaphorePermit,
}

/// Failure tracking window
struct FailureWindow {
    failures: Vec<Instant>,
    last_reset: Instant,
}

/// Metric state-monitor with higher-level metric stats: derived from `EndpointMetric`
#[derive(Debug, Clone)]
pub struct EndpointStats {
    pub id: String,
    pub url: String,
    pub health: EndpointHealth,
    pub avg_response_time_ms: f64,
    pub error_rate: f64,
    pub score: f64,
    pub block_height: Option<u64>,
}

/// Builder for RpcClient with progressive configuration
pub struct RpcClientBuilder {
    registry: EndpointRegistry,
    max_concurrent: usize,
    enable_stats_cache: bool,
}

/// Error Types RPC handeling
#[derive(Debug, thiserror::Error, Clone)]
pub enum RpcError {
    #[error("Failed to acquire RPC permit within {timeout:?}")]
    PermitAcquisitionTimeout { timeout: Duration },

    #[error("Semaphore closed")]
    SemaphoreClosed,

    #[error("No RPC endpoints available for chain {chain_id}")]
    NoEndpointsAvailable { chain_id: ChainId },

    #[error("All RPC endpoints unhealthy for chain {chain_id}")]
    AllEndpointsUnhealthy { chain_id: ChainId },

    #[error("Transport error: {0}")]
    TransportError(String),
}

// ============================================================================
// method implimentations.

impl RpcClient {
    /// Creates client with specified concurrency limit
    ///
    /// # Arguments
    /// * `max_concurrent_requests` - Global limit across all chains/endpoints
    /// * `endpoint_registry` - custom registry for creating the RpcClient.
    pub fn new(max_concurrent_requests: usize, endpoint_registry: EndpointRegistry) -> Self {
        Self {
            endpoint_registry,
            semaphore: Arc::new(Semaphore::new(max_concurrent_requests)),
            faliure_tacker: Arc::new(DashMap::new()),
            state_cache: Arc::new(DashMap::new()),
        }
    }

    // ========================================================================
    // Primary API: Permit Acquisition + Endpoint Selection

    /// Acquires permit and selects optimal endpoint in one operation
    ///
    /// # Performance
    /// - Semaphore acquisition: O(1) async
    /// - Endpoint selection: O(n) with lock-free metric reads
    /// - Total latency: <100μs typical
    ///
    /// # Example
    /// ```ignore
    /// let context = client
    ///     .acquire_and_select(&chain_id, Some(sender), Duration::from_secs(5))
    ///     .await?;
    ///
    /// let result = context.provider.call(&tx).await;
    /// context.record_success(start.elapsed()); // Lock-free
    /// ```
    pub async fn aquire_and_select(
        &self,
        chain_id: &ChainId,
        sticky_index: Option<usize>,
        timeout: Duration,
    ) -> Result<RpcContext, RpcError> {
        // aquire permit with timeout
        let permit = tokio::time::timeout(timeout, Arc::clone(&self.semaphore).acquire_owned())
            .await
            .map_err(|_| RpcError::PermitAcquisitionTimeout { timeout })?
            .map_err(|_| RpcError::SemaphoreClosed)?;

        // Get pool (lock-free DashMap read)
        let pool = self
            .endpoint_registry
            .get(chain_id)
            .ok_or(RpcError::NoEndpointsAvailable {
                chain_id: *chain_id,
            })?;

        // select strategy
        let strategy = match sticky_index {
            Some(index) => LoadBalancingStrategy::StickySession {
                sticky_index: index,
            },
            None => LoadBalancingStrategy::WeightedLeastResponseTime,
        };

        // select endpoint
        let pool_extract =
            pool.select_endpoint(&strategy)
                .await
                .ok_or(RpcError::AllEndpointsUnhealthy {
                    chain_id: *chain_id,
                })?;

        let (provider, metrics) = (pool_extract.provider(), pool_extract.metrics());
        Ok(RpcContext {
            provider,
            metrics,
            chain_id: *chain_id,
            _permit: permit,
        })
    }

    // ========================================================================
    // Batch Operations (High-Throughput)

    /// Executes multiple RPC calls with automatic load balancing
    ///
    /// Optimized for batch transaction submission or bulk queries.
    /// Distributes load across endpoints automatically.
    ///
    /// # Example
    /// ```ignore
    /// let calls = vec![
    ///     (ChainId::from(1), Some(index1), call1),
    ///     (ChainId::from(1), Some(index2), call2),
    ///     (ChainId::from(137), None, call3),
    /// ];
    ///
    /// let results = client.execute_batch(calls, Duration::from_secs(10)).await;
    /// ```
    pub async fn execute_batch<F, Fut, R>(
        &self,
        calls: Vec<(ChainId, Option<usize>, F)>,
        timeout: Duration,
    ) -> Vec<Result<R, RpcError>>
    where
        F: FnOnce(Arc<dyn Provider + Send + Sync>) -> Fut + Send,
        Fut: Future<Output = Result<R, TransportError>> + Send,
        R: Send,
    {
        let mut join_set = JoinSet::new();
        for (chain_id, sticky_index, call) in calls {
            let client_ref = Arc::new(*self); // Need to clone self for 'static lifetime

            join_set.spawn(async move {
                match client_ref
                    .aquire_and_select(&chain_id, sticky_index, timeout)
                    .await
                {
                    Ok(ctx) => {
                        let start = Instant::now();
                        match call(Arc::clone(&ctx.provider)).await {
                            Ok(result) => {
                                ctx.record_sucess(start.elapsed());
                                Ok(result)
                            }
                            Err(e) => {
                                ctx.record_failure();
                                Err(RpcError::TransportError(e.to_string()))
                            }
                        }
                    }
                    Err(e) => Err(e),
                }
            });
        }
        let mut results = Vec::with_capacity(join_set.len());
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(return_result) => results.push(return_result),
                Err(err) => results.push(Err(RpcError::TransportError(format!(
                    "Batch_Transport: Rpc call panicked: {}",
                    err
                )))),
            }
        }

        results
    }
}
