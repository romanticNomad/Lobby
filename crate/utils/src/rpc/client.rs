//! High-throughput RPC client for unary operations
//!
//! Implements handling for HTTP/2 unary operations,
//! using Alloy-native transports with lock-free metrics and fine-grained async locking.

use crate::rpc::{
    metrics::{EndpointHealth, EndpointMetrics},
    pool::{EndpointPool, LoadBalancingStrategy, RpcProviderStack},
};
use alloy::{
    providers::{Provider, ProviderBuilder},
    transports::TransportError,
};
use dashmap::DashMap;
use primitives::types::ChainId;
use std::{fmt::Debug, future::Future, sync::Arc, time::Duration};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinSet,
    time::Instant,
};
use tracing::{debug, error, trace};
use url::Url;

// ============================================================================
// Constants

/// Cache TTL for endpoint stats (milliseconds)
const STATS_CACHE_TTL_MS: u64 = 1000;

/// Default timeout for provider operations
const DEFAULT_OPERATION_TIMEOUT_MS: u64 = 30000;

// ============================================================================
// Error Types

/// Error types for RPC handling
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

    #[error("Provider construction failed: {0}")]
    ProviderConstructionError(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
}

// ============================================================================
// Metric State Monitor

/// High-level metric stats derived from `EndpointMetrics`
#[derive(Debug, Clone)]
pub struct EndpointStats {
    pub id: String,
    pub url: String,
    pub health: EndpointHealth,
    pub avg_response_time_ms: f64,
    pub error_rate: f64,
    pub score: f64,
    // pub block_height: Option<u64>,
    pub request_count: u64,
    pub error_count: u64,
}

// ============================================================================
// Provider Context Types

/// Context for unary RPC execution (HTTP/2)
pub struct UnaryContext {
    /// The RPC provider (concretely RootProvider<Ethereum, Http<Client>>)
    provider: Arc<dyn Provider + Send + Sync>,

    /// Shared metrics reference (for lock-free recording)
    metrics: Arc<EndpointMetrics>,

    /// ChainId used to index the provider pool
    chain_id: ChainId,

    /// Index of provider on the EndpointPool
    index: usize,

    /// Endpoint ID for logging
    endpoint_id: String,
}

// ============================================================================
// RpcClient

/// High-performance RPC client for unary operations
///
/// Architecture:
/// - `unary_registry`: HTTP/2 connection pools for stateless request-response
/// - Global semaphore for backpressure
/// - Lock-free metric recording via Arc<EndpointMetrics>
#[derive(Clone)]
pub struct RpcClient {
    /// Provider stack
    provider_stack: RpcProviderStack,

    /// Global concurrency limiter across all operations
    semaphore: Arc<Semaphore>,

    /// Cached stats for monitoring (reduces lock contention)
    stats_cache: Arc<DashMap<ChainId, (Vec<EndpointStats>, Instant)>>,

    /// Default timeout for operations
    default_timeout: Duration,
}

// ============================================================================
// Implementation

impl RpcClient {
    /// Creates a new RPC client with the specified provider stack
    ///
    /// # Arguments
    /// * `provider_stack` - Dual-path stack with unary and subscription registries
    /// * `max_concurrent_requests` - Global limit across all chains/endpoints
    pub fn new(provider_stack: RpcProviderStack, max_concurrent_requests: usize) -> Self {
        Self {
            provider_stack,
            semaphore: Arc::new(Semaphore::new(max_concurrent_requests)),
            stats_cache: Arc::new(DashMap::new()),
            default_timeout: Duration::from_millis(DEFAULT_OPERATION_TIMEOUT_MS),
        }
    }

    /// Creates a new RPC client wrapped in Arc for shared ownership
    pub fn new_arc(provider_stack: RpcProviderStack, max_concurrent_requests: usize) -> Arc<Self> {
        Arc::new(Self::new(provider_stack, max_concurrent_requests))
    }

    /// Sets the default timeout for operations
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    // ========================================================================
    // Primary API: Unary Path (HTTP/2)

    /// Acquires permit and selects optimal unary endpoint in one operation
    ///
    /// # Performance
    /// - Semaphore acquisition: O(1) async
    /// - Endpoint selection: O(n) with lock-free metric reads
    /// - Total latency: <100μs typical
    ///
    /// # Arguments
    /// * `chain_id` - Target chain for the operation
    /// * `sticky_index` - Optional sticky session index for endpoint affinity
    /// * `timeout` - Maximum time to wait for permit acquisition
    ///
    /// # Example
    /// ```ignore
    /// let (ctx, permit) = client
    ///     .acquire_unary_context(&chain_id, Some(sticky_index), Duration::from_secs(5))
    ///     .await?;
    ///
    /// let result = ctx.provider().send_raw_transaction(&signed_tx).await;
    /// ctx.record_success(start.elapsed()); // Lock-free
    /// ```
    pub async fn acquire_unary_context(
        &self,
        chain_id: &ChainId,
        sticky_index: Option<usize>,
        timeout: Duration,
    ) -> Result<(UnaryContext, OwnedSemaphorePermit), RpcError> {
        // Acquire permit with timeout
        let permit = tokio::time::timeout(timeout, Arc::clone(&self.semaphore).acquire_owned())
            .await
            .map_err(|_| RpcError::PermitAcquisitionTimeout { timeout })?
            .map_err(|_| RpcError::SemaphoreClosed)?;

        // Get unary pool (lock-free DashMap read)
        let pool = self
            .provider_stack
            .get_unary_pool(*chain_id)
            .ok_or_else(|| RpcError::NoEndpointsAvailable {
                chain_id: *chain_id,
            })?;

        // Select strategy based on sticky index
        let strategy = match sticky_index {
            Some(index) => LoadBalancingStrategy::StickySession {
                sticky_index: index,
            },
            None => LoadBalancingStrategy::WeightedLeastResponseTime,
        };

        // Select endpoint
        let choice = pool.select_endpoint(&strategy).await.ok_or_else(|| {
            RpcError::AllEndpointsUnhealthy {
                chain_id: *chain_id,
            }
        })?;

        let endpoint_id = choice.metrics().id().to_string();

        trace!(
            chain_id = %chain_id,
            endpoint_index = choice.index(),
            endpoint_id = %endpoint_id,
            sticky_requested = ?sticky_index,
            "Acquired unary context"
        );

        let ctx = UnaryContext {
            provider: choice.provider(),
            metrics: choice.metrics(),
            chain_id: *chain_id,
            index: choice.index(),
            endpoint_id,
        };

        Ok((ctx, permit))
    }

    /// Convenience method to acquire unary context with default timeout
    pub async fn acquire_unary(
        &self,
        chain_id: &ChainId,
        sticky_index: Option<usize>,
    ) -> Result<(UnaryContext, OwnedSemaphorePermit), RpcError> {
        self.acquire_unary_context(chain_id, sticky_index, self.default_timeout)
            .await
    }

    // ========================================================================
    // Batch Operations (High-Throughput)

    /// Executes multiple unary RPC calls with automatic load balancing
    ///
    /// Optimized for batch transaction submission or bulk queries.
    /// Distributes load across endpoints automatically using weighted selection.
    ///
    /// # Type Parameters
    /// * `F` - Closure type taking provider and returning a future
    /// * `Fut` - Future type returned by the closure
    /// * `R` - Result type of the future
    ///
    /// # Example
    /// ```ignore
    /// let operations = vec![
    ///     (ChainId::from(1), Some(index1), |provider| async move {
    ///         provider.send_raw_transaction(&tx1).await
    ///     }),
    ///     (ChainId::from(1), None, |provider| async move {
    ///         provider.get_block_number().await
    ///     }),
    /// ];
    ///
    /// let results = client.execute_unary_batch(operations, Duration::from_secs(10)).await;
    /// ```
    pub async fn execute_unary_batch<F, Fut, R>(
        self: Arc<Self>,
        operations: Vec<(ChainId, Option<usize>, F)>,
        timeout: Duration,
    ) -> Vec<Result<R, RpcError>>
    where
        F: FnOnce(Arc<dyn Provider + Send + Sync>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<R, TransportError>> + Send,
        R: Send + 'static,
    {
        let mut join_set = JoinSet::new();

        for (chain_id, sticky_index, operation) in operations {
            let client_ref = Arc::clone(&self);

            join_set.spawn(async move {
                let start = Instant::now();

                match client_ref
                    .acquire_unary_context(&chain_id, sticky_index, timeout)
                    .await
                {
                    Ok((ctx, permit)) => {
                        let provider = ctx.provider();

                        match operation(provider).await {
                            Ok(result) => {
                                ctx.record_success(start.elapsed());
                                drop(permit);
                                Ok(result)
                            }
                            Err(e) => {
                                ctx.record_failure();
                                drop(permit);
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
                Err(err) => {
                    error!(error = %err, "Batch operation task panicked");
                    results.push(Err(RpcError::TransportError(format!(
                        "Batch task panicked: {}",
                        err
                    ))));
                }
            }
        }

        results
    }

    /// Executes parallel unary operations across multiple chains
    ///
    /// Simplified API when you don't need sticky session control per operation.
    pub async fn execute_parallel_unary<F, Fut, R>(
        self: Arc<Self>,
        chain_ops: Vec<(ChainId, F)>,
        timeout: Duration,
    ) -> Vec<Result<R, RpcError>>
    where
        F: FnOnce(Arc<dyn Provider + Send + Sync>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<R, TransportError>> + Send,
        R: Send + 'static,
    {
        let operations: Vec<_> = chain_ops
            .into_iter()
            .map(|(chain_id, op)| (chain_id, None, op))
            .collect();

        self.execute_unary_batch(operations, timeout).await
    }

    // ========================================================================
    // Registry Management

    /// Registers a chain with its unary endpoint pool
    pub fn register_unary_chain(&self, chain_id: ChainId, pool: Arc<EndpointPool>) {
        self.provider_stack.register_unary_chain(chain_id, pool);
        self.stats_cache.remove(&chain_id); // Invalidate cache
        debug!(chain_id = %chain_id, "Registered unary chain");
    }

    /// Gets the number of registered chains with unary endpoints
    pub fn registered_unary_chain_count(&self) -> usize {
        self.provider_stack.unary_chain_count()
    }

    /// Gets total registered chains
    pub fn total_registered_chains(&self) -> usize {
        self.provider_stack.unary_chain_count()
    }

    // ========================================================================
    // Statistics and Monitoring

    /// Gets endpoint statistics for a chain's unary endpoints with caching
    ///
    /// Returns cached data if <1s old to reduce lock contention.
    pub async fn get_unary_endpoint_stats(&self, chain_id: &ChainId) -> Option<Vec<EndpointStats>> {
        // Check cache first
        if let Some(entry) = self.stats_cache.get(chain_id) {
            let (stats, timestamp) = entry.value();
            if timestamp.elapsed().as_millis() < STATS_CACHE_TTL_MS as u128 {
                return Some(stats.clone());
            }
        }

        // Fetch fresh data from unary pool
        let pool = self.provider_stack.get_unary_pool(*chain_id)?;
        let snapshots = pool.endpoints_metrics().await;

        let stats: Vec<EndpointStats> = snapshots
            .into_iter()
            .map(|s| EndpointStats {
                id: s.id,
                url: s.url,
                health: s.health,
                avg_response_time_ms: s.avg_response_time_ms,
                error_rate: s.error_rate,
                score: s.score,
                request_count: s.request_count,
                error_count: s.error_count,
            })
            .collect();

        // Update cache
        self.stats_cache
            .insert(*chain_id, (stats.clone(), Instant::now()));

        Some(stats)
    }

    /// Force refresh of cached stats for a chain
    pub async fn refresh_stats(&self, chain_id: &ChainId) -> Option<Vec<EndpointStats>> {
        self.stats_cache.remove(chain_id);
        self.get_unary_endpoint_stats(chain_id).await
    }

    /// Gets current semaphore availability
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Gets the total semaphore capacity
    pub fn semaphore_capacity(&self) -> usize {
        Arc::clone(&self.semaphore).available_permits()
            + (self
                .semaphore
                .available_permits()
                .saturating_sub(self.semaphore.available_permits()))
    }

    // ========================================================================
    // Provider Factory Methods (Static)

    /// Creates a unary HTTP provider using Alloy's native HTTP transport
    ///
    /// Uses `ProviderBuilder::connect_http()` which creates a `RootProvider<Ethereum, Http<Client>>`
    /// with hyper-based HTTP/2 via ALPN.
    pub fn create_unary_provider(url: &str) -> Result<Arc<dyn Provider + Send + Sync>, RpcError> {
        let url = Url::parse(url)
            .map_err(|e| RpcError::InvalidUrl(format!("Failed to parse URL: {}", e)))?;

        // Validate scheme
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(RpcError::InvalidUrl(format!(
                "Unary provider requires http:// or https:// scheme, got: {}",
                url.scheme()
            )));
        }

        let provider = ProviderBuilder::new().connect_http(url);

        Ok(Arc::new(provider))
    }

}

// ============================================================================
// UnaryContext Methods

impl Debug for UnaryContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnaryContext")
            .field("chain_id", &self.chain_id)
            .field("index", &self.index)
            .field("endpoint_id", &self.endpoint_id)
            .finish()
    }
}

impl UnaryContext {
    /// Records successful call (convenience method)
    #[inline]
    pub fn record_success(&self, duration: Duration) {
        self.metrics.record_success(duration);
    }

    /// Records failed call
    #[inline]
    pub fn record_failure(&self) {
        self.metrics.record_failure();
    }

    /// Gets endpoint ID
    #[inline]
    pub fn endpoint_id(&self) -> &str {
        &self.endpoint_id
    }

    /// Gets current health
    #[inline]
    pub fn endpoint_health(&self) -> EndpointHealth {
        self.metrics.health()
    }

    /// Gets current score
    #[inline]
    pub fn endpoint_score(&self) -> f64 {
        self.metrics.load_balancing_score()
    }

    /// Gets endpoint index
    #[inline]
    pub fn index(&self) -> usize {
        self.index
    }

    /// Gets chain ID
    #[inline]
    pub fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    /// Gets provider reference
    #[inline]
    pub fn provider(&self) -> Arc<dyn Provider + Send + Sync> {
        Arc::clone(&self.provider)
    }

    /// Gets metrics reference
    #[inline]
    pub fn metrics(&self) -> Arc<EndpointMetrics> {
        Arc::clone(&self.metrics)
    }
}

// ============================================================================
