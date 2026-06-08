use crate::loadgen::RECIPIENT_ADDRESS;
use alloy::primitives::B256;
use dashmap::DashMap;
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
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
    TxReceipt(Arc<StaticReceipt>),
    ReceiptNotFound,
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
    pub fn gen_receipts(addresses: Vec<String>) -> DashMap<String, Arc<StaticReceipt>> {
        let receipt_collection = DashMap::new();
        for from in addresses {
            let receipt = Arc::new(StaticReceipt {
                transaction_hash: format!("0x{:0>64}", "mocktransactionhash"),
                status: 1,
                block_number: BLOCK_NUMBER,
                block_hash: format!("0x{:0>64}", "mockblockhash"),
                from: from.clone(),
                to: Some(RECIPIENT_ADDRESS.to_string()),
                gas_used: 21_000,
                cumulative_gas_used: 21_000,
                effective_gas_price: 1_000_000_000,
                tx_type: 2,
                logs: vec![],
            });

            receipt_collection.insert(from, receipt.clone());
        }

        receipt_collection
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
    /// TxHash -> Deterministic Receipt (zero-copy reads)
    pub receipt_collection: DashMap<String, Arc<StaticReceipt>>,
}

// ============================================================
// implementations for ChainState

impl ChainState {
    pub fn new(addresses: Vec<String>) -> Self {
        let nonce_collection = DashMap::new();
        for address in addresses.iter() {
            nonce_collection.insert(address.to_owned(), AtomicU64::new(0));
        }
        let receipt_collection = StaticReceipt::gen_receipts(addresses);
        Self {
            nonce_collection,
            receipt_collection,
        }
    }

    /// Called after successful nonce validation in `eth_sendRawTransaction`.
    ///
    /// Demo receipt are already built using `StaticReceipt::gen_receipts`
    /// therefore only nonce is updated after validation.
    pub fn update_state(&self, from: String) -> Result<(), StateError> {
        match self.nonce_collection.get(&from) {
            Some(nonce) => {
                nonce.fetch_add(1, Ordering::Relaxed);
            }
            None => Err(StateError::UpdateError(format!(
                "address not found: {from}"
            )))?,
        }

        Ok(())
    }
}

// ============================================================
