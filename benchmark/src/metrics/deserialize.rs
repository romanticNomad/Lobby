use hdrhistogram::Histogram;
use serde::Deserialize;
use std::time::{Duration, Instant};
use tracing::info;

// ===========================================================
// Constants & Configuration

/// Maximum expected latency for histogram bounds (60 seconds in microseconds).
const HISTOGRAM_MAX_US: u64 = 60_000_000;
/// Significant figures for HDRHistogram precision (3 sigfigs is standard for institutional reporting).
const HISTOGRAM_SIGFIGS: u8 = 3;

// ===========================================================
// Data Structures

/// Mirrors the `LatencyRecord` streamed by Lobby's `telemetry` exporter via UDS.
#[derive(Debug, Clone, Deserialize)]
pub struct LatencyRecord {
    #[allow(dead_code)]
    pub execution_id: String,
    pub relayhost_duration_us: u64,
    pub nonce_duration_us: u64,
    pub sign_duration_us: u64,
    pub broadcast_duration_us: u64,
    pub total_pipeline_us: u64,
}

/// Hdrhistogram payload holder for each pipeline stage
#[derive(Debug, Clone)]
pub struct StageMetrics {
    pub histogram: Histogram<u64>,
    pub dropped_count: u64,
}

impl StageMetrics {
    pub fn new() -> Self {
        Self {
            histogram: Histogram::<u64>::new_with_bounds(1, HISTOGRAM_MAX_US, HISTOGRAM_SIGFIGS)
                .expect("Failed to initialize Hdrhistogram"),
            dropped_count: 0,
        }
    }

    /// `record` wrapper of `hdrhistogram::Histogram::record` function,
    ///
    ///  accounts for `low` value of `1 us`.
    #[inline]
    fn record(&mut self, value_us: u64) {
        let value = value_us.max(1);
        if self.histogram.record(value).is_err() {
            self.dropped_count += 1;
        }
    }

    /// merger function in case multiple workers read UDS stream.
    #[allow(dead_code)]
    fn merge(&mut self, other: &StageMetrics) {
        self.histogram.add(&other.histogram).ok();
        self.dropped_count += other.dropped_count;
    }
}

/// The primary aggregator for pipeline telemetry.
///
/// Designed to be owned by a single UDS reader task (zero contention),
/// but supports merging if the harness scales to multiple collector workers.
#[derive(Debug, Clone)]
pub struct MetricsCollector {
    pub relayhost: StageMetrics,
    pub nonce: StageMetrics,
    pub sign: StageMetrics,
    pub broadcast: StageMetrics,
    pub total_pipeline: StageMetrics,

    // Filtering configuration
    test_start: Instant,
    warmup: Duration,
    steady_state: Duration,

    // Counters for observability
    pub total_received: u64,
    pub total_filtered_out: u64,
}

impl MetricsCollector {
    pub fn new(test_start: Instant, warmup: Duration, steady_state: Duration) -> Self {
        Self {
            relayhost: StageMetrics::new(),
            nonce: StageMetrics::new(),
            sign: StageMetrics::new(),
            broadcast: StageMetrics::new(),
            total_pipeline: StageMetrics::new(),
            test_start,
            warmup,
            steady_state,
            total_received: 0,
            total_filtered_out: 0,
        }
    }

    /// Records a telemetry sample, applying strict warmup/drain exclusion.
    pub fn submit_metrics(&mut self, latency_record: LatencyRecord, received_at: Instant) {
        self.total_received += 1;

        // Calculate elapsed time since test start
        let elapsed = received_at.duration_since(self.test_start);

        // Strict window filtering: [warmup, warmup + steady_state]
        // This excludes the first 5s (warmup/JIT) and anything after 55s (drain/cooldown)
        if elapsed < self.warmup || elapsed > self.warmup + self.steady_state {
            self.total_filtered_out += 1;
            return;
        }

        // Record into respective stage histograms
        self.relayhost.record(latency_record.relayhost_duration_us);
        self.nonce.record(latency_record.nonce_duration_us);
        self.sign.record(latency_record.sign_duration_us);
        self.broadcast.record(latency_record.broadcast_duration_us);
        self.total_pipeline.record(latency_record.total_pipeline_us);
    }

    /// Generates an institutional-grade summary report of the collected metrics.
    pub fn report(&self) {
        info!("=======================================================");
        info!("  LOBBY PIPELINE LATENCY REPORT (Steady-State Window)  ");
        info!("=======================================================");
        info!("Total Samples Received: {}", self.total_received);
        info!(
            "Samples Filtered (Warmup/Drain): {}",
            self.total_filtered_out
        );
        info!("-------------------------------------------------------");

        self.print_stage("RelayHost", &self.relayhost);
        self.print_stage("Nonce Reserve", &self.nonce);
        self.print_stage("Sign (ECDSA)", &self.sign);
        self.print_stage("Broadcast (RPC)", &self.broadcast);
        self.print_stage("Total Pipeline", &self.total_pipeline);

        info!("=======================================================");
    }

    /// helper function to display latency percentiles.
    fn print_stage(&self, name: &str, metrics: &StageMetrics) {
        if metrics.histogram.len() == 0 {
            info!("{:<15} | No samples recorded", name);
            return;
        }

        let p50 = metrics.histogram.value_at_quantile(0.50);
        let p95 = metrics.histogram.value_at_quantile(0.95);
        let p99 = metrics.histogram.value_at_quantile(0.99);
        let p999 = metrics.histogram.value_at_quantile(0.999);
        let max = metrics.histogram.max();

        info!(
            "{:<15} | p50: {:>6}µs | p95: {:>6}µs | p99: {:>6}µs | p99.9: {:>6}µs | max: {:>6}µs | dropped: {}",
            name, p50, p95, p99, p999, max, metrics.dropped_count
        );
    }
}

// ===========================================================
