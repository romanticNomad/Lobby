use serde::Deserialize;

// ===========================================================
// Constants & Configuration
/// Maximum expected latency for histogram bounds (60 seconds in microseconds).
const HISTOGRAM_MAX_US: u64 = 60_000_000;
/// Significant figures for HDRHistogram precision (3 sigfigs is standard for institutional reporting).
const HISTOGRAM_SIGFIGS: u8 = 3;

// ===========================================================
// Data Structures

/// Mirrors the `LatencyRecord` streamed by Lobby's telemetry exporter via UDS.
#[derive(Debug, Clone, Deserialize)]
pub struct LatencyHistogram {
    pub execution_id: String,
    pub relayhost_duration_us: u64,
    pub nonce_duration_us: u64,
    pub sign_duration_us: u64,
    pub broadcast_duration_us: u64,
    pub total_pipeline_us: u64,
}


