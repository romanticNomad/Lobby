//! RPC endpoint pool management with dual-path architecture (unary/subscription)
//!
//! Implements separate pools for HTTP/2 unary operations and WebSocket subscriptions,
//! using lock-free metrics and fine-grained async locking for high throughput.

use crate::rpc::metrics::{EndpointMetrics, EndpointMetricsSnapshot};
use alloy::providers::Provider;
use dashmap::DashMap;
use primitives::types::ChainId;
use std::{
    fmt::Debug,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
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

/// Registry for WebSocket subscription pools
pub type SubscriptionRegistry = Arc<DashMap<ChainId, Arc<WsPool>>>;

// ============================================================================
// Dual-Path Provider Stack

/// Dual-registry stack for workload isolation between unary and subscription paths
///
/// Architecture:
/// - `unary`: HTTP/2 connection pools for stateless request-response operations
/// - `subscription`: WebSocket session pools for stateful server-push operations
#[derive(Clone)]
pub struct RpcProviderStack {
    /// Unary registry: HTTP/2 connection pools per chain
    pub unary: EndpointRegistry,

    /// Subscription registry: WebSocket session pools per chain
    pub subscription: SubscriptionRegistry,
}

impl RpcProviderStack {
    /// Creates new stack with separate registries
    pub fn new() -> Self {
        Self {
            unary: Arc::new(DashMap::new()),
            subscription: Arc::new(DashMap::new()),
        }
    }

    /// Gets pool for unary operations (HTTP/2)
    pub fn get_unary_pool(&self, chain_id: ChainId) -> Option<Arc<EndpointPool>> {
        self.unary
            .get(&chain_id)
            .map(|entry| Arc::clone(entry.value()))
    }

    /// Gets pool for subscription operations (WebSocket)
    pub fn get_subscription_pool(&self, chain_id: ChainId) -> Option<Arc<WsPool>> {
        self.subscription
            .get(&chain_id)
            .map(|entry| Arc::clone(entry.value()))
    }

    /// Registers a chain with its unary endpoint pool
    pub fn register_unary_chain(&self, chain_id: ChainId, pool: Arc<EndpointPool>) {
        self.unary.insert(chain_id, pool);
    }

    /// Registers a chain with its subscription pool
    pub fn register_subscription_chain(&self, chain_id: ChainId, pool: Arc<WsPool>) {
        self.subscription.insert(chain_id, pool);
    }

    /// Gets total number of chains with unary endpoints
    pub fn unary_chain_count(&self) -> usize {
        self.unary.len()
    }

    /// Gets total number of chains with subscription endpoints
    pub fn subscription_chain_count(&self) -> usize {
        self.subscription.len()
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

    /// Round-robin counter for uniform distribution strategy
    round_robin_counter: AtomicUsize,
}

/// Single endpoint entry with shared ownership for unary operations
pub struct EndpointEntry {
    /// The RPC provider implementation (concretely RootProvider<Ethereum, Http<Client>>)
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
            round_robin_counter: AtomicUsize::new(0),
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
    /// - Round-robin: O(1) atomic increment
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
            LoadBalancingStrategy::RoundRobin => self.select_round_robin().await,
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

    /// Simple round-robin for uniform distribution
    ///
    /// Uses atomic increment for lock-free coordination across threads.
    async fn select_round_robin(&self) -> Option<LoadBalancerChoice> {
        let endpoints = self.endpoints.read().await;

        if endpoints.is_empty() {
            return None;
        }

        // Get next index atomically
        let idx = self.round_robin_counter.fetch_add(1, Ordering::Relaxed) % endpoints.len();
        let entry = endpoints.get(idx)?;

        Some(LoadBalancerChoice::new(
            entry.provider(),
            entry.metrics(),
            idx,
        ))
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
// WebSocket Subscription Pool

/// Pool for WebSocket subscription endpoints
///
/// WebSocket connections are stateful and long-lived, so we use
/// round-robin assignment with session affinity per subscription.
#[derive(Debug)]
pub struct WsPool {
    /// ChainId for the blockchain
    chain_id: ChainId,

    /// WebSocket endpoints
    endpoints: RwLock<Vec<Arc<WsEndpointEntry>>>,

    /// Round-robin counter for assignment
    counter: AtomicUsize,

    /// Active connection count per endpoint (for load balancing)
    connection_counts: Vec<AtomicUsize>,
}

/// WebSocket endpoint entry
pub struct WsEndpointEntry {
    /// The WebSocket provider (concretely RootProvider<Ethereum, Ws>)
    provider: Arc<dyn Provider + Send + Sync>,

    /// Endpoint metadata
    metrics: Arc<EndpointMetrics>,

    /// URL for reconnection
    url: String,
}

impl Debug for WsEndpointEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WsEndpointEntry")
            .field("url", &self.url)
            .field("metrics", &self.metrics)
            .finish()
    }
}

impl WsEndpointEntry {
    /// Creates a new WebSocket endpoint entry
    pub fn new(
        provider: Arc<dyn Provider + Send + Sync>,
        metrics: Arc<EndpointMetrics>,
        url: String,
    ) -> Self {
        Self {
            provider,
            metrics,
            url,
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
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl WsPool {
    /// Creates a new WebSocket pool for a chain
    pub fn new(chain_id: ChainId) -> Self {
        Self {
            chain_id,
            endpoints: RwLock::new(Vec::with_capacity(DEFAULT_POOL_CAPACITY)),
            counter: AtomicUsize::new(0),
            connection_counts: Vec::new(),
        }
    }

    /// Returns the chain ID
    #[inline]
    pub fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    /// Adds a WebSocket endpoint to the pool
    pub async fn add_endpoint(
        &self,
        provider: Arc<dyn Provider + Send + Sync>,
        metrics: EndpointMetrics,
        url: String,
    ) {
        let entry = WsEndpointEntry::new(provider, Arc::new(metrics), url);
        let mut endpoints = self.endpoints.write().await;
        endpoints.push(Arc::new(entry));
        drop(endpoints);
    }

    /// Gets the number of WebSocket endpoints
    pub async fn endpoint_count(&self) -> usize {
        self.endpoints.read().await.len()
    }

    /// Selects next WebSocket endpoint using round-robin
    ///
    /// WebSocket connections are typically long-lived, so we use
    /// simple round-robin for initial assignment.
    pub async fn select_endpoint(&self) -> Option<WsLoadBalancerChoice> {
        let endpoints = self.endpoints.read().await;

        if endpoints.is_empty() {
            return None;
        }

        // Get next index atomically
        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % endpoints.len();
        let entry = endpoints.get(idx)?;

        // Increment connection count
        if idx < self.connection_counts.len() {
            self.connection_counts[idx].fetch_add(1, Ordering::Relaxed);
        }

        Some(WsLoadBalancerChoice::new(
            entry.provider(),
            entry.metrics(),
            idx,
            entry.url.clone(),
        ))
    }

    /// Gets metrics for all WebSocket endpoints
    pub async fn endpoints_metrics(&self) -> Vec<EndpointMetricsSnapshot> {
        let endpoints = self.endpoints.read().await;
        endpoints.iter().map(|e| e.metrics.snapshot()).collect()
    }
}

// ============================================================================
// Load Balancing Strategy

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadBalancingStrategy {
    /// Weighted by performance score (best for general queries)
    /// Uses response time and health metrics for intelligent selection
    WeightedLeastResponseTime,

    /// Sticky session based on endpoint index (best for transactions)
    /// Maintains affinity to ensure consistent mempool view
    StickySession { sticky_index: usize },

    /// Uniform distribution (best for cacheable reads)
    /// Simple round-robin across all healthy endpoints
    RoundRobin,
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

    /// Creates a round-robin strategy
    #[inline]
    pub fn round_robin() -> Self {
        Self::RoundRobin
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

/// Result of WebSocket endpoint selection
pub struct WsLoadBalancerChoice {
    provider: Arc<dyn Provider + Send + Sync>,
    metrics: Arc<EndpointMetrics>,
    index: usize,
    url: String,
}

impl Debug for WsLoadBalancerChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WsLoadBalancerChoice")
            .field("index", &self.index)
            .field("url", &self.url)
            .field("metrics", &self.metrics)
            .finish()
    }
}

impl WsLoadBalancerChoice {
    pub fn new(
        provider: Arc<dyn Provider + Send + Sync>,
        metrics: Arc<EndpointMetrics>,
        index: usize,
        url: String,
    ) -> Self {
        Self {
            provider,
            metrics,
            index,
            url,
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

    #[inline]
    pub fn url(&self) -> &str {
        &self.url
    }
}

// ============================================================================
