use crate::loadgen::keys::ApiStack;
use bytes::Bytes;
use rand::seq::SliceRandom;
use reqwest::Client;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::mpsc;

// =============================================================================
// Constants
// =============================================================================

/// A dummy EVM address used for receiving transactions.
///
/// Used for lobby benchmarking only; no real funds are ever sent to this account.
pub const RECIPIENT_ADDRESS: &str = "0x430b3af2c718497fe0add817c8ead48c8bd2ef61";

// =============================================================================
// Errors
// =============================================================================

/// Errors involved in trigger action to Lobby server
#[derive(Debug, Error)]
pub enum TriggerError {
    #[error("Http request error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Unexpected status code received: {0}")]
    UnexpectedStatus(reqwest::StatusCode),

    #[error("Missing execution_id in response")]
    MissingExecutionId,

    #[error("Json deserialization failed: {0}")]
    SerDe(#[from] serde_json::Error),
}

// =============================================================================
// Payload Structures
// =============================================================================

/// Thread-safe, pre-serialized transaction payload with associated API key.
///
/// Designed for O(1) random selection and zero-copy cloning via `Bytes`.
#[derive(Debug, Clone)]
pub struct PayloadEntry {
    pub index: usize,
    pub api_key: String,
    pub payload: Bytes,
}

/// Pre-serialized payloads from each api_key in the `ApiStack`.
///
/// Features:
/// * Cheap to clone across Tokio workers (`Arc`-backed).
#[derive(Debug, Clone)]
pub struct Payloads {
    entries: Arc<Vec<PayloadEntry>>,
}

impl Payloads {
    /// Builds transaction payloads for the addresses collected from `test_keys.json`.
    ///
    /// ### Default Values
    /// * `to`: RECIPIENT_ADDRESS
    /// * `value`: 0.01 ETH
    ///
    /// **Note:** JSON-RPC body values are fixed for benchmarking determinism.
    /// Actual gas/nonce/state will be handled by `mockrpc.rs` or live RPC.
    pub fn build_payloads(api_stack: &ApiStack, chain_ids: &[u64]) -> Self {
        assert!(!chain_ids.is_empty(), "chain_ids cannot be empty");

        let entries: Vec<PayloadEntry> = api_stack
            .iter()
            .map(|elements| {
                let api_key = elements.value().to_string();
                let index = usize::try_from(elements.key().clone())
                    .expect("API key index exceeds usize limit");

                // Deterministic chain ID distribution across accounts
                let chain_id = chain_ids[index % chain_ids.len()];
                let chain_id_hex = format!("0x{:x}", chain_id);

                // Robust parsing of the Lobby API key format: <token>:<client_id>:<from_address>
                let from_address = elements
                    .value()
                    .split(':')
                    .nth(2)
                    .expect("Invalid API key format: expected <token>:<client_id>:<from_address>");

                let rpc_payload = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "eth_sendRawTransaction",
                    "params": [{
                        "from": from_address,
                        "to": RECIPIENT_ADDRESS,
                        "value": "0x2386f26fc10000",
                        "chainId": chain_id_hex,
                        "gas": "0x5208",
                        "maxFeePerGas": "0xba43b7400",
                        "maxPriorityFeePerGas": "0x77359400"
                    }],
                    "id": 1
                });

                let payload: Bytes = serde_json::to_vec(&rpc_payload)
                    .expect("Pre-serialization failed")
                    .into();

                PayloadEntry {
                    index,
                    api_key,
                    payload,
                }
            })
            .collect();

        Self {
            entries: Arc::new(entries),
        }
    }

    /// Helper function to extract the `Arc`-backed entries for `TxTrigger`.
    pub fn entries(&self) -> Arc<Vec<PayloadEntry>> {
        self.entries.clone()
    }
}

// =============================================================================
// Dispatch Record
// =============================================================================

/// Structured output from a successful dispatch, consumed by `metrics.rs`.
#[derive(Debug, Clone)]
pub struct DispatchRecord {
    /// `execution_id` for status retrieval
    execution_id: String,
    /// Timestamp immediately before HTTP POST.
    pub t_send: Instant,
    /// Timestamp on HTTP 202 Accepted.
    pub t_accept: Instant,
    /// Index of the API key/account used (useful for shard-distribution validation).
    pub api_key_index: usize,
}

impl DispatchRecord {
    #[inline]
    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }

    #[inline]
    pub fn round_trip_latency_us(&self) -> u64 {
        self.t_accept.duration_since(self.t_send).as_micros() as u64
    }

    #[inline]
    pub fn api_key_index(&self) -> usize {
        self.api_key_index
    }
}

// =============================================================================
// Rate Controller (Virtual Clock Pacer)
// =============================================================================

/// Deterministic inter-arrival scheduler using a Virtual Clock Pacer.
///
/// Prevents multi-worker synchronization bursts by assigning each worker
/// an exact theoretical dispatch timestamp based on a shared virtual clock.
pub struct DynamicRateController {
    ramp_duration_us: f64,
    min_inter_arrival_us: f64,
    max_inter_arrival_us: f64,
    /// Shared virtual clock tracking the next allowed dispatch time in microseconds.
    /// Uses `std::sync::Mutex` as the critical section is non-blocking (nanoseconds).
    next_virtual_time_us: Mutex<f64>,
}

impl DynamicRateController {
    /// Creates a new rate controller.
    /// * `ramp_secs`: Duration of the linear ramp (e.g., 5.0)
    /// * `target_tps`: Steady-state throughput (e.g., 1000.0)
    /// * `initial_delay_us`: Starting inter-arrival delay (e.g., 10_000 for ~100 TPS)
    pub fn new(ramp_secs: f64, target_tps: f64, initial_delay_us: f64) -> Self {
        Self {
            ramp_duration_us: ramp_secs * 1_000_000.0,
            min_inter_arrival_us: 1_000_000.0 / target_tps,
            max_inter_arrival_us: initial_delay_us,
            next_virtual_time_us: Mutex::new(0.0),
        }
    }

    /// Claims the next dispatch slot and blocks the async task until the exact deadline.
    /// Guarantees deterministic spacing even across 1000+ concurrent Tokio workers.
    pub async fn wait_for_next_slot(&self, start_instant: Instant) {
        // 1. Lock virtual clock, claim timestamp, calculate next delay, drop lock immediately
        let (target_virtual_time, delay_us) = {
            let mut next_time = self.next_virtual_time_us.lock().unwrap();
            let current_virtual = *next_time;

            let current_delay = if current_virtual < self.ramp_duration_us {
                let progress = current_virtual / self.ramp_duration_us;
                self.max_inter_arrival_us
                    - (progress * (self.max_inter_arrival_us - self.min_inter_arrival_us))
            } else {
                self.min_inter_arrival_us
            };

            *next_time = current_virtual + current_delay;
            (current_virtual, current_delay)
        };

        // 2. Calculate wall-clock deadline
        let target_instant = start_instant + Duration::from_micros(target_virtual_time as u64);
        let now = Instant::now();

        // 3. Sleep until deadline (if we haven't fallen behind)
        if target_instant > now {
            tokio::time::sleep(target_instant - now).await;
        }

        // Note: `delay_us` is calculated but not directly used for sleep here,
        // as we sleep until the absolute `target_instant` to prevent drift accumulation.
        let _ = delay_us;
    }
}

// =============================================================================
// Main Struct: `TxTrigger`
// =============================================================================

/// High-throughput transaction dispatcher with deterministic rate control.
///
/// Designed to be cloned across multiple Tokio worker tasks. Internally shares
/// `Arc`-backed state for payloads and rate limiting to avoid lock contention
/// and heap allocations during the hot dispatch path.
#[derive(Clone)]
pub struct TxTrigger {
    payloads: Arc<Vec<PayloadEntry>>,
    client: Client,
    base_url: Arc<str>,
    rate_controller: Arc<DynamicRateController>,
}

impl TxTrigger {
    pub fn new(
        payloads: Payloads,
        client: Client,
        base_url: String,
        rate_controller: Arc<DynamicRateController>,
    ) -> Self {
        Self {
            payloads: payloads.entries,
            client,
            base_url: Arc::from(base_url.as_str()),
            rate_controller,
        }
    }

    /// Unified dispatch function. Handles timing, payload selection, HTTP POST,
    /// and metric emission in a single call.
    ///
    /// * `start_instant` - Wall-clock benchmark start time (shared across workers).
    /// * `metrics_tx` - Non-blocking channel to `metrics.rs` histogram aggregator.
    pub async fn ramp_dispatch(
        &self,
        start_instant: Instant,
        metrics_tx: &mpsc::Sender<DispatchRecord>,
    ) -> Result<(), TriggerError> {
        // 1. Wait for exact inter-arrival slot (ramp or steady handled automatically)
        self.rate_controller.wait_for_next_slot(start_instant).await;

        // 2. Zero-copy payload selection (O(1), lock-free)
        let mut rng = rand::thread_rng();
        let entry = self
            .payloads
            .choose(&mut rng)
            .expect("Payloads collection is empty");

        let t_send = Instant::now();

        // 3. Fire request (reqwest streams Bytes directly to kernel socket)
        let response = self
            .client
            .post(&*self.base_url)
            .header("Authorization", format!("Bearer {}", entry.api_key))
            .header("Content-Type", "application/json")
            .body(entry.payload.clone()) // Arc-bump, zero allocation
            .send()
            .await?;

        let status = response.status();
        if status != reqwest::StatusCode::ACCEPTED {
            return Err(TriggerError::UnexpectedStatus(status));
        }

        let t_accept = Instant::now();

        // 4. Extract execution_id for server-side pipeline correlation
        let body: serde_json::Value = response.json().await?;
        let execution_id = body
            .get("result")
            .and_then(|r| r.get("execution_id"))
            .and_then(|id| id.as_str())
            .ok_or(TriggerError::MissingExecutionId)?
            .to_string();

        // 5. Emit metric record (non-blocking, drop if backpressured)
        let record = DispatchRecord {
            execution_id,
            t_send,
            t_accept,
            api_key_index: entry.index,
        };

        // Use try_send to avoid blocking the hot path if the metrics aggregator lags
        let _ = metrics_tx.try_send(record);

        Ok(())
    }
}