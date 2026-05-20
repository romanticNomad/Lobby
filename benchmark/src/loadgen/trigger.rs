use crate::loadgen::keys::{ApiStack, get_addresses};
use bytes::Bytes;

// contants
// ============================================================

/// A dummy EVM adddress used for recieving transactions.
///
/// **Used for lobby benchmarking only, and no real funds are ever sent to this account.**
pub const RECIPIENT_ADDRESS: &str = "0x430b3af2c718497fe0add817c8ead48c8bd2ef61";

// structs
// ============================================================

/// Placeholder for collection of addresses derived from the `ApiStack`
///
/// `Collection` -> Vec<Address>.
pub struct Addresses {
    collection: Vec<String>,
}

impl Addresses {
    pub fn new(api_stack: &ApiStack) -> Self {
        let collection = get_addresses(api_stack);

        Self { collection }
    }
}

// ============================================================

/// Presirealized payloads from each api_key in the `ApiStack`
///
/// `Collection` -> Vec<(api_key, tx_payload_bytes)>.
pub struct Payloads {
    collection: Vec<(String, bytes::Bytes)>,
}

impl Payloads {
    /// Builds transaction payloads for the addresses collected from `test_keys.json`.
    ///
    /// **Default Values**
    /// * to: RECIPIENT_ADDRESS
    /// * valus: 0.01 eth
    /// * chain_id: Hoodi testnet
    ///
    /// **Note**: json-body values (exepct `chain_id` and `to`) don't matter since we will be using a mock_rpc anyways.
    pub fn build_payloads(api_stack: &ApiStack) -> Self {
        let collection: Vec<(String, Bytes)> = api_stack
            .iter()
            .map(|elements| {
                let api_key = elements.value().to_string();
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
                let tx_payload_bytes: bytes::Bytes = serde_json::to_vec(&rpc_payload)
                    .expect("Presirealization failed.")
                    .into();

                (api_key, tx_payload_bytes)
            })
            .collect();

        Self { collection }
    }
}

// ============================================================
