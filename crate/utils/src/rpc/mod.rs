pub mod metadata;
pub mod pool;
pub mod rpc;

pub use metadata::*;
pub use pool::*;
pub use rpc::*;

use alloy::{primitives::Address, providers::Provider};
use dashmap::DashMap;
use primitives::types::ChainId;
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

// ============================================================
// RPC Connection Manager with Circuit Breaker

/// Managed RPC provider registry with endpoint pooling and load balancing
pub struct ManagedRpcProviderRegistry {
    /// Registry of endpoint pools (one per chain)
    endpoint_registry: RpcEndpointRegistry,
    /// Concurrency semaphore for limiting concurrent RPC calls
    semaphore: Arc<Semaphore>,
    /// Failure tracker for circuit breaker logic
    failure_tracker: Arc<DashMap<String, FailureWindow>>,
}

/// Internal tracking for an RPC call with endpoint selection
pub struct RpcCallContext {
    /// Selected endpoint ID for recording metrics
    pub endpoint_id: String,
    /// The provider to use for the call
    pub provider: Arc<dyn Provider + Send + Sync>,
    /// Chain ID for the call
    pub chain_id: ChainId,
}

impl ManagedRpcProviderRegistry {
    pub fn new(
        registry: RpcEndpointRegistry,
        rpc_concurrency: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let semaphore = Arc::new(Semaphore::new(rpc_concurrency));
        let failure_tracker = Arc::new(DashMap::new());

        Ok(Self {
            endpoint_registry: registry,
            semaphore,
            failure_tracker,
        })
    }

    /// Acquire permit and select an endpoint for an RPC call
    ///
    /// If `sender_address` is provided, uses sticky session routing for nonce management
    pub async fn acquire_permit_and_select(
        &self,
        chain_id: &ChainId,
        sender_address: Option<Address>,
        rpc_timeout: Duration,
    ) -> Result<(OwnedSemaphorePermit, RpcCallContext), RpcError> {
        // Acquire semaphore permit first
        let permit = tokio::time::timeout(rpc_timeout, Arc::clone(&self.semaphore).acquire_owned())
            .await
            .map_err(|_| RpcError::Timeout(rpc_timeout))?
            .expect("Rpc: Semaphore is closed");

        // Get the pool for this chain
        let pool = self
            .endpoint_registry
            .get(chain_id)
            .ok_or(RpcError::ProviderNotFound(*chain_id))?;

        // Select endpoint using weighted least response time or sticky session
        let (provider, endpoint_id) = select_endpoint_from_pool(&pool, sender_address).await?;

        let context = RpcCallContext {
            endpoint_id,
            provider,
            chain_id: *chain_id,
        };

        Ok((permit, context))
    }

    /// Get a provider for a chain (legacy method, uses no sticky session)
    pub fn provider(
        &self,
        chain_id: &ChainId,
    ) -> Result<Arc<dyn Provider + Send + Sync>, RpcError> {
        let pool = self
            .endpoint_registry
            .get(chain_id)
            .ok_or(RpcError::ProviderNotFound(*chain_id))?;

        // For backward compatibility, just return the first healthy endpoint
        // This should be used sparingly - prefer acquire_permit_and_select
        for (provider, metadata) in &pool.endpoints {
            if metadata.is_healthy() {
                return Ok(provider.clone());
            }
        }

        // No healthy endpoints, return first one anyway
        pool.endpoints
            .first()
            .map(|(p, _)| p.clone())
            .ok_or(RpcError::ProviderNotFound(*chain_id))
    }

    /// Record successful RPC call for metrics
    pub fn record_success(&self, chain_id: &ChainId, endpoint_id: &str, duration: Duration) {
        if let Some(pool) = self.endpoint_registry.get(chain_id) {
            // Find and update the endpoint metadata
            for (_, metadata) in &pool.endpoints {
                if metadata.id == endpoint_id {
                    // Clone to avoid holding the lock during the update
                    let mut meta_clone = metadata.clone();
                    meta_clone.record_success(duration.as_millis() as u64);
                    break;
                }
            }
        }
    }

    /// Record failure for circuit breaker logic (legacy method)
    pub fn record_failure(&self, method: &str) {
        let mut entry = self
            .failure_tracker
            .entry(method.to_string())
            .or_insert_with(|| FailureWindow {
                failures: Vec::new(),
                last_reset: Instant::now(),
            });

        // Reset window every 60 seconds
        if entry.last_reset.elapsed() > Duration::from_secs(60) {
            entry.failures.clear();
            entry.last_reset = Instant::now();
        }

        entry.failures.push(Instant::now());

        // Circuit breaker: if >10 failures in 60s, log warning
        if entry.failures.len() > 10 {
            tracing::warn!(
                method = %method,
                failures = entry.failures.len(),
                "High RPC failure rate detected - possible connection exhaustion"
            );
        }
    }

    /// Record failure for a specific endpoint
    pub fn record_endpoint_failure(&self, chain_id: &ChainId, endpoint_id: &str) {
        if let Some(pool) = self.endpoint_registry.get(chain_id) {
            for (_, metadata) in &pool.endpoints {
                if metadata.id == endpoint_id {
                    let mut meta_clone = metadata.clone();
                    meta_clone.record_failure();
                    break;
                }
            }
        }

        // Also record in the legacy failure tracker
        self.record_failure(endpoint_id);
    }
}

// ============================================================
// RPC Error Types

#[derive(Debug)]
pub enum RpcError {
    Timeout(Duration),
    ProviderNotFound(ChainId),
}

/// Tracks failure rate for circuit breaking
struct FailureWindow {
    failures: Vec<Instant>,
    last_reset: Instant,
}

// ============================================================
// Load Balancing Helpers

/// Select an endpoint from a pool using weighted least response time
/// or sticky session routing if sender_address is provided
async fn select_endpoint_from_pool(
    pool: &RpcEndpointPool,
    sender_address: Option<Address>,
) -> Result<(Arc<dyn Provider + Send + Sync>, String), RpcError> {
    if pool.endpoints.is_empty() {
        return Err(RpcError::ProviderNotFound(pool.chain_id));
    }

    // Filter healthy endpoints
    let healthy: Vec<(usize, &Arc<dyn Provider + Send + Sync>, &EndpointMetadata)> = pool
        .endpoints
        .iter()
        .enumerate()
        .filter(|(_, (_, metadata))| metadata.is_healthy())
        .map(|(idx, (provider, metadata))| (idx, provider, metadata))
        .collect();

    if healthy.is_empty() {
        // No healthy endpoints - check for circuit breaker recovery
        let available: Vec<(usize, &Arc<dyn Provider + Send + Sync>, &EndpointMetadata)> = pool
            .endpoints
            .iter()
            .enumerate()
            .filter(|(_, (_, metadata))| {
                metadata
                    .circuit_breaker_until
                    .map_or(true, |until| Instant::now() >= until)
            })
            .map(|(idx, (provider, metadata))| (idx, provider, metadata))
            .collect();

        if available.is_empty() {
            return Err(RpcError::ProviderNotFound(pool.chain_id));
        }

        // Fall back to round-robin among available endpoints
        let (_, provider, metadata) = &available[0];
        return Ok((Arc::clone(*provider), metadata.id.clone()));
    }

    // If sender_address provided, use consistent hashing for sticky session
    if let Some(address) = sender_address {
        let hash = calculate_address_hash(address);
        let idx = (hash as usize) % healthy.len();
        let (_, provider, metadata) = &healthy[idx];
        return Ok((Arc::clone(*provider), metadata.id.clone()));
    }

    // Otherwise, use weighted least response time
    let mut best = &healthy[0];
    let mut best_score = best.2.score();

    for item in &healthy[1..] {
        let score = item.2.score();
        if score > best_score {
            best_score = score;
            best = item;
        }
    }

    Ok((Arc::clone(best.1), best.2.id.clone()))
}

/// Calculate a hash from an address for consistent hashing (sticky sessions)
fn calculate_address_hash(address: Address) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    address.hash(&mut hasher);
    hasher.finish()
}

/// RpcProviderStack builder for orchestration of `Validator`` and `Broadcaster``
///
/// Separate registries prevent connection pool exhaustion.
#[derive(Clone)]
pub struct RpcProviderStack {
    /// Used ONLY by broadcaster/signer (write operations)
    pub broadcast_registry: RpcEndpointRegistry,
    /// Used ONLY by validator (read operations - polling receipts)
    pub validator_registry: RpcEndpointRegistry,
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
