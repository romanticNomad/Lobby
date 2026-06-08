use crate::loadgen::RECIPIENT_ADDRESS;
use crate::mockrpc::MockRpcState;
use dashmap::DashMap;
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicU64},
};
use thiserror::Error;
// ============================================================

/// Constant `block_number`.
pub const BLOCK_NUMBER: u64 = 18_000_000;

// ============================================================
// State structs
pub enum StateUpdateOutcome {
    NonceAdvanced(u64),
    NonceTooLow,
}

#[derive(Clone, Debug, Error)]
pub enum StateError {
    #[error("ChainState update failed: {0}")]
    UpdateError(String),
}
// ============================================================

/// Lightweight EVM receipt template.
/// Uses String for hashes for simplicity.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StaticReceipt {
    pub transaction_hash: String,
    pub status: u8, // 1 = success
    pub block_number: u64,
    pub block_hash: String,
    pub from: String,
    pub to: Option<String>,
    pub gas_used: u64,
    pub cumulative_gas_used: u64,
    pub effective_gas_price: u64,
    #[serde(rename = "type")]
    pub tx_type: u8, // 2 = EIP-1559
    pub logs: Vec<serde_json::Value>, // Empty for mock
}

impl StaticReceipt {
    pub fn gen_receipt() -> Arc<StaticReceipt> {
        let receipt = Arc::new(StaticReceipt {
            transaction_hash: format!("0x{:0>64}", "mocktransactionhash"),
            status: 1,
            block_number: BLOCK_NUMBER,
            block_hash: format!("0x{:0>64}", "mockblockhash"),
            from: RECIPIENT_ADDRESS.to_string(),
            to: Some(RECIPIENT_ADDRESS.to_string()),
            gas_used: 21_000,
            cumulative_gas_used: 21_000,
            effective_gas_price: 1_000_000_000,
            tx_type: 2,
            logs: vec![],
        });

        receipt
    }

    #[inline]
    pub fn get_hash(&self) -> String {
        self.transaction_hash.clone()
    }
}

// ============================================================

/// State Collection of Benchmark RPC servers.
///
/// Uses `String` in place of  `Address` and `TxHash` for simplicity.
#[derive(Debug)]
pub struct ChainState {
    /// Address -> Expected Nonce (lock-free atomic advancement)
    pub nonce_collection: DashMap<String, AtomicU64>,
    /// Deterministic Receipt (zero-copy reads)
    pub static_receipt: Arc<StaticReceipt>,
}

// ============================================================
// implementations for ChainState

impl ChainState {
    pub fn new(addresses: Vec<String>) -> Self {
        let nonce_collection = DashMap::new();
        for address in addresses.iter() {
            nonce_collection.insert(address.to_owned(), AtomicU64::new(0));
        }
        let static_receipt = StaticReceipt::gen_receipt();
        Self {
            nonce_collection,
            static_receipt,
        }
    }
}

impl MockRpcState for ChainState {
    fn update_nonce(&self, address: String, nonce_rlp: u64) -> StateUpdateOutcome {
        // dummy reponse
        StateUpdateOutcome::NonceTooLow
    }
    fn fetch_receipt(&self) -> Arc<StaticReceipt> {
        let receipt = self.static_receipt.clone();
        receipt
    }
}

// ============================================================
