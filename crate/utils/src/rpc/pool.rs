//! RPC endpoint pool management with async-optimized concurrency
//!
//! Uses lock-free metrics and fine-grained async locking for high throughput.

use crate::rpc::metadata::{EndpointMetrics, EndpointMetricsSnapshot};
use alloy::{primitives::Address, providers::Provider};
use dashmap::DashMap;
use primitives::types::ChainId;
use std::{
    fmt::Debug,
    hash::{DefaultHasher, Hash, Hasher},
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
    /// - Sticky session: O(1) with consistent hashing
    pub async fn select_endpoint(
        &self,
        strategy: &LoadBalancingStrategy,
    ) -> Option<(Arc<dyn Provider + Send + Sync>, Arc<EndpointMetrics>)> {
        match strategy {
            LoadBalancingStrategy::WeightedLeastResponseTime => {
                self.select_by_weighted_score().await
            }
            LoadBalancingStrategy::StickySession { sender_address } => {
                self.select_by_sticky_session(*sender_address).await
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
    async fn select_by_weighted_score(
        &self,
    ) -> Option<(Arc<dyn Provider + Send + Sync>, Arc<EndpointMetrics>)> {
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
            self.select_circuit_breaker_recovery().await;
        }

        // Weighted random selection (roulette wheel)
        // Use fastrand for async-friendly random numbers
        let threshold = fastrand::f64() * total_score;
        let mut cumulative = 0.0;

        for (idx, score) in &scored_endpoints {
            cumulative += score;
            if cumulative >= threshold {
                let endpoint = self.endpoints.read().await;
                let entry = endpoint.get(*idx)?;
                return Some((Arc::clone(&entry.provider), Arc::clone(&entry.metrics)));
            }
        }

        // fallback to last
        let last_index = scored_endpoints.last()?.0;
        let endpoint = self.endpoints.read().await;
        let entry = endpoint.get(last_index)?;
        Some((Arc::clone(&entry.provider), Arc::clone(&entry.metrics)))
    }

    /// Sticky session with consistent hashing
    ///
    /// Routes same sender to same endpoint for nonce management.
    /// Falls back to weighted selection if preferred endpoint unhealthy.
    ///
    /// Time: O(1) average case
    async fn select_by_sticky_session(
        &self,
        sender_address: Address,
    ) -> Option<(Arc<dyn Provider + Send + Sync>, Arc<EndpointMetrics>)> {
        let endpoints = self.endpoints.read().await;

        if endpoints.is_empty() {
            return None;
        }

        // Build list of healthy endpoints for consistent hashing
        let mut healthy: Vec<(usize, Arc<EndpointEntry>)> = Vec::with_capacity(endpoints.len());

        for (idx, entry) in endpoints.iter().enumerate() {
            if entry.metrics.is_available() {
                healthy.push((idx, Arc::clone(entry)));
            }
        }

        drop(endpoints);

        if healthy.is_empty() {
            return self.select_circuit_breaker_recovery().await;
        }

        // Consistent hashing
        let hash = hash_address(sender_address);
        let index = (hash as usize) % healthy.len();
        let (_, entry) = healthy.swap_remove(index);

        Some((Arc::clone(&entry.provider), Arc::clone(&entry.metrics)))
    }

    /// Simple round-robin for uniform distribution
    async fn select_round_robin(
        &self,
    ) -> Option<(Arc<dyn Provider + Send + Sync>, Arc<EndpointMetrics>)> {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static COUNTER: AtomicUsize = AtomicUsize::new(0);

        let endpoints = self.endpoints.read().await;
        if endpoints.is_empty() {
            return None;
        }

        // Get next index atomically
        let idx = COUNTER.fetch_add(1, Ordering::Relaxed) % endpoints.len();
        let entry = endpoints.get(idx)?;

        Some((Arc::clone(&entry.provider), Arc::clone(&entry.metrics)))
    }

    /// Circuit breaker recovery selection
    ///
    /// Attempts to find endpoints where circuit breaker has expired.
    /// Time: O(n)
    async fn select_circuit_breaker_recovery(
        &self,
    ) -> Option<(Arc<dyn Provider + Send + Sync>, Arc<EndpointMetrics>)> {
        let endpoints = self.endpoints.read().await;
        let now = Instant::now();

        // find expired circuit breaker
        for entry in endpoints.iter() {
            if let Some(expiry) = entry.metrics.circuit_breaker_until() {
                if now > expiry {
                    return Some((Arc::clone(&entry.provider), Arc::clone(&entry.metrics)));
                }
            }
        }

        // last resort: return the first endpoint irrespective of health
        endpoints
            .first()
            .map(|entry| (Arc::clone(&entry.provider), Arc::clone(&entry.metrics)))
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

    /// Consistent hashing by sender (best for transactions)
    StickySession { sender_address: Address },

    /// Uniform distribution (best for cacheable reads)
    RoundRobin,
}

impl LoadBalancingStrategy {
    #[inline]
    pub fn weighted() -> Self {
        Self::WeightedLeastResponseTime
    }

    #[inline]
    pub fn sticky(sender_address: Address) -> Self {
        Self::StickySession { sender_address }
    }

    #[inline]
    pub fn round_robin() -> Self {
        Self::RoundRobin
    }
}

// ============================================================================
// Helper Functions

#[inline]
fn hash_address(address: Address) -> u64 {
    let mut hasher = DefaultHasher::new();
    address.hash(&mut hasher);
    hasher.finish()
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
