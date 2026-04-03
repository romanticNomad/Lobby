pub mod rpc;

use alloy::providers::Provider;
use dashmap::DashMap;
use primitives::types::{ChainId, RpcProviderRegistry};
pub use rpc::*;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

// ============================================================
// RPC Connection Manager with Circuit Breaker

/// Managed RPC provider with built-in connection limiting
pub struct ManagedRpcProviderRegistry {
    providers: RpcProviderRegistry,
    semaphore: Arc<Semaphore>,
    failure_tracker: Arc<DashMap<String, FailureWindow>>,
}

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

impl ManagedRpcProviderRegistry {
    pub fn new(
        providers: RpcProviderRegistry,
        rpc_concurrecy: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let semaphore = Arc::new(Semaphore::new(rpc_concurrecy));
        let failure_tracker = Arc::new(DashMap::new());

        Ok(Self {
            providers,
            semaphore,
            failure_tracker,
        })
    }

    /// Acquire permit before making RPC call
    pub async fn aquire_permit(
        &self,
        rpc_timeout: Duration,
    ) -> Result<OwnedSemaphorePermit, RpcError> {
        let permit = tokio::time::timeout(rpc_timeout, Arc::clone(&self.semaphore).acquire_owned())
            .await
            .map_err(|_| RpcError::Timeout(rpc_timeout))?
            .expect("Rpc: Semaphore is closed");
        Ok(permit)
    }

    /// fetch provider from `ManagedRpcProviderRegistry` using the given `ChainId`
    pub fn provider(
        &self,
        chain_id: &ChainId,
    ) -> Result<Arc<dyn Provider + Send + Sync>, RpcError> {
        let provider = self
            .providers
            .get(chain_id)
            .ok_or(RpcError::ProviderNotFound(*chain_id))?
            .clone();

        Ok(provider)
    }

    /// Record failure for circuit breaker logic
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
}

// ============================================================

/// RpcProviderStack builder for orchestration of `Validator`` and `Broadcaster``
///
/// Separate registries prevent connection pool exhaustion.
#[derive(Clone)]
pub struct RpcProviderStack {
    /// Used ONLY by broadcaster/signer (write operations)
    pub broadcast_registry: RpcProviderRegistry,
    /// Used ONLY by validator (read operations - polling receipts)
    pub validator_registry: RpcProviderRegistry,
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
