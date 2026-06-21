use crate::loadgen::keys::ApiStack;
use bytes::Bytes;
use rand::seq::SliceRandom;
use reqwest::Client;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::mpsc;

// constants
// ============================================================

/// A dummy EVM adddress used for recieving transactions.
///
/// **Used for lobby benchmarking only, and no real funds are ever sent to this account.**
pub const RECIPIENT_ADDRESS: &str = "0x430b3af2c718497fe0add817c8ead48c8bd2ef61";

// data structures
// ============================================================
// errors

/// Errors involved in trigger action to Lobby server
#[derive(Debug, Error)]
pub enum TriggerError {
    #[error("Http request error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Unexpected status code recieved: {0}")]
    UnexpectedStatus(reqwest::StatusCode),

    #[error("Missing execution_id in responce")]
    MissingExecutionId,

    #[error("Json deserialization failed: {0}")]
    SerDe(#[from] serde_json::Error),
}

// ============================================================
// payload structs

/// Thread-safe, pre-serialized transaction payload with associated API key.
///
/// Designed for O(1) random selection and zero-copy cloning via `Bytes`.
#[derive(Debug, Clone)]
pub struct PayloadEntry {
    pub index: usize,
    pub api_key: String,
    pub payload: Bytes,
}

/// Presirealized payloads from each api_key in the `ApiStack`
///
/// Features:
/// * Cheap to clone across Tokio workers (`Arc`-backed).
/// * Avoids heap calls by using array of `PayloadEntry` instead of vector.
#[derive(Debug, Clone)]
pub struct Payloads {
    entries: Arc<[PayloadEntry]>,
}

impl Payloads {
    /// Builds transaction payloads for the addresses collected from `test_keys.json`.
    ///
    /// **Default Values**
    /// * to: RECIPIENT_ADDRESS
    /// * valus: 0.01 eth
    /// * chain_id: Hoodi testnet
    ///
    /// **Note:** JSON-RPC body values are fixed for benchmarking determinism.
    /// Actual gas/nonce/state will be handled by `mockrpc.rs` or live RPC.
    pub fn build_payloads(api_stack: &ApiStack) -> Self {
        let entries: Vec<PayloadEntry> = api_stack
            .iter()
            .map(|elements| {
                let api_key = elements.value().to_string();
                let index =
                    usize::try_from(elements.key().clone()).expect("value exceeds usize limit");
                let api_key_elements: Vec<&str> = elements.value().split(":").collect();
                let from_address = api_key_elements[2];

                let rpc_payload = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "eth_sendRawTransaction",
                    "params": [{
                        "from": from_address,
                        "to": RECIPIENT_ADDRESS,
                        "value": "0x2386f26fc10000",
                        "chainId": "0x88bb0",
                        "gas": "0x5208",
                        "maxFeePerGas": "0xba43b7400",
                        "maxPriorityFeePerGas": "0x77359400"
                    }],
                    "id": 1
                });
                let payload: Bytes = serde_json::to_vec(&rpc_payload)
                    .expect("Pre-sirealization failed.")
                    .into();

                PayloadEntry {
                    index,
                    api_key,
                    payload,
                }
            })
            .collect();

        Self {
            entries: entries.into(),
        }
    }

    /// Helper function to build the `TxTrigger` from
    /// the already built `Payloads` struct.
    pub fn entries(&self) -> Arc<[PayloadEntry]> {
        self.entries.clone()
    }
}

// =============================================================================
// Dispatch Record

/// Structured output from a successful dispatch, consumed by `mod.rs`.
#[derive(Debug, Clone)]
pub struct DispatchRecord {
    /// `execution_id` for status retrival
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
    pub fn get_execution_id(&self) -> String {
        self.execution_id.clone()
    }

    #[inline]
    pub fn get_round_trip_latency(&self) -> u64 {
        self.t_accept.duration_since(self.t_send).as_micros() as u64
    }

    #[inline]
    pub fn get_api_key_index(&self) -> usize {
        self.api_key_index
    }
}

// =============================================================================
// Rate Controller

/// Deterministic inter-arrival scheduler.
///
/// Calculates exact sleep duration based on elapsed wall-clock time.
/// Naturally handles linear ramp → steady-state without phase boundaries.
pub struct DynamicRateController {
    ramp_duration_us: f64,
    min_inter_arrival_us: f64,
    max_inter_arrival_us: f64,
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
        }
    }

    /// Blocks the async task until the exact next dispatch slot.
    /// Uses closed-loop drift correction: recalculates delay every iteration.
    pub async fn wait_for_next_slot(&self, start_instant: Instant) {
        let elapsed_us = start_instant.elapsed().as_micros() as f64;

        // Linear interpolation of inter-arrival time during ramp
        let delay_us = if elapsed_us < self.ramp_duration_us {
            let progress = elapsed_us / self.ramp_duration_us;
            self.max_inter_arrival_us
                - (progress * (self.max_inter_arrival_us - self.min_inter_arrival_us))
        } else {
            // Steady state: constant inter-arrival
            self.min_inter_arrival_us
        };

        // Sleep until the calculated deadline
        tokio::time::sleep(Duration::from_micros(delay_us as u64)).await;
    }
}

// ============================================================
// main struct `TxTrigger`

/// High-throughput transaction dispatcher with deterministic rate control.
///
/// Designed to be cloned across multiple Tokio worker tasks. Internally shares
/// `Arc`-backed state for payloads and rate limiting to avoid lock contention
/// and heap allocations during the hot dispatch path.
pub struct TxTrigger {
    payloads: Arc<[PayloadEntry]>,
    client: Client,
    base_url: String,
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
            base_url,
            rate_controller,
        }
    }

    /// Unified dispatch function. Handles timing, payload selection, HTTP POST,
    /// and metric emission in a single call. No phase switching required.
    ///
    /// * `start_instant` - Wall-clock benchmark start time (shared across workers).
    /// * `metrics_tx` - Non-blocking channel to `mod.rs` histogram aggregator.
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
            .post(&self.base_url)
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
        let _ = metrics_tx.try_send(record);

        Ok(())
    }
}

// ============================================================
