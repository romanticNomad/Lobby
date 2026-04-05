use alloy::providers::Provider;
use dashmap::DashMap;
use primitives::types::ChainId;
use std::sync::Arc;

// ============================================================
// RPC Endpoint Pool Types for Load Balancing

/// Health status of an RPC endpoint
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointHealth {
    /// Endpoint is healthy and accepting requests
    Healthy,
    /// Endpoint is experiencing issues but still usable
    Degraded,
    /// Endpoint is unhealthy and should not receive requests
    Unhealthy,
}

impl Default for EndpointHealth {
    fn default() -> Self {
        EndpointHealth::Healthy
    }
}

/// Metadata tracked for each RPC endpoint in the pool
#[derive(Debug, Clone)]
pub struct EndpointMetadata {
    /// Unique identifier for this endpoint (e.g., "1_A" for chain 1, endpoint A)
    pub id: String,
    /// The RPC endpoint URL
    pub url: String,
    /// Current health status
    pub health: EndpointHealth,
    /// Rolling window of response times in milliseconds (last 100 requests)
    pub response_times_ms: Vec<u64>,
    /// Error count in current window
    pub error_count: u32,
    /// Total requests in current window
    pub request_count: u32,
    /// Last time the endpoint was successfully used
    pub last_success: Option<std::time::Instant>,
    /// Current block height (updated by health checker)
    pub block_height: Option<u64>,
    /// Circuit breaker: time when endpoint can be retried after failure
    pub circuit_breaker_until: Option<std::time::Instant>,
}

impl EndpointMetadata {
    /// Create new endpoint metadata
    pub fn new(id: String, url: String) -> Self {
        Self {
            id,
            url,
            health: EndpointHealth::Healthy,
            response_times_ms: Vec::with_capacity(100),
            error_count: 0,
            request_count: 0,
            last_success: None,
            block_height: None,
            circuit_breaker_until: None,
        }
    }

    /// Calculate average response time from rolling window
    pub fn avg_response_time_ms(&self) -> f64 {
        if self.response_times_ms.is_empty() {
            return 100.0; // Default to 100ms if no data
        }
        let sum: u64 = self.response_times_ms.iter().sum();
        sum as f64 / self.response_times_ms.len() as f64
    }

    /// Calculate error rate (0.0 to 1.0)
    pub fn error_rate(&self) -> f64 {
        if self.request_count == 0 {
            return 0.0;
        }
        self.error_count as f64 / self.request_count as f64
    }

    /// Record a successful request with its duration
    pub fn record_success(&mut self, duration_ms: u64) {
        self.last_success = Some(std::time::Instant::now());
        self.request_count += 1;

        // Maintain rolling window of last 100 response times
        if self.response_times_ms.len() >= 100 {
            self.response_times_ms.remove(0);
        }
        self.response_times_ms.push(duration_ms);

        // Reset circuit breaker if it was active
        self.circuit_breaker_until = None;

        // Update health based on performance
        self.update_health();
    }

    /// Record a failed request
    pub fn record_failure(&mut self) {
        self.error_count += 1;
        self.request_count += 1;
        self.update_health();
    }

    /// Update health status based on error rate and circuit breaker
    fn update_health(&mut self) {
        // Check if circuit breaker is active
        if let Some(until) = self.circuit_breaker_until {
            if std::time::Instant::now() < until {
                self.health = EndpointHealth::Unhealthy;
                return;
            }
        }

        let error_rate = self.error_rate();
        self.health = if error_rate > 0.30 {
            EndpointHealth::Unhealthy
        } else if error_rate > 0.10 {
            EndpointHealth::Degraded
        } else {
            EndpointHealth::Healthy
        };
    }

    /// Activate circuit breaker with exponential backoff
    pub fn activate_circuit_breaker(&mut self, attempt: u32) {
        let backoff_seconds = match attempt {
            0 => 10,
            1 => 30,
            _ => 60,
        };
        self.circuit_breaker_until =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(backoff_seconds));
        self.health = EndpointHealth::Unhealthy;
    }

    /// Calculate weighted score for load balancing (higher = better)
    /// Formula: (1 / avg_response_time) * health_multiplier
    pub fn score(&self) -> f64 {
        if self.health == EndpointHealth::Unhealthy {
            return 0.0;
        }

        let health_multiplier = match self.health {
            EndpointHealth::Healthy => 1.0,
            EndpointHealth::Degraded => 0.5,
            EndpointHealth::Unhealthy => 0.0,
        };

        let avg_response = self.avg_response_time_ms().max(1.0); // Avoid division by zero
        (1.0 / avg_response) * health_multiplier
    }

    /// Check if endpoint is healthy enough to receive traffic
    pub fn is_healthy(&self) -> bool {
        self.health != EndpointHealth::Unhealthy
    }
}

// ============================================================
// RPC Endpoint Pool

/// Pool of RPC endpoints for a single chain with load balancing
pub struct RpcEndpointPool {
    /// Chain ID this pool serves
    pub chain_id: ChainId,
    /// Available endpoints with their metadata
    pub endpoints: Vec<(Arc<dyn Provider + Send + Sync>, EndpointMetadata)>,
}

/// Registry mapping chain IDs to endpoint pools
pub type RpcEndpointRegistry = Arc<DashMap<ChainId, RpcEndpointPool>>;

// ============================================================
