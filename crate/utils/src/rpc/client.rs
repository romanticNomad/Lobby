//! High-throughput RPC client with async-optimized request orchestration

use crate::rpc::{
    metrics::{EndpointHealth, EndpointMetrics},
    pool::{EndpointPool, EndpointRegistry, LoadBalancingStrategy},
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

    /// chached states for monitoring (reduce lock contention)
    stats_cache: Arc<DashMap<ChainId, (Vec<EndpointStats>, Instant)>>,
}

/// Context for RPC execution (zero-copy design)
pub struct RpcProviderContext {
    /// The RPC provider
    provider: Arc<dyn Provider + Send + Sync>,

    /// Shared metrics reference (for lock-free recording)
    metrics: Arc<EndpointMetrics>,

    /// ChainId used to index the provier pool
    chain_id: ChainId,

    /// Index of provider on the EndpointPool
    index: usize,
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
    pub fn new_arc(
        max_concurrent_requests: usize,
        endpoint_registry: EndpointRegistry,
    ) -> Arc<Self> {
        Arc::new(Self {
            endpoint_registry,
            semaphore: Arc::new(Semaphore::new(max_concurrent_requests)),
            stats_cache: Arc::new(DashMap::new()),
        })
    }

    /// dummy method: for testing;
    pub fn new(max_concurrent_requests: usize) -> Self {
        Self {
            endpoint_registry: Arc::new(DashMap::new()),
            semaphore: Arc::new(Semaphore::new(max_concurrent_requests)),
            stats_cache: Arc::new(DashMap::new()),
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
    ) -> Result<(RpcProviderContext, OwnedSemaphorePermit), RpcError> {
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

        let (provider, metrics, index) = (
            pool_extract.provider(),
            pool_extract.metrics(),
            pool_extract.index(),
        );
        Ok((
            RpcProviderContext {
                provider,
                metrics,
                chain_id: *chain_id,
                index,
            },
            permit,
        ))
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
        self: Arc<Self>,
        calls: Vec<(ChainId, Option<usize>, F)>,
        timeout: Duration,
    ) -> Vec<Result<R, RpcError>>
    where
        F: FnOnce(Arc<dyn Provider + Send + Sync>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<R, TransportError>> + Send,
        R: Send + 'static,
    {
        let mut join_set = JoinSet::new();
        for (chain_id, sticky_index, call) in calls {
            let client_ref = Arc::clone(&self);

            join_set.spawn(async move {
                match client_ref
                    .aquire_and_select(&chain_id, sticky_index, timeout)
                    .await
                {
                    Ok((ctx, permit)) => {
                        let start = Instant::now();
                        match call(Arc::clone(&ctx.provider)).await {
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
                Err(err) => results.push(Err(RpcError::TransportError(format!(
                    "Batch_Transport: JoinSet panicked: {}",
                    err
                )))),
            }
        }

        results
    }

    // ========================================================================
    // Registry Management

    /// Registers a chain with its endpoint pool
    pub fn register_chain(&self, chain_id: ChainId, pool: Arc<EndpointPool>) {
        self.endpoint_registry.insert(chain_id, pool);
        self.stats_cache.remove(&chain_id); // Invalidate cache
    }

    pub fn registered_chain_count(&self) -> usize {
        self.endpoint_registry.len()
    }

    /// Gets endpoint statistics with caching
    ///
    /// Returns cached data if <1s old to reduce lock contention.
    pub async fn get_endpoint_stats(&self, chain_id: &ChainId) -> Option<Vec<EndpointStats>> {
        // Check cache first
        if let Some(entry) = self.stats_cache.get(chain_id) {
            let (stats, timestamp) = entry.value();
            if timestamp.elapsed().as_millis() < STATS_CACHE_TTL_MS as u128 {
                return Some(stats.clone());
            }
        }

        // Fetch fresh data
        let pool = self.endpoint_registry.get(chain_id)?;
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
                block_height: s.block_height,
            })
            .collect();

        // Update cache
        self.stats_cache
            .insert(*chain_id, (stats.clone(), Instant::now()));

        Some(stats)
    }

    /// Force refresh of cached stats
    pub async fn refresh_stats(&self, chain_id: &ChainId) -> Option<Vec<EndpointStats>> {
        self.stats_cache.remove(chain_id);
        self.get_endpoint_stats(chain_id).await
    }

    /// Gets current semaphore availability
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}

// ============================================================================
// RpcProviderContext Methods

impl RpcProviderContext {
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
        self.metrics.id()
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
}

// ============================================================================
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_concurrent_permit_acquisition() {
        let client = Arc::new(RpcClient::new(5));
        let counter = Arc::new(AtomicUsize::new(0));

        let mut join_set = JoinSet::new();
        for _ in 0..10 {
            let client_clone = Arc::clone(&client);
            let counter_clone = Arc::clone(&counter);

            join_set.spawn(async move {
                // Simulate work with permit
                let _permit = client_clone
                    .semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .unwrap();
                counter_clone.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
            });
        }

        // All 10 should complete despite limit of 5 (queued)
        let _ = join_set.join_all().await;
        
        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }
}

// ============================================================================
