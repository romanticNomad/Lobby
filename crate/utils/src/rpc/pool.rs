//! RPC endpoint pool management with async-optimized concurrency
//!
//! Uses lock-free metrics and fine-grained async locking for high throughput.

use crate::rpc::metadata::{EndpointMetrics, EndpointMetricsSnapshot};
use alloy::providers::Provider;
use dashmap::DashMap;
use primitives::types::ChainId;
use std::{
    fmt::Debug,
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
use tokio::sync::RwLock;

// ============================================================================
// Type Aliases

/// Registry mapping chain IDs to their endpoint pools
/// Uses DashMap for concurrent read/write without blocking
pub type EndpointRegistry = Arc<DashMap<ChainId, Arc<EndpointPool>>>;

// TTL limit for the cached healthy endpoint indeces.
pub const CACHE_TTL: u64 = 5;

// ============================================================================
// Endpoint Pool

/// High-performance endpoint pool with lock-free metric access
///
/// Architecture:
/// - DashMap for registry-level concurrency (lock-free reads)
/// - Vec<Arc<EndpointMetrics>> for lock-free metric reads
/// - RwLock only for structural changes (add/remove endpoints)
#[derive(Debug)]
pub struct EndpointPool {
    /// ChainId for the blockchain supported by this pool
    chain_id: ChainId,

    /// Endpoints with shared metrics (Arc for lock-free access)
    ///
    /// Metrics are stored separately to allow concurrent updates without
    /// locking the provider or other endpoint data.
    endpoints: RwLock<Vec<Arc<EndpointEntry>>>,

    /// Cached healthy endpoint indeces, periodially updated
    healthy_cache: RwLock<Vec<usize>>,

    /// last cache update timestamp
    cache_timestamp: std::sync::atomic::AtomicU64,
}

// ============================================================================
// entry point for rpc provider and its metrics

/// Single endpoint entry with shared ownership
pub struct EndpointEntry {
    /// The RPC provider implementation
    provider: Arc<dyn Provider + Send + Sync>,

    /// Shared metrics (lock-free atomic operations)
    metrics: Arc<EndpointMetrics>,
}

// custom Debug implimentation
impl Debug for EndpointEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndpointEntry")
            .field("metric", &self.metrics)
            .finish()
    }
}

impl EndpointEntry {
    // create a new instance of EndpointEntry
    pub fn new(provider: Arc<dyn Provider + Send + Sync>, metrics: Arc<EndpointMetrics>) -> Self {
        Self { provider, metrics }
    }
}

// ============================================================================
// method implimentations for EndpointPool

impl EndpointPool {
    /// Creates new endpoint pool for a specific chain
    pub fn new(chain_id: ChainId) -> Self {
        Self {
            chain_id,
            endpoints: RwLock::new(Vec::new()),
            healthy_cache: RwLock::new(Vec::new()),
            cache_timestamp: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Adds endpoint to pool (acquires write lock briefly)
    async fn add_endpoint(
        &self,
        provider: Arc<dyn Provider + Send + Sync>,
        metrics: EndpointMetrics,
    ) {
        let entry = EndpointEntry::new(provider, Arc::new(metrics));
        let mut endpoints = self.endpoints.write().await;
        endpoints.push(Arc::new(entry));
        drop(endpoints);

        // update cache
        self.update_healthy_cache().await;
    }

    #[inline]
    pub fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    // number of endpoints present in the pool
    pub async fn endpoint_cout(&self) -> usize {
        self.endpoints.read().await.len()
    }

    /// Count healthy endpoints using cached data when fresh
    pub async fn healthy_endpoint_count(&self) -> usize {
        // Check cache freshness (5 second TTL)
        let now = Instant::now().elapsed().as_secs();
        let cache_lifetime = self.cache_timestamp.load(Ordering::Acquire);

        // TTL set to 5 seconds
        if now.saturating_sub(cache_lifetime) < CACHE_TTL {
            return self.healthy_cache.read().await.len();
        }

        // if TTL expired then recalculate
        self.update_healthy_cache().await;
        self.healthy_cache.read().await.len()
    }

    /// Updates healthy endpoint cache (internal)
    async fn update_healthy_cache(&self) {
        let endpoints = self.endpoints.read().await;
        let mut healthy_endpoint_indeces = Vec::with_capacity(endpoints.len());

        for (idx, entry) in endpoints.iter().enumerate() {
            // lock-free health check
            if entry.metrics.is_available() {
                healthy_endpoint_indeces.push(idx);
            }
        }

        let now = Instant::now().elapsed().as_secs();
        let mut cache = self.healthy_cache.write().await;
        *cache = healthy_endpoint_indeces;
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
            LoadBalancingStrategy::RoundRobin => self.select_round_robin().await,
        }
    }

    /// Weighted least response time selection
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

        // collect scores (lock-free)
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
        // Use fastrand for async-friendly random numbers
        let threshold = fastrand::f64() * total_score;
        let mut cumulative = 0.0;
        let mut selected_idx = 0;

        for (idx, score) in &scored_endpoints {
            cumulative += score;
            if cumulative >= threshold {
                selected_idx = *idx;
                break;
            }
        }

        // If no selection made, use the last one
        if cumulative < threshold {
            selected_idx = scored_endpoints.last()?.0;
        }

        let endpoint = self.endpoints.read().await;
        let entry = endpoint.get(selected_idx)?;
        Some(LoadBalancerChoice::new(
            Arc::clone(&entry.provider),
            Arc::clone(&entry.metrics),
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
                    Arc::clone(&entry.provider),
                    Arc::clone(&entry.metrics),
                    sticky_index,
                ));
            }
        }

        drop(endpoints);

        // Fallback to weighted selection if endpoint is unhealthy or invalid
        self.select_by_weighted_score().await
    }

    /// Simple round-robin for uniform distribution
    async fn select_round_robin(&self) -> Option<LoadBalancerChoice> {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static COUNTER: AtomicUsize = AtomicUsize::new(0);

        let endpoints = self.endpoints.read().await;
        if endpoints.is_empty() {
            return None;
        }

        // Get next index atomically
        let idx = COUNTER.fetch_add(1, Ordering::Relaxed) % endpoints.len();
        let entry = endpoints.get(idx)?;

        Some(LoadBalancerChoice::new(
            Arc::clone(&entry.provider),
            Arc::clone(&entry.metrics),
            idx,
        ))
    }

    /// Circuit breaker recovery selection
    ///
    /// Attempts to find endpoints where circuit breaker has expired.
    /// Time: O(n)
    async fn select_circuit_breaker_recovery(&self) -> Option<LoadBalancerChoice> {
        let endpoints = self.endpoints.read().await;
        let now = Instant::now();

        // find expired circuit breaker
        for (idx, entry) in endpoints.iter().enumerate() {
            if let Some(expiry) = entry.metrics.circuit_breaker_until() {
                if now > expiry {
                    return Some(LoadBalancerChoice::new(
                        Arc::clone(&entry.provider),
                        Arc::clone(&entry.metrics),
                        idx,
                    ));
                }
            }
        }

        // last resort: return the first endpoint irrespective of health
        endpoints.first().map(|entry| {
            LoadBalancerChoice::new(Arc::clone(&entry.provider), Arc::clone(&entry.metrics), 0)
        })
    }

    // ========================================================================
    // Metric Updates (Lock-Free via Arc)

    /// Updates metrics for a specific endpoint using closure
    ///
    /// Lock-free: Metrics are atomically updated via Arc<EndpointMetrics>
    pub async fn update_endpoint_metrics<F>(&self, endpoint_id: String, update_fn: F)
    where
        F: FnOnce(&EndpointMetrics),
    {
        let endpoints = self.endpoints.read().await;
        for entry in endpoints.iter() {
            if endpoint_id == entry.metrics.id {
                update_fn(&entry.metrics);
                return;
            }
        }
    }

    /// Batch update multiple endpoints (more efficient than individual updates)
    pub async fn batch_update_metrics<F>(&self, updates: Vec<(String, F)>)
    where
        F: Fn(&EndpointMetrics),
    {
        let endpoints = self.endpoints.read().await;

        for (id, update_fn) in updates {
            if let Some(entry) = endpoints.iter().find(|e| e.metrics.id() == id) {
                update_fn(&entry.metrics);
            }
        }
    }

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
            .map(|e| Arc::clone(&e.metrics))
    }
}

// ============================================================================
// Load Balancing Strategy

#[derive(Debug, Clone)]
pub enum LoadBalancingStrategy {
    /// Weighted by performance score (best for general queries)
    WeightedLeastResponseTime,

    /// Sticky session based on endpoint index (best for transactions)
    /// Use this to maintain session affinity with an endpoint selected
    /// by weighted score or round-robin strategies.
    StickySession { sticky_index: usize },

    /// Uniform distribution (best for cacheable reads)
    RoundRobin,
}

impl LoadBalancingStrategy {
    #[inline]
    pub fn weighted() -> Self {
        Self::WeightedLeastResponseTime
    }

    #[inline]
    pub fn sticky(sticky_index: usize) -> Self {
        Self::StickySession { sticky_index }
    }

    #[inline]
    pub fn round_robin() -> Self {
        Self::RoundRobin
    }
}

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

// ============================================================================
// RPC Provider Stack

/// Dual-registry stack for workload isolation
#[derive(Clone)]
pub struct RpcProviderStack {
    pub broadcast_registry: EndpointRegistry,
    pub validator_registry: EndpointRegistry,
}

impl RpcProviderStack {
    /// Creates new stack with separate registries
    pub fn new() -> Self {
        Self {
            broadcast_registry: Arc::new(DashMap::new()),
            validator_registry: Arc::new(DashMap::new()),
        }
    }

    /// Gets pool for broadcasting (write operations)
    pub fn get_broadcast_pool(&self, chain_id: ChainId) -> Option<Arc<EndpointPool>> {
        self.broadcast_registry
            .get(&chain_id)
            .map(|entry| Arc::clone(entry.value()))
    }

    /// Gets pool for validation (read operations)
    pub fn get_validator_pool(&self, chain_id: ChainId) -> Option<Arc<EndpointPool>> {
        self.validator_registry
            .get(&chain_id)
            .map(|entry| Arc::clone(entry.value()))
    }
}

// ============================================================================
// Environment Loading

// use std::env;

// /// Loads RPC endpoints from environment
// ///
// /// Format: RPC_ENDPOINTS_<CHAIN_ID>=url1,url2,url3
// pub async fn load_endpoint_from_env() -> EndpointRegistry {

// }

// ============================================================================
// Tests

/// Placeholder Provider from testing.
struct PlaceholderProvider;
impl Provider for PlaceholderProvider {
    fn root(&self) -> &alloy::providers::RootProvider<alloy::network::Ethereum> {
        unimplemented!("test use only")
    }
}

// ============================================================================
// unit test module

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::task::JoinSet;

    #[tokio::test]
    async fn test_pool_concurrent_selection() {
        let pool = Arc::new(EndpointPool::new(ChainId::try_from(1).unwrap()));

        // Add mock endpoints
        for i in 0..5 {
            let metrics = EndpointMetrics::new(
                format!("test_{}", i),
                format!("http://localhost:{}", 8545 + i),
            );

            // Record some response times to differentiate scores
            for _ in 0..10 {
                metrics.record_success(std::time::Duration::from_millis(50 + i as u64 * 10));
            }

            pool.add_endpoint(Arc::new(PlaceholderProvider), metrics)
                .await;
        }

        // Concurrent selections
        let mut handles = JoinSet::new();

        for _ in 0..100 {
            let pool_clone = Arc::clone(&pool);
            handles.spawn(async move {
                let strategy = LoadBalancingStrategy::weighted();
                pool_clone.select_endpoint(&strategy).await
            });
        }

        // await all submissions
        let results = handles.join_all().await;
        let success_count = results.iter().filter(|r| r.to_owned().is_some()).count();

        assert!(success_count >= 95, "Most selections should succeed");
    }
}

// ============================================================================
