//! High-throughput RPC client for unary operations
//!
//! Implements handling for HTTP/2 unary operations,
//! using Alloy-native transports with lock-free metrics and fine-grained async locking.

use crate::rpc::{
    LobbyRpcError,
    metrics::{EndpointHealth, EndpointMetrics},
    pool::{EndpointPool, LoadBalancingStrategy, RpcProviderStack, SelectActor},
};
use alloy::{
    providers::{Provider, ProviderBuilder},
    transports::TransportError,
};
use dashmap::DashMap;
use primitives::types::ChainId;
use std::{collections::HashMap, fmt::Debug, future::Future, sync::Arc, time::Duration};
use tokio::{sync::OwnedSemaphorePermit, time::Instant};
use tracing::{debug, trace};
use url::Url;

// ============================================================================
// Constants

/// Cache TTL for endpoint stats (milliseconds)
const STATS_CACHE_TTL_MS: u64 = 1000;

/// Default timeout for provider operations
#[allow(dead_code)]
const DEFAULT_OPERATION_TIMEOUT_MS: u64 = 30000;

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
    pub request_count: u64,
    pub error_count: u64,
}

// ============================================================================
// Provider Context Types

/// Context for unary RPC execution (HTTP/2)
pub struct UnaryContext {
    /// The RPC provider (concretely RootProvider<Ethereum>)
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

    /// Cached stats for monitoring (reduces lock contention)
    stats_cache: Arc<DashMap<ChainId, (Vec<EndpointStats>, Instant)>>,
}

// ============================================================================
// Implementation

impl RpcClient {
    /// Creates a new RPC client with the specified provider stack
    ///
    /// # Arguments
    /// * `provider_stack` - Dual-path stack with unary and subscription registries
    /// * `max_concurrent_requests` - Global limit across all chains/endpoints
    pub fn new(provider_stack: RpcProviderStack) -> Self {
        Self {
            provider_stack,
            stats_cache: Arc::new(DashMap::new()),
        }
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
    /// * `actor` - `SelectActor` enum, seperate providers for `broadcast` and `validator` actor.
    /// * `chain_id` - Target chain for the operation
    /// * `sticky_index` - Optional sticky session index for endpoint affinity
    /// * `timeout` - Maximum time to wait for permit acquisition
    ///
    /// # Example
    /// ```ignore
    /// let (ctx, permit) = client
    ///     .acquire_unary_context(SelectActor::Broadcast, &chain_id, Some(sticky_index), Duration::from_secs(5))
    ///     .await?;
    ///
    /// let result = ctx.provider().send_raw_transaction(&signed_tx).await;
    /// ctx.record_success(start.elapsed()); // Lock-free
    /// ```
    pub async fn acquire_unary_context(
        &self,
        actor: SelectActor,
        chain_id: &ChainId,
        sticky_index: Option<usize>,
        timeout: Duration,
    ) -> Result<(UnaryContext, OwnedSemaphorePermit), LobbyRpcError> {
        // Acquire permit with timeout
        let permit = self.provider_stack.get_semaphore(timeout, &actor).await?;

        // Get unary pool (lock-free DashMap read)
        let pool = self
            .provider_stack
            .get_pool(&actor, *chain_id)
            .ok_or_else(|| LobbyRpcError::NoEndpointsAvailable {
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
            LobbyRpcError::AllEndpointsUnhealthy {
                chain_id: *chain_id,
            }
        })?;

        let endpoint_id = choice.metrics().id().to_string();

        trace!(
            chain_id = %chain_id,
            endpoint_index = choice.index(),
            endpoint_id = %endpoint_id,
            sticky_requested = ?sticky_index,
            "Acquired unary context for {:?}", actor
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

    // ========================================================================
    // Call Operations

    /// Executes unary RPC calls with automatic load balancing
    ///
    /// Acquires a provider context from the weighted pool, executes the operation,
    /// and records success/failure metrics automatically.
    ///
    /// # Type Parameters
    /// * `F` - Closure type taking provider and returning a future
    /// * `Fut` - Future type returned by the closure
    /// * `R` - Result type of the future
    ///
    /// # Example
    /// ```ignore
    /// let result = client
    ///     .execute_unary(
    ///         ChainId::from(1),
    ///         None,
    ///         Duration::from_secs(10),
    ///         |provider| async move { provider.get_block_number().await },
    ///     )
    ///     .await;
    /// ```
    pub async fn execute_unary<F, Fut, R>(
        &self,
        actor: SelectActor,
        chain_id: ChainId,
        sticky_index: Option<usize>,
        timeout: Duration,
        operation: F,
    ) -> Result<R, LobbyRpcError>
    where
        F: FnOnce(Arc<dyn Provider + Send + Sync>) -> Fut,
        Fut: Future<Output = Result<R, TransportError>>,
    {
        // recording duration for unary operation
        let start = Instant::now();

        match self
            .acquire_unary_context(actor, &chain_id, sticky_index, timeout)
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
                        Err(LobbyRpcError::TransportError(e.to_string()))
                    }
                }
            }
            Err(e) => Err(e),
        }
    }

    // ========================================================================
    // Registry Management

    /// Registers a chain with its unary endpoint pool
    pub fn register_chain(
        &self,
        chain_id: ChainId,
        broadcast_pool: Arc<EndpointPool>,
        validator_pool: Arc<EndpointPool>,
    ) {
        self.provider_stack
            .register_chain_pool(chain_id, broadcast_pool, validator_pool);
        self.stats_cache.remove(&chain_id); // Invalidate cache
        debug!(chain_id = %chain_id, "Registered unary chain");
    }

    // ========================================================================
    // Statistics and Monitoring

    /// Gets endpoint statistics for a chain's unary endpoints with caching
    ///
    /// Returns cached data if <1s old to reduce lock contention.
    pub async fn get_unary_endpoint_stats(
        &self,
        actor: SelectActor,
        chain_id: &ChainId,
    ) -> Option<Vec<EndpointStats>> {
        // Check cache first
        if let Some(entry) = self.stats_cache.get(chain_id) {
            let (stats, timestamp) = entry.value();
            if timestamp.elapsed().as_millis() < STATS_CACHE_TTL_MS as u128 {
                return Some(stats.clone());
            }
        }

        // Fetch fresh data from unary pool
        let pool = self.provider_stack.get_pool(&actor, *chain_id)?;
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
    pub async fn refresh_stats(
        &self,
        actor: SelectActor,
        chain_id: &ChainId,
    ) -> Option<Vec<EndpointStats>> {
        self.stats_cache.remove(chain_id);
        self.get_unary_endpoint_stats(actor, chain_id).await
    }

    // ========================================================================
    // Provider Factory Methods (Static)

    /// Creates a unary HTTP provider using Alloy's native HTTP transport
    ///
    /// Uses `ProviderBuilder::connect_http()` which creates a `RootProvider<Ethereum, Http<Client>>`
    /// with hyper-based HTTP/2 via ALPN.
    pub fn create_unary_provider(
        url: &str,
    ) -> Result<Arc<dyn Provider + Send + Sync>, LobbyRpcError> {
        let url = Url::parse(url)
            .map_err(|e| LobbyRpcError::InvalidUrl(format!("Failed to parse URL: {}", e)))?;

        // Validate scheme
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(LobbyRpcError::InvalidUrl(format!(
                "Unary provider requires http:// or https:// scheme, got: {}",
                url.scheme()
            )));
        }

        let provider = ProviderBuilder::new().connect_http(url);
        Ok(Arc::new(provider))
    }

    // ========================================================================
    // Accessor helpers

    #[inline]
    pub fn get_provider_stack(&self) -> &RpcProviderStack {
        &self.provider_stack
    }

    /// Get a dictionary of chains and number of endpoints affiliated to each chain.
    /// **meant to tracing purpose**
    ///
    /// Since broadcast and validator have identicle endpoints, choose any actor.
    pub async fn get_endpoint_hashmap(&self) -> Result<HashMap<ChainId, usize>, LobbyRpcError> {
        let mut endpoint_hashmap: HashMap<ChainId, usize> = HashMap::new();
        let provider_stack = self.get_provider_stack();
        let chains = provider_stack.get_registred_chains();

        for chain_id in chains {
            let endpoint_count = provider_stack
                .get_pool(&SelectActor::Broadcast, chain_id)
                .ok_or_else(|| {
                    LobbyRpcError::EndpointPoolCorrupted(format!(
                        "pool no found for: {:?}",
                        chain_id
                    ))
                })?
                .get_endpoint_count()
                .await;

            endpoint_hashmap.insert(chain_id, endpoint_count);
        }

        Ok(endpoint_hashmap)
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
