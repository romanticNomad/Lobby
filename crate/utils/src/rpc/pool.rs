use alloy::primitives::Address;
use alloy::providers::Provider;
use dashmap::DashMap;
use primitives::types::ChainId;
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

use crate::rpc::{RpcEndpointPool, RpcEndpointRegistry, RpcError};

// ============================================================
// RPC Endpoint Pool with Load Balancing

/// Thread-safe RPC endpoint pool with weighted least response time load balancing
#[derive(Clone)]
pub struct ManagedEndpointPool {
    /// The underlying pool data
    pool: Arc<RwLock<RpcEndpointPool>>,
    /// Current index for round-robin fallback
    round_robin_index: Arc<std::sync::atomic::AtomicUsize>,
}

impl ManagedEndpointPool {
    /// Create a new managed pool from an RpcEndpointPool
    pub fn new(pool: RpcEndpointPool) -> Self {
        Self {
            pool: Arc::new(RwLock::new(pool)),
            round_robin_index: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Select an endpoint using weighted least response time algorithm
    ///
    /// If `sender_address` is provided, uses consistent hashing for sticky sessions
    /// (critical for EVM nonce management). Otherwise uses weighted scoring.
    pub async fn select_provider(
        &self,
        sender_address: Option<Address>,
    ) -> Result<(Arc<dyn Provider + Send + Sync>, String), RpcError> {
        let pool = self.pool.read().await;

        if pool.endpoints.is_empty() {
            return Err(RpcError::ProviderNotFound(pool.chain_id));
        }

        // Filter healthy endpoints
        let healthy_indices: Vec<usize> = pool
            .endpoints
            .iter()
            .enumerate()
            .filter(|(_, (_, metadata))| metadata.is_healthy())
            .map(|(idx, _)| idx)
            .collect();

        if healthy_indices.is_empty() {
            // No healthy endpoints - try to find one that's not circuit-broken
            let available_indices: Vec<usize> = pool
                .endpoints
                .iter()
                .enumerate()
                .filter(|(_, (_, metadata))| {
                    metadata
                        .circuit_breaker_until
                        .map_or(true, |until| Instant::now() >= until)
                })
                .map(|(idx, _)| idx)
                .collect();

            if available_indices.is_empty() {
                return Err(RpcError::ProviderNotFound(pool.chain_id));
            }

            // Fall back to round-robin among available endpoints
            let idx = self
                .round_robin_index
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                % available_indices.len();
            let endpoint_idx = available_indices[idx];
            let (provider, metadata) = &pool.endpoints[endpoint_idx];
            return Ok((provider.clone(), metadata.id.clone()));
        }

        // If sender_address provided, use consistent hashing for sticky session
        if let Some(address) = sender_address {
            let hash = calculate_address_hash(address);
            let idx = (hash as usize) % healthy_indices.len();
            let endpoint_idx = healthy_indices[idx];
            let (provider, metadata) = &pool.endpoints[endpoint_idx];
            return Ok((provider.clone(), metadata.id.clone()));
        }

        // Otherwise, use weighted least response time
        let mut best_idx = healthy_indices[0];
        let mut best_score = pool.endpoints[best_idx].1.score();

        for &idx in &healthy_indices[1..] {
            let score = pool.endpoints[idx].1.score();
            if score > best_score {
                best_score = score;
                best_idx = idx;
            }
        }

        let (provider, metadata) = &pool.endpoints[best_idx];
        Ok((provider.clone(), metadata.id.clone()))
    }

    /// Record a successful request for an endpoint
    pub async fn record_success(&self, endpoint_id: &str, duration: Duration) {
        let mut pool = self.pool.write().await;
        for (_, metadata) in &mut pool.endpoints {
            if metadata.id == endpoint_id {
                metadata.record_success(duration.as_millis() as u64);
                break;
            }
        }
    }

    /// Record a failed request for an endpoint
    pub async fn record_failure(&self, endpoint_id: &str) {
        let mut pool = self.pool.write().await;
        for (_, metadata) in &mut pool.endpoints {
            if metadata.id == endpoint_id {
                metadata.record_failure();
                break;
            }
        }
    }

    /// Activate circuit breaker for an endpoint
    pub async fn activate_circuit_breaker(&self, endpoint_id: &str, attempt: u32) {
        let mut pool = self.pool.write().await;
        for (_, metadata) in &mut pool.endpoints {
            if metadata.id == endpoint_id {
                metadata.activate_circuit_breaker(attempt);
                tracing::warn!(
                    endpoint_id = %endpoint_id,
                    chain_id = %pool.chain_id,
                    attempt = attempt,
                    "Circuit breaker activated for endpoint"
                );
                break;
            }
        }
    }

    /// Update block height for an endpoint (called by health checker)
    pub async fn update_block_height(&self, endpoint_id: &str, block_height: u64) {
        let mut pool = self.pool.write().await;
        for (_, metadata) in &mut pool.endpoints {
            if metadata.id == endpoint_id {
                metadata.block_height = Some(block_height);
                break;
            }
        }
    }

    /// Get current health status of all endpoints
    pub async fn health_summary(&self) -> Vec<(String, String, bool)> {
        let pool = self.pool.read().await;
        pool.endpoints
            .iter()
            .map(|(_, metadata)| {
                (
                    metadata.id.clone(),
                    metadata.url.clone(),
                    metadata.is_healthy(),
                )
            })
            .collect()
    }

    /// Get chain ID for this pool
    pub fn chain_id(&self) -> ChainId {
        // We need to read from the pool to get the chain_id
        // Since this is read-only and we don't want to make this async,
        // we'll store chain_id separately in the registry
        unimplemented!("Use registry to get chain_id")
    }
}

/// Calculate a hash from an address for consistent hashing
fn calculate_address_hash(address: Address) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    address.hash(&mut hasher);
    hasher.finish()
}

// ============================================================
// Registry of Managed Endpoint Pools

/// Registry mapping chain IDs to managed endpoint pools
#[derive(Clone)]
pub struct ManagedEndpointRegistry {
    pools: Arc<DashMap<ChainId, ManagedEndpointPool>>,
}

impl ManagedEndpointRegistry {
    /// Create a new registry from an RpcEndpointRegistry
    pub fn new(registry: RpcEndpointRegistry) -> Self {
        let pools = Arc::new(DashMap::new());

        for entry in registry.iter() {
            let chain_id = *entry.key();
            // Get the pool - since we can't clone RpcEndpointPool, we need to take ownership
            // But DashMap entry gives us a reference, so we need a different approach
            // For now, let's remove from registry and insert into pools
            if let Some((_, pool)) = registry.remove(&chain_id) {
                let managed_pool = ManagedEndpointPool::new(pool);
                pools.insert(chain_id, managed_pool);
            }
        }

        Self { pools }
    }

    /// Select a provider for a chain
    ///
    /// If `sender_address` is provided, uses sticky session routing
    pub async fn select_provider(
        &self,
        chain_id: &ChainId,
        sender_address: Option<Address>,
    ) -> Result<(Arc<dyn Provider + Send + Sync>, String), RpcError> {
        let pool = self
            .pools
            .get(chain_id)
            .ok_or(RpcError::ProviderNotFound(*chain_id))?;

        pool.select_provider(sender_address).await
    }

    /// Get the managed pool for a chain
    pub fn get_pool(&self, chain_id: &ChainId) -> Option<ManagedEndpointPool> {
        self.pools.get(chain_id).map(|p| p.value().clone())
    }

    /// Record success for a specific endpoint
    pub async fn record_success(&self, chain_id: &ChainId, endpoint_id: &str, duration: Duration) {
        if let Some(pool) = self.pools.get(chain_id) {
            pool.record_success(endpoint_id, duration).await;
        }
    }

    /// Record failure for a specific endpoint
    pub async fn record_failure(&self, chain_id: &ChainId, endpoint_id: &str) {
        if let Some(pool) = self.pools.get(chain_id) {
            pool.record_failure(endpoint_id).await;
        }
    }

    /// Activate circuit breaker for a specific endpoint
    pub async fn activate_circuit_breaker(
        &self,
        chain_id: &ChainId,
        endpoint_id: &str,
        attempt: u32,
    ) {
        if let Some(pool) = self.pools.get(chain_id) {
            pool.activate_circuit_breaker(endpoint_id, attempt).await;
        }
    }

    /// Get health summary for all pools
    pub async fn health_summary(&self) -> DashMap<ChainId, Vec<(String, String, bool)>> {
        let summary = DashMap::new();
        for entry in self.pools.iter() {
            let chain_id = *entry.key();
            let pool = entry.value();
            summary.insert(chain_id, pool.health_summary().await);
        }
        summary
    }
}

// ============================================================
// Health Checker Background Task

/// Spawn a background health checker task for the registry
pub fn spawn_health_checker(registry: ManagedEndpointRegistry, check_interval: Duration) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(check_interval);

        loop {
            interval.tick().await;

            for entry in registry.pools.iter() {
                let chain_id = *entry.key();
                let pool = entry.value();

                // Get all endpoints for this pool
                let endpoints = {
                    let pool_guard = pool.pool.read().await;
                    pool_guard
                        .endpoints
                        .iter()
                        .map(|(_, metadata)| (metadata.id.clone(), metadata.url.clone()))
                        .collect::<Vec<_>>()
                };

                // Check health of each endpoint
                for (endpoint_id, url) in endpoints {
                    match check_endpoint_health(&url).await {
                        Ok((block_height, latency_ms)) => {
                            pool.update_block_height(&endpoint_id, block_height).await;
                            pool.record_success(&endpoint_id, Duration::from_millis(latency_ms))
                                .await;
                            tracing::debug!(
                                endpoint_id = %endpoint_id,
                                chain_id = %chain_id,
                                block_height = block_height,
                                latency_ms = latency_ms,
                                "Health check passed"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                endpoint_id = %endpoint_id,
                                chain_id = %chain_id,
                                error = %e,
                                "Health check failed"
                            );
                            pool.record_failure(&endpoint_id).await;
                        }
                    }
                }
            }
        }
    });
}

/// Check endpoint health by querying eth_blockNumber
async fn check_endpoint_health(url: &str) -> Result<(u64, u64), String> {
    use alloy::providers::{Provider, ProviderBuilder};

    let start = Instant::now();
    let provider = ProviderBuilder::new()
        .connect_http(url.parse().map_err(|e| format!("Invalid URL: {}", e))?);

    let block_number = provider
        .get_block_number()
        .await
        .map_err(|e| format!("RPC error: {}", e))?;

    let latency_ms = start.elapsed().as_millis() as u64;

    Ok((block_number, latency_ms))
}

// ============================================================
