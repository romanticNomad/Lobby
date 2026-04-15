//! RPC endpoint pool management for unary operations
//!
//! Implements pools for HTTP/2 unary operations,
//! using lock-free metrics and fine-grained async locking for high throughput.

use crate::rpc::metrics::{EndpointMetrics, EndpointMetricsSnapshot};
use alloy::providers::Provider;
use dashmap::DashMap;
use primitives::types::ChainId;
use std::{
    fmt::Debug,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
use tokio::sync::RwLock;

// ============================================================================
// Constants

/// Cache TTL for healthy endpoint indices (seconds)
pub const CACHE_TTL_SECS: u64 = 5;

/// Default capacity for endpoint pools
const DEFAULT_POOL_CAPACITY: usize = 16;

// ============================================================================
// Type Aliases

/// Registry mapping chain IDs to their endpoint pools (unary)
pub type EndpointRegistry = Arc<DashMap<ChainId, Arc<EndpointPool>>>;

// ============================================================================
// SelectActor

/// Enables the selection of seperate `EndpointRegistry` for
/// `broadcast` or `validator` actor.
pub enum SelectActor {
    Broadcast,
    Validator
}

// ============================================================================
// Provider Stack

/// Registry stack for unary (HTTP/2) RPC operations.
///
/// Architecture:
/// `HTTP/2` connection pools for stateless request-response operations.
/// manages registries for `broadcast` and `validator` actors simultanously, for consistent operations.
#[derive(Clone)]
pub struct RpcProviderStack {
    broadcast: EndpointRegistry,
    validator: EndpointRegistry,
}

impl RpcProviderStack {
    /// Creates new stack with unary registry
    pub fn new() -> Self {
        Self {
            broadcast: Arc::new(DashMap::new()),
            validator: Arc::new(DashMap::new()),
        }
    }

    /// Gets pools for unary operations (HTTP/2)
    /// seperatly for `broadcast` and `validator` actor.
    pub fn get_pool(
        &self,
        actor: SelectActor,
        chain_id: ChainId,
    ) -> Option<Arc<EndpointPool>> {
        match actor {
            SelectActor::Broadcast => {
                self
                .broadcast
                .get(&chain_id)
                .map(|entry| Arc::clone(entry.value()))
            },
            SelectActor::Validator => {
                self
                .validator
                .get(&chain_id)
                .map(|entry| Arc::clone(entry.value()))
            }
        }
    }

    /// Registers a chain with its unary endpoint pool (same endpoint is registered for both actors for consistency)
    pub fn register_chain(&self, chain_id: ChainId, pool: Arc<EndpointPool>) {
        self.broadcast.insert(chain_id, Arc::clone(&pool));
        self.validator.insert(chain_id, Arc::clone(&pool));
    }

    /// Gets total number of chains in broadcast_registry
    pub fn broadcast_chain_count(&self) -> usize {
        self.broadcast.len()
    }

    /// Gets total number of chains in validator_registry
    pub fn validator_chain_count(&self) -> usize {
        self.validator.len()
    }
}

impl Default for RpcProviderStack {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Unary Endpoint Pool (HTTP/2)

/// High-performance endpoint pool for unary HTTP/2 operations
///
/// Architecture:
/// - DashMap for registry-level concurrency (lock-free reads)
/// - Vec<Arc<EndpointEntry>> for lock-free metric reads
/// - RwLock only for structural changes (add/remove endpoints)
/// - Cached healthy endpoint indices with TTL
#[derive(Debug)]
pub struct EndpointPool {
    /// ChainId for the blockchain supported by this pool
    chain_id: ChainId,

    /// Endpoints with shared metrics (Arc for lock-free access)
    endpoints: RwLock<Vec<Arc<EndpointEntry>>>,

    /// Cached healthy endpoint indices, periodically updated
    healthy_cache: RwLock<Vec<usize>>,

    /// Last cache update timestamp (seconds since epoch)
    cache_timestamp: AtomicU64,
}

/// Single endpoint entry with shared ownership for unary operations
pub struct EndpointEntry {
    /// The RPC provider implementation (concretely RootProvider<Ethereum>)
    provider: Arc<dyn Provider + Send + Sync>,

    /// Shared metrics (lock-free atomic operations)
    metrics: Arc<EndpointMetrics>,
}

impl Debug for EndpointEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndpointEntry")
            .field("metrics", &self.metrics)
            .finish()
    }
}

impl EndpointEntry {
    /// Creates a new endpoint entry
    pub fn new(provider: Arc<dyn Provider + Send + Sync>, metrics: Arc<EndpointMetrics>) -> Self {
        Self { provider, metrics }
    }

    /// Returns reference to the provider
    #[inline]
    pub fn provider(&self) -> Arc<dyn Provider + Send + Sync> {
        Arc::clone(&self.provider)
    }

    /// Returns reference to the metrics
    #[inline]
    pub fn metrics(&self) -> Arc<EndpointMetrics> {
        Arc::clone(&self.metrics)
    }
}

impl EndpointPool {
    /// Creates new endpoint pool for a specific chain
    pub fn new(chain_id: ChainId) -> Self {
        Self {
            chain_id,
            endpoints: RwLock::new(Vec::with_capacity(DEFAULT_POOL_CAPACITY)),
            healthy_cache: RwLock::new(Vec::new()),
            cache_timestamp: AtomicU64::new(0),
        }
    }

    /// Returns the chain ID for this pool
    #[inline]
    pub fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    /// Adds endpoint to pool (acquires write lock briefly)
    pub async fn add_endpoint(
        &self,
        provider: Arc<dyn Provider + Send + Sync>,
        metrics: EndpointMetrics,
    ) {
        let entry = EndpointEntry::new(provider, Arc::new(metrics));
        let mut endpoints = self.endpoints.write().await;
        endpoints.push(Arc::new(entry));
        drop(endpoints);

        // Update cache
        self.update_healthy_cache().await;
    }

    /// Number of endpoints present in the pool
    pub async fn endpoint_count(&self) -> usize {
        self.endpoints.read().await.len()
    }

    /// Count healthy endpoints using cached data when fresh
    pub async fn healthy_endpoint_count(&self) -> usize {
        // Check cache freshness
        let now = Instant::now().elapsed().as_secs();
        let cache_time = self.cache_timestamp.load(Ordering::Acquire);

        if now.saturating_sub(cache_time) < CACHE_TTL_SECS {
            return self.healthy_cache.read().await.len();
        }

        // Cache expired, recalculate
        self.update_healthy_cache().await;
        self.healthy_cache.read().await.len()
    }

    /// Updates healthy endpoint cache (internal)
    async fn update_healthy_cache(&self) {
        let endpoints = self.endpoints.read().await;
        let mut healthy_indices = Vec::with_capacity(endpoints.len());

        for (idx, entry) in endpoints.iter().enumerate() {
            // Lock-free health check
            if entry.metrics.is_available() {
                healthy_indices.push(idx);
            }
        }

        let now = Instant::now().elapsed().as_secs();
        let mut cache = self.healthy_cache.write().await;
        *cache = healthy_indices;
        drop(cache);

        self.cache_timestamp.store(now, Ordering::Release);
    }

    // ========================================================================
    // Endpoint Selection (Async-Optimized)

    /// Selects endpoint using specified strategy
    ///
    /// # Performance
    /// - Weighted selection: O(n) with lock-free metric reads
    /// - Sticky session: O(1) with index-based lookup
    pub async fn select_endpoint(
        &self,
        strategy: &LoadBalancingStrategy,
    ) -> Option<LoadBalancerChoice> {
        match strategy {
            LoadBalancingStrategy::WeightedLeastResponseTime => {
                self.select_by_weighted_score().await
            }
            LoadBalancingStrategy::StickySession { sticky_index } => {
                self.select_by_sticky_session(*sticky_index).await
            }
        }
    }

    /// Weighted least response time selection using roulette wheel algorithm
    ///
    /// Algorithm:
    /// 1. Collect all healthy endpoints with scores (lock-free reads)
    /// 2. Use weighted random selection (roulette wheel)
    ///
    /// Time: O(n) where n = endpoint count
    async fn select_by_weighted_score(&self) -> Option<LoadBalancerChoice> {
        let endpoints = self.endpoints.read().await;

        if endpoints.is_empty() {
            return None;
        }

        // Collect scores (lock-free reads)
        let mut scored_endpoints: Vec<(usize, f64)> = Vec::with_capacity(endpoints.len());
        let mut total_score: f64 = 0.0;

        for (idx, entry) in endpoints.iter().enumerate() {
            let score = entry.metrics.load_balancing_score();
            if score > 0.0 {
                scored_endpoints.push((idx, score));
                total_score += score;
            }
        }

        drop(endpoints);

        if scored_endpoints.is_empty() {
            return self.select_circuit_breaker_recovery().await;
        }

        // Weighted random selection (roulette wheel)
        let threshold = fastrand::f64() * total_score;
        let mut cumulative = 0.0;
        let mut selected_idx = 0;

        for (idx, score) in &scored_endpoints {
            cumulative += *score;
            if cumulative >= threshold {
                selected_idx = *idx;
                break;
            }
        }

        // Fallback to last if no selection made (floating point edge case)
        if cumulative < threshold {
            selected_idx = scored_endpoints.last()?.0;
        }

        let endpoints = self.endpoints.read().await;
        let entry = endpoints.get(selected_idx)?;

        Some(LoadBalancerChoice::new(
            entry.provider(),
            entry.metrics(),
            selected_idx,
        ))
    }

    /// Sticky session with index-based endpoint selection
    ///
    /// Verifies that the endpoint at the given index is healthy.
    /// Falls back to weighted selection if the endpoint is unhealthy.
    ///
    /// This allows sticky sessions to be synchronized with the selection
    /// returned by weighted score or round-robin strategies.
    ///
    /// Time: O(1) average case
    async fn select_by_sticky_session(&self, sticky_index: usize) -> Option<LoadBalancerChoice> {
        let endpoints = self.endpoints.read().await;

        if endpoints.is_empty() {
            return None;
        }

        // Check if the requested index is valid and endpoint is healthy
        if let Some(entry) = endpoints.get(sticky_index) {
            if entry.metrics.is_available() {
                return Some(LoadBalancerChoice::new(
                    entry.provider(),
                    entry.metrics(),
                    sticky_index,
                ));
            }
        }

        drop(endpoints);

        // Fallback to weighted selection if endpoint is unhealthy or invalid
        self.select_by_weighted_score().await
    }

    /// Circuit breaker recovery selection
    ///
    /// Attempts to find endpoints where circuit breaker has expired.
    /// Falls back to first endpoint as last resort.
    ///
    /// Time: O(n)
    async fn select_circuit_breaker_recovery(&self) -> Option<LoadBalancerChoice> {
        let endpoints = self.endpoints.read().await;
        let now = Instant::now();

        // Find expired circuit breaker
        for (idx, entry) in endpoints.iter().enumerate() {
            if let Some(expiry) = entry.metrics.circuit_breaker_until() {
                if now > expiry {
                    return Some(LoadBalancerChoice::new(
                        entry.provider(),
                        entry.metrics(),
                        idx,
                    ));
                }
            }
        }

        // Last resort: return the first endpoint irrespective of health
        endpoints
            .first()
            .map(|entry| LoadBalancerChoice::new(entry.provider(), entry.metrics(), 0))
    }

    // ========================================================================
    // Metric Lookups (Lock-Free via Arc)

    /// Returns snapshot of all metrics (for monitoring)
    ///
    /// Time: O(n), creates clones to avoid holding locks
    pub async fn endpoints_metrics(&self) -> Vec<EndpointMetricsSnapshot> {
        let endpoints = self.endpoints.read().await;

        endpoints
            .iter()
            .map(|entry| entry.metrics.snapshot())
            .collect()
    }

    /// Find endpoint by ID (for targeted operations)
    pub async fn find_endpoint(&self, endpoint_id: &str) -> Option<Arc<EndpointMetrics>> {
        let endpoints = self.endpoints.read().await;

        endpoints
            .iter()
            .find(|e| e.metrics.id() == endpoint_id)
            .map(|e| e.metrics())
    }

    /// Gets the best available endpoint index for sticky session initialization
    ///
    /// Returns the index of the highest-scored healthy endpoint
    pub async fn best_endpoint_index(&self) -> Option<usize> {
        let choice = self.select_by_weighted_score().await?;
        Some(choice.index)
    }
}

// ============================================================================
// Load Balancing Strategy

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadBalancingStrategy {
    /// Uses response time and health metrics for intelligent selection
    WeightedLeastResponseTime,

    /// Sticky session based on endpoint index (best for nonce consistency)
    StickySession { sticky_index: usize },
}

impl LoadBalancingStrategy {
    /// Creates a weighted least response time strategy
    #[inline]
    pub fn weighted() -> Self {
        Self::WeightedLeastResponseTime
    }

    /// Creates a sticky session strategy
    #[inline]
    pub fn sticky(sticky_index: usize) -> Self {
        Self::StickySession { sticky_index }
    }
}

// ============================================================================
// Load Balancer Choice Types

/// Result of unary endpoint selection
pub struct LoadBalancerChoice {
    provider: Arc<dyn Provider + Send + Sync>,
    metrics: Arc<EndpointMetrics>,
    index: usize,
}

impl LoadBalancerChoice {
    pub fn new(
        provider: Arc<dyn Provider + Send + Sync>,
        metrics: Arc<EndpointMetrics>,
        index: usize,
    ) -> Self {
        Self {
            provider,
            metrics,
            index,
        }
    }

    #[inline]
    pub fn provider(&self) -> Arc<dyn Provider + Send + Sync> {
        Arc::clone(&self.provider)
    }

    #[inline]
    pub fn metrics(&self) -> Arc<EndpointMetrics> {
        Arc::clone(&self.metrics)
    }

    #[inline]
    pub fn index(&self) -> usize {
        self.index
    }
}

impl Debug for LoadBalancerChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadBalancerChoice")
            .field("index", &self.index)
            .field("metrics", &self.metrics)
            .finish()
    }
}

// ============================================================================
