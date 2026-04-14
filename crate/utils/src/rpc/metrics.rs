//! This module provides:
//!
//! Endpoint metadata, health tracking and
//! High-performance implementation using atomics and lock-free structures.

use std::{
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::{Duration, Instant},
};

// ============================================================================
// Constants

/// Maximum size of the response time rolling window (power of 2 for fast modulo)
const RESPONSE_TIME_WINDOW_SIZE: usize = 128;
const RESPONSE_TIME_MASK: usize = RESPONSE_TIME_WINDOW_SIZE - 1;

/// Default response time when no metrics are available (in milliseconds)
const DEFAULT_RESPONSE_TIME_MS: f64 = 100.0;

/// Circuit breaker backoff durations (exponential)
const CIRCUIT_BREAKER_BACKOFF: [u64; 3] = [10, 30, 60];

/// Minimum requests before health calculation (prevents volatile early readings)
const MIN_REQUESTS_FOR_HEALTH: u32 = 10;

/// Health Thresholds
const DEGRADED_THRESHOLD: f64 = 0.15;
const UNHEALTHY_THRESHOLD: f64 = 0.40;

// ============================================================================
// Health Status

/// Health status of an RPC endpoint
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndpointHealth {
    /// Fully operational
    Healthy,
    /// Elevated errors, reduced traffic
    Degraded,
    /// Circuit breaker active or critical error rate
    Unhealthy,
}

impl Default for EndpointHealth {
    fn default() -> Self {
        Self::Healthy
    }
}

impl EndpointHealth {
    /// Traffic multiplier for load balancing weighting
    #[inline]
    pub fn traffic_multiplier(&self) -> f64 {
        match self {
            Self::Healthy => 1.0,
            Self::Degraded => 0.5,
            Self::Unhealthy => 0.0,
        }
    }

    #[inline]
    pub fn is_available(&self) -> bool {
        !matches!(self, Self::Unhealthy)
    }
}

// ============================================================================
// Atomic Metrics

/// Lock-free performance metrics using atomics for high-throughput updates
#[derive(Debug)]
pub struct EndpointMetrics {
    /// Unique identifier
    pub id: String,

    /// RPC endpoint URL
    pub url: String,

    /// Current health (stored as u8 for atomic operations)
    health: AtomicU32,

    /// Response time ring buffer (lock-free with atomic index)
    response_times_ms: [AtomicU64; RESPONSE_TIME_WINDOW_SIZE],

    /// Current write index in ring buffer
    response_time_index: AtomicU64,

    /// Error count (atomic)
    error_count: AtomicU64,

    /// Request count (atomic)
    request_count: AtomicU64,

    /// Last success timestamp (atomic, stores duration since epoch as millis)
    last_success_at: AtomicU64,

    /// Current block height
    // block_height: AtomicU64,

    /// Circuit breaker expiry (atomic, stores millis since epoch)
    circuit_breaker_until: AtomicU64,

    /// Circuit breaker attempt counter
    circuit_breaker_attempts: AtomicU32,

    /// Epoch start for metric windows (for periodic reset)
    window_epoch: AtomicU64,

    /// Tier-specific degraded error threshold (f64 stored as u64 bits)
    degraded_threshold: AtomicU64,

    /// Tier-specific unhealthy error threshold (f64 stored as u64 bits)
    unhealthy_threshold: AtomicU64,
}

impl Clone for EndpointMetrics {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            url: self.url.clone(),
            health: AtomicU32::new(self.health.load(Ordering::Relaxed)),
            response_times_ms: std::array::from_fn(|i| {
                AtomicU64::new(self.response_times_ms[i].load(Ordering::Relaxed))
            }),
            response_time_index: AtomicU64::new(self.response_time_index.load(Ordering::Relaxed)),
            error_count: AtomicU64::new(self.error_count.load(Ordering::Relaxed)),
            request_count: AtomicU64::new(self.request_count.load(Ordering::Relaxed)),
            last_success_at: AtomicU64::new(self.last_success_at.load(Ordering::Relaxed)),
            // block_height: AtomicU64::new(self.block_height.load(Ordering::Relaxed)),
            circuit_breaker_until: AtomicU64::new(
                self.circuit_breaker_until.load(Ordering::Relaxed),
            ),
            circuit_breaker_attempts: AtomicU32::new(
                self.circuit_breaker_attempts.load(Ordering::Relaxed),
            ),
            window_epoch: AtomicU64::new(self.window_epoch.load(Ordering::Relaxed)),
            degraded_threshold: AtomicU64::new(self.degraded_threshold.load(Ordering::Relaxed)),
            unhealthy_threshold: AtomicU64::new(self.unhealthy_threshold.load(Ordering::Relaxed)),
        }
    }
}

impl EndpointMetrics {

    /// Creates new endpoint metrics
    pub fn new(id: String, url: String) -> Self {
        let degraded = DEGRADED_THRESHOLD;
        let unhealthy = UNHEALTHY_THRESHOLD;

        Self {
            id,
            url,
            health: AtomicU32::new(EndpointHealth::Healthy as u32),
            response_times_ms: std::array::from_fn(|_| AtomicU64::new(0)),
            response_time_index: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            request_count: AtomicU64::new(0),
            last_success_at: AtomicU64::new(0),
            // block_height: AtomicU64::new(0),
            circuit_breaker_until: AtomicU64::new(0),
            circuit_breaker_attempts: AtomicU32::new(0),
            window_epoch: AtomicU64::new(Instant::now().elapsed().as_millis() as u64),
            degraded_threshold: AtomicU64::new(degraded.to_bits()),
            unhealthy_threshold: AtomicU64::new(unhealthy.to_bits()),
        }
    }

    // ========================================================================
    // Managing error_thresholds

    /// DEFAULT VALUES: degraded -> 0.15; unhealthy -> 0.40
    pub fn set_error_thresholds(&self, degraded: f64, unhealthy: f64) {
        self.degraded_threshold
            .store(degraded.to_bits(), Ordering::Release);
        self.unhealthy_threshold
            .store(unhealthy.to_bits(), Ordering::Release);
    }

    #[inline]
    /// Gets the current degraded threshold
    fn degraded_threshold(&self) -> f64 {
        f64::from_bits(self.degraded_threshold.load(Ordering::Acquire))
    }

    #[inline]
    /// Gets the current unhealthy threshold
    fn unhealthy_threshold(&self) -> f64 {
        f64::from_bits(self.unhealthy_threshold.load(Ordering::Acquire))
    }

    // ========================================================================
    // Metric Calculation (Read Operations)

    /// Calculates average response time from atomic ring buffer
    /// O(1) amortized, lock-free
    pub fn average_response_time_ms(&self) -> f64 {
        let index = self.response_time_index.load(Ordering::Acquire) as usize;

        if index == 0 {
            return DEFAULT_RESPONSE_TIME_MS;
        }

        let count = index.min(RESPONSE_TIME_WINDOW_SIZE);
        let mut sum: u64 = 0;
        let mut valid_count = 0;

        // Sample every 4th entry for O(1) approximation at high throughput
        let step = if count > 32 { 4 } else { 1 };

        for i in (0..count).step_by(step) {
            let idx = (index.wrapping_sub(1).wrapping_sub(i)) & RESPONSE_TIME_MASK;
            let val = self.response_times_ms[idx].load(Ordering::Relaxed);

            if val > 0 {
                sum += val;
                valid_count += 1;
            }
        }

        if valid_count == 0 {
            return DEFAULT_RESPONSE_TIME_MS;
        }

        let avg = sum as f64 / valid_count as f64;
        // Apply step correction for sampling
        avg * step as f64
    }

    /// Current error rate (0.0 to 1.0), lock-free
    pub fn error_rate(&self) -> f64 {
        let requests = self.request_count.load(Ordering::Relaxed);
        if requests < MIN_REQUESTS_FOR_HEALTH as u64 {
            return 0.0;
        }
        let errors = self.error_count.load(Ordering::Relaxed);
        errors as f64 / requests as f64
    }

    /// Load balancing score: higher = better endpoint
    /// Formula: (1 / avg_response_time) * health_multiplier
    pub fn load_balancing_score(&self) -> f64 {
        let health = self.health();
        let multiplier = health.traffic_multiplier();

        if multiplier == 0.0 {
            return 0.0;
        }

        // Use cached average with minimum 1ms to prevent division issues
        let avg_ms = self.average_response_time_ms().max(1.0);
        (1000.0 / avg_ms) * multiplier
    }

    /// Current health status (atomic read)
    pub fn health(&self) -> EndpointHealth {
        match self.health.load(Ordering::Acquire) {
            0 => EndpointHealth::Healthy,
            1 => EndpointHealth::Degraded,
            _ => EndpointHealth::Unhealthy,
        }
    }

    // ========================================================================
    // Metric Recording (Write Operations) - All Lock-Free

    /// Records successful request - O(1), wait-free
    pub fn record_success(&self, duration: Duration) {
        let duration_ms = duration.as_millis() as u64;
        let now = Instant::now();
        let now_millis = now.elapsed().as_millis() as u64;

        // Update ring buffer (lock-free)
        let index = self.response_time_index.fetch_add(1, Ordering::AcqRel);
        let slot = (index as usize) & RESPONSE_TIME_MASK;
        self.response_times_ms[slot].store(duration_ms, Ordering::Release);

        // Update counters
        self.request_count.fetch_add(1, Ordering::Relaxed);
        self.last_success_at.store(now_millis, Ordering::Release);

        // Reset circuit breaker on success
        let cb_until = self.circuit_breaker_until.load(Ordering::Acquire);
        if cb_until > 0 && now_millis >= cb_until {
            self.circuit_breaker_until.store(0, Ordering::Release);
            self.circuit_breaker_attempts.store(0, Ordering::Release);
        }

        // Recalculate health
        self.update_health_status();
    }

    /// Records failed request - O(1), wait-free
    pub fn record_failure(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
        self.request_count.fetch_add(1, Ordering::Relaxed);
        self.update_health_status();
    }

    /// Updates block height (atomic)
    // pub fn update_block_height(&self, height: u64) {
    //     self.block_height.store(height, Ordering::Release);
    // }

    // ========================================================================
    // Health Management

    /// Updates health based on current metrics - O(1)
    fn update_health_status(&self) {
        // Check circuit breaker first
        let cb_until = self.circuit_breaker_until.load(Ordering::Acquire);
        if cb_until > 0 {
            let now = Instant::now().elapsed().as_millis() as u64;
            if now < cb_until {
                self.health.store(2, Ordering::Release); // Unhealthy
                return;
            }
            // Circuit breaker expired, clear it
            self.circuit_breaker_until.store(0, Ordering::Release);
        }

        // Calculate health based on error rate with thresholds
        let error_rate = self.error_rate();
        let degraded_threshold = self.degraded_threshold();
        let unhealthy_threshold = self.unhealthy_threshold();

        let new_health = if error_rate > unhealthy_threshold {
            2 // Unhealthy
        } else if error_rate > degraded_threshold {
            1 // Degraded
        } else {
            0 // Healthy
        };

        self.health.store(new_health, Ordering::Release);
    }

    /// Activates circuit breaker with exponential backoff
    pub fn activate_circuit_breaker(&self) {
        let attempts = self.circuit_breaker_attempts.load(Ordering::Acquire) as usize;
        let backoff_idx = attempts.min(CIRCUIT_BREAKER_BACKOFF.len() - 1);
        let backoff_secs = CIRCUIT_BREAKER_BACKOFF[backoff_idx];

        let now = Instant::now();
        let expiry = now + Duration::from_secs(backoff_secs);
        let expiry_ms = expiry.elapsed().as_millis() as u64;

        self.circuit_breaker_until
            .store(expiry_ms, Ordering::Release);
        self.circuit_breaker_attempts
            .fetch_add(1, Ordering::Relaxed);
        self.health.store(2, Ordering::Release);
    }

    /// Resets metric window (for periodic cleanup)
    pub fn reset_window(&self) {
        let now = Instant::now().elapsed().as_millis() as u64;
        self.window_epoch.store(now, Ordering::Release);
        self.error_count.store(0, Ordering::Relaxed);
        self.request_count.store(0, Ordering::Relaxed);
        // Response times preserved for continuity
    }

    /// Checks if endpoint is available (lock-free)
    #[inline]
    pub fn is_available(&self) -> bool {
        self.health().is_available()
    }

    // ========================================================================
    // Accessors

    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[inline]
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn last_success(&self) -> Option<Instant> {
        let millis = self.last_success_at.load(Ordering::Acquire);
        if millis == 0 {
            None
        } else {
            // Reconstruct instant from stored duration
            Some(
                Instant::now()
                    - Duration::from_millis(Instant::now().elapsed().as_millis() as u64 - millis),
            )
        }
    }

    // pub fn block_height(&self) -> Option<u64> {
    //     let height = self.block_height.load(Ordering::Acquire);
    //     if height == 0 { None } else { Some(height) }
    // }

    pub fn circuit_breaker_until(&self) -> Option<Instant> {
        let millis = self.circuit_breaker_until.load(Ordering::Acquire);
        if millis == 0 {
            None
        } else {
            let now = Instant::now();
            let now_millis = now.elapsed().as_millis() as u64;
            if now_millis < millis {
                Some(now + Duration::from_millis(millis - now_millis))
            } else {
                None
            }
        }
    }

    /// Snapshot for monitoring (atomic read of all fields)
    pub fn snapshot(&self) -> EndpointMetricsSnapshot {
        EndpointMetricsSnapshot {
            id: self.id.clone(),
            url: self.url.clone(),
            health: self.health(),
            avg_response_time_ms: self.average_response_time_ms(),
            error_rate: self.error_rate(),
            score: self.load_balancing_score(),
            // block_height: self.block_height(),
            request_count: self.request_count.load(Ordering::Relaxed),
            error_count: self.error_count.load(Ordering::Relaxed),
        }
    }
}

/// Immutable snapshot for external consumption
#[derive(Debug, Clone)]
pub struct EndpointMetricsSnapshot {
    pub id: String,
    pub url: String,
    pub health: EndpointHealth,
    pub avg_response_time_ms: f64,
    pub error_rate: f64,
    pub score: f64,
    // pub block_height: Option<u64>,
    pub request_count: u64,
    pub error_count: u64,
}

// ============================================================================
