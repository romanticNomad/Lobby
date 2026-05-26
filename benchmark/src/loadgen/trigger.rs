use std::{sync::Arc, time::Instant};

use crate::loadgen::keys::ApiStack;
use bytes::Bytes;
use governor::{
    RateLimiter,
    clock::DefaultClock,
    middleware::NoOpMiddleware,
    state::{InMemoryState, NotKeyed},
};
use rand::seq::SliceRandom;
use reqwest::Client;
use thiserror::Error;

// contants
// ============================================================

/// A dummy EVM adddress used for recieving transactions.
///
/// **Used for lobby benchmarking only, and no real funds are ever sent to this account.**
pub const RECIPIENT_ADDRESS: &str = "0x430b3af2c718497fe0add817c8ead48c8bd2ef61";

// rate limiter type alias
// ============================================================

/// Unkeyed, direct-state rate limiter using the default tokio clock.
/// Shared across workers via `Arc` to distribute permits without contention.
pub type TriggerRateLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>;

// structs
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
                let payload: bytes::Bytes = serde_json::to_vec(&rpc_payload)
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

    /// Helper function to build the TxTrigger from the already built `Payloads` struct.
    pub fn entries(&self) -> Arc<[PayloadEntry]> {
        self.entries.clone()
    }
}

// =============================================================================
// Dispatch Record

/// Structured output from a successful dispatch, consumed by `metrics.rs`.
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

// ============================================================
// main struct `TxTrigger`

/// High-throughput transaction dispatcher with deterministic rate control.
///
/// Designed to be cloned across multiple Tokio worker tasks. Internally shares
/// `Arc`-backed state for payloads and rate limiting to avoid lock contention
/// and heap allocations during the hot dispatch path.
pub struct TxTrigger {
    payloads: Arc<[PayloadEntry]>,
    rate_limiter: Arc<TriggerRateLimiter>,
    client: Client,
    base_url: String,
}

impl TxTrigger {
    /// Creates a new `TxTrigger` instance.
    ///
    /// * `payloads` - Pre-serialized transaction payloads.
    /// * `rate_limiter` - Shared governor instance for steady-state TPS control.
    /// * `client` - Pre-warmed `reqwest::Client` with connection pooling enabled.
    /// * `base_url` - Target Lobby submission endpoint (e.g., `http://127.0.0.1:3000/v1/transactions`).
    pub fn new(
        payloads: Payloads,
        rate_limiter: Arc<TriggerRateLimiter>,
        client: Client,
        base_url: String,
    ) -> Self {
        Self {
            payloads: payloads.entries(),
            rate_limiter,
            client,
            base_url,
        }
    }

    /// Acquires a rate-limit permit asynchronously.
    ///
    /// Yields to the Tokio scheduler until a token is available.
    /// **Note:** This is intended for the steady-state phase (5s–55s).
    /// During the ramp phase (0s–5s), the orchestrator should use
    /// `tokio::time::sleep` with a linearly decreasing interval instead.
    pub async fn acquire_permit(&self) {
        self.rate_limiter.until_ready().await;
    }

    /// Selects a random payload in O(1) without locking.
    ///
    /// Uses `rand::thread_rng` for uniform distribution across `ByAddress`
    /// shards in Lobby's pipeline.
    pub fn select_payload(&self) -> &PayloadEntry {
        let mut rnd = rand::thread_rng();
        self.payloads
            .choose(&mut rnd)
            .expect("Payload collection must have valid elements")
    }

    /// Dispatches a single transaction to Lobby, respecting the rate limiter.
    ///
    /// Returns a `DispatchRecord` containing timestamps and identifiers
    /// required by `metrics.rs` to compute client-acceptance latency.
    pub async fn dispatch(&self) -> Result<DispatchRecord, TriggerError> {
        // rate control
        self.acquire_permit().await;

        // payload selection (random distribution)
        let payload_entry = self.select_payload();
        let api_key_index = payload_entry.index;

        // mark submission instant
        let t_send = Instant::now();

        // submit request to Lobby server
        let response = self
            .client
            .post(&self.base_url)
            .header("Authorization", format!("Bearer {}", payload_entry.api_key))
            .header("Content-Type", "application/json")
            .body(payload_entry.payload.clone()) // Bytes::clone() is reference-counted (zero-copy)
            .send()
            .await?;

        // validate submission
        let status = response.status();
        if status != reqwest::StatusCode::ACCEPTED {
            return Err(TriggerError::UnexpectedStatus(status)); // expected status_code = 202
        }

        // marke acceptance instant
        let t_accept = Instant::now();

        // extract execution_id
        let response_body: serde_json::Value = response.json().await?;
        let execution_id = response_body
            .get("result")
            .and_then(|r| r.get("execution_id"))
            .and_then(|id| id.as_str())
            .ok_or(TriggerError::MissingExecutionId)?
            .to_string();

        Ok(DispatchRecord {
            execution_id,
            t_send,
            t_accept,
            api_key_index,
        })
    }
}

// ============================================================
