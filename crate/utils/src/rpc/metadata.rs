//! Endpoint metadata and health tracking
//!
//! This module provides types and logic for tracking RPC endpoint performance,
//! health status, and implementing circuit breaker patterns.

use std::time::{Duration, Instant};

// ============================================================================
// Constants

/// Maximum size of the response time rolling window
const RESPONSE_TIME_WINDOW_SIZE: usize = 100;

/// Default response time when no metrics are available (in milliseconds)
const DEFAULT_RESPONSE_TIME_MS: f64 = 100.0;

/// Error rate threshold for degraded health (10%)
const DEGRADED_ERROR_THRESHOLD: f64 = 0.10;

/// Error rate threshold for unhealthy status (30%)
const UNHEALTHY_ERROR_THRESHOLD: f64 = 0.30;

/// Circuit breaker backoff durations (in seconds)
const CIRCUIT_BREAKER_BACKOFF: [u64; 3] = [10, 30, 60];

// ============================================================================
// Health Status

/// Health status of an RPC endpoint
///
/// The health status is determined by the error rate and circuit breaker state:
/// - `Healthy`: Error rate < 10%
/// - `Degraded`: Error rate 10-30%
/// - `Unhealthy`: Error rate > 30% or circuit breaker active
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointHealth {
    /// Endpoint is fully operational and accepting requests
    Healthy,
    /// Endpoint has elevated error rates but is still usable (receives reduced traffic)
    Degraded,
    /// Endpoint should not receive traffic (circuit breaker active or high error rate)
    Unhealthy,
}

impl Default for EndpointHealth {
    fn default() -> Self {
        Self::Healthy
    }
}

impl EndpointHealth {
    /// Returns the traffic multiplier for this health status
    ///
    /// - Healthy: 1.0 (full traffic)
    /// - Degraded: 0.5 (50% traffic reduction)
    /// - Unhealthy: 0.0 (no traffic)
    pub fn traffic_multiplier(&self) -> f64 {
        match self {
            Self::Healthy => 1.0,
            Self::Degraded => 0.5,
            Self::Unhealthy => 0.0,
        }
    }

    /// Returns true if the endpoint can receive traffic
    pub fn is_available(&self) -> bool {
        !matches!(self, Self::Unhealthy)
    }
}

// ============================================================================
// Endpoint Metrics

/// Performance metrics and health tracking for an RPC endpoint
///
/// Tracks response times, error rates, and implements circuit breaker logic
/// for adaptive load balancing.
#[derive(Debug, Clone)]
pub struct EndpointMetrics {
    /// Unique identifier for this endpoint (e.g., "eth_mainnet_alchemy_1")
    pub id: String,

    /// The RPC endpoint URL
    pub url: String,

    /// Current health status
    pub health: EndpointHealth,

    /// Rolling window of recent response times (in milliseconds)
    response_times_ms: Vec<u64>,

    /// Number of errors in the current measurement window
    error_count: u32,

    /// Total number of requests in the current measurement window
    request_count: u32,

    /// Timestamp of the last successful request
    last_success_at: Option<Instant>,

    /// Current block height (updated by health checker background task)
    block_height: Option<u64>,

    /// Circuit breaker: endpoint unavailable until this time
    circuit_breaker_until: Option<Instant>,

    /// Number of consecutive circuit breaker activations (for exponential backoff)
    circuit_breaker_attempts: u32,
}

impl EndpointMetrics {
    /// Creates new endpoint metrics with default values
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this endpoint
    /// * `url` - RPC endpoint URL
    pub fn new(id: String, url: String) -> Self {
        Self {
            id,
            url,
            health: EndpointHealth::Healthy,
            response_times_ms: Vec::with_capacity(RESPONSE_TIME_WINDOW_SIZE),
            error_count: 0,
            request_count: 0,
            last_success_at: None,
            block_height: None,
            circuit_breaker_until: None,
            circuit_breaker_attempts: 0,
        }
    }

    // ========================================================================
    // Metric Calculation

    /// Calculates the average response time from the rolling window
    ///
    /// Returns `DEFAULT_RESPONSE_TIME_MS` if no data is available.
    pub fn average_response_time_ms(&self) -> f64 {
        if self.response_times_ms.is_empty() {
            return DEFAULT_RESPONSE_TIME_MS;
        }

        let sum: u64 = self.response_times_ms.iter().sum();
        sum as f64 / self.response_times_ms.len() as f64
    }

    /// Calculates the current error rate (0.0 to 1.0)
    pub fn error_rate(&self) -> f64 {
        if self.request_count == 0 {
            return 0 as f64;
        }

        self.error_count as f64 / self.request_count as f64
    }

    /// Calculates the load balancing score for this endpoint
    ///
    /// Higher scores indicate better endpoints. The score combines:
    /// - Inverse of response time (faster = higher score)
    /// - Health multiplier (healthy endpoints preferred)
    ///
    /// Formula: `score = (1 / avg_response_time) * health_multiplier`
    ///
    /// Unhealthy endpoints always return 0.0.
    pub fn load_balancing_score(&self) -> f64 {
        let health_multiplier = self.health.traffic_multiplier();
        if health_multiplier == 0.0 {
            return 0.0;
        }

        let avg_responce_ms = self.average_response_time_ms().max(1.0);
        (1.0 / avg_responce_ms) * health_multiplier
    }

    // ========================================================================
    // Metric Recording

    /// Records a successful request with its duration
    ///
    /// Updates:
    /// - Response time rolling window
    /// - Request count
    /// - Last success timestamp
    /// - Clears circuit breaker if active
    /// - Recalculates health status
    pub fn record_success(&mut self, duration: Duration) {
        let duration_ms = duration.as_millis() as u64;

        // update time stamp and request count
        self.last_success_at = Some(Instant::now());
        self.request_count += 1;

        // rolling window of response time
        if self.response_times_ms.len() >= RESPONSE_TIME_WINDOW_SIZE {
            self.response_times_ms.remove(0);
        }
        self.response_times_ms.push(duration_ms);

        // reset circuite breaker on successful request
        if self.circuit_breaker_until.is_some() {
            self.circuit_breaker_until = None;
            self.circuit_breaker_attempts = 0;
        }

        // recalculate health based on updated metrics
        self.update_health_status();
    }

    /// Records a failed request
    ///
    /// Increments error count and recalculates health status.
    pub fn record_failure(&mut self) {
        self.error_count += 1;
        self.request_count += 1;
        self.update_health_status();
    }

    /// Updates block height (called by health checker)
    pub fn update_block_height(&mut self, height: u64) {
        self.block_height = Some(height);
    }

    // ========================================================================
    // Health Management

    /// Updates health status based on current metrics and circuit breaker state
    ///
    /// Health determination logic:
    /// 1. If circuit breaker is active and not expired -> Unhealthy
    /// 2. If error rate > 30% -> Unhealthy
    /// 3. If error rate > 10% -> Degraded
    /// 4. Otherwise -> Healthy
    pub fn update_health_status(&mut self) {
        // check circuit breaker
        if let Some(break_time) = self.circuit_breaker_until {
            if Instant::now() <= break_time {
                self.health = EndpointHealth::Unhealthy;
                return;
            }
        } else {
            // circuit breaker has expired
            self.circuit_breaker_until = None;
        }

        // determine health based on error rate
        let error_rate = self.error_rate();
        self.health = if error_rate > UNHEALTHY_ERROR_THRESHOLD {
            EndpointHealth::Unhealthy
        } else if error_rate > DEGRADED_ERROR_THRESHOLD {
            EndpointHealth::Degraded
        } else {
            EndpointHealth::Healthy
        };
    }

    /// Activates the circuit breaker with exponential backoff
    ///
    /// Backoff schedule:
    /// - 1st failure: 10 seconds
    /// - 2nd failure: 30 seconds
    /// - 3rd+ failures: 60 seconds
    pub fn activate_circuit_breaker(&mut self) {
        let backoff_index =
            (self.circuit_breaker_attempts as usize).min(CIRCUIT_BREAKER_BACKOFF.len() - 1);
        let backoff_duration = Duration::from_secs(CIRCUIT_BREAKER_BACKOFF[backoff_index]);

        self.circuit_breaker_until = Some(Instant::now() + backoff_duration);
        self.circuit_breaker_attempts += 1;
        self.health = EndpointHealth::Unhealthy;
    }

    /// Resets the metrics window (useful for periodic cleanup)
    pub fn reset_window(&mut self) {
        self.error_count = 0;
        self.request_count = 0;
        // Keep response times for continued performance tracking
    }

    /// Returns true if the endpoint is healthy enough to receive traffic
    #[inline]
    pub fn is_available(&self) -> bool {
        self.health.is_available()
    }

    // ========================================================================
    // Accessors

    /// Returns the endpoint ID
    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the endpoint URL
    #[inline]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the current health status
    #[inline]
    pub fn health(&self) -> EndpointHealth {
        self.health
    }

    /// Returns the current block height if available
    #[inline]
    pub fn block_height(&self) -> Option<u64> {
        self.block_height
    }

    /// Returns the timestamp of the last successful request
    #[inline]
    pub fn last_success(&self) -> Option<Instant> {
        self.last_success_at
    }
}

// ============================================================================
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_metrics() {
        let metrics =
            EndpointMetrics::new("test_1".to_string(), "http://localhost:8545".to_string());

        assert_eq!(metrics.id(), "test_1".to_string());
        assert_eq!(metrics.health(), EndpointHealth::Healthy);
        assert_eq!(metrics.error_rate(), 0.0);
        assert_eq!(metrics.average_response_time_ms(), DEFAULT_RESPONSE_TIME_MS);
    }

    #[test]
    fn test_success_recording() {
        let mut metrics =
            EndpointMetrics::new("test_1".to_string(), "http://localhost:8545".to_string());

        metrics.record_success(Duration::from_millis(50));

        assert_eq!(metrics.request_count, 1);
        assert_eq!(metrics.error_count, 0);
        assert_eq!(metrics.average_response_time_ms(), 50.0);
        assert!(metrics.last_success().is_some());
    }

    #[test]
    fn test_health_degradation() {
        let mut metrics =
            EndpointMetrics::new("test_1".to_string(), "http://localhost:8545".to_string());

        // Add some successes
        for _ in 0..8 {
            metrics.record_success(Duration::from_millis(100));
        }
        assert_eq!(metrics.health(), EndpointHealth::Healthy);

        // Add failures to reach degraded threshold (15% error rate)
        metrics.record_failure();
        metrics.record_failure();
        assert_eq!(metrics.health(), EndpointHealth::Degraded);

        // Add more failures to reach unhealthy threshold (40% error rate)
        metrics.record_failure();
        metrics.record_failure();
        assert_eq!(metrics.health(), EndpointHealth::Unhealthy);
    }

    #[test]
    fn test_circuit_breaker() {
        let mut metrics =
            EndpointMetrics::new("test_1".to_string(), "http://localhost:8545".to_string());

        metrics.activate_circuit_breaker();
        assert_eq!(metrics.health(), EndpointHealth::Unhealthy);
        assert!(metrics.circuit_breaker_until.is_some());

        // Success should clear circuit breaker
        metrics.record_success(Duration::from_millis(100));
        assert_eq!(metrics.circuit_breaker_attempts, 0);
        assert!(metrics.circuit_breaker_until.is_none());
    }

    #[test]
    fn test_load_balancing_score() {
        let mut metrics =
            EndpointMetrics::new("test_1".to_string(), "http://localhost:8545".to_string());

        // Fast endpoint should have high score
        metrics.record_success(Duration::from_millis(10));
        let fast_score = metrics.load_balancing_score();

        // Slow endpoint should have lower score
        metrics.response_times_ms.clear();
        metrics.record_success(Duration::from_millis(100));
        let slow_score = metrics.load_balancing_score();

        assert!(fast_score > slow_score);

        // Unhealthy endpoint should have zero score
        metrics.health = EndpointHealth::Unhealthy;
        assert_eq!(metrics.load_balancing_score(), 0.0);
    }
}

// ========================================================================
