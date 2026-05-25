use std::sync::Arc;

use crate::loadgen::keys::ApiStack;
use bytes::Bytes;
use thiserror::Error;

// contants
// ============================================================

/// A dummy EVM adddress used for recieving transactions.
///
/// **Used for lobby benchmarking only, and no real funds are ever sent to this account.**
pub const RECIPIENT_ADDRESS: &str = "0x430b3af2c718497fe0add817c8ead48c8bd2ef61";

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
/// * No Dynamic memory allocation.
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
    /// **Note**: json-body values (exepct `chain_id` and `to`) don't matter since we will be using a `mock_rpc` anyways.
    pub fn build_payloads(api_stack: &ApiStack) -> Self {
        let entries: Vec<PayloadEntry> = api_stack
            .iter()
            .map(|elements| {
                let api_key = elements.value().to_string();
                let index =
                    usize::try_from(elements.key().clone()).expect("value exeeds usize limit");
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
                    .expect("Presirealization failed.")
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
}

// ============================================================

/// High-throughput transaction trigger with deterministic rate control.
/// Designed to be shared across multiple Tokio worker tasks via `Arc`.
pub struct TxTrigger {}
